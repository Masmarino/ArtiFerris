use std::sync::Arc;
use std::time::Duration as StdDuration;

use artiferris_domain::audit::AuditRetentionPort;
use chrono::{Duration, Utc};
use rand::RngExt;

use crate::error::ApplicationError;

pub const DEFAULT_AUDIT_RETENTION_DAYS: i64 = 365;
/// A hundred years; a huge day count would overflow the cutoff date.
pub const MAX_AUDIT_RETENTION_DAYS: i64 = 36_500;
const BATCH_SIZE: i64 = 5_000;

/// Shortly after startup, so an instance redeployed more often than daily still prunes.
const FIRST_SWEEP_DELAY: StdDuration = StdDuration::from_secs(5 * 60);
const FIRST_SWEEP_JITTER: StdDuration = StdDuration::from_secs(60);
pub const SWEEP_INTERVAL: StdDuration = StdDuration::from_secs(24 * 60 * 60);

/// `AUDIT_RETENTION_DAYS`: unset or empty keeps a year, `0` keeps everything, anything else is a day count up to 36500. A typo fails startup.
pub fn parse_audit_retention_days(raw: Option<&str>) -> Result<Option<i64>, String> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(Some(DEFAULT_AUDIT_RETENTION_DAYS)),
        Some(value) => match value.parse::<u32>() {
            Ok(0) => Ok(None),
            Ok(days) if i64::from(days) <= MAX_AUDIT_RETENTION_DAYS => Ok(Some(i64::from(days))),
            Ok(_) => Err(format!("invalid AUDIT_RETENTION_DAYS: {value} is more than the {MAX_AUDIT_RETENTION_DAYS} days allowed (use 0 to keep everything)")),
            Err(_) => Err(format!("invalid AUDIT_RETENTION_DAYS: {value:?} is not a number of days")),
        },
    }
}

/// Jittered so replicas that start together don't sweep together.
pub fn first_sweep_delay() -> StdDuration {
    FIRST_SWEEP_DELAY + FIRST_SWEEP_JITTER.mul_f64(rand::rng().random::<f64>())
}

pub struct PruneAuditEventsUseCase {
    store: Arc<dyn AuditRetentionPort>,
    retention_days: Option<i64>,
}

impl PruneAuditEventsUseCase {
    pub fn new(store: Arc<dyn AuditRetentionPort>, retention_days: Option<i64>) -> Self {
        Self { store, retention_days }
    }

    /// Sweeps after `first_delay`, then every `interval`.
    pub async fn run_forever(&self, first_delay: StdDuration, interval: StdDuration) {
        tokio::time::sleep(first_delay).await;
        loop {
            match self.execute().await {
                Ok(removed) if removed > 0 => tracing::info!(removed, "audit sweep removed events past the retention window"),
                Ok(_) => {}
                Err(e) => tracing::warn!("audit sweep failed: {e}"),
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Deletes in batches so a first run over years of history never holds one long transaction.
    pub async fn execute(&self) -> Result<u64, ApplicationError> {
        let Some(days) = self.retention_days else {
            return Ok(0);
        };
        let cutoff = Utc::now() - Duration::days(days);
        let mut total = 0;
        loop {
            let removed = self.store.delete_audit_events_before(cutoff, BATCH_SIZE).await?;
            total += removed;
            if removed < BATCH_SIZE as u64 {
                return Ok(total);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use artiferris_domain::error::EventStoreError;
    use async_trait::async_trait;
    use chrono::DateTime;

    use super::*;

    struct Store {
        remaining: Mutex<u64>,
        calls: Mutex<Vec<(DateTime<Utc>, i64)>>,
    }

    #[async_trait]
    impl AuditRetentionPort for Store {
        async fn delete_audit_events_before(&self, cutoff: DateTime<Utc>, limit: i64) -> Result<u64, EventStoreError> {
            self.calls.lock().unwrap().push((cutoff, limit));
            let mut remaining = self.remaining.lock().unwrap();
            let removed = (*remaining).min(limit as u64);
            *remaining -= removed;
            Ok(removed)
        }
    }

    fn store(remaining: u64) -> Arc<Store> {
        Arc::new(Store { remaining: Mutex::new(remaining), calls: Mutex::new(Vec::new()) })
    }

    #[test]
    fn unset_or_empty_keeps_a_year() {
        assert_eq!(parse_audit_retention_days(None), Ok(Some(365)));
        assert_eq!(parse_audit_retention_days(Some("  ")), Ok(Some(365)));
    }

    #[test]
    fn zero_keeps_everything_and_a_number_is_taken_as_days() {
        assert_eq!(parse_audit_retention_days(Some("0")), Ok(None));
        assert_eq!(parse_audit_retention_days(Some("90")), Ok(Some(90)));
    }

    #[test]
    fn the_retention_is_capped_at_a_hundred_years_instead_of_overflowing_the_cutoff() {
        assert_eq!(parse_audit_retention_days(Some("36500")), Ok(Some(36_500)));
        assert!(parse_audit_retention_days(Some("36501")).is_err());
        assert!(parse_audit_retention_days(Some("999999999")).is_err());
        assert!(parse_audit_retention_days(Some("4294967295")).is_err());
    }

    #[test]
    fn the_first_sweep_comes_within_minutes_of_startup_not_a_day_later() {
        for _ in 0..50 {
            let delay = first_sweep_delay();
            assert!(delay >= StdDuration::from_secs(5 * 60) && delay <= StdDuration::from_secs(6 * 60), "{delay:?}");
        }
    }

    #[tokio::test]
    async fn the_schedule_sweeps_once_after_the_first_delay_and_again_every_interval() {
        let store = store(0);
        let use_case = Arc::new(PruneAuditEventsUseCase::new(store.clone(), Some(365)));
        let task = tokio::spawn({
            let use_case = use_case.clone();
            async move { use_case.run_forever(StdDuration::from_millis(20), StdDuration::from_millis(20)).await }
        });

        assert!(store.calls.lock().unwrap().is_empty(), "nothing before the first delay");
        for _ in 0..200 {
            if store.calls.lock().unwrap().len() >= 3 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
        task.abort();

        assert!(store.calls.lock().unwrap().len() >= 3, "a first sweep and then one per interval");
    }

    #[test]
    fn a_typo_or_negative_value_is_an_error() {
        assert!(parse_audit_retention_days(Some("12 months")).is_err());
        assert!(parse_audit_retention_days(Some("-5")).is_err());
    }

    #[tokio::test]
    async fn events_older_than_the_retention_window_are_deleted_in_batches_until_none_are_left() {
        let store = store(12_000);
        let removed = PruneAuditEventsUseCase::new(store.clone(), Some(365)).execute().await.unwrap();

        assert_eq!(removed, 12_000);
        let calls = store.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        let expected = Utc::now() - Duration::days(365);
        assert!((calls[0].0 - expected).num_seconds().abs() < 5);
    }

    #[tokio::test]
    async fn a_disabled_retention_touches_nothing() {
        let store = store(10);
        assert_eq!(PruneAuditEventsUseCase::new(store.clone(), None).execute().await.unwrap(), 0);
        assert!(store.calls.lock().unwrap().is_empty());
    }
}
