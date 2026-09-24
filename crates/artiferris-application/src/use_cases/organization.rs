use std::sync::Arc;

use artiferris_domain::organization::{Organization, OrganizationRepositoryPort, OrganizationSlug};
use uuid::Uuid;

use crate::error::ApplicationError;

/// True for anything shaped like a personal-org slug (`u` + 24 lowercase hex chars) — a normal
/// organization must never be creatable with a slug in this shape, or it could squat a real or
/// future user's personal namespace (B-9).
fn looks_like_personal_org_slug(slug: &str) -> bool {
    slug.len() == 25 && slug.starts_with('u') && slug[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub struct CreateOrganizationUseCase {
    organizations: Arc<dyn OrganizationRepositoryPort>,
}

impl CreateOrganizationUseCase {
    pub fn new(organizations: Arc<dyn OrganizationRepositoryPort>) -> Self {
        Self { organizations }
    }

    pub async fn execute(&self, slug: &str, display_name: &str) -> Result<Uuid, ApplicationError> {
        let slug = OrganizationSlug::parse_new(slug)?;
        if looks_like_personal_org_slug(slug.as_str()) {
            return Err(ApplicationError::ReservedOrganizationSlug);
        }
        if self.organizations.find_by_slug(&slug).await?.is_some() {
            return Err(ApplicationError::OrganizationSlugTaken);
        }
        let id = Uuid::new_v4();
        let org = Organization { id, slug, display_name: display_name.to_string(), is_personal: false, is_public: false, created_at: chrono::Utc::now() };
        self.organizations.create(&org).await?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::error::DomainError;
    use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_an_organization_succeeds_and_is_findable(pool: sqlx::PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool));
        let use_case = CreateOrganizationUseCase::new(organizations.clone());

        let id = use_case.execute("acme", "Acme Corp").await.unwrap();

        let found = organizations.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(found.slug.as_str(), "acme");
        assert_eq!(found.display_name, "Acme Corp");
        assert!(!found.is_public);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_an_organization_with_a_taken_slug_fails(pool: sqlx::PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool));
        let use_case = CreateOrganizationUseCase::new(organizations.clone());
        use_case.execute("acme", "Acme Corp").await.unwrap();

        let result = use_case.execute("acme", "Acme Corp Again").await;

        assert!(matches!(result, Err(ApplicationError::OrganizationSlugTaken)));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_slug_using_the_reserved_prefix_is_rejected(pool: sqlx::PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool));
        let use_case = CreateOrganizationUseCase::new(organizations);

        let err = use_case.execute("artiferris-npm", "Impostor").await.unwrap_err();

        assert!(matches!(err, ApplicationError::Domain(DomainError::ReservedName(_))));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_invalid_slug_is_rejected_before_touching_the_database(pool: sqlx::PgPool) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool));
        let use_case = CreateOrganizationUseCase::new(organizations);

        let result = use_case.execute("1nvalid", "Whatever").await;

        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_an_organization_with_a_personal_org_shaped_slug_is_rejected(pool: sqlx::PgPool) {
        use crate::use_cases::personal_repository::personal_organization_slug;
        let repo = Arc::new(PostgresOrganizationRepository::new(pool));
        let use_case = CreateOrganizationUseCase::new(repo);
        let personal_shaped = personal_organization_slug(uuid::Uuid::new_v4());

        let err = use_case.execute(personal_shaped.as_str(), "Squatter Inc").await.unwrap_err();
        assert!(matches!(err, ApplicationError::ReservedOrganizationSlug));
    }
}
