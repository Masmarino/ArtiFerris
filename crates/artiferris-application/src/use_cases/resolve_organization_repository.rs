use std::sync::Arc;

use artiferris_domain::organization::{OrganizationRepositoryPort, OrganizationSlug};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary};

use crate::error::ApplicationError;
use crate::use_cases::public_catalog::has_control_character;

/// Looks up a repository by organization slug and name. Personal organizations go through
/// `ResolvePersonalRepositoryUseCase`.
pub struct ResolveOrganizationRepositoryUseCase {
    organizations: Arc<dyn OrganizationRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
}

impl ResolveOrganizationRepositoryUseCase {
    pub fn new(organizations: Arc<dyn OrganizationRepositoryPort>, repositories: Arc<dyn PackageRepositoryQueryPort>) -> Self {
        Self { organizations, repositories }
    }

    /// `None` for an unknown slug, a personal organization or an unknown name, indistinguishably.
    pub async fn execute(&self, slug: &str, repo_name: &str) -> Result<Option<PackageRepositorySummary>, ApplicationError> {
        let Ok(slug) = OrganizationSlug::parse(slug) else { return Ok(None) };
        if has_control_character(repo_name) {
            return Ok(None);
        }
        let Some(organization) = self.organizations.find_by_slug(&slug).await? else { return Ok(None) };
        if organization.is_personal {
            return Ok(None);
        }
        Ok(self.repositories.find_by_org_and_name(organization.id, repo_name).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
    use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
    use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
    use sqlx::PgPool;
    use uuid::Uuid;

    use crate::use_cases::organization::CreateOrganizationUseCase;
    use crate::use_cases::package_repository::CreatePackageRepositoryUseCase;

    struct Fixture {
        resolve: ResolveOrganizationRepositoryUseCase,
        acme_id: Uuid,
        store: Arc<PostgresPackageRepositoryStore>,
        organizations: Arc<PostgresOrganizationRepository>,
    }

    async fn fixture(pool: PgPool) -> Fixture {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let store = Arc::new(PostgresPackageRepositoryStore::new(pool, "test-secret".to_string()));
        let acme_id = CreateOrganizationUseCase::new(organizations.clone()).execute("acme", "Acme").await.unwrap();
        CreatePackageRepositoryUseCase::new(store.clone(), store.clone())
            .execute(acme_id, "libs", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, Uuid::new_v4())
            .await
            .unwrap();
        Fixture { resolve: ResolveOrganizationRepositoryUseCase::new(organizations.clone(), store.clone()), acme_id, store, organizations }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn resolves_a_repository_by_organization_slug_and_name(pool: PgPool) {
        let f = fixture(pool).await;

        let found = f.resolve.execute("acme", "libs").await.unwrap().unwrap();

        assert_eq!((found.name.as_str(), found.organization_id), ("libs", f.acme_id));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn unknown_slug_unknown_repository_and_malformed_slug_all_resolve_to_none(pool: PgPool) {
        let f = fixture(pool).await;

        assert!(f.resolve.execute("nobody", "libs").await.unwrap().is_none());
        assert!(f.resolve.execute("acme", "nope").await.unwrap().is_none());
        assert!(f.resolve.execute("!bad slug!", "libs").await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_name_with_a_control_character_resolves_to_none_instead_of_failing(pool: PgPool) {
        let f = fixture(pool).await;

        for name in ["a\u{0}b", "\u{0}", "tab\tbed"] {
            assert!(f.resolve.execute("acme", name).await.unwrap().is_none(), "{name:?}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_personal_organization_is_never_resolved_this_way(pool: PgPool) {
        use artiferris_domain::organization::Organization;
        let f = fixture(pool).await;
        let personal = Organization {
            id: Uuid::new_v4(),
            slug: OrganizationSlug::parse("u0123456789abcdef01234567").unwrap(),
            display_name: "alice".to_string(),
            is_public: false,
            is_personal: true,
            created_at: chrono::Utc::now(),
        };
        f.organizations.create(&personal).await.unwrap();
        CreatePackageRepositoryUseCase::new(f.store.clone(), f.store.clone())
            .execute(personal.id, "secret-project", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, Uuid::new_v4())
            .await
            .unwrap();

        assert!(f.resolve.execute("u0123456789abcdef01234567", "secret-project").await.unwrap().is_none());
    }
}
