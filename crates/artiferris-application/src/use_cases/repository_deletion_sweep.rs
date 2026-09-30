use std::sync::Arc;

use artiferris_domain::docker_registry::{Digest, DockerBlobStorePort};
use artiferris_domain::package_repository::RepositoryDeletionSweepPort;
use artiferris_domain::storage::StorageBackendPort;

use crate::error::ApplicationError;

/// Run daily by a background timer, not on any request path — hard-deletes repositories that have
/// been soft-deleted past their 30-day undo window, freeing the storage a soft delete alone never
/// reclaims (B-39). Mirrors `SweepExpiredDockerUploadsUseCase`'s shape: a thin wrapper around a single
/// port call, plus best-effort on-disk cleanup for what that call reports as reclaimed — the DB-side
/// bookkeeping (the cascade, the group-membership fix, the blob reference-count decrement) is already
/// durably committed by the time either cleanup step runs, so a failure in either is logged and
/// skipped rather than propagated (fix round 1, Findings 2 & 3): a stray on-disk file is recoverable
/// by a future cleanup pass, unlike DB state lost mid-sweep.
pub struct RepositoryDeletionSweepUseCase {
    repositories: Arc<dyn RepositoryDeletionSweepPort>,
    blobs: Arc<dyn DockerBlobStorePort>,
    storage: Arc<dyn StorageBackendPort>,
}

impl RepositoryDeletionSweepUseCase {
    pub fn new(repositories: Arc<dyn RepositoryDeletionSweepPort>, blobs: Arc<dyn DockerBlobStorePort>, storage: Arc<dyn StorageBackendPort>) -> Self {
        Self { repositories, blobs, storage }
    }

    /// Returns the number of repositories hard-deleted.
    pub async fn execute(&self) -> Result<usize, ApplicationError> {
        let result = self.repositories.hard_delete_repositories_past_grace_period().await?;

        let reclaimed_digests: Vec<Digest> = result
            .reclaimed_docker_blob_digests
            .iter()
            .filter_map(|d| match Digest::parse(d) {
                Ok(digest) => Some(digest),
                Err(e) => {
                    tracing::warn!(digest = %d, error = %e, "repository deletion sweep reported a malformed digest; skipping its file removal");
                    None
                }
            })
            .collect();
        self.blobs.remove_reclaimed_blob_files(&reclaimed_digests).await;

        for repository_id in &result.swept_repository_ids {
            if let Err(e) = self.storage.delete_repository(*repository_id).await {
                tracing::warn!(repository_id = %repository_id, error = %e, "repository deletion sweep hard-deleted a repository's rows but failed to remove its on-disk storage directory");
            }
        }

        Ok(result.repositories_removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::error::DomainError;
    use artiferris_domain::package_repository::HardDeleteSweepResult;
    use async_trait::async_trait;
    use crate::use_cases::docker_test_support::FakeDockerBlobStore;
    use crate::use_cases::npm_test_support::FakeStorage;
    use std::sync::Mutex;
    use uuid::Uuid;

    struct FakeRepositoriesSweep {
        result: Mutex<Option<HardDeleteSweepResult>>,
    }

    impl FakeRepositoriesSweep {
        fn new(result: HardDeleteSweepResult) -> Self {
            Self { result: Mutex::new(Some(result)) }
        }
    }

    #[async_trait]
    impl RepositoryDeletionSweepPort for FakeRepositoriesSweep {
        async fn hard_delete_repositories_past_grace_period(&self) -> Result<HardDeleteSweepResult, DomainError> {
            Ok(self.result.lock().unwrap().take().unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn sweeping_reports_the_count_of_repositories_removed_by_the_port() {
        let repositories = Arc::new(FakeRepositoriesSweep::new(HardDeleteSweepResult {
            repositories_removed: 3,
            swept_repository_ids: vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()],
            reclaimed_docker_blob_digests: Vec::new(),
        }));
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let storage = Arc::new(FakeStorage::new());

        let use_case = RepositoryDeletionSweepUseCase::new(repositories, blobs, storage);
        let removed = use_case.execute().await.unwrap();

        assert_eq!(removed, 3);
    }

    /// Fix round 1, Finding 3: the port itself now does the reference-count decrement (and deletes
    /// any blob row that reaches zero) transactionally, before ever returning — this use case's only
    /// remaining job for Docker blobs is best-effort on-disk file cleanup for what the port reports
    /// as already reclaimed, routed through `remove_reclaimed_blob_files` (the locked, re-checking
    /// primitive).
    #[tokio::test]
    async fn sweeping_best_effort_removes_every_docker_blob_file_the_port_reports_as_reclaimed() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let storage = Arc::new(FakeStorage::new());
        let digest = Digest::of(b"cascade-orphaned-layer").as_str().to_string();
        let repositories = Arc::new(FakeRepositoriesSweep::new(HardDeleteSweepResult {
            repositories_removed: 1,
            swept_repository_ids: vec![Uuid::new_v4()],
            reclaimed_docker_blob_digests: vec![digest.clone()],
        }));

        let use_case = RepositoryDeletionSweepUseCase::new(repositories, blobs.clone(), storage);
        use_case.execute().await.unwrap();

        assert_eq!(blobs.removed_reclaimed_digests.lock().unwrap().as_slice(), [digest]);
    }

    /// Fix round 1, Finding 2: npm tarballs (and anything else under a repository's own storage
    /// root) are on-disk, not just in the DB — the cascade delete the port runs never touches them,
    /// so the use case must explicitly reclaim each swept repository's directory afterward.
    #[tokio::test]
    async fn sweeping_removes_each_swept_repositorys_on_disk_storage_directory() {
        let swept_repository_id = Uuid::new_v4();
        let untouched_repository_id = Uuid::new_v4();
        let storage = Arc::new(FakeStorage::new());
        storage.write(swept_repository_id, "left-pad/1.0.0.tgz", b"tarball-bytes").await.unwrap();
        storage.write(untouched_repository_id, "other-pkg/1.0.0.tgz", b"other-bytes").await.unwrap();
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let repositories = Arc::new(FakeRepositoriesSweep::new(HardDeleteSweepResult {
            repositories_removed: 1,
            swept_repository_ids: vec![swept_repository_id],
            reclaimed_docker_blob_digests: Vec::new(),
        }));

        let use_case = RepositoryDeletionSweepUseCase::new(repositories, blobs, storage.clone());
        use_case.execute().await.unwrap();

        assert!(storage.read(swept_repository_id, "left-pad/1.0.0.tgz").await.is_err(), "the swept repository's on-disk tarball must be gone");
        assert!(
            storage.read(untouched_repository_id, "other-pkg/1.0.0.tgz").await.is_ok(),
            "a repository the port did NOT report as swept must be left completely untouched"
        );
    }
}
