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

/// Which half of a passkey flow a stored ceremony belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeremonyKind {
    Registration,
    Authentication,
}

/// Where the instances of a deployment keep a passkey ceremony between its `start` and its `finish`, which may be
/// answered by different instances. The state is an opaque blob (the application layer's serialized webauthn state).
#[async_trait]
pub trait PasskeyCeremonyStorePort: Send + Sync {
    /// Stores a ceremony for `ttl` and returns its challenge id. A user's oldest ceremonies beyond `max_per_user`, then the
    /// oldest of anyone beyond `max_total`, are dropped to make room, so unauthenticated starts cannot grow the store
    /// without bound.
    async fn insert(&self, user_id: Uuid, kind: CeremonyKind, state: Vec<u8>, ttl: chrono::Duration, max_per_user: usize, max_total: usize) -> Result<Uuid, DomainError>;

    /// Single use, like a nonce: the ceremony is removed, whoever asks, and handed back only to its own user.
    async fn take(&self, challenge_id: Uuid, user_id: Uuid) -> Result<Option<(CeremonyKind, Vec<u8>)>, DomainError>;
}
