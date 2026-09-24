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
}

#[async_trait]
pub trait WebauthnCredentialPort: Send + Sync {
    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<WebauthnCredential>, DomainError>;
    /// `audit` is written in the same transaction as the credential.
    async fn insert(&self, credential: &WebauthnCredential, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError>;
    async fn update_passkey_data(&self, id: Uuid, passkey_data: Vec<u8>) -> Result<(), DomainError>;
    /// Scoped to `user_id` so one user can't delete another's credential. `audit` goes in the same transaction.
    async fn delete(&self, id: Uuid, user_id: Uuid, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError>;
    async fn count_for_user(&self, user_id: Uuid) -> Result<i64, DomainError>;
}
