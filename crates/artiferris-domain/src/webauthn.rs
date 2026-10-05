use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::audit::SecurityAuditRecord;
use crate::error::DomainError;

/// `passkey_data` is an opaque serialized blob (the application layer's `Passkey` type) — the domain layer only persists it. Holds only the credential's public key, so it needs no encryption at rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebauthnCredential {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub passkey_data: Vec<u8>,
    pub created_at: DateTime<Utc>,
    /// When it last signed its owner in. `None` until its first use.
    pub last_used_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait WebauthnCredentialPort: Send + Sync {
    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<WebauthnCredential>, DomainError>;
    /// Batched, for a list of accounts: which of these users have at least one passkey. One lookup per user by
    /// default; a store overrides it with a single query.
    async fn holders_among(&self, user_ids: &[Uuid]) -> Result<std::collections::HashSet<Uuid>, DomainError> {
        let mut holders = std::collections::HashSet::new();
        for &user_id in user_ids {
            if !self.list_for_user(user_id).await?.is_empty() {
                holders.insert(user_id);
            }
        }
        Ok(holders)
    }
    /// `audit` is written in the same transaction as the credential.
    async fn insert(&self, credential: &WebauthnCredential, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError>;
    async fn update_passkey_data(&self, id: Uuid, passkey_data: Vec<u8>) -> Result<(), DomainError>;
    /// Records a successful sign-in with this passkey.
    async fn mark_used(&self, id: Uuid, at: DateTime<Utc>) -> Result<(), DomainError>;
    /// Scoped to `user_id` so one user can't delete another's credential. `audit` goes in the same transaction.
    async fn delete(&self, id: Uuid, user_id: Uuid, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError>;
    async fn count_for_user(&self, user_id: Uuid) -> Result<i64, DomainError>;
    /// Every passkey of the account, for an administrator resetting its second factors. One delete per passkey by
    /// default; a store overrides it with a single statement.
    async fn delete_all_for_user(&self, user_id: Uuid) -> Result<(), DomainError> {
        for credential in self.list_for_user(user_id).await? {
            self.delete(credential.id, user_id, None).await?;
        }
        Ok(())
    }
}
