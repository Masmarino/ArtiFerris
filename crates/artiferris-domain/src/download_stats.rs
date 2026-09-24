use async_trait::async_trait;
use chrono::NaiveDate;
use uuid::Uuid;

use crate::error::DomainError;
use crate::package_repository::RepositoryFormat;

/// "This week" is today and the six days before it, in UTC.
pub const RECENT_WINDOW_DAYS: i64 = 7;

/// Counts older than this are dropped; nothing shown to a visitor ever reaches that far back.
pub const RETENTION_DAYS: i64 = 396;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadCount {
    pub day: NaiveDate,
    pub repository_id: Uuid,
    pub format: RepositoryFormat,
    /// The npm package name or the Docker image name.
    pub name: String,
    pub downloads: i64,
}

/// Called on the download path, so it must never block or fail: implementations only note the download.
pub trait DownloadRecorderPort: Send + Sync {
    fn record(&self, repository_id: Uuid, format: RepositoryFormat, name: &str);
}

/// Drops every download, for the places (and tests) that don't count them.
pub struct NoopDownloadRecorder;

impl DownloadRecorderPort for NoopDownloadRecorder {
    fn record(&self, _repository_id: Uuid, _format: RepositoryFormat, _name: &str) {}
}

#[async_trait]
pub trait DownloadStatsPort: Send + Sync {
    /// Adds to the existing counts, so several instances can flush into the same table.
    async fn add_batch(&self, counts: &[DownloadCount]) -> Result<(), DomainError>;
    async fn downloads_last_7_days(&self, repository_id: Uuid, format: RepositoryFormat, name: &str) -> Result<i64, DomainError>;
    /// Removes every day strictly before `day`; returns how many rows went.
    async fn prune_before(&self, day: NaiveDate) -> Result<u64, DomainError>;
}
