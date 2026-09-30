use std::sync::Arc;

use chrono::{Duration, Utc};
use artiferris_domain::docker_registry::{DockerBlobStorePort, DockerUploadSessionPort};

use crate::error::ApplicationError;

/// Longer than an upload session lives, so a push that is still finishing never loses a layer.
const UNREFERENCED_BLOB_GRACE: Duration = Duration::hours(48);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UploadSweepReport {
    pub sessions_removed: usize,
    pub blob_links_removed: usize,
    pub blobs_removed: usize,
    pub temp_files_removed: usize,
}

/// Run periodically by a background timer: reclaims upload sessions abandoned mid `docker push` (M-13), which the lazy
/// sweep in `find` never reaches, and blobs no manifest ever referenced.
pub struct SweepExpiredDockerUploadsUseCase {
    sessions: Arc<dyn DockerUploadSessionPort>,
    blobs: Arc<dyn DockerBlobStorePort>,
}

impl SweepExpiredDockerUploadsUseCase {
    pub fn new(sessions: Arc<dyn DockerUploadSessionPort>, blobs: Arc<dyn DockerBlobStorePort>) -> Self {
        Self { sessions, blobs }
    }

    pub async fn execute(&self) -> Result<UploadSweepReport, ApplicationError> {
        let sessions_removed = self.sessions.sweep_expired_uploads().await?;
        let blobs = self.blobs.sweep_unreferenced_blobs(Utc::now() - UNREFERENCED_BLOB_GRACE).await?;
        Ok(UploadSweepReport { sessions_removed, blob_links_removed: blobs.links_removed, blobs_removed: blobs.blobs_removed, temp_files_removed: blobs.temp_files_removed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::docker_test_support::{FakeDockerBlobStore, FakeUploadSessions};
    use crate::use_cases::docker_upload::StartBlobUploadUseCase;
    use uuid::Uuid;

    #[tokio::test]
    async fn sweeping_reports_the_count_removed_by_the_port() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let start = StartBlobUploadUseCase::new(sessions.clone());
        start.execute(Uuid::new_v4()).await.unwrap();
        start.execute(Uuid::new_v4()).await.unwrap();

        let use_case = SweepExpiredDockerUploadsUseCase::new(sessions, Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.unwrap();

        assert_eq!(report, UploadSweepReport::default());
    }
}
