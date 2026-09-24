use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use artiferris_domain::download_stats::{DownloadCount, DownloadRecorderPort, DownloadStatsPort, RETENTION_DAYS};
use artiferris_domain::package_repository::RepositoryFormat;
use chrono::{NaiveDate, Utc};
use uuid::Uuid;

use crate::error::ApplicationError;

/// Bounds memory if downloads of an unbounded number of distinct names ever arrive between two flushes.
const MAX_BUFFERED_KEYS: usize = 200_000;

type Key = (NaiveDate, Uuid, RepositoryFormat, String);

/// Counts downloads in memory and hands them over in batches: a download never waits on the database. A crash loses at
/// most the last flush interval of counts, which the figures (indicative by design) tolerate.
#[derive(Default)]
pub struct DownloadCounterBuffer {
    counts: Mutex<HashMap<Key, i64>>,
}

impl DownloadCounterBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Empties the buffer and returns what it held.
    pub fn drain(&self) -> Vec<DownloadCount> {
        let drained = std::mem::take(&mut *self.lock());
        drained.into_iter().map(|((day, repository_id, format, name), downloads)| DownloadCount { day, repository_id, format, name, downloads }).collect()
    }

    /// Puts counts back after a failed flush, on top of whatever arrived meanwhile.
    pub fn restore(&self, counts: Vec<DownloadCount>) {
        let mut buffered = self.lock();
        for count in counts {
            *buffered.entry((count.day, count.repository_id, count.format, count.name)).or_insert(0) += count.downloads;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Key, i64>> {
        self.counts.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl DownloadRecorderPort for DownloadCounterBuffer {
    fn record(&self, repository_id: Uuid, format: RepositoryFormat, name: &str) {
        let key = (Utc::now().date_naive(), repository_id, format, name.to_string());
        let mut counts = self.lock();
        if counts.len() >= MAX_BUFFERED_KEYS && !counts.contains_key(&key) {
            return;
        }
        *counts.entry(key).or_insert(0) += 1;
    }
}

pub struct FlushDownloadCountersUseCase {
    buffer: Arc<DownloadCounterBuffer>,
    store: Arc<dyn DownloadStatsPort>,
}

impl FlushDownloadCountersUseCase {
    pub fn new(buffer: Arc<DownloadCounterBuffer>, store: Arc<dyn DownloadStatsPort>) -> Self {
        Self { buffer, store }
    }

    /// Returns how many distinct counters were written. On failure the counts go back into the buffer for the next attempt.
    pub async fn execute(&self) -> Result<usize, ApplicationError> {
        let counts = self.buffer.drain();
        if counts.is_empty() {
            return Ok(0);
        }
        if let Err(e) = self.store.add_batch(&counts).await {
            self.buffer.restore(counts);
            return Err(e.into());
        }
        Ok(counts.len())
    }
}

pub struct PruneDownloadStatsUseCase {
    store: Arc<dyn DownloadStatsPort>,
}

impl PruneDownloadStatsUseCase {
    pub fn new(store: Arc<dyn DownloadStatsPort>) -> Self {
        Self { store }
    }

    pub async fn execute(&self) -> Result<u64, ApplicationError> {
        let oldest_kept = Utc::now().date_naive() - chrono::Duration::days(RETENTION_DAYS);
        Ok(self.store.prune_before(oldest_kept).await?)
    }
}

#[cfg(test)]
mod tests {
    use artiferris_domain::error::DomainError;
    use async_trait::async_trait;

    use super::*;

    #[derive(Default)]
    struct RecordingStore {
        batches: Mutex<Vec<Vec<DownloadCount>>>,
        fail: Mutex<bool>,
        pruned_before: Mutex<Option<NaiveDate>>,
    }

    #[async_trait]
    impl DownloadStatsPort for RecordingStore {
        async fn add_batch(&self, counts: &[DownloadCount]) -> Result<(), DomainError> {
            if *self.fail.lock().unwrap() {
                return Err(DomainError::Infrastructure("db down".into()));
            }
            self.batches.lock().unwrap().push(counts.to_vec());
            Ok(())
        }
        async fn downloads_last_7_days(&self, _repository_id: Uuid, _format: RepositoryFormat, _name: &str) -> Result<i64, DomainError> {
            Ok(0)
        }
        async fn prune_before(&self, day: NaiveDate) -> Result<u64, DomainError> {
            *self.pruned_before.lock().unwrap() = Some(day);
            Ok(3)
        }
    }

    fn total(counts: &[DownloadCount]) -> i64 {
        counts.iter().map(|c| c.downloads).sum()
    }

    #[test]
    fn repeated_downloads_of_one_package_share_a_counter() {
        let buffer = DownloadCounterBuffer::new();
        let repository = Uuid::new_v4();

        for _ in 0..3 {
            buffer.record(repository, RepositoryFormat::Npm, "left-pad");
        }
        buffer.record(repository, RepositoryFormat::Npm, "right-pad");
        buffer.record(repository, RepositoryFormat::Docker, "left-pad");

        let mut drained = buffer.drain();
        drained.sort_by(|a, b| (a.format as u8, &a.name).cmp(&(b.format as u8, &b.name)));
        assert_eq!(drained.iter().map(|c| (c.name.as_str(), c.downloads)).collect::<Vec<_>>(), vec![("left-pad", 3), ("right-pad", 1), ("left-pad", 1)]);
        assert!(drained.iter().all(|c| c.day == Utc::now().date_naive() && c.repository_id == repository));
    }

    #[test]
    fn draining_empties_the_buffer() {
        let buffer = DownloadCounterBuffer::new();
        buffer.record(Uuid::new_v4(), RepositoryFormat::Npm, "a");

        assert_eq!(buffer.drain().len(), 1);
        assert!(buffer.drain().is_empty());
    }

    #[tokio::test]
    async fn a_flush_writes_the_counts_once_and_empties_the_buffer() {
        let buffer = Arc::new(DownloadCounterBuffer::new());
        let store = Arc::new(RecordingStore::default());
        let repository = Uuid::new_v4();
        buffer.record(repository, RepositoryFormat::Npm, "a");
        buffer.record(repository, RepositoryFormat::Npm, "a");
        let flush = FlushDownloadCountersUseCase::new(buffer.clone(), store.clone());

        assert_eq!(flush.execute().await.unwrap(), 1);
        assert_eq!(flush.execute().await.unwrap(), 0, "nothing new to write");

        let batches = store.batches.lock().unwrap();
        assert_eq!((batches.len(), total(&batches[0])), (1, 2));
    }

    #[tokio::test]
    async fn a_failed_flush_keeps_the_counts_for_the_next_attempt() {
        let buffer = Arc::new(DownloadCounterBuffer::new());
        let store = Arc::new(RecordingStore::default());
        let repository = Uuid::new_v4();
        buffer.record(repository, RepositoryFormat::Npm, "a");
        let flush = FlushDownloadCountersUseCase::new(buffer.clone(), store.clone());
        *store.fail.lock().unwrap() = true;

        assert!(flush.execute().await.is_err());
        buffer.record(repository, RepositoryFormat::Npm, "a");
        *store.fail.lock().unwrap() = false;
        assert_eq!(flush.execute().await.unwrap(), 1);

        assert_eq!(total(&store.batches.lock().unwrap()[0]), 2, "the lost flush's download and the new one are both counted");
    }

    #[tokio::test]
    async fn pruning_drops_days_older_than_the_retention_window() {
        let store = Arc::new(RecordingStore::default());

        let removed = PruneDownloadStatsUseCase::new(store.clone()).execute().await.unwrap();

        assert_eq!(removed, 3);
        assert_eq!(store.pruned_before.lock().unwrap().unwrap(), Utc::now().date_naive() - chrono::Duration::days(RETENTION_DAYS));
    }
}
