use std::time::Duration;

use async_trait::async_trait;

use crate::error::DomainError;

/// One count of failed attempts: the key it was counted under, the threshold and the window it is judged against.
pub type AttemptBudget<'a> = (&'a str, usize, Duration);

/// A key that has reached its threshold, as an administrator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedKey {
    pub key: String,
    pub remaining_seconds: u64,
}

/// Where the instances of a deployment count failed logins, so that an attacker gets one budget however many
/// instances answer, and so that an administrator's unlock reaches all of them. A key's failures are judged against
/// the longer of the window they were counted with and the one a later caller asks for.
#[async_trait]
pub trait LoginAttemptStorePort: Send + Sync {
    /// Counts one attempt against every budget in one step, or counts nothing and returns `false` if one is full.
    async fn reserve_all(&self, budgets: &[AttemptBudget<'_>]) -> Result<bool, DomainError>;

    async fn is_throttled(&self, key: &str, max_attempts: usize, window: Duration) -> Result<bool, DomainError>;

    /// Counts a failure unless the key is already past its threshold.
    async fn record_failure(&self, key: &str, max_attempts: usize, window: Duration) -> Result<(), DomainError>;

    /// Gives back the most recent attempt counted under `key`.
    async fn release(&self, key: &str) -> Result<(), DomainError>;

    async fn clear(&self, key: &str) -> Result<(), DomainError>;

    /// Wipes every login key that counts `username`: the shared one and each organization's own.
    async fn clear_username(&self, username: &str) -> Result<(), DomainError>;

    /// The login keys currently at `max_attempts` or more, longest lock first, at most `limit` of them.
    async fn blocked(&self, max_attempts: usize, window: Duration, limit: usize) -> Result<Vec<BlockedKey>, DomainError>;
}
