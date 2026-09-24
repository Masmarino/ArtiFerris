use std::sync::Arc;

use artiferris_domain::docker_registry::{ByteStream, Digest, DockerBlobStorePort, DockerUploadSession, DockerUploadSessionPort};
use artiferris_domain::error::DomainError;
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, RepositoryQuotaLockPort};
use uuid::Uuid;

use crate::error::ApplicationError;

/// Fetches the session and checks it belongs to `repository_id`, treating a foreign session the same as a nonexistent one.
async fn find_own_session(sessions: &dyn DockerUploadSessionPort, session_id: Uuid, repository_id: Uuid) -> Result<DockerUploadSession, ApplicationError> {
    let Some(session) = sessions.find(session_id).await? else {
        return Err(ApplicationError::DockerUploadSessionNotFound);
    };
    if session.package_repository_id != repository_id {
        return Err(ApplicationError::DockerUploadSessionNotFound);
    }
    Ok(session)
}

fn upload_error(error: DomainError) -> ApplicationError {
    match error {
        DomainError::UploadSessionNotFound => ApplicationError::DockerUploadSessionNotFound,
        DomainError::UploadInProgress => ApplicationError::DockerUploadInProgress,
        DomainError::TooManyUploads => ApplicationError::DockerTooManyUploads,
        DomainError::UploadTooLarge => ApplicationError::DockerUploadTooLarge,
        DomainError::ChunkOffsetMismatch { expected, got } => ApplicationError::DockerChunkOffsetMismatch { expected, got },
        other => other.into(),
    }
}

pub struct StartBlobUploadUseCase {
    sessions: Arc<dyn DockerUploadSessionPort>,
}

impl StartBlobUploadUseCase {
    pub fn new(sessions: Arc<dyn DockerUploadSessionPort>) -> Self {
        Self { sessions }
    }

    pub async fn execute(&self, repository_id: Uuid) -> Result<DockerUploadSession, ApplicationError> {
        self.sessions.create(repository_id).await.map_err(upload_error)
    }
}

pub struct PatchBlobUploadUseCase {
    sessions: Arc<dyn DockerUploadSessionPort>,
    blobs: Arc<dyn DockerBlobStorePort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    quota_lock: Arc<dyn RepositoryQuotaLockPort>,
}

impl PatchBlobUploadUseCase {
    pub fn new(
        sessions: Arc<dyn DockerUploadSessionPort>,
        blobs: Arc<dyn DockerBlobStorePort>,
        repositories: Arc<dyn PackageRepositoryQueryPort>,
        quota_lock: Arc<dyn RepositoryQuotaLockPort>,
    ) -> Self {
        Self { sessions, blobs, repositories, quota_lock }
    }

    /// `execute_stream` for a chunk that is already in memory.
    pub async fn execute(&self, session_id: Uuid, repository_id: Uuid, chunk: &[u8], expected_start: Option<i64>) -> Result<i64, ApplicationError> {
        let body: ByteStream = Box::pin(futures::stream::once(std::future::ready(Ok(bytes::Bytes::copy_from_slice(chunk)))));
        self.execute_stream(session_id, repository_id, body, expected_start, u64::MAX).await
    }

    /// `expected_start`, from a client-sent `Content-Range`, must match the offset at write time. At most `request_limit`
    /// bytes, and no more than the quota has left after referenced blobs, unreferenced uploads and every session's staged bytes.
    /// Sessions streaming at the same moment can each see the whole room, so the total is checked again once the chunk has
    /// landed and the chunk is taken back if it doesn't fit.
    pub async fn execute_stream(&self, session_id: Uuid, repository_id: Uuid, body: ByteStream, expected_start: Option<i64>, request_limit: u64) -> Result<i64, ApplicationError> {
        let session = find_own_session(&*self.sessions, session_id, repository_id).await?;
        let quota = self.repositories.find_by_id(repository_id).await?.and_then(|repo| repo.quota_bytes).map(|quota| quota.max(0) as u64);
        let mut limit = request_limit;
        let mut bounded_by_quota = false;
        if let Some(quota) = quota {
            let used = self.blobs.used_bytes_for_repository(repository_id).await?;
            let staged = self.sessions.staged_bytes_for_repository(repository_id).await?;
            let room = quota.saturating_sub(used.saturating_add(staged));
            if room < limit {
                limit = room;
                bounded_by_quota = true;
            }
        }
        let total = match self.sessions.append_stream(session_id, body, expected_start, limit).await {
            Err(DomainError::UploadTooLarge) if bounded_by_quota => return Err(ApplicationError::StorageQuotaExceeded),
            other => other.map_err(upload_error)?,
        };
        if let Some(quota) = quota {
            self.keep_within_quota(repository_id, session_id, session.bytes_received, total, quota).await?;
        }
        Ok(total)
    }

    /// Serialized per repository, so of two chunks that don't both fit, the second to get here is the one taken back.
    async fn keep_within_quota(&self, repository_id: Uuid, session_id: Uuid, before: i64, after: i64, quota: u64) -> Result<(), ApplicationError> {
        let _lock = self.quota_lock.acquire_repository_lock(repository_id).await?;
        let used = self.blobs.used_bytes_for_repository(repository_id).await?;
        let staged = self.sessions.staged_bytes_for_repository(repository_id).await?;
        if used.saturating_add(staged) > quota {
            self.sessions.rewind(session_id, after, before).await.map_err(upload_error)?;
            return Err(ApplicationError::StorageQuotaExceeded);
        }
        Ok(())
    }

    /// Frees the session and its staged bytes.
    pub async fn cancel(&self, session_id: Uuid, repository_id: Uuid) -> Result<(), ApplicationError> {
        find_own_session(&*self.sessions, session_id, repository_id).await?;
        self.sessions.delete(session_id).await.map_err(upload_error)
    }
}

pub struct CompleteBlobUploadUseCase {
    sessions: Arc<dyn DockerUploadSessionPort>,
    blobs: Arc<dyn DockerBlobStorePort>,
}

impl CompleteBlobUploadUseCase {
    pub fn new(sessions: Arc<dyn DockerUploadSessionPort>, blobs: Arc<dyn DockerBlobStorePort>) -> Self {
        Self { sessions, blobs }
    }

    /// Verifies staged bytes hash to `expected_digest` before storing — never trusts the client's claim. Doesn't increment the blob's ref count; that's `PutManifestUseCase`'s job.
    /// The session is sealed first, so no chunk can land between the hash and the adoption. A mismatch discards it.
    pub async fn execute(&self, session_id: Uuid, repository_id: Uuid, expected_digest: &Digest) -> Result<(), ApplicationError> {
        find_own_session(&*self.sessions, session_id, repository_id).await?;
        let session = self.sessions.seal(session_id).await.map_err(upload_error)?;
        let (computed, size_bytes) = self.sessions.hash_staged_file(session_id).await.map_err(upload_error)?;
        if &computed != expected_digest {
            let _ = self.sessions.delete(session_id).await;
            return Err(ApplicationError::DockerDigestMismatch {
                expected: expected_digest.as_str().to_string(),
                computed: computed.as_str().to_string(),
            });
        }
        // The staged file already holds these exact bytes — adopt it in place, no second full copy.
        self.blobs.adopt_staged_file(&computed, &session.staging_path, size_bytes).await?;
        self.blobs.link_to_repository(session.package_repository_id, &computed).await?;
        self.sessions.delete(session_id).await?;
        Ok(())
    }
}

/// A single-POST upload, through the same session, quota and verification path as a chunked one.
pub struct MonolithicBlobUploadUseCase {
    start: Arc<StartBlobUploadUseCase>,
    patch: Arc<PatchBlobUploadUseCase>,
    complete: Arc<CompleteBlobUploadUseCase>,
    sessions: Arc<dyn DockerUploadSessionPort>,
}

impl MonolithicBlobUploadUseCase {
    pub fn new(
        start: Arc<StartBlobUploadUseCase>,
        patch: Arc<PatchBlobUploadUseCase>,
        complete: Arc<CompleteBlobUploadUseCase>,
        sessions: Arc<dyn DockerUploadSessionPort>,
    ) -> Self {
        Self { start, patch, complete, sessions }
    }

    pub async fn execute(&self, repository_id: Uuid, expected_digest: &Digest, body: ByteStream, request_limit: u64) -> Result<(), ApplicationError> {
        let session = self.start.execute(repository_id).await?;
        let outcome = async {
            self.patch.execute_stream(session.id, repository_id, body, None, request_limit).await?;
            self.complete.execute(session.id, repository_id, expected_digest).await
        }
        .await;
        if outcome.is_err() {
            let _ = self.sessions.delete(session.id).await;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::docker_test_support::{FakeDockerBlobStore, FakeRepositories, FakeUploadSessions};
    use crate::use_cases::npm_test_support::FakeRepositoryQuotaLock;
    use artiferris_domain::docker_registry::Digest;

    /// Unlimited quota (`None`) — most tests here aren't exercising quota behavior at all.
    fn repo_with_quota(id: Uuid, quota_bytes: Option<i64>) -> artiferris_domain::package_repository::PackageRepositorySummary {
        artiferris_domain::package_repository::PackageRepositorySummary {
            id,
            organization_id: Uuid::new_v4(),
            name: "myrepo".to_string(),
            format: artiferris_domain::package_repository::RepositoryFormat::Docker,
            repo_type: artiferris_domain::package_repository::RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes,
            retention_keep_last_n: None,
            is_public: false,
        }
    }

    /// `PatchBlobUploadUseCase` with no repository registered — `find_by_id` returns `None`, so the
    /// quota check is skipped, same as `repo_with_quota(id, None)` would give.
    fn patch_use_case(sessions: Arc<FakeUploadSessions>) -> PatchBlobUploadUseCase {
        PatchBlobUploadUseCase::new(sessions, Arc::new(FakeDockerBlobStore::new()), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()))
    }

    #[tokio::test]
    async fn starting_an_upload_creates_a_session() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let use_case = StartBlobUploadUseCase::new(sessions);
        let repository_id = Uuid::new_v4();
        let session = use_case.execute(repository_id).await.unwrap();
        assert_eq!(session.bytes_received, 0);
    }

    #[tokio::test]
    async fn patching_appends_bytes_and_reports_the_new_total() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let repository_id = Uuid::new_v4();
        let start = StartBlobUploadUseCase::new(sessions.clone());
        let session = start.execute(repository_id).await.unwrap();

        let use_case = patch_use_case(sessions);
        let total = use_case.execute(session.id, repository_id, b"hello ", None).await.unwrap();
        assert_eq!(total, 6);
        let total = use_case.execute(session.id, repository_id, b"world", Some(6)).await.unwrap();
        assert_eq!(total, 11);
    }

    #[tokio::test]
    async fn patching_with_a_content_range_start_that_does_not_match_the_current_offset_is_rejected() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let repository_id = Uuid::new_v4();
        let start = StartBlobUploadUseCase::new(sessions.clone());
        let session = start.execute(repository_id).await.unwrap();
        let use_case = patch_use_case(sessions);
        use_case.execute(session.id, repository_id, b"hello ", None).await.unwrap();

        // A retried or reordered chunk claiming to start at 0 when 6 bytes are already staged.
        let err = use_case.execute(session.id, repository_id, b"world", Some(0)).await.unwrap_err();

        assert!(matches!(err, ApplicationError::DockerChunkOffsetMismatch { expected: 6, got: 0 }), "got {err:?}");
    }

    #[tokio::test]
    async fn patching_a_session_through_a_different_repositorys_authorization_is_rejected() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let start = StartBlobUploadUseCase::new(sessions.clone());
        let session = start.execute(Uuid::new_v4()).await.unwrap();
        let use_case = patch_use_case(sessions);

        let err = use_case.execute(session.id, Uuid::new_v4(), b"hello", None).await.unwrap_err();

        assert!(matches!(err, ApplicationError::DockerUploadSessionNotFound), "got {err:?}");
    }

    #[tokio::test]
    async fn patching_a_chunk_that_would_exceed_the_repositorys_quota_is_rejected() {
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(repo_with_quota(repository_id, Some(10)));
        let sessions = Arc::new(FakeUploadSessions::new());
        let start = StartBlobUploadUseCase::new(sessions.clone());
        let session = start.execute(repository_id).await.unwrap();
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let use_case = PatchBlobUploadUseCase::new(sessions, blobs, repositories, Arc::new(FakeRepositoryQuotaLock::new()));

        let err = use_case.execute(session.id, repository_id, &[0u8; 20], None).await.unwrap_err();

        assert!(matches!(err, ApplicationError::StorageQuotaExceeded), "got {err:?}");
    }

    #[tokio::test]
    async fn completing_verifies_the_digest_and_stores_the_blob() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let repository_id = Uuid::new_v4();
        let start = StartBlobUploadUseCase::new(sessions.clone());
        let session = start.execute(repository_id).await.unwrap();
        let patch = patch_use_case(sessions.clone());
        patch.execute(session.id, repository_id, b"blob-bytes", None).await.unwrap();

        let real_digest = Digest::of(b"blob-bytes");
        let use_case = CompleteBlobUploadUseCase::new(sessions, blobs.clone());
        use_case.execute(session.id, repository_id, &real_digest).await.unwrap();

        assert!(blobs.exists(&real_digest).await.unwrap());
    }

    #[tokio::test]
    async fn completing_with_a_mismatched_digest_is_rejected() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let repository_id = Uuid::new_v4();
        let start = StartBlobUploadUseCase::new(sessions.clone());
        let session = start.execute(repository_id).await.unwrap();
        let patch = patch_use_case(sessions.clone());
        patch.execute(session.id, repository_id, b"blob-bytes", None).await.unwrap();

        let wrong_digest = Digest::of(b"different-bytes");
        let use_case = CompleteBlobUploadUseCase::new(sessions, blobs.clone());
        let result = use_case.execute(session.id, repository_id, &wrong_digest).await;
        assert!(matches!(result, Err(ApplicationError::DockerDigestMismatch { .. })));
        assert!(!blobs.exists(&wrong_digest).await.unwrap());
    }

    #[tokio::test]
    async fn completing_a_session_through_a_different_repositorys_authorization_is_rejected() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let victim_repository_id = Uuid::new_v4();
        let start = StartBlobUploadUseCase::new(sessions.clone());
        let session = start.execute(victim_repository_id).await.unwrap();
        let patch = patch_use_case(sessions.clone());
        patch.execute(session.id, victim_repository_id, b"blob-bytes", None).await.unwrap();

        let digest = Digest::of(b"blob-bytes");
        let use_case = CompleteBlobUploadUseCase::new(sessions, blobs.clone());
        let attacker_repository_id = Uuid::new_v4();
        let err = use_case.execute(session.id, attacker_repository_id, &digest).await.unwrap_err();

        assert!(matches!(err, ApplicationError::DockerUploadSessionNotFound), "got {err:?}");
        assert!(!blobs.exists(&digest).await.unwrap(), "the blob must not be linked anywhere when the repository check fails");
    }

    fn monolithic(sessions: Arc<FakeUploadSessions>, blobs: Arc<FakeDockerBlobStore>, repositories: Arc<FakeRepositories>) -> MonolithicBlobUploadUseCase {
        MonolithicBlobUploadUseCase::new(
            Arc::new(StartBlobUploadUseCase::new(sessions.clone())),
            Arc::new(PatchBlobUploadUseCase::new(sessions.clone(), blobs.clone(), repositories, Arc::new(FakeRepositoryQuotaLock::new()))),
            Arc::new(CompleteBlobUploadUseCase::new(sessions.clone(), blobs)),
            sessions,
        )
    }

    fn body(bytes: &[u8]) -> ByteStream {
        Box::pin(futures::stream::once(std::future::ready(Ok(bytes::Bytes::copy_from_slice(bytes)))))
    }

    #[tokio::test]
    async fn monolithic_upload_verifies_and_stores_in_one_call() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let sessions = Arc::new(FakeUploadSessions::new());
        let use_case = monolithic(sessions.clone(), blobs.clone(), Arc::new(FakeRepositories::new()));
        let digest = Digest::of(b"monolithic-bytes");
        use_case.execute(Uuid::new_v4(), &digest, body(b"monolithic-bytes"), u64::MAX).await.unwrap();
        assert!(blobs.exists(&digest).await.unwrap());
        assert!(sessions.sessions.lock().unwrap().is_empty(), "no session is left behind");
    }

    #[tokio::test]
    async fn a_monolithic_upload_over_the_quota_is_rejected_and_leaves_nothing_behind() {
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(repo_with_quota(repository_id, Some(10)));
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let sessions = Arc::new(FakeUploadSessions::new());
        let use_case = monolithic(sessions.clone(), blobs.clone(), repositories);
        let digest = Digest::of(&[7u8; 20]);

        let err = use_case.execute(repository_id, &digest, body(&[7u8; 20]), u64::MAX).await.unwrap_err();

        assert!(matches!(err, ApplicationError::StorageQuotaExceeded), "got {err:?}");
        assert!(!blobs.exists(&digest).await.unwrap());
        assert!(sessions.sessions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_monolithic_upload_with_a_wrong_digest_is_rejected_and_leaves_no_session() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let sessions = Arc::new(FakeUploadSessions::new());
        let use_case = monolithic(sessions.clone(), blobs.clone(), Arc::new(FakeRepositories::new()));

        let err = use_case.execute(Uuid::new_v4(), &Digest::of(b"something else"), body(b"monolithic-bytes"), u64::MAX).await.unwrap_err();

        assert!(matches!(err, ApplicationError::DockerDigestMismatch { .. }), "got {err:?}");
        assert!(sessions.sessions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn staged_bytes_of_other_open_sessions_count_against_the_quota() {
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(repo_with_quota(repository_id, Some(100)));
        let sessions = Arc::new(FakeUploadSessions::new());
        let start = StartBlobUploadUseCase::new(sessions.clone());
        let first = start.execute(repository_id).await.unwrap();
        let second = start.execute(repository_id).await.unwrap();
        let use_case = PatchBlobUploadUseCase::new(sessions, Arc::new(FakeDockerBlobStore::new()), repositories, Arc::new(FakeRepositoryQuotaLock::new()));

        use_case.execute(first.id, repository_id, &[0u8; 60], None).await.unwrap();
        let err = use_case.execute(second.id, repository_id, &[0u8; 60], None).await.unwrap_err();

        assert!(matches!(err, ApplicationError::StorageQuotaExceeded), "two sessions must not each get the whole quota, got {err:?}");
        use_case.execute(second.id, repository_id, &[0u8; 40], None).await.unwrap();
    }

    #[tokio::test]
    async fn a_chunk_over_the_request_limit_is_rejected_without_a_quota() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let repository_id = Uuid::new_v4();
        let session = StartBlobUploadUseCase::new(sessions.clone()).execute(repository_id).await.unwrap();
        let use_case = patch_use_case(sessions.clone());

        let err = use_case.execute_stream(session.id, repository_id, body(&[0u8; 20]), None, 10).await.unwrap_err();

        assert!(matches!(err, ApplicationError::DockerUploadTooLarge), "got {err:?}");
        assert_eq!(sessions.find(session.id).await.unwrap().unwrap().bytes_received, 0, "nothing was kept");
    }

    #[tokio::test]
    async fn a_repository_cannot_hold_more_than_the_allowed_number_of_open_uploads() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let start = StartBlobUploadUseCase::new(sessions);
        let repository_id = Uuid::new_v4();
        for _ in 0..artiferris_domain::docker_registry::MAX_OPEN_UPLOADS_PER_REPOSITORY {
            start.execute(repository_id).await.unwrap();
        }

        let err = start.execute(repository_id).await.unwrap_err();

        assert!(matches!(err, ApplicationError::DockerTooManyUploads), "got {err:?}");
        start.execute(Uuid::new_v4()).await.unwrap();
    }

    #[tokio::test]
    async fn completing_seals_the_session_so_a_later_chunk_cannot_change_what_was_hashed() {
        let sessions = Arc::new(FakeUploadSessions::new());
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let repository_id = Uuid::new_v4();
        let session = StartBlobUploadUseCase::new(sessions.clone()).execute(repository_id).await.unwrap();
        let patch = patch_use_case(sessions.clone());
        patch.execute(session.id, repository_id, b"blob-bytes", None).await.unwrap();

        sessions.seal(session.id).await.unwrap();
        let err = patch.execute(session.id, repository_id, b"-late", None).await.unwrap_err();

        assert!(matches!(err, ApplicationError::DockerUploadInProgress), "got {err:?}");
        let complete = CompleteBlobUploadUseCase::new(sessions, blobs.clone());
        complete.execute(session.id, repository_id, &Digest::of(b"blob-bytes")).await.unwrap();
        assert!(blobs.exists(&Digest::of(b"blob-bytes")).await.unwrap());
    }

    /// The quota lock holder needs pool connections of its own for the re-check; waiters must not be sitting on them.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn concurrent_chunks_on_a_quota_repository_do_not_starve_a_small_pool(pool: sqlx::PgPool) {
        use artiferris_infrastructure::filesystem_docker_blob_store::FilesystemDockerBlobStore;
        use artiferris_infrastructure::postgres::docker_upload_session_repository::PostgresDockerUploadSessionRepository;
        use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;

        let small_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(std::time::Duration::from_secs(3))
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap();
        let repository_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, quota_bytes, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'docker', 'hosted', NULL, $4, 1, now(), now())",
            repository_id,
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            format!("small-pool-{repository_id}"),
            1_000_000i64,
        )
        .execute(&pool)
        .await
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(PostgresPackageRepositoryStore::new(small_pool.clone(), "test-secret".to_string()));
        let sessions = Arc::new(PostgresDockerUploadSessionRepository::new(small_pool.clone(), dir.path().join("staging")));
        let blobs = Arc::new(FilesystemDockerBlobStore::new(small_pool.clone(), dir.path().join("blobs")));
        let use_case = Arc::new(PatchBlobUploadUseCase::new(sessions.clone(), blobs, store.clone(), store));
        let mut ids = Vec::new();
        for _ in 0..20 {
            ids.push(sessions.create(repository_id).await.unwrap().id);
        }

        let started = std::time::Instant::now();
        let mut tasks = tokio::task::JoinSet::new();
        for id in ids {
            let use_case = use_case.clone();
            tasks.spawn(async move { use_case.execute(id, repository_id, &[1u8; 1024], None).await });
        }
        let mut failures = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            if let Err(e) = joined.unwrap() {
                failures.push(format!("{e:?}"));
            }
        }

        assert!(failures.is_empty(), "chunks failed: {failures:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(3), "took {:?}", started.elapsed());
    }
}
