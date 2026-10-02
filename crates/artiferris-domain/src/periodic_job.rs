use std::time::Duration;

use async_trait::async_trait;

use crate::error::DomainError;

/// Lets the instances of a deployment take turns at a job that must run once per interval, whatever their number.
#[async_trait]
pub trait PeriodicJobPort: Send + Sync {
    /// `true` for the one caller that may run the job now: nobody started it within `interval`. The start is recorded
    /// in the same step, so of any number of instances asking at once only one gets `true`. A job that dies mid-run
    /// is simply started again after the interval, so jobs must be safe to repeat.
    async fn claim(&self, name: &str, interval: Duration) -> Result<bool, DomainError>;
}
