use std::path::PathBuf;

use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt;
use artiferris_domain::docker_registry::{BlobSweepReport, ByteStream, DOCKER_TAG_QUOTA_BYTES, Digest, DockerBlobStorePort};
use artiferris_domain::error::DomainError;
use sqlx::{Acquire, PgPool};
use tokio::fs;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::atomic_write::{finish_atomic_write, write_temp, TempFileGuard};
use crate::error_ext::InfraErr;

/// A large backlog of orphans is worked off over several sweeps.
const SWEEP_BATCH_LIMIT: i64 = 1000;

/// A temp file this old belongs to a write that is long dead: no fill runs anywhere near this long.
const STALE_TEMP_FILE_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

pub struct FilesystemDockerBlobStore {
    pool: PgPool,
    root: PathBuf,
}

impl FilesystemDockerBlobStore {
    pub fn new(pool: PgPool, root: impl Into<PathBuf>) -> Self {
        Self { pool, root: root.into() }
    }

    fn hex_part(digest: &Digest) -> Result<&str, DomainError> {
        digest.as_str().strip_prefix("sha256:").ok_or_else(|| DomainError::Validation(format!("unsupported digest algorithm: {}", digest.as_str())))
    }

    /// Shards two levels deep by the digest's first four hex chars.
    fn storage_key(hex: &str) -> String {
        format!("sha256/{}/{}/{}", &hex[0..2], &hex[2..4], hex)
    }

    /// The locked half of `write`: one transaction spans the digest lock and the final rename (see
    /// `delete_row_if_unreferenced_now` for the race against a concurrent delete).
    async fn publish_written_blob(&self, digest: &Digest, storage_key: &str, tmp_path: &std::path::Path, target: &std::path::Path, size_bytes: i64) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest.as_str()).execute(&mut *tx).await.infra_err()?;

        finish_atomic_write(tmp_path, target).await.infra_err()?;

        sqlx::query!(
            "INSERT INTO docker_blobs (digest, size_bytes, storage_key, reference_count) VALUES ($1, $2, $3, 0) \
             ON CONFLICT (digest) DO UPDATE SET created_at = now()",
            digest.as_str(),
            size_bytes,
            storage_key,
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    /// Phase 1 of `delete_if_unreferenced`: DB-only, under the digest lock, no filesystem I/O. Deletes the row
    /// if nothing references or links the blob; a link or manifest row that lands at the same moment shows up as a
    /// foreign-key violation, which leaves the blob alone. Returns the deleted row's `storage_key`.
    async fn delete_row_if_unreferenced_now(&self, digest: &Digest) -> Result<Option<String>, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest.as_str()).execute(&mut *tx).await.infra_err()?;

        let mut savepoint = tx.begin().await.infra_err()?;
        let deleted = match sqlx::query!(
            "DELETE FROM docker_blobs WHERE digest = $1 AND reference_count <= 0 \
             AND NOT EXISTS (SELECT 1 FROM docker_repository_blobs WHERE blob_digest = $1) \
             AND NOT EXISTS (SELECT 1 FROM docker_manifest_blobs WHERE blob_digest = $1) \
             RETURNING storage_key",
            digest.as_str()
        )
        .fetch_optional(&mut *savepoint)
        .await
        {
            Ok(deleted) => {
                savepoint.commit().await.infra_err()?;
                deleted
            }
            Err(sqlx::Error::Database(e)) if e.is_foreign_key_violation() => {
                savepoint.rollback().await.infra_err()?;
                None
            }
            Err(e) => return Err(DomainError::Infrastructure(e.to_string())),
        };
        tx.commit().await.infra_err()?;
        Ok(deleted.map(|row| row.storage_key))
    }

    /// The sweep's `delete_row_if_unreferenced_now`: deletes the row only if it is still old, unreferenced and unlinked
    /// under the digest lock. Returns the storage key when it deleted.
    async fn delete_row_if_unreferenced(&self, digest: &Digest, older_than: chrono::DateTime<chrono::Utc>) -> Result<Option<String>, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest.as_str()).execute(&mut *tx).await.infra_err()?;

        let mut savepoint = tx.begin().await.infra_err()?;
        let deleted = match sqlx::query!(
            "DELETE FROM docker_blobs WHERE digest = $1 AND created_at < $2 AND reference_count <= 0 \
             AND NOT EXISTS (SELECT 1 FROM docker_repository_blobs WHERE blob_digest = $1) \
             AND NOT EXISTS (SELECT 1 FROM docker_manifest_blobs WHERE blob_digest = $1) \
             RETURNING storage_key",
            digest.as_str(),
            older_than,
        )
        .fetch_optional(&mut *savepoint)
        .await
        {
            Ok(deleted) => {
                savepoint.commit().await.infra_err()?;
                deleted
            }
            Err(sqlx::Error::Database(e)) if e.is_foreign_key_violation() => {
                savepoint.rollback().await.infra_err()?;
                None
            }
            Err(e) => return Err(DomainError::Infrastructure(e.to_string())),
        };
        tx.commit().await.infra_err()?;
        Ok(deleted.map(|row| row.storage_key))
    }

    /// Phase 2 of `delete_if_unreferenced`: only called when phase 1 actually deleted a
    /// row. Begins a NEW, separate transaction, RE-ACQUIRES the SAME digest lock, and RE-CHECKS
    /// whether the `docker_blobs` row for this digest still does not exist.
    ///
    /// This re-check is what makes it safe to not hold one lock across phase 1 and the filesystem
    /// removal: a concurrent `write`/`adopt_staged_file` for the SAME digest can freely race into
    /// the gap between phase 1's commit (which released the lock) and this call's own lock
    /// acquisition, and recreate the row. If it did, that row and its file are the correct,
    /// current state — this call must leave them alone. Only if the row is STILL absent does it
    /// remove the file backing phase 1's deleted row.
    ///
    /// Keeping phase 1's DELETE (durable the moment it commits) separate from phase 2 (whose only
    /// content is the lock and a read-only re-check) also means a crash, a cancelled request, or a
    /// COMMIT failure after `remove_file` here only loses phase 2's own commit — which has nothing
    /// of substance to lose — never the decrement or the DELETE. The worst case is an orphaned
    /// file with no row, not a ghost row with no file.
    async fn remove_file_if_still_absent(&self, digest: &Digest, storage_key: &str) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest.as_str()).execute(&mut *tx).await.infra_err()?;

        let row = sqlx::query!("SELECT EXISTS(SELECT 1 FROM docker_blobs WHERE digest = $1) AS \"exists!\"", digest.as_str())
            .fetch_one(&mut *tx)
            .await
            .infra_err()?;
        if !row.exists {
            let _ = fs::remove_file(self.root.join(storage_key)).await;
        }
        tx.commit().await.infra_err()?;
        Ok(())
    }
}

#[async_trait]
impl DockerBlobStorePort for FilesystemDockerBlobStore {
    async fn write(&self, digest: &Digest, bytes: &[u8]) -> Result<(), DomainError> {
        let hex = Self::hex_part(digest)?;
        let storage_key = Self::storage_key(hex);
        let target = self.root.join(&storage_key);

        // Write the bytes to a fresh temp file BEFORE acquiring the digest lock. A per-call UUID
        // temp path can't collide with any concurrent writer or deleter of this same digest, so
        // nothing about this step needs the lock — only the final rename onto `target` (which
        // touches the shared path) and the INSERT do. This keeps a large blob's `fs::write` off
        // the digest lock and off a held pool connection, same idea `adopt_staged_file` already
        // gets for free (its bytes are staged by an earlier, separate call).
        let tmp_path = write_temp(&target, hex, bytes).await.infra_err()?;
        let _tmp = TempFileGuard::new(tmp_path.clone());

        self.publish_written_blob(digest, &storage_key, &tmp_path, &target, bytes.len() as i64).await
    }

    async fn write_stream(&self, digest: &Digest, mut body: ByteStream, max_bytes: u64) -> Result<u64, DomainError> {
        let hex = Self::hex_part(digest)?;
        let storage_key = Self::storage_key(hex);
        let target = self.root.join(&storage_key);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await.infra_err()?;
        }
        let tmp_path = target.with_file_name(format!("{hex}.tmp-{}", Uuid::new_v4()));
        let _tmp = TempFileGuard::new(tmp_path.clone());

        let mut file = fs::File::create(&tmp_path).await.infra_err()?;
        let mut hasher = Sha256::new();
        let mut size: u64 = 0;
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            size += chunk.len() as u64;
            if size > max_bytes {
                return Err(DomainError::UploadTooLarge);
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.infra_err()?;
        }
        file.sync_data().await.infra_err()?;
        drop(file);
        let computed = format!("sha256:{}", hex::encode(hasher.finalize()));
        if computed != digest.as_str() {
            return Err(DomainError::DigestMismatch { expected: digest.as_str().to_string(), computed });
        }

        self.publish_written_blob(digest, &storage_key, &tmp_path, &target, size as i64).await?;
        Ok(size)
    }

    async fn adopt_staged_file(&self, digest: &Digest, staging_path: &str, size_bytes: u64) -> Result<(), DomainError> {
        let hex = Self::hex_part(digest)?;
        let storage_key = Self::storage_key(hex);
        let target = self.root.join(&storage_key);

        // Same digest-scoped advisory lock as `write`, held for the same reason across this
        // method's own DB+filesystem work.
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest.as_str()).execute(&mut *tx).await.infra_err()?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await.infra_err()?;
        }
        match fs::metadata(&target).await {
            Ok(existing) if existing.len() == size_bytes => {
                let _ = fs::remove_file(staging_path).await;
            }
            _ => fs::rename(staging_path, &target).await.infra_err()?,
        }

        sqlx::query!(
            "INSERT INTO docker_blobs (digest, size_bytes, storage_key, reference_count) VALUES ($1, $2, $3, 0) \
             ON CONFLICT (digest) DO UPDATE SET created_at = now()",
            digest.as_str(),
            size_bytes as i64,
            storage_key,
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn read(&self, digest: &Digest) -> Result<Vec<u8>, DomainError> {
        let row = sqlx::query!("SELECT storage_key FROM docker_blobs WHERE digest = $1", digest.as_str())
            .fetch_optional(&self.pool)
            .await
            .infra_err()?
            .ok_or_else(|| DomainError::Infrastructure(format!("blob not found: {}", digest.as_str())))?;
        fs::read(self.root.join(row.storage_key)).await.infra_err()
    }

    async fn read_stream(&self, digest: &Digest) -> Result<ByteStream, DomainError> {
        let row = sqlx::query!("SELECT storage_key FROM docker_blobs WHERE digest = $1", digest.as_str())
            .fetch_optional(&self.pool)
            .await
            .infra_err()?
            .ok_or_else(|| DomainError::Infrastructure(format!("blob not found: {}", digest.as_str())))?;
        let file = fs::File::open(self.root.join(row.storage_key)).await.infra_err()?;
        Ok(Box::pin(ReaderStream::new(file).map_err(|e| DomainError::Infrastructure(e.to_string()))))
    }

    async fn link_to_repository(&self, repository_id: Uuid, digest: &Digest) -> Result<(), DomainError> {
        sqlx::query!(
            "INSERT INTO docker_repository_blobs (package_repository_id, blob_digest) VALUES ($1, $2) \
             ON CONFLICT (package_repository_id, blob_digest) DO NOTHING",
            repository_id,
            digest.as_str(),
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn is_uploaded_to_repository(&self, repository_id: Uuid, digest: &Digest) -> Result<bool, DomainError> {
        let row = sqlx::query!(
            "SELECT EXISTS(SELECT 1 FROM docker_repository_blobs WHERE package_repository_id = $1 AND blob_digest = $2) AS \"exists!\"",
            repository_id,
            digest.as_str()
        )
        .fetch_one(&self.pool)
        .await
        .infra_err()?;
        Ok(row.exists)
    }

    async fn unlink_from_repository_if_unreferenced(&self, repository_id: Uuid, digest: &Digest) -> Result<(), DomainError> {
        // The NOT EXISTS re-verifies "no manifest in this repository references `digest` anymore" at
        // the same statement that removes the link — closing the race window between the caller's own
        // `blob_is_reachable` check and this call (e.g. a concurrent manifest push landing a fresh
        // `docker_manifest_blobs` row for the same digest in between).
        sqlx::query!(
            "DELETE FROM docker_repository_blobs \
             WHERE package_repository_id = $1 AND blob_digest = $2 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM docker_manifest_blobs dmb \
                   JOIN docker_manifests dm ON dm.id = dmb.manifest_id \
                   WHERE dm.package_repository_id = $1 AND dmb.blob_digest = $2 \
               )",
            repository_id,
            digest.as_str(),
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn exists(&self, digest: &Digest) -> Result<bool, DomainError> {
        let row = sqlx::query!("SELECT EXISTS(SELECT 1 FROM docker_blobs WHERE digest = $1) AS \"exists!\"", digest.as_str())
            .fetch_one(&self.pool)
            .await
            .infra_err()?;
        Ok(row.exists)
    }

    async fn size_if_exists(&self, digest: &Digest) -> Result<Option<u64>, DomainError> {
        let row = sqlx::query!("SELECT size_bytes FROM docker_blobs WHERE digest = $1", digest.as_str())
            .fetch_optional(&self.pool)
            .await
            .infra_err()?;
        Ok(row.map(|r| r.size_bytes as u64))
    }

    async fn existing_digests(&self, digests: &[Digest]) -> Result<std::collections::HashSet<String>, DomainError> {
        if digests.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let digest_strs: Vec<String> = digests.iter().map(|d| d.as_str().to_string()).collect();
        let rows = sqlx::query!("SELECT digest FROM docker_blobs WHERE digest = ANY($1)", &digest_strs)
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        Ok(rows.into_iter().map(|r| r.digest).collect())
    }

    async fn sum_sizes(&self, digests: &[Digest]) -> Result<u64, DomainError> {
        if digests.is_empty() {
            return Ok(0);
        }
        let digest_strs: Vec<String> = digests.iter().map(|d| d.as_str().to_string()).collect();
        let total: Option<i64> = sqlx::query_scalar!("SELECT SUM(size_bytes)::BIGINT FROM docker_blobs WHERE digest = ANY($1)", &digest_strs)
            .fetch_one(&self.pool)
            .await
            .infra_err()?;
        Ok(total.unwrap_or(0) as u64)
    }

    async fn increment_ref(&self, digest: &Digest) -> Result<(), DomainError> {
        sqlx::query!("UPDATE docker_blobs SET reference_count = reference_count + 1 WHERE digest = $1", digest.as_str())
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn increment_ref_all(&self, digests: &[Digest]) -> Result<(), DomainError> {
        if digests.is_empty() {
            return Ok(());
        }
        let digest_strs: Vec<String> = digests.iter().map(|d| d.as_str().to_string()).collect();
        sqlx::query!("UPDATE docker_blobs SET reference_count = reference_count + 1 WHERE digest = ANY($1)", &digest_strs)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn delete_if_unreferenced(&self, digest: &Digest) -> Result<bool, DomainError> {
        let Some(storage_key) = self.delete_row_if_unreferenced_now(digest).await? else {
            return Ok(false);
        };
        // The row is gone for good at this point, so a failure removing the file must not fail the call: a stray file is
        // recoverable, and an error here would abort the caller's loop over a manifest's other blobs.
        if let Err(e) = self.remove_file_if_still_absent(digest, &storage_key).await {
            tracing::warn!(digest = %digest.as_str(), storage_key, error = %e, "the blob row is gone but its file could not be removed");
        }
        Ok(true)
    }

    async fn remove_reclaimed_blob_files(&self, digests: &[Digest]) {
        for digest in digests {
            let Ok(hex) = Self::hex_part(digest) else {
                tracing::warn!(digest = %digest.as_str(), "repository deletion sweep reported a digest with an unsupported algorithm; skipping its file removal");
                continue;
            };
            let storage_key = Self::storage_key(hex);
            if let Err(e) = self.remove_file_if_still_absent(digest, &storage_key).await {
                tracing::warn!(digest = %digest.as_str(), storage_key, error = %e, "failed to remove an on-disk Docker blob file during the repository deletion sweep; its docker_blobs row is already gone");
            }
        }
    }

    async fn used_bytes_for_repository(&self, repository_id: Uuid) -> Result<u64, DomainError> {
        let total: i64 = sqlx::query_scalar!(
            "SELECT ( \
                 COALESCE((SELECT SUM(db.size_bytes) FROM docker_blobs db \
                           WHERE db.digest IN ( \
                               SELECT dmb.blob_digest FROM docker_manifest_blobs dmb \
                               JOIN docker_manifests dm ON dm.id = dmb.manifest_id \
                               WHERE dm.package_repository_id = $1 \
                               UNION \
                               SELECT blob_digest FROM docker_repository_blobs WHERE package_repository_id = $1 \
                           )), 0) \
                 + COALESCE((SELECT SUM(octet_length(body)) FROM docker_manifests WHERE package_repository_id = $1), 0) \
                 + (SELECT count(*) FROM docker_tags WHERE package_repository_id = $1) * $2 \
             )::BIGINT AS \"total!\"",
            repository_id,
            DOCKER_TAG_QUOTA_BYTES,
        )
        .fetch_one(&self.pool)
        .await
        .infra_err()?;
        Ok(total as u64)
    }

    async fn used_bytes_for_repositories(&self, repository_ids: &[Uuid]) -> Result<std::collections::HashMap<Uuid, u64>, DomainError> {
        let mut used: std::collections::HashMap<Uuid, u64> = std::collections::HashMap::new();
        let blobs = sqlx::query!(
            "SELECT repository_id AS \"repository_id!\", SUM(db.size_bytes)::BIGINT AS total_bytes FROM ( \
                 SELECT dm.package_repository_id AS repository_id, dmb.blob_digest \
                 FROM docker_manifests dm \
                 JOIN docker_manifest_blobs dmb ON dmb.manifest_id = dm.id \
                 WHERE dm.package_repository_id = ANY($1) \
                 UNION \
                 SELECT package_repository_id, blob_digest FROM docker_repository_blobs WHERE package_repository_id = ANY($1) \
             ) distinct_blobs \
             JOIN docker_blobs db ON db.digest = distinct_blobs.blob_digest \
             GROUP BY repository_id",
            repository_ids
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        let manifests = sqlx::query!(
            "SELECT package_repository_id, SUM(octet_length(body))::BIGINT AS total_bytes FROM docker_manifests WHERE package_repository_id = ANY($1) GROUP BY package_repository_id",
            repository_ids
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        let tags = sqlx::query!("SELECT package_repository_id, count(*) AS tags FROM docker_tags WHERE package_repository_id = ANY($1) GROUP BY package_repository_id", repository_ids)
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        for row in blobs {
            *used.entry(row.repository_id).or_default() += row.total_bytes.unwrap_or(0) as u64;
        }
        for row in manifests {
            *used.entry(row.package_repository_id).or_default() += row.total_bytes.unwrap_or(0) as u64;
        }
        for row in tags {
            *used.entry(row.package_repository_id).or_default() += row.tags.unwrap_or(0) as u64 * DOCKER_TAG_QUOTA_BYTES as u64;
        }
        Ok(used)
    }

    async fn sweep_unreferenced_blobs(&self, older_than: chrono::DateTime<chrono::Utc>) -> Result<BlobSweepReport, DomainError> {
        let temp_files_removed = remove_stale_temp_files(&self.root, STALE_TEMP_FILE_AGE).await;

        let counts_corrected = sqlx::query!(
            "UPDATE docker_blobs b SET reference_count = actual.references \
             FROM ( \
                 SELECT blob.digest, count(dmb.manifest_id) AS references FROM docker_blobs blob \
                 LEFT JOIN docker_manifest_blobs dmb ON dmb.blob_digest = blob.digest \
                 WHERE blob.created_at < $1 \
                 GROUP BY blob.digest \
             ) actual \
             WHERE actual.digest = b.digest AND b.reference_count <> actual.references",
            older_than
        )
        .execute(&self.pool)
        .await
        .infra_err()?
        .rows_affected() as usize;

        // Links first: one no manifest of its (hosted) repository references, past the grace period, is abandoned.
        let links_removed = sqlx::query!(
            "DELETE FROM docker_repository_blobs l USING package_repository_projections r \
             WHERE r.id = l.package_repository_id AND r.repo_type = 'hosted' AND l.created_at < $1 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM docker_manifest_blobs dmb JOIN docker_manifests dm ON dm.id = dmb.manifest_id \
                   WHERE dm.package_repository_id = l.package_repository_id AND dmb.blob_digest = l.blob_digest \
               )",
            older_than
        )
        .execute(&self.pool)
        .await
        .infra_err()?
        .rows_affected() as usize;

        let candidates = sqlx::query_scalar!(
            "SELECT b.digest FROM docker_blobs b \
             WHERE b.created_at < $1 AND b.reference_count <= 0 \
               AND NOT EXISTS (SELECT 1 FROM docker_repository_blobs l WHERE l.blob_digest = b.digest) \
               AND NOT EXISTS (SELECT 1 FROM docker_manifest_blobs dmb WHERE dmb.blob_digest = b.digest) \
             LIMIT $2",
            older_than,
            SWEEP_BATCH_LIMIT,
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;

        let mut blobs_removed = 0;
        for raw in candidates {
            let Ok(digest) = Digest::parse(&raw) else { continue };
            match self.delete_row_if_unreferenced(&digest, older_than).await {
                Ok(Some(storage_key)) => {
                    blobs_removed += 1;
                    if let Err(e) = self.remove_file_if_still_absent(&digest, &storage_key).await {
                        tracing::warn!(digest = %digest.as_str(), storage_key, error = %e, "unreferenced blob row removed but its file could not be, leaving the file behind");
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(digest = %digest.as_str(), error = %e, "failed to sweep an unreferenced blob"),
            }
        }
        Ok(BlobSweepReport { links_removed, blobs_removed, counts_corrected, temp_files_removed })
    }
}

/// Removes `*.tmp-*` files under `root` not touched for `max_age`. Failures are logged and skipped: this is housekeeping.
async fn remove_stale_temp_files(root: &std::path::Path, max_age: std::time::Duration) -> usize {
    let mut removed = 0;
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let Ok(mut entries) = fs::read_dir(&directory).await else { continue };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(file_type) = entry.file_type().await else { continue };
            if file_type.is_dir() {
                directories.push(entry.path());
                continue;
            }
            if !entry.file_name().to_string_lossy().contains(".tmp-") {
                continue;
            }
            let stale = entry.metadata().await.and_then(|meta| meta.modified()).is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > max_age));
            if !stale {
                continue;
            }
            match fs::remove_file(entry.path()).await {
                Ok(()) => removed += 1,
                Err(e) => tracing::warn!(path = %entry.path().display(), error = %e, "could not remove a stale temp file"),
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One manifest reference given up, the way `PostgresDockerManifestRepository` does it, then the reclaim `DeleteManifestUseCase` follows with.
    async fn release(store: &FilesystemDockerBlobStore, digest: &Digest) -> Result<bool, DomainError> {
        sqlx::query!("UPDATE docker_blobs SET reference_count = reference_count - 1 WHERE digest = $1", digest.as_str()).execute(&store.pool).await.infra_err()?;
        store.delete_if_unreferenced(digest).await
    }

    #[sqlx::test]
    async fn writes_then_reads_back_the_same_bytes(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"layer-bytes");
        store.write(&digest, b"layer-bytes").await.unwrap();

        assert_eq!(store.read(&digest).await.unwrap(), b"layer-bytes");
        assert!(store.exists(&digest).await.unwrap());
    }

    fn stream_of(chunks: Vec<Vec<u8>>) -> ByteStream {
        Box::pin(futures_util::stream::iter(chunks.into_iter().map(|chunk| Ok(bytes::Bytes::from(chunk)))))
    }

    fn leftover_files(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).unwrap().flatten().map(|entry| if entry.path().is_dir() { leftover_files(&entry.path()) } else { 1 }).sum()
    }

    #[sqlx::test]
    async fn a_streamed_blob_is_verified_as_it_is_stored_and_read_back_whole(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let content: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let digest = Digest::of(&content);
        let chunks: Vec<Vec<u8>> = content.chunks(64 * 1024).map(<[u8]>::to_vec).collect();

        let size = store.write_stream(&digest, stream_of(chunks), u64::MAX).await.unwrap();

        assert_eq!(size, content.len() as u64);
        assert_eq!(store.read(&digest).await.unwrap(), content);
        assert_eq!(store.size_if_exists(&digest).await.unwrap(), Some(content.len() as u64));
        assert_eq!(leftover_files(dir.path()), 1, "only the blob itself is on disk");
    }

    #[sqlx::test]
    async fn a_streamed_write_that_is_dropped_midway_leaves_no_temp_file(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"a layer whose download gets cut off");
        let stalled: ByteStream = Box::pin(futures_util::stream::once(std::future::ready(Ok(bytes::Bytes::from_static(b"the first chunk")))).chain(futures_util::stream::pending()));

        let outcome = tokio::time::timeout(std::time::Duration::from_millis(200), store.write_stream(&digest, stalled, u64::MAX)).await;

        assert!(outcome.is_err(), "the stream never finishes, so the write must still be running when it is dropped");
        assert_eq!(leftover_files(dir.path()), 0);
    }

    #[sqlx::test]
    async fn the_sweep_removes_stale_temp_files_and_nothing_else(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let kept = Digest::of(b"a blob that is in use");
        store.write(&kept, b"a blob that is in use").await.unwrap();
        let shard = dir.path().join("sha256/ab/cd");
        std::fs::create_dir_all(&shard).unwrap();
        let old = shard.join(format!("{}.tmp-{}", "ab".repeat(32), Uuid::new_v4()));
        let fresh = shard.join(format!("{}.tmp-{}", "cd".repeat(32), Uuid::new_v4()));
        std::fs::write(&old, b"partial").unwrap();
        std::fs::write(&fresh, b"partial").unwrap();
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(two_hours_ago).unwrap();

        let report = store.sweep_unreferenced_blobs(chrono::Utc::now() - chrono::Duration::hours(48)).await.unwrap();

        assert_eq!(report.temp_files_removed, 1);
        assert!(!old.exists());
        assert!(fresh.exists(), "a write may still be running");
        assert_eq!(store.read(&kept).await.unwrap(), b"a blob that is in use");
    }

    #[sqlx::test]
    async fn a_streamed_blob_with_the_wrong_digest_leaves_nothing_behind(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"what the client asked for");

        let result = store.write_stream(&digest, stream_of(vec![b"something ".to_vec(), b"else".to_vec()]), u64::MAX).await;

        assert!(matches!(result, Err(DomainError::DigestMismatch { .. })), "{result:?}");
        assert!(!store.exists(&digest).await.unwrap());
        assert_eq!(leftover_files(dir.path()), 0);
    }

    #[sqlx::test]
    async fn a_streamed_blob_over_the_limit_leaves_nothing_behind(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let content = vec![7u8; 100];
        let digest = Digest::of(&content);

        let result = store.write_stream(&digest, stream_of(vec![content[..60].to_vec(), content[60..].to_vec()]), 99).await;

        assert!(matches!(result, Err(DomainError::UploadTooLarge)), "{result:?}");
        assert!(!store.exists(&digest).await.unwrap());
        assert_eq!(leftover_files(dir.path()), 0);
    }

    #[sqlx::test]
    async fn a_stream_that_breaks_half_way_leaves_nothing_behind(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"never fully arrives");
        let body: ByteStream = Box::pin(futures_util::stream::iter(vec![Ok(bytes::Bytes::from_static(b"never ")), Err(DomainError::Infrastructure("connection reset".to_string()))]));

        let result = store.write_stream(&digest, body, u64::MAX).await;

        assert!(result.is_err());
        assert!(!store.exists(&digest).await.unwrap());
        assert_eq!(leftover_files(dir.path()), 0);
    }

    #[sqlx::test]
    async fn writing_the_same_digest_twice_is_a_safe_no_op(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"layer-bytes");
        store.write(&digest, b"layer-bytes").await.unwrap();
        store.increment_ref(&digest).await.unwrap();
        store.write(&digest, b"layer-bytes").await.unwrap();
        store.increment_ref(&digest).await.unwrap();

        assert!(!release(&store, &digest).await.unwrap());
        assert!(store.exists(&digest).await.unwrap());
    }

    #[sqlx::test]
    async fn decrementing_to_zero_deletes_the_row_and_the_file(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"layer-bytes");
        store.write(&digest, b"layer-bytes").await.unwrap();
        store.increment_ref(&digest).await.unwrap();

        assert!(release(&store, &digest).await.unwrap());
        assert!(!store.exists(&digest).await.unwrap());
        assert!(store.read(&digest).await.is_err());
    }

    #[sqlx::test]
    async fn a_blob_with_two_references_survives_one_decrement(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"shared-base-layer");
        store.write(&digest, b"shared-base-layer").await.unwrap();
        store.increment_ref(&digest).await.unwrap();
        store.increment_ref(&digest).await.unwrap();

        assert!(!release(&store, &digest).await.unwrap());
        assert!(store.exists(&digest).await.unwrap());
    }

    /// Mirrors what the repository deletion sweep itself already did, transactionally, before ever
    /// calling this: `docker_blobs` row gone, file still on disk. `remove_reclaimed_blob_files` must
    /// re-derive the storage key from the digest alone (no DB round trip needed for that) and remove
    /// the file once its re-check confirms the row is still absent.
    #[sqlx::test]
    async fn remove_reclaimed_blob_files_removes_the_file_once_its_row_is_already_gone(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let digest = Digest::of(b"layer-bytes");
        store.write(&digest, b"layer-bytes").await.unwrap();
        let storage_key = FilesystemDockerBlobStore::storage_key(FilesystemDockerBlobStore::hex_part(&digest).unwrap());
        assert!(dir.path().join(&storage_key).exists(), "sanity check: the file must exist before the row is removed");

        sqlx::query!("DELETE FROM docker_blobs WHERE digest = $1", digest.as_str()).execute(&pool).await.unwrap();

        store.remove_reclaimed_blob_files(&[digest.clone()]).await;

        assert!(!dir.path().join(&storage_key).exists(), "the file must be removed once its row is confirmed gone");
    }

    #[sqlx::test]
    async fn remove_reclaimed_blob_files_silently_skips_a_digest_with_no_file(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());

        store.remove_reclaimed_blob_files(&[Digest::of(b"never-written")]).await;
    }

    /// Every existing test here passes a single-element slice — this pins that each digest in a
    /// multi-digest batch is removed independently of the others.
    #[sqlx::test]
    async fn remove_reclaimed_blob_files_removes_every_digest_in_a_multi_digest_batch(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let digest_a = Digest::of(b"multi-batch-layer-a");
        let digest_b = Digest::of(b"multi-batch-layer-b");
        store.write(&digest_a, b"multi-batch-layer-a").await.unwrap();
        store.write(&digest_b, b"multi-batch-layer-b").await.unwrap();
        let storage_key_a = FilesystemDockerBlobStore::storage_key(FilesystemDockerBlobStore::hex_part(&digest_a).unwrap());
        let storage_key_b = FilesystemDockerBlobStore::storage_key(FilesystemDockerBlobStore::hex_part(&digest_b).unwrap());

        sqlx::query!("DELETE FROM docker_blobs WHERE digest = $1", digest_a.as_str()).execute(&pool).await.unwrap();
        sqlx::query!("DELETE FROM docker_blobs WHERE digest = $1", digest_b.as_str()).execute(&pool).await.unwrap();

        store.remove_reclaimed_blob_files(&[digest_a.clone(), digest_b.clone()]).await;

        assert!(!dir.path().join(&storage_key_a).exists(), "the first digest's file must be removed");
        assert!(!dir.path().join(&storage_key_b).exists(), "the second digest's file must be removed too, independently");
        assert!(!store.exists(&digest_a).await.unwrap());
        assert!(!store.exists(&digest_b).await.unwrap());
    }

    /// Race regression test: the sweep's own transaction has already committed the `docker_blobs`
    /// row's deletion by the time `remove_reclaimed_blob_files` is called — like the test above — but
    /// here a concurrent `write` for the SAME digest lands in the gap between that commit and this
    /// call. Sequential `.await`s are enough to force the interleaving deterministically, the same
    /// "direct-call" technique `a_concurrent_write_landing_between_phase_1_and_phase_2_...` uses for
    /// `release`'s own two phases — `remove_reclaimed_blob_files` reuses
    /// `remove_file_if_still_absent` unchanged, so the same protection applies here too.
    #[sqlx::test]
    async fn a_concurrent_write_landing_before_remove_reclaimed_blob_files_is_detected_and_the_file_survives(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let digest = Digest::of(b"sweep-race-repro");
        store.write(&digest, b"original-bytes").await.unwrap();

        sqlx::query!("DELETE FROM docker_blobs WHERE digest = $1", digest.as_str()).execute(&pool).await.unwrap();

        store.write(&digest, b"new-bytes-from-concurrent-write").await.unwrap();

        store.remove_reclaimed_blob_files(&[digest.clone()]).await;

        assert!(store.exists(&digest).await.unwrap(), "the write, which landed in the gap, must have recreated the row");
        assert_eq!(
            store.read(&digest).await.unwrap(),
            b"new-bytes-from-concurrent-write",
            "the file backing the recreated row must be the write's own bytes — remove_reclaimed_blob_files must have skipped removing it"
        );
    }

    /// Regression test for the fix that stops a phase-2 (file-removal) failure from propagating as
    /// an `Err` out of `release`, even though phase 1 (the decrement +
    /// row delete) is already durably committed by the time phase 2 could possibly fail —
    /// `docker_manifest_delete.rs`'s loop over a manifest's blob digests calls this once per
    /// digest with `?`, and used to abort entirely (skipping every later digest's own decrement)
    /// the instant one digest's phase 2 failed.
    ///
    /// The failure is genuinely induced, not fabricated: a second, deliberately tiny pool (one
    /// connection, a short acquire timeout) points at the SAME database. A bystander, queued for
    /// that pool's one connection while phase 1 still holds it checked out (confirmed blocked on
    /// the digest's advisory lock, the same technique the tests above use), is guaranteed — by
    /// sqlx's FIFO connection handoff — to be granted the connection the instant phase 1 commits
    /// and releases it, ahead of phase 2's own later request. Phase 2's `pool.begin()` then
    /// genuinely times out waiting for a connection: exactly the "transient DB connectivity
    /// issue" shape this fix targets.
    #[sqlx::test]
    async fn a_phase_2_connection_failure_does_not_lose_phase_1s_already_committed_decrement(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let digest = Digest::of(b"phase-2-connection-failure-repro");

        let starved_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_millis(300))
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();
        let store = FilesystemDockerBlobStore::new(starved_pool.clone(), dir.path());
        store.write(&digest, b"phase-2-connection-failure-repro").await.unwrap();
        store.increment_ref(&digest).await.unwrap();

        let mut blocker_tx = pool.begin().await.unwrap();
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest.as_str()).execute(&mut *blocker_tx).await.unwrap();

        let decrement_handle = {
            let store_pool = starved_pool.clone();
            let dir_path = dir.path().to_path_buf();
            let digest = digest.clone();
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                rt_handle.block_on(async move {
                    let store = FilesystemDockerBlobStore::new(store_pool, dir_path);
                    release(&store, &digest).await
                })
            })
        };

        let mut observed_phase_1_blocked = false;
        for _ in 0..500 {
            let blocked: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%pg_advisory_xact_lock%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if blocked.0 >= 1 {
                observed_phase_1_blocked = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(observed_phase_1_blocked, "phase 1 never blocked on the digest's advisory lock — this test isn't exercising the real code path");

        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let bystander_handle = {
            let starved_pool = starved_pool.clone();
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                rt_handle.block_on(async move {
                    let _ = queued_tx.send(());
                    starved_pool.acquire().await
                })
            })
        };
        queued_rx.await.unwrap();

        blocker_tx.commit().await.unwrap();

        let bystander_conn = bystander_handle.await.unwrap().unwrap();

        let decrement_result = decrement_handle.await.unwrap();
        assert!(
            decrement_result.is_ok(),
            "phase 2's connection-acquisition failure must not propagate as an Err out of release — got {decrement_result:?}"
        );
        assert!(decrement_result.unwrap(), "phase 1's decrement + delete must still be reported as done, even though phase 2 (starved of a connection) failed");

        drop(bystander_conn);

        assert!(!store.exists(&digest).await.unwrap(), "phase 1's DELETE must remain durably committed despite phase 2's connection failure");
    }

    #[sqlx::test]
    async fn exists_reports_false_for_an_unwritten_digest(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        assert!(!store.exists(&Digest::of(b"never-written")).await.unwrap());
    }

    #[sqlx::test]
    async fn size_if_exists_returns_the_real_byte_length_without_reading_the_blob(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"layer-bytes");
        store.write(&digest, b"layer-bytes").await.unwrap();

        assert_eq!(store.size_if_exists(&digest).await.unwrap(), Some(b"layer-bytes".len() as u64));
    }

    #[sqlx::test]
    async fn existing_digests_returns_only_the_ones_actually_present(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let present = Digest::of(b"present-blob");
        let missing = Digest::of(b"missing-blob");
        store.write(&present, b"present-blob").await.unwrap();

        let found = store.existing_digests(&[present.clone(), missing]).await.unwrap();

        assert_eq!(found, std::collections::HashSet::from([present.as_str().to_string()]));
    }

    #[sqlx::test]
    async fn sum_sizes_adds_only_the_digests_that_exist(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let a = Digest::of(b"aaaaa"); // 5 bytes
        let b = Digest::of(b"bbbbbbbbbb"); // 10 bytes
        let missing = Digest::of(b"never-written");
        store.write(&a, b"aaaaa").await.unwrap();
        store.write(&b, b"bbbbbbbbbb").await.unwrap();

        let total = store.sum_sizes(&[a, b, missing]).await.unwrap();

        assert_eq!(total, 15);
    }

    #[sqlx::test]
    async fn increment_ref_all_bumps_every_given_digest_by_exactly_one(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let a = Digest::of(b"blob-a");
        let b = Digest::of(b"blob-b");
        store.write(&a, b"blob-a").await.unwrap();
        store.write(&b, b"blob-b").await.unwrap();

        store.increment_ref_all(&[a.clone(), b.clone()]).await.unwrap();

        assert!(release(&store, &a).await.unwrap());
        assert!(release(&store, &b).await.unwrap());
    }

    #[sqlx::test]
    async fn size_if_exists_returns_none_for_an_unwritten_digest(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        assert_eq!(store.size_if_exists(&Digest::of(b"never-written")).await.unwrap(), None);
    }

    async fn seed_repository(pool: &sqlx::PgPool) -> Uuid {
        let repository_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'docker', 'hosted', 1, now(), now())",
            repository_id,
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            format!("repo-{repository_id}"),
        )
        .execute(pool)
        .await
        .unwrap();
        repository_id
    }

    async fn seed_manifest_with_blobs(pool: &sqlx::PgPool, repository_id: Uuid, blob_digests: &[&Digest]) {
        let manifest_id = Uuid::new_v4();
        let manifest_digest = Digest::of(manifest_id.as_bytes());
        sqlx::query!(
            "INSERT INTO docker_manifests (id, package_repository_id, image_name, digest, media_type, body, created_at) \
             VALUES ($1, $2, 'myimage', $3, 'application/vnd.docker.distribution.manifest.v2+json', '{}', now())",
            manifest_id,
            repository_id,
            manifest_digest.as_str(),
        )
        .execute(pool)
        .await
        .unwrap();
        for digest in blob_digests {
            sqlx::query!(
                "INSERT INTO docker_manifest_blobs (manifest_id, blob_digest) VALUES ($1, $2)",
                manifest_id,
                digest.as_str(),
            )
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[sqlx::test]
    async fn used_bytes_for_repository_sums_distinct_blobs_referenced_by_its_manifests(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;

        let base_layer = Digest::of(b"base-layer");
        let app_layer = Digest::of(b"app-layer");
        store.write(&base_layer, b"base-layer").await.unwrap(); // 10 bytes
        store.write(&app_layer, b"app-layer-content").await.unwrap(); // 17 bytes

        seed_manifest_with_blobs(&pool, repository_id, &[&base_layer, &app_layer]).await;
        seed_manifest_with_blobs(&pool, repository_id, &[&base_layer]).await;

        let used = store.used_bytes_for_repository(repository_id).await.unwrap();
        assert_eq!(used, b"base-layer".len() as u64 + b"app-layer-content".len() as u64 + 2 * 2);
    }

    #[sqlx::test]
    async fn manifest_bodies_and_tags_count_against_the_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;
        let other_repository_id = seed_repository(&pool).await;
        seed_manifest_with_blobs(&pool, repository_id, &[]).await;
        seed_manifest_with_blobs(&pool, other_repository_id, &[]).await;
        for tag in ["v1", "v2", "v3"] {
            sqlx::query!("INSERT INTO docker_tags (package_repository_id, image_name, tag, manifest_id) SELECT $1, 'myimage', $2, id FROM docker_manifests WHERE package_repository_id = $1", repository_id, tag)
                .execute(&pool)
                .await
                .unwrap();
        }

        let expected = 2 + 3 * DOCKER_TAG_QUOTA_BYTES as u64;
        assert_eq!(store.used_bytes_for_repository(repository_id).await.unwrap(), expected);
        let batch = store.used_bytes_for_repositories(&[repository_id, other_repository_id]).await.unwrap();
        assert_eq!(batch.get(&repository_id), Some(&expected));
        assert_eq!(batch.get(&other_repository_id), Some(&2));
    }

    #[sqlx::test]
    async fn used_bytes_for_repository_ignores_manifests_in_other_repositories(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;
        let other_repository_id = seed_repository(&pool).await;

        let digest = Digest::of(b"only-in-other-repo");
        store.write(&digest, b"only-in-other-repo").await.unwrap();
        seed_manifest_with_blobs(&pool, other_repository_id, &[&digest]).await;

        assert_eq!(store.used_bytes_for_repository(repository_id).await.unwrap(), 0);
    }

    #[sqlx::test]
    async fn used_bytes_for_repository_is_zero_for_a_repository_with_no_manifests(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;

        assert_eq!(store.used_bytes_for_repository(repository_id).await.unwrap(), 0);
    }

    /// Repo A's last (manifest-backed) reference to a digest is removed while repo B still holds
    /// a link-only row for the SAME digest — no manifest, never incremented, exactly what a proxy
    /// repository's blob cache leaves behind (see `docker_blob_get.rs`'s `execute_proxy`, which
    /// links a fetched blob without ever calling `increment_ref`). An unconditional DELETE would
    /// hit B's link row's FK and roll the WHOLE transaction back — silently losing A's decrement
    /// too, forever, since nothing else would ever retry it. The row must survive (B's link still
    /// legitimately needs it reachable) but the decrement itself must land: `reference_count`
    /// must reach exactly 0, not be rolled back to 1.
    #[sqlx::test]
    async fn decrementing_the_last_real_reference_still_commits_even_when_a_different_repositorys_stale_link_survives(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repo_b = seed_repository(&pool).await;
        let digest = Digest::of(b"finding1-repro");
        store.write(&digest, b"finding1-repro").await.unwrap();
        store.increment_ref(&digest).await.unwrap(); // repo A's real (manifest-backed) reference
        store.link_to_repository(repo_b, &digest).await.unwrap(); // repo B's link-only row, never incremented

        let deleted = release(&store, &digest).await.unwrap();
        assert!(!deleted, "must not be deleted while repo B's link row exists");
        assert!(store.exists(&digest).await.unwrap(), "row must survive — repo B's link still needs it reachable");

        let row = sqlx::query!("SELECT reference_count FROM docker_blobs WHERE digest = $1", digest.as_str()).fetch_one(&pool).await.unwrap();
        assert_eq!(row.reference_count, 0, "the decrement must not be rolled back just because another repository's link row still exists");

        // Once repo B's stale link is ALSO removed (the real-world trigger: no manifest in repo B ever
        // referenced this digest, so `unlink_from_repository_if_unreferenced` clears it — the same call
        // `DeleteManifestUseCase` makes for its own repository), the blob is genuinely reclaimable.
        // `reference_count` is already at 0 from the decrement above, so this mirrors the hard-delete
        // sweep's own retry shape (`package_repository_store.rs`) rather than calling
        // `release` a second time, which would correspond to no real caller —
        // nothing decrements twice for one reference.
        store.unlink_from_repository_if_unreferenced(repo_b, &digest).await.unwrap();
        let reclaimed = sqlx::query!(
            "DELETE FROM docker_blobs WHERE digest = $1 AND reference_count <= 0 \
             AND NOT EXISTS (SELECT 1 FROM docker_repository_blobs WHERE blob_digest = $1) \
             RETURNING storage_key",
            digest.as_str()
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(reclaimed.is_some(), "once no repository's link references it anymore, the blob must be genuinely reclaimable");
    }

    /// The FK race the savepoint above guards against, reproduced with two real,
    /// separately-connected transactions and explicit commit ordering rather than a sleep-based
    /// guess. `link_to_repository`'s INSERT is started and deliberately left uncommitted on its
    /// own connection for the whole test — this holds the FK's `FOR KEY SHARE` lock on the
    /// `docker_blobs` row the whole time, so it is guaranteed to still be uncommitted (and thus
    /// invisible to `NOT EXISTS`, under read-committed) when the decrement's guard runs, and still
    /// be holding the lock when the decrement's DELETE tries to acquire it. The decrement itself
    /// runs concurrently on a genuinely separate OS thread and pool connection (`spawn_blocking`,
    /// mirroring `docker_manifest_put.rs`'s real-concurrency race test — a plain `tokio::join!` on
    /// this test's single-threaded runtime would just run the two operations back to back and
    /// never contend). A third connection polls `pg_stat_activity` until it observes the
    /// decrement's backend genuinely blocked waiting for that lock — not a fixed sleep, so this
    /// can't spuriously pass by finishing before the race is even set up — and only then commits
    /// the link. That ordering guarantees Postgres's end-of-statement FK check fires while the
    /// DELETE is still in flight. Without the savepoint, this FK violation would propagate out of
    /// the whole transaction and roll the decrement back with it (`reference_count` stuck at 1,
    /// and a 500 out of `DeleteManifestUseCase`); with it, only the DELETE's savepoint rolls back,
    /// so the call returns `Ok(false)` and the decrement survives.
    #[sqlx::test]
    async fn a_concurrent_link_landing_between_the_guard_and_the_delete_does_not_lose_the_decrement(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repo_b = seed_repository(&pool).await;
        let digest = Digest::of(b"n1-repro");
        store.write(&digest, b"n1-repro").await.unwrap();
        store.increment_ref(&digest).await.unwrap();

        let mut linker_tx = pool.begin().await.unwrap();
        sqlx::query!(
            "INSERT INTO docker_repository_blobs (package_repository_id, blob_digest) VALUES ($1, $2) \
             ON CONFLICT (package_repository_id, blob_digest) DO NOTHING",
            repo_b,
            digest.as_str(),
        )
        .execute(&mut *linker_tx)
        .await
        .unwrap();

        let decrement_handle = {
            let store_pool = pool.clone();
            let dir_path = dir.path().to_path_buf();
            let digest = digest.clone();
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                rt_handle.block_on(async move {
                    let store = FilesystemDockerBlobStore::new(store_pool, dir_path);
                    release(&store, &digest).await
                })
            })
        };

        let mut observed_blocked = false;
        for _ in 0..500 {
            let blocked: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%DELETE FROM docker_blobs%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if blocked.0 > 0 {
                observed_blocked = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(observed_blocked, "the decrement's DELETE never entered a lock wait — this test isn't actually exercising the race it claims to");

        linker_tx.commit().await.unwrap();

        let result = decrement_handle.await.unwrap();
        assert!(result.is_ok(), "the concurrent link's FK violation must not surface as an error to the caller — got {result:?}");
        assert!(!result.unwrap(), "the row is not reclaimable while repo B's link now exists");

        let row = sqlx::query!("SELECT reference_count FROM docker_blobs WHERE digest = $1", digest.as_str()).fetch_one(&pool).await.unwrap();
        assert_eq!(row.reference_count, 0, "the decrement must survive even though its DELETE hit the concurrent link's FK");
        assert!(store.exists(&digest).await.unwrap(), "the row and its file must survive — repo B's link makes it reachable again");
        assert!(store.read(&digest).await.is_ok(), "the file itself must still be there, not just the row");
    }

    /// A real-concurrency counterpart to the more targeted
    /// `a_concurrent_write_landing_between_phase_1_and_phase_2_is_detected_and_the_file_survives`
    /// below. This test drives the REAL, public `release` and the REAL
    /// `write` as genuinely concurrent, separately-connected background tasks, rather than calling
    /// the two internal phases directly — proving the wired-up public method produces the right
    /// outcome under real interleaving (the phase-2 re-check logic itself is covered in isolation
    /// by the more targeted test below).
    ///
    /// Phase 1 commits and releases the digest lock as soon as the row is deleted, which lets the
    /// already-queued `write` (queued behind `blocker_tx` before phase 1 even started, confirmed by
    /// the polls below) get granted the lock next — strictly before the decrement's own phase 2 can
    /// re-request it, since phase 2's request necessarily happens after phase 1's commit, i.e. after
    /// `write`'s request was already queued. Postgres's FIFO lock grants make this deterministic:
    /// `write` lands in the phase-1-to-phase-2 gap every time.
    ///
    /// This test controls the race deterministically with its own separately-held advisory lock
    /// (`blocker_tx`, the SAME `pg_advisory_xact_lock(hashtext($1))` key as the digest lock under
    /// test) — mirroring
    /// `a_concurrent_link_landing_between_the_guard_and_the_delete_does_not_lose_the_decrement`'s use
    /// of an explicit, manually-committed transaction as a deterministic gate. Both
    /// `release` and `write` run as REAL, separately-connected background
    /// tasks (`spawn_blocking`, for genuine OS-thread concurrency — see that same test's comment on
    /// why a plain `tokio::join!` wouldn't interleave). `blocker_tx` acquires the digest's advisory
    /// lock FIRST, so both background tasks queue up behind it; a `pg_stat_activity` poll confirms the
    /// decrement is blocked, then a second poll confirms the write ALSO joins the same lock's wait
    /// queue before `blocker_tx` releases the lock.
    #[sqlx::test]
    async fn a_concurrent_write_cannot_land_in_the_window_between_a_delete_and_its_file_removal(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let digest = Digest::of(b"round3-repro");
        store.write(&digest, b"original-bytes").await.unwrap();
        store.increment_ref(&digest).await.unwrap();

        let mut blocker_tx = pool.begin().await.unwrap();
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest.as_str()).execute(&mut *blocker_tx).await.unwrap();

        let decrement_handle = {
            let store_pool = pool.clone();
            let dir_path = dir.path().to_path_buf();
            let digest = digest.clone();
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                rt_handle.block_on(async move {
                    let store = FilesystemDockerBlobStore::new(store_pool, dir_path);
                    release(&store, &digest).await
                })
            })
        };

        let mut observed_decrement_blocked = false;
        for _ in 0..500 {
            let blocked: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%pg_advisory_xact_lock%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if blocked.0 >= 1 {
                observed_decrement_blocked = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            observed_decrement_blocked,
            "the decrement never entered a lock wait on the digest's advisory lock — this test isn't exercising the race it claims to"
        );

        let write_handle = {
            let store_pool = pool.clone();
            let dir_path = dir.path().to_path_buf();
            let digest = digest.clone();
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                rt_handle.block_on(async move {
                    let store = FilesystemDockerBlobStore::new(store_pool, dir_path);
                    store.write(&digest, b"new-bytes-from-concurrent-write").await
                })
            })
        };

        let mut observed_both_blocked = false;
        for _ in 0..500 {
            let blocked: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%pg_advisory_xact_lock%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if blocked.0 >= 2 {
                observed_both_blocked = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            observed_both_blocked,
            "the write never joined the decrement's lock wait queue — this test isn't exercising genuine concurrency"
        );

        blocker_tx.commit().await.unwrap();

        let decrement_result = decrement_handle.await.unwrap();
        assert!(decrement_result.unwrap(), "the blob had exactly one reference and no links — phase 1 must have deleted the row");

        let write_result = write_handle.await.unwrap();
        assert!(write_result.is_ok(), "the write must succeed once phase 1's transaction (and the digest lock) is released — got {write_result:?}");

        assert!(store.exists(&digest).await.unwrap(), "the write, which ran after phase 1's delete, must have recreated the row");
        assert_eq!(
            store.read(&digest).await.unwrap(),
            b"new-bytes-from-concurrent-write",
            "the file backing the recreated row must be the write's own bytes, not lost to phase 2's removal"
        );
    }

    /// `release` is two separately-transacted phases —
    /// `delete_row_if_unreferenced_now` (phase 1, commits and releases the digest lock the
    /// moment the row is deleted) and `remove_file_if_still_absent` (phase 2, re-acquires the SAME
    /// lock and re-checks before touching the file) — specifically so the gap between them can be
    /// hit deterministically, with no lock gate, no `pg_stat_activity` polling, and no background
    /// tasks: it's just two sequential `.await`s here, with a real concurrent `write` call inserted
    /// between them. Phase 1's transaction has genuinely committed (and released the lock) by the
    /// time the `write` below runs — matching real production behavior, where nothing blocks a
    /// concurrent `write` from landing in exactly this window — and phase 2 hasn't yet re-acquired
    /// it, so this is the exact interleaving the original bug depended on, forced rather than raced.
    #[sqlx::test]
    async fn a_concurrent_write_landing_between_phase_1_and_phase_2_is_detected_and_the_file_survives(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let digest = Digest::of(b"round4-repro");
        store.write(&digest, b"original-bytes").await.unwrap();
        store.increment_ref(&digest).await.unwrap();

        sqlx::query!("UPDATE docker_blobs SET reference_count = reference_count - 1 WHERE digest = $1", digest.as_str()).execute(&pool).await.unwrap();
        let deleted_storage_key = store
            .delete_row_if_unreferenced_now(&digest)
            .await
            .unwrap()
            .expect("the blob had exactly one reference and no links — phase 1 must have deleted the row");

        store.write(&digest, b"new-bytes-from-concurrent-write").await.unwrap();

        store.remove_file_if_still_absent(&digest, &deleted_storage_key).await.unwrap();

        assert!(store.exists(&digest).await.unwrap(), "the write, which landed in the phase 1/phase 2 gap, must have recreated the row");
        assert_eq!(
            store.read(&digest).await.unwrap(),
            b"new-bytes-from-concurrent-write",
            "the file backing the recreated row must be the write's own bytes — phase 2 must have skipped removing it"
        );
    }

    /// Proves `write`'s rename and INSERT genuinely happen only once it holds the digest lock —
    /// not just that `write` eventually succeeds. Neither test above would catch the rename
    /// moving to run before lock acquisition: the phase-based test only calls `write` after
    /// phase 1 has already released the lock, and the blocker test only checks `write`'s eventual
    /// result, never when the file itself materializes relative to the lock. Here the digest lock
    /// is held by `blocker_tx` first, so a concurrent `write` must join its wait queue; while it's
    /// confirmed waiting, neither the file nor the `docker_blobs` row may exist yet.
    #[sqlx::test]
    async fn a_write_does_not_create_the_file_or_row_before_it_holds_the_digest_lock(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let digest = Digest::of(b"write-lock-boundary-repro");
        let storage_key = FilesystemDockerBlobStore::storage_key(FilesystemDockerBlobStore::hex_part(&digest).unwrap());
        let target_path = dir.path().join(&storage_key);

        let mut blocker_tx = pool.begin().await.unwrap();
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest.as_str()).execute(&mut *blocker_tx).await.unwrap();

        let write_handle = {
            let store_pool = pool.clone();
            let dir_path = dir.path().to_path_buf();
            let digest = digest.clone();
            let rt_handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                rt_handle.block_on(async move {
                    let store = FilesystemDockerBlobStore::new(store_pool, dir_path);
                    store.write(&digest, b"payload").await
                })
            })
        };

        let mut observed_blocked = false;
        for _ in 0..500 {
            let blocked: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%pg_advisory_xact_lock%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if blocked.0 >= 1 {
                observed_blocked = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(observed_blocked, "the write never joined the digest lock's wait queue — this test isn't exercising the boundary it claims to");

        assert!(!target_path.exists(), "the rename must not happen before the write holds the digest lock");
        assert!(!store.exists(&digest).await.unwrap(), "the row must not appear before the write holds the digest lock");

        blocker_tx.commit().await.unwrap();

        write_handle.await.unwrap().unwrap();

        assert!(store.exists(&digest).await.unwrap(), "the write must complete once the lock is released");
        assert_eq!(store.read(&digest).await.unwrap(), b"payload", "the file must contain the write's own bytes");
    }

    /// Reproduces the bug this fix addresses, at the SQL level: a `docker_repository_blobs` link row
    /// created at upload time (independent of any manifest) survives on its own once the manifest
    /// that used to make it non-stale is gone — here simulated by deleting the manifest row directly,
    /// the same state `DeleteManifestUseCase` leaves behind. Before this fix existed at all,
    /// `release` would hit this row's FK and roll its own decrement back
    /// forever; `unlink_from_repository_if_unreferenced` is what makes it possible to first remove the
    /// now-stale row so the decrement can actually succeed.
    #[sqlx::test]
    async fn unlink_from_repository_if_unreferenced_removes_a_stale_link_row(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;
        let digest = Digest::of(b"orphaned-after-manifest-delete");
        store.write(&digest, b"orphaned-after-manifest-delete").await.unwrap();
        store.link_to_repository(repository_id, &digest).await.unwrap();

        store.unlink_from_repository_if_unreferenced(repository_id, &digest).await.unwrap();

        assert!(!store.is_uploaded_to_repository(repository_id, &digest).await.unwrap(), "the stale link row must be gone");
    }

    /// The other half of the same behavior: a link row backed by a manifest that's still very much
    /// there must survive — this is the "don't reintroduce premature deletion" case. A blob genuinely
    /// still in use (reachable via `docker_manifest_blobs`) must never lose its repository link just
    /// because some OTHER, unrelated digest's manifest got deleted.
    #[sqlx::test]
    async fn unlink_from_repository_if_unreferenced_leaves_a_still_referenced_link_row_alone(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;
        let digest = Digest::of(b"still-in-a-live-manifest");
        store.write(&digest, b"still-in-a-live-manifest").await.unwrap();
        store.link_to_repository(repository_id, &digest).await.unwrap();
        seed_manifest_with_blobs(&pool, repository_id, &[&digest]).await;

        store.unlink_from_repository_if_unreferenced(repository_id, &digest).await.unwrap();

        assert!(store.is_uploaded_to_repository(repository_id, &digest).await.unwrap(), "a link backed by a live manifest reference must survive");
    }

    async fn age(pool: &sqlx::PgPool, repository_id: Uuid, digest: &Digest, hours: i32) {
        sqlx::query!("UPDATE docker_repository_blobs SET created_at = now() - make_interval(hours => $3) WHERE package_repository_id = $1 AND blob_digest = $2", repository_id, digest.as_str(), hours)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query!("UPDATE docker_blobs SET created_at = now() - make_interval(hours => $2) WHERE digest = $1", digest.as_str(), hours).execute(pool).await.unwrap();
    }

    async fn blob_file_exists(dir: &std::path::Path, digest: &Digest) -> bool {
        let hex = digest.as_str().strip_prefix("sha256:").unwrap();
        dir.join(FilesystemDockerBlobStore::storage_key(hex)).exists()
    }

    #[sqlx::test]
    async fn used_bytes_count_uploaded_blobs_no_manifest_references_once_each(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;
        let other_repository_id = seed_repository(&pool).await;
        let referenced = Digest::of(b"referenced-and-linked");
        let uploaded_only = Digest::of(b"uploaded-only");
        let elsewhere = Digest::of(b"linked-elsewhere");
        for (digest, bytes) in [(&referenced, &b"referenced-and-linked"[..]), (&uploaded_only, b"uploaded-only"), (&elsewhere, b"linked-elsewhere")] {
            store.write(digest, bytes).await.unwrap();
        }
        store.link_to_repository(repository_id, &referenced).await.unwrap();
        store.link_to_repository(repository_id, &uploaded_only).await.unwrap();
        store.link_to_repository(other_repository_id, &elsewhere).await.unwrap();
        seed_manifest_with_blobs(&pool, repository_id, &[&referenced]).await;

        let expected = (b"referenced-and-linked".len() + b"uploaded-only".len() + 2) as u64;
        assert_eq!(store.used_bytes_for_repository(repository_id).await.unwrap(), expected);
        let batch = store.used_bytes_for_repositories(&[repository_id, other_repository_id]).await.unwrap();
        assert_eq!(batch.get(&repository_id), Some(&expected));
        assert_eq!(batch.get(&other_repository_id), Some(&(b"linked-elsewhere".len() as u64)));
    }

    #[sqlx::test]
    async fn the_sweep_reclaims_only_old_links_and_blobs_nothing_references(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;
        let sibling_id = seed_repository(&pool).await;
        let abandoned = Digest::of(b"abandoned-upload");
        let referenced = Digest::of(b"referenced-by-a-manifest");
        let fresh = Digest::of(b"still-waiting-for-its-manifest");
        let shared = Digest::of(b"abandoned-here-but-linked-there");
        let orphan = Digest::of(b"no-link-at-all");
        for (digest, bytes) in [
            (&abandoned, &b"abandoned-upload"[..]),
            (&referenced, b"referenced-by-a-manifest"),
            (&fresh, b"still-waiting-for-its-manifest"),
            (&shared, b"abandoned-here-but-linked-there"),
            (&orphan, b"no-link-at-all"),
        ] {
            store.write(digest, bytes).await.unwrap();
        }
        for digest in [&abandoned, &referenced, &fresh, &shared] {
            store.link_to_repository(repository_id, digest).await.unwrap();
        }
        store.link_to_repository(sibling_id, &shared).await.unwrap();
        seed_manifest_with_blobs(&pool, repository_id, &[&referenced]).await;
        store.increment_ref(&referenced).await.unwrap();
        for digest in [&abandoned, &referenced, &shared] {
            age(&pool, repository_id, digest, 100).await;
        }
        age(&pool, sibling_id, &shared, 1).await;
        sqlx::query!("UPDATE docker_blobs SET created_at = now() - interval '100 hours' WHERE digest = $1", orphan.as_str()).execute(&pool).await.unwrap();

        let report = store.sweep_unreferenced_blobs(chrono::Utc::now() - chrono::Duration::hours(48)).await.unwrap();

        assert_eq!(report.links_removed, 2, "the abandoned link and the shared one, in the repository that never referenced them");
        assert_eq!(report.blobs_removed, 2, "the abandoned blob and the linkless orphan");
        assert!(!store.exists(&abandoned).await.unwrap() && !blob_file_exists(dir.path(), &abandoned).await);
        assert!(!store.exists(&orphan).await.unwrap() && !blob_file_exists(dir.path(), &orphan).await);
        for kept in [&referenced, &fresh, &shared] {
            assert!(store.exists(kept).await.unwrap() && blob_file_exists(dir.path(), kept).await, "{}", kept.as_str());
        }
        assert!(store.is_uploaded_to_repository(sibling_id, &shared).await.unwrap(), "the sibling's own link is untouched");
        assert!(store.is_uploaded_to_repository(repository_id, &referenced).await.unwrap(), "a link backed by a manifest stays");
        assert!(store.is_uploaded_to_repository(repository_id, &fresh).await.unwrap(), "a young link stays");
    }

    /// Otherwise a re-upload of an old, unlinked blob could be swept between its adoption and its link.
    #[sqlx::test]
    async fn re_uploading_a_blob_restarts_its_grace_period_without_touching_its_count(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let digest = Digest::of(b"old-blob-uploaded-again");
        store.write(&digest, b"old-blob-uploaded-again").await.unwrap();
        store.increment_ref(&digest).await.unwrap();
        sqlx::query!("UPDATE docker_blobs SET created_at = now() - interval '100 hours' WHERE digest = $1", digest.as_str()).execute(&pool).await.unwrap();

        store.write(&digest, b"old-blob-uploaded-again").await.unwrap();

        let row = sqlx::query!("SELECT reference_count, created_at FROM docker_blobs WHERE digest = $1", digest.as_str()).fetch_one(&pool).await.unwrap();
        assert_eq!(row.reference_count, 1);
        assert!(row.created_at > chrono::Utc::now() - chrono::Duration::minutes(5));
    }

    #[sqlx::test]
    async fn the_sweep_recomputes_reference_counts_from_the_manifests(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let repository_id = seed_repository(&pool).await;
        let leaked = Digest::of(b"count-left-too-high");
        let undercounted = Digest::of(b"count-too-low-but-referenced");
        let recent = Digest::of(b"too-young-to-touch");
        for (digest, bytes) in [(&leaked, &b"count-left-too-high"[..]), (&undercounted, b"count-too-low-but-referenced"), (&recent, b"too-young-to-touch")] {
            store.write(digest, bytes).await.unwrap();
        }
        seed_manifest_with_blobs(&pool, repository_id, &[&undercounted]).await;
        sqlx::query!("UPDATE docker_blobs SET reference_count = 3, created_at = now() - interval '100 hours' WHERE digest = $1", leaked.as_str()).execute(&pool).await.unwrap();
        sqlx::query!("UPDATE docker_blobs SET reference_count = 0, created_at = now() - interval '100 hours' WHERE digest = $1", undercounted.as_str()).execute(&pool).await.unwrap();
        sqlx::query!("UPDATE docker_blobs SET reference_count = 7 WHERE digest = $1", recent.as_str()).execute(&pool).await.unwrap();

        let report = store.sweep_unreferenced_blobs(chrono::Utc::now() - chrono::Duration::hours(48)).await.unwrap();

        assert_eq!(report.counts_corrected, 2);
        assert_eq!(report.blobs_removed, 1, "the leaked blob has no manifest behind its count, so it goes");
        assert!(!store.exists(&leaked).await.unwrap() && !blob_file_exists(dir.path(), &leaked).await);
        let counts = sqlx::query!("SELECT digest, reference_count FROM docker_blobs ORDER BY digest").fetch_all(&pool).await.unwrap();
        let count_of = |digest: &Digest| counts.iter().find(|row| row.digest == digest.as_str()).map(|row| row.reference_count);
        assert_eq!(count_of(&undercounted), Some(1));
        assert_eq!(count_of(&recent), Some(7), "a blob still inside the grace period is left alone");
    }

    /// A proxy repository's blob cache is made of exactly such links, with no manifest behind them.
    #[sqlx::test]
    async fn the_sweep_leaves_a_proxy_repositorys_blob_cache_alone(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool.clone(), dir.path());
        let proxy_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'docker', 'proxy', 'https://registry.example.com', 1, now(), now())",
            proxy_id,
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            format!("proxy-{proxy_id}"),
        )
        .execute(&pool)
        .await
        .unwrap();
        let cached = Digest::of(b"cached-upstream-layer");
        store.write(&cached, b"cached-upstream-layer").await.unwrap();
        store.link_to_repository(proxy_id, &cached).await.unwrap();
        age(&pool, proxy_id, &cached, 1000).await;

        let report = store.sweep_unreferenced_blobs(chrono::Utc::now() - chrono::Duration::hours(48)).await.unwrap();

        assert_eq!(report, BlobSweepReport::default());
        assert!(store.is_uploaded_to_repository(proxy_id, &cached).await.unwrap());
    }

    #[sqlx::test]
    async fn adopting_a_staged_file_never_replaces_a_stored_blob_of_the_same_size(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"layer-bytes");
        store.write(&digest, b"layer-bytes").await.unwrap();
        let hex = digest.as_str().strip_prefix("sha256:").unwrap();
        let stored = dir.path().join(FilesystemDockerBlobStore::storage_key(hex));
        let before = std::fs::metadata(&stored).unwrap().modified().unwrap();
        let staged = dir.path().join("staged.upload");
        std::fs::write(&staged, b"layer-bytes").unwrap();

        store.adopt_staged_file(&digest, staged.to_str().unwrap(), 11).await.unwrap();

        assert_eq!(std::fs::metadata(&stored).unwrap().modified().unwrap(), before, "the stored file was not rewritten");
        assert!(!staged.exists(), "the staged copy is dropped");
        assert_eq!(store.read(&digest).await.unwrap(), b"layer-bytes");
    }

    #[sqlx::test]
    async fn adopting_replaces_a_stored_file_of_the_wrong_size_with_the_verified_bytes(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"layer-bytes");
        let hex = digest.as_str().strip_prefix("sha256:").unwrap();
        let stored = dir.path().join(FilesystemDockerBlobStore::storage_key(hex));
        std::fs::create_dir_all(stored.parent().unwrap()).unwrap();
        std::fs::write(&stored, b"truncated").unwrap();
        let staged = dir.path().join("staged.upload");
        std::fs::write(&staged, b"layer-bytes").unwrap();

        store.adopt_staged_file(&digest, staged.to_str().unwrap(), 11).await.unwrap();

        assert_eq!(std::fs::read(&stored).unwrap(), b"layer-bytes");
    }

    #[sqlx::test]
    async fn a_write_that_fails_leaves_no_temp_file_behind(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemDockerBlobStore::new(pool, dir.path());
        let digest = Digest::of(b"layer-bytes");
        let hex = digest.as_str().strip_prefix("sha256:").unwrap();
        let target = dir.path().join(FilesystemDockerBlobStore::storage_key(hex));
        std::fs::create_dir_all(target.join("occupied")).unwrap();

        assert!(store.write(&digest, b"layer-bytes").await.is_err());

        let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap()).unwrap().flatten().filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert!(!store.exists(&digest).await.unwrap());
    }
}
