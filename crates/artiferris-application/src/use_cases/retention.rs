use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifestRepositoryPort};
use artiferris_domain::npm_package::NpmPackageRepositoryPort;
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, RepositoryFormat, RepositoryType};
use uuid::Uuid;

use crate::error::ApplicationError;
use crate::use_cases::docker_manifest_delete::DeleteManifestUseCase;
use crate::use_cases::npm_unpublish::UnpublishNpmPackageUseCase;

/// Nil UUID: never a real user's id, so always distinguishable from a human-triggered deletion.
pub const RETENTION_SWEEP_ACTOR: Uuid = Uuid::nil();

/// Never deleted regardless of rank — `docker pull image` with no explicit tag resolves to this.
const PROTECTED_DOCKER_TAG: &str = "latest";

/// Covers a multi-arch push (members first, the index after) still in progress.
const UNTAGGED_MANIFEST_GRACE: Duration = Duration::days(7);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetentionSweepReport {
    pub npm_versions_deleted: usize,
    pub docker_tags_deleted: usize,
    pub docker_untagged_manifests_deleted: usize,
}

/// Run periodically by a background timer, not on any request path. Deletes via `UnpublishNpmPackageUseCase`/`DeleteManifestUseCase` to reuse their storage/audit side effects.
pub struct SweepRetentionUseCase {
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    npm_packages: Arc<dyn NpmPackageRepositoryPort>,
    unpublish_npm: Arc<UnpublishNpmPackageUseCase>,
    docker_manifests: Arc<dyn DockerManifestRepositoryPort>,
    delete_docker_manifest: Arc<DeleteManifestUseCase>,
}

impl SweepRetentionUseCase {
    pub fn new(
        repositories: Arc<dyn PackageRepositoryQueryPort>,
        npm_packages: Arc<dyn NpmPackageRepositoryPort>,
        unpublish_npm: Arc<UnpublishNpmPackageUseCase>,
        docker_manifests: Arc<dyn DockerManifestRepositoryPort>,
        delete_docker_manifest: Arc<DeleteManifestUseCase>,
    ) -> Self {
        Self { repositories, npm_packages, unpublish_npm, docker_manifests, delete_docker_manifest }
    }

    pub async fn execute(&self) -> Result<RetentionSweepReport, ApplicationError> {
        let mut report = RetentionSweepReport::default();
        for repo in self.repositories.list_all().await? {
            let Some(keep_n) = repo.retention_keep_last_n else { continue };
            // Floor at 1 regardless of source (B-37): a `Some(0)` here — from a pre-existing bad
            // event, or any future bug elsewhere — must never mean "keep nothing, delete everything
            // not dist-tagged." This is independent of the domain-layer `apply` sanitization.
            let keep_n = keep_n.max(1) as usize;
            let swept = match repo.format {
                RepositoryFormat::Npm => self.sweep_npm(repo.id, keep_n, &mut report).await,
                RepositoryFormat::Docker => self.sweep_docker(repo.id, keep_n, repo.repo_type == RepositoryType::Hosted, &mut report).await,
            };
            // One repository's trouble must not keep the ones after it from being swept.
            if let Err(e) = swept {
                tracing::warn!(repository_id = %repo.id, error = %e, "retention sweep failed for a repository, moving on to the next");
            }
        }
        Ok(report)
    }

    /// Newest-first, keeps everything within `keep_n` plus any version a dist-tag still points at — a sweep must never silently break `npm install @latest`.
    ///
    /// Dist-tags are deliberately NOT pre-fetched once for the whole sweep (B-41): this loop can
    /// take seconds to minutes across a large repository, and `unpublish_npm.execute_version` on
    /// one version can run concurrently with an `npm dist-tag add` landing on another. A snapshot
    /// taken before the loop started would be blind to that race. Instead, each candidate's
    /// protection is re-checked immediately before its own deletion call, against current state.
    async fn sweep_npm(&self, repository_id: Uuid, keep_n: usize, report: &mut RetentionSweepReport) -> Result<(), ApplicationError> {
        let packages = self.npm_packages.search(repository_id, "", i64::MAX).await?;
        let package_ids: Vec<Uuid> = packages.iter().map(|p| p.id).collect();
        let mut versions_by_package: std::collections::HashMap<Uuid, Vec<_>> = std::collections::HashMap::new();
        for version in self.npm_packages.list_versions_for_packages(&package_ids).await? {
            versions_by_package.entry(version.npm_package_id).or_default().push(version);
        }

        for package in packages {
            let mut versions = versions_by_package.remove(&package.id).unwrap_or_default();
            if versions.len() <= keep_n {
                continue;
            }
            versions.sort_by_key(|v| std::cmp::Reverse(v.published_at));
            for (rank, version) in versions.iter().enumerate() {
                if rank < keep_n {
                    continue;
                }
                // Fresh, per-version query right before the delete decision — see method doc.
                let protected: HashSet<String> = self
                    .npm_packages
                    .list_dist_tags_for_packages(&[package.id])
                    .await?
                    .into_iter()
                    .map(|tag| tag.version.as_str())
                    .collect();
                if protected.contains(&version.version.as_str()) {
                    continue;
                }
                // Errors are swallowed: one package's stale state must not abort the whole sweep.
                if self.unpublish_npm.execute_version(repository_id, &package.name, &version.version, RETENTION_SWEEP_ACTOR).await.is_ok() {
                    report.npm_versions_deleted += 1;
                }
            }
        }
        Ok(())
    }

    /// Ranks tags by when the tag itself was set (a rollback makes the old digest the newest) and deletes everything past
    /// `keep_n`, except `latest`. On a hosted repository it then reclaims manifests no tag or manifest list references.
    async fn sweep_docker(&self, repository_id: Uuid, keep_n: usize, reclaim_untagged: bool, report: &mut RetentionSweepReport) -> Result<(), ApplicationError> {
        let mut tags_by_image: std::collections::HashMap<DockerImageName, Vec<(String, Digest, DateTime<Utc>)>> = std::collections::HashMap::new();
        for (image_name, tag, digest, updated_at) in self.docker_manifests.list_repository_tag_updates(repository_id).await? {
            tags_by_image.entry(image_name).or_default().push((tag, digest, updated_at));
        }

        for (image_name, mut tags) in tags_by_image {
            if tags.len() <= keep_n {
                continue;
            }
            tags.sort_by_key(|(_, _, updated_at)| std::cmp::Reverse(*updated_at));

            // A digest is protected if ANY of its tags is — deleting by digest removes every tag sharing it, so one kept alias must not doom the rest.
            let mut protected_digests = HashSet::new();
            for (rank, (tag, digest, _updated_at)) in tags.iter().enumerate() {
                if rank < keep_n || tag == PROTECTED_DOCKER_TAG {
                    protected_digests.insert(digest.clone());
                }
            }

            let mut deleted_digests = HashSet::new();
            for (_tag, digest, _updated_at) in &tags {
                if protected_digests.contains(digest) || deleted_digests.contains(digest) {
                    continue;
                }
                if self.delete_docker_manifest.execute(repository_id, &image_name, digest, RETENTION_SWEEP_ACTOR).await.is_ok() {
                    report.docker_tags_deleted += 1;
                    deleted_digests.insert(digest.clone());
                }
            }
        }

        if !reclaim_untagged {
            return Ok(());
        }
        let untagged_before = Utc::now() - UNTAGGED_MANIFEST_GRACE;
        for (image_name, digest) in self.docker_manifests.list_untagged_manifests(repository_id, untagged_before).await? {
            if let Ok(true) = self.delete_docker_manifest.execute_if_untagged(repository_id, &image_name, &digest, untagged_before, RETENTION_SWEEP_ACTOR).await {
                report.docker_untagged_manifests_deleted += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::docker_test_support::{FakeDockerBlobStore, FakeDockerEvents, FakeDockerManifestRepository, FakeRepositories as FakeDockerRepositories};
    use crate::use_cases::npm_test_support::{FakeEvents, FakePackages, FakeRepositories as FakeNpmRepositories, FakeStorage};
    use chrono::{Duration, Utc};
    use artiferris_domain::docker_registry::{Digest, DockerManifest, DockerMediaType};
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};
    use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryType};
    use artiferris_domain::storage::StorageBackendPort;

    fn npm_repo(id: Uuid, keep_last_n: Option<i32>) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id,
            organization_id: Uuid::new_v4(),
            name: "npm-repo".to_string(),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: keep_last_n,
            is_public: false,
        }
    }

    fn docker_repo(id: Uuid, keep_last_n: Option<i32>) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id,
            organization_id: Uuid::new_v4(),
            name: "docker-repo".to_string(),
            format: RepositoryFormat::Docker,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: keep_last_n,
            is_public: false,
        }
    }

    async fn seed_npm_version(packages: &FakePackages, storage: &FakeStorage, repository_id: Uuid, package_id: Uuid, version_str: &str, published_at: chrono::DateTime<Utc>) {
        let version = NpmVersion::parse(version_str).unwrap();
        let storage_key = format!("pkg/-/pkg-{version_str}.tgz");
        storage.write(repository_id, &storage_key, b"bytes").await.unwrap();
        packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package_id,
                version,
                manifest: serde_json::json!({}),
                shasum: "s".into(),
                integrity: "i".into(),
                tarball_storage_key: storage_key,
                tarball_size_bytes: 5,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at,
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
    }

    fn use_case_with(
        repositories: Arc<dyn artiferris_domain::package_repository::PackageRepositoryQueryPort>,
        packages: Arc<FakePackages>,
        storage: Arc<FakeStorage>,
        manifests: Arc<FakeDockerManifestRepository>,
        blobs: Arc<FakeDockerBlobStore>,
    ) -> SweepRetentionUseCase {
        let events = Arc::new(FakeEvents::new());
        let unpublish = Arc::new(UnpublishNpmPackageUseCase::new(packages.clone(), storage.clone(), events));
        let docker_events = Arc::new(FakeDockerEvents::new());
        let delete_manifest = Arc::new(DeleteManifestUseCase::new(manifests.clone(), blobs, docker_events));
        SweepRetentionUseCase::new(repositories, packages, unpublish, manifests, delete_manifest)
    }

    #[tokio::test]
    async fn prunes_npm_versions_beyond_the_keep_count_oldest_first() {
        let repositories = Arc::new(FakeNpmRepositories::new());
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(npm_repo(repository_id, Some(2)));

        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        let now = Utc::now();
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.0.0", now - Duration::days(3)).await;
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.1.0", now - Duration::days(2)).await;
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.2.0", now - Duration::days(1)).await;
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.3.0", now).await;

        let use_case = use_case_with(repositories, packages.clone(), storage, Arc::new(FakeDockerManifestRepository::new()), Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.unwrap();

        assert_eq!(report.npm_versions_deleted, 2);
        let remaining = packages.list_versions(package.id).await.unwrap();
        let remaining_versions: Vec<String> = remaining.iter().map(|v| v.version.as_str()).collect();
        assert!(remaining_versions.contains(&"1.3.0".to_string()));
        assert!(remaining_versions.contains(&"1.2.0".to_string()));
        assert!(!remaining_versions.contains(&"1.0.0".to_string()));
        assert!(!remaining_versions.contains(&"1.1.0".to_string()));
    }

    #[tokio::test]
    async fn a_repository_that_fails_does_not_keep_the_others_from_being_swept() {
        let repositories = Arc::new(FakeNpmRepositories::new());
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let mut ids = [Uuid::new_v4(), Uuid::new_v4()];
        ids.sort();
        let [failing_id, healthy_id] = ids;
        repositories.insert(npm_repo(failing_id, Some(1)));
        repositories.insert(npm_repo(healthy_id, Some(1)));
        *packages.failing_search_repository.lock().unwrap() = Some(failing_id);
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: healthy_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        seed_npm_version(&packages, &storage, healthy_id, package.id, "1.0.0", Utc::now() - Duration::days(1)).await;
        seed_npm_version(&packages, &storage, healthy_id, package.id, "2.0.0", Utc::now()).await;

        let use_case = use_case_with(repositories, packages, storage, Arc::new(FakeDockerManifestRepository::new()), Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.expect("one repository's failure is logged, not returned");

        assert_eq!(report.npm_versions_deleted, 1);
    }

    #[tokio::test]
    async fn a_dist_tagged_old_version_is_never_pruned() {
        let repositories = Arc::new(FakeNpmRepositories::new());
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(npm_repo(repository_id, Some(1)));

        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        let now = Utc::now();
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.0.0", now - Duration::days(2)).await;
        seed_npm_version(&packages, &storage, repository_id, package.id, "2.0.0", now).await;
        // An old version deliberately still tagged (e.g. a maintained LTS line) must survive even though rank alone would prune it.
        packages.set_dist_tag(package.id, "lts", &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        let use_case = use_case_with(repositories, packages.clone(), storage, Arc::new(FakeDockerManifestRepository::new()), Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.unwrap();

        assert_eq!(report.npm_versions_deleted, 0);
        let remaining: Vec<String> = packages.list_versions(package.id).await.unwrap().iter().map(|v| v.version.as_str()).collect();
        assert!(remaining.contains(&"1.0.0".to_string()));
    }

    /// B-41: `sweep_npm` must not decide a version's fate from a dist-tag snapshot taken before
    /// the deletion loop started. Simulated deterministically via `FakePackages`'s one-shot hook
    /// (see its doc comment) instead of real concurrency: the hook adds a NEW dist-tag pointing at
    /// an old version right after the sweep's first `list_dist_tags_for_packages` call returns,
    /// whichever version that call happens to be for. A snapshot-once implementation queries
    /// exactly once (before any package's versions are examined) and never observes the addition.
    /// A fixed implementation that re-checks before each individual deletion queries again later
    /// (for the next candidate version) and does observe it.
    #[tokio::test]
    async fn a_dist_tag_added_during_the_sweep_still_protects_its_version_from_deletion() {
        let repositories = Arc::new(FakeNpmRepositories::new());
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(npm_repo(repository_id, Some(1)));

        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        let now = Utc::now();
        // Newest-first candidates for deletion beyond keep_n=1: 1.1.0 is checked before 1.0.0.
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.0.0", now - Duration::days(2)).await;
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.1.0", now - Duration::days(1)).await;
        seed_npm_version(&packages, &storage, repository_id, package.id, "2.0.0", now).await;

        // No dist-tag exists when the sweep starts. As soon as the sweep's first dist-tag query
        // returns (for 1.1.0, the first deletion candidate), simulate a concurrent
        // `npm dist-tag add` landing on the OLDER 1.0.0 — the next candidate in line.
        let package_id = package.id;
        let packages_for_hook = packages.clone();
        *packages.after_first_list_dist_tags_for_packages.lock().unwrap() = Some(Box::new(move || {
            packages_for_hook.dist_tags.lock().unwrap().insert((package_id, "hotfix".to_string()), NpmVersion::parse("1.0.0").unwrap());
        }));

        let use_case = use_case_with(repositories, packages.clone(), storage, Arc::new(FakeDockerManifestRepository::new()), Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.unwrap();

        let remaining: Vec<String> = packages.list_versions(package.id).await.unwrap().iter().map(|v| v.version.as_str()).collect();
        assert!(
            remaining.contains(&"1.0.0".to_string()),
            "a dist-tag added mid-sweep must still protect its version from deletion; remaining={remaining:?} deleted={}",
            report.npm_versions_deleted
        );
        assert!(!remaining.contains(&"1.1.0".to_string()), "1.1.0 was never tagged and must still be pruned");
    }

    #[tokio::test]
    async fn a_repository_without_a_policy_is_left_untouched() {
        let repositories = Arc::new(FakeNpmRepositories::new());
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(npm_repo(repository_id, None));

        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        let now = Utc::now();
        for i in 0..5 {
            seed_npm_version(&packages, &storage, repository_id, package.id, &format!("1.{i}.0"), now - Duration::days(i)).await;
        }

        let use_case = use_case_with(repositories, packages.clone(), storage, Arc::new(FakeDockerManifestRepository::new()), Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.unwrap();

        assert_eq!(report.npm_versions_deleted, 0);
        assert_eq!(packages.list_versions(package.id).await.unwrap().len(), 5);
    }

    fn docker_manifest(repository_id: Uuid, name: &artiferris_domain::docker_registry::DockerImageName, content: &[u8], created_at: chrono::DateTime<Utc>) -> DockerManifest {
        DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            image_name: name.clone(),
            digest: Digest::of(content),
            media_type: DockerMediaType::DockerV2Manifest,
            body: content.to_vec(),
            created_at,
        }
    }

    #[tokio::test]
    async fn prunes_docker_tags_beyond_the_keep_count_but_never_latest() {
        let repositories = Arc::new(FakeDockerRepositories::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(docker_repo(repository_id, Some(1)));
        let image_name = artiferris_domain::docker_registry::DockerImageName::parse("my-app").unwrap();
        let now = Utc::now();

        let v1 = docker_manifest(repository_id, &image_name, b"v1", now - Duration::days(2));
        let v2 = docker_manifest(repository_id, &image_name, b"v2", now - Duration::days(1));
        let latest = docker_manifest(repository_id, &image_name, b"latest-content", now - Duration::days(3));
        manifests.insert_manifest(&v1, &[]).await.unwrap();
        manifests.insert_manifest(&v2, &[]).await.unwrap();
        manifests.insert_manifest(&latest, &[]).await.unwrap();
        manifests.set_tag_at(repository_id, &image_name, "1.0.0", v1.id, now - Duration::days(2)).await;
        manifests.set_tag_at(repository_id, &image_name, "1.1.0", v2.id, now - Duration::days(1)).await;
        // Oldest, but protected by name.
        manifests.set_tag_at(repository_id, &image_name, "latest", latest.id, now - Duration::days(3)).await;

        let use_case = use_case_with(
            repositories,
            Arc::new(FakePackages::new()),
            Arc::new(FakeStorage::new()),
            manifests.clone(),
            Arc::new(FakeDockerBlobStore::new()),
        );
        let report = use_case.execute().await.unwrap();

        assert_eq!(report.docker_tags_deleted, 1);
        let remaining_tags = manifests.list_tags(repository_id, &image_name).await.unwrap();
        assert!(remaining_tags.contains(&"1.1.0".to_string()), "the newest non-latest tag must survive");
        assert!(remaining_tags.contains(&"latest".to_string()), "latest must never be pruned");
        assert!(!remaining_tags.contains(&"1.0.0".to_string()), "the oldest non-latest tag must be pruned");
    }

    #[tokio::test]
    async fn a_kept_tag_survives_even_when_a_pruned_tag_shares_its_digest() {
        let repositories = Arc::new(FakeDockerRepositories::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(docker_repo(repository_id, Some(1)));
        let image_name = artiferris_domain::docker_registry::DockerImageName::parse("my-app").unwrap();
        let now = Utc::now();

        // "v3" and "stable" share a digest (common in CI/CD); "v2" is separate and older.
        let shared_build = docker_manifest(repository_id, &image_name, b"shared-build", now);
        let v2 = docker_manifest(repository_id, &image_name, b"v2", now - Duration::days(1));
        manifests.insert_manifest(&shared_build, &[]).await.unwrap();
        manifests.insert_manifest(&v2, &[]).await.unwrap();
        manifests.set_tag_at(repository_id, &image_name, "v3", shared_build.id, now).await;
        manifests.set_tag_at(repository_id, &image_name, "stable", shared_build.id, now).await;
        manifests.set_tag_at(repository_id, &image_name, "v2", v2.id, now - Duration::days(1)).await;

        let use_case = use_case_with(
            repositories,
            Arc::new(FakePackages::new()),
            Arc::new(FakeStorage::new()),
            manifests.clone(),
            Arc::new(FakeDockerBlobStore::new()),
        );
        let report = use_case.execute().await.unwrap();

        assert_eq!(report.docker_tags_deleted, 1, "only the v2 digest should have been deleted");
        let remaining_tags = manifests.list_tags(repository_id, &image_name).await.unwrap();
        assert!(remaining_tags.contains(&"v3".to_string()), "v3 must survive: it shares a digest with a kept tag");
        assert!(remaining_tags.contains(&"stable".to_string()), "stable must survive: it shares a digest with a kept tag");
        assert!(!remaining_tags.contains(&"v2".to_string()), "v2 has its own, older digest and must be pruned");
    }

    #[tokio::test]
    async fn a_docker_repository_within_the_keep_count_is_untouched() {
        let repositories = Arc::new(FakeDockerRepositories::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(docker_repo(repository_id, Some(5)));
        let image_name = artiferris_domain::docker_registry::DockerImageName::parse("my-app").unwrap();
        let now = Utc::now();
        let manifest = docker_manifest(repository_id, &image_name, b"only-tag", now);
        manifests.insert_manifest(&manifest, &[]).await.unwrap();
        manifests.set_tag(repository_id, &image_name, "1.0.0", manifest.id).await.unwrap();

        let use_case = use_case_with(
            repositories,
            Arc::new(FakePackages::new()),
            Arc::new(FakeStorage::new()),
            manifests.clone(),
            Arc::new(FakeDockerBlobStore::new()),
        );
        let report = use_case.execute().await.unwrap();

        assert_eq!(report.docker_tags_deleted, 0);
        assert_eq!(manifests.list_tags(repository_id, &image_name).await.unwrap(), vec!["1.0.0".to_string()]);
    }

    /// B-37 belt-and-suspenders: even if a repository's projection somehow reports
    /// `retention_keep_last_n = Some(0)` (e.g. a gap the domain-layer `apply` fix doesn't fully
    /// close, or a future bug elsewhere), the sweep's own floor must prevent wholesale deletion.
    /// Seeded directly into the fake projection, bypassing the domain layer entirely, so this
    /// proves the sweep's defense doesn't depend on the domain-layer fix.
    #[tokio::test]
    async fn the_sweep_never_treats_a_zero_keep_count_as_delete_everything() {
        let repositories = Arc::new(FakeNpmRepositories::new());
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(npm_repo(repository_id, Some(0)));

        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        let now = Utc::now();
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.0.0", now - Duration::days(2)).await;
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.1.0", now - Duration::days(1)).await;
        seed_npm_version(&packages, &storage, repository_id, package.id, "1.2.0", now).await;

        let use_case = use_case_with(repositories, packages.clone(), storage, Arc::new(FakeDockerManifestRepository::new()), Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.unwrap();

        assert!(report.npm_versions_deleted < 3, "a Some(0) keep count must not delete every version; got {}", report.npm_versions_deleted);
        let remaining = packages.list_versions(package.id).await.unwrap();
        assert!(!remaining.is_empty(), "at least the newest version must survive a Some(0) keep count");
        assert!(remaining.iter().any(|v| v.version.as_str() == "1.2.0"), "the newest version must be the one kept");
    }

    async fn tagged_manifest(manifests: &FakeDockerManifestRepository, repository_id: Uuid, image_name: &artiferris_domain::docker_registry::DockerImageName, content: &[u8], created_at: DateTime<Utc>, tag: &str, tagged_at: DateTime<Utc>) -> DockerManifest {
        let manifest = docker_manifest(repository_id, image_name, content, created_at);
        manifests.insert_manifest(&manifest, &[]).await.unwrap();
        manifests.set_tag_at(repository_id, image_name, tag, manifest.id, tagged_at).await;
        manifest
    }

    /// Re-tagging `prod` on a six-month-old digest is a rollback, and must survive.
    #[tokio::test]
    async fn a_tag_moved_back_onto_an_old_digest_survives_the_sweep() {
        let repositories = Arc::new(FakeDockerRepositories::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(docker_repo(repository_id, Some(2)));
        let image_name = artiferris_domain::docker_registry::DockerImageName::parse("my-app").unwrap();
        let now = Utc::now();

        let old_release = tagged_manifest(&manifests, repository_id, &image_name, b"release-1", now - Duration::days(180), "prod", now - Duration::days(1)).await;
        tagged_manifest(&manifests, repository_id, &image_name, b"release-2", now - Duration::days(10), "v2", now - Duration::days(10)).await;
        tagged_manifest(&manifests, repository_id, &image_name, b"release-3", now - Duration::days(5), "v3", now - Duration::days(5)).await;
        tagged_manifest(&manifests, repository_id, &image_name, b"release-4", now - Duration::days(2), "v4", now - Duration::days(2)).await;

        let use_case = use_case_with(repositories, Arc::new(FakePackages::new()), Arc::new(FakeStorage::new()), manifests.clone(), Arc::new(FakeDockerBlobStore::new()));
        use_case.execute().await.unwrap();

        let remaining = manifests.list_tags(repository_id, &image_name).await.unwrap();
        assert!(remaining.contains(&"prod".to_string()), "the rolled-back tag must survive: {remaining:?}");
        assert!(remaining.contains(&"v4".to_string()), "the newest tag must survive: {remaining:?}");
        assert!(!remaining.contains(&"v2".to_string()) && !remaining.contains(&"v3".to_string()), "the two oldest tags go: {remaining:?}");
        assert!(manifests.find_manifest_by_digest(repository_id, &image_name, &old_release.digest).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn an_untagged_manifest_is_reclaimed_after_the_grace_period_but_not_before() {
        let repositories = Arc::new(FakeDockerRepositories::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(docker_repo(repository_id, Some(5)));
        let image_name = artiferris_domain::docker_registry::DockerImageName::parse("my-app").unwrap();
        let now = Utc::now();
        let stale = docker_manifest(repository_id, &image_name, b"replaced-latest", now - Duration::days(30));
        let fresh = docker_manifest(repository_id, &image_name, b"just-pushed", now - Duration::days(1));
        manifests.insert_manifest(&stale, &[]).await.unwrap();
        manifests.insert_manifest(&fresh, &[]).await.unwrap();

        let use_case = use_case_with(repositories, Arc::new(FakePackages::new()), Arc::new(FakeStorage::new()), manifests.clone(), Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.unwrap();

        assert_eq!(report.docker_untagged_manifests_deleted, 1);
        assert!(manifests.find_manifest_by_digest(repository_id, &image_name, &stale.digest).await.unwrap().is_none());
        assert!(manifests.find_manifest_by_digest(repository_id, &image_name, &fresh.digest).await.unwrap().is_some(), "inside the grace period");
    }

    #[tokio::test]
    async fn the_untagged_reclaim_never_touches_a_tagged_manifest_or_a_manifest_list_member() {
        let repositories = Arc::new(FakeDockerRepositories::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(docker_repo(repository_id, Some(5)));
        let image_name = artiferris_domain::docker_registry::DockerImageName::parse("my-app").unwrap();
        let now = Utc::now();
        let long_ago = now - Duration::days(90);
        let tagged = tagged_manifest(&manifests, repository_id, &image_name, b"tagged", long_ago, "stable", long_ago).await;
        let member = docker_manifest(repository_id, &image_name, b"arm64-member", long_ago);
        manifests.insert_manifest(&member, &[]).await.unwrap();
        let mut index = docker_manifest(repository_id, &image_name, b"multi-arch-index", long_ago);
        index.media_type = DockerMediaType::OciIndex;
        manifests.insert_manifest(&index, &[]).await.unwrap();
        manifests.insert_manifest_list_members(index.id, &[member.digest.clone()]).await.unwrap();
        manifests.set_tag_at(repository_id, &image_name, "multi", index.id, long_ago).await;

        let use_case = use_case_with(repositories, Arc::new(FakePackages::new()), Arc::new(FakeStorage::new()), manifests.clone(), Arc::new(FakeDockerBlobStore::new()));
        let report = use_case.execute().await.unwrap();

        assert_eq!(report.docker_untagged_manifests_deleted, 0);
        for kept in [&tagged, &member, &index] {
            assert!(manifests.find_manifest_by_digest(repository_id, &image_name, &kept.digest).await.unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn a_repository_without_a_retention_policy_keeps_its_untagged_manifests() {
        let repositories = Arc::new(FakeDockerRepositories::new());
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(docker_repo(repository_id, None));
        let image_name = artiferris_domain::docker_registry::DockerImageName::parse("my-app").unwrap();
        let pinned_by_digest = docker_manifest(repository_id, &image_name, b"pinned", Utc::now() - Duration::days(90));
        manifests.insert_manifest(&pinned_by_digest, &[]).await.unwrap();

        let use_case = use_case_with(repositories, Arc::new(FakePackages::new()), Arc::new(FakeStorage::new()), manifests.clone(), Arc::new(FakeDockerBlobStore::new()));
        use_case.execute().await.unwrap();

        assert!(manifests.find_manifest_by_digest(repository_id, &image_name, &pinned_by_digest.digest).await.unwrap().is_some());
    }
}
