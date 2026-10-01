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
    /// Replaces the whole set, invalidating every prior code. Each entry is an opaque salted hash
    /// (`<salt-hex>:<digest-hex>`), never comparable to a plaintext. `audit` goes in the same transaction.
    async fn replace_all(&self, user_id: Uuid, code_hashes: &[String], audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError>;
    /// Takes the plaintext code, not a hash: each stored hash has its own salt, so implementations fetch the user's
    /// unused hashes and verify against each. Atomic: `true` only if a matching unused code existed, so two concurrent
    /// attempts cannot both succeed.
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
