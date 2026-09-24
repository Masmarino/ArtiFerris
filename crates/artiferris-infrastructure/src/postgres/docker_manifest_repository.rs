use async_trait::async_trait;
use chrono::{DateTime, Utc};
use artiferris_domain::docker_registry::{DOCKER_TAG_QUOTA_BYTES, Digest, DockerImageName, DockerManifest, DockerManifestRepositoryPort, DockerMediaType, MAX_TAGS_PER_REPOSITORY};
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresDockerManifestRepository {
    pool: PgPool,
}

impl PostgresDockerManifestRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Takes back the reference each blob of the manifest holds. Meant to run in the transaction that deletes the manifest.
async fn release_blob_references(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, manifest_id: Uuid) -> Result<(), DomainError> {
    sqlx::query!(
        "UPDATE docker_blobs SET reference_count = reference_count - 1 \
         WHERE digest IN (SELECT blob_digest FROM docker_manifest_blobs WHERE manifest_id = $1)",
        manifest_id
    )
    .execute(&mut **tx)
    .await
    .infra_err()?;
    Ok(())
}

/// Each digest once: `docker_manifest_blobs` has one row per (manifest, blob), however often a layer is listed.
fn unique_digest_strs(digests: &[Digest]) -> Vec<&str> {
    let mut seen = std::collections::HashSet::new();
    digests.iter().map(Digest::as_str).filter(|digest| seen.insert(*digest)).collect()
}

struct ManifestRow {
    id: Uuid,
    package_repository_id: Uuid,
    image_name: String,
    digest: String,
    media_type: String,
    body: Vec<u8>,
    created_at: DateTime<Utc>,
}

impl ManifestRow {
    fn into_domain(self) -> Result<DockerManifest, DomainError> {
        Ok(DockerManifest {
            id: self.id,
            package_repository_id: self.package_repository_id,
            image_name: DockerImageName::parse(&self.image_name)?,
            digest: Digest::parse(&self.digest)?,
            media_type: DockerMediaType::parse(&self.media_type)?,
            body: self.body,
            created_at: self.created_at,
        })
    }
}

#[async_trait]
impl DockerManifestRepositoryPort for PostgresDockerManifestRepository {
    async fn find_manifest_by_tag(&self, repository_id: Uuid, image_name: &DockerImageName, tag: &str) -> Result<Option<DockerManifest>, DomainError> {
        let row = sqlx::query_as!(
            ManifestRow,
            "SELECT m.id, m.package_repository_id, m.image_name, m.digest, m.media_type, m.body, m.created_at \
             FROM docker_manifests m JOIN docker_tags t ON t.manifest_id = m.id \
             WHERE t.package_repository_id = $1 AND t.image_name = $2 AND t.tag = $3",
            repository_id,
            image_name.as_str(),
            tag
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(ManifestRow::into_domain).transpose()
    }

    async fn find_manifest_by_digest(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest) -> Result<Option<DockerManifest>, DomainError> {
        let row = sqlx::query_as!(
            ManifestRow,
            "SELECT id, package_repository_id, image_name, digest, media_type, body, created_at FROM docker_manifests \
             WHERE package_repository_id = $1 AND image_name = $2 AND digest = $3",
            repository_id,
            image_name.as_str(),
            digest.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(ManifestRow::into_domain).transpose()
    }

    async fn insert_manifest(&self, manifest: &DockerManifest, blob_digests: &[Digest]) -> Result<(Uuid, bool), DomainError> {
        // Transactional so a crash mid-insert can't leave the manifest row without its blob rows.
        let mut tx = self.pool.begin().await.infra_err()?;

        // Idempotent for a re-pushed byte-identical manifest. `RETURNING id` distinguishes "inserted" from "already present" so the caller gets the real, persisted id either way.
        let inserted = sqlx::query!(
            "INSERT INTO docker_manifests (id, package_repository_id, image_name, digest, media_type, body, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (package_repository_id, image_name, digest) DO NOTHING \
             RETURNING id",
            manifest.id,
            manifest.package_repository_id,
            manifest.image_name.as_str(),
            manifest.digest.as_str(),
            manifest.media_type.as_str(),
            manifest.body,
            manifest.created_at,
        )
        .fetch_optional(&mut *tx)
        .await
        .infra_err()?;

        let Some(inserted) = inserted else {
            let existing = sqlx::query!(
                "SELECT id FROM docker_manifests WHERE package_repository_id = $1 AND image_name = $2 AND digest = $3",
                manifest.package_repository_id,
                manifest.image_name.as_str(),
                manifest.digest.as_str(),
            )
            .fetch_one(&mut *tx)
            .await
            .infra_err()?;
            tx.commit().await.infra_err()?;
            return Ok((existing.id, false));
        };

        let blob_digest_strs = unique_digest_strs(blob_digests);
        sqlx::query!(
            "INSERT INTO docker_manifest_blobs (manifest_id, blob_digest) SELECT $1, * FROM UNNEST($2::text[])",
            inserted.id,
            &blob_digest_strs as &[&str],
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        tx.commit().await.infra_err()?;
        Ok((inserted.id, true))
    }

    async fn insert_manifest_with_checks(
        &self,
        repository_id: Uuid,
        manifest: &DockerManifest,
        blob_digests: &[Digest],
        quota_bytes: Option<i64>,
    ) -> Result<(Uuid, bool), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;

        // Same convention as `PostgresPackageRepositoryStore::append` and friends: an advisory lock keyed
        // on the repository id, held for the rest of this transaction. Serializes concurrent pushes to
        // THIS repository only — a push to a different repository hashes to a different key and is not
        // blocked by this one (B-18).
        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", repository_id.to_string()).execute(&mut *tx).await.infra_err()?;

        // Re-verify reachability inside the lock (`blob_is_reachable` OR `is_uploaded_to_repository`), against `&mut *tx` so a
        // concurrent delete can't slip in before the insert. One query for the whole manifest.
        let wanted = unique_digest_strs(blob_digests);
        let reachable: std::collections::HashSet<String> = sqlx::query_scalar!(
            "SELECT blob_digest AS \"blob_digest!\" FROM ( \
                 SELECT dmb.blob_digest FROM docker_manifest_blobs dmb JOIN docker_manifests dm ON dm.id = dmb.manifest_id \
                 WHERE dm.package_repository_id = $1 AND dmb.blob_digest = ANY($2) \
                 UNION \
                 SELECT blob_digest FROM docker_repository_blobs WHERE package_repository_id = $1 AND blob_digest = ANY($2) \
             ) reachable",
            repository_id,
            &wanted as &[&str],
        )
        .fetch_all(&mut *tx)
        .await
        .infra_err()?
        .into_iter()
        .collect();
        if let Some(missing) = wanted.iter().find(|digest| !reachable.contains(**digest)) {
            return Err(DomainError::DockerBlobNotReachable((*missing).to_string()));
        }

        // Re-verify the quota inside the lock: what its manifests already reference plus this manifest's blobs, each once, and the
        // manifest bodies and tags (`used_bytes_for_repository` counts the same things). Uploaded-but-unreferenced blobs count at
        // upload time, not here. Re-summing under the lock closes the lost-update race (two pushes both reading "not yet exceeded").
        if let Some(quota) = quota_bytes {
            let total: i64 = sqlx::query_scalar!(
                "SELECT ( \
                     COALESCE((SELECT SUM(db.size_bytes) FROM docker_blobs db \
                               WHERE db.digest IN ( \
                                   SELECT dmb.blob_digest FROM docker_manifest_blobs dmb \
                                   JOIN docker_manifests dm ON dm.id = dmb.manifest_id \
                                   WHERE dm.package_repository_id = $1 \
                                   UNION \
                                   SELECT UNNEST($2::text[]) \
                               )), 0) \
                     + COALESCE((SELECT SUM(octet_length(body)) FROM docker_manifests \
                                 WHERE package_repository_id = $1 AND NOT (image_name = $3 AND digest = $4)), 0) \
                     + $5 \
                     + (SELECT count(*) FROM docker_tags WHERE package_repository_id = $1) * $6 \
                 )::BIGINT AS \"total!\"",
                repository_id,
                &wanted as &[&str],
                manifest.image_name.as_str(),
                manifest.digest.as_str(),
                manifest.body.len() as i64,
                DOCKER_TAG_QUOTA_BYTES,
            )
            .fetch_one(&mut *tx)
            .await
            .infra_err()?;
            if total as u64 > quota as u64 {
                return Err(DomainError::StorageQuotaExceeded);
            }
        }

        // From here down: the same body as `insert_manifest`, against this already-open `tx` instead of
        // one it opens and commits itself.
        let inserted = sqlx::query!(
            "INSERT INTO docker_manifests (id, package_repository_id, image_name, digest, media_type, body, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (package_repository_id, image_name, digest) DO NOTHING \
             RETURNING id",
            manifest.id,
            manifest.package_repository_id,
            manifest.image_name.as_str(),
            manifest.digest.as_str(),
            manifest.media_type.as_str(),
            manifest.body,
            manifest.created_at,
        )
        .fetch_optional(&mut *tx)
        .await
        .infra_err()?;

        let Some(inserted) = inserted else {
            let existing = sqlx::query!(
                "SELECT id FROM docker_manifests WHERE package_repository_id = $1 AND image_name = $2 AND digest = $3",
                manifest.package_repository_id,
                manifest.image_name.as_str(),
                manifest.digest.as_str(),
            )
            .fetch_one(&mut *tx)
            .await
            .infra_err()?;
            tx.commit().await.infra_err()?;
            return Ok((existing.id, false));
        };

        sqlx::query!(
            "INSERT INTO docker_manifest_blobs (manifest_id, blob_digest) SELECT $1, * FROM UNNEST($2::text[])",
            inserted.id,
            &wanted as &[&str],
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;

        // Only on a real insert — an idempotent re-push of byte-identical content returned above and never reaches here, so a re-push can't double-count a ref.
        sqlx::query!("UPDATE docker_blobs SET reference_count = reference_count + 1 WHERE digest = ANY($1)", &wanted as &[&str])
            .execute(&mut *tx)
            .await
            .infra_err()?;

        tx.commit().await.infra_err()?;
        Ok((inserted.id, true))
    }

    async fn insert_manifest_list_members(&self, list_manifest_id: Uuid, member_digests: &[Digest]) -> Result<(), DomainError> {
        // Transactional and idempotent: a retry after a partial failure must not PK-violate.
        let mut tx = self.pool.begin().await.infra_err()?;
        let member_digest_strs: Vec<&str> = member_digests.iter().map(|d| d.as_str()).collect();
        sqlx::query!(
            "INSERT INTO docker_manifest_list_members (list_manifest_id, member_digest) SELECT $1, * FROM UNNEST($2::text[]) \
             ON CONFLICT (list_manifest_id, member_digest) DO NOTHING",
            list_manifest_id,
            &member_digest_strs as &[&str],
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn list_manifest_blob_digests(&self, manifest_id: Uuid) -> Result<Vec<Digest>, DomainError> {
        let rows = sqlx::query!("SELECT blob_digest FROM docker_manifest_blobs WHERE manifest_id = $1", manifest_id)
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        rows.into_iter().map(|r| Digest::parse(&r.blob_digest)).collect()
    }

    async fn list_manifest_list_member_digests(&self, manifest_id: Uuid) -> Result<Vec<Digest>, DomainError> {
        let rows = sqlx::query!("SELECT member_digest FROM docker_manifest_list_members WHERE list_manifest_id = $1", manifest_id)
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        rows.into_iter().map(|r| Digest::parse(&r.member_digest)).collect()
    }

    async fn set_tag(&self, repository_id: Uuid, image_name: &DockerImageName, tag: &str, manifest_id: Uuid) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        // Keeps a reclaim of this manifest from deleting it, and the tag with it, while the tag is being set.
        sqlx::query!("SELECT id FROM docker_manifests WHERE id = $1 FOR KEY SHARE", manifest_id).fetch_optional(&mut *tx).await.infra_err()?;
        let previous = sqlx::query_scalar!(
            "SELECT manifest_id FROM docker_tags WHERE package_repository_id = $1 AND image_name = $2 AND tag = $3 FOR UPDATE",
            repository_id,
            image_name.as_str(),
            tag,
        )
        .fetch_optional(&mut *tx)
        .await
        .infra_err()?;
        if previous.is_none() {
            let held = sqlx::query_scalar!("SELECT count(*) FROM docker_tags WHERE package_repository_id = $1", repository_id).fetch_one(&mut *tx).await.infra_err()?.unwrap_or(0);
            if held >= MAX_TAGS_PER_REPOSITORY {
                return Err(DomainError::TooManyTags);
            }
        }
        sqlx::query!(
            "INSERT INTO docker_tags (package_repository_id, image_name, tag, manifest_id, updated_at) VALUES ($1, $2, $3, $4, now()) \
             ON CONFLICT (package_repository_id, image_name, tag) DO UPDATE SET manifest_id = $4, updated_at = now()",
            repository_id,
            image_name.as_str(),
            tag,
            manifest_id,
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        sqlx::query!("UPDATE docker_manifests SET untagged_since = NULL WHERE id = $1 AND untagged_since IS NOT NULL", manifest_id).execute(&mut *tx).await.infra_err()?;
        // The retention grace period for the manifest the tag left runs from now, if that was its last tag.
        if let Some(previous) = previous.filter(|previous| *previous != manifest_id) {
            sqlx::query!(
                "UPDATE docker_manifests m SET untagged_since = now() WHERE m.id = $1 AND NOT EXISTS (SELECT 1 FROM docker_tags t WHERE t.manifest_id = m.id)",
                previous
            )
            .execute(&mut *tx)
            .await
            .infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn delete_manifest(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        let manifest = sqlx::query!(
            "SELECT id FROM docker_manifests WHERE package_repository_id = $1 AND image_name = $2 AND digest = $3 FOR UPDATE",
            repository_id,
            image_name.as_str(),
            digest.as_str(),
        )
        .fetch_optional(&mut *tx)
        .await
        .infra_err()?;
        if let Some(manifest) = manifest {
            release_blob_references(&mut tx, manifest.id).await?;
            sqlx::query!("DELETE FROM docker_manifests WHERE id = $1", manifest.id).execute(&mut *tx).await.infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn list_tags(&self, repository_id: Uuid, image_name: &DockerImageName) -> Result<Vec<String>, DomainError> {
        let rows = sqlx::query!(
            "SELECT tag FROM docker_tags WHERE package_repository_id = $1 AND image_name = $2 ORDER BY tag",
            repository_id,
            image_name.as_str()
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        Ok(rows.into_iter().map(|r| r.tag).collect())
    }

    async fn list_repository_image_names(&self, repository_id: Uuid) -> Result<Vec<DockerImageName>, DomainError> {
        // An untagged manifest reachable only by digest doesn't surface here.
        let rows = sqlx::query!(
            "SELECT DISTINCT image_name FROM docker_tags WHERE package_repository_id = $1 ORDER BY image_name",
            repository_id
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(|r| DockerImageName::parse(&r.image_name)).collect()
    }

    async fn list_image_names_for_repositories(&self, repository_ids: &[Uuid]) -> Result<Vec<(Uuid, DockerImageName)>, DomainError> {
        let rows = sqlx::query!(
            "SELECT DISTINCT package_repository_id, image_name FROM docker_tags WHERE package_repository_id = ANY($1) ORDER BY package_repository_id, image_name",
            repository_ids
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        // An unparseable row is skipped rather than poisoning the whole batched result.
        Ok(rows.into_iter().filter_map(|r| DockerImageName::parse(&r.image_name).ok().map(|name| (r.package_repository_id, name))).collect())
    }

    async fn list_image_names_page(&self, repository_id: Uuid, after: Option<&str>, limit: i64) -> Result<Vec<DockerImageName>, DomainError> {
        let rows = sqlx::query!(
            "SELECT image_name FROM docker_tags WHERE package_repository_id = $1 AND ($2::text IS NULL OR image_name COLLATE \"C\" > $2 COLLATE \"C\") \
             GROUP BY image_name ORDER BY image_name COLLATE \"C\" LIMIT $3",
            repository_id,
            after,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        Ok(rows.into_iter().filter_map(|r| DockerImageName::parse(&r.image_name).ok()).collect())
    }

    async fn list_recent_tags_for_images(&self, repository_id: Uuid, image_names: &[String], per_image: i64) -> Result<Vec<(DockerImageName, String)>, DomainError> {
        let rows = sqlx::query!(
            "SELECT t.image_name AS \"image_name!\", t.tag AS \"tag!\" FROM unnest($2::text[]) AS i(name) \
             CROSS JOIN LATERAL (SELECT image_name, tag, updated_at FROM docker_tags WHERE package_repository_id = $1 AND image_name = i.name ORDER BY updated_at DESC, tag LIMIT $3) t \
             ORDER BY t.image_name, t.updated_at DESC, t.tag",
            repository_id,
            image_names,
            per_image
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        Ok(rows.into_iter().filter_map(|r| DockerImageName::parse(&r.image_name).ok().map(|name| (name, r.tag))).collect())
    }

    async fn list_latest_manifest_id_per_image(&self, repository_id: Uuid, image_names: &[String]) -> Result<Vec<(DockerImageName, Uuid)>, DomainError> {
        let rows = sqlx::query!(
            "SELECT DISTINCT ON (image_name) image_name, manifest_id \
             FROM docker_tags WHERE package_repository_id = $1 AND image_name = ANY($2) \
             ORDER BY image_name, updated_at DESC",
            repository_id,
            image_names
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        Ok(rows.into_iter().filter_map(|r| DockerImageName::parse(&r.image_name).ok().map(|name| (name, r.manifest_id))).collect())
    }

    async fn list_distinct_digests_for_image(&self, repository_id: Uuid, image_name: &DockerImageName) -> Result<Vec<Digest>, DomainError> {
        let rows = sqlx::query!(
            "SELECT DISTINCT m.digest FROM docker_manifests m JOIN docker_tags t ON t.manifest_id = m.id \
             WHERE t.package_repository_id = $1 AND t.image_name = $2",
            repository_id,
            image_name.as_str()
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(|r| Digest::parse(&r.digest)).collect()
    }

    async fn list_tag_manifest_summaries(&self, repository_id: Uuid, image_name: &DockerImageName, limit: i64) -> Result<Vec<(String, Digest, DockerMediaType, DateTime<Utc>)>, DomainError> {
        let rows = sqlx::query!(
            "SELECT t.tag, m.digest, m.media_type, m.created_at \
             FROM docker_tags t JOIN docker_manifests m ON m.id = t.manifest_id \
             WHERE t.package_repository_id = $1 AND t.image_name = $2 ORDER BY t.updated_at DESC, t.tag LIMIT $3",
            repository_id,
            image_name.as_str(),
            limit
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(|r| Ok((r.tag, Digest::parse(&r.digest)?, DockerMediaType::parse(&r.media_type)?, r.created_at))).collect()
    }

    async fn list_tagged_manifest_bodies(&self, repository_id: Uuid, image_name: &DockerImageName, digests: &[String], max_bytes: i64) -> Result<Vec<(Digest, Vec<u8>)>, DomainError> {
        let rows = sqlx::query!(
            "SELECT DISTINCT ON (m.digest) m.digest, m.body \
             FROM docker_tags t JOIN docker_manifests m ON m.id = t.manifest_id \
             WHERE t.package_repository_id = $1 AND t.image_name = $2 AND m.digest = ANY($3) AND octet_length(m.body)::bigint <= $4 ORDER BY m.digest",
            repository_id,
            image_name.as_str(),
            digests,
            max_bytes
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(|r| Ok((Digest::parse(&r.digest)?, r.body))).collect()
    }

    async fn list_repository_tag_manifest_summaries(&self, repository_id: Uuid) -> Result<Vec<(DockerImageName, String, Digest, DockerMediaType, DateTime<Utc>)>, DomainError> {
        let rows = sqlx::query!(
            "SELECT t.image_name, t.tag, m.digest, m.media_type, m.created_at \
             FROM docker_tags t JOIN docker_manifests m ON m.id = t.manifest_id \
             WHERE t.package_repository_id = $1 ORDER BY t.image_name, t.tag",
            repository_id
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter()
            .map(|r| Ok((DockerImageName::parse(&r.image_name)?, r.tag, Digest::parse(&r.digest)?, DockerMediaType::parse(&r.media_type)?, r.created_at)))
            .collect()
    }

    async fn list_repository_tag_updates(&self, repository_id: Uuid) -> Result<Vec<(DockerImageName, String, Digest, DateTime<Utc>)>, DomainError> {
        let rows = sqlx::query!(
            "SELECT t.image_name, t.tag, m.digest, t.updated_at \
             FROM docker_tags t JOIN docker_manifests m ON m.id = t.manifest_id \
             WHERE t.package_repository_id = $1 ORDER BY t.image_name, t.tag",
            repository_id
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        // A row that no longer parses is skipped rather than failing the listing.
        Ok(rows
            .into_iter()
            .filter_map(|r| match (DockerImageName::parse(&r.image_name), Digest::parse(&r.digest)) {
                (Ok(image_name), Ok(digest)) => Some((image_name, r.tag, digest, r.updated_at)),
                _ => {
                    tracing::warn!(repository_id = %repository_id, image_name = r.image_name, tag = r.tag, "skipping a tag whose image name or digest does not parse");
                    None
                }
            })
            .collect())
    }

    async fn list_untagged_manifests(&self, repository_id: Uuid, untagged_before: DateTime<Utc>) -> Result<Vec<(DockerImageName, Digest)>, DomainError> {
        let rows = sqlx::query!(
            "SELECT m.image_name, m.digest FROM docker_manifests m \
             WHERE m.package_repository_id = $1 AND COALESCE(m.untagged_since, m.created_at) < $2 \
               AND NOT EXISTS (SELECT 1 FROM docker_tags t WHERE t.manifest_id = m.id) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM docker_manifest_list_members lm JOIN docker_manifests l ON l.id = lm.list_manifest_id \
                   WHERE l.package_repository_id = m.package_repository_id AND l.image_name = m.image_name AND lm.member_digest = m.digest \
               ) \
             ORDER BY m.created_at",
            repository_id,
            untagged_before
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(|r| Ok((DockerImageName::parse(&r.image_name)?, Digest::parse(&r.digest)?))).collect()
    }

    async fn delete_untagged_manifest(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest, untagged_before: DateTime<Utc>) -> Result<bool, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        // Locked first and checked second: a check made in the locking statement can't see a tag committed while it waited.
        let untagged = sqlx::query!(
            "SELECT m.id FROM docker_manifests m \
             WHERE m.package_repository_id = $1 AND m.image_name = $2 AND m.digest = $3 AND COALESCE(m.untagged_since, m.created_at) < $4 \
             FOR UPDATE",
            repository_id,
            image_name.as_str(),
            digest.as_str(),
            untagged_before
        )
        .fetch_optional(&mut *tx)
        .await
        .infra_err()?;
        let Some(untagged) = untagged else {
            return Ok(false);
        };
        let referenced = sqlx::query!(
            "SELECT (EXISTS (SELECT 1 FROM docker_tags t WHERE t.manifest_id = $1) \
                 OR EXISTS ( \
                     SELECT 1 FROM docker_manifest_list_members lm JOIN docker_manifests l ON l.id = lm.list_manifest_id \
                     WHERE l.package_repository_id = $2 AND l.image_name = $3 AND lm.member_digest = $4 \
                 )) AS \"referenced!\"",
            untagged.id,
            repository_id,
            image_name.as_str(),
            digest.as_str(),
        )
        .fetch_one(&mut *tx)
        .await
        .infra_err()?;
        if referenced.referenced {
            return Ok(false);
        }
        release_blob_references(&mut tx, untagged.id).await?;
        sqlx::query!("DELETE FROM docker_manifests WHERE id = $1", untagged.id).execute(&mut *tx).await.infra_err()?;
        tx.commit().await.infra_err()?;
        Ok(true)
    }

    async fn blob_is_reachable(&self, repository_id: Uuid, digest: &Digest) -> Result<bool, DomainError> {
        let row = sqlx::query!(
            "SELECT EXISTS( \
                 SELECT 1 FROM docker_manifest_blobs dmb \
                 JOIN docker_manifests dm ON dm.id = dmb.manifest_id \
                 WHERE dm.package_repository_id = $1 AND dmb.blob_digest = $2 \
             ) AS \"exists!\"",
            repository_id,
            digest.as_str()
        )
        .fetch_one(&self.pool)
        .await
        .infra_err()?;
        Ok(row.exists)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::docker_registry::DockerMediaType;

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

    // Postgres enforces the FK to docker_blobs, unlike the in-memory fake.
    async fn seed_blob(pool: &sqlx::PgPool, digest: &Digest) {
        sqlx::query!(
            "INSERT INTO docker_blobs (digest, size_bytes, storage_key, reference_count, created_at) \
             VALUES ($1, 0, $1, 0, now())",
            digest.as_str(),
        )
        .execute(pool)
        .await
        .unwrap();
    }

    fn sample_manifest(repository_id: Uuid, image_name: &DockerImageName) -> DockerManifest {
        DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            image_name: image_name.clone(),
            digest: Digest::of(b"manifest-bytes"),
            media_type: DockerMediaType::DockerV2Manifest,
            body: br#"{"schemaVersion": 2}"#.to_vec(),
            created_at: chrono::Utc::now(),
        }
    }

    #[sqlx::test]
    async fn inserts_a_manifest_with_its_blobs_and_finds_it_by_digest(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = sample_manifest(repository_id, &image_name);
        let blob_digest = Digest::of(b"layer-bytes");
        seed_blob(&pool, &blob_digest).await;
        repo.insert_manifest(&manifest, &[blob_digest.clone()]).await.unwrap();

        let found = repo.find_manifest_by_digest(repository_id, &image_name, &manifest.digest).await.unwrap().unwrap();
        assert_eq!(found.id, manifest.id);
        assert_eq!(found.media_type, DockerMediaType::DockerV2Manifest);
        assert_eq!(found.body, manifest.body);

        let blob_digests = repo.list_manifest_blob_digests(manifest.id).await.unwrap();
        assert_eq!(blob_digests, vec![blob_digest]);
    }

    /// Makes `DELETE FROM docker_manifests` fail, after the statements that came before it in the transaction have run.
    async fn make_manifest_deletes_fail(pool: &sqlx::PgPool) {
        sqlx::query("CREATE FUNCTION refuse_manifest_delete() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql").execute(pool).await.unwrap();
        sqlx::query("CREATE TRIGGER refuse_manifest_delete BEFORE DELETE ON docker_manifests FOR EACH ROW EXECUTE FUNCTION refuse_manifest_delete()").execute(pool).await.unwrap();
    }

    async fn allow_manifest_deletes(pool: &sqlx::PgPool) {
        sqlx::query("DROP TRIGGER refuse_manifest_delete ON docker_manifests").execute(pool).await.unwrap();
    }

    async fn reference_count(pool: &sqlx::PgPool, digest: &Digest) -> i64 {
        sqlx::query_scalar!("SELECT reference_count FROM docker_blobs WHERE digest = $1", digest.as_str()).fetch_one(pool).await.unwrap()
    }

    /// A manifest holding one reference to a blob, as `insert_manifest_with_checks` leaves it.
    async fn seed_referenced_manifest(pool: &sqlx::PgPool, repo: &PostgresDockerManifestRepository, repository_id: Uuid, name: &DockerImageName, layer: &Digest, age_days: i64) -> DockerManifest {
        seed_blob(pool, layer).await;
        let mut manifest = sample_manifest(repository_id, name);
        manifest.created_at = chrono::Utc::now() - chrono::Duration::days(age_days);
        repo.insert_manifest(&manifest, &[layer.clone()]).await.unwrap();
        sqlx::query!("UPDATE docker_blobs SET reference_count = 1 WHERE digest = $1", layer.as_str()).execute(pool).await.unwrap();
        manifest
    }

    #[sqlx::test]
    async fn deleting_a_manifest_gives_up_its_blob_references_in_the_same_transaction(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let name = DockerImageName::parse("app").unwrap();
        let layer = Digest::of(b"layer");
        let manifest = seed_referenced_manifest(&pool, &repo, repository_id, &name, &layer, 0).await;

        make_manifest_deletes_fail(&pool).await;
        assert!(repo.delete_manifest(repository_id, &name, &manifest.digest).await.is_err());
        assert_eq!(reference_count(&pool, &layer).await, 1, "a delete that failed must not have released anything");
        assert!(repo.find_manifest_by_digest(repository_id, &name, &manifest.digest).await.unwrap().is_some());

        allow_manifest_deletes(&pool).await;
        repo.delete_manifest(repository_id, &name, &manifest.digest).await.unwrap();
        assert_eq!(reference_count(&pool, &layer).await, 0);
    }

    #[sqlx::test]
    async fn deleting_an_untagged_manifest_gives_up_its_blob_references_in_the_same_transaction(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let name = DockerImageName::parse("app").unwrap();
        let layer = Digest::of(b"layer");
        let manifest = seed_referenced_manifest(&pool, &repo, repository_id, &name, &layer, 30).await;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(7);

        make_manifest_deletes_fail(&pool).await;
        assert!(repo.delete_untagged_manifest(repository_id, &name, &manifest.digest, cutoff).await.is_err());
        assert_eq!(reference_count(&pool, &layer).await, 1, "a delete that failed must not have released anything");

        allow_manifest_deletes(&pool).await;
        assert!(repo.delete_untagged_manifest(repository_id, &name, &manifest.digest, cutoff).await.unwrap());
        assert_eq!(reference_count(&pool, &layer).await, 0);
    }

    #[sqlx::test]
    async fn re_inserting_the_same_digest_is_a_no_op_not_an_error(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = sample_manifest(repository_id, &image_name);
        let blob_digest = Digest::of(b"layer-bytes");
        seed_blob(&pool, &blob_digest).await;
        let (first_id, first_inserted) = repo.insert_manifest(&manifest, &[blob_digest.clone()]).await.unwrap();
        assert_eq!(first_id, manifest.id);
        assert!(first_inserted);

        // A second insert of the identical (repository, image, digest) must return the pre-existing row's id, not the caller's own unpersisted one.
        let mut second_attempt = sample_manifest(repository_id, &image_name);
        second_attempt.id = Uuid::new_v4();
        assert_ne!(second_attempt.id, manifest.id);
        let (second_id, second_inserted) = repo.insert_manifest(&second_attempt, &[blob_digest.clone()]).await.unwrap();
        assert_eq!(second_id, manifest.id, "insert_manifest must return the pre-existing row's id on conflict, not the caller's own unpersisted id");
        assert!(!second_inserted, "a conflict must report false, not true — callers rely on this to avoid double-counting blob refs");

        assert_eq!(repo.list_manifest_blob_digests(manifest.id).await.unwrap(), vec![blob_digest]);
    }

    #[sqlx::test]
    async fn setting_a_tag_makes_the_manifest_findable_by_tag(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = sample_manifest(repository_id, &image_name);
        repo.insert_manifest(&manifest, &[]).await.unwrap();

        repo.set_tag(repository_id, &image_name, "latest", manifest.id).await.unwrap();

        let found = repo.find_manifest_by_tag(repository_id, &image_name, "latest").await.unwrap().unwrap();
        assert_eq!(found.id, manifest.id);
        assert!(repo.find_manifest_by_tag(repository_id, &image_name, "missing").await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn re_tagging_moves_the_tag_to_the_new_manifest(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let first = sample_manifest(repository_id, &image_name);
        repo.insert_manifest(&first, &[]).await.unwrap();
        repo.set_tag(repository_id, &image_name, "latest", first.id).await.unwrap();
        let mut second = sample_manifest(repository_id, &image_name);
        second.digest = Digest::of(b"a-different-manifest-body");
        repo.insert_manifest(&second, &[]).await.unwrap();

        repo.set_tag(repository_id, &image_name, "latest", second.id).await.unwrap();

        let found = repo.find_manifest_by_tag(repository_id, &image_name, "latest").await.unwrap().unwrap();
        assert_eq!(found.id, second.id);
    }

    #[sqlx::test]
    async fn inserts_and_lists_manifest_list_members(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let mut list_manifest = sample_manifest(repository_id, &image_name);
        list_manifest.media_type = DockerMediaType::OciIndex;
        repo.insert_manifest(&list_manifest, &[]).await.unwrap();
        let member_a = Digest::of(b"amd64-manifest");
        let member_b = Digest::of(b"arm64-manifest");

        repo.insert_manifest_list_members(list_manifest.id, &[member_a.clone(), member_b.clone()]).await.unwrap();

        let mut members = repo.list_manifest_list_member_digests(list_manifest.id).await.unwrap();
        members.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let mut expected = vec![member_a, member_b];
        expected.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        assert_eq!(members, expected);
    }

    #[sqlx::test]
    async fn retrying_insert_manifest_list_members_after_a_partial_success_does_not_fail(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let mut list_manifest = sample_manifest(repository_id, &image_name);
        list_manifest.media_type = DockerMediaType::OciIndex;
        repo.insert_manifest(&list_manifest, &[]).await.unwrap();
        let member_a = Digest::of(b"amd64-manifest");
        let member_b = Digest::of(b"arm64-manifest");

        // Simulates a client retry after a member was already recorded on a prior attempt.
        repo.insert_manifest_list_members(list_manifest.id, &[member_a.clone()]).await.unwrap();
        repo.insert_manifest_list_members(list_manifest.id, &[member_a.clone(), member_b.clone()]).await.unwrap();

        let mut members = repo.list_manifest_list_member_digests(list_manifest.id).await.unwrap();
        members.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let mut expected = vec![member_a, member_b];
        expected.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        assert_eq!(members, expected);
    }

    #[sqlx::test]
    async fn deleting_a_manifest_removes_it_and_its_tag(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = sample_manifest(repository_id, &image_name);
        repo.insert_manifest(&manifest, &[]).await.unwrap();
        repo.set_tag(repository_id, &image_name, "latest", manifest.id).await.unwrap();

        repo.delete_manifest(repository_id, &image_name, &manifest.digest).await.unwrap();

        assert!(repo.find_manifest_by_digest(repository_id, &image_name, &manifest.digest).await.unwrap().is_none());
        assert!(repo.find_manifest_by_tag(repository_id, &image_name, "latest").await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn lists_sorted_tags_for_one_image_and_deduplicated_catalog_names(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = sample_manifest(repository_id, &image_name);
        repo.insert_manifest(&manifest, &[]).await.unwrap();
        repo.set_tag(repository_id, &image_name, "v2", manifest.id).await.unwrap();
        repo.set_tag(repository_id, &image_name, "v1", manifest.id).await.unwrap();

        assert_eq!(repo.list_tags(repository_id, &image_name).await.unwrap(), vec!["v1".to_string(), "v2".to_string()]);
        assert_eq!(repo.list_repository_image_names(repository_id).await.unwrap(), vec![image_name]);
    }

    #[sqlx::test]
    async fn list_recent_tags_for_images_batches_the_tags_of_every_image_in_one_query(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let app = DockerImageName::parse("app").unwrap();
        let worker = DockerImageName::parse("worker").unwrap();
        let app_manifest = sample_manifest(repository_id, &app);
        let worker_manifest = sample_manifest(repository_id, &worker);
        repo.insert_manifest(&app_manifest, &[]).await.unwrap();
        repo.insert_manifest(&worker_manifest, &[]).await.unwrap();
        repo.set_tag(repository_id, &app, "latest", app_manifest.id).await.unwrap();
        repo.set_tag(repository_id, &app, "v1", app_manifest.id).await.unwrap();
        repo.set_tag(repository_id, &worker, "latest", worker_manifest.id).await.unwrap();

        let pairs = repo.list_recent_tags_for_images(repository_id, &["app".to_string(), "worker".to_string()], 10).await.unwrap();

        assert_eq!(
            pairs,
            vec![(app.clone(), "v1".to_string()), (app, "latest".to_string()), (worker, "latest".to_string())]
        );
    }

    #[sqlx::test]
    async fn recent_tags_are_cut_per_image_in_the_query(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let busy = DockerImageName::parse("busy").unwrap();
        let quiet = DockerImageName::parse("quiet").unwrap();
        let manifest = sample_manifest(repository_id, &busy);
        repo.insert_manifest(&manifest, &[]).await.unwrap();
        for i in 0..300 {
            repo.set_tag(repository_id, &busy, &format!("t{i:03}"), manifest.id).await.unwrap();
        }
        repo.set_tag(repository_id, &quiet, "latest", manifest.id).await.unwrap();

        let pairs = repo.list_recent_tags_for_images(repository_id, &["busy".to_string(), "quiet".to_string()], 101).await.unwrap();

        let busy_tags: Vec<&str> = pairs.iter().filter(|(name, _)| *name == busy).map(|(_, tag)| tag.as_str()).collect();
        assert_eq!(busy_tags.len(), 101, "at most the cap, not the 300 stored");
        assert!(busy_tags.contains(&"t299") && !busy_tags.contains(&"t000"), "the most recently updated tags are the ones kept");
        assert_eq!(pairs.iter().filter(|(name, _)| *name == quiet).count(), 1);
    }

    #[sqlx::test]
    async fn image_name_pages_follow_byte_order_whatever_the_column_collation(pool: sqlx::PgPool) {
        sqlx::query("ALTER TABLE docker_tags ALTER COLUMN image_name TYPE text COLLATE \"und-x-icu\"").execute(&pool).await.unwrap();
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let mut expected = ["ab", "a-c", "a_c", "a.c", "a-b", "a/b", "foo-bar", "foobar", "foo-baz", "z", "b2"].map(String::from).to_vec();
        for name in &expected {
            let image = DockerImageName::parse(name).unwrap();
            let manifest = sample_manifest(repository_id, &image);
            repo.insert_manifest(&manifest, &[]).await.unwrap();
            repo.set_tag(repository_id, &image, "latest", manifest.id).await.unwrap();
            repo.set_tag(repository_id, &image, "v1", manifest.id).await.unwrap();
        }
        expected.sort();

        let mut seen = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page = repo.list_image_names_page(repository_id, after.as_deref(), 3).await.unwrap();
            let Some(last) = page.last() else { break };
            after = Some(last.as_str().to_string());
            seen.extend(page.into_iter().map(|n| n.as_str().to_string()));
        }

        assert_eq!(seen, expected, "every image once, in byte order, across pages");
    }

    #[sqlx::test]
    async fn image_names_come_in_pages_by_name(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        for name in ["c", "a", "b"] {
            let image = DockerImageName::parse(name).unwrap();
            let manifest = sample_manifest(repository_id, &image);
            repo.insert_manifest(&manifest, &[]).await.unwrap();
            repo.set_tag(repository_id, &image, "latest", manifest.id).await.unwrap();
            repo.set_tag(repository_id, &image, "v1", manifest.id).await.unwrap();
        }
        let names = |page: Vec<DockerImageName>| page.into_iter().map(|n| n.as_str().to_string()).collect::<Vec<_>>();

        assert_eq!(names(repo.list_image_names_page(repository_id, None, 2).await.unwrap()), vec!["a", "b"], "each image once, whatever its tags");
        assert_eq!(names(repo.list_image_names_page(repository_id, Some("b"), 2).await.unwrap()), vec!["c"]);
    }

    #[sqlx::test]
    async fn a_details_page_reads_the_newest_tags_and_only_the_small_bodies_asked_for(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image = DockerImageName::parse("app").unwrap();
        let mut digests = Vec::new();
        for (i, body) in [b"{\"a\":1}".to_vec(), b"{\"b\":2}".to_vec(), format!("{{\"pad\":\"{}\"}}", "x".repeat(1000)).into_bytes()].into_iter().enumerate() {
            let mut manifest = sample_manifest(repository_id, &image);
            manifest.digest = Digest::of(&body);
            manifest.body = body;
            repo.insert_manifest(&manifest, &[]).await.unwrap();
            repo.set_tag(repository_id, &image, &format!("t{i}"), manifest.id).await.unwrap();
            sqlx::query!("UPDATE docker_tags SET updated_at = now() - make_interval(hours => $2) WHERE package_repository_id = $1 AND tag = $3", repository_id, (10 - i) as i32, format!("t{i}")).execute(&pool).await.unwrap();
            digests.push(manifest.digest.as_str().to_string());
        }

        let newest = repo.list_tag_manifest_summaries(repository_id, &image, 2).await.unwrap();
        assert_eq!(newest.iter().map(|(tag, ..)| tag.as_str()).collect::<Vec<_>>(), vec!["t2", "t1"]);

        let bodies = repo.list_tagged_manifest_bodies(repository_id, &image, &digests[1..], 100).await.unwrap();
        assert_eq!(bodies.len(), 1, "the first digest was not asked for and the last is over the size limit");
        assert_eq!(bodies[0].0.as_str(), digests[1]);
    }

    #[sqlx::test]
    async fn list_latest_manifest_id_per_image_batches_across_every_image_in_the_repository(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let app = DockerImageName::parse("app").unwrap();
        let worker = DockerImageName::parse("worker").unwrap();
        let app_manifest = sample_manifest(repository_id, &app);
        let worker_manifest = sample_manifest(repository_id, &worker);
        repo.insert_manifest(&app_manifest, &[]).await.unwrap();
        repo.insert_manifest(&worker_manifest, &[]).await.unwrap();
        repo.set_tag(repository_id, &app, "latest", app_manifest.id).await.unwrap();
        repo.set_tag(repository_id, &worker, "latest", worker_manifest.id).await.unwrap();

        let mut latest = repo.list_latest_manifest_id_per_image(repository_id, &["app".to_string(), "worker".to_string()]).await.unwrap();
        latest.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));

        assert_eq!(latest, vec![(app, app_manifest.id), (worker, worker_manifest.id)]);
    }

    #[sqlx::test]
    async fn list_latest_manifest_id_per_image_follows_the_most_recently_updated_tag(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("app").unwrap();
        let old_manifest = sample_manifest(repository_id, &image_name);
        let mut new_manifest = sample_manifest(repository_id, &image_name);
        new_manifest.digest = Digest::of(b"a different manifest body");
        repo.insert_manifest(&old_manifest, &[]).await.unwrap();
        repo.insert_manifest(&new_manifest, &[]).await.unwrap();
        repo.set_tag(repository_id, &image_name, "1.0.0", old_manifest.id).await.unwrap();
        repo.set_tag(repository_id, &image_name, "latest", new_manifest.id).await.unwrap();
        // Force a deterministic ordering rather than relying on two `now()` calls landing microseconds apart.
        sqlx::query!(
            "UPDATE docker_tags SET updated_at = now() - interval '1 hour' WHERE package_repository_id = $1 AND tag = '1.0.0'",
            repository_id
        )
        .execute(&pool)
        .await
        .unwrap();

        let latest = repo.list_latest_manifest_id_per_image(repository_id, &["app".to_string()]).await.unwrap();

        assert_eq!(latest, vec![(image_name, new_manifest.id)]);
    }

    async fn seed_sized_blob(pool: &sqlx::PgPool, digest: &Digest, size: i64) {
        sqlx::query!("INSERT INTO docker_blobs (digest, size_bytes, storage_key, reference_count, created_at) VALUES ($1, $2, $1, 0, now())", digest.as_str(), size).execute(pool).await.unwrap();
    }

    async fn link(pool: &sqlx::PgPool, repository_id: Uuid, digest: &Digest) {
        sqlx::query!("INSERT INTO docker_repository_blobs (package_repository_id, blob_digest) VALUES ($1, $2)", repository_id, digest.as_str()).execute(pool).await.unwrap();
    }

    #[sqlx::test]
    async fn a_manifest_that_lists_a_layer_several_times_is_stored_with_one_row_and_one_reference(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let layer = Digest::of(b"shared-layer");
        let config = Digest::of(b"config");
        seed_blob(&pool, &layer).await;
        seed_blob(&pool, &config).await;
        link(&pool, repository_id, &layer).await;
        link(&pool, repository_id, &config).await;
        let manifest = sample_manifest(repository_id, &image_name);

        let (id, inserted) = repo.insert_manifest_with_checks(repository_id, &manifest, &[config.clone(), layer.clone(), layer.clone(), layer.clone()], None).await.unwrap();

        assert!(inserted);
        let mut stored = repo.list_manifest_blob_digests(id).await.unwrap();
        stored.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let mut expected = vec![config, layer.clone()];
        expected.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        assert_eq!(stored, expected);
        let references: i64 = sqlx::query_scalar!("SELECT reference_count FROM docker_blobs WHERE digest = $1", layer.as_str()).fetch_one(&pool).await.unwrap();
        assert_eq!(references, 1);
    }

    #[sqlx::test]
    async fn one_unreachable_blob_among_many_rejects_the_whole_manifest(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        let other_repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        seed_repository(&pool, other_repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let mine = Digest::of(b"mine");
        let theirs = Digest::of(b"theirs");
        seed_blob(&pool, &mine).await;
        seed_blob(&pool, &theirs).await;
        link(&pool, repository_id, &mine).await;
        link(&pool, other_repository_id, &theirs).await;

        let result = repo.insert_manifest_with_checks(repository_id, &sample_manifest(repository_id, &image_name), &[mine, theirs.clone()], None).await;

        assert_eq!(result, Err(DomainError::DockerBlobNotReachable(theirs.as_str().to_string())));
    }

    /// The quota counts each distinct blob once, however many manifests and repeated layers name it (and the manifest bodies on top).
    #[sqlx::test]
    async fn the_manifest_quota_counts_each_blob_once(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let layer = Digest::of(b"layer");
        seed_sized_blob(&pool, &layer, 40).await;
        link(&pool, repository_id, &layer).await;
        let first = sample_manifest(repository_id, &image_name);

        // Three listings of a 40-byte layer are 40 bytes, not 120 (which would not fit in 100).
        repo.insert_manifest_with_checks(repository_id, &first, &[layer.clone(), layer.clone(), layer.clone()], Some(100)).await.unwrap();
        // A second manifest over the same layer adds only its own 20-byte body.
        let mut second = sample_manifest(repository_id, &image_name);
        second.id = Uuid::new_v4();
        second.digest = Digest::of(b"another manifest");
        repo.insert_manifest_with_checks(repository_id, &second, &[layer], Some(100)).await.unwrap();
    }

    #[sqlx::test]
    async fn manifest_bodies_and_tags_count_against_the_quota_even_without_blobs(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let mut padded = sample_manifest(repository_id, &image_name);
        padded.body = vec![b' '; 3000];
        padded.digest = Digest::of(&padded.body);

        // A 3000-byte body, no blobs at all.
        assert_eq!(repo.insert_manifest_with_checks(repository_id, &padded, &[], Some(2999)).await.unwrap_err(), DomainError::StorageQuotaExceeded);
        let (id, _) = repo.insert_manifest_with_checks(repository_id, &padded, &[], Some(3000)).await.unwrap();
        // Re-pushing what is already stored adds nothing, however tight the quota.
        repo.insert_manifest_with_checks(repository_id, &padded, &[], Some(3000)).await.unwrap();
        // Each tag costs a fixed amount on top.
        repo.set_tag(repository_id, &image_name, "v1", id).await.unwrap();
        let mut second = sample_manifest(repository_id, &image_name);
        second.id = Uuid::new_v4();
        second.digest = Digest::of(b"second");
        assert_eq!(repo.insert_manifest_with_checks(repository_id, &second, &[], Some(3000 + DOCKER_TAG_QUOTA_BYTES + 19)).await.unwrap_err(), DomainError::StorageQuotaExceeded);
        repo.insert_manifest_with_checks(repository_id, &second, &[], Some(3000 + DOCKER_TAG_QUOTA_BYTES + 20)).await.unwrap();
    }

    #[sqlx::test]
    async fn a_repository_holds_no_more_tags_than_the_cap_but_existing_ones_can_still_move(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = manifest_aged(&repo, repository_id, &image_name, b"tagged a lot", 1).await;
        sqlx::query!(
            "INSERT INTO docker_tags (package_repository_id, image_name, tag, manifest_id) SELECT $1, 'myimage', 't' || n, $2 FROM generate_series(1, $3::int) n",
            repository_id,
            manifest.id,
            MAX_TAGS_PER_REPOSITORY as i32,
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(repo.set_tag(repository_id, &image_name, "one-too-many", manifest.id).await.unwrap_err(), DomainError::TooManyTags);
        repo.set_tag(repository_id, &image_name, "t1", manifest.id).await.expect("moving a tag that exists adds none");
    }

    #[sqlx::test]
    async fn tag_updates_carry_the_time_the_tag_itself_was_set(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let old_release = sample_manifest(repository_id, &image_name);
        let mut backdated = old_release.clone();
        backdated.created_at = chrono::Utc::now() - chrono::Duration::days(180);
        repo.insert_manifest(&backdated, &[]).await.unwrap();

        repo.set_tag(repository_id, &image_name, "prod", backdated.id).await.unwrap();

        let updates = repo.list_repository_tag_updates(repository_id).await.unwrap();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].3 > chrono::Utc::now() - chrono::Duration::minutes(1), "the tag was set just now, whatever the manifest's own age");
        let summaries = repo.list_repository_tag_manifest_summaries(repository_id).await.unwrap();
        assert!(summaries[0].4 < chrono::Utc::now() - chrono::Duration::days(100), "while the manifest itself is old");
    }

    async fn manifest_aged(repo: &PostgresDockerManifestRepository, repository_id: Uuid, image_name: &DockerImageName, content: &[u8], days: i64) -> DockerManifest {
        let mut manifest = sample_manifest(repository_id, image_name);
        manifest.digest = Digest::of(content);
        manifest.created_at = chrono::Utc::now() - chrono::Duration::days(days);
        repo.insert_manifest(&manifest, &[]).await.unwrap();
        manifest
    }

    #[sqlx::test]
    async fn untagged_manifests_are_listed_only_when_old_untagged_and_not_a_list_member(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let other_image = DockerImageName::parse("otherimage").unwrap();
        let orphan = manifest_aged(&repo, repository_id, &image_name, b"orphan", 30).await;
        let tagged = manifest_aged(&repo, repository_id, &image_name, b"tagged", 30).await;
        repo.set_tag(repository_id, &image_name, "stable", tagged.id).await.unwrap();
        let young = manifest_aged(&repo, repository_id, &image_name, b"young", 1).await;
        let member = manifest_aged(&repo, repository_id, &image_name, b"member", 30).await;
        let index = manifest_aged(&repo, repository_id, &image_name, b"index", 30).await;
        repo.set_tag(repository_id, &image_name, "multi", index.id).await.unwrap();
        repo.insert_manifest_list_members(index.id, &[member.digest.clone()]).await.unwrap();
        // The same digest under another image name is not covered by that image's list.
        let same_digest_elsewhere = {
            let mut m = sample_manifest(repository_id, &other_image);
            m.digest = member.digest.clone();
            m.created_at = chrono::Utc::now() - chrono::Duration::days(30);
            repo.insert_manifest(&m, &[]).await.unwrap();
            m
        };
        let listed = repo.list_untagged_manifests(repository_id, chrono::Utc::now() - chrono::Duration::days(7)).await.unwrap();

        let mut digests: Vec<(String, String)> = listed.into_iter().map(|(name, digest)| (name.as_str().to_string(), digest.as_str().to_string())).collect();
        digests.sort();
        let mut expected = vec![
            ("myimage".to_string(), orphan.digest.as_str().to_string()),
            ("otherimage".to_string(), same_digest_elsewhere.digest.as_str().to_string()),
        ];
        expected.sort();
        assert_eq!(digests, expected);
        assert!(!digests.iter().any(|(_, d)| d == tagged.digest.as_str() || d == young.digest.as_str()));
    }

    /// A digest-pinned rollback target must not be swept the moment its tag moves on, however old the manifest is.
    #[sqlx::test]
    async fn the_grace_period_of_a_manifest_a_tag_moved_away_from_runs_from_that_moment(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let old_release = manifest_aged(&repo, repository_id, &image_name, b"old-release", 90).await;
        let new_release = manifest_aged(&repo, repository_id, &image_name, b"new-release", 1).await;
        repo.set_tag(repository_id, &image_name, "stable", old_release.id).await.unwrap();
        let cutoff = chrono::Utc::now() - chrono::Duration::days(7);

        repo.set_tag(repository_id, &image_name, "stable", new_release.id).await.unwrap();

        assert!(repo.list_untagged_manifests(repository_id, cutoff).await.unwrap().is_empty(), "untagged a moment ago, so still inside its grace period");
        assert!(!repo.delete_untagged_manifest(repository_id, &image_name, &old_release.digest, cutoff).await.unwrap());

        sqlx::query!("UPDATE docker_manifests SET untagged_since = now() - interval '8 days' WHERE id = $1", old_release.id).execute(&pool).await.unwrap();
        assert_eq!(repo.list_untagged_manifests(repository_id, cutoff).await.unwrap().len(), 1, "a week later it is reclaimable");

        repo.set_tag(repository_id, &image_name, "stable", old_release.id).await.unwrap();
        assert!(repo.list_untagged_manifests(repository_id, cutoff).await.unwrap().is_empty(), "tagged again");
        let cleared = sqlx::query_scalar!("SELECT untagged_since FROM docker_manifests WHERE id = $1", old_release.id).fetch_one(&pool).await.unwrap();
        assert!(cleared.is_none());
    }

    #[sqlx::test]
    async fn a_manifest_that_keeps_another_tag_is_not_marked_untagged_when_one_tag_moves(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let shared = manifest_aged(&repo, repository_id, &image_name, b"shared", 90).await;
        let other = manifest_aged(&repo, repository_id, &image_name, b"other", 1).await;
        repo.set_tag(repository_id, &image_name, "stable", shared.id).await.unwrap();
        repo.set_tag(repository_id, &image_name, "v1", shared.id).await.unwrap();

        repo.set_tag(repository_id, &image_name, "stable", other.id).await.unwrap();

        let marked = sqlx::query_scalar!("SELECT untagged_since FROM docker_manifests WHERE id = $1", shared.id).fetch_one(&pool).await.unwrap();
        assert!(marked.is_none(), "still tagged as v1");
    }

    #[sqlx::test]
    async fn deleting_an_untagged_manifest_rechecks_that_it_is_still_untagged(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = manifest_aged(&repo, repository_id, &image_name, b"revived", 30).await;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(7);

        // Somebody tags the digest between the sweep's listing and its delete.
        repo.set_tag(repository_id, &image_name, "revived", manifest.id).await.unwrap();
        assert!(!repo.delete_untagged_manifest(repository_id, &image_name, &manifest.digest, cutoff).await.unwrap());
        assert!(repo.find_manifest_by_digest(repository_id, &image_name, &manifest.digest).await.unwrap().is_some());

        sqlx::query!("DELETE FROM docker_tags WHERE manifest_id = $1", manifest.id).execute(&pool).await.unwrap();
        assert!(repo.delete_untagged_manifest(repository_id, &image_name, &manifest.digest, cutoff).await.unwrap());
        assert!(repo.find_manifest_by_digest(repository_id, &image_name, &manifest.digest).await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn a_reclaim_waiting_on_a_push_that_is_tagging_the_manifest_leaves_it_and_its_tag_alone(pool: sqlx::PgPool) {
        let repo = std::sync::Arc::new(PostgresDockerManifestRepository::new(pool.clone()));
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = manifest_aged(&repo, repository_id, &image_name, b"revived", 30).await;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(7);

        // What `set_tag` holds until it commits: the manifest row and the new tag.
        let mut tagging = pool.begin().await.unwrap();
        sqlx::query!("SELECT id FROM docker_manifests WHERE id = $1 FOR KEY SHARE", manifest.id).fetch_one(&mut *tagging).await.unwrap();
        sqlx::query!("INSERT INTO docker_tags (package_repository_id, image_name, tag, manifest_id) VALUES ($1, 'myimage', 'revived', $2)", repository_id, manifest.id).execute(&mut *tagging).await.unwrap();
        let reclaim = {
            let repo = repo.clone();
            let image_name = image_name.clone();
            let digest = manifest.digest.clone();
            tokio::spawn(async move { repo.delete_untagged_manifest(repository_id, &image_name, &digest, cutoff).await })
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
        assert!(queued, "the reclaim never waited on the manifest row");
        tagging.commit().await.unwrap();

        assert!(!reclaim.await.unwrap().unwrap(), "the manifest was tagged while the reclaim waited");
        assert!(repo.find_manifest_by_digest(repository_id, &image_name, &manifest.digest).await.unwrap().is_some());
        assert_eq!(repo.find_manifest_by_tag(repository_id, &image_name, "revived").await.unwrap().map(|found| found.id), Some(manifest.id));
    }

    /// Migrates to 0009, loads manifests the way an installation from before 0010 has them, then applies 0010 the way an upgrade would.
    #[sqlx::test(migrations = false)]
    async fn migration_0010_starts_the_grace_period_of_manifests_that_are_untagged_at_upgrade(pool: sqlx::PgPool) {
        let mut before = sqlx::migrate!("./migrations");
        before.migrations = before.migrations.iter().filter(|m| m.version < 10).cloned().collect::<Vec<_>>().into();
        before.run(&pool).await.unwrap();

        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let (tagged, untagged, never_tagged) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        for (id, content) in [(tagged, "tagged"), (untagged, "untagged"), (never_tagged, "never")] {
            sqlx::query("INSERT INTO docker_manifests (id, package_repository_id, image_name, digest, media_type, body, created_at) VALUES ($1, $2, 'myimage', $3, 'application/vnd.docker.distribution.manifest.v2+json', '\x7b7d', now() - interval '90 days')")
                .bind(id)
                .bind(repository_id)
                .bind(Digest::of(content.as_bytes()).as_str())
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO docker_tags (package_repository_id, image_name, tag, manifest_id) VALUES ($1, 'myimage', 'latest', $2)").bind(repository_id).bind(tagged).execute(&pool).await.unwrap();

        sqlx::raw_sql(include_str!("../../migrations/0010_docker_manifest_untagged_since.sql")).execute(&pool).await.unwrap();

        let untagged_since = |id: Uuid| {
            let pool = pool.clone();
            async move { sqlx::query_scalar::<_, Option<DateTime<Utc>>>("SELECT untagged_since FROM docker_manifests WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap() }
        };
        assert!(untagged_since(tagged).await.is_none(), "a tagged manifest is not untagged");
        let started = untagged_since(untagged).await.expect("the grace period starts at the upgrade");
        assert!(chrono::Utc::now() - started < chrono::Duration::minutes(1));
        assert!(untagged_since(never_tagged).await.is_some(), "a manifest that was never tagged gets the same grace");
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let cutoff = chrono::Utc::now() - chrono::Duration::days(7);
        assert!(repo.list_untagged_manifests(repository_id, cutoff).await.unwrap().is_empty(), "nothing is reclaimable at the first sweep after the upgrade");

        // A second run keeps the dates.
        sqlx::raw_sql(include_str!("../../migrations/0010_docker_manifest_untagged_since.sql")).execute(&pool).await.unwrap();
        assert_eq!(untagged_since(untagged).await, Some(started));
    }

    #[sqlx::test]
    async fn a_tag_row_that_no_longer_parses_does_not_hide_the_others_from_the_retention_listing(pool: sqlx::PgPool) {
        let repo = PostgresDockerManifestRepository::new(pool.clone());
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, repository_id).await;
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest = manifest_aged(&repo, repository_id, &image_name, b"good", 1).await;
        repo.set_tag(repository_id, &image_name, "v1", manifest.id).await.unwrap();
        sqlx::query!("INSERT INTO docker_tags (package_repository_id, image_name, tag, manifest_id) VALUES ($1, 'Not A Valid Name', 'v1', $2)", repository_id, manifest.id).execute(&pool).await.unwrap();

        let updates = repo.list_repository_tag_updates(repository_id).await.unwrap();

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1, "v1");
    }
}
