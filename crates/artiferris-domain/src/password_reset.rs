use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::DomainError;

/// A link an administrator issued so someone can choose a new password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordReset {
    pub user_id: Uuid,
    /// SHA-256 hex digest of the mailed token: the token itself is never stored.
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

#[async_trait]
pub trait PasswordResetPort: Send + Sync {
    /// Replaces the account's link, if any, so a second reset voids the first instead of leaving two that work.
    async fn replace(&self, reset: &PasswordReset) -> Result<(), DomainError>;
    /// Single use: deletes and returns the link for this token if it hasn't expired. Of several parallel calls, only
    /// one gets `Some`.
    async fn consume(&self, token_hash: &str) -> Result<Option<PasswordReset>, DomainError>;
    /// Puts back a consumed link whose password could not be written, but only while the account has no newer link,
    /// so it never revives an old link over one issued in the meantime. `true` if it was put back.
    async fn restore(&self, reset: &PasswordReset) -> Result<bool, DomainError>;
}
