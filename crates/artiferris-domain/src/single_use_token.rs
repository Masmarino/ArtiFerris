use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::DomainError;

/// Where the instances of a deployment remember which short-lived tokens are already spent.
#[async_trait]
pub trait SingleUseTokenStorePort: Send + Sync {
    /// `true` the first time `digest` is seen, `false` for a replay. The entry is kept for at least `lifetime`, which
    /// must outlast the token. A user's oldest entries beyond `max_per_user` are forgotten, so a script cannot fill the
    /// store.
    async fn consume(&self, user_id: Uuid, digest: [u8; 32], lifetime: Duration, max_per_user: usize) -> Result<bool, DomainError>;
}
