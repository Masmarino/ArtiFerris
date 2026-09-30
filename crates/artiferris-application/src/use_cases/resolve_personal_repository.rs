use std::sync::Arc;

use artiferris_domain::organization::OrganizationRepositoryPort;
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary};
use artiferris_domain::user::{UserRepositoryPort, Username};

use crate::error::ApplicationError;
use crate::use_cases::personal_repository::personal_organization_slug;
use crate::use_cases::public_catalog::has_control_character;

/// Looks up a personal project by owner username and project name (`@alice/my-lib`, `alice/my-lib`). Shared by npm and
/// docker.
pub struct ResolvePersonalRepositoryUseCase {
    users: Arc<dyn UserRepositoryPort>,
    organizations: Arc<dyn OrganizationRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
}

impl ResolvePersonalRepositoryUseCase {
    pub fn new(users: Arc<dyn UserRepositoryPort>, organizations: Arc<dyn OrganizationRepositoryPort>, repositories: Arc<dyn PackageRepositoryQueryPort>) -> Self {
        Self { users, organizations, repositories }
    }

    /// `None` for an unknown user, an unreserved namespace or an unknown repository, indistinguishably.
    pub async fn execute(&self, username: &str, repo_name: &str) -> Result<Option<PackageRepositorySummary>, ApplicationError> {
        let Ok(username) = Username::parse(username) else { return Ok(None) };
        if has_control_character(repo_name) {
            return Ok(None);
        }
        let Some(user) = self.users.find_by_username(&username).await? else { return Ok(None) };

        let slug = personal_organization_slug(user.id);
        let Some(org) = self.organizations.find_by_slug(&slug).await? else { return Ok(None) };

        Ok(self.repositories.find_by_org_and_name(org.id, repo_name).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
    use artiferris_domain::user::User;
    use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
    use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
    use artiferris_infrastructure::postgres::user_repository::PostgresUserRepository;
    use sqlx::PgPool;
    use uuid::Uuid;

    use crate::use_cases::personal_repository::{CreateUserProjectUseCase, ReservePersonalOrganizationUseCase};

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
    async fn resolves_a_project_by_username_and_repo_name(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let user_id = seed_user(&pool, "alice").await;

        let reserve = ReservePersonalOrganizationUseCase::new(organizations.clone(), users.clone());
        reserve.execute(user_id).await.unwrap();
        let create_project = CreateUserProjectUseCase::new(organizations.clone(), repository_store.clone(), repository_store.clone());
        create_project.execute(user_id, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap();

        let resolve = ResolvePersonalRepositoryUseCase::new(users, organizations, repository_store);
        let found = resolve.execute("alice", "my-lib").await.unwrap();

        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "my-lib");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn resolving_an_unknown_username_returns_none(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));

        let resolve = ResolvePersonalRepositoryUseCase::new(users, organizations, repository_store);
        let found = resolve.execute("nobody", "my-lib").await.unwrap();

        assert!(found.is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn resolving_an_unreserved_personal_namespace_returns_none(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        seed_user(&pool, "bob").await;

        let resolve = ResolvePersonalRepositoryUseCase::new(users, organizations, repository_store);
        let found = resolve.execute("bob", "my-lib").await.unwrap();

        assert!(found.is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repo_name_with_a_control_character_resolves_to_none_instead_of_failing(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let user_id = seed_user(&pool, "alice").await;
        ReservePersonalOrganizationUseCase::new(organizations.clone(), users.clone()).execute(user_id).await.unwrap();

        let resolve = ResolvePersonalRepositoryUseCase::new(users, organizations, repository_store);

        for name in ["a\u{0}b", "\u{0}", "tab\tbed"] {
            assert!(resolve.execute("alice", name).await.unwrap().is_none(), "{name:?}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn resolving_an_unknown_repo_name_returns_none(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let user_id = seed_user(&pool, "alice").await;
        let reserve = ReservePersonalOrganizationUseCase::new(organizations.clone(), users.clone());
        reserve.execute(user_id).await.unwrap();

        let resolve = ResolvePersonalRepositoryUseCase::new(users, organizations, repository_store);
        let found = resolve.execute("alice", "does-not-exist").await.unwrap();

        assert!(found.is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_invalid_username_returns_none_instead_of_an_error(pool: PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));

        let resolve = ResolvePersonalRepositoryUseCase::new(users, organizations, repository_store);
        let found = resolve.execute("!not a valid username!", "my-lib").await.unwrap();

        assert!(found.is_none());
    }
}
