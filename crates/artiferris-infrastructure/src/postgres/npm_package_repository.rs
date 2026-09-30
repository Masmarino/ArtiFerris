use async_trait::async_trait;
use chrono::{DateTime, Utc};
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use artiferris_domain::npm_package::{
    MAX_MANIFEST_BYTES_PER_PACKAGE, MAX_VERSIONS_PER_PACKAGE, NpmDistTag, NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageRepositoryPort, NpmPackageVersion, NpmVersion,
    UnpublishVersionOutcome, UnpublishedPackage, UnpublishedVersion,
};
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresNpmPackageRepository {
    pool: PgPool,
}

impl PostgresNpmPackageRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// `%`, `_` and `\` must match literally, not act as ILIKE wildcards.
fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Takes the lock every publish and unpublish of the package serializes on. `None` if there is no such package.
async fn lock_package(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, repository_id: Uuid, name: &NpmPackageName) -> Result<Option<Uuid>, DomainError> {
    let row = sqlx::query!("SELECT id FROM npm_packages WHERE package_repository_id = $1 AND name = $2 FOR UPDATE", repository_id, name.as_str())
        .fetch_optional(&mut **tx)
        .await
        .infra_err()?;
    Ok(row.map(|row| row.id))
}

/// `lock_package`, creating the package first if it doesn't exist. Loops because an unpublish can delete the row between the two statements.
async fn lock_package_creating_it(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, package: &NpmPackage) -> Result<Uuid, DomainError> {
    for _ in 0..5 {
        sqlx::query!(
            "INSERT INTO npm_packages (id, package_repository_id, name, created_at, updated_at, metadata_fetched_at, cached_metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (package_repository_id, name) DO NOTHING",
            package.id,
            package.package_repository_id,
            package.name.as_str(),
            package.created_at,
            package.updated_at,
            package.metadata_fetched_at,
            package.cached_metadata,
        )
        .execute(&mut **tx)
        .await
        .infra_err()?;
        if let Some(id) = lock_package(tx, package.package_repository_id, &package.name).await? {
            return Ok(id);
        }
    }
    Err(DomainError::Infrastructure("the package kept disappearing while it was being locked".to_string()))
}

async fn record_unpublished(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<(), DomainError> {
    sqlx::query!(
        "INSERT INTO npm_unpublished_versions (package_repository_id, package_name, version) VALUES ($1, $2, $3) \
         ON CONFLICT (package_repository_id, package_name, version) DO NOTHING",
        repository_id,
        name.as_str(),
        version.as_str(),
    )
    .execute(&mut **tx)
    .await
    .infra_err()?;
    Ok(())
}

/// Whether any version of the package with this `release` (no build metadata) was unpublished.
async fn was_unpublished(executor: impl sqlx::PgExecutor<'_>, repository_id: Uuid, name: &NpmPackageName, release: &str) -> Result<bool, DomainError> {
    let row = sqlx::query!(
        "SELECT EXISTS(SELECT 1 FROM npm_unpublished_versions WHERE package_repository_id = $1 AND package_name = $2 AND split_part(version, '+', 1) = $3) AS \"exists!\"",
        repository_id,
        name.as_str(),
        release,
    )
    .fetch_one(executor)
    .await
    .infra_err()?;
    Ok(row.exists)
}

struct PackageRow {
    id: Uuid,
    package_repository_id: Uuid,
    name: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    metadata_fetched_at: Option<DateTime<Utc>>,
    cached_metadata: Option<serde_json::Value>,
}

impl PackageRow {
    fn into_domain(self) -> Result<NpmPackage, DomainError> {
        Ok(NpmPackage {
            id: self.id,
            package_repository_id: self.package_repository_id,
            name: NpmPackageName::parse(&self.name)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata_fetched_at: self.metadata_fetched_at,
            cached_metadata: self.cached_metadata,
        })
    }
}

/// Like `PackageRow` but skips `cached_metadata` — can be huge, and search() never reads it.
struct SearchRow {
    id: Uuid,
    package_repository_id: Uuid,
    name: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl SearchRow {
    fn into_domain(self) -> Result<NpmPackage, DomainError> {
        Ok(NpmPackage {
            id: self.id,
            package_repository_id: self.package_repository_id,
            name: NpmPackageName::parse(&self.name)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata_fetched_at: None,
            cached_metadata: None,
        })
    }
}

/// Backs `list_versions_for_packages` — same idea as `SearchRow`, no `manifest`.
struct VersionSummaryRow {
    id: Uuid,
    npm_package_id: Uuid,
    version: String,
    shasum: String,
    tarball_size_bytes: i64,
    deprecated: bool,
    deprecated_message: Option<String>,
    published_at: DateTime<Utc>,
}

impl VersionSummaryRow {
    fn into_domain(self) -> Result<artiferris_domain::npm_package::NpmPackageVersionSummary, DomainError> {
        Ok(artiferris_domain::npm_package::NpmPackageVersionSummary {
            id: self.id,
            npm_package_id: self.npm_package_id,
            version: NpmVersion::parse(&self.version)?,
            shasum: self.shasum,
            tarball_size_bytes: self.tarball_size_bytes,
            deprecated: self.deprecated,
            deprecated_message: self.deprecated_message,
            published_at: self.published_at,
        })
    }
}

struct VersionRow {
    id: Uuid,
    npm_package_id: Uuid,
    version: String,
    manifest: serde_json::Value,
    shasum: String,
    integrity: String,
    tarball_storage_key: String,
    tarball_size_bytes: i64,
    deprecated: bool,
    deprecated_message: Option<String>,
    published_by: Option<Uuid>,
    published_at: DateTime<Utc>,
    origin: String,
}

impl VersionRow {
    fn into_domain(self) -> Result<NpmPackageVersion, DomainError> {
        Ok(NpmPackageVersion {
            id: self.id,
            npm_package_id: self.npm_package_id,
            version: NpmVersion::parse(&self.version)?,
            manifest: self.manifest,
            shasum: self.shasum,
            integrity: self.integrity,
            tarball_storage_key: self.tarball_storage_key,
            tarball_size_bytes: self.tarball_size_bytes,
            deprecated: self.deprecated,
            deprecated_message: self.deprecated_message,
            published_by: self.published_by,
            published_at: self.published_at,
            origin: match self.origin.as_str() {
                "local" => NpmPackageOrigin::Local,
                _ => NpmPackageOrigin::ProxyCache,
            },
        })
    }
}

#[async_trait]
impl NpmPackageRepositoryPort for PostgresNpmPackageRepository {
    async fn find_package(&self, repository_id: Uuid, name: &NpmPackageName) -> Result<Option<NpmPackage>, DomainError> {
        let row = sqlx::query_as!(
            PackageRow,
            "SELECT id, package_repository_id, name, created_at, updated_at, metadata_fetched_at, cached_metadata \
             FROM npm_packages WHERE package_repository_id = $1 AND name = $2",
            repository_id,
            name.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(PackageRow::into_domain).transpose()
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<NpmPackage>, DomainError> {
        let row = sqlx::query_as!(
            PackageRow,
            "SELECT id, package_repository_id, name, created_at, updated_at, metadata_fetched_at, cached_metadata \
             FROM npm_packages WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(PackageRow::into_domain).transpose()
    }

    async fn create_package(&self, package: &NpmPackage) -> Result<Uuid, DomainError> {
        let inserted = sqlx::query_scalar!(
            "INSERT INTO npm_packages (id, package_repository_id, name, created_at, updated_at, metadata_fetched_at, cached_metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (package_repository_id, name) DO NOTHING \
             RETURNING id",
            package.id,
            package.package_repository_id,
            package.name.as_str(),
            package.created_at,
            package.updated_at,
            package.metadata_fetched_at,
            package.cached_metadata,
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        if let Some(id) = inserted {
            return Ok(id);
        }
        let existing = sqlx::query_scalar!(
            "SELECT id FROM npm_packages WHERE package_repository_id = $1 AND name = $2",
            package.package_repository_id,
            package.name.as_str()
        )
        .fetch_one(&self.pool)
        .await
        .infra_err()?;
        Ok(existing)
    }

    async fn touch_metadata_fetched_at(&self, npm_package_id: Uuid, fetched_at: DateTime<Utc>) -> Result<(), DomainError> {
        sqlx::query!("UPDATE npm_packages SET metadata_fetched_at = $2, updated_at = now() WHERE id = $1", npm_package_id, fetched_at)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn list_versions(&self, npm_package_id: Uuid) -> Result<Vec<NpmPackageVersion>, DomainError> {
        let rows = sqlx::query_as!(
            VersionRow,
            "SELECT id, npm_package_id, version, manifest, shasum, integrity, tarball_storage_key, tarball_size_bytes, \
                    deprecated, deprecated_message, published_by, published_at, origin \
             FROM npm_package_versions WHERE npm_package_id = $1 ORDER BY published_at",
            npm_package_id
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(VersionRow::into_domain).collect()
    }

    async fn list_versions_for_packages(&self, npm_package_ids: &[Uuid]) -> Result<Vec<artiferris_domain::npm_package::NpmPackageVersionSummary>, DomainError> {
        if npm_package_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as!(
            VersionSummaryRow,
            "SELECT id, npm_package_id, version, shasum, tarball_size_bytes, deprecated, deprecated_message, published_at \
             FROM npm_package_versions WHERE npm_package_id = ANY($1) ORDER BY npm_package_id, published_at",
            npm_package_ids
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(VersionSummaryRow::into_domain).collect()
    }

    async fn list_latest_versions_for_packages(&self, npm_package_ids: &[Uuid], per_package: i64) -> Result<Vec<artiferris_domain::npm_package::NpmPackageVersionSummary>, DomainError> {
        if npm_package_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as!(
            VersionSummaryRow,
            "SELECT v.id AS \"id!\", v.npm_package_id AS \"npm_package_id!\", v.version AS \"version!\", v.shasum AS \"shasum!\", v.tarball_size_bytes AS \"tarball_size_bytes!\", \
                    v.deprecated AS \"deprecated!\", v.deprecated_message, v.published_at AS \"published_at!\" \
             FROM unnest($1::uuid[]) AS p(id) \
             CROSS JOIN LATERAL (SELECT id, npm_package_id, version, shasum, tarball_size_bytes, deprecated, deprecated_message, published_at \
                                 FROM npm_package_versions WHERE npm_package_id = p.id ORDER BY published_at DESC, id LIMIT $2) v \
             ORDER BY v.npm_package_id, v.published_at DESC, v.id",
            npm_package_ids,
            per_package
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(VersionSummaryRow::into_domain).collect()
    }

    async fn find_version(&self, npm_package_id: Uuid, version: &NpmVersion) -> Result<Option<NpmPackageVersion>, DomainError> {
        let row = sqlx::query_as!(
            VersionRow,
            "SELECT id, npm_package_id, version, manifest, shasum, integrity, tarball_storage_key, tarball_size_bytes, \
                    deprecated, deprecated_message, published_by, published_at, origin \
             FROM npm_package_versions WHERE npm_package_id = $1 AND version = $2",
            npm_package_id,
            version.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(VersionRow::into_domain).transpose()
    }

    async fn insert_version(&self, version: &NpmPackageVersion) -> Result<(), DomainError> {
        let origin = match version.origin {
            NpmPackageOrigin::Local => "local",
            NpmPackageOrigin::ProxyCache => "proxy_cache",
        };
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!(
            "INSERT INTO npm_package_versions \
                (id, npm_package_id, version, manifest, shasum, integrity, tarball_storage_key, tarball_size_bytes, \
                 deprecated, deprecated_message, published_by, published_at, origin) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            version.id,
            version.npm_package_id,
            version.version.as_str(),
            version.manifest,
            version.shasum,
            version.integrity,
            version.tarball_storage_key,
            version.tarball_size_bytes,
            version.deprecated,
            version.deprecated_message,
            version.published_by,
            version.published_at,
            origin,
        )
        .execute(&mut *tx)
        .await
        // The unique (npm_package_id, version) constraint is what actually resolves a same-version
        // publish race now that the storage key includes a fresh random component per publish
        // attempt and never collides, even between identical-bytes retries (Bug 4b, fix rounds 1
        // and 2: the advisory lock that used to serialize this was removed because holding a pool
        // connection for it while a second connection was needed for the re-check starved the pool
        // under concurrent load). Whichever concurrent insert commits second hits this constraint —
        // map it to a dedicated variant so the application layer can surface the same
        // `PackageVersionExists` the early existence check already returns, instead of a raw 500.
        .map_err(|e| match &e {
            sqlx::Error::Database(db_err) if db_err.constraint() == Some("npm_package_versions_npm_package_id_version_key") => DomainError::NpmVersionAlreadyExists,
            _ => DomainError::Infrastructure(e.to_string()),
        })?;
        sqlx::query!("INSERT INTO npm_download_counters (npm_package_version_id, count) VALUES ($1, 0)", version.id)
            .execute(&mut *tx)
            .await
            .infra_err()?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn publish_version(&self, package: &NpmPackage, version: &NpmPackageVersion, dist_tags: &[String]) -> Result<Uuid, DomainError> {
        let origin = match version.origin {
            NpmPackageOrigin::Local => "local",
            NpmPackageOrigin::ProxyCache => "proxy_cache",
        };
        let mut tx = self.pool.begin().await.infra_err()?;
        let package_id = lock_package_creating_it(&mut tx, package).await?;

        let release = version.version.release();
        if was_unpublished(&mut *tx, package.package_repository_id, &package.name, &release).await? {
            return Err(DomainError::NpmVersionAlreadyExists);
        }
        let held = sqlx::query!(
            "SELECT count(*) AS \"versions!\", COALESCE(SUM(octet_length(manifest::text)), 0)::BIGINT AS \"manifest_bytes!\", \
                    COALESCE(bool_or(split_part(version, '+', 1) = $2), false) AS \"same_release!\" \
             FROM npm_package_versions WHERE npm_package_id = $1",
            package_id,
            release,
        )
        .fetch_one(&mut *tx)
        .await
        .infra_err()?;
        if held.same_release {
            return Err(DomainError::NpmVersionAlreadyExists);
        }
        if held.versions >= MAX_VERSIONS_PER_PACKAGE {
            return Err(DomainError::NpmPackageLimit(format!("a package can hold at most {MAX_VERSIONS_PER_PACKAGE} versions")));
        }
        let manifest_bytes = serde_json::to_vec(&version.manifest).map_or(0, |bytes| bytes.len() as i64);
        if held.manifest_bytes + manifest_bytes > MAX_MANIFEST_BYTES_PER_PACKAGE {
            return Err(DomainError::NpmPackageLimit(format!("the version manifests of a package can take at most {} MiB in total", MAX_MANIFEST_BYTES_PER_PACKAGE / (1024 * 1024))));
        }

        sqlx::query!(
            "INSERT INTO npm_package_versions \
                (id, npm_package_id, version, manifest, shasum, integrity, tarball_storage_key, tarball_size_bytes, \
                 deprecated, deprecated_message, published_by, published_at, origin) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            version.id,
            package_id,
            version.version.as_str(),
            version.manifest,
            version.shasum,
            version.integrity,
            version.tarball_storage_key,
            version.tarball_size_bytes,
            version.deprecated,
            version.deprecated_message,
            version.published_by,
            version.published_at,
            origin,
        )
        .execute(&mut *tx)
        .await
        // Of two publishes of the same new version, the one that commits second is stopped by this constraint.
        .map_err(|e| match &e {
            sqlx::Error::Database(db_err) if db_err.constraint() == Some("npm_package_versions_npm_package_id_version_key") => DomainError::NpmVersionAlreadyExists,
            _ => DomainError::Infrastructure(e.to_string()),
        })?;
        sqlx::query!("INSERT INTO npm_download_counters (npm_package_version_id, count) VALUES ($1, 0)", version.id).execute(&mut *tx).await.infra_err()?;
        for tag in dist_tags {
            sqlx::query!(
                "INSERT INTO npm_dist_tags (npm_package_id, tag, version, updated_at) VALUES ($1, $2, $3, now()) \
                 ON CONFLICT (npm_package_id, tag) DO UPDATE SET version = EXCLUDED.version, updated_at = now()",
                package_id,
                tag,
                version.version.as_str(),
            )
            .execute(&mut *tx)
            .await
            .infra_err()?;
        }
        tx.commit().await.map_err(|e| DomainError::CommitFailed(e.to_string()))?;
        Ok(package_id)
    }

    async fn unpublish_version(&self, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<UnpublishVersionOutcome, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        let Some(package_id) = lock_package(&mut tx, repository_id, name).await? else {
            return Ok(UnpublishVersionOutcome::PackageNotFound);
        };
        let removed = sqlx::query!("DELETE FROM npm_package_versions WHERE npm_package_id = $1 AND version = $2 RETURNING tarball_storage_key", package_id, version.as_str())
            .fetch_optional(&mut *tx)
            .await
            .infra_err()?;
        let Some(removed) = removed else {
            return Ok(UnpublishVersionOutcome::VersionNotFound);
        };
        record_unpublished(&mut tx, repository_id, name, version).await?;
        sqlx::query!("DELETE FROM npm_dist_tags WHERE npm_package_id = $1 AND version = $2", package_id, version.as_str()).execute(&mut *tx).await.infra_err()?;
        let has_versions = sqlx::query!("SELECT EXISTS(SELECT 1 FROM npm_package_versions WHERE npm_package_id = $1) AS \"exists!\"", package_id)
            .fetch_one(&mut *tx)
            .await
            .infra_err()?;
        if !has_versions.exists {
            sqlx::query!("DELETE FROM npm_packages WHERE id = $1", package_id).execute(&mut *tx).await.infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(UnpublishVersionOutcome::Removed(UnpublishedVersion { package_id, tarball_storage_key: removed.tarball_storage_key, package_deleted: !has_versions.exists }))
    }

    async fn unpublish_package(&self, repository_id: Uuid, name: &NpmPackageName) -> Result<Option<UnpublishedPackage>, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        let Some(package_id) = lock_package(&mut tx, repository_id, name).await? else {
            return Ok(None);
        };
        let versions = sqlx::query!("SELECT version, tarball_storage_key FROM npm_package_versions WHERE npm_package_id = $1", package_id).fetch_all(&mut *tx).await.infra_err()?;
        for version in &versions {
            record_unpublished(&mut tx, repository_id, name, &NpmVersion::parse(&version.version)?).await?;
        }
        sqlx::query!("DELETE FROM npm_packages WHERE id = $1", package_id).execute(&mut *tx).await.infra_err()?;
        tx.commit().await.infra_err()?;
        Ok(Some(UnpublishedPackage { package_id, tarball_storage_keys: versions.into_iter().map(|v| v.tarball_storage_key).collect() }))
    }

    async fn was_unpublished(&self, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<bool, DomainError> {
        was_unpublished(&self.pool, repository_id, name, &version.release()).await
    }

    async fn set_deprecated(&self, npm_package_id: Uuid, version: &NpmVersion, message: Option<&str>) -> Result<(), DomainError> {
        sqlx::query!(
            "UPDATE npm_package_versions SET deprecated = TRUE, deprecated_message = $3 WHERE npm_package_id = $1 AND version = $2",
            npm_package_id,
            version.as_str(),
            message,
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn list_dist_tags(&self, npm_package_id: Uuid) -> Result<Vec<NpmDistTag>, DomainError> {
        let rows = sqlx::query!("SELECT tag, version FROM npm_dist_tags WHERE npm_package_id = $1", npm_package_id)
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        rows.into_iter()
            .map(|r| Ok(NpmDistTag { npm_package_id, tag: r.tag, version: NpmVersion::parse(&r.version)? }))
            .collect()
    }

    async fn list_dist_tags_for_packages(&self, npm_package_ids: &[Uuid]) -> Result<Vec<NpmDistTag>, DomainError> {
        if npm_package_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query!(
            "SELECT npm_package_id, tag, version FROM npm_dist_tags WHERE npm_package_id = ANY($1) ORDER BY npm_package_id",
            npm_package_ids
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter()
            .map(|r| Ok(NpmDistTag { npm_package_id: r.npm_package_id, tag: r.tag, version: NpmVersion::parse(&r.version)? }))
            .collect()
    }

    async fn set_dist_tag(&self, npm_package_id: Uuid, tag: &str, version: &NpmVersion) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("SELECT id FROM npm_packages WHERE id = $1 FOR UPDATE", npm_package_id).fetch_optional(&mut *tx).await.infra_err()?;
        let exists = sqlx::query!("SELECT EXISTS(SELECT 1 FROM npm_package_versions WHERE npm_package_id = $1 AND version = $2) AS \"exists!\"", npm_package_id, version.as_str())
            .fetch_one(&mut *tx)
            .await
            .infra_err()?;
        if !exists.exists {
            return Err(DomainError::NpmVersionNotFound);
        }
        sqlx::query!(
            "INSERT INTO npm_dist_tags (npm_package_id, tag, version, updated_at) VALUES ($1, $2, $3, now()) \
             ON CONFLICT (npm_package_id, tag) DO UPDATE SET version = EXCLUDED.version, updated_at = now()",
            npm_package_id,
            tag,
            version.as_str(),
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn delete_dist_tag(&self, npm_package_id: Uuid, tag: &str) -> Result<(), DomainError> {
        sqlx::query!("DELETE FROM npm_dist_tags WHERE npm_package_id = $1 AND tag = $2", npm_package_id, tag)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn search(&self, repository_id: Uuid, query: &str, limit: i64) -> Result<Vec<NpmPackage>, DomainError> {
        let rows = sqlx::query_as!(
            SearchRow,
            "SELECT id, package_repository_id, name, created_at, updated_at \
             FROM npm_packages WHERE package_repository_id = $1 AND name ILIKE '%' || $2 || '%' ORDER BY name LIMIT $3",
            repository_id,
            escape_like(query),
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(SearchRow::into_domain).collect()
    }

    async fn list_packages_page(&self, repository_id: Uuid, after: Option<&str>, limit: i64) -> Result<Vec<NpmPackage>, DomainError> {
        let rows = sqlx::query_as!(
            SearchRow,
            "SELECT id, package_repository_id, name, created_at, updated_at \
             FROM npm_packages WHERE package_repository_id = $1 AND ($2::text IS NULL OR name > $2) ORDER BY name LIMIT $3",
            repository_id,
            after,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(SearchRow::into_domain).collect()
    }

    async fn set_cached_metadata(&self, npm_package_id: Uuid, metadata: serde_json::Value) -> Result<(), DomainError> {
        sqlx::query!("UPDATE npm_packages SET cached_metadata = $2, updated_at = now() WHERE id = $1", npm_package_id, metadata)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

    fn sample_package(repository_id: Uuid, name: &str) -> NpmPackage {
        NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: NpmPackageName::parse(name).unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        }
    }

    #[sqlx::test]
    async fn creates_and_finds_a_package_by_name(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.create_package(&package).await.unwrap();

        let found = repo.find_package(repository_id, &package.name).await.unwrap().unwrap();
        assert_eq!(found.id, package.id);
        assert_eq!(found.package_repository_id, repository_id);
        assert_eq!(found.name.as_str(), "left-pad");
        assert!(found.metadata_fetched_at.is_none());
        assert!(found.cached_metadata.is_none());

        let other_name = NpmPackageName::parse("right-pad").unwrap();
        assert!(repo.find_package(repository_id, &other_name).await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn inserts_and_lists_versions(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.create_package(&package).await.unwrap();

        let version = NpmPackageVersion {
            id: Uuid::new_v4(), npm_package_id: package.id, version: NpmVersion::parse("1.0.0").unwrap(),
            manifest: serde_json::json!({"name": "left-pad"}), shasum: "abc".into(), integrity: "sha512-xyz".into(),
            tarball_storage_key: "left-pad/-/left-pad-1.0.0.tgz".into(), tarball_size_bytes: 42,
            deprecated: false, deprecated_message: None, published_by: None, published_at: chrono::Utc::now(),
            origin: NpmPackageOrigin::Local,
        };
        repo.insert_version(&version).await.unwrap();

        let versions = repo.list_versions(package.id).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].id, version.id);
        assert_eq!(versions[0].shasum, "abc");
        assert_eq!(versions[0].integrity, "sha512-xyz");
        assert_eq!(versions[0].tarball_storage_key, "left-pad/-/left-pad-1.0.0.tgz");
        assert_eq!(versions[0].tarball_size_bytes, 42);
        assert!(!versions[0].deprecated);
        assert_eq!(versions[0].origin, NpmPackageOrigin::Local);
        assert_eq!(versions[0].manifest, serde_json::json!({"name": "left-pad"}));

        let found = repo.find_version(package.id, &version.version).await.unwrap().unwrap();
        assert_eq!(found.id, version.id);

        let count: (i64,) =
            sqlx::query_as("SELECT count FROM npm_download_counters WHERE npm_package_version_id = $1")
                .bind(version.id)
                .fetch_one(&repo.pool)
                .await
                .unwrap();
        assert_eq!(count.0, 0);

        assert!(matches!(repo.unpublish_version(repository_id, &package.name, &version.version).await.unwrap(), UnpublishVersionOutcome::Removed(_)));
        assert!(repo.list_versions(package.id).await.unwrap().is_empty());
        assert!(repo.find_version(package.id, &version.version).await.unwrap().is_none());
    }

    fn sample_version(npm_package_id: Uuid, version: &str) -> NpmPackageVersion {
        NpmPackageVersion {
            id: Uuid::new_v4(), npm_package_id, version: NpmVersion::parse(version).unwrap(),
            manifest: serde_json::json!({}), shasum: "abc".into(), integrity: "sha512-xyz".into(),
            tarball_storage_key: format!("pkg/-/pkg-{version}.tgz"), tarball_size_bytes: 1,
            deprecated: false, deprecated_message: None, published_by: None, published_at: chrono::Utc::now(),
            origin: NpmPackageOrigin::Local,
        }
    }

    #[sqlx::test]
    async fn list_versions_for_packages_batches_across_packages_without_cross_contamination(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let left_pad = sample_package(repository_id, "left-pad");
        let right_pad = sample_package(repository_id, "right-pad");
        repo.create_package(&left_pad).await.unwrap();
        repo.create_package(&right_pad).await.unwrap();
        repo.insert_version(&sample_version(left_pad.id, "1.0.0")).await.unwrap();
        repo.insert_version(&sample_version(left_pad.id, "2.0.0")).await.unwrap();
        repo.insert_version(&sample_version(right_pad.id, "1.0.0")).await.unwrap();

        let versions = repo.list_versions_for_packages(&[left_pad.id, right_pad.id]).await.unwrap();

        let left_pad_versions: Vec<_> = versions.iter().filter(|v| v.npm_package_id == left_pad.id).map(|v| v.version.as_str()).collect();
        let right_pad_versions: Vec<_> = versions.iter().filter(|v| v.npm_package_id == right_pad.id).map(|v| v.version.as_str()).collect();
        assert_eq!(versions.len(), 3);
        assert_eq!(left_pad_versions.len(), 2);
        assert_eq!(right_pad_versions, vec!["1.0.0"]);
    }

    #[sqlx::test]
    async fn list_latest_versions_for_packages_cuts_each_package_to_its_newest_versions_in_the_query(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let big = sample_package(repository_id, "big");
        let small = sample_package(repository_id, "small");
        repo.create_package(&big).await.unwrap();
        repo.create_package(&small).await.unwrap();
        let start = chrono::Utc::now() - chrono::Duration::days(1);
        for i in 0..300 {
            let mut version = sample_version(big.id, &format!("1.0.{i}"));
            version.published_at = start + chrono::Duration::seconds(i);
            repo.insert_version(&version).await.unwrap();
        }
        repo.insert_version(&sample_version(small.id, "1.0.0")).await.unwrap();

        let versions = repo.list_latest_versions_for_packages(&[big.id, small.id], 201).await.unwrap();

        let of_big: Vec<String> = versions.iter().filter(|v| v.npm_package_id == big.id).map(|v| v.version.as_str()).collect();
        assert_eq!(of_big.len(), 201, "at most the cap, not the 300 stored");
        assert_eq!((of_big.first().map(String::as_str), of_big.last().map(String::as_str)), (Some("1.0.299"), Some("1.0.99")), "the newest ones, newest first");
        assert_eq!(versions.iter().filter(|v| v.npm_package_id == small.id).count(), 1);
    }

    #[sqlx::test]
    async fn list_versions_for_packages_is_empty_for_an_empty_input(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        assert!(repo.list_versions_for_packages(&[]).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn dist_tags_round_trip(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.create_package(&package).await.unwrap();
        repo.insert_version(&sample_version(package.id, "1.0.0")).await.unwrap();
        repo.insert_version(&sample_version(package.id, "2.0.0")).await.unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();

        repo.set_dist_tag(package.id, "latest", &version).await.unwrap();
        let tags = repo.list_dist_tags(package.id).await.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].tag, "latest");
        assert_eq!(tags[0].version, version);
        assert_eq!(tags[0].npm_package_id, package.id);

        let version_2 = NpmVersion::parse("2.0.0").unwrap();
        repo.set_dist_tag(package.id, "latest", &version_2).await.unwrap();
        let tags = repo.list_dist_tags(package.id).await.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].version, version_2);

        repo.delete_dist_tag(package.id, "latest").await.unwrap();
        assert!(repo.list_dist_tags(package.id).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn search_matches_a_name_substring(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        repo.create_package(&sample_package(repository_id, "left-pad")).await.unwrap();
        repo.create_package(&sample_package(repository_id, "right-pad")).await.unwrap();

        let results = repo.search(repository_id, "left", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name.as_str(), "left-pad");

        let all = repo.search(repository_id, "pad", 10).await.unwrap();
        assert_eq!(all.len(), 2);
        let limited = repo.search(repository_id, "pad", 1).await.unwrap();
        assert_eq!(limited.len(), 1);

        assert!(repo.search(repository_id, "nonexistent", 10).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn list_packages_page_follows_the_name_order_from_after_and_stops_at_the_limit(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        for name in ["d", "b", "e", "a", "c"] {
            repo.create_package(&sample_package(repository_id, name)).await.unwrap();
        }
        repo.create_package(&sample_package(Uuid::new_v4(), "z")).await.ok();

        let names = |page: Vec<NpmPackage>| page.into_iter().map(|p| p.name.as_str().to_string()).collect::<Vec<_>>();

        assert_eq!(names(repo.list_packages_page(repository_id, None, 2).await.unwrap()), vec!["a", "b"]);
        assert_eq!(names(repo.list_packages_page(repository_id, Some("b"), 2).await.unwrap()), vec!["c", "d"]);
        assert_eq!(names(repo.list_packages_page(repository_id, Some("d"), 10).await.unwrap()), vec!["e"]);
        assert!(repo.list_packages_page(repository_id, Some("e"), 10).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn search_treats_like_wildcards_as_literal_text(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        repo.create_package(&sample_package(repository_id, "left-pad")).await.unwrap();
        repo.create_package(&sample_package(repository_id, "my_pkg")).await.unwrap();
        repo.create_package(&sample_package(repository_id, "myxpkg")).await.unwrap();

        assert!(repo.search(repository_id, "%", 10).await.unwrap().is_empty(), "a bare % must not match everything");
        let underscore = repo.search(repository_id, "my_pkg", 10).await.unwrap();
        assert_eq!(underscore.len(), 1, "_ must not match any single character");
        assert_eq!(underscore[0].name.as_str(), "my_pkg");
        assert!(repo.search(repository_id, "\\", 10).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn touch_metadata_fetched_at_and_set_cached_metadata_update_the_package(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.create_package(&package).await.unwrap();

        let fetched_at = chrono::Utc::now();
        repo.touch_metadata_fetched_at(package.id, fetched_at).await.unwrap();
        let found = repo.find_package(repository_id, &package.name).await.unwrap().unwrap();
        assert_eq!(
            found.metadata_fetched_at.unwrap().timestamp_millis(),
            fetched_at.timestamp_millis()
        );

        let metadata = serde_json::json!({"dist-tags": {"latest": "1.0.0"}});
        repo.set_cached_metadata(package.id, metadata.clone()).await.unwrap();
        let found = repo.find_package(repository_id, &package.name).await.unwrap().unwrap();
        assert_eq!(found.cached_metadata, Some(metadata));
    }

    #[sqlx::test]
    async fn set_deprecated_marks_the_version_deprecated_with_a_message(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.create_package(&package).await.unwrap();

        let version = NpmPackageVersion {
            id: Uuid::new_v4(), npm_package_id: package.id, version: NpmVersion::parse("1.0.0").unwrap(),
            manifest: serde_json::json!({"name": "left-pad"}), shasum: "abc".into(), integrity: "sha512-xyz".into(),
            tarball_storage_key: "left-pad/-/left-pad-1.0.0.tgz".into(), tarball_size_bytes: 42,
            deprecated: false, deprecated_message: None, published_by: None, published_at: chrono::Utc::now(),
            origin: NpmPackageOrigin::Local,
        };
        repo.insert_version(&version).await.unwrap();

        repo.set_deprecated(package.id, &version.version, Some("use left-pad2 instead")).await.unwrap();
        let found = repo.find_version(package.id, &version.version).await.unwrap().unwrap();
        assert!(found.deprecated);
        assert_eq!(found.deprecated_message.as_deref(), Some("use left-pad2 instead"));
    }

    #[sqlx::test]
    async fn unpublishing_a_whole_package_removes_it_with_its_versions_and_tags_and_remembers_every_version(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        let tags = vec!["latest".to_string()];
        repo.publish_version(&package, &sample_version(package.id, "1.0.0"), &[]).await.unwrap();
        let package_id = repo.publish_version(&package, &sample_version(package.id, "1.0.1"), &tags).await.unwrap();

        let removed = repo.unpublish_package(repository_id, &package.name).await.unwrap().unwrap();

        assert_eq!(removed.package_id, package_id);
        assert_eq!(removed.tarball_storage_keys.len(), 2);
        assert!(repo.find_package(repository_id, &package.name).await.unwrap().is_none());
        assert!(repo.list_versions(package_id).await.unwrap().is_empty());
        assert!(repo.list_dist_tags(package_id).await.unwrap().is_empty());
        for version in ["1.0.0", "1.0.1"] {
            assert!(repo.was_unpublished(repository_id, &package.name, &NpmVersion::parse(version).unwrap()).await.unwrap());
        }
        assert!(repo.unpublish_package(repository_id, &package.name).await.unwrap().is_none());
    }

    async fn seed_repository(pool: &sqlx::PgPool, id: Uuid) {
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'npm', 'hosted', NULL, 1, now(), now())",
            id,
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            format!("repo-{id}"),
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn creating_the_same_package_twice_hands_back_the_one_row(pool: sqlx::PgPool) {
        let repo = std::sync::Arc::new(PostgresNpmPackageRepository::new(pool));
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;

        let first = sample_package(repository_id, "left-pad");
        let second = sample_package(repository_id, "left-pad");
        assert_ne!(first.id, second.id);

        assert_eq!(repo.create_package(&first).await.unwrap(), first.id);
        assert_eq!(repo.create_package(&second).await.unwrap(), first.id, "the loser learns the winner's id");
    }

    #[sqlx::test]
    async fn concurrent_cold_requests_creating_a_package_all_succeed_on_one_row(pool: sqlx::PgPool) {
        let repo = std::sync::Arc::new(PostgresNpmPackageRepository::new(pool));
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let repo = repo.clone();
                tokio::spawn(async move { repo.create_package(&sample_package(repository_id, "left-pad")).await })
            })
            .collect();
        let mut ids = std::collections::HashSet::new();
        for handle in handles {
            ids.insert(handle.await.unwrap().unwrap());
        }

        assert_eq!(ids.len(), 1);
    }

    #[sqlx::test]
    async fn an_unpublished_version_is_remembered_per_repository_and_name_and_cannot_be_published_again(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        let other_repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        seed_repository(&repo.pool, other_repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        let version = sample_version(package.id, "1.0.0");
        repo.publish_version(&package, &version, &[]).await.unwrap();
        let name = package.name.clone();

        assert!(!repo.was_unpublished(repository_id, &name, &version.version).await.unwrap());
        assert!(matches!(repo.unpublish_version(repository_id, &name, &version.version).await.unwrap(), UnpublishVersionOutcome::Removed(_)));

        assert!(repo.was_unpublished(repository_id, &name, &version.version).await.unwrap());
        assert!(!repo.was_unpublished(other_repository_id, &name, &version.version).await.unwrap());
        assert!(!repo.was_unpublished(repository_id, &name, &NpmVersion::parse("1.0.1").unwrap()).await.unwrap());
        let again = repo.publish_version(&sample_package(repository_id, "left-pad"), &sample_version(package.id, "1.0.0"), &["latest".to_string()]).await;
        assert!(matches!(again, Err(DomainError::NpmVersionAlreadyExists)));
        assert!(repo.find_package(repository_id, &name).await.unwrap().is_none(), "a refused publish leaves no package row and no tag behind");
    }

    #[sqlx::test]
    async fn unpublishing_a_version_drops_its_tags_and_the_package_only_with_its_last_version(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        let name = package.name.clone();
        let package_id = repo.publish_version(&package, &sample_version(package.id, "1.0.0"), &["stable".to_string()]).await.unwrap();
        repo.publish_version(&package, &sample_version(package.id, "2.0.0"), &["latest".to_string()]).await.unwrap();

        let UnpublishVersionOutcome::Removed(first) = repo.unpublish_version(repository_id, &name, &NpmVersion::parse("2.0.0").unwrap()).await.unwrap() else { panic!("not removed") };
        assert!(!first.package_deleted);
        assert_eq!(first.tarball_storage_key, "pkg/-/pkg-2.0.0.tgz");
        let tags: Vec<String> = repo.list_dist_tags(package_id).await.unwrap().into_iter().map(|t| t.tag).collect();
        assert_eq!(tags, vec!["stable".to_string()], "the tag that pointed at the removed version is gone");

        let UnpublishVersionOutcome::Removed(last) = repo.unpublish_version(repository_id, &name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap() else { panic!("not removed") };
        assert!(last.package_deleted);
        assert!(repo.find_by_id(package_id).await.unwrap().is_none());
        assert!(matches!(repo.unpublish_version(repository_id, &name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap(), UnpublishVersionOutcome::PackageNotFound));
    }

    #[sqlx::test]
    async fn a_publish_waits_for_an_unpublish_holding_the_package_and_then_recreates_the_package(pool: sqlx::PgPool) {
        let repo = std::sync::Arc::new(PostgresNpmPackageRepository::new(pool.clone()));
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.publish_version(&package, &sample_version(package.id, "1.0.0"), &[]).await.unwrap();

        let mut unpublish_tx = pool.begin().await.unwrap();
        let locked = lock_package(&mut unpublish_tx, repository_id, &package.name).await.unwrap().unwrap();
        let publish = {
            let repo = repo.clone();
            let package = sample_package(repository_id, "left-pad");
            tokio::spawn(async move { repo.publish_version(&package, &sample_version(package.id, "2.0.0"), &["latest".to_string()]).await })
        };
        let mut queued = false;
        for _ in 0..500 {
            let waiting: (i64,) = sqlx::query_as("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%FOR UPDATE%'").fetch_one(&pool).await.unwrap();
            if waiting.0 >= 1 {
                queued = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(queued, "the publish never waited on the package lock");
        sqlx::query!("DELETE FROM npm_packages WHERE id = $1", locked).execute(&mut *unpublish_tx).await.unwrap();
        unpublish_tx.commit().await.unwrap();

        let new_package_id = publish.await.unwrap().expect("the publish must not fail on the package that vanished under it");

        assert_ne!(new_package_id, locked, "the package was recreated");
        assert_eq!(repo.list_versions(new_package_id).await.unwrap().len(), 1);
        assert_eq!(repo.list_dist_tags(new_package_id).await.unwrap().len(), 1);
    }

    #[sqlx::test]
    async fn a_tag_cannot_point_at_a_version_that_does_not_exist(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        let package_id = repo.publish_version(&package, &sample_version(package.id, "1.0.0"), &[]).await.unwrap();

        let err = repo.set_dist_tag(package_id, "beta", &NpmVersion::parse("9.9.9").unwrap()).await.unwrap_err();

        assert_eq!(err, DomainError::NpmVersionNotFound);
        assert!(repo.list_dist_tags(package_id).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn setting_a_tag_waits_for_an_unpublish_in_flight_and_then_finds_the_version_gone(pool: sqlx::PgPool) {
        let repo = std::sync::Arc::new(PostgresNpmPackageRepository::new(pool.clone()));
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.publish_version(&package, &sample_version(package.id, "1.0.0"), &[]).await.unwrap();
        let package_id = repo.publish_version(&package, &sample_version(package.id, "2.0.0"), &[]).await.unwrap();

        let mut unpublish_tx = pool.begin().await.unwrap();
        lock_package(&mut unpublish_tx, repository_id, &package.name).await.unwrap().unwrap();
        sqlx::query!("DELETE FROM npm_package_versions WHERE npm_package_id = $1 AND version = '2.0.0'", package_id).execute(&mut *unpublish_tx).await.unwrap();
        let tagging = {
            let repo = repo.clone();
            tokio::spawn(async move { repo.set_dist_tag(package_id, "beta", &NpmVersion::parse("2.0.0").unwrap()).await })
        };
        let mut queued = false;
        for _ in 0..500 {
            let waiting: (i64,) = sqlx::query_as("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%FOR UPDATE%'").fetch_one(&pool).await.unwrap();
            if waiting.0 >= 1 {
                queued = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(queued, "the tag change never waited on the package lock");
        unpublish_tx.commit().await.unwrap();

        assert_eq!(tagging.await.unwrap().unwrap_err(), DomainError::NpmVersionNotFound);
        assert!(repo.list_dist_tags(package_id).await.unwrap().is_empty(), "no tag may be left on the removed version");
    }

    #[sqlx::test]
    async fn versions_that_differ_only_in_build_metadata_are_the_same_release(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.publish_version(&package, &sample_version(package.id, "1.0.0"), &[]).await.unwrap();
        repo.publish_version(&package, &sample_version(package.id, "2.0.0+first"), &[]).await.unwrap();

        let plain_after_build = repo.publish_version(&package, &sample_version(package.id, "1.0.0+rebuilt"), &[]).await;
        let build_after_build = repo.publish_version(&package, &sample_version(package.id, "2.0.0+second"), &[]).await;
        let bare_after_build = repo.publish_version(&package, &sample_version(package.id, "2.0.0"), &[]).await;
        let prerelease = repo.publish_version(&package, &sample_version(package.id, "1.0.0-beta.1+x"), &[]).await;

        assert_eq!(plain_after_build.unwrap_err(), DomainError::NpmVersionAlreadyExists);
        assert_eq!(build_after_build.unwrap_err(), DomainError::NpmVersionAlreadyExists);
        assert_eq!(bare_after_build.unwrap_err(), DomainError::NpmVersionAlreadyExists);
        prerelease.expect("a prerelease is another release");
    }

    #[sqlx::test]
    async fn an_unpublished_version_cannot_come_back_under_other_build_metadata(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        repo.publish_version(&package, &sample_version(package.id, "1.0.0"), &[]).await.unwrap();
        repo.publish_version(&package, &sample_version(package.id, "2.0.0"), &[]).await.unwrap();
        repo.unpublish_version(repository_id, &package.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        let republished = repo.publish_version(&package, &sample_version(package.id, "1.0.0+evil"), &[]).await;

        assert_eq!(republished.unwrap_err(), DomainError::NpmVersionAlreadyExists);
        assert!(repo.was_unpublished(repository_id, &package.name, &NpmVersion::parse("1.0.0+other").unwrap()).await.unwrap());
    }

    #[sqlx::test]
    async fn a_package_cannot_take_more_than_the_allowed_number_of_versions(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        let package_id = repo.publish_version(&package, &sample_version(package.id, "0.0.1"), &[]).await.unwrap();
        sqlx::query!(
            "INSERT INTO npm_package_versions (id, npm_package_id, version, manifest, shasum, integrity, tarball_storage_key, tarball_size_bytes, origin) \
             SELECT gen_random_uuid(), $1, '1.0.' || n, '{}'::jsonb, 'abc', 'sha512-xyz', 'k' || n, 1, 'local' FROM generate_series(1, $2::int - 1) n",
            package_id,
            MAX_VERSIONS_PER_PACKAGE as i32,
        )
        .execute(&repo.pool)
        .await
        .unwrap();

        let err = repo.publish_version(&package, &sample_version(package.id, "2.0.0"), &[]).await.unwrap_err();

        assert!(matches!(err, DomainError::NpmPackageLimit(_)), "{err:?}");
    }

    #[sqlx::test]
    async fn a_package_cannot_take_more_manifest_bytes_than_the_allowed_total(pool: sqlx::PgPool) {
        let repo = PostgresNpmPackageRepository::new(pool);
        let repository_id = Uuid::new_v4();
        seed_repository(&repo.pool, repository_id).await;
        let package = sample_package(repository_id, "left-pad");
        let package_id = repo.publish_version(&package, &sample_version(package.id, "0.0.1"), &[]).await.unwrap();
        sqlx::query!(
            "INSERT INTO npm_package_versions (id, npm_package_id, version, manifest, shasum, integrity, tarball_storage_key, tarball_size_bytes, origin) \
             SELECT gen_random_uuid(), $1, '1.0.' || n, jsonb_build_object('padding', repeat('a', 1000000)), 'abc', 'sha512-xyz', 'k' || n, 1, 'local' FROM generate_series(1, 32) n",
            package_id,
        )
        .execute(&repo.pool)
        .await
        .unwrap();
        let mut heavy = sample_version(package.id, "2.0.0");
        heavy.manifest = serde_json::json!({ "padding": "a".repeat(2_000_000) });

        let err = repo.publish_version(&package, &heavy, &[]).await.unwrap_err();

        assert!(matches!(err, DomainError::NpmPackageLimit(_)), "{err:?}");
        repo.publish_version(&package, &sample_version(package.id, "2.0.1"), &[]).await.expect("a small manifest still fits");
    }
}
