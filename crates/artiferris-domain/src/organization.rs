use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::DomainError;
use crate::reserved_names::reject_reserved_name;

/// Every environment seeds exactly this id as the public organization (see `0001_init.sql`).
pub const PUBLIC_ORGANIZATION_ID: Uuid = Uuid::from_u128(1);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrganizationSlug(String);

impl OrganizationSlug {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let len_ok = (3..=32).contains(&raw.len());
        let starts_with_letter = raw.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        let chars_ok = raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

        if len_ok && starts_with_letter && chars_ok {
            Ok(Self(raw.to_string()))
        } else {
            Err(DomainError::InvalidOrganizationSlug(raw.to_string()))
        }
    }

    /// For a slug being created. `parse` stays lenient so organizations that predate the `artiferris-` reservation
    /// still load.
    pub fn parse_new(raw: &str) -> Result<Self, DomainError> {
        let slug = Self::parse(raw)?;
        reject_reserved_name(raw)?;
        Ok(slug)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Organization {
    pub id: Uuid,
    pub slug: OrganizationSlug,
    pub display_name: String,
    pub is_public: bool,
    /// A personal namespace's own hidden organization, left out of every organization listing and picker.
    pub is_personal: bool,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait OrganizationRepositoryPort: Send + Sync {
    async fn create(&self, org: &Organization) -> Result<(), DomainError>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Organization>, DomainError>;
    async fn find_by_slug(&self, slug: &OrganizationSlug) -> Result<Option<Organization>, DomainError>;
    /// Exactly one row always has `is_public = true` on a correctly migrated database —
    /// returns `Err`, not a panic, if that invariant is ever broken.
    async fn find_public(&self) -> Result<Organization, DomainError>;
    async fn list_all(&self) -> Result<Vec<Organization>, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_slug_cannot_use_the_reserved_prefix() {
        assert_eq!(OrganizationSlug::parse_new("ArtiFerris-npm"), Err(DomainError::ReservedName("ArtiFerris-npm".to_string())));
        assert!(OrganizationSlug::parse_new("acme").is_ok());
    }

    #[test]
    fn an_existing_slug_with_the_reserved_prefix_can_still_be_looked_up() {
        assert!(OrganizationSlug::parse("artiferris-legacy").is_ok());
    }

    #[test]
    fn a_valid_slug_parses() {
        assert!(OrganizationSlug::parse("acme").is_ok());
        assert!(OrganizationSlug::parse("acme-corp").is_ok());
        assert!(OrganizationSlug::parse("acme_corp").is_ok());
    }

    #[test]
    fn a_slug_shorter_than_three_characters_is_rejected() {
        assert!(OrganizationSlug::parse("ab").is_err());
    }

    #[test]
    fn a_slug_not_starting_with_a_letter_is_rejected() {
        assert!(OrganizationSlug::parse("1acme").is_err());
        assert!(OrganizationSlug::parse("-acme").is_err());
    }

    #[test]
    fn a_slug_with_invalid_characters_is_rejected() {
        assert!(OrganizationSlug::parse("acme.corp").is_err());
        assert!(OrganizationSlug::parse("acme corp").is_err());
    }

    #[test]
    fn public_organization_id_matches_the_seeded_migration_row() {
        assert_eq!(PUBLIC_ORGANIZATION_ID, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
    }

    #[test]
    fn a_personal_organization_is_constructed_with_the_flag_set() {
        let org = Organization {
            id: Uuid::new_v4(),
            slug: OrganizationSlug::parse("u-abc123").unwrap(),
            display_name: "alice".to_string(),
            is_public: false,
            is_personal: true,
            created_at: Utc::now(),
        };
        assert!(org.is_personal);
    }
}
