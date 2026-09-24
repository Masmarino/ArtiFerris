use std::sync::Arc;

use artiferris_domain::audit::{DockerRegistryEvent, EventPublisherPort};
use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerManifestRepositoryPort, DockerMediaType, parse_docker_tag};
use artiferris_domain::error::DomainError;
use artiferris_domain::package_repository::PackageRepositoryQueryPort;
use uuid::Uuid;

use crate::error::ApplicationError;

pub struct PutManifestUseCase {
    manifests: Arc<dyn DockerManifestRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    events: Arc<dyn EventPublisherPort>,
}

impl PutManifestUseCase {
    pub fn new(manifests: Arc<dyn DockerManifestRepositoryPort>, repositories: Arc<dyn PackageRepositoryQueryPort>, events: Arc<dyn EventPublisherPort>) -> Self {
        Self { manifests, repositories, events }
    }

    /// The digest is computed from `raw_body`, never trusted from the client.
    pub async fn execute(
        &self,
        repository_id: Uuid,
        image_name: &DockerImageName,
        reference: &str,
        media_type: DockerMediaType,
        raw_body: &[u8],
        actor_id: Uuid,
    ) -> Result<Digest, ApplicationError> {
        // A digest reference (manifest-list members) is already reachable by digest — don't tag it.
        // Reject a malformed tag (M-16) before any database work starts, not just before it's tagged:
        // once `insert_manifest_with_checks`/`insert_manifest` commits, the manifest row, its blob
        // links, and the bumped blob reference counts are permanent — a later rejection can no longer
        // undo them, leaving an untagged manifest that's invisible to tags/list and the retention sweep
        // but still consumes quota and blocks blob cleanup forever. Mapped to InvalidDockerPayload (not
        // left as a bare DomainError via `?`) so it surfaces as a 400 in errors.rs, matching every other
        // client-input validation in this file.
        if Digest::parse(reference).is_err() {
            parse_docker_tag(reference).map_err(|e| ApplicationError::InvalidDockerPayload(e.to_string()))?;
        }

        let digest = Digest::of(raw_body);
        if let Ok(claimed) = Digest::parse(reference) {
            if claimed != digest {
                return Err(ApplicationError::DockerDigestMismatch { expected: claimed.as_str().to_string(), computed: digest.as_str().to_string() });
            }
        }
        // Parsed only to extract digests below — the manifest stores raw_body verbatim, not this.
        let parsed: serde_json::Value =
            serde_json::from_slice(raw_body).map_err(|e| ApplicationError::InvalidDockerPayload(e.to_string()))?;

        let manifest = DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            image_name: image_name.clone(),
            digest: digest.clone(),
            media_type,
            body: raw_body.to_vec(),
            created_at: chrono::Utc::now(),
        };

        // No blob ref-counting for an index: its members hold their own blob refs, released when they're deleted individually.
        let (blob_digests, member_digests) = if media_type.is_index() {
            (Vec::new(), Some(extract_manifest_list_member_digests(&parsed)?))
        } else {
            (extract_blob_digests(&parsed)?, None)
        };
        // Quota is checked at manifest-push time, not at blob upload — a manifest push is what attributes bytes to a repository. The limit itself is static config, read outside the lock below; only the repository's current usage needs re-checking under it.
        let quota_bytes = self.repositories.find_by_id(repository_id).await?.and_then(|repo| repo.quota_bytes);

        // Reachability, the quota sum, the insert, and the blob ref-count bump all happen in one
        // transaction under a per-repository advisory lock (B-18) — see the port method's doc comment.
        // The returned id is the row's real one, which differs from `manifest.id` on conflict — every reference below must use it.
        let (manifest_id, _inserted) = match self.manifests.insert_manifest_with_checks(repository_id, &manifest, &blob_digests, quota_bytes).await {
            Err(DomainError::DockerBlobNotReachable(_)) => return Err(ApplicationError::DockerBlobNotFound),
            Err(DomainError::StorageQuotaExceeded) => return Err(ApplicationError::StorageQuotaExceeded),
            other => other?,
        };
        if let Some(member_digests) = member_digests {
            self.manifests.insert_manifest_list_members(manifest_id, &member_digests).await?;
        }

        // A digest reference (manifest-list members) is already reachable by digest — don't tag it.
        // Grammar already validated above, before any database work began.
        if Digest::parse(reference).is_err() {
            self.manifests.set_tag(repository_id, image_name, reference, manifest_id).await.map_err(|e| match e {
                DomainError::TooManyTags => ApplicationError::DockerTooManyTags,
                other => other.into(),
            })?;
        }

        self.events
            .publish_docker_event(
                DockerRegistryEvent::ImagePushed { image_name: image_name.as_str().to_string(), digest: digest.as_str().to_string() },
                repository_id,
                Some(actor_id),
            )
            .await?;

        Ok(digest)
    }
}

/// Most blob (or member manifest) entries one manifest may list; real images have a few dozen.
const MAX_MANIFEST_ENTRIES: usize = 4096;

/// Each digest once, in first-seen order: an image may list the same layer twice.
fn distinct_digests<'a>(entries: impl Iterator<Item = &'a serde_json::Value>) -> Result<Vec<Digest>, ApplicationError> {
    let mut seen = std::collections::HashSet::new();
    let mut digests = Vec::new();
    let mut listed = 0usize;
    for entry in entries {
        listed += 1;
        if listed > MAX_MANIFEST_ENTRIES {
            return Err(ApplicationError::InvalidDockerPayload(format!("a manifest may list at most {MAX_MANIFEST_ENTRIES} entries")));
        }
        if let Some(raw) = entry.get("digest").and_then(|d| d.as_str()) {
            let digest = Digest::parse(raw).map_err(|e| ApplicationError::InvalidDockerPayload(e.to_string()))?;
            if seen.insert(digest.clone()) {
                digests.push(digest);
            }
        }
    }
    Ok(digests)
}

/// Extracts every distinct blob digest a single-image manifest references: `config.digest` plus every entry in `layers[].digest`.
fn extract_blob_digests(body: &serde_json::Value) -> Result<Vec<Digest>, ApplicationError> {
    let config = body.get("config").into_iter();
    let layers = body.get("layers").and_then(|l| l.as_array()).into_iter().flatten();
    distinct_digests(config.chain(layers))
}

/// Extracts every distinct member manifest digest a manifest list / OCI index references (`manifests[].digest`).
fn extract_manifest_list_member_digests(body: &serde_json::Value) -> Result<Vec<Digest>, ApplicationError> {
    distinct_digests(body.get("manifests").and_then(|m| m.as_array()).into_iter().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::docker_test_support::{FakeDockerBlobStore, FakeDockerEvents, FakeDockerManifestRepository, FakeRepositories};
    use artiferris_domain::docker_registry::{Digest, DockerBlobStorePort, DockerImageName};

    fn sample_manifest_body(blob_digest: &str) -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "config": { "digest": blob_digest, "size": 100 },
            "layers": []
        })
    }

    /// A quota that one 20-byte blob and its manifest body fit in, but two of them don't (manifest bodies count against the quota too).
    fn quota_for_one_of_two_pushes() -> i64 {
        let body_len = serde_json::to_vec(&sample_manifest_body(Digest::of(b"blob-content-aaaa").as_str())).unwrap().len() as i64;
        20 + body_len + 10
    }

    #[tokio::test]
    async fn pushing_a_manifest_stores_it_and_tags_it_and_increments_blob_refs() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let config_digest = Digest::of(b"config-bytes");
        blobs.write(&config_digest, b"config-bytes").await.unwrap();
        blobs.link_to_repository(repository_id, &config_digest).await.unwrap();
        manifests.link_blob_store(blobs.clone());

        let use_case = PutManifestUseCase::new(manifests.clone(), Arc::new(FakeRepositories::new()), events);
        let body = sample_manifest_body(config_digest.as_str());
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let media_type = DockerMediaType::DockerV2Manifest;

        use_case.execute(repository_id, &name, "latest", media_type, &body_bytes, Uuid::new_v4()).await.unwrap();

        let tagged = manifests.find_manifest_by_tag(repository_id, &name, "latest").await.unwrap();
        assert!(tagged.is_some());
        assert!(matches!(blobs.blobs.lock().unwrap().get(config_digest.as_str()), Some((_, 1))));
    }

    #[tokio::test]
    async fn pushing_a_second_tag_of_byte_identical_content_tags_both() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let config_digest = Digest::of(b"config-bytes");
        blobs.write(&config_digest, b"config-bytes").await.unwrap();
        blobs.link_to_repository(repository_id, &config_digest).await.unwrap();
        manifests.link_blob_store(blobs.clone());

        let use_case = PutManifestUseCase::new(manifests.clone(), Arc::new(FakeRepositories::new()), events);
        let body = sample_manifest_body(config_digest.as_str());
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let media_type = DockerMediaType::DockerV2Manifest;

        use_case.execute(repository_id, &name, "latest", media_type, &body_bytes, Uuid::new_v4()).await.unwrap();
        use_case.execute(repository_id, &name, "v2", media_type, &body_bytes, Uuid::new_v4()).await.unwrap();

        let latest = manifests.find_manifest_by_tag(repository_id, &name, "latest").await.unwrap();
        let v2 = manifests.find_manifest_by_tag(repository_id, &name, "v2").await.unwrap();
        assert!(latest.is_some());
        assert!(v2.is_some());
        assert_eq!(latest.unwrap().id, v2.unwrap().id, "both tags must resolve to the SAME persisted manifest row");
        assert!(
            matches!(blobs.blobs.lock().unwrap().get(config_digest.as_str()), Some((_, 1))),
            "re-pushing byte-identical content must not bump the ref count again — the blob rows already exist from the first push"
        );
    }

    #[tokio::test]
    async fn pushing_a_manifest_with_a_malformed_tag_is_rejected() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let config_digest = Digest::of(b"config-bytes");
        blobs.write(&config_digest, b"config-bytes").await.unwrap();
        blobs.link_to_repository(repository_id, &config_digest).await.unwrap();
        manifests.link_blob_store(blobs);

        let use_case = PutManifestUseCase::new(manifests.clone(), Arc::new(FakeRepositories::new()), events);
        let body = sample_manifest_body(config_digest.as_str());
        let body_bytes = serde_json::to_vec(&body).unwrap();

        // "!" is outside the OCI tag grammar `[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}`.
        let result =
            use_case.execute(repository_id, &name, "latest!", DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await;

        assert!(matches!(result, Err(ApplicationError::InvalidDockerPayload(_))), "got {result:?}");
        assert_eq!(
            manifests.list_tags(repository_id, &name).await.unwrap(),
            Vec::<String>::new(),
            "a malformed tag must not be persisted"
        );
        // The tag grammar is checked before any database work starts, so a rejected push must never
        // reach insert_manifest_with_checks in the first place — not just leave the tag unset. Proves
        // the manifest row itself (and its blob ref-count bump) was never committed; the tag-only
        // assertion above wouldn't have caught the manifest being orphaned but persisted.
        let manifest_digest = Digest::of(&body_bytes);
        assert!(
            manifests.find_manifest_by_digest(repository_id, &name, &manifest_digest).await.unwrap().is_none(),
            "a malformed-tag push must be rejected before the manifest is ever persisted"
        );
    }

    #[tokio::test]
    async fn pushing_by_digest_reference_does_not_create_a_tag() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let config_digest = Digest::of(b"config-bytes");
        blobs.write(&config_digest, b"config-bytes").await.unwrap();
        blobs.link_to_repository(repository_id, &config_digest).await.unwrap();
        manifests.link_blob_store(blobs);
        let use_case = PutManifestUseCase::new(manifests.clone(), Arc::new(FakeRepositories::new()), events);
        let body = sample_manifest_body(config_digest.as_str());
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let manifest_digest = Digest::of(&body_bytes);

        use_case.execute(repository_id, &name, manifest_digest.as_str(), DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await.unwrap();

        assert_eq!(manifests.list_tags(repository_id, &name).await.unwrap(), Vec::<String>::new());
        assert!(manifests.find_manifest_by_digest(repository_id, &name, &manifest_digest).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn pushing_by_a_digest_that_is_not_the_bodys_digest_is_rejected_and_stores_nothing() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let use_case = PutManifestUseCase::new(manifests.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeDockerEvents::new()));
        let body_bytes = serde_json::to_vec(&sample_manifest_body(Digest::of(b"config-bytes").as_str())).unwrap();
        let wrong = Digest::of(b"something else");

        let err = use_case.execute(repository_id, &name, wrong.as_str(), DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await.unwrap_err();

        assert!(matches!(err, ApplicationError::DockerDigestMismatch { .. }), "{err:?}");
        assert!(manifests.find_manifest_by_digest(repository_id, &name, &Digest::of(&body_bytes)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn pushing_a_manifest_referencing_an_unknown_blob_is_rejected() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let use_case = PutManifestUseCase::new(manifests, Arc::new(FakeRepositories::new()), events);
        let name = DockerImageName::parse("myimage").unwrap();
        let missing_digest = Digest::of(b"never-uploaded");
        let body = sample_manifest_body(missing_digest.as_str());
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = use_case.execute(Uuid::new_v4(), &name, "latest", DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await;
        assert!(matches!(result, Err(ApplicationError::DockerBlobNotFound)));
    }

    #[tokio::test]
    async fn pushing_a_manifest_referencing_a_blob_that_exists_globally_but_belongs_to_a_different_repository_is_rejected() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let victim_repository_id = Uuid::new_v4();
        let attacker_repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let victim_digest = Digest::of(b"victims-private-layer");
        blobs.write(&victim_digest, b"victims-private-layer").await.unwrap();
        blobs.link_to_repository(victim_repository_id, &victim_digest).await.unwrap();
        manifests.link_blob_store(blobs);

        let use_case = PutManifestUseCase::new(manifests, Arc::new(FakeRepositories::new()), events);
        let body = sample_manifest_body(victim_digest.as_str());
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = use_case.execute(attacker_repository_id, &name, "latest", DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await;

        assert!(matches!(result, Err(ApplicationError::DockerBlobNotFound)), "got {result:?}");
    }

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

    #[tokio::test]
    async fn pushing_a_manifest_whose_blobs_exceed_the_quota_is_rejected() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let config_digest = Digest::of(b"config-bytes"); // 12 bytes
        blobs.write(&config_digest, b"config-bytes").await.unwrap();
        blobs.link_to_repository(repository_id, &config_digest).await.unwrap();
        manifests.link_blob_store(blobs);
        repositories.insert(repo_with_quota(repository_id, Some(5)));

        let use_case = PutManifestUseCase::new(manifests, repositories, events);
        let body = sample_manifest_body(config_digest.as_str());
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = use_case.execute(repository_id, &name, "latest", DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await;

        assert!(matches!(result, Err(ApplicationError::StorageQuotaExceeded)), "got {result:?}");
    }

    #[tokio::test]
    async fn pushing_a_manifest_within_the_quota_succeeds() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let config_digest = Digest::of(b"config-bytes");
        blobs.write(&config_digest, b"config-bytes").await.unwrap();
        blobs.link_to_repository(repository_id, &config_digest).await.unwrap();
        manifests.link_blob_store(blobs);
        repositories.insert(repo_with_quota(repository_id, Some(1024)));

        let use_case = PutManifestUseCase::new(manifests, repositories, events);
        let body = sample_manifest_body(config_digest.as_str());
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = use_case.execute(repository_id, &name, "latest", DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await;

        assert!(result.is_ok(), "got {result:?}");
    }

    #[tokio::test]
    async fn pushing_a_manifest_list_index_is_never_quota_checked() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let events = Arc::new(FakeDockerEvents::new());
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        repositories.insert(repo_with_quota(repository_id, Some(1)));

        let use_case = PutManifestUseCase::new(manifests, repositories, events);
        let member_digest = Digest::of(b"member-manifest-bytes");
        let index_body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [{ "digest": member_digest.as_str(), "size": 100, "platform": { "architecture": "amd64", "os": "linux" } }]
        });
        let body_bytes = serde_json::to_vec(&index_body).unwrap();

        let result =
            use_case.execute(repository_id, &name, "latest", DockerMediaType::OciIndex, &body_bytes, Uuid::new_v4()).await;

        assert!(result.is_ok(), "got {result:?}");
    }

    /// The in-memory fakes above can't model this — the race is across two real database
    /// connections. Seeds a repository with a tight quota and two individually-under-quota blobs
    /// already uploaded to it, then races two real pushes (each referencing a different one of those
    /// blobs) through the real `PostgresDockerManifestRepository`-backed use case. Before B-18's fix,
    /// this either failed outright (both pushes read "under quota" before either committed) or was
    /// flaky depending on how the two connections happened to interleave.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_concurrent_pushes_that_together_exceed_the_quota_do_not_both_succeed(pool: sqlx::PgPool) {
        let repository_id = Uuid::new_v4();
        let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, quota_bytes, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'docker', 'hosted', NULL, $4, 1, now(), now())",
            repository_id,
            organization_id,
            format!("race-repo-{repository_id}"),
            quota_for_one_of_two_pushes(),
        )
        .execute(&pool)
        .await
        .unwrap();

        // Two distinct blobs, each 20 bytes — individually under the quota (with their manifest bodies), together over it.
        let blob_a = Digest::of(b"blob-content-aaaa");
        let blob_b = Digest::of(b"blob-content-bbbb");
        for digest in [&blob_a, &blob_b] {
            sqlx::query!(
                "INSERT INTO docker_blobs (digest, size_bytes, storage_key, reference_count, created_at) VALUES ($1, 20, $1, 0, now())",
                digest.as_str(),
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query!("INSERT INTO docker_repository_blobs (package_repository_id, blob_digest) VALUES ($1, $2)", repository_id, digest.as_str())
                .execute(&pool)
                .await
                .unwrap();
        }

        let name = DockerImageName::parse("myimage").unwrap();
        let body_a = serde_json::to_vec(&sample_manifest_body(blob_a.as_str())).unwrap();
        let body_b = serde_json::to_vec(&sample_manifest_body(blob_b.as_str())).unwrap();

        // `#[sqlx::test]` drives this whole test on a single-threaded current-thread Tokio runtime, and
        // on localhost every query round trip resolves fast enough that a plain `tokio::join!` never
        // actually interleaves the two pushes — one runs to completion (commit included) before the
        // other's very first query is even polled, so the two transactions never contend regardless of
        // whether the fix exists. `spawn_blocking` puts each push on its own real OS thread (tokio's
        // blocking pool) for genuine preemptive concurrency, while `Handle::current().block_on` keeps
        // both on the SAME runtime as `pool` — a `PgPool` isn't safe to drive from a second, unrelated
        // runtime (an entirely separate `Runtime::new_current_thread()` per thread was tried first and
        // reliably deadlocked pool acquisition until the 30s acquire-timeout fired).
        let run_push = |pool: sqlx::PgPool, tag: &'static str, body: Vec<u8>| {
            let name = name.clone();
            let handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                handle.block_on(async move {
                    let manifests = Arc::new(artiferris_infrastructure::postgres::docker_manifest_repository::PostgresDockerManifestRepository::new(pool));
                    let repositories = Arc::new(FakeRepositories::new());
                    repositories.insert(repo_with_quota(repository_id, Some(quota_for_one_of_two_pushes())));
                    let use_case = PutManifestUseCase::new(manifests, repositories, Arc::new(FakeDockerEvents::new()));
                    use_case.execute(repository_id, &name, tag, DockerMediaType::DockerV2Manifest, &body, Uuid::new_v4()).await
                })
            })
        };

        let handle_a = run_push(pool.clone(), "tag-a", body_a);
        let handle_b = run_push(pool, "tag-b", body_b);
        let result_a = handle_a.await.unwrap();
        let result_b = handle_b.await.unwrap();

        let successes = [&result_a, &result_b].into_iter().filter(|r| r.is_ok()).count();
        assert_eq!(successes, 1, "exactly one of two concurrent pushes that together exceed the quota must succeed — got a={result_a:?} b={result_b:?}");
        let rejected = if result_a.is_err() { &result_a } else { &result_b };
        assert!(matches!(rejected, Err(ApplicationError::StorageQuotaExceeded)), "the rejected push must fail with StorageQuotaExceeded, got {rejected:?}");
    }

    /// Deterministic companion to the timing-based test above. Mutation-testing that test (batch-2
    /// follow-up investigation: temporarily removing B-18's lock call, then replacing it with a
    /// no-op `SELECT 1`, then releasing it in a separate committed transaction before the
    /// checks/insert run) showed it has the SAME blind spot npm's original timing-based quota-lock
    /// test had before it was replaced (`npm_publish.rs`): totally removing the lock is caught
    /// reliably (30/30 runs), but a no-op'd lock is only caught ~23% of the time (7/30) and an
    /// early-released lock only ~3% of the time (1/30) — on localhost, `#[sqlx::test]` +
    /// `spawn_blocking` usually lets one push run to completion before the other's first query is
    /// even polled, regardless of whether the lock actually does anything.
    ///
    /// Docker's push has no externally-pausable step like npm's `StorageBackendPort::write` to hook
    /// a `PausingStorage`-style decorator into: `insert_manifest_with_checks` is a single function,
    /// entirely SQL, with no port call inside its transaction. Rather than adding a test-only pause
    /// hook to production code, this reuses a row lock the production code ALREADY takes as part of
    /// its normal work — the same "compete for a lock production code already takes" technique
    /// `a_concurrent_link_landing_between_the_guard_and_the_delete_does_not_lose_the_decrement`
    /// (`filesystem_docker_blob_store.rs`) uses. A bystander connection pre-locks push A's blob row
    /// in `docker_blobs` with `SELECT ... FOR UPDATE` before A even starts. A then runs all the way
    /// through the advisory-lock acquire, the reachability check, the quota recheck, and its own
    /// manifest-row insert — i.e. it is fully "inside" its locked critical section, quota check
    /// already passed — and only then blocks: the very next statement, inserting into
    /// `docker_manifest_blobs` (which has an FK to `docker_blobs(digest)`), needs a `FOR KEY SHARE`
    /// lock on that same blob row to satisfy the FK check, which conflicts with the bystander's
    /// stronger `FOR UPDATE` lock. That block is confirmed via `pg_stat_activity` (not a sleep)
    /// before push B is even started; B's own attempt to acquire the SAME per-repository advisory
    /// lock — still held by A, which hasn't committed — is then confirmed blocked too, before the
    /// bystander row lock is released and A is allowed to finish and commit. Only then does B
    /// unblock, see A's now-committed usage, and get correctly rejected for quota.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_push_blocked_on_the_quota_lock_is_rejected_once_the_holder_commits_over_quota(pool: sqlx::PgPool) {
        let repository_id = Uuid::new_v4();
        let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, quota_bytes, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'docker', 'hosted', NULL, $4, 1, now(), now())",
            repository_id,
            organization_id,
            format!("blocking-race-repo-{repository_id}"),
            quota_for_one_of_two_pushes(),
        )
        .execute(&pool)
        .await
        .unwrap();

        // Two distinct blobs, each 20 bytes — individually under the quota (with their manifest bodies), together over it.
        let blob_a = Digest::of(b"blob-content-aaaa");
        let blob_b = Digest::of(b"blob-content-bbbb");
        for digest in [&blob_a, &blob_b] {
            sqlx::query!(
                "INSERT INTO docker_blobs (digest, size_bytes, storage_key, reference_count, created_at) VALUES ($1, 20, $1, 0, now())",
                digest.as_str(),
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query!("INSERT INTO docker_repository_blobs (package_repository_id, blob_digest) VALUES ($1, $2)", repository_id, digest.as_str())
                .execute(&pool)
                .await
                .unwrap();
        }

        let name = DockerImageName::parse("myimage").unwrap();
        let body_a = serde_json::to_vec(&sample_manifest_body(blob_a.as_str())).unwrap();
        let body_b = serde_json::to_vec(&sample_manifest_body(blob_b.as_str())).unwrap();

        // Bystander connection: locks blob A's row before push A even starts, so push A runs all the
        // way through the work `two_concurrent_pushes_...` relies on thread-scheduling luck to
        // interleave, and only then blocks, deterministically, at the ref-count UPDATE.
        let mut blocker_tx = pool.begin().await.unwrap();
        sqlx::query!("SELECT reference_count FROM docker_blobs WHERE digest = $1 FOR UPDATE", blob_a.as_str())
            .fetch_one(&mut *blocker_tx)
            .await
            .unwrap();

        let run_push = |pool: sqlx::PgPool, tag: &'static str, body: Vec<u8>| {
            let name = name.clone();
            let handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                handle.block_on(async move {
                    let manifests = Arc::new(artiferris_infrastructure::postgres::docker_manifest_repository::PostgresDockerManifestRepository::new(pool));
                    let repositories = Arc::new(FakeRepositories::new());
                    repositories.insert(repo_with_quota(repository_id, Some(quota_for_one_of_two_pushes())));
                    let use_case = PutManifestUseCase::new(manifests, repositories, Arc::new(FakeDockerEvents::new()));
                    use_case.execute(repository_id, &name, tag, DockerMediaType::DockerV2Manifest, &body, Uuid::new_v4()).await
                })
            })
        };

        let handle_a = run_push(pool.clone(), "tag-a", body_a);

        // Wait until push A is genuinely blocked — it has already acquired the advisory lock and
        // passed its own reachability + quota checks and inserted its manifest row; the bystander's
        // `FOR UPDATE` lock on blob A's `docker_blobs` row is incompatible with the `FOR KEY SHARE`
        // lock the very next statement's FK check needs on that same row (inserting into
        // `docker_manifest_blobs`, which references `docker_blobs(digest)`), so A blocks there —
        // one statement earlier than the ref-count `UPDATE` itself, but still after every check has
        // already passed and the manifest row already inserted.
        let mut a_blocked = false;
        for _ in 0..500 {
            let blocked: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%INSERT INTO docker_manifest_blobs%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if blocked.0 > 0 {
                a_blocked = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(a_blocked, "push A never blocked inserting its manifest-blob link — this test isn't exercising the interleaving it claims to");

        let handle_b = run_push(pool.clone(), "tag-b", body_b);

        // Confirm push B is blocked trying to acquire the SAME per-repository advisory lock A still
        // holds — proving the two pushes genuinely contend on the lock, not merely on timing.
        let mut b_blocked = false;
        for _ in 0..500 {
            let blocked: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%pg_advisory_xact_lock%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if blocked.0 > 0 {
                b_blocked = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(b_blocked, "push B never entered a lock wait on pg_advisory_xact_lock — this test isn't exercising the blocking it claims to");

        // Release the bystander lock — only now can A's UPDATE, and then its commit (which releases
        // the advisory lock), proceed. If B were merely racing rather than genuinely blocked, it
        // could already have read stale usage and be past its own check by this point.
        blocker_tx.rollback().await.unwrap();

        let result_a = handle_a.await.unwrap();
        let result_b = handle_b.await.unwrap();

        assert!(result_a.is_ok(), "push A, which already held the lock and passed its own recheck, must succeed, got {result_a:?}");
        assert!(
            matches!(result_b, Err(ApplicationError::StorageQuotaExceeded)),
            "push B, unblocked only after A's commit, must be rejected for quota, got {result_b:?}"
        );
    }

    #[tokio::test]
    async fn a_layer_listed_twice_is_stored_and_counted_once() {
        let blobs = Arc::new(FakeDockerBlobStore::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let config_digest = Digest::of(b"config-bytes");
        let layer_digest = Digest::of(b"layer-bytes");
        for (digest, bytes) in [(&config_digest, &b"config-bytes"[..]), (&layer_digest, &b"layer-bytes"[..])] {
            blobs.write(digest, bytes).await.unwrap();
            blobs.link_to_repository(repository_id, digest).await.unwrap();
        }
        manifests.link_blob_store(blobs.clone());
        let use_case = PutManifestUseCase::new(manifests.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeDockerEvents::new()));
        let body = serde_json::json!({
            "schemaVersion": 2,
            "config": { "digest": config_digest.as_str() },
            "layers": [{ "digest": layer_digest.as_str() }, { "digest": layer_digest.as_str() }]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        use_case.execute(repository_id, &name, "latest", DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await.unwrap();

        let stored = manifests.find_manifest_by_tag(repository_id, &name, "latest").await.unwrap().unwrap();
        assert_eq!(manifests.list_manifest_blob_digests(stored.id).await.unwrap().len(), 2, "config and one layer");
        assert!(matches!(blobs.blobs.lock().unwrap().get(layer_digest.as_str()), Some((_, 1))), "one reference for the repeated layer");
    }

    #[tokio::test]
    async fn a_manifest_listing_more_entries_than_the_cap_is_rejected_before_any_database_work() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let layer = Digest::of(b"layer");
        let layers: Vec<_> = (0..MAX_MANIFEST_ENTRIES + 1).map(|_| serde_json::json!({ "digest": layer.as_str() })).collect();
        let body_bytes = serde_json::to_vec(&serde_json::json!({ "schemaVersion": 2, "config": { "digest": layer.as_str() }, "layers": layers })).unwrap();
        let use_case = PutManifestUseCase::new(manifests.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeDockerEvents::new()));

        let result = use_case.execute(repository_id, &name, "latest", DockerMediaType::DockerV2Manifest, &body_bytes, Uuid::new_v4()).await;

        assert!(matches!(result, Err(ApplicationError::InvalidDockerPayload(_))), "got {result:?}");
        assert!(manifests.find_manifest_by_digest(repository_id, &name, &Digest::of(&body_bytes)).await.unwrap().is_none());
    }
}
