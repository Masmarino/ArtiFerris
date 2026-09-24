use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{AdminAuditRecord, AuditRecord, SecurityAuditRecord};
use crate::error::DomainError;
use crate::reserved_names::reject_reserved_name;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Username(String);

impl Username {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let len_ok = (3..=32).contains(&raw.len());
        let starts_with_letter = raw.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        let chars_ok = raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

        if len_ok && starts_with_letter && chars_ok {
            Ok(Self(raw.to_ascii_lowercase()))
        } else {
            Err(DomainError::InvalidUsername(raw.to_string()))
        }
    }

    /// For a username being created. `parse` stays lenient so an account that predates the
    /// `artiferris-` reservation can still log in.
    pub fn parse_new(raw: &str) -> Result<Self, DomainError> {
        let username = Self::parse(raw)?;
        reject_reserved_name(raw)?;
        Ok(username)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Mirrors the frontend's own minimum, enforced again here since the API is reachable without the UI.
pub const MIN_PASSWORD_LENGTH: usize = 8;

/// No `Debug`/`Clone` — a plaintext password should be hard to log or copy around.
pub struct Password(String);

impl Password {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        if raw.chars().count() < MIN_PASSWORD_LENGTH {
            return Err(DomainError::PasswordTooShort);
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub username: Username,
    pub password_hash: String,
    pub is_super_admin: bool,
    pub is_organization_admin: bool,
    pub organization_id: Uuid,
    pub created_at: DateTime<Utc>,
    /// Tokens issued before this instant are rejected; bumped on password change, sign-out-everywhere and admin demotion so a stolen session or Docker token stops working.
    pub tokens_valid_after: DateTime<Utc>,
    /// `None` only for accounts created before this field existed.
    pub email: Option<String>,
}

impl User {
    /// Whether a token minted at `issued_at` predates the last revocation. Compared to the second: a JWT's `iat` has no sub-second precision, so a token minted in the same second as a revocation still passes.
    pub fn has_revoked_tokens_issued_at(&self, issued_at: DateTime<Utc>) -> bool {
        issued_at.timestamp() < self.tokens_valid_after.timestamp()
    }
}

#[async_trait]
pub trait UserRepositoryPort: Send + Sync {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, DomainError>;
    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, DomainError>;
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, DomainError>;
    async fn list_all(&self) -> Result<Vec<User>, DomainError>;

    /// Exactly the users whose id is in `ids` — scoped at the database level (`WHERE id = ANY($1)`),
    /// not implemented via `list_all()` and filtering in memory (M-21, B-7). For a caller that only
    /// needs a handful of users out of a potentially large table, e.g. resolving usernames for a
    /// repository's permission grants.
    async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<User>, DomainError>;
    /// `COUNT(*) WHERE organization_id = $1`, pushed to SQL rather than `list_all()` filtered and
    /// counted in application code (M-21, B-7's admin-stats follow-up).
    async fn count_by_organization(&self, organization_id: Uuid) -> Result<i64, DomainError>;

    /// Case-insensitive substring match on username, scoped to one organization, sorted by
    /// username, capped at `limit` — all pushed to SQL, not filtered in application code after a
    /// full-table `list_all()` read (M-21, B-7).
    async fn search_by_organization(&self, organization_id: Uuid, query: &str, limit: i64) -> Result<Vec<User>, DomainError>;
    /// The super-admin counterpart of `search_by_organization`: same match/sort/cap, but with no
    /// organization filter — a super-admin's search is intentionally cross-organization (mirrors
    /// `list_users`' own `is_super_admin` bypass of its organization scope).
    async fn search_all_organizations(&self, query: &str, limit: i64) -> Result<Vec<User>, DomainError>;

    async fn insert(&self, user: &User) -> Result<(), DomainError>;
    async fn delete(&self, id: Uuid) -> Result<(), DomainError>;
    /// Also bumps `tokens_valid_after` to now, revoking every token issued before this call. `audit` goes in the same transaction.
    async fn update_password(&self, id: Uuid, new_password_hash: String, audit: Option<&AuditRecord>) -> Result<(), DomainError>;
    /// A demotion also bumps `tokens_valid_after`, so a Docker token carrying the old role stops working.
    async fn set_super_admin(&self, id: Uuid, is_super_admin: bool) -> Result<(), DomainError>;

    /// `false` if this would leave zero super-admins (a no-op then). `audit` goes in the same transaction.
    async fn delete_unless_last_super_admin(&self, id: Uuid, audit: Option<&AdminAuditRecord>) -> Result<bool, DomainError>;

    /// `false` if the demotion would leave zero super-admins (a no-op then). A demotion also bumps `tokens_valid_after`.
    /// `audit` is written in the same transaction as the change.
    async fn set_super_admin_unless_last(&self, id: Uuid, is_super_admin: bool, audit: Option<&AdminAuditRecord>) -> Result<bool, DomainError>;

    /// A demotion also bumps `tokens_valid_after`. `audit` goes in the same transaction.
    async fn set_organization_admin(&self, id: Uuid, is_organization_admin: bool, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError>;
}

/// Kept apart from `UserRepositoryPort` so its many test doubles don't each need these.
#[async_trait]
pub trait UserSecurityPort: Send + Sync {
    /// Sets `tokens_valid_after` to now: every session, Docker access token and API token issued before this call stops working. `audit` goes in the same transaction.
    async fn revoke_sessions(&self, id: Uuid, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError>;

    /// Case-insensitive and within one organization, which is the scope of a verified address. Never a self-registered account.
    async fn find_by_verified_email(&self, organization_id: Uuid, email: &str) -> Result<Option<User>, DomainError>;

    /// Like `UserRepositoryPort::insert`, but the account's email counts as verified. For accounts an identity provider vouched for.
    /// `EmailTaken` if the organization already has a verified holder of the address, `UsernameTaken` if the username is gone.
    async fn insert_with_verified_email(&self, user: &User) -> Result<(), DomainError>;

    /// `false` when nothing changed, including when another account of the organization already holds the address as verified.
    async fn mark_email_verified(&self, id: Uuid) -> Result<bool, DomainError>;
}

#[async_trait]
pub trait PasswordHasherPort: Send + Sync {
    /// Runs on a blocking thread pool internally — Argon2 is too slow for an async worker.
    async fn hash(&self, plain_password: &str) -> Result<String, DomainError>;
    /// `Ok(false)` for a wrong password or an unparseable hash. `DomainError::Busy` when too much hashing is already running to take this on.
    async fn verify(&self, plain_password: &str, hash: &str) -> Result<bool, DomainError>;
}

/// What a valid token proves: whose it is, and when it was minted — lets callers reject tokens minted before a password change.
#[derive(Debug, Clone, Copy)]
pub struct VerifiedToken {
    pub user_id: Uuid,
    pub issued_at: DateTime<Utc>,
}

#[async_trait]
pub trait TokenIssuerPort: Send + Sync {
    fn issue(&self, user_id: Uuid, ttl: chrono::Duration) -> Result<String, DomainError>;
    fn verify(&self, token: &str) -> Result<VerifiedToken, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_username_cannot_use_the_reserved_prefix() {
        assert_eq!(Username::parse_new("artiferris-docker"), Err(DomainError::ReservedName("artiferris-docker".to_string())));
        assert!(Username::parse_new("florian").is_ok());
    }

    #[test]
    fn an_existing_username_with_the_reserved_prefix_can_still_log_in() {
        assert!(Username::parse("artiferris-legacy").is_ok());
    }

    fn user_valid_after(tokens_valid_after: DateTime<Utc>) -> User {
        User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::new_v4(),
            created_at: Utc::now(),
            tokens_valid_after,
            email: None,
        }
    }

    #[test]
    fn a_token_issued_before_the_revocation_is_revoked() {
        let user = user_valid_after(Utc::now());
        assert!(user.has_revoked_tokens_issued_at(Utc::now() - chrono::Duration::seconds(5)));
    }

    #[test]
    fn a_token_issued_after_the_revocation_is_not_revoked() {
        let user = user_valid_after(Utc::now() - chrono::Duration::seconds(5));
        assert!(!user.has_revoked_tokens_issued_at(Utc::now()));
    }

    #[test]
    fn accepts_a_valid_username() {
        let username = Username::parse("florian_01").unwrap();
        assert_eq!(username.as_str(), "florian_01");
    }

    #[test]
    fn rejects_a_username_shorter_than_3_chars() {
        let err = Username::parse("ab").unwrap_err();
        assert!(matches!(err, DomainError::InvalidUsername(_)));
    }

    #[test]
    fn rejects_a_username_with_invalid_characters() {
        let err = Username::parse("florian!").unwrap_err();
        assert!(matches!(err, DomainError::InvalidUsername(_)));
    }

    #[test]
    fn rejects_a_username_not_starting_with_a_letter() {
        let err = Username::parse("1florian").unwrap_err();
        assert!(matches!(err, DomainError::InvalidUsername(_)));
    }

    #[test]
    fn accepts_a_password_of_the_minimum_length() {
        let password = Password::parse("12345678").unwrap();
        assert_eq!(password.as_str(), "12345678");
    }

    #[test]
    fn rejects_a_password_below_the_minimum_length() {
        assert!(matches!(Password::parse("1234567"), Err(DomainError::PasswordTooShort)));
    }

    #[test]
    fn parsing_normalizes_case() {
        assert_eq!(Username::parse("Alice").unwrap().as_str(), "alice");
        assert_eq!(Username::parse("ALICE").unwrap().as_str(), "alice");
    }
}
