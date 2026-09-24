use std::sync::Arc;

use artiferris_domain::audit::{DockerRegistryEvent, EventPublisherPort};
use artiferris_domain::docker_registry::{Digest, DockerBlobStorePort, DockerImageName, DockerManifestRepositoryPort};
use uuid::Uuid;

use crate::error::ApplicationError;

pub struct DeleteManifestUseCase {
    manifests: Arc<dyn DockerManifestRepositoryPort>,
    blobs: Arc<dyn DockerBlobStorePort>,
    events: Arc<dyn EventPublisherPort>,
}

impl DeleteManifestUseCase {
    pub fn new(manifests: Arc<dyn DockerManifestRepositoryPort>, blobs: Arc<dyn DockerBlobStorePort>, events: Arc<dyn EventPublisherPort>) -> Self {
        Self { manifests, blobs, events }
    }

    pub async fn execute(&self, repository_id: Uuid, image_name: &DockerImageName, digest: &Digest, actor_id: Uuid) -> Result<(), ApplicationError> {
        let manifest = self
            .manifests
            .find_manifest_by_digest(repository_id, image_name, digest)
            .await?
            .ok_or(ApplicationError::DockerManifestNotFound)?;

        let blob_digests = self.blob_digests_of(&manifest).await?;
        // The manifest row goes first (its join rows cascade), together with its blobs' reference counts.
        self.manifests.delete_manifest(repository_id, image_name, digest).await?;
        self.release_blobs_and_publish(repository_id, image_name, digest, blob_digests, actor_id).await
    }

    /// Deletes the manifest only while it is still untagged, unlisted and has been for longer than `untagged_before`; `Ok(false)` when it stayed.
    pub async fn execute_if_untagged(
        &self,
        repository_id: Uuid,
        image_name: &DockerImageName,
        digest: &Digest,
        untagged_before: chrono::DateTime<chrono::Utc>,
        actor_id: Uuid,
    ) -> Result<bool, ApplicationError> {
        let Some(manifest) = self.manifests.find_manifest_by_digest(repository_id, image_name, digest).await? else {
            return Ok(false);
        };
        let blob_digests = self.blob_digests_of(&manifest).await?;
        if !self.manifests.delete_untagged_manifest(repository_id, image_name, digest, untagged_before).await? {
            return Ok(false);
        }
        self.release_blobs_and_publish(repository_id, image_name, digest, blob_digests, actor_id).await?;
        Ok(true)
    }

    /// Read before the delete: the join rows go with the manifest.
    async fn blob_digests_of(&self, manifest: &artiferris_domain::docker_registry::DockerManifest) -> Result<Vec<Digest>, ApplicationError> {
        if manifest.media_type.is_index() {
            // A manifest list holds no blob references; its members release theirs when deleted.
            return Ok(Vec::new());
        }
        Ok(self.manifests.list_manifest_blob_digests(manifest.id).await?)
    }

    async fn release_blobs_and_publish(
        &self,
        repository_id: Uuid,
        image_name: &DockerImageName,
        digest: &Digest,
        blob_digests: Vec<Digest>,
        actor_id: Uuid,
    ) -> Result<(), ApplicationError> {
        for blob_digest in blob_digests {
            // The manifest's link row (created at upload time) outlives it, and would keep the blob from being reclaimed.
            if !self.manifests.blob_is_reachable(repository_id, &blob_digest).await? {
                self.blobs.unlink_from_repository_if_unreferenced(repository_id, &blob_digest).await?;
            }
            self.blobs.delete_if_unreferenced(&blob_digest).await?;
        }

        self.events
            .publish_docker_event(
                DockerRegistryEvent::ManifestDeleted { image_name: image_name.as_str().to_string(), digest: digest.as_str().to_string() },
                repository_id,
                Some(actor_id),
            )
            .await?;

        Ok(())
    }
}

/// Deletes an entire image (every tag), rather than one tag/digest at a time.
pub struct DeleteDockerImageUseCase {
    manifests: Arc<dyn DockerManifestRepositoryPort>,
    delete_manifest: Arc<DeleteManifestUseCase>,
}

impl DeleteDockerImageUseCase {
    pub fn new(manifests: Arc<dyn DockerManifestRepositoryPort>, delete_manifest: Arc<DeleteManifestUseCase>) -> Self {
        Self { manifests, delete_manifest }
    }

    pub async fn execute(&self, repository_id: Uuid, image_name: &DockerImageName, actor_id: Uuid) -> Result<(), ApplicationError> {
        let digests = self.manifests.list_distinct_digests_for_image(repository_id, image_name).await?;
        for digest in digests {
            self.delete_manifest.execute(repository_id, image_name, &digest, actor_id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::docker_test_support::{FakeDockerBlobStore, FakeDockerEvents, FakeDockerManifestRepository};
    use artiferris_domain::docker_registry::{DockerImageName, DockerManifest, DockerMediaType};

    async fn seed_manifest_with_one_blob(manifests: &FakeDockerManifestRepository, blobs: &Arc<FakeDockerBlobStore>, repository_id: Uuid, name: &DockerImageName) -> (Digest, DockerManifest) {
        manifests.link_blob_store(blobs.clone());
        let blob_digest = Digest::of(b"layer-bytes");
        blobs.write(&blob_digest, b"layer-bytes").await.unwrap();
        blobs.increment_ref(&blob_digest).await.unwrap();
        let manifest = DockerManifest {
            id: Uuid::new_v4(), package_repository_id: repository_id, image_name: name.clone(),
            digest: Digest::of(b"manifest-bytes"), media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(), created_at: chrono::Utc::now(),
        };
        manifests.insert_manifest(&manifest, &[blob_digest.clone()]).await.unwrap();
        (blob_digest, manifest)
    }

    #[tokio::test]
    async fn deleting_a_manifest_decrements_and_removes_a_now_unreferenced_blob() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let (blob_digest, manifest) = seed_manifest_with_one_blob(&manifests, &blobs, repository_id, &name).await;

        let use_case = DeleteManifestUseCase::new(manifests.clone(), blobs.clone(), events);
        use_case.execute(repository_id, &name, &manifest.digest, Uuid::new_v4()).await.unwrap();

        assert!(!blobs.exists(&blob_digest).await.unwrap());
        assert!(manifests.find_manifest_by_digest(repository_id, &name, &manifest.digest).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_blob_shared_by_two_manifests_survives_deleting_only_one() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let (blob_digest, manifest_a) = seed_manifest_with_one_blob(&manifests, &blobs, repository_id, &name).await;
        // A second manifest referencing the SAME blob (as real images sharing a base layer would).
        blobs.increment_ref(&blob_digest).await.unwrap();
        let manifest_b = DockerManifest {
            id: Uuid::new_v4(), package_repository_id: repository_id, image_name: name.clone(),
            digest: Digest::of(b"other-manifest-bytes"), media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(), created_at: chrono::Utc::now(),
        };
        manifests.insert_manifest(&manifest_b, &[blob_digest.clone()]).await.unwrap();

        let use_case = DeleteManifestUseCase::new(manifests, blobs.clone(), events);
        use_case.execute(repository_id, &name, &manifest_a.digest, Uuid::new_v4()).await.unwrap();

        assert!(blobs.exists(&blob_digest).await.unwrap(), "blob is still referenced by manifest_b, must survive");
    }

    #[tokio::test]
    async fn deleting_a_whole_image_removes_every_tag_including_ones_sharing_a_digest() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let (_blob_a, manifest_a) = seed_manifest_with_one_blob(&manifests, &blobs, repository_id, &name).await;
        manifests.set_tag(repository_id, &name, "v1", manifest_a.id).await.unwrap();
        manifests.set_tag(repository_id, &name, "stable", manifest_a.id).await.unwrap();

        let other_blob = Digest::of(b"other-layer-bytes");
        blobs.write(&other_blob, b"other-layer-bytes").await.unwrap();
        let manifest_b = DockerManifest {
            id: Uuid::new_v4(), package_repository_id: repository_id, image_name: name.clone(),
            digest: Digest::of(b"other-manifest-bytes"), media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(), created_at: chrono::Utc::now(),
        };
        manifests.insert_manifest(&manifest_b, &[other_blob.clone()]).await.unwrap();
        manifests.set_tag(repository_id, &name, "v2", manifest_b.id).await.unwrap();

        let delete_manifest = Arc::new(DeleteManifestUseCase::new(manifests.clone(), blobs.clone(), events));
        let use_case = DeleteDockerImageUseCase::new(manifests.clone(), delete_manifest);
        use_case.execute(repository_id, &name, Uuid::new_v4()).await.unwrap();

        assert!(manifests.list_tags(repository_id, &name).await.unwrap().is_empty());
        assert!(manifests.find_manifest_by_digest(repository_id, &name, &manifest_a.digest).await.unwrap().is_none());
        assert!(manifests.find_manifest_by_digest(repository_id, &name, &manifest_b.digest).await.unwrap().is_none());
    }

    /// End-to-end regression coverage over a REAL Postgres database, using the actual
    /// `PostgresDockerManifestRepository` and `FilesystemDockerBlobStore` adapters instead of the fakes
    /// above (the fakes don't model `docker_repository_blobs`'s foreign key at all, so they can't
    /// reproduce this bug). Mirrors the real push/delete flow: a monolithic blob upload creates the
    /// blob's `docker_repository_blobs` link row independent of any manifest (exactly like a real
    /// `docker push`'s layer upload, which always lands before the manifest PUT that references it),
    /// then a manifest push references it. Before this fix, deleting that manifest would decrement the
    /// blob to zero real references and then immediately roll that decrement back — forever — the
    /// instant the reclaim saw the leftover, by-then-stale link row.
    mod real_postgres_blob_reclamation {
        use super::*;
        use crate::use_cases::docker_manifest_put::PutManifestUseCase;
        use crate::use_cases::docker_test_support::FakeRepositories;
        use artiferris_infrastructure::filesystem_docker_blob_store::FilesystemDockerBlobStore;
        use artiferris_infrastructure::postgres::docker_manifest_repository::PostgresDockerManifestRepository;

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

        fn repo_summary(id: Uuid) -> artiferris_domain::package_repository::PackageRepositorySummary {
            artiferris_domain::package_repository::PackageRepositorySummary {
                id,
                organization_id: Uuid::new_v4(),
                name: format!("repo-{id}"),
                format: artiferris_domain::package_repository::RepositoryFormat::Docker,
                repo_type: artiferris_domain::package_repository::RepositoryType::Hosted,
                remote_url: None,
                remote_username: None,
                remote_password: None,
                group_members: vec![],
                quota_bytes: None,
                retention_keep_last_n: None,
                is_public: false,
            }
        }

        fn manifest_body(config_digest: &str, salt: &str) -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                "config": { "digest": config_digest, "size": 5 },
                "layers": [],
                // Makes two manifests referencing the same config blob hash to different digests.
                "x-test-salt": salt,
            }))
            .unwrap()
        }

        /// The exact scenario in the bug report: a blob's last real (manifest) reference is removed,
        /// and it must now be genuinely reclaimed — row AND on-disk file gone — not left pinned forever
        /// by its own stale upload-time link row.
        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn deleting_a_manifest_reclaims_a_blob_whose_stale_repository_link_would_otherwise_block_it_forever(pool: sqlx::PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let blobs = Arc::new(FilesystemDockerBlobStore::new(pool.clone(), dir.path()));
            let manifests = Arc::new(PostgresDockerManifestRepository::new(pool.clone()));
            let repositories = Arc::new(FakeRepositories::new());
            let events = Arc::new(FakeDockerEvents::new());
            let repository_id = seed_repository(&pool).await;
            repositories.insert(repo_summary(repository_id));
            let name = DockerImageName::parse("myimage").unwrap();

            let config_bytes = b"cfg-0";
            let config_digest = Digest::of(config_bytes);
            blobs.write(&config_digest, config_bytes).await.unwrap();
            blobs.link_to_repository(repository_id, &config_digest).await.unwrap();

            let put_manifest = PutManifestUseCase::new(manifests.clone(), repositories, events.clone());
            let manifest_digest =
                put_manifest.execute(repository_id, &name, "v1", DockerMediaType::DockerV2Manifest, &manifest_body(config_digest.as_str(), "a"), Uuid::new_v4()).await.unwrap();

            // Sanity: the upload-time link row exists, and the blob is genuinely referenced once.
            assert!(blobs.is_uploaded_to_repository(repository_id, &config_digest).await.unwrap());

            let delete_manifest = DeleteManifestUseCase::new(manifests, blobs.clone(), events);
            delete_manifest.execute(repository_id, &name, &manifest_digest, Uuid::new_v4()).await.unwrap();

            assert!(!blobs.exists(&config_digest).await.unwrap(), "the blob's docker_blobs row must be gone — its only real reference was just deleted");
            assert!(
                !blobs.is_uploaded_to_repository(repository_id, &config_digest).await.unwrap(),
                "the stale docker_repository_blobs link row must be cleaned up along with the blob"
            );
            // Mirrors `FilesystemDockerBlobStore::storage_key`'s two-level sharding (private to that
            // crate, so recomputed here rather than reused) — sha256/<first 2 hex>/<next 2 hex>/<full hex>.
            let hex = config_digest.as_str().strip_prefix("sha256:").unwrap();
            let file_still_present = dir.path().join(format!("sha256/{}/{}/{}", &hex[0..2], &hex[2..4], hex)).exists();
            assert!(!file_still_present, "the on-disk blob file must be removed too, not just the row");
        }

        /// The "don't reintroduce premature deletion" half: a blob still genuinely referenced by
        /// another live manifest in the SAME repository must survive deleting a different manifest that
        /// happens to also (still) reference it.
        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn deleting_one_of_two_manifests_sharing_a_blob_in_the_same_repository_leaves_it_intact(pool: sqlx::PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let blobs = Arc::new(FilesystemDockerBlobStore::new(pool.clone(), dir.path()));
            let manifests = Arc::new(PostgresDockerManifestRepository::new(pool.clone()));
            let repositories = Arc::new(FakeRepositories::new());
            let events = Arc::new(FakeDockerEvents::new());
            let repository_id = seed_repository(&pool).await;
            repositories.insert(repo_summary(repository_id));
            let name = DockerImageName::parse("myimage").unwrap();

            let config_bytes = b"shared-cfg";
            let config_digest = Digest::of(config_bytes);
            blobs.write(&config_digest, config_bytes).await.unwrap();
            blobs.link_to_repository(repository_id, &config_digest).await.unwrap();

            let put_manifest = PutManifestUseCase::new(manifests.clone(), repositories, events.clone());
            let digest_a =
                put_manifest.execute(repository_id, &name, "v1", DockerMediaType::DockerV2Manifest, &manifest_body(config_digest.as_str(), "a"), Uuid::new_v4()).await.unwrap();
            put_manifest.execute(repository_id, &name, "v2", DockerMediaType::DockerV2Manifest, &manifest_body(config_digest.as_str(), "b"), Uuid::new_v4()).await.unwrap();

            let delete_manifest = DeleteManifestUseCase::new(manifests, blobs.clone(), events);
            delete_manifest.execute(repository_id, &name, &digest_a, Uuid::new_v4()).await.unwrap();

            assert!(blobs.exists(&config_digest).await.unwrap(), "still referenced by v2's manifest — must survive");
            assert!(blobs.is_uploaded_to_repository(repository_id, &config_digest).await.unwrap(), "the link row backing a live reference must survive too");
        }

        /// A stronger "don't reintroduce premature deletion" case: the blob is uploaded directly to a
        /// SECOND, unrelated repository (independent of any manifest there — exactly the scenario
        /// `docker_repository_blobs`'s own schema comment and the hard-delete sweep's tests document),
        /// while the first repository's manifest — its only real (manifest) reference anywhere — is
        /// deleted. The fix must decrement the now-zero global reference count but must NOT delete the
        /// row or file, since the second repository's link still legitimately needs it reachable.
        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn deleting_a_manifest_does_not_delete_a_blob_still_directly_linked_to_another_repository(pool: sqlx::PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let blobs = Arc::new(FilesystemDockerBlobStore::new(pool.clone(), dir.path()));
            let manifests = Arc::new(PostgresDockerManifestRepository::new(pool.clone()));
            let repositories = Arc::new(FakeRepositories::new());
            let events = Arc::new(FakeDockerEvents::new());
            let repository_id = seed_repository(&pool).await;
            let other_repository_id = seed_repository(&pool).await;
            repositories.insert(repo_summary(repository_id));
            let name = DockerImageName::parse("myimage").unwrap();

            let config_bytes = b"cross-repo-cfg";
            let config_digest = Digest::of(config_bytes);
            blobs.write(&config_digest, config_bytes).await.unwrap();
            blobs.link_to_repository(repository_id, &config_digest).await.unwrap();
            // Directly uploaded to the OTHER repository too, with no manifest there ever referencing it.
            blobs.link_to_repository(other_repository_id, &config_digest).await.unwrap();

            let put_manifest = PutManifestUseCase::new(manifests.clone(), repositories, events.clone());
            let manifest_digest =
                put_manifest.execute(repository_id, &name, "v1", DockerMediaType::DockerV2Manifest, &manifest_body(config_digest.as_str(), "a"), Uuid::new_v4()).await.unwrap();

            let delete_manifest = DeleteManifestUseCase::new(manifests, blobs.clone(), events);
            delete_manifest.execute(repository_id, &name, &manifest_digest, Uuid::new_v4()).await.unwrap();

            assert!(blobs.exists(&config_digest).await.unwrap(), "still linked to the other repository — must not be deleted");
            assert!(!blobs.is_uploaded_to_repository(repository_id, &config_digest).await.unwrap(), "this repository's OWN stale link must still be cleaned up");
            assert!(blobs.is_uploaded_to_repository(other_repository_id, &config_digest).await.unwrap(), "the other repository's link must be untouched");
            // Fix round 1 (Finding 1): the row surviving is not enough on its own to prove the fix —
            // the pre-fix code ALSO left the row in place, but only because it rolled the whole
            // transaction back (including the decrement) the instant it hit `other_repository_id`'s
            // link row's FK. `reference_count` must actually reach 0 here, not silently stay at 1.
            let row = sqlx::query!("SELECT reference_count FROM docker_blobs WHERE digest = $1", config_digest.as_str()).fetch_one(&pool).await.unwrap();
            assert_eq!(row.reference_count, 0, "the decrement must commit even though the row itself can't be deleted yet");
        }
    }

    /// A failure removing one digest's file (a starved connection pool here, which a fake can't reproduce) must not
    /// abort the loop and leave the digests after it unreclaimed.
    mod phase_2_failure_does_not_abort_the_loop {
        use super::*;
        use artiferris_infrastructure::filesystem_docker_blob_store::FilesystemDockerBlobStore;

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_digests_phase_2_failure_does_not_skip_a_later_digests_own_decrement(pool: sqlx::PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let starved_pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(std::time::Duration::from_millis(300))
                .connect_with((*pool.connect_options()).clone())
                .await
                .unwrap();
            let blobs = Arc::new(FilesystemDockerBlobStore::new(starved_pool.clone(), dir.path()));
            let manifests = Arc::new(FakeDockerManifestRepository::new());
            let events = Arc::new(FakeDockerEvents::new());
            let repository_id = Uuid::new_v4();
            let name = DockerImageName::parse("myimage").unwrap();

            let digest_a = Digest::of(b"phase-2-loop-digest-a");
            let digest_b = Digest::of(b"phase-2-loop-digest-b");
            blobs.write(&digest_a, b"phase-2-loop-digest-a").await.unwrap();
            blobs.write(&digest_b, b"phase-2-loop-digest-b").await.unwrap();
            blobs.increment_ref(&digest_a).await.unwrap();
            blobs.increment_ref(&digest_b).await.unwrap();

            let manifest = DockerManifest {
                id: Uuid::new_v4(), package_repository_id: repository_id, image_name: name.clone(),
                digest: Digest::of(b"phase-2-loop-manifest"), media_type: DockerMediaType::DockerV2Manifest,
                body: b"{}".to_vec(), created_at: chrono::Utc::now(),
            };
            // Insertion order pins iteration order: A is reclaimed before B.
            manifests.insert_manifest(&manifest, &[digest_a.clone(), digest_b.clone()]).await.unwrap();
            // The Postgres manifest repository takes these references back when it deletes the manifest; the fake doesn't.
            sqlx::query!("UPDATE docker_blobs SET reference_count = 0 WHERE digest = ANY($1)", &[digest_a.as_str().to_string(), digest_b.as_str().to_string()] as &[String]).execute(&pool).await.unwrap();

            // Holds digest A's advisory lock, so its phase 1 — once it grabs the starved pool's
            // one connection — blocks there without releasing it, the same setup as the
            // infrastructure-level test this one builds on.
            let mut blocker_tx = pool.begin().await.unwrap();
            sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", digest_a.as_str()).execute(&mut *blocker_tx).await.unwrap();

            let use_case = DeleteManifestUseCase::new(manifests.clone(), blobs.clone(), events);
            let execute_handle = {
                let rt_handle = tokio::runtime::Handle::current();
                let manifest_digest = manifest.digest.clone();
                tokio::task::spawn_blocking(move || rt_handle.block_on(async move { use_case.execute(repository_id, &name, &manifest_digest, Uuid::new_v4()).await }))
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
            assert!(observed_phase_1_blocked, "digest A's phase 1 never blocked on its advisory lock — this test isn't exercising the real code path");

            let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
            let bystander_handle = {
                let starved_pool = starved_pool.clone();
                let rt_handle = tokio::runtime::Handle::current();
                tokio::task::spawn_blocking(move || {
                    rt_handle.block_on(async move {
                        let _ = queued_tx.send(());
                        let conn = starved_pool.acquire().await.unwrap();
                        // Held well past the starved pool's own 300ms acquire timeout, so digest
                        // A's phase 2 genuinely times out while this connection is still checked
                        // out — but released before digest B's own queries would need to wait
                        // long enough to matter.
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        drop(conn);
                    })
                })
            };
            queued_rx.await.unwrap();

            blocker_tx.commit().await.unwrap();
            bystander_handle.await.unwrap();

            let result = execute_handle.await.unwrap();
            assert!(result.is_ok(), "digest A's phase-2 failure must not surface as an Err out of the whole use case — got {result:?}");

            assert!(!blobs.exists(&digest_a).await.unwrap(), "digest A's row deletion must still be committed despite its own file removal failing");
            assert!(
                !blobs.exists(&digest_b).await.unwrap(),
                "digest B, processed AFTER A in the loop, must still have been reclaimed"
            );
        }
    }

    #[tokio::test]
    async fn deleting_an_image_with_no_tags_at_all_is_a_no_op() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let events = Arc::new(FakeDockerEvents::new());
        let delete_manifest = Arc::new(DeleteManifestUseCase::new(manifests.clone(), blobs, events));
        let use_case = DeleteDockerImageUseCase::new(manifests, delete_manifest);

        use_case.execute(Uuid::new_v4(), &DockerImageName::parse("never-pushed").unwrap(), Uuid::new_v4()).await.unwrap();
    }
}
