use async_trait::async_trait;
use uuid::Uuid;

use crate::error::DomainError;

/// A key of the limiter after hashing: the store never sees a client address.
pub type RateLimitKey = [u8; 16];

/// One counter: `count` requests seen by one instance for `key` in the window starting at `window` (a window number,
/// not a timestamp).
pub type RateLimitCount = (RateLimitKey, u64, u32);

/// Where the instances of a deployment tell each other how much of a client's budget they have spent.
#[async_trait]
pub trait RateLimitStorePort: Send + Sync {
    /// Records `instance`'s own totals. Absolute values, so a retry changes nothing.
    async fn publish(&self, instance: Uuid, counts: &[RateLimitCount]) -> Result<(), DomainError>;

    /// What every other instance has counted for `keys` in `windows`, summed per key and window.
    async fn others(&self, instance: Uuid, keys: &[RateLimitKey], windows: [u64; 2]) -> Result<Vec<RateLimitCount>, DomainError>;

    /// Drops the counters of windows before `window`; returns how many.
    async fn purge_before(&self, window: u64) -> Result<u64, DomainError>;
}
