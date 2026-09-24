use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::audit::SecurityAuditRecord;
use crate::error::DomainError;

/// `confirmed = false` means enrolled but not yet verified — only a confirmed credential is consulted at login.
#[derive(Clone, PartialEq, Eq)]
pub struct TotpCredential {
    pub user_id: Uuid,
    /// Plaintext at this layer — encryption at rest is the Postgres adapter's job.
    pub secret: String,
    pub confirmed: bool,
    /// Anti-replay: `VerifyTotpUseCase` rejects any code whose step isn't strictly greater than this.
    pub last_used_step: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for TotpCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TotpCredential")
            .field("user_id", &self.user_id)
            .field("secret", &"[redacted]")
            .field("confirmed", &self.confirmed)
            .field("last_used_step", &self.last_used_step)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[async_trait]
pub trait TotpCredentialPort: Send + Sync {
    async fn get(&self, user_id: Uuid) -> Result<Option<TotpCredential>, DomainError>;
    /// Stores an unconfirmed credential, replacing an earlier unconfirmed one. `false` when the user already has a confirmed one.
    async fn begin_enrollment(&self, user_id: Uuid, secret: &str, created_at: DateTime<Utc>) -> Result<bool, DomainError>;
    /// Confirms the attempt started at `enrollment_created_at` and records `step` as used. `false` if a newer attempt replaced it or it's already confirmed.
    async fn confirm(&self, user_id: Uuid, enrollment_created_at: DateTime<Utc>, step: i64) -> Result<bool, DomainError>;
    /// Atomically advances `last_used_step` only if `step` is newer — `false` means a concurrent call already claimed it.
    async fn set_last_used_step(&self, user_id: Uuid, step: i64) -> Result<bool, DomainError>;
    async fn delete(&self, user_id: Uuid) -> Result<(), DomainError>;
}

#[async_trait]
pub trait BackupCodePort: Send + Sync {
    /// Replaces the whole set — invalidates every prior code. Each entry is an opaque, individually
    /// salted hash string (`<salt-hex>:<digest-hex>`, see `hash_backup_code` in the application
    /// layer) — never a raw digest, and never directly comparable by equality against a plaintext.
    /// `audit` is written in the same transaction as the new set.
    async fn replace_all(&self, user_id: Uuid, code_hashes: &[String], audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError>;
    /// Takes the *plaintext* candidate code, not a hash: because each stored hash carries its own
    /// random salt, there is no single re-derived value to look up by exact match. Implementations
    /// must fetch this user's still-unused stored hashes and verify the plaintext against each
    /// (e.g. via `verify_backup_code`) until one matches.
    /// Atomic: `true` only if a matching code existed and was still unused, so two concurrent
    /// attempts on the same code can't both succeed.
    async fn try_consume(&self, user_id: Uuid, plaintext_code: &str) -> Result<bool, DomainError>;
    async fn count_unused(&self, user_id: Uuid) -> Result<i64, DomainError>;
    async fn delete_all(&self, user_id: Uuid) -> Result<(), DomainError>;
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_totp_seed() {
        let credential = TotpCredential { user_id: Uuid::new_v4(), secret: "TOTPSEEDVALUE".to_string(), confirmed: true, last_used_step: None, created_at: Utc::now() };

        assert!(!format!("{credential:?}").contains("TOTPSEEDVALUE"));
    }
}
