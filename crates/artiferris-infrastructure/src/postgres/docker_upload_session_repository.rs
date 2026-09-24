use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use artiferris_domain::docker_registry::{ByteStream, Digest, DockerUploadSession, DockerUploadSessionPort, MAX_OPEN_UPLOADS_PER_REPOSITORY};
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use futures_util::StreamExt;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;

/// How often a chunk still being received pushes its session's expiry out again.
const EXPIRY_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Above this many entries, `session_lock` drops the locks nobody is holding.
const LOCK_TABLE_PRUNE_THRESHOLD: usize = 1024;

pub struct PostgresDockerUploadSessionRepository {
    pool: PgPool,
    staging_root: PathBuf,
    /// One lock per session being written, so appends to a session run one after the other. Per process, like the staging volume.
    write_locks: Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    refresh_interval: std::time::Duration,
}

impl PostgresDockerUploadSessionRepository {
    pub fn new(pool: PgPool, staging_root: impl Into<PathBuf>) -> Self {
        Self { pool, staging_root: staging_root.into(), write_locks: Mutex::new(HashMap::new()), refresh_interval: EXPIRY_REFRESH_INTERVAL }
    }

    pub fn with_refresh_interval(mut self, interval: std::time::Duration) -> Self {
        self.refresh_interval = interval;
        self
    }

    fn staging_path_for(&self, id: Uuid) -> PathBuf {
        self.staging_root.join(format!("{id}.upload"))
    }

    fn session_lock(&self, id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.write_locks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if locks.len() >= LOCK_TABLE_PRUNE_THRESHOLD {
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        }
        locks.entry(id).or_default().clone()
    }

    fn forget_lock(&self, id: Uuid) {
        self.write_locks.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&id);
    }
}

#[async_trait]
impl DockerUploadSessionPort for PostgresDockerUploadSessionRepository {
    async fn create(&self, package_repository_id: Uuid) -> Result<DockerUploadSession, DomainError> {
        let id = Uuid::new_v4();
        let staging_path = self.staging_path_for(id);
        let staging_path_str = staging_path.to_string_lossy().to_string();

        // The advisory lock makes count-then-insert exact.
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", format!("docker-uploads:{package_repository_id}")).execute(&mut *tx).await.infra_err()?;
        let open = sqlx::query_scalar!(
            "SELECT count(*) FROM docker_blob_uploads WHERE package_repository_id = $1 AND expires_at > now()",
            package_repository_id
        )
        .fetch_one(&mut *tx)
        .await
        .infra_err()?
        .unwrap_or(0);
        if open >= MAX_OPEN_UPLOADS_PER_REPOSITORY as i64 {
            return Err(DomainError::TooManyUploads);
        }
        if let Some(parent) = staging_path.parent() {
            fs::create_dir_all(parent).await.infra_err()?;
        }
        fs::write(&staging_path, []).await.infra_err()?;
        let row = sqlx::query!(
            "INSERT INTO docker_blob_uploads (id, package_repository_id, staging_path, bytes_received, created_at, expires_at) \
             VALUES ($1, $2, $3, 0, now(), now() + interval '1 hour') \
             RETURNING id, package_repository_id, staging_path, bytes_received, created_at, expires_at",
            id,
            package_repository_id,
            staging_path_str,
        )
        .fetch_one(&mut *tx)
        .await;
        let row = match row {
            Ok(row) => row,
            Err(e) => {
                let _ = fs::remove_file(&staging_path).await;
                return Err(DomainError::Infrastructure(e.to_string()));
            }
        };
        if let Err(e) = tx.commit().await {
            let _ = fs::remove_file(&staging_path).await;
            return Err(DomainError::Infrastructure(e.to_string()));
        }
        Ok(DockerUploadSession {
            id: row.id,
            package_repository_id: row.package_repository_id,
            staging_path: row.staging_path,
            bytes_received: row.bytes_received,
            created_at: row.created_at,
            expires_at: row.expires_at,
        })
    }

    async fn find(&self, id: Uuid) -> Result<Option<DockerUploadSession>, DomainError> {
        let row = sqlx::query!(
            "SELECT id, package_repository_id, staging_path, bytes_received, created_at, expires_at FROM docker_blob_uploads WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        let Some(row) = row else { return Ok(None) };

        if row.expires_at < chrono::Utc::now() {
            let _ = fs::remove_file(&row.staging_path).await;
            sqlx::query!("DELETE FROM docker_blob_uploads WHERE id = $1", id).execute(&self.pool).await.ok();
            self.forget_lock(id);
            return Ok(None);
        }

        Ok(Some(DockerUploadSession {
            id: row.id,
            package_repository_id: row.package_repository_id,
            staging_path: row.staging_path,
            bytes_received: row.bytes_received,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }))
    }

    async fn append_chunk(&self, id: Uuid, chunk: &[u8], expected_start: Option<i64>) -> Result<i64, DomainError> {
        let body: ByteStream = Box::pin(futures_util::stream::once(std::future::ready(Ok(bytes::Bytes::copy_from_slice(chunk)))));
        self.append_stream(id, body, expected_start, u64::MAX).await
    }

    async fn append_stream(&self, id: Uuid, mut chunk: ByteStream, expected_start: Option<i64>, max_bytes: u64) -> Result<i64, DomainError> {
        let lock = self.session_lock(id);
        let _writing = lock.lock().await;

        // Being written to counts as activity, so a slow upload isn't swept from under itself.
        let row = sqlx::query!("UPDATE docker_blob_uploads SET expires_at = now() + interval '1 hour' WHERE id = $1 RETURNING staging_path, bytes_received, sealed_at", id)
            .fetch_optional(&self.pool)
            .await
            .infra_err()?
            .ok_or(DomainError::UploadSessionNotFound)?;
        if row.sealed_at.is_some() {
            return Err(DomainError::UploadInProgress);
        }
        if let Some(expected_start) = expected_start {
            if expected_start != row.bytes_received {
                return Err(DomainError::ChunkOffsetMismatch { expected: row.bytes_received, got: expected_start });
            }
        }

        // The write must land at exactly `row.bytes_received` — if the on-disk file has drifted ahead
        // (e.g. a crash after a prior write but before its counter update committed), truncate back to
        // the recorded offset first, so a retried/duplicate write can never double-append (M-14).
        let start = row.bytes_received as u64;
        let mut file = fs::OpenOptions::new().write(true).open(&row.staging_path).await.infra_err()?;
        file.seek(std::io::SeekFrom::Start(start)).await.infra_err()?;
        file.set_len(start).await.infra_err()?;

        let mut written: u64 = 0;
        let mut last_refresh = tokio::time::Instant::now();
        let outcome: Result<(), DomainError> = async {
            while let Some(part) = chunk.next().await {
                let part = part?;
                // A single chunk can take longer than the hour a session lives; keep it from being swept while it arrives.
                if last_refresh.elapsed() >= self.refresh_interval {
                    let refreshed = sqlx::query!("UPDATE docker_blob_uploads SET expires_at = now() + interval '1 hour' WHERE id = $1", id).execute(&self.pool).await.infra_err()?;
                    if refreshed.rows_affected() == 0 {
                        return Err(DomainError::UploadSessionNotFound);
                    }
                    last_refresh = tokio::time::Instant::now();
                }
                written += part.len() as u64;
                if written > max_bytes {
                    return Err(DomainError::UploadTooLarge);
                }
                file.write_all(&part).await.infra_err()?;
            }
            file.sync_data().await.infra_err()
        }
        .await;
        if let Err(e) = outcome {
            let _ = file.set_len(start).await;
            return Err(e);
        }

        let updated = sqlx::query!(
            "UPDATE docker_blob_uploads SET bytes_received = $2, expires_at = now() + interval '1 hour' WHERE id = $1 AND bytes_received = $3 AND sealed_at IS NULL RETURNING bytes_received",
            id,
            (start + written) as i64,
            start as i64,
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        match updated {
            Some(updated) => Ok(updated.bytes_received),
            None => {
                let _ = file.set_len(start).await;
                Err(DomainError::UploadInProgress)
            }
        }
    }

    async fn rewind(&self, id: Uuid, from_bytes: i64, to_bytes: i64) -> Result<(), DomainError> {
        let lock = self.session_lock(id);
        let _writing = lock.lock().await;
        let rewound = sqlx::query!(
            "UPDATE docker_blob_uploads SET bytes_received = $3 WHERE id = $1 AND bytes_received = $2 AND sealed_at IS NULL RETURNING staging_path",
            id,
            from_bytes,
            to_bytes,
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        if let Some(row) = rewound {
            let file = fs::OpenOptions::new().write(true).open(&row.staging_path).await.infra_err()?;
            file.set_len(to_bytes as u64).await.infra_err()?;
        }
        Ok(())
    }

    async fn seal(&self, id: Uuid) -> Result<DockerUploadSession, DomainError> {
        // Waits for a chunk still being written; once sealed, no later chunk is accepted.
        let lock = self.session_lock(id);
        let _writing = lock.lock().await;
        let row = sqlx::query!(
            "UPDATE docker_blob_uploads SET sealed_at = COALESCE(sealed_at, now()) WHERE id = $1 AND expires_at > now() \
             RETURNING id, package_repository_id, staging_path, bytes_received, created_at, expires_at",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?
        .ok_or(DomainError::UploadSessionNotFound)?;
        Ok(DockerUploadSession {
            id: row.id,
            package_repository_id: row.package_repository_id,
            staging_path: row.staging_path,
            bytes_received: row.bytes_received,
            created_at: row.created_at,
            expires_at: row.expires_at,
        })
    }

    async fn hash_staged_file(&self, id: Uuid) -> Result<(Digest, u64), DomainError> {
        let session = self.find(id).await?.ok_or(DomainError::UploadSessionNotFound)?;
        let mut file = fs::File::open(&session.staging_path).await.infra_err()?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let n = file.read(&mut buf).await.infra_err()?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            total += n as u64;
        }
        let digest = Digest::parse(&format!("sha256:{}", hex::encode(hasher.finalize()))).expect("well-formed sha256 hex digest");
        Ok((digest, total))
    }

    async fn delete(&self, id: Uuid) -> Result<(), DomainError> {
        if let Some(session) = self.find(id).await? {
            let _ = fs::remove_file(&session.staging_path).await;
        }
        sqlx::query!("DELETE FROM docker_blob_uploads WHERE id = $1", id)
            .execute(&self.pool)
            .await
            .infra_err()?;
        self.forget_lock(id);
        Ok(())
    }

    async fn staged_bytes_for_repository(&self, package_repository_id: Uuid) -> Result<u64, DomainError> {
        let staged = sqlx::query_scalar!(
            "SELECT COALESCE(SUM(bytes_received), 0)::BIGINT FROM docker_blob_uploads WHERE package_repository_id = $1 AND expires_at > now()",
            package_repository_id
        )
        .fetch_one(&self.pool)
        .await
        .infra_err()?;
        Ok(staged.unwrap_or(0) as u64)
    }

    async fn sweep_expired_uploads(&self) -> Result<usize, DomainError> {
        let rows = sqlx::query!("DELETE FROM docker_blob_uploads WHERE expires_at < now() RETURNING id, staging_path")
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        for row in &rows {
            let _ = fs::remove_file(&row.staging_path).await;
            self.forget_lock(row.id);
        }
        Ok(rows.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::docker_registry::MAX_OPEN_UPLOADS_PER_REPOSITORY;

    async fn seed_repository(pool: &sqlx::PgPool, id: Uuid) {
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'docker', 'hosted', NULL, 1, now(), now())",
            id,
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            format!("repo-{id}"),
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn creates_a_session_with_zero_bytes_received(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());

        let session = sessions.create(repository_id).await.unwrap();

        assert_eq!(session.package_repository_id, repository_id);
        assert_eq!(session.bytes_received, 0);
        assert_eq!(sessions.find(session.id).await.unwrap().unwrap().id, session.id);
    }

    #[sqlx::test]
    async fn a_session_expires_after_an_hour_of_silence_and_every_chunk_starts_the_hour_over(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool.clone(), dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        assert!(session.expires_at < chrono::Utc::now() + chrono::Duration::minutes(61), "{}", session.expires_at);

        sqlx::query!("UPDATE docker_blob_uploads SET expires_at = now() + interval '1 minute' WHERE id = $1", session.id).execute(&pool).await.unwrap();
        sessions.append_chunk(session.id, b"hello", None).await.unwrap();

        let refreshed = sessions.find(session.id).await.unwrap().unwrap();
        assert!(refreshed.expires_at > chrono::Utc::now() + chrono::Duration::minutes(50), "{}", refreshed.expires_at);
    }

    #[sqlx::test]
    async fn a_chunk_that_takes_longer_than_the_session_lifetime_is_not_swept_while_it_arrives(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = Arc::new(PostgresDockerUploadSessionRepository::new(pool.clone(), dir.path()).with_refresh_interval(std::time::Duration::from_millis(50)));
        let session = sessions.create(repository_id).await.unwrap();
        // Ten parts, one every 100 ms.
        let slow: ByteStream = Box::pin(futures_util::stream::unfold(0u8, |sent| async move {
            if sent == 10 {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            Some((Ok(bytes::Bytes::from_static(b"part-")), sent + 1))
        }));
        let upload = {
            let sessions = sessions.clone();
            tokio::spawn(async move { sessions.append_stream(session.id, slow, None, u64::MAX).await })
        };

        // What an hour of streaming does to the expiry, then a sweep, with the chunk still arriving.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        sqlx::query!("UPDATE docker_blob_uploads SET expires_at = now() - interval '1 minute' WHERE id = $1", session.id).execute(&pool).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        sessions.sweep_expired_uploads().await.unwrap();

        assert_eq!(upload.await.unwrap().expect("the upload must not be cut off by the sweep"), 50);
    }

    #[sqlx::test]
    async fn rewinding_takes_a_chunk_back_only_if_the_session_is_still_where_the_chunk_left_it(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        sessions.append_chunk(session.id, b"hello-", None).await.unwrap();
        sessions.append_chunk(session.id, b"world", None).await.unwrap();

        sessions.rewind(session.id, 6, 0).await.unwrap();
        assert_eq!(sessions.find(session.id).await.unwrap().unwrap().bytes_received, 11, "the session had moved on");

        sessions.rewind(session.id, 11, 6).await.unwrap();
        assert_eq!(sessions.find(session.id).await.unwrap().unwrap().bytes_received, 6);
        assert_eq!(sessions.hash_staged_file(session.id).await.unwrap(), (Digest::of(b"hello-"), 6));
    }

    #[sqlx::test]
    async fn appending_chunks_accumulates_bytes_in_order(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());
        let session = sessions.create(repository_id).await.unwrap();

        let total_after_first = sessions.append_chunk(session.id, b"hello-", Some(0)).await.unwrap();
        let total_after_second = sessions.append_chunk(session.id, b"world", Some(6)).await.unwrap();

        assert_eq!(total_after_first, 6);
        assert_eq!(total_after_second, 11);
        assert_eq!(sessions.hash_staged_file(session.id).await.unwrap(), (Digest::of(b"hello-world"), 11));
        assert_eq!(sessions.find(session.id).await.unwrap().unwrap().bytes_received, 11);
    }

    /// Larger than the 64 KiB read buffer `hash_staged_file` uses internally, so this actually
    /// exercises more than one read/hash-update iteration.
    #[sqlx::test]
    async fn hash_staged_file_matches_a_one_shot_hash_for_content_spanning_multiple_reads(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        let content = vec![0xABu8; 200 * 1024];
        sessions.append_chunk(session.id, &content, Some(0)).await.unwrap();

        let (digest, size) = sessions.hash_staged_file(session.id).await.unwrap();

        assert_eq!(digest, Digest::of(&content));
        assert_eq!(size, content.len() as u64);
    }

    #[sqlx::test]
    async fn concurrent_appends_do_not_lose_a_chunks_contribution_to_the_byte_count(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = std::sync::Arc::new(PostgresDockerUploadSessionRepository::new(pool, dir.path()));
        let session = sessions.create(repository_id).await.unwrap();

        let a = sessions.clone();
        let b = sessions.clone();
        let id = session.id;
        let (result_a, result_b) = tokio::join!(a.append_chunk(id, &[0u8; 100], None), b.append_chunk(id, &[0u8; 50], None));
        result_a.unwrap();
        result_b.unwrap();

        assert_eq!(sessions.find(id).await.unwrap().unwrap().bytes_received, 150, "both concurrent chunks must be counted, not just the last writer");
    }

    #[sqlx::test]
    async fn two_concurrent_chunks_claiming_the_same_offset_only_let_one_through(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = std::sync::Arc::new(PostgresDockerUploadSessionRepository::new(pool, dir.path()));
        let session = sessions.create(repository_id).await.unwrap();

        let a = sessions.clone();
        let b = sessions.clone();
        let id = session.id;
        let (result_a, result_b) = tokio::join!(a.append_chunk(id, &[1u8; 10], Some(0)), b.append_chunk(id, &[2u8; 10], Some(0)));

        let outcomes = [result_a, result_b];
        let successes = outcomes.iter().filter(|r| r.is_ok()).count();
        let mismatches = outcomes.iter().filter(|r| matches!(r, Err(DomainError::ChunkOffsetMismatch { .. }))).count();
        assert_eq!(successes, 1, "only one of two chunks claiming the same start offset may be accepted");
        assert_eq!(mismatches, 1, "the loser must see a clear offset mismatch, not silently corrupt the stream");
        assert_eq!(sessions.find(id).await.unwrap().unwrap().bytes_received, 10, "only the winning chunk's bytes were counted");
    }

    #[sqlx::test]
    async fn append_chunk_self_heals_when_the_file_is_ahead_of_the_recorded_offset(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        // Simulate a prior crash: bytes already on disk, but the DB row was never updated to match.
        tokio::fs::write(&session.staging_path, b"stray-bytes-from-a-crash").await.unwrap();

        let new_offset = sessions.append_chunk(session.id, b"real-chunk", Some(0)).await.unwrap();

        assert_eq!(new_offset, 10, "the write must land at the RECORDED offset (0), not the file's actual (desynced) EOF");
        let on_disk = tokio::fs::read(&session.staging_path).await.unwrap();
        assert_eq!(on_disk, b"real-chunk", "the stray pre-crash bytes must be truncated away, not appended after");
    }

    #[sqlx::test]
    async fn deleting_a_session_removes_its_row_and_staging_file(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        sessions.append_chunk(session.id, b"data", None).await.unwrap();

        sessions.delete(session.id).await.unwrap();

        assert!(sessions.find(session.id).await.unwrap().is_none());
        assert!(sessions.hash_staged_file(session.id).await.is_err());
    }

    #[sqlx::test]
    async fn an_expired_session_is_lazily_swept_on_the_next_find(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool.clone(), dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        sqlx::query!("UPDATE docker_blob_uploads SET expires_at = now() - interval '1 hour' WHERE id = $1", session.id)
            .execute(&pool)
            .await
            .unwrap();

        assert!(sessions.find(session.id).await.unwrap().is_none());
        let remaining: i64 = sqlx::query_scalar!("SELECT count(*) FROM docker_blob_uploads WHERE id = $1", session.id).fetch_one(&pool).await.unwrap().unwrap();
        assert_eq!(remaining, 0);
        assert!(!std::path::Path::new(&session.staging_path).exists());
    }

    /// Unlike `an_expired_session_is_lazily_swept_on_the_next_find` above, nothing ever calls `find`
    /// on this session again — the background sweep (M-13) is the only thing that ever reclaims it.
    #[sqlx::test]
    async fn sweep_expired_uploads_removes_a_session_nobody_ever_looks_up_again(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool.clone(), dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        sqlx::query!("UPDATE docker_blob_uploads SET expires_at = now() - interval '1 hour' WHERE id = $1", session.id)
            .execute(&pool)
            .await
            .unwrap();

        let swept = sessions.sweep_expired_uploads().await.unwrap();

        assert_eq!(swept, 1);
        assert!(!std::path::Path::new(&session.staging_path).exists(), "the staging file must be removed too");
        let remaining = sqlx::query!("SELECT id FROM docker_blob_uploads WHERE id = $1", session.id).fetch_optional(&pool).await.unwrap();
        assert!(remaining.is_none());
    }

    fn stream_of(chunks: Vec<Result<&'static [u8], DomainError>>) -> ByteStream {
        Box::pin(futures_util::stream::iter(chunks.into_iter().map(|chunk| chunk.map(bytes::Bytes::from_static))))
    }

    #[sqlx::test]
    async fn a_streamed_body_lands_chunk_by_chunk_and_counts_once_complete(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());
        let session = sessions.create(repository_id).await.unwrap();

        let total = sessions.append_stream(session.id, stream_of(vec![Ok(b"hello-"), Ok(b"world")]), Some(0), 100).await.unwrap();

        assert_eq!(total, 11);
        assert_eq!(sessions.hash_staged_file(session.id).await.unwrap(), (Digest::of(b"hello-world"), 11));
        assert_eq!(sessions.staged_bytes_for_repository(repository_id).await.unwrap(), 11);
    }

    #[sqlx::test]
    async fn a_stream_that_fails_or_runs_over_its_limit_leaves_the_session_where_it_was(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        sessions.append_chunk(session.id, b"kept", Some(0)).await.unwrap();

        let dropped = sessions.append_stream(session.id, stream_of(vec![Ok(b"partial"), Err(DomainError::Validation("client went away".into()))]), None, 100).await;
        assert!(matches!(dropped, Err(DomainError::Validation(_))));
        let too_big = sessions.append_stream(session.id, stream_of(vec![Ok(b"0123456789")]), None, 5).await;
        assert!(matches!(too_big, Err(DomainError::UploadTooLarge)));

        assert_eq!(sessions.find(session.id).await.unwrap().unwrap().bytes_received, 4);
        assert_eq!(sessions.hash_staged_file(session.id).await.unwrap(), (Digest::of(b"kept"), 4), "the partial bytes are gone from the file too");
    }

    #[sqlx::test]
    async fn a_sealed_session_takes_no_more_chunks_and_sealing_is_idempotent(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool, dir.path());
        let session = sessions.create(repository_id).await.unwrap();
        sessions.append_chunk(session.id, b"hashed", None).await.unwrap();

        let sealed = sessions.seal(session.id).await.unwrap();
        sessions.seal(session.id).await.unwrap();
        let late = sessions.append_chunk(session.id, b"-too-late", None).await;

        assert_eq!(sealed.bytes_received, 6);
        assert!(matches!(late, Err(DomainError::UploadInProgress)), "{late:?}");
        assert_eq!(sessions.hash_staged_file(session.id).await.unwrap(), (Digest::of(b"hashed"), 6));
    }

    /// Whichever way a slow chunk and the seal interleave, the file is either hashed with the whole chunk or without any of it, and never grows after the seal.
    #[sqlx::test]
    async fn sealing_waits_for_a_chunk_that_is_still_arriving(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = std::sync::Arc::new(PostgresDockerUploadSessionRepository::new(pool, dir.path()));
        let session = sessions.create(repository_id).await.unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<Result<bytes::Bytes, DomainError>>();
        let slow: ByteStream = Box::pin(futures_util::stream::poll_fn(move |cx| receiver.poll_recv(cx)));
        sender.send(Ok(bytes::Bytes::from_static(b"first-half-"))).unwrap();

        let appender = {
            let sessions = sessions.clone();
            tokio::spawn(async move { sessions.append_stream(session.id, slow, None, 100).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let sealer = {
            let sessions = sessions.clone();
            tokio::spawn(async move { sessions.seal(session.id).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!sealer.is_finished(), "sealing must wait for the chunk in flight");
        sender.send(Ok(bytes::Bytes::from_static(b"second-half"))).unwrap();
        drop(sender);

        appender.await.unwrap().unwrap();
        let sealed = sealer.await.unwrap().unwrap();

        assert_eq!(sealed.bytes_received, 22);
        assert_eq!(sessions.hash_staged_file(session.id).await.unwrap(), (Digest::of(b"first-half-second-half"), 22));
    }

    #[sqlx::test]
    async fn a_repository_cannot_open_more_uploads_than_the_cap_and_others_are_unaffected(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        let other_repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        seed_repository(&pool, other_repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool.clone(), dir.path());
        for _ in 0..MAX_OPEN_UPLOADS_PER_REPOSITORY {
            sessions.create(repository_id).await.unwrap();
        }

        assert!(matches!(sessions.create(repository_id).await, Err(DomainError::TooManyUploads)));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), MAX_OPEN_UPLOADS_PER_REPOSITORY, "the refused session left no staging file");
        sessions.create(other_repository_id).await.unwrap();

        // Expired sessions no longer count.
        sqlx::query!("UPDATE docker_blob_uploads SET expires_at = now() - interval '1 hour' WHERE package_repository_id = $1", repository_id).execute(&pool).await.unwrap();
        sessions.create(repository_id).await.unwrap();
    }

    #[sqlx::test]
    async fn concurrent_creates_never_exceed_the_cap(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let sessions = std::sync::Arc::new(PostgresDockerUploadSessionRepository::new(pool.clone(), dir.path()));

        let handles: Vec<_> = (0..MAX_OPEN_UPLOADS_PER_REPOSITORY + 8)
            .map(|_| {
                let sessions = sessions.clone();
                tokio::spawn(async move { sessions.create(repository_id).await.is_ok() })
            })
            .collect();
        let mut created = 0;
        for handle in handles {
            created += usize::from(handle.await.unwrap());
        }

        assert_eq!(created, MAX_OPEN_UPLOADS_PER_REPOSITORY);
    }

    #[sqlx::test]
    async fn staged_bytes_add_up_across_a_repositorys_open_sessions_only(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        let other_repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        seed_repository(&pool, other_repository_id).await;
        let sessions = PostgresDockerUploadSessionRepository::new(pool.clone(), dir.path());
        for (repository, bytes) in [(repository_id, 10usize), (repository_id, 5), (other_repository_id, 100)] {
            let session = sessions.create(repository).await.unwrap();
            sessions.append_chunk(session.id, &vec![0u8; bytes], None).await.unwrap();
        }
        let expired = sessions.create(repository_id).await.unwrap();
        sessions.append_chunk(expired.id, &[0u8; 1000], None).await.unwrap();
        sqlx::query!("UPDATE docker_blob_uploads SET expires_at = now() - interval '1 hour' WHERE id = $1", expired.id).execute(&pool).await.unwrap();

        assert_eq!(sessions.staged_bytes_for_repository(repository_id).await.unwrap(), 15);
        assert_eq!(sessions.staged_bytes_for_repository(Uuid::new_v4()).await.unwrap(), 0);
    }
}
