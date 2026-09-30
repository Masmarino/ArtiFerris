use std::sync::Arc;

use artiferris_domain::organization::{Organization, OrganizationRepositoryPort, OrganizationSlug};
use artiferris_domain::package_repository::{
    PackageRepository, PackageRepositoryQueryPort, RepositoryFormat, RepositoryType, parse_repository_name,
};
use artiferris_domain::permission::{Permission, Role};
use artiferris_domain::personal_repository::PersonalProjectProvisioningPort;
use artiferris_domain::user::UserRepositoryPort;
use uuid::Uuid;

use crate::error::ApplicationError;

/// Slug is derived from the user id, not the username, so it can't collide with real org slugs.
pub fn personal_organization_slug(user_id: Uuid) -> OrganizationSlug {
    let hex = user_id.simple().to_string();
    OrganizationSlug::parse(&format!("u{}", &hex[..24])).expect("a UUID-derived slug is always valid")
}

pub struct ReservePersonalOrganizationUseCase {
    organizations: Arc<dyn OrganizationRepositoryPort>,
    users: Arc<dyn UserRepositoryPort>,
}

impl ReservePersonalOrganizationUseCase {
    pub fn new(organizations: Arc<dyn OrganizationRepositoryPort>, users: Arc<dyn UserRepositoryPort>) -> Self {
        Self { organizations, users }
    }

    pub async fn execute(&self, user_id: Uuid) -> Result<Uuid, ApplicationError> {
        let user = self.users.find_by_id(user_id).await?.ok_or(ApplicationError::NoPersonalOrganization)?;

        let slug = personal_organization_slug(user_id);

        if self.organizations.find_by_slug(&slug).await?.is_some() {
            return Err(ApplicationError::PersonalOrganizationAlreadyExists);
        }

        let org = Organization {
            id: Uuid::new_v4(),
            slug,
            display_name: user.username.as_str().to_string(),
            is_public: false,
            is_personal: true,
            created_at: chrono::Utc::now(),
        };
        self.organizations.create(&org).await?;
        Ok(org.id)
    }
}

pub struct FindMyPersonalOrganizationUseCase {
    organizations: Arc<dyn OrganizationRepositoryPort>,
}

impl FindMyPersonalOrganizationUseCase {
    pub fn new(organizations: Arc<dyn OrganizationRepositoryPort>) -> Self {
        Self { organizations }
    }

    pub async fn execute(&self, user_id: Uuid) -> Result<Option<Uuid>, ApplicationError> {
        let slug = personal_organization_slug(user_id);
        Ok(self.organizations.find_by_slug(&slug).await?.map(|org| org.id))
    }
}

pub struct CreateUserProjectUseCase {
    organizations: Arc<dyn OrganizationRepositoryPort>,
    provisioning: Arc<dyn PersonalProjectProvisioningPort>,
    repository_query: Arc<dyn PackageRepositoryQueryPort>,
}

impl CreateUserProjectUseCase {
    pub fn new(
        organizations: Arc<dyn OrganizationRepositoryPort>,
        provisioning: Arc<dyn PersonalProjectProvisioningPort>,
        repository_query: Arc<dyn PackageRepositoryQueryPort>,
    ) -> Self {
        Self { organizations, provisioning, repository_query }
    }

    pub async fn execute(&self, owner_user_id: Uuid, name: &str, format: RepositoryFormat, repo_type: RepositoryType) -> Result<Uuid, ApplicationError> {
        let slug = personal_organization_slug(owner_user_id);
        let personal_org = self.organizations.find_by_slug(&slug).await?.ok_or(ApplicationError::NoPersonalOrganization)?;

        let name = parse_repository_name(name)?;
        if self.repository_query.find_by_org_and_name(personal_org.id, &name).await?.is_some() {
            return Err(ApplicationError::RepositoryNameTaken);
        }

        let repository_id = Uuid::new_v4();
        let created = PackageRepository::create(repository_id, personal_org.id, name, format, repo_type, None, None, None)?;
        let grant = Permission::default().grant(owner_user_id, repository_id, Role::Admin);

        // Repository creation and the owner's grant persist together in one transaction, or
        // neither does — see `PersonalProjectProvisioningPort`'s doc comment for why that matters.
        self.provisioning.create_with_owner_grant(repository_id, created, owner_user_id, grant, owner_user_id).await?;

        Ok(repository_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use artiferris_domain::package_repository::{PackageRepositoryQueryPort, RepositoryFormat, RepositoryType};
    use artiferris_domain::permission::{PermissionQueryPort, Role};
    use artiferris_domain::user::{User, Username};
    use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
    use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
    use artiferris_infrastructure::postgres::permission_store::PostgresPermissionStore;
    use artiferris_infrastructure::postgres::user_repository::PostgresUserRepository;
    use sqlx::PgPool;

    async fn seed_user(pool: &PgPool, username: &str) -> Uuid {
        let users = PostgresUserRepository::new(pool.clone());
        let id = Uuid::new_v4();
        users
            .insert(&User {
                id,
                username: Username::parse(username).unwrap(),
                password_hash: "irrelevant".to_string(),
                is_super_admin: false,
                is_organization_admin: false,
                organization_id: PUBLIC_ORGANIZATION_ID,
                created_at: chrono::Utc::now(),
                tokens_valid_after: chrono::Utc::now(),
                email: None,
            })
            .await
            .unwrap();
        id
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn reserving_creates_a_hidden_personal_organization(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let user_id = seed_user(&pool, "alice").await;
        let use_case = ReservePersonalOrganizationUseCase::new(organizations.clone(), users);

        let org_id = use_case.execute(user_id).await.unwrap();

        let org = organizations.find_by_id(org_id).await.unwrap().unwrap();
        assert!(org.is_personal);
        assert!(!org.is_public);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn reserving_twice_fails(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let user_id = seed_user(&pool, "alice").await;
        let use_case = ReservePersonalOrganizationUseCase::new(organizations, users);

        use_case.execute(user_id).await.unwrap();
        let err = use_case.execute(user_id).await.unwrap_err();
        assert!(matches!(err, ApplicationError::PersonalOrganizationAlreadyExists));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn reserving_for_an_unknown_user_fails(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let use_case = ReservePersonalOrganizationUseCase::new(organizations, users);

        let err = use_case.execute(Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::NoPersonalOrganization));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_derived_slug_is_deterministic_for_the_same_user(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let user_id = seed_user(&pool, "alice").await;
        let use_case = ReservePersonalOrganizationUseCase::new(organizations.clone(), users);

        let org_id = use_case.execute(user_id).await.unwrap();
        let org = organizations.find_by_id(org_id).await.unwrap().unwrap();

        let expected_slug = OrganizationSlug::parse(&format!("u{}", &user_id.simple().to_string()[..24])).unwrap();
        assert_eq!(org.slug, expected_slug);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_project_grants_the_creator_admin(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let permission_store = Arc::new(PostgresPermissionStore::new(pool.clone()));
        let user_id = seed_user(&pool, "alice").await;

        let reserve = ReservePersonalOrganizationUseCase::new(organizations.clone(), users.clone());
        reserve.execute(user_id).await.unwrap();

        let create_project = CreateUserProjectUseCase::new(organizations, repository_store.clone(), repository_store.clone());
        let repo_id = create_project.execute(user_id, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap();

        let role = permission_store.find_role(user_id, repo_id).await.unwrap();
        assert_eq!(role, Some(Role::Admin));

        let repo = repository_store.find_by_id(repo_id).await.unwrap().unwrap();
        assert_eq!(repo.name, "my-lib");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_project_without_a_reserved_namespace_fails(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let user_id = seed_user(&pool, "alice").await;

        let create_project = CreateUserProjectUseCase::new(organizations, repository_store.clone(), repository_store);
        let err = create_project.execute(user_id, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap_err();
        assert!(matches!(err, ApplicationError::NoPersonalOrganization));
    }

    /// A fake `PersonalProjectProvisioningPort` that always fails, simulating a transient error
    /// (pool exhaustion, a dropped connection) hitting the atomic provisioning call.
    struct AlwaysFailsProvisioning;

    #[async_trait::async_trait]
    impl artiferris_domain::personal_repository::PersonalProjectProvisioningPort for AlwaysFailsProvisioning {
        async fn create_with_owner_grant(
            &self,
            _repository_id: Uuid,
            _repository_event: artiferris_domain::package_repository::PackageRepositoryEvent,
            _owner_user_id: Uuid,
            _permission_event: artiferris_domain::permission::PermissionEvent,
            _actor_id: Uuid,
        ) -> Result<(), artiferris_domain::error::EventStoreError> {
            Err(artiferris_domain::error::EventStoreError::Storage("simulated transient failure".to_string()))
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failure_granting_the_owner_permission_does_not_leave_an_ungranted_orphan_repository(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let user_id = seed_user(&pool, "alice").await;
        ReservePersonalOrganizationUseCase::new(organizations.clone(), users).execute(user_id).await.unwrap();

        let failing_create_project = CreateUserProjectUseCase::new(organizations.clone(), Arc::new(AlwaysFailsProvisioning), repository_store.clone());
        let err = failing_create_project.execute(user_id, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap_err();
        assert!(matches!(err, ApplicationError::EventStore(_)), "expected the provisioning failure to propagate, got {err:?}");

        let personal_org = organizations.find_by_slug(&personal_organization_slug(user_id)).await.unwrap().unwrap();
        assert!(
            repository_store.find_by_org_and_name(personal_org.id, "my-lib").await.unwrap().is_none(),
            "the failed attempt must not have left a repository behind"
        );

        let retry_create_project = CreateUserProjectUseCase::new(organizations, repository_store.clone(), repository_store.clone());
        let repo_id = retry_create_project.execute(user_id, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap();
        assert!(repository_store.find_by_id(repo_id).await.unwrap().is_some());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn finding_a_reserved_personal_organization_returns_its_id(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let user_id = seed_user(&pool, "alice").await;
        let reserve = ReservePersonalOrganizationUseCase::new(organizations.clone(), users);
        let org_id = reserve.execute(user_id).await.unwrap();

        let find = FindMyPersonalOrganizationUseCase::new(organizations);
        let found = find.execute(user_id).await.unwrap();

        assert_eq!(found, Some(org_id));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn finding_an_unreserved_personal_organization_returns_none(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let user_id = seed_user(&pool, "alice").await;
        let find = FindMyPersonalOrganizationUseCase::new(organizations);

        let found = find.execute(user_id).await.unwrap();

        assert_eq!(found, None);
    }
}
