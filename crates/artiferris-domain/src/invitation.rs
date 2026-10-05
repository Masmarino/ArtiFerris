use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::audit::AdminAuditRecord;
use crate::error::DomainError;

/// A row's presence means the account hasn't activated yet — it has an unusable placeholder password hash. Deleted once activation succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInvitation {
    pub user_id: Uuid,
    /// SHA-256 hex digest of the mailed token — the raw token is never stored.
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

#[async_trait]
pub trait UserInvitationPort: Send + Sync {
    /// `audit` is written in the same transaction as the invitation.
    async fn upsert(&self, invitation: &UserInvitation, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError>;
    async fn find_by_token_hash(&self, token_hash: &str) -> Result<Option<UserInvitation>, DomainError>;
    async fn find_by_user_id(&self, user_id: Uuid) -> Result<Option<UserInvitation>, DomainError>;
    /// Batched form of `find_by_user_id`: which of these users have a pending invitation.
    async fn list_pending_user_ids(&self, user_ids: &[Uuid]) -> Result<HashSet<Uuid>, DomainError>;
    /// Batched: when each of these users' invitation expires, for those who have one (expired or not). One lookup per
    /// user by default; a store overrides it with a single query.
    async fn invitation_expiries(&self, user_ids: &[Uuid]) -> Result<std::collections::HashMap<Uuid, chrono::DateTime<chrono::Utc>>, DomainError> {
        let mut expiries = std::collections::HashMap::new();
        for &user_id in user_ids {
            if let Some(invitation) = self.find_by_user_id(user_id).await? {
                expiries.insert(user_id, invitation.expires_at);
            }
        }
        Ok(expiries)
    }
    async fn delete(&self, user_id: Uuid) -> Result<(), DomainError>;
    /// Single use: deletes and returns the invitation for this token if it hasn't expired. Of several parallel calls, only one gets `Some`.
    async fn redeem(&self, token_hash: &str) -> Result<Option<UserInvitation>, DomainError>;
}
