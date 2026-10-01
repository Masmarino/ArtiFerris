use async_trait::async_trait;
use artiferris_domain::error::{DomainError, EventStoreError};
use crate::error_ext::{InfraErr, StorageErr};
use artiferris_domain::package_repository::{
    HardDeleteSweepResult, PackageRepositoryEvent, PackageRepositoryEventStorePort, PackageRepositoryQueryPort, PackageRepositorySummary,
    RepositoryDeletionSweepPort, RepositoryFormat, RepositoryLockGuard, RepositoryQuotaLockPort, RepositoryType,
};
use artiferris_domain::permission::PermissionEvent;
use artiferris_domain::personal_repository::PersonalProjectProvisioningPort;
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::OwnedMutexGuard;
use uuid::Uuid;

use artiferris_application::keyed_locks::KeyedLocks;
use crate::postgres::permission_store;
use crate::secret_box;

pub struct PostgresPackageRepositoryStore {
    pool: PgPool,
    /// Derives the AES-256 key for `secret_box`, used to encrypt `remote_password` at rest.
    secrets_encryption_key: String,
    /// Waiters for a repository's quota lock queue here, not on a pool connection.
    quota_locks: KeyedLocks<Uuid>,
}

impl PostgresPackageRepositoryStore {
    pub fn new(pool: PgPool, secrets_encryption_key: String) -> Self {
        Self { pool, secrets_encryption_key, quota_locks: KeyedLocks::new() }
    }

    /// A listing must not fail because one row's password cannot be decrypted: that row lists without it, while
    /// `find_by_*`, which proxying uses, still fails.
    fn password_for_listing(&self, repository_id: Uuid, stored: Option<String>) -> Option<String> {
        match secret_box::open_packed(&stored?, &self.secrets_encryption_key, secret_box::PROXY_REMOTE_PASSWORD) {
            Ok(password) => Some(password),
            Err(e) => {
                tracing::error!(%repository_id, "could not decrypt the remote password of a listed repository: {e}");
                None
            }
        }
    }

    fn open_remote_password(&self, repository_id: Uuid, stored: &str) -> Result<String, DomainError> {
        secret_box::open_packed(stored, &self.secrets_encryption_key, secret_box::PROXY_REMOTE_PASSWORD)
            .inspect_err(|e| tracing::error!(%repository_id, "the stored remote password of this proxy repository cannot be read, it cannot reach its upstream until an admin enters it again: {e}"))
    }

    /// Encrypts a `Created` event's `remote_password` before it's ever persisted.
    fn encrypt_secrets(&self, event: PackageRepositoryEvent) -> PackageRepositoryEvent {
        match event {
            PackageRepositoryEvent::Created { repository_id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password } => {
                PackageRepositoryEvent::Created {
                    repository_id,
                    organization_id,
                    name,
                    format,
                    repo_type,
                    remote_url,
                    remote_username,
                    remote_password: remote_password.map(|pw| secret_box::seal_packed(&pw, &self.secrets_encryption_key, secret_box::PROXY_REMOTE_PASSWORD)),
                }
            }
            other => other,
        }
    }

    /// The inverse of `encrypt_secrets`, applied when replaying events out of the journal.
    fn decrypt_secrets(&self, event: PackageRepositoryEvent) -> Result<PackageRepositoryEvent, EventStoreError> {
        match event {
            PackageRepositoryEvent::Created { repository_id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password } => {
                Ok(PackageRepositoryEvent::Created {
                    repository_id,
                    organization_id,
                    name,
                    format,
                    repo_type,
                    remote_url,
                    remote_username,
                    remote_password: remote_password.map(|pw| self.open_remote_password(repository_id, &pw)).transpose().storage_err()?,
                })
            }
            other => Ok(other),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_summary(
        &self,
        id: Uuid,
        organization_id: Uuid,
        name: String,
        format: String,
        repo_type: String,
        remote_url: Option<String>,
        remote_username: Option<String>,
        remote_password: Option<String>,
        quota_bytes: Option<i64>,
        retention_keep_last_n: Option<i32>,
        is_public: bool,
    ) -> Result<PackageRepositorySummary, EventStoreError> {
        let repo_type = repo_type_from_str(&repo_type)
            .ok_or_else(|| EventStoreError::Storage(format!("unknown repo_type {repo_type}")))?;

        let members = if repo_type == RepositoryType::Group {
            sqlx::query_scalar!(
                "SELECT member_repository_id FROM package_repository_group_members WHERE group_repository_id = $1 ORDER BY position",
                id
            )
            .fetch_all(&self.pool)
            .await
            .storage_err()?
        } else {
            Vec::new()
        };

        Ok(PackageRepositorySummary {
            id,
            organization_id,
            name,
            format: format_from_str(&format).ok_or_else(|| EventStoreError::Storage(format!("unknown format {format}")))?,
            repo_type,
            remote_url,
            remote_username,
            remote_password: remote_password.map(|pw| self.open_remote_password(id, &pw)).transpose().storage_err()?,
            group_members: members,
            quota_bytes,
            retention_keep_last_n,
            is_public,
        })
    }
}

fn format_to_str(format: RepositoryFormat) -> &'static str {
    match format {
        RepositoryFormat::Npm => "npm",
        RepositoryFormat::Docker => "docker",
    }
}

fn format_from_str(raw: &str) -> Option<RepositoryFormat> {
    match raw {
        "npm" => Some(RepositoryFormat::Npm),
        "docker" => Some(RepositoryFormat::Docker),
        _ => None,
    }
}

fn repo_type_to_str(repo_type: RepositoryType) -> &'static str {
    match repo_type {
        RepositoryType::Hosted => "hosted",
        RepositoryType::Proxy => "proxy",
        RepositoryType::Group => "group",
    }
}

fn repo_type_from_str(raw: &str) -> Option<RepositoryType> {
    match raw {
        "hosted" => Some(RepositoryType::Hosted),
        "proxy" => Some(RepositoryType::Proxy),
        "group" => Some(RepositoryType::Group),
        _ => None,
    }
}

#[async_trait]
impl PackageRepositoryEventStorePort for PostgresPackageRepositoryStore {
    async fn load(&self, repository_id: Uuid) -> Result<(u64, Vec<PackageRepositoryEvent>), EventStoreError> {
        let rows = sqlx::query!(
            "SELECT payload, version FROM domain_events \
             WHERE aggregate_type = 'PackageRepository' AND aggregate_id = $1 ORDER BY version",
            repository_id.to_string()
        )
        .fetch_all(&self.pool)
        .await
        .storage_err()?;

        let mut version = 0u64;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let event: PackageRepositoryEvent = serde_json::from_value(row.payload).storage_err()?;
            events.push(self.decrypt_secrets(event)?);
            version = row.version as u64;
        }
        Ok((version, events))
    }

    async fn append(
        &self,
        repository_id: Uuid,
        expected_version: u64,
        events: Vec<PackageRepositoryEvent>,
        actor_id: Uuid,
    ) -> Result<(), EventStoreError> {
        let mut tx = self.pool.begin().await.storage_err()?;
        self.append_in_tx(&mut tx, repository_id, expected_version, events, actor_id).await?;
        tx.commit().await.storage_err()?;
        Ok(())
    }
}

impl PostgresPackageRepositoryStore {
    /// `append`'s write logic, reusable by `create_with_owner_grant` inside its own transaction.
    pub(crate) async fn append_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        repository_id: Uuid,
        expected_version: u64,
        events: Vec<PackageRepositoryEvent>,
        actor_id: Uuid,
    ) -> Result<(), EventStoreError> {
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", repository_id.to_string())
            .execute(&mut **tx)
            .await
            .storage_err()?;

        let current_version: i64 = sqlx::query_scalar!(
            "SELECT version FROM domain_events \
             WHERE aggregate_type = 'PackageRepository' AND aggregate_id = $1 ORDER BY version DESC LIMIT 1 FOR UPDATE",
            repository_id.to_string()
        )
        .fetch_optional(&mut **tx)
        .await
        .storage_err()?
        .unwrap_or(0);

        if current_version as u64 != expected_version {
            return Err(EventStoreError::ConcurrencyConflict { expected: expected_version, actual: current_version as u64 });
        }

        let mut next_version = current_version;
        for event in events {
            next_version += 1;
            let event = self.encrypt_secrets(event);
            let payload = serde_json::to_value(&event).storage_err()?;
            let created_in = match &event {
                PackageRepositoryEvent::Created { organization_id, .. } => Some(*organization_id),
                _ => None,
            };
            sqlx::query!(
                "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, actor_id, organization_id) \
                 VALUES ('PackageRepository', $1, $2, $3, $4, $5, COALESCE($6, repository_organization($1)))",
                repository_id.to_string(),
                event.event_type(),
                payload,
                next_version,
                actor_id,
                created_in
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;

            apply_to_projection(tx, repository_id, &event, next_version).await?;
        }

        Ok(())
    }
}

/// Runs each aggregate's append helper in one shared transaction, so the repository and its owner's grant persist
/// together or not at all.
#[async_trait]
impl PersonalProjectProvisioningPort for PostgresPackageRepositoryStore {
    async fn create_with_owner_grant(
        &self,
        repository_id: Uuid,
        repository_event: PackageRepositoryEvent,
        owner_user_id: Uuid,
        permission_event: PermissionEvent,
        actor_id: Uuid,
    ) -> Result<(), EventStoreError> {
        let mut tx = self.pool.begin().await.storage_err()?;

        self.append_in_tx(&mut tx, repository_id, 0, vec![repository_event], actor_id).await?;

        if !matches!(permission_event, PermissionEvent::Granted { .. }) {
            return Err(EventStoreError::Storage("PersonalProjectProvisioningPort::create_with_owner_grant requires a Granted event".to_string()));
        }
        permission_store::append_in_tx(&mut tx, owner_user_id, repository_id, 0, vec![permission_event], actor_id).await?;

        tx.commit().await.storage_err()?;
        Ok(())
    }
}

async fn apply_to_projection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    repository_id: Uuid,
    event: &PackageRepositoryEvent,
    version: i64,
) -> Result<(), EventStoreError> {
    match event {
        PackageRepositoryEvent::Created { organization_id, name, format, repo_type, remote_url, remote_username, remote_password, .. } => {
            sqlx::query!(
                "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password, version, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now(), now())",
                repository_id,
                organization_id,
                name,
                format_to_str(*format),
                repo_type_to_str(*repo_type),
                remote_url.as_deref(),
                remote_username.as_deref(),
                remote_password.as_deref(),
                version
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
        PackageRepositoryEvent::Renamed { new_name, .. } => {
            sqlx::query!(
                "UPDATE package_repository_projections SET name = $1, version = $2, updated_at = now() WHERE id = $3",
                new_name,
                version,
                repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
        PackageRepositoryEvent::RemoteUrlChanged { remote_url, .. } => {
            sqlx::query!(
                "UPDATE package_repository_projections SET remote_url = $1, version = $2, updated_at = now() WHERE id = $3",
                remote_url,
                version,
                repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
        PackageRepositoryEvent::GroupMemberAdded { member_repository_id, position, .. } => {
            sqlx::query!(
                "INSERT INTO package_repository_group_members (group_repository_id, member_repository_id, position) \
                 VALUES ($1, $2, $3) \
                 ON CONFLICT (group_repository_id, member_repository_id) DO UPDATE SET position = EXCLUDED.position",
                repository_id,
                member_repository_id,
                position
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
            sqlx::query!(
                "UPDATE package_repository_projections SET version = $1, updated_at = now() WHERE id = $2",
                version,
                repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
        PackageRepositoryEvent::GroupMemberRemoved { member_repository_id, .. } => {
            sqlx::query!(
                "DELETE FROM package_repository_group_members WHERE group_repository_id = $1 AND member_repository_id = $2",
                repository_id,
                member_repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
            sqlx::query!(
                "UPDATE package_repository_projections SET version = $1, updated_at = now() WHERE id = $2",
                version,
                repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
        PackageRepositoryEvent::QuotaSet { quota_bytes, .. } => {
            sqlx::query!(
                "UPDATE package_repository_projections SET quota_bytes = $1, version = $2, updated_at = now() WHERE id = $3",
                *quota_bytes,
                version,
                repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
        PackageRepositoryEvent::RetentionPolicySet { keep_last_n_versions, .. } => {
            sqlx::query!(
                "UPDATE package_repository_projections SET retention_keep_last_n = $1, version = $2, updated_at = now() WHERE id = $3",
                *keep_last_n_versions,
                version,
                repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
        PackageRepositoryEvent::VisibilityChanged { is_public, .. } => {
            sqlx::query!(
                "UPDATE package_repository_projections SET is_public = $1, version = $2, updated_at = now() WHERE id = $3",
                *is_public,
                version,
                repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
        PackageRepositoryEvent::Deleted { .. } => {
            sqlx::query!(
                "UPDATE package_repository_projections SET deleted_at = now(), version = $1, updated_at = now() WHERE id = $2",
                version,
                repository_id
            )
            .execute(&mut **tx)
            .await
            .storage_err()?;
        }
    }
    Ok(())
}

#[async_trait]
impl PackageRepositoryQueryPort for PostgresPackageRepositoryStore {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
        let row = sqlx::query!(
            "SELECT id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password, quota_bytes, retention_keep_last_n, is_public FROM package_repository_projections WHERE id = $1 AND deleted_at IS NULL",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .storage_err()?;
        match row {
            Some(row) => Ok(Some(self.build_summary(row.id, row.organization_id, row.name, row.format, row.repo_type, row.remote_url, row.remote_username, row.remote_password, row.quota_bytes, row.retention_keep_last_n, row.is_public).await?)),
            None => Ok(None),
        }
    }

    async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
        let row = sqlx::query!(
            "SELECT id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password, quota_bytes, retention_keep_last_n, is_public FROM package_repository_projections WHERE organization_id = $1 AND name = $2 AND deleted_at IS NULL",
            organization_id, name
        )
        .fetch_optional(&self.pool)
        .await
        .storage_err()?;
        match row {
            Some(row) => Ok(Some(self.build_summary(row.id, row.organization_id, row.name, row.format, row.repo_type, row.remote_url, row.remote_username, row.remote_password, row.quota_bytes, row.retention_keep_last_n, row.is_public).await?)),
            None => Ok(None),
        }
    }

    async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
        let rows = sqlx::query!(
            "SELECT id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password, quota_bytes, retention_keep_last_n, is_public FROM package_repository_projections WHERE deleted_at IS NULL"
        )
        .fetch_all(&self.pool)
        .await
        .storage_err()?;

        let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        let member_rows = sqlx::query!(
            "SELECT group_repository_id, member_repository_id FROM package_repository_group_members \
             WHERE group_repository_id = ANY($1) ORDER BY group_repository_id, position",
            &ids
        )
        .fetch_all(&self.pool)
        .await
        .storage_err()?;
        let mut members_by_group: std::collections::HashMap<Uuid, Vec<Uuid>> = std::collections::HashMap::new();
        for row in member_rows {
            members_by_group.entry(row.group_repository_id).or_default().push(row.member_repository_id);
        }

        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            summaries.push(PackageRepositorySummary {
                id: row.id,
                organization_id: row.organization_id,
                name: row.name,
                format: format_from_str(&row.format).ok_or_else(|| EventStoreError::Storage(format!("unknown format {}", row.format)))?,
                repo_type: repo_type_from_str(&row.repo_type).ok_or_else(|| EventStoreError::Storage(format!("unknown repo_type {}", row.repo_type)))?,
                remote_url: row.remote_url,
                remote_username: row.remote_username,
                remote_password: self.password_for_listing(row.id, row.remote_password),
                group_members: members_by_group.remove(&row.id).unwrap_or_default(),
                quota_bytes: row.quota_bytes,
                retention_keep_last_n: row.retention_keep_last_n,
                is_public: row.is_public,
            });
        }
        Ok(summaries)
    }

    /// Same as `list_all`, filtered by `organization_id` in the `WHERE` clause.
    async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
        let rows = sqlx::query!(
            "SELECT id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password, quota_bytes, retention_keep_last_n, is_public FROM package_repository_projections WHERE organization_id = $1 AND deleted_at IS NULL",
            organization_id
        )
        .fetch_all(&self.pool)
        .await
        .storage_err()?;

        let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        let member_rows = sqlx::query!(
            "SELECT group_repository_id, member_repository_id FROM package_repository_group_members \
             WHERE group_repository_id = ANY($1) ORDER BY group_repository_id, position",
            &ids
        )
        .fetch_all(&self.pool)
        .await
        .storage_err()?;
        let mut members_by_group: std::collections::HashMap<Uuid, Vec<Uuid>> = std::collections::HashMap::new();
        for row in member_rows {
            members_by_group.entry(row.group_repository_id).or_default().push(row.member_repository_id);
        }

        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            summaries.push(PackageRepositorySummary {
                id: row.id,
                organization_id: row.organization_id,
                name: row.name,
                format: format_from_str(&row.format).ok_or_else(|| EventStoreError::Storage(format!("unknown format {}", row.format)))?,
                repo_type: repo_type_from_str(&row.repo_type).ok_or_else(|| EventStoreError::Storage(format!("unknown repo_type {}", row.repo_type)))?,
                remote_url: row.remote_url,
                remote_username: row.remote_username,
                remote_password: self.password_for_listing(row.id, row.remote_password),
                group_members: members_by_group.remove(&row.id).unwrap_or_default(),
                quota_bytes: row.quota_bytes,
                retention_keep_last_n: row.retention_keep_last_n,
                is_public: row.is_public,
            });
        }
        Ok(summaries)
    }
}

#[async_trait]
impl RepositoryDeletionSweepPort for PostgresPackageRepositoryStore {
    async fn hard_delete_repositories_past_grace_period(&self) -> Result<HardDeleteSweepResult, artiferris_domain::error::DomainError> {
        let doomed_ids: Vec<Uuid> = sqlx::query_scalar!(
            "SELECT id FROM package_repository_projections WHERE deleted_at IS NOT NULL AND deleted_at < now() - interval '30 days'"
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;

        let mut result = HardDeleteSweepResult::default();

        // Each doomed repository is hard-deleted in its own transaction: in one batched statement a single failing
        // repository rolled back the whole run, every day. Per repository, a failure only costs that repository's turn;
        // it is retried the next day.
        for repository_id in doomed_ids {
            match self.hard_delete_one_repository(repository_id).await {
                Ok(outcome) => {
                    result.repositories_removed += 1;
                    result.swept_repository_ids.push(repository_id);
                    result.reclaimed_docker_blob_digests.extend(outcome);
                }
                Err(e) => {
                    tracing::warn!(repository_id = %repository_id, error = %e, "repository deletion sweep failed to hard-delete a repository; it will be retried on the next run");
                }
            }
        }

        Ok(result)
    }
}

/// Holds the transaction that took the advisory lock; dropping it releases the lock like a commit. Neither field is
/// read.
struct PostgresRepositoryLockGuard {
    _tx: sqlx::Transaction<'static, Postgres>,
    _in_process: OwnedMutexGuard<()>,
}

impl RepositoryLockGuard for PostgresRepositoryLockGuard {}

#[async_trait]
impl RepositoryQuotaLockPort for PostgresPackageRepositoryStore {
    /// Serializes quota-affecting writes to one repository: in-process by an in-memory lock, across processes by
    /// `pg_advisory_xact_lock` (the key the Docker manifest insert takes). The in-memory lock goes first so waiters
    /// hold no pool connection, which the holder needs.
    async fn acquire_repository_lock(&self, repository_id: Uuid) -> Result<Box<dyn RepositoryLockGuard>, EventStoreError> {
        let in_process = self.quota_locks.lock(repository_id).await;
        let mut tx = self.pool.begin().await.storage_err()?;
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", repository_id.to_string()).execute(&mut *tx).await.storage_err()?;
        Ok(Box::new(PostgresRepositoryLockGuard { _tx: tx, _in_process: in_process }))
    }
}

impl PostgresPackageRepositoryStore {
    /// Hard-deletes one repository in its own transaction. Returns the digests of Docker blobs to remove from disk
    /// after the commit, via `remove_reclaimed_blob_files`, which re-locks and re-checks each digest.
    async fn hard_delete_one_repository(&self, repository_id: Uuid) -> Result<Vec<String>, artiferris_domain::error::DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;

        let orphaned_docker_blob_digests: Vec<String> = sqlx::query_scalar!(
            "SELECT dmb.blob_digest FROM docker_manifest_blobs dmb \
             JOIN docker_manifests dm ON dm.id = dmb.manifest_id \
             WHERE dm.package_repository_id = $1",
            repository_id
        )
        .fetch_all(&mut *tx)
        .await
        .infra_err()?;

        let all_linked_digests: Vec<String> =
            sqlx::query_scalar!("SELECT DISTINCT blob_digest FROM docker_repository_blobs WHERE package_repository_id = $1", repository_id)
                .fetch_all(&mut *tx)
                .await
                .infra_err()?;

        let reclaim_candidate_digests: Vec<String> = orphaned_docker_blob_digests
            .iter()
            .cloned()
            .chain(all_linked_digests)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        sqlx::query!("DELETE FROM package_repository_group_members WHERE member_repository_id = $1", repository_id)
            .execute(&mut *tx)
            .await
            .infra_err()?;

        let deleted = sqlx::query!("DELETE FROM package_repository_projections WHERE id = $1", repository_id)
            .execute(&mut *tx)
            .await
            .infra_err()?;

        if deleted.rows_affected() == 0 {
            tx.commit().await.infra_err()?;
            return Ok(Vec::new());
        }

        let mut reclaimed_docker_blob_digests = Vec::new();
        if !reclaim_candidate_digests.is_empty() {
            sqlx::query!(
                "UPDATE docker_blobs SET reference_count = reference_count - counts.cnt \
                 FROM (SELECT blob_digest, COUNT(*) AS cnt FROM unnest($1::text[]) AS blob_digest GROUP BY blob_digest) counts \
                 WHERE docker_blobs.digest = counts.blob_digest",
                &orphaned_docker_blob_digests
            )
            .execute(&mut *tx)
            .await
            .infra_err()?;

            let rows = sqlx::query!(
                "DELETE FROM docker_blobs \
                 WHERE digest = ANY($1) AND reference_count <= 0 \
                   AND NOT EXISTS (SELECT 1 FROM docker_repository_blobs drb WHERE drb.blob_digest = docker_blobs.digest) \
                 RETURNING digest",
                &reclaim_candidate_digests
            )
            .fetch_all(&mut *tx)
            .await
            .infra_err()?;
            reclaimed_docker_blob_digests = rows.into_iter().map(|r| r.digest).collect();
        }

        tx.commit().await.infra_err()?;

        Ok(reclaimed_docker_blob_digests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::docker_registry::Digest;
    use artiferris_domain::permission::{PermissionEventStorePort, Role};

    /// `package_repository_projections.organization_id` is a foreign key: a second organization needs a real row.
    async fn seed_organization(pool: &PgPool, id: Uuid, slug: &str) {
        sqlx::query!("INSERT INTO organizations (id, slug, display_name) VALUES ($1, $2, $3)", id, slug, slug)
            .execute(pool)
            .await
            .unwrap();
    }

    #[sqlx::test]
    async fn creates_and_finds_a_repository(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let id = Uuid::new_v4();
        let event = PackageRepositoryEvent::Created {
            repository_id: id,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            name: "my-repo".to_string(),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
        };
        store.append(id, 0, vec![event], Uuid::new_v4()).await.unwrap();

        let summary = store.find_by_org_and_name(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-repo").await.unwrap().unwrap();
        assert_eq!(summary.id, id);
        assert_eq!(summary.repo_type, RepositoryType::Hosted);
    }

    #[sqlx::test]
    async fn a_proxy_repositorys_remote_password_round_trips_through_find_and_through_load(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let id = Uuid::new_v4();
        store
            .append(
                id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "proxy-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Proxy,
                    remote_url: Some("https://registry.npmjs.org".to_string()),
                    remote_username: Some("svc-account".to_string()),
                    remote_password: Some("s3cret-upstream-token".to_string()),
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let summary = store.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(summary.remote_password.as_deref(), Some("s3cret-upstream-token"));

        let (_, events) = store.load(id).await.unwrap();
        assert!(matches!(&events[0], PackageRepositoryEvent::Created { remote_password: Some(pw), .. } if pw == "s3cret-upstream-token"));
    }

    #[sqlx::test]
    async fn the_remote_password_is_never_stored_in_plaintext(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let id = Uuid::new_v4();
        store
            .append(
                id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "proxy-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Proxy,
                    remote_url: Some("https://registry.npmjs.org".to_string()),
                    remote_username: Some("svc-account".to_string()),
                    remote_password: Some("s3cret-upstream-token".to_string()),
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let projection_row: (Option<String>,) =
            sqlx::query_as("SELECT remote_password FROM package_repository_projections WHERE id = $1").bind(id).fetch_one(&store.pool).await.unwrap();
        assert!(!projection_row.0.unwrap().contains("s3cret-upstream-token"), "the plaintext password must never appear in the projection row");

        let event_row: (serde_json::Value,) =
            sqlx::query_as("SELECT payload FROM domain_events WHERE aggregate_type = 'PackageRepository' AND aggregate_id = $1").bind(id.to_string()).fetch_one(&store.pool).await.unwrap();
        assert!(!event_row.0.to_string().contains("s3cret-upstream-token"), "the plaintext password must never appear in the event journal");
    }

    #[sqlx::test]
    async fn one_undecryptable_remote_password_does_not_fail_the_listing(pool: sqlx::PgPool) {
        let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let broken_id = Uuid::new_v4();
        let healthy_id = Uuid::new_v4();
        let written_with_another_key = PostgresPackageRepositoryStore::new(pool.clone(), "some-other-key".to_string());
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        for (id, name, writer) in [(broken_id, "broken", &written_with_another_key), (healthy_id, "healthy", &store)] {
            writer
                .append(
                    id,
                    0,
                    vec![PackageRepositoryEvent::Created {
                        repository_id: id,
                        organization_id,
                        name: name.to_string(),
                        format: RepositoryFormat::Npm,
                        repo_type: RepositoryType::Proxy,
                        remote_url: Some("https://registry.npmjs.org".to_string()),
                        remote_username: None,
                        remote_password: Some("token".to_string()),
                    }],
                    Uuid::new_v4(),
                )
                .await
                .unwrap();
        }

        let all = store.list_all().await.expect("the listing survives one bad row");
        let in_org = store.list_by_organization(organization_id).await.expect("so does the per-organization listing");

        for listed in [all, in_org] {
            assert_eq!(listed.iter().find(|r| r.id == healthy_id).unwrap().remote_password.as_deref(), Some("token"));
            assert_eq!(listed.iter().find(|r| r.id == broken_id).unwrap().remote_password, None);
        }
        assert!(store.find_by_id(broken_id).await.is_err(), "the lookup a proxy request goes through still fails loudly");
    }

    #[sqlx::test]
    async fn a_deleted_repositorys_name_can_be_reused(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let first_id = Uuid::new_v4();
        store
            .append(
                first_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: first_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "recyclable".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store
            .append(first_id, 1, vec![PackageRepositoryEvent::Deleted { repository_id: first_id }], Uuid::new_v4())
            .await
            .unwrap();
        assert!(store.find_by_org_and_name(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "recyclable").await.unwrap().is_none());

        let second_id = Uuid::new_v4();
        store
            .append(
                second_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: second_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "recyclable".to_string(),
                    format: RepositoryFormat::Docker,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let summary = store.find_by_org_and_name(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "recyclable").await.unwrap().unwrap();
        assert_eq!(summary.id, second_id);
    }

    #[sqlx::test]
    async fn two_live_repositories_still_cannot_share_a_name(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let first_id = Uuid::new_v4();
        store
            .append(
                first_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: first_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "exclusive".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let second_id = Uuid::new_v4();
        let result = store
            .append(
                second_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: second_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "exclusive".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await;

        assert!(result.is_err());
    }

    #[sqlx::test]
    async fn adding_a_group_member_is_reflected_in_the_summary(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let group_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        store
            .append(
                group_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: group_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "group-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Group,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store
            .append(
                member_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: member_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "member-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store
            .append(
                group_id,
                1,
                vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: group_id, member_repository_id: member_id, position: 0 }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let summary = store.find_by_id(group_id).await.unwrap().unwrap();
        assert_eq!(summary.group_members, vec![member_id]);
    }

    #[sqlx::test]
    async fn list_all_batches_group_members_across_every_repository(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let group_a = Uuid::new_v4();
        let group_b = Uuid::new_v4();
        let member_a = Uuid::new_v4();
        let member_b = Uuid::new_v4();
        for (group_id, name) in [(group_a, "group-a"), (group_b, "group-b")] {
            store
                .append(
                    group_id,
                    0,
                    vec![PackageRepositoryEvent::Created {
                        repository_id: group_id,
                        organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                        name: name.to_string(),
                        format: RepositoryFormat::Npm,
                        repo_type: RepositoryType::Group,
                        remote_url: None,
                        remote_username: None,
                        remote_password: None,
                    }],
                    Uuid::new_v4(),
                )
                .await
                .unwrap();
        }
        for (member_id, name) in [(member_a, "member-a"), (member_b, "member-b")] {
            store
                .append(
                    member_id,
                    0,
                    vec![PackageRepositoryEvent::Created {
                        repository_id: member_id,
                        organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                        name: name.to_string(),
                        format: RepositoryFormat::Npm,
                        repo_type: RepositoryType::Hosted,
                        remote_url: None,
                        remote_username: None,
                        remote_password: None,
                    }],
                    Uuid::new_v4(),
                )
                .await
                .unwrap();
        }
        store.append(group_a, 1, vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: group_a, member_repository_id: member_a, position: 0 }], Uuid::new_v4()).await.unwrap();
        store.append(group_b, 1, vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: group_b, member_repository_id: member_b, position: 0 }], Uuid::new_v4()).await.unwrap();

        let all = store.list_all().await.unwrap();

        assert_eq!(all.iter().find(|r| r.id == group_a).unwrap().group_members, vec![member_a]);
        assert_eq!(all.iter().find(|r| r.id == group_b).unwrap().group_members, vec![member_b]);
    }

    #[sqlx::test]
    async fn setting_a_quota_is_reflected_in_the_summary(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let id = Uuid::new_v4();
        store
            .append(
                id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "quota-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        assert_eq!(store.find_by_id(id).await.unwrap().unwrap().quota_bytes, None, "unset by default");

        store.append(id, 1, vec![PackageRepositoryEvent::QuotaSet { repository_id: id, quota_bytes: Some(1024) }], Uuid::new_v4()).await.unwrap();
        assert_eq!(store.find_by_id(id).await.unwrap().unwrap().quota_bytes, Some(1024));

        store.append(id, 2, vec![PackageRepositoryEvent::QuotaSet { repository_id: id, quota_bytes: None }], Uuid::new_v4()).await.unwrap();
        assert_eq!(store.find_by_id(id).await.unwrap().unwrap().quota_bytes, None, "clearing the quota restores unlimited");
    }

    #[sqlx::test]
    async fn setting_a_retention_policy_is_reflected_in_the_summary(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let id = Uuid::new_v4();
        store
            .append(
                id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "retention-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        assert_eq!(store.find_by_id(id).await.unwrap().unwrap().retention_keep_last_n, None, "unset by default");

        store.append(id, 1, vec![PackageRepositoryEvent::RetentionPolicySet { repository_id: id, keep_last_n_versions: Some(3) }], Uuid::new_v4()).await.unwrap();
        assert_eq!(store.find_by_id(id).await.unwrap().unwrap().retention_keep_last_n, Some(3));

        store.append(id, 2, vec![PackageRepositoryEvent::RetentionPolicySet { repository_id: id, keep_last_n_versions: None }], Uuid::new_v4()).await.unwrap();
        assert_eq!(store.find_by_id(id).await.unwrap().unwrap().retention_keep_last_n, None, "clearing disables the policy again");
    }

    #[sqlx::test]
    async fn setting_visibility_is_reflected_in_the_summary(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let id = Uuid::new_v4();
        store
            .append(
                id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "visibility-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        assert!(!store.find_by_id(id).await.unwrap().unwrap().is_public, "defaults to private");

        store.append(id, 1, vec![PackageRepositoryEvent::VisibilityChanged { repository_id: id, is_public: true }], Uuid::new_v4()).await.unwrap();
        assert!(store.find_by_id(id).await.unwrap().unwrap().is_public);

        store.append(id, 2, vec![PackageRepositoryEvent::VisibilityChanged { repository_id: id, is_public: false }], Uuid::new_v4()).await.unwrap();
        assert!(!store.find_by_id(id).await.unwrap().unwrap().is_public, "clearing visibility returns to private");
    }

    #[sqlx::test]
    async fn deleting_hides_the_repository_from_list_all(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let id = Uuid::new_v4();
        store
            .append(
                id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "to-delete".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store.append(id, 1, vec![PackageRepositoryEvent::Deleted { repository_id: id }], Uuid::new_v4()).await.unwrap();

        assert!(store.list_all().await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn list_by_organization_only_returns_that_organizations_repositories(pool: sqlx::PgPool) {
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();
        seed_organization(&pool, org_a, "org-a").await;
        seed_organization(&pool, org_b, "org-b").await;
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let repo_a = Uuid::new_v4();
        let repo_b = Uuid::new_v4();
        store
            .append(
                repo_a,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: repo_a,
                    organization_id: org_a,
                    name: "repo-a".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store
            .append(
                repo_b,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: repo_b,
                    organization_id: org_b,
                    name: "repo-b".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let results = store.list_by_organization(org_a).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].organization_id, org_a);
        assert_eq!(results[0].id, repo_a);
    }

    #[sqlx::test]
    async fn list_by_organization_excludes_a_deleted_repository(pool: sqlx::PgPool) {
        let org_id = Uuid::new_v4();
        seed_organization(&pool, org_id, "org-with-a-deleted-repo").await;
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let id = Uuid::new_v4();
        store
            .append(
                id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: id,
                    organization_id: org_id,
                    name: "to-delete".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store.append(id, 1, vec![PackageRepositoryEvent::Deleted { repository_id: id }], Uuid::new_v4()).await.unwrap();

        assert!(store.list_by_organization(org_id).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn list_by_organization_batches_group_members_the_same_way_as_list_all(pool: sqlx::PgPool) {
        let org_id = Uuid::new_v4();
        seed_organization(&pool, org_id, "org-with-a-group-repo").await;
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let group_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        store
            .append(
                group_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: group_id,
                    organization_id: org_id,
                    name: "group-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Group,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store
            .append(
                member_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: member_id,
                    organization_id: org_id,
                    name: "member-repo".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store
            .append(
                group_id,
                1,
                vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: group_id, member_repository_id: member_id, position: 0 }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let results = store.list_by_organization(org_id).await.unwrap();

        assert_eq!(results.iter().find(|r| r.id == group_id).unwrap().group_members, vec![member_id]);
    }

    #[sqlx::test]
    async fn serializes_concurrent_first_appends_for_a_new_aggregate(pool: sqlx::PgPool) {
        let store_a = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let store_b = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let repository_id = Uuid::new_v4();

        let (result_a, result_b) = tokio::join!(
            store_a.append(
                repository_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "race-repo-a".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4()
            ),
            store_b.append(
                repository_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "race-repo-b".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4()
            )
        );

        let outcomes = [result_a, result_b];
        let successes = outcomes.iter().filter(|r| r.is_ok()).count();
        let conflicts = outcomes.iter().filter(|r| matches!(r, Err(EventStoreError::ConcurrencyConflict { expected: 0, actual: 1 }))).count();

        assert_eq!(successes, 1, "expected exactly one append to succeed, got: {outcomes:?}");
        assert_eq!(conflicts, 1, "expected the loser to get a clean ConcurrencyConflict{{expected:0, actual:1}}, got: {outcomes:?}");
    }

    /// A `Revoked` event is never valid first in a stream, so the port rejects it after the repository insert ran in
    /// the same transaction: a real rollback test.
    #[sqlx::test]
    async fn create_with_owner_grant_leaves_no_partial_state_when_the_permission_side_is_rejected(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let repository_id = Uuid::new_v4();
        let owner_user_id = Uuid::new_v4();
        let repository_event = PackageRepositoryEvent::Created {
            repository_id,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            name: "atomic-repo".to_string(),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
        };
        let invalid_permission_event = PermissionEvent::Revoked { user_id: owner_user_id, repository_id };

        let err = store
            .create_with_owner_grant(repository_id, repository_event, owner_user_id, invalid_permission_event, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(err, EventStoreError::Storage(_)), "expected a Storage error rejecting the non-Granted event, got {err:?}");

        assert!(store.find_by_id(repository_id).await.unwrap().is_none());
        let event_count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM domain_events WHERE aggregate_type = 'PackageRepository' AND aggregate_id = $1",
            repository_id.to_string()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(event_count, 0, "the repository-side domain_events insert must have rolled back");
        let permission_row_count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM permission_projections WHERE user_id = $1 AND repository_id = $2",
            owner_user_id,
            repository_id
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(permission_row_count, 0);
    }

    /// Repository and owner's grant persist together from one call.
    #[sqlx::test]
    async fn create_with_owner_grant_persists_both_the_repository_and_the_grant(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool, "test-secret".to_string());
        let repository_id = Uuid::new_v4();
        let owner_user_id = Uuid::new_v4();
        let repository_event = PackageRepositoryEvent::Created {
            repository_id,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            name: "provisioned-repo".to_string(),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
        };
        let permission_event = PermissionEvent::Granted { user_id: owner_user_id, repository_id, role: Role::Admin };

        store
            .create_with_owner_grant(repository_id, repository_event, owner_user_id, permission_event, owner_user_id)
            .await
            .unwrap();

        assert!(store.find_by_id(repository_id).await.unwrap().is_some());
        let role: Option<String> = sqlx::query_scalar("SELECT role FROM permission_projections WHERE user_id = $1 AND repository_id = $2")
            .bind(owner_user_id)
            .bind(repository_id)
            .fetch_optional(&store.pool)
            .await
            .unwrap();
        assert_eq!(role.as_deref(), Some("admin"));
    }

    #[sqlx::test]
    async fn every_event_of_a_repository_and_its_grants_is_filed_under_its_organization(pool: sqlx::PgPool) {
        let organization_id = Uuid::new_v4();
        seed_organization(&pool, organization_id, "acme").await;
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let permissions = crate::postgres::permission_store::PostgresPermissionStore::new(pool.clone());
        let repository_id = Uuid::new_v4();
        let owner = Uuid::new_v4();
        let created = PackageRepositoryEvent::Created {
            repository_id,
            organization_id,
            name: "acme-repo".to_string(),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
        };
        store.create_with_owner_grant(repository_id, created, owner, PermissionEvent::Granted { user_id: owner, repository_id, role: Role::Admin }, owner).await.unwrap();
        store.append(repository_id, 1, vec![PackageRepositoryEvent::QuotaSet { repository_id, quota_bytes: Some(10) }], owner).await.unwrap();
        let other = Uuid::new_v4();
        permissions.append(other, repository_id, 0, vec![PermissionEvent::Granted { user_id: other, repository_id, role: Role::Read }], owner).await.unwrap();

        let unstamped: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE organization_id IS DISTINCT FROM $1")
            .bind(organization_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let stamped: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE organization_id = $1").bind(organization_id).fetch_one(&pool).await.unwrap();
        assert_eq!(unstamped, 0);
        assert_eq!(stamped, 4, "created, quota, and both grants");
    }

    /// Creates a repository, soft-deletes it through `apply_to_projection`, then backdates `deleted_at` directly: there
    /// is no port method for that.
    async fn create_and_soft_delete_backdated(store: &PostgresPackageRepositoryStore, id: Uuid, name: &str, days_ago: i64) {
        store
            .append(
                id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: name.to_string(),
                    format: RepositoryFormat::Docker,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store.append(id, 1, vec![PackageRepositoryEvent::Deleted { repository_id: id }], Uuid::new_v4()).await.unwrap();
        sqlx::query!(
            "UPDATE package_repository_projections SET deleted_at = now() - ($1 || ' days')::interval WHERE id = $2",
            days_ago.to_string(),
            id
        )
        .execute(&store.pool)
        .await
        .unwrap();
    }

    /// Inserts a `docker_blobs` row as production does, with a `storage_key` distinct from the digest.
    async fn seed_docker_blob(pool: &PgPool, digest: &Digest, storage_key: &str, reference_count: i64) {
        sqlx::query!(
            "INSERT INTO docker_blobs (digest, size_bytes, storage_key, reference_count, created_at) VALUES ($1, 11, $2, $3, now())",
            digest.as_str(),
            storage_key,
            reference_count,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_manifest_with_blob(pool: &PgPool, repository_id: Uuid, blob_digest: &Digest) -> Uuid {
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
        sqlx::query!("INSERT INTO docker_manifest_blobs (manifest_id, blob_digest) VALUES ($1, $2)", manifest_id, blob_digest.as_str())
            .execute(pool)
            .await
            .unwrap();
        manifest_id
    }

    #[sqlx::test]
    async fn sweeping_a_repository_soft_deleted_past_the_grace_period_hard_deletes_it_and_its_dependents(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let repository_id = Uuid::new_v4();
        create_and_soft_delete_backdated(&store, repository_id, "doomed-repo", 31).await;

        let blob_digest = Digest::of(b"cascade-orphaned-layer");
        seed_docker_blob(&pool, &blob_digest, "sha256/ab/cd/cascade-orphaned-layer", 1).await;
        let manifest_id = seed_manifest_with_blob(&pool, repository_id, &blob_digest).await;

        let result = store.hard_delete_repositories_past_grace_period().await.unwrap();

        assert_eq!(result.repositories_removed, 1);
        assert_eq!(result.swept_repository_ids, vec![repository_id]);

        let repo_row: Option<Uuid> = sqlx::query_scalar!("SELECT id FROM package_repository_projections WHERE id = $1", repository_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert!(repo_row.is_none(), "the projection row must be genuinely gone, not just soft-deleted");

        let manifest_row: Option<Uuid> = sqlx::query_scalar!("SELECT id FROM docker_manifests WHERE id = $1", manifest_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert!(manifest_row.is_none(), "ON DELETE CASCADE from package_repository_projections must have removed the dependent manifest row");

        // The decrement and the row deletion happen in the sweep's transaction: this blob had one reference and no
        // other link, so it is gone when the call returns, with its digest reported for file cleanup.
        assert_eq!(result.reclaimed_docker_blob_digests, vec![blob_digest.as_str().to_string()]);
        let blob_row: Option<i64> = sqlx::query_scalar!("SELECT reference_count FROM docker_blobs WHERE digest = $1", blob_digest.as_str())
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(blob_row, None, "a zero-referenced, unlinked blob's row must be deleted transactionally, not left for a later step");
    }

    /// A blob linked by another live repository keeps its row while the decrement still happens; the NOT EXISTS guard
    /// avoids the FK violation.
    #[sqlx::test]
    async fn sweeping_decrements_a_shared_blobs_reference_count_but_does_not_delete_it_while_another_repository_still_links_it(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let doomed_repository_id = Uuid::new_v4();
        create_and_soft_delete_backdated(&store, doomed_repository_id, "doomed-repo-shared-blob", 31).await;

        let other_repository_id = Uuid::new_v4();
        store
            .append(
                other_repository_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: other_repository_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "still-alive-repo".to_string(),
                    format: RepositoryFormat::Docker,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let blob_digest = Digest::of(b"shared-across-two-repositories");
        seed_docker_blob(&pool, &blob_digest, "sha256/ab/cd/shared-across-two-repositories", 1).await;
        seed_manifest_with_blob(&pool, doomed_repository_id, &blob_digest).await;
        sqlx::query!(
            "INSERT INTO docker_repository_blobs (package_repository_id, blob_digest) VALUES ($1, $2)",
            other_repository_id,
            blob_digest.as_str(),
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = store.hard_delete_repositories_past_grace_period().await.unwrap();

        assert_eq!(result.repositories_removed, 1);
        assert!(result.reclaimed_docker_blob_digests.is_empty(), "still linked elsewhere — must not be reported for on-disk deletion");

        let blob_row: Option<i64> = sqlx::query_scalar!("SELECT reference_count FROM docker_blobs WHERE digest = $1", blob_digest.as_str())
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(blob_row, Some(0), "the decrement itself must survive even though the row couldn't be deleted");
    }

    /// Mirrors the private `FilesystemDockerBlobStore::storage_key` sharding to assert the real on-disk path.
    fn expected_storage_key(digest: &Digest) -> String {
        let hex = digest.as_str().strip_prefix("sha256:").unwrap();
        format!("sha256/{}/{}/{}", &hex[0..2], &hex[2..4], hex)
    }

    /// A link row with no manifest (what a proxy cache leaves) is reclaimed once its repository is hard-deleted. Runs
    /// the wired use case so file removal is exercised end to end.
    #[sqlx::test]
    async fn sweeping_reclaims_a_link_only_digest_no_manifest_ever_referenced_closing_the_gc_gap(pool: sqlx::PgPool) {
        use crate::filesystem_storage::FilesystemStorageBackend;
        use artiferris_application::use_cases::repository_deletion_sweep::RepositoryDeletionSweepUseCase;
        use artiferris_domain::docker_registry::DockerBlobStorePort;

        let store = std::sync::Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let repository_id = Uuid::new_v4();
        create_and_soft_delete_backdated(&store, repository_id, "doomed-proxy-cache-repo", 31).await;

        let blob_dir = tempfile::tempdir().unwrap();
        let docker_blobs = std::sync::Arc::new(crate::filesystem_docker_blob_store::FilesystemDockerBlobStore::new(pool.clone(), blob_dir.path()));
        let digest = Digest::of(b"proxy-cached-layer-never-in-a-manifest");
        docker_blobs.write(&digest, b"proxy-cached-layer-never-in-a-manifest").await.unwrap();
        docker_blobs.link_to_repository(repository_id, &digest).await.unwrap();
        let file_path = blob_dir.path().join(expected_storage_key(&digest));
        assert!(file_path.exists(), "sanity check: the file exists before the sweep runs");

        let storage_dir = tempfile::tempdir().unwrap();
        let storage = std::sync::Arc::new(FilesystemStorageBackend::new(storage_dir.path()));

        let use_case = RepositoryDeletionSweepUseCase::new(store, docker_blobs.clone(), storage);
        let removed = use_case.execute().await.unwrap();

        assert_eq!(removed, 1);
        assert!(!docker_blobs.exists(&digest).await.unwrap(), "the GC gap must be closed — a link-only digest's row must now be reclaimed");
        assert!(!file_path.exists(), "the file itself must be removed too, via the locked remove_reclaimed_blob_files path");
    }

    /// Widening the candidates must not over-reclaim a digest another live repository links: row and file survive.
    #[sqlx::test]
    async fn sweeping_does_not_reclaim_a_link_only_digest_a_different_live_repository_still_links(pool: sqlx::PgPool) {
        use crate::filesystem_storage::FilesystemStorageBackend;
        use artiferris_application::use_cases::repository_deletion_sweep::RepositoryDeletionSweepUseCase;
        use artiferris_domain::docker_registry::DockerBlobStorePort;

        let store = std::sync::Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let repo_b = Uuid::new_v4();
        create_and_soft_delete_backdated(&store, repo_b, "doomed-repo-b", 31).await;

        let repo_c = Uuid::new_v4();
        store
            .append(
                repo_c,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: repo_c,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "still-alive-repo-c".to_string(),
                    format: RepositoryFormat::Docker,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let blob_dir = tempfile::tempdir().unwrap();
        let docker_blobs = std::sync::Arc::new(crate::filesystem_docker_blob_store::FilesystemDockerBlobStore::new(pool.clone(), blob_dir.path()));
        let digest = Digest::of(b"linked-by-both-b-and-c");
        docker_blobs.write(&digest, b"linked-by-both-b-and-c").await.unwrap();
        docker_blobs.link_to_repository(repo_b, &digest).await.unwrap();
        docker_blobs.link_to_repository(repo_c, &digest).await.unwrap();
        let file_path = blob_dir.path().join(expected_storage_key(&digest));

        let storage_dir = tempfile::tempdir().unwrap();
        let storage = std::sync::Arc::new(FilesystemStorageBackend::new(storage_dir.path()));

        let use_case = RepositoryDeletionSweepUseCase::new(store, docker_blobs.clone(), storage);
        let removed = use_case.execute().await.unwrap();

        assert_eq!(removed, 1);
        assert!(docker_blobs.exists(&digest).await.unwrap(), "repo C's live link must keep the row reachable — widening must not over-reclaim it");
        assert!(file_path.exists(), "the file must survive too, since the row was never eligible for deletion");
    }

    /// The decrement stays on `orphaned_docker_blob_digests`, not the wider union the DELETE uses: D is only linked by
    /// doomed A but referenced by live B's manifest, and A's hard-delete must not steal B's count.
    #[sqlx::test]
    async fn sweeping_a_link_only_digest_does_not_decrement_a_live_repositorys_manifest_reference(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let doomed_repository_id = Uuid::new_v4();
        create_and_soft_delete_backdated(&store, doomed_repository_id, "doomed-link-only-repo", 31).await;

        let live_repository_id = Uuid::new_v4();
        store
            .append(
                live_repository_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: live_repository_id,
                    organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    name: "still-alive-repo-with-manifest".to_string(),
                    format: RepositoryFormat::Docker,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let blob_digest = Digest::of(b"link-only-for-doomed-manifest-for-live");
        seed_docker_blob(&pool, &blob_digest, "sha256/ab/cd/link-only-for-doomed-manifest-for-live", 1).await;
        seed_manifest_with_blob(&pool, live_repository_id, &blob_digest).await;
        sqlx::query!(
            "INSERT INTO docker_repository_blobs (package_repository_id, blob_digest) VALUES ($1, $2)",
            doomed_repository_id,
            blob_digest.as_str(),
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO docker_repository_blobs (package_repository_id, blob_digest) VALUES ($1, $2)",
            live_repository_id,
            blob_digest.as_str(),
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = store.hard_delete_repositories_past_grace_period().await.unwrap();

        assert_eq!(result.repositories_removed, 1);
        assert!(result.reclaimed_docker_blob_digests.is_empty(), "still linked by the live repository — must not be reported for on-disk deletion");

        let reference_count: Option<i64> = sqlx::query_scalar!("SELECT reference_count FROM docker_blobs WHERE digest = $1", blob_digest.as_str())
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(
            reference_count,
            Some(1),
            "the doomed repository only ever linked this digest, never referenced it in a manifest — its hard-delete must not touch a count that reflects the live repository's own manifest reference"
        );
    }

    /// A repository still listed as another live group's member must be hard-deletable: the member FK has no `ON
    /// DELETE`, and one violation in a batched delete used to roll back the whole run.
    #[sqlx::test]
    async fn sweeping_a_group_member_soft_deleted_past_the_grace_period_removes_the_stale_membership_row_and_still_hard_deletes_it(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let group_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let org_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

        store
            .append(
                group_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: group_id,
                    organization_id: org_id,
                    name: "still-alive-group".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Group,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store
            .append(
                member_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id: member_id,
                    organization_id: org_id,
                    name: "doomed-group-member".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Hosted,
                    remote_url: None,
                    remote_username: None,
                    remote_password: None,
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        store
            .append(
                group_id,
                1,
                vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: group_id, member_repository_id: member_id, position: 0 }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        store.append(member_id, 1, vec![PackageRepositoryEvent::Deleted { repository_id: member_id }], Uuid::new_v4()).await.unwrap();
        sqlx::query!("UPDATE package_repository_projections SET deleted_at = now() - interval '31 days' WHERE id = $1", member_id)
            .execute(&pool)
            .await
            .unwrap();

        let membership_before: Option<Uuid> = sqlx::query_scalar!(
            "SELECT member_repository_id FROM package_repository_group_members WHERE group_repository_id = $1 AND member_repository_id = $2",
            group_id,
            member_id
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(membership_before.is_some(), "sanity check: the stale membership row exists before the sweep runs");

        let result = store.hard_delete_repositories_past_grace_period().await.unwrap();

        assert_eq!(result.repositories_removed, 1, "the group member must be hard-deleted, not blocked by the missing ON DELETE action on its FK");
        assert_eq!(result.swept_repository_ids, vec![member_id]);

        let member_row: Option<Uuid> = sqlx::query_scalar!("SELECT id FROM package_repository_projections WHERE id = $1", member_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert!(member_row.is_none(), "the member repository's projection row must be genuinely gone, not blocked by the FK");

        let membership_after: Option<Uuid> = sqlx::query_scalar!(
            "SELECT member_repository_id FROM package_repository_group_members WHERE group_repository_id = $1 AND member_repository_id = $2",
            group_id,
            member_id
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(membership_after.is_none(), "the stale group membership row must be cleaned up along with the member");

        let group_summary = store.find_by_id(group_id).await.unwrap();
        assert!(group_summary.is_some(), "the still-live group must survive the sweep");
        assert!(group_summary.unwrap().group_members.is_empty(), "the group's member list must no longer include the hard-deleted repository");
    }

    #[sqlx::test]
    async fn a_repository_soft_deleted_within_the_grace_period_is_not_swept(pool: sqlx::PgPool) {
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let repository_id = Uuid::new_v4();
        create_and_soft_delete_backdated(&store, repository_id, "recently-deleted-repo", 1).await;

        let result = store.hard_delete_repositories_past_grace_period().await.unwrap();

        assert_eq!(result.repositories_removed, 0);
        let repo_row: Option<Uuid> = sqlx::query_scalar!("SELECT id FROM package_repository_projections WHERE id = $1", repository_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert!(repo_row.is_some(), "still within the 30-day undo window — must not be hard-deleted yet");
    }

    /// End-to-end run of the daily sweep wired as in `AppState::build`, with the real filesystem backends: a
    /// hard-deleted repository's npm tarball is freed from disk.
    #[sqlx::test]
    async fn sweeping_frees_a_hard_deleted_repositorys_npm_tarball_from_disk(pool: sqlx::PgPool) {
        use crate::filesystem_storage::FilesystemStorageBackend;
        use artiferris_application::use_cases::repository_deletion_sweep::RepositoryDeletionSweepUseCase;
        use artiferris_domain::storage::StorageBackendPort;

        let store = std::sync::Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let repository_id = Uuid::new_v4();
        create_and_soft_delete_backdated(&store, repository_id, "doomed-npm-repo", 31).await;

        let storage_dir = tempfile::tempdir().unwrap();
        let storage = std::sync::Arc::new(FilesystemStorageBackend::new(storage_dir.path()));
        storage.write(repository_id, "left-pad/1.0.0.tgz", b"tarball-bytes").await.unwrap();
        assert!(
            storage_dir.path().join(repository_id.to_string()).exists(),
            "sanity check: the tarball must actually be on disk before the sweep runs"
        );

        let another_repository_id = Uuid::new_v4();
        storage.write(another_repository_id, "kept-pkg/1.0.0.tgz", b"kept-bytes").await.unwrap();

        let blob_dir = tempfile::tempdir().unwrap();
        let docker_blobs = std::sync::Arc::new(crate::filesystem_docker_blob_store::FilesystemDockerBlobStore::new(pool.clone(), blob_dir.path()));

        let use_case = RepositoryDeletionSweepUseCase::new(store, docker_blobs, storage.clone());
        let removed = use_case.execute().await.unwrap();

        assert_eq!(removed, 1);
        assert!(
            !storage_dir.path().join(repository_id.to_string()).exists(),
            "the hard-deleted repository's on-disk tarball directory must be gone after the sweep"
        );
        assert_eq!(
            storage.read(another_repository_id, "kept-pkg/1.0.0.tgz").await.unwrap(),
            b"kept-bytes",
            "an untouched repository's files must survive the sweep"
        );
    }
}
