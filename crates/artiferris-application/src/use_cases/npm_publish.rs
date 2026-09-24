use std::sync::Arc;

use bytes::Bytes;
use chrono::Utc;
use artiferris_domain::audit::{EventPublisherPort, NpmPackageEvent};
use artiferris_domain::npm_package::{
    NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageRepositoryPort, NpmPackageVersion, NpmVersion, validate_dist_tag,
};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, RepositoryQuotaLockPort};
use artiferris_domain::storage::StorageBackendPort;
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha512;
use uuid::Uuid;

use crate::error::ApplicationError;

/// It is served back in every metadata response, anonymous ones included.
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

pub struct PublishNpmPackageUseCase {
    packages: Arc<dyn NpmPackageRepositoryPort>,
    storage: Arc<dyn StorageBackendPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    quota_lock: Arc<dyn RepositoryQuotaLockPort>,
    events: Arc<dyn EventPublisherPort>,
}

impl PublishNpmPackageUseCase {
    pub fn new(
        packages: Arc<dyn NpmPackageRepositoryPort>,
        storage: Arc<dyn StorageBackendPort>,
        repositories: Arc<dyn PackageRepositoryQueryPort>,
        quota_lock: Arc<dyn RepositoryQuotaLockPort>,
        events: Arc<dyn EventPublisherPort>,
    ) -> Self {
        Self { packages, storage, repositories, quota_lock, events }
    }

    /// `dist_tags` are the tags the publisher asked for (`--tag`); with none, a stable version moves `latest` and a
    /// prerelease moves nothing. `name` and `version` are written into the stored manifest.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        repository_id: Uuid,
        name: &NpmPackageName,
        version: &NpmVersion,
        manifest: serde_json::Value,
        tarball_bytes: Bytes,
        dist_tags: &[String],
        publisher_id: Uuid,
    ) -> Result<Uuid, ApplicationError> {
        for tag in dist_tags {
            validate_dist_tag(tag).map_err(|e| ApplicationError::InvalidNpmPayload(e.to_string()))?;
        }
        let mut manifest = manifest;
        let Some(fields) = manifest.as_object_mut() else {
            return Err(ApplicationError::InvalidNpmPayload("the version manifest must be a JSON object".into()));
        };
        fields.insert("name".to_string(), serde_json::json!(name.as_str()));
        fields.insert("version".to_string(), serde_json::json!(version.as_str()));
        if serde_json::to_vec(&manifest).map_or(true, |bytes| bytes.len() > MAX_MANIFEST_BYTES) {
            return Err(ApplicationError::InvalidNpmPayload(format!("the version manifest is larger than {MAX_MANIFEST_BYTES} bytes")));
        }

        // An unpublished version never comes back.
        if self.packages.was_unpublished(repository_id, name, version).await? {
            return Err(ApplicationError::PackageVersionExists);
        }

        // Cheap early refusal; the authoritative check is the one `publish_version` makes under the package lock.
        if let Some(existing) = self.packages.find_package(repository_id, name).await? {
            if self.packages.find_version(existing.id, version).await?.is_some() {
                return Err(ApplicationError::PackageVersionExists);
            }
        }

        // Hashing a full tarball is CPU-bound; off the async executor so a large publish doesn't stall other requests.
        // `Bytes::clone()` is a refcount bump (O(1)), not a copy — unlike the `Vec<u8>` this used to be built from.
        // Done before the quota lock below, and before the storage key is derived: hashing only
        // needs the tarball bytes, and the storage key below needs `shasum` already computed.
        let tarball_for_hashing = tarball_bytes.clone();
        let (shasum, integrity) = tokio::task::spawn_blocking(move || {
            let shasum = hex::encode(Sha1::digest(&tarball_for_hashing));
            let integrity = format!("sha512-{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, Sha512::digest(&tarball_for_hashing)));
            (shasum, integrity)
        })
        .await
        .map_err(|e| artiferris_domain::error::DomainError::Infrastructure(e.to_string()))?;

        let quota_bytes = self.repositories.find_by_id(repository_id).await?.and_then(|repo| repo.quota_bytes);

        // Held across the recheck-and-write, only when a quota applies (unlimited repos have nothing
        // to race on). Closes the TOCTOU window between reading used_bytes and the tarball landing on
        // disk — same pg_advisory_xact_lock convention as Docker's B-18 fix.
        let lock = if let Some(quota) = quota_bytes {
            let lock = self.quota_lock.acquire_repository_lock(repository_id).await?;
            let used = self.storage.used_bytes(repository_id).await?;
            if used + tarball_bytes.len() as u64 > quota as u64 {
                return Err(ApplicationError::StorageQuotaExceeded);
            }
            Some(lock)
        } else {
            None
        };

        // Derived server-side, never from the client's filename. The per-attempt `attempt_id` keeps two publishes of the same
        // version, identical bytes included, from sharing a file, so the loser's cleanup can't delete the winner's tarball.
        let attempt_id = Uuid::new_v4();
        let safe_scope_and_name = name.as_str().trim_start_matches('@');
        let tarball_name = format!("{}-{shasum}-{attempt_id}.tgz", version.as_str());
        let storage_key = format!("{safe_scope_and_name}/-/{tarball_name}");
        self.storage.write(repository_id, &storage_key, &tarball_bytes).await?;
        // The write it protected is done; the rest doesn't need the quota lock.
        drop(lock);

        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: name.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        let npm_version = NpmPackageVersion {
            id: Uuid::new_v4(),
            npm_package_id: package.id,
            version: version.clone(),
            manifest,
            shasum,
            integrity,
            tarball_storage_key: storage_key.clone(),
            tarball_size_bytes: tarball_bytes.len() as i64,
            deprecated: false,
            deprecated_message: None,
            published_by: Some(publisher_id),
            published_at: Utc::now(),
            origin: NpmPackageOrigin::Local,
        };
        // A prerelease never moves `latest` on its own.
        let tags_to_set: Vec<String> = if !dist_tags.is_empty() {
            dist_tags.to_vec()
        } else if !version.is_prerelease() {
            vec!["latest".to_string()]
        } else {
            Vec::new()
        };
        let package_id = match self.packages.publish_version(&package, &npm_version, &tags_to_set).await {
            Ok(package_id) => package_id,
            Err(e) => {
                // The tarball goes unless the commit itself failed: its outcome is unknown, and deleting the file of a row
                // that did commit would be worse than leaking one that didn't.
                if matches!(e, artiferris_domain::error::DomainError::CommitFailed(_)) {
                    tracing::warn!(%repository_id, storage_key, error = %e, "the commit of a publish failed with an unknown outcome; leaving its tarball in place");
                } else if let Err(cleanup_err) = self.storage.delete(repository_id, &storage_key).await {
                    tracing::warn!(%repository_id, storage_key, error = %cleanup_err, "could not remove the tarball of a publish that was refused");
                }
                return Err(match e {
                    artiferris_domain::error::DomainError::NpmVersionAlreadyExists => ApplicationError::PackageVersionExists,
                    artiferris_domain::error::DomainError::NpmPackageLimit(message) => ApplicationError::InvalidNpmPayload(message),
                    other => other.into(),
                });
            }
        };

        self.events
            .publish_npm_event(
                NpmPackageEvent::PackagePushed { package_name: name.as_str().to_string(), version: version.as_str() },
                package_id,
                repository_id,
                Some(publisher_id),
            )
            .await?;

        Ok(npm_version.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::npm_test_support::{FakeEvents, FakePackages, FakeRepositories, FakeRepositoryQuotaLock, FakeStorage};
    use artiferris_domain::npm_package::{NpmPackageName, NpmVersion};

    fn use_case() -> PublishNpmPackageUseCase {
        PublishNpmPackageUseCase::new(
            Arc::new(FakePackages::new()),
            Arc::new(FakeStorage::new()),
            Arc::new(FakeRepositories::new()),
            Arc::new(FakeRepositoryQuotaLock::new()),
            Arc::new(FakeEvents::new()),
        )
    }

    #[tokio::test]
    async fn publishing_a_new_package_creates_it_and_sets_latest() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let events = Arc::new(FakeEvents::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), storage.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), events.clone());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        use_case.execute(repository_id, &name, &version, serde_json::json!({}), Bytes::from_static(b"tarball-bytes"), &[], Uuid::new_v4()).await.unwrap();

        let package = packages.packages.lock().unwrap().get(&(repository_id, name.as_str().to_string())).cloned().expect("package should have been created");
        let latest = packages.dist_tags.lock().unwrap().get(&(package.id, "latest".to_string())).cloned();
        assert_eq!(latest, Some(version), "publishing a package's first (stable) version must set the `latest` dist-tag to it");
    }

    /// Storage keys are content-derived AND unique per publish attempt (Bug 4b, fix rounds 1-3) —
    /// `{scope/name}/-/{version}-{shasum}-{attempt-uuid}.tgz`. The package name appears ONLY in the
    /// directory prefix, not in the filename itself (fix round 3: repeating it in the filename ate
    /// into the filesystem's NAME_MAX for no benefit). The trailing UUID is generated fresh inside
    /// `execute` on every call and can't be predicted from the outside, so tests can only assert the
    /// shasum-bearing PREFIX this way; the actual persisted key is read back off the fake
    /// repository's own version record (the equivalent of a DB row) rather than reconstructed
    /// independently — see the review of fix round 1, which found that independently reconstructing
    /// the full key is a weaker proof, since it can pass even when production and the test happen to
    /// compute the same (wrong) key.
    fn expected_tgz_name_prefix(version: &str, tarball: &[u8]) -> String {
        format!("{version}-{}-", hex::encode(Sha1::digest(tarball)))
    }

    #[tokio::test]
    async fn each_version_gets_its_own_storage_key_derived_from_name_and_version() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let events = Arc::new(FakeEvents::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), storage.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), events);
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let v1 = NpmVersion::parse("1.0.0").unwrap();
        let v2 = NpmVersion::parse("2.0.0").unwrap();

        use_case.execute(repository_id, &name, &v1, serde_json::json!({}), Bytes::from_static(b"v1-bytes"), &[], Uuid::new_v4()).await.unwrap();
        use_case.execute(repository_id, &name, &v2, serde_json::json!({}), Bytes::from_static(b"v2-bytes"), &[], Uuid::new_v4()).await.unwrap();

        let package = packages.packages.lock().unwrap().get(&(repository_id, name.as_str().to_string())).cloned().expect("package should have been created");
        let key1 = packages.versions.lock().unwrap().get(&(package.id, "1.0.0".to_string())).unwrap().tarball_storage_key.clone();
        let key2 = packages.versions.lock().unwrap().get(&(package.id, "2.0.0".to_string())).unwrap().tarball_storage_key.clone();

        assert_ne!(key1, key2, "each version must get its own storage key");
        assert!(
            key1.starts_with(&format!("left-pad/-/{}", expected_tgz_name_prefix("1.0.0", b"v1-bytes"))) && key1.ends_with(".tgz"),
            "v1's storage key must be content-derived from its own bytes, got {key1}"
        );
        assert!(
            key2.starts_with(&format!("left-pad/-/{}", expected_tgz_name_prefix("2.0.0", b"v2-bytes"))) && key2.ends_with(".tgz"),
            "v2's storage key must be content-derived from its own bytes, got {key2}"
        );
        assert_eq!(storage.read(repository_id, &key1).await.unwrap(), b"v1-bytes");
        assert_eq!(storage.read(repository_id, &key2).await.unwrap(), b"v2-bytes");
    }

    #[tokio::test]
    async fn a_scoped_packages_storage_key_uses_only_the_local_name() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let events = Arc::new(FakeEvents::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), storage.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), events);
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("@myscope/mypkg").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();

        use_case.execute(repository_id, &name, &version, serde_json::json!({}), Bytes::from_static(b"bytes"), &[], Uuid::new_v4()).await.unwrap();

        let package = packages.packages.lock().unwrap().get(&(repository_id, name.as_str().to_string())).cloned().expect("package should have been created");
        let key = packages.versions.lock().unwrap().get(&(package.id, "1.0.0".to_string())).unwrap().tarball_storage_key.clone();

        assert!(
            key.starts_with(&format!("myscope/mypkg/-/{}", expected_tgz_name_prefix("1.0.0", b"bytes"))) && key.ends_with(".tgz"),
            "a scoped package's storage key must use only the local name in its path prefix, got {key}"
        );
        assert_eq!(storage.read(repository_id, &key).await.unwrap(), b"bytes");
    }

    /// Bug 4b, fix round 3 regression test: a maximum-length npm package name must still be
    /// publishable now that the filename no longer repeats the package name (only the directory
    /// prefix does). Uses a real `FilesystemStorageBackend` against a tempdir — the bug was a raw
    /// filesystem NAME_MAX error, which a `FakeStorage` (an in-memory `HashMap`) can't reproduce.
    ///
    /// Boundary math (all figures count bytes; version "1.0.0", a 40-hex-char SHA-1 shasum and a
    /// 36-character UUID `attempt_id`, all ASCII):
    ///   OLD filename `{name}-{version}-{shasum}-{attempt_id}.tgz`, at the 214-character maximum
    ///   npm package-name length: 214 + 1 + 5 + 1 + 40 + 1 + 36 + 4 = 302 bytes — over the
    ///   255-byte NAME_MAX that APFS, ext4 and overlayfs all enforce per path component, so this
    ///   used to fail with a raw "File name too long" I/O error before the version row was ever
    ///   inserted.
    ///   NEW filename `{version}-{shasum}-{attempt_id}.tgz`: 5 + 1 + 40 + 1 + 36 + 4 = 87 bytes,
    ///   independent of the package name — comfortably under NAME_MAX regardless of how long the
    ///   name is. The name itself only appears in the directory-prefix path component (214 bytes
    ///   here, itself well under NAME_MAX).
    #[tokio::test]
    async fn publishing_a_package_name_at_the_npm_length_limit_succeeds() {
        let name_raw = "a".repeat(214);
        assert_eq!(name_raw.len(), 214, "sanity check: npm's own maximum package-name length");
        let name = NpmPackageName::parse(&name_raw).expect("214 lowercase-letter characters is a valid, maximum-length npm package name");

        let storage_root = tempfile::tempdir().unwrap();
        let storage: Arc<dyn StorageBackendPort> =
            Arc::new(artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend::new(storage_root.path().to_path_buf()));
        let packages = Arc::new(FakePackages::new());
        let events = Arc::new(FakeEvents::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), storage.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), events);
        let repository_id = Uuid::new_v4();
        let version = NpmVersion::parse("1.0.0").unwrap();

        let result = use_case.execute(repository_id, &name, &version, serde_json::json!({}), Bytes::from_static(b"tarball-bytes"), &[], Uuid::new_v4()).await;

        assert!(result.is_ok(), "a maximum-length npm package name must still be publishable, got {result:?}");
        let package = packages.packages.lock().unwrap().get(&(repository_id, name.as_str().to_string())).cloned().expect("package should have been created");
        let key = packages.versions.lock().unwrap().get(&(package.id, "1.0.0".to_string())).unwrap().tarball_storage_key.clone();
        assert_eq!(
            storage.read(repository_id, &key).await.unwrap(),
            b"tarball-bytes",
            "the tarball must actually be readable back from disk at the persisted key, not just accepted in memory"
        );
    }

    #[tokio::test]
    async fn a_version_that_differs_only_in_build_metadata_is_the_same_release() {
        let use_case = use_case();
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let publisher = Uuid::new_v4();
        let publish = |version: &str| {
            let version = NpmVersion::parse(version).unwrap();
            let use_case = &use_case;
            let name = &name;
            async move { use_case.execute(repository_id, name, &version, serde_json::json!({}), Bytes::from_static(b"tarball-bytes"), &[], publisher).await }
        };

        publish("1.0.0").await.unwrap();

        assert!(matches!(publish("1.0.0+other-bytes").await, Err(ApplicationError::PackageVersionExists)));
    }

    #[tokio::test]
    async fn an_unpublished_version_cannot_come_back_under_other_build_metadata() {
        let packages = Arc::new(FakePackages::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), Arc::new(FakeStorage::new()), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let publisher = Uuid::new_v4();
        let removed = NpmVersion::parse("1.0.0").unwrap();
        packages.unpublished.lock().unwrap().insert((repository_id, name.as_str().to_string(), removed.as_str()));

        let result = use_case.execute(repository_id, &name, &NpmVersion::parse("1.0.0+b1").unwrap(), serde_json::json!({}), Bytes::from_static(b"tarball-bytes"), &[], publisher).await;

        assert!(matches!(result, Err(ApplicationError::PackageVersionExists)), "{result:?}");
    }

    #[tokio::test]
    async fn a_package_at_the_version_limit_takes_no_more_and_says_so() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), storage, Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let publisher = Uuid::new_v4();
        use_case.execute(repository_id, &name, &NpmVersion::parse("0.0.1").unwrap(), serde_json::json!({}), Bytes::from_static(b"tarball-bytes"), &[], publisher).await.unwrap();
        let package = packages.packages.lock().unwrap().get(&(repository_id, name.as_str().to_string())).cloned().unwrap();
        let template = packages.versions.lock().unwrap().values().next().cloned().unwrap();
        for patch in 1..artiferris_domain::npm_package::MAX_VERSIONS_PER_PACKAGE {
            let mut filler = template.clone();
            filler.id = Uuid::new_v4();
            filler.version = NpmVersion::parse(&format!("1.0.{patch}")).unwrap();
            packages.versions.lock().unwrap().insert((package.id, filler.version.as_str()), filler);
        }

        let result = use_case.execute(repository_id, &name, &NpmVersion::parse("2.0.0").unwrap(), serde_json::json!({}), Bytes::from_static(b"tarball-bytes"), &[], publisher).await;

        assert!(matches!(result, Err(ApplicationError::InvalidNpmPayload(_))), "{result:?}");
    }

    #[tokio::test]
    async fn republishing_the_same_version_is_rejected() {
        let use_case = use_case();
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        let publisher = Uuid::new_v4();
        use_case.execute(repository_id, &name, &version, serde_json::json!({}), Bytes::from_static(b"tarball-bytes"), &[], publisher).await.unwrap();
        let result = use_case.execute(repository_id, &name, &version, serde_json::json!({}), Bytes::from_static(b"other-bytes"), &[], publisher).await;
        assert!(matches!(result, Err(ApplicationError::PackageVersionExists)));
    }

    #[tokio::test]
    async fn publishing_a_prerelease_version_does_not_move_latest() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let events = Arc::new(FakeEvents::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), storage.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), events.clone());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let stable = NpmVersion::parse("1.0.0").unwrap();
        let prerelease = NpmVersion::parse("2.0.0-beta.1").unwrap();
        let publisher = Uuid::new_v4();
        use_case.execute(repository_id, &name, &stable, serde_json::json!({}), Bytes::from_static(b"a"), &[], publisher).await.unwrap();
        use_case.execute(repository_id, &name, &prerelease, serde_json::json!({}), Bytes::from_static(b"b"), &[], publisher).await.unwrap();

        let package = packages.packages.lock().unwrap().get(&(repository_id, name.as_str().to_string())).cloned().expect("package should have been created");
        let latest = packages.dist_tags.lock().unwrap().get(&(package.id, "latest".to_string())).cloned();
        assert_eq!(latest, Some(stable), "publishing a prerelease version must not move the `latest` dist-tag");
    }

    fn repo_with_quota(id: Uuid, quota_bytes: Option<i64>) -> artiferris_domain::package_repository::PackageRepositorySummary {
        artiferris_domain::package_repository::PackageRepositorySummary {
            id,
            organization_id: Uuid::new_v4(),
            name: "left-pad".to_string(),
            format: artiferris_domain::package_repository::RepositoryFormat::Npm,
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
    async fn publishing_a_tarball_larger_than_the_quota_is_rejected() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repositories = Arc::new(FakeRepositories::new());
        let events = Arc::new(FakeEvents::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(repo_with_quota(repository_id, Some(5)));
        let use_case = PublishNpmPackageUseCase::new(packages, storage, repositories, Arc::new(FakeRepositoryQuotaLock::new()), events);
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();

        let result = use_case.execute(repository_id, &name, &version, serde_json::json!({}), Bytes::from_static(b"0123456789"), &[], Uuid::new_v4()).await;

        assert!(matches!(result, Err(ApplicationError::StorageQuotaExceeded)), "got {result:?}");
    }

    #[tokio::test]
    async fn publishing_within_the_quota_succeeds() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repositories = Arc::new(FakeRepositories::new());
        let events = Arc::new(FakeEvents::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(repo_with_quota(repository_id, Some(1024)));
        let use_case = PublishNpmPackageUseCase::new(packages, storage, repositories, Arc::new(FakeRepositoryQuotaLock::new()), events);
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();

        let result = use_case.execute(repository_id, &name, &version, serde_json::json!({}), Bytes::from_static(b"0123456789"), &[], Uuid::new_v4()).await;

        assert!(result.is_ok(), "got {result:?}");
    }

    #[tokio::test]
    async fn an_unset_quota_never_blocks_a_publish() {
        let use_case = use_case();
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();

        // No repository was ever inserted into the fake — find_by_id returns
        // None, same as an unlimited repository would.
        let result = use_case.execute(repository_id, &name, &version, serde_json::json!({}), Bytes::from_static(b"0123456789"), &[], Uuid::new_v4()).await;

        assert!(result.is_ok(), "got {result:?}");
    }

    /// Fakes can't model this: the race is between two real, independent `execute` calls, each
    /// with its own view of `used_bytes` (a real directory scan). Races two publishes of
    /// different versions to the SAME tightly-quota'd repository — each under quota alone,
    /// together over it — through the real Postgres-backed lock and a `FilesystemStorageBackend`
    /// shared across both tasks. Before the fix, both could read `used_bytes` before either had
    /// written its tarball, both pass, and both write.
    ///
    /// This is a real-concurrency smoke test, kept alongside the deterministic test below (which
    /// is what actually proves the lock blocks) — see that test's doc comment for why this one
    /// alone isn't enough: it relies on OS thread scheduling for the interleaving, and mutation
    /// testing showed that luck overwhelmingly favors NOT catching a broken-but-present lock.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_concurrent_publishes_that_together_exceed_the_quota_do_not_both_succeed(pool: sqlx::PgPool) {
        let repository_id = Uuid::new_v4();
        let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, quota_bytes, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'npm', 'hosted', NULL, $4, 1, now(), now())",
            repository_id,
            organization_id,
            format!("race-repo-{repository_id}"),
            30i64,
        )
        .execute(&pool)
        .await
        .unwrap();

        let storage_root = tempfile::tempdir().unwrap();
        let storage: Arc<dyn StorageBackendPort> =
            Arc::new(artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend::new(storage_root.path().to_path_buf()));
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version_a = NpmVersion::parse("1.0.0").unwrap();
        let version_b = NpmVersion::parse("2.0.0").unwrap();

        // Two 20-byte tarballs — individually well under the 30-byte quota, together over it.
        let tarball_a = Bytes::from_static(b"aaaaaaaaaaaaaaaaaaaa");
        let tarball_b = Bytes::from_static(b"bbbbbbbbbbbbbbbbbbbb");

        // Same reasoning as Docker's `two_concurrent_pushes_that_together_exceed_the_quota_do_not_both_succeed`
        // (`docker_manifest_put.rs`): `#[sqlx::test]` drives this whole test on a single-threaded
        // current-thread Tokio runtime, and on localhost every query round trip resolves fast enough that a
        // plain `tokio::join!` never actually interleaves the two publishes. `spawn_blocking` puts each
        // publish on its own real OS thread for genuine preemptive concurrency, while
        // `Handle::current().block_on` keeps both on the SAME runtime as `pool` — a `PgPool` isn't safe to
        // drive from a second, unrelated runtime.
        let run_publish = |pool: sqlx::PgPool, storage: Arc<dyn StorageBackendPort>, version: NpmVersion, tarball: Bytes| {
            let name = name.clone();
            let handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                handle.block_on(async move {
                    let repositories = Arc::new(artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore::new(
                        pool.clone(),
                        "test-secret".to_string(),
                    ));
                    let packages = Arc::new(FakePackages::new());
                    let events = Arc::new(FakeEvents::new());
                    // Same instance for BOTH `repositories` and `quota_lock` — it implements both
                    // ports, and this is the whole point of the test: it needs the REAL lock.
                    let use_case = PublishNpmPackageUseCase::new(packages, storage, repositories.clone(), repositories, events);
                    use_case.execute(repository_id, &name, &version, serde_json::json!({}), tarball, &[], Uuid::new_v4()).await
                })
            })
        };

        let handle_a = run_publish(pool.clone(), storage.clone(), version_a, tarball_a);
        let handle_b = run_publish(pool, storage, version_b, tarball_b);
        let result_a = handle_a.await.unwrap();
        let result_b = handle_b.await.unwrap();

        let successes = [&result_a, &result_b].into_iter().filter(|r| r.is_ok()).count();
        assert_eq!(successes, 1, "exactly one of two concurrent publishes that together exceed the quota must succeed — got a={result_a:?} b={result_b:?}");
        let rejected = if result_a.is_err() { &result_a } else { &result_b };
        assert!(matches!(rejected, Err(ApplicationError::StorageQuotaExceeded)), "the rejected publish must fail with StorageQuotaExceeded, got {rejected:?}");
    }

    /// Test-only decorator that pauses a `write()` call mid-flight, under program control, so a
    /// test can drive the exact interleaving of two racing publishes instead of hoping OS thread
    /// scheduling produces it. `write()` signals `entered_write` (execution has reached the
    /// storage layer — i.e. after the caller already acquired the lock and passed its own quota
    /// recheck) and then waits on `release_write` before delegating to the real backend.
    ///
    /// Also records the `path` (the storage key `execute` actually derived and attempted to write)
    /// into `written_key`, so a test can learn a LOSING publish's real, randomly-suffixed key by
    /// observing what production code actually did — not by independently re-deriving the key
    /// formula (Bug 4b, fix round 1 review, Issue 2: re-deriving the key is weaker proof than
    /// reading back what was actually used).
    struct PausingStorage {
        inner: Arc<dyn StorageBackendPort>,
        entered_write: Arc<tokio::sync::Notify>,
        release_write: Arc<tokio::sync::Notify>,
        written_key: Arc<std::sync::Mutex<Option<String>>>,
    }

    impl PausingStorage {
        fn new(inner: Arc<dyn StorageBackendPort>, entered_write: Arc<tokio::sync::Notify>, release_write: Arc<tokio::sync::Notify>) -> Self {
            Self { inner, entered_write, release_write, written_key: Arc::new(std::sync::Mutex::new(None)) }
        }
    }

    #[async_trait::async_trait]
    impl StorageBackendPort for PausingStorage {
        async fn write(&self, repository_id: Uuid, path: &str, data: &[u8]) -> Result<(), artiferris_domain::storage::StorageError> {
            *self.written_key.lock().unwrap() = Some(path.to_string());
            self.entered_write.notify_one();
            self.release_write.notified().await;
            self.inner.write(repository_id, path, data).await
        }
        async fn read(&self, repository_id: Uuid, path: &str) -> Result<Vec<u8>, artiferris_domain::storage::StorageError> {
            self.inner.read(repository_id, path).await
        }
        async fn read_stream(&self, repository_id: Uuid, path: &str) -> Result<artiferris_domain::storage::ByteStream, artiferris_domain::storage::StorageError> {
            self.inner.read_stream(repository_id, path).await
        }
        async fn delete(&self, repository_id: Uuid, path: &str) -> Result<(), artiferris_domain::storage::StorageError> {
            self.inner.delete(repository_id, path).await
        }
        async fn delete_repository(&self, repository_id: Uuid) -> Result<(), artiferris_domain::storage::StorageError> {
            self.inner.delete_repository(repository_id).await
        }
        async fn used_bytes(&self, repository_id: Uuid) -> Result<u64, artiferris_domain::storage::StorageError> {
            self.inner.used_bytes(repository_id).await
        }
        async fn is_healthy(&self) -> bool {
            self.inner.is_healthy().await
        }
        async fn volume_space(&self) -> Result<artiferris_domain::storage::VolumeSpace, artiferris_domain::storage::StorageError> {
            self.inner.volume_space().await
        }
    }

    /// Deterministic companion to the timing-based test above (opus fix-review,
    /// bug2-fix-review.md section 4): that test relies on real OS-thread scheduling to interleave
    /// two racing publishes, and mutation testing showed the luck goes the wrong way most of the
    /// time — it misses a broken lock query (4% catch rate) and an early-released lock (0%).
    ///
    /// This test drives the interleaving directly instead of hoping for it: publish A is paused
    /// inside `write()` (having already acquired the lock and passed its own quota recheck), and
    /// the test asserts — by polling `pg_stat_activity` for a lock wait, the same technique Bug
    /// 1's `a_concurrent_link_landing_between_the_guard_and_the_delete_does_not_lose_the_decrement`
    /// uses (`filesystem_docker_blob_store.rs`), not a sleep-and-hope — that publish B is
    /// genuinely BLOCKED trying to acquire the same advisory lock, before letting A finish and
    /// checking that B is then rejected for quota.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_publish_blocked_on_the_quota_lock_is_rejected_once_the_holder_writes_over_quota(pool: sqlx::PgPool) {
        let repository_id = Uuid::new_v4();
        let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, quota_bytes, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'npm', 'hosted', NULL, $4, 1, now(), now())",
            repository_id,
            organization_id,
            format!("blocking-race-repo-{repository_id}"),
            30i64,
        )
        .execute(&pool)
        .await
        .unwrap();

        let storage_root = tempfile::tempdir().unwrap();
        let real_storage: Arc<dyn StorageBackendPort> =
            Arc::new(artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend::new(storage_root.path().to_path_buf()));
        let entered_write = Arc::new(tokio::sync::Notify::new());
        let release_write = Arc::new(tokio::sync::Notify::new());
        let paused_storage: Arc<dyn StorageBackendPort> =
            Arc::new(PausingStorage::new(real_storage.clone(), entered_write.clone(), release_write.clone()));

        let name = NpmPackageName::parse("left-pad").unwrap();
        let version_a = NpmVersion::parse("1.0.0").unwrap();
        let version_b = NpmVersion::parse("2.0.0").unwrap();
        // 20 bytes each: individually under the 30-byte quota, together over it.
        let tarball_a = Bytes::from_static(b"aaaaaaaaaaaaaaaaaaaa");
        let tarball_b = Bytes::from_static(b"bbbbbbbbbbbbbbbbbbbb");

        let repositories_a = Arc::new(artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore::new(
            pool.clone(),
            "test-secret".to_string(),
        ));
        let use_case_a = PublishNpmPackageUseCase::new(Arc::new(FakePackages::new()), paused_storage, repositories_a.clone(), repositories_a, Arc::new(FakeEvents::new()));
        let name_a = name.clone();
        let handle_a = tokio::spawn(async move { use_case_a.execute(repository_id, &name_a, &version_a, serde_json::json!({}), tarball_a, &[], Uuid::new_v4()).await });

        // Wait until A is paused inside write() — it has already acquired the lock and passed its
        // own quota recheck. Everything B does from here races the lock, not the quota read.
        entered_write.notified().await;

        let repositories_b = Arc::new(artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore::new(
            pool.clone(),
            "test-secret".to_string(),
        ));
        let use_case_b = PublishNpmPackageUseCase::new(Arc::new(FakePackages::new()), real_storage, repositories_b.clone(), repositories_b, Arc::new(FakeEvents::new()));
        let name_b = name.clone();
        let handle_b = tokio::spawn(async move { use_case_b.execute(repository_id, &name_b, &version_b, serde_json::json!({}), tarball_b, &[], Uuid::new_v4()).await });

        // Poll pg_stat_activity instead of sleeping-and-hoping, so the test proves B is genuinely
        // waiting on the lock, not merely "hasn't finished yet".
        let mut observed_blocked = false;
        for _ in 0..500 {
            let blocked: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query ILIKE '%pg_advisory_xact_lock%'",
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
        assert!(observed_blocked, "publish B never entered a lock wait on pg_advisory_xact_lock — this test isn't exercising the blocking it claims to");

        // Only now let A finish. If B were merely racing rather than genuinely blocked, it could
        // already have read a stale `used_bytes` and be past its own check by this point.
        release_write.notify_one();

        let result_a = handle_a.await.unwrap();
        let result_b = handle_b.await.unwrap();

        assert!(result_a.is_ok(), "publish A, which already held the lock and passed its own recheck, must succeed, got {result_a:?}");
        assert!(matches!(result_b, Err(ApplicationError::StorageQuotaExceeded)), "publish B, unblocked only after A's write landed, must be rejected for quota, got {result_b:?}");
    }

    /// Test helper: sets up a repository + publisher user row and returns the pieces every
    /// concurrent-publish test below needs — kept in one place so the two race tests (different
    /// bytes, identical bytes) stay focused on the interleaving and assertions that differ between
    /// them, not on repeated boilerplate.
    async fn setup_version_race_fixtures(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
        let repository_id = Uuid::new_v4();
        let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, quota_bytes, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'npm', 'hosted', NULL, NULL, 1, now(), now())",
            repository_id,
            organization_id,
            format!("version-race-repo-{repository_id}"),
        )
        .execute(pool)
        .await
        .unwrap();

        // `insert_version` sets `published_by`, which has an FK to `users` — needs a real row.
        let publisher_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, password_hash, organization_id) VALUES ($1, $2, 'x', $3)",
            publisher_id,
            format!("publisher-{publisher_id}"),
            organization_id,
        )
        .execute(pool)
        .await
        .unwrap();

        (repository_id, publisher_id)
    }

    /// Regression test for the same-version publish integrity race (batch-2 follow-up
    /// investigation, found in passing while investigating Docker's B-18 test), fix round 1: the
    /// advisory lock this file used to test here was removed (it starved the connection pool under
    /// concurrent load — see `npm_publish.rs::execute`'s storage-key comment) in favor of making the
    /// storage key content-derived, so two concurrent publishes of the SAME not-yet-existing version
    /// with DIFFERENT tarball bytes can never write to the same path. Simpler than the old lock-based
    /// test: there's no lock to prove is held, so no `pg_stat_activity` polling — the existing unique
    /// `(npm_package_id, version)` constraint on `insert_version` is what resolves the race, and this
    /// test only needs to force the two `execute` calls to genuinely overlap.
    ///
    /// Publish A is paused inside `write()` (having already passed its own early `find_version`
    /// check, same as B). While A is paused, B runs to completion for real against the real Postgres
    /// pool and real filesystem storage — B's own early check also still sees no row, so this is a
    /// genuine concurrent race, not a sequential retry. Releasing A then lets it write its own
    /// (different) key and hit `insert_version`, which now loses to B's already-committed row.
    ///
    /// Strengthened (Bug 4b, fix round 2, per the round-1 review's Issue 2): the winner's storage key
    /// is now read back off the persisted DB row (`row.tarball_storage_key`) instead of being
    /// independently re-derived from the same formula production uses — re-deriving is weaker proof,
    /// since it can pass even when production and the test happen to compute the same (wrong) key.
    /// The loser's key is likewise learned from `PausingStorage`'s `written_key` (what production
    /// actually attempted to write), not recomputed either.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_concurrent_publishes_of_the_same_new_version_with_different_bytes_do_not_corrupt_each_others_state(pool: sqlx::PgPool) {
        let (repository_id, publisher_id) = setup_version_race_fixtures(&pool).await;

        let storage_root = tempfile::tempdir().unwrap();
        let real_storage: Arc<dyn StorageBackendPort> =
            Arc::new(artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend::new(storage_root.path().to_path_buf()));
        let entered_write = Arc::new(tokio::sync::Notify::new());
        let release_write = Arc::new(tokio::sync::Notify::new());
        let pausing_storage = Arc::new(PausingStorage::new(real_storage.clone(), entered_write.clone(), release_write.clone()));
        let written_key_a = pausing_storage.written_key.clone();
        let paused_storage: Arc<dyn StorageBackendPort> = pausing_storage;

        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        // Deliberately different content (and length) — under the OLD deterministic storage key,
        // whichever write landed last would win the file on disk regardless of which insert won.
        let tarball_a = Bytes::from_static(b"tarball-content-from-publisher-A");
        let tarball_b = Bytes::from_static(b"totally-different-bytes-from-publisher-B-here");

        let packages_a = Arc::new(artiferris_infrastructure::postgres::npm_package_repository::PostgresNpmPackageRepository::new(pool.clone()));
        let use_case_a =
            PublishNpmPackageUseCase::new(packages_a, paused_storage, Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let name_a = name.clone();
        let version_a = version.clone();
        let tarball_a_for_task = tarball_a.clone();
        let handle_a =
            tokio::spawn(async move { use_case_a.execute(repository_id, &name_a, &version_a, serde_json::json!({"from": "A"}), tarball_a_for_task, &[], publisher_id).await });

        // Wait until A is paused inside write() — it has already passed its own early find_version
        // check, but hasn't written or inserted anything yet.
        entered_write.notified().await;

        let packages_b = Arc::new(artiferris_infrastructure::postgres::npm_package_repository::PostgresNpmPackageRepository::new(pool.clone()));
        let use_case_b = PublishNpmPackageUseCase::new(
            packages_b,
            real_storage.clone(),
            Arc::new(FakeRepositories::new()),
            Arc::new(FakeRepositoryQuotaLock::new()),
            Arc::new(FakeEvents::new()),
        );
        // B runs to full completion here, for real, while A is still paused mid-write — a genuine
        // concurrent overlap between the two `execute` calls, not a sequential retry.
        let result_b = use_case_b.execute(repository_id, &name, &version, serde_json::json!({"from": "B"}), tarball_b.clone(), &[], publisher_id).await;
        assert!(result_b.is_ok(), "publish B, racing A with no lock in its way, must succeed, got {result_b:?}");

        // Only now let A proceed. It writes to its own (different) key, then loses at the
        // `insert_version` unique constraint against the row B already committed.
        release_write.notify_one();
        let result_a = handle_a.await.unwrap();
        assert!(
            matches!(result_a, Err(ApplicationError::PackageVersionExists)),
            "publish A, unblocked only after B committed its version row, must be rejected as already existing, got {result_a:?}"
        );

        // Read the winner's storage key back off the PERSISTED ROW — not re-derived from the key
        // formula — and only then use it to check what's actually on disk and in the row together.
        let expected_shasum = hex::encode(Sha1::digest(&tarball_b));
        let row = sqlx::query!(
            "SELECT npv.shasum, npv.tarball_size_bytes, npv.tarball_storage_key FROM npm_package_versions npv \
             JOIN npm_packages np ON np.id = npv.npm_package_id \
             WHERE np.package_repository_id = $1 AND npv.version = '1.0.0'",
            repository_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.shasum, expected_shasum, "the persisted shasum must match publish B's actual content");
        assert_eq!(row.tarball_size_bytes as usize, tarball_b.len(), "the persisted size must match the bytes actually on disk");

        // The critical assertion: the bytes actually on disk AT THE ROW'S OWN KEY must be publish
        // B's content — the only publish that actually won and was persisted — never some
        // interleaving of A's and B's, and never one's content paired with the other's metadata.
        let on_disk = real_storage.read(repository_id, &row.tarball_storage_key).await.unwrap();
        assert_eq!(on_disk, tarball_b.to_vec(), "the bytes on disk at the winner's own persisted key must be B's content");

        // The loser's file must not linger: best-effort cleanup after the losing insert_version.
        // Its key comes from what production actually wrote (`PausingStorage`), not a re-derived
        // formula, and must differ from the winner's persisted key.
        let key_a = written_key_a.lock().unwrap().clone().expect("publish A must have reached write() before being rejected");
        assert_ne!(key_a, row.tarball_storage_key, "sanity check: the two different-bytes publishes must land on different storage keys");
        let loser_still_on_disk = real_storage.read(repository_id, &key_a).await;
        assert!(loser_still_on_disk.is_err(), "publish A's orphaned tarball must have been cleaned up after it lost the race, found: {loser_still_on_disk:?}");
    }

    /// Regression test for the IDENTICAL-bytes collision (Bug 4b, fix round 2). Fix round 1's
    /// content-derived key (name+version+shasum, no random component) closed the DIFFERENT-bytes
    /// race exercised by the test above, but opened a narrower one: two concurrent publishes of the
    /// SAME not-yet-existing version with IDENTICAL tarball bytes hash to the SAME shasum and
    /// therefore the SAME storage key. This is realistic, not just theoretical — `npm pack` output
    /// is deterministic for a given commit, so a double-triggered CI run, or an npm client retrying a
    /// slow/timed-out upload, produces byte-identical tarballs.
    ///
    /// Under a purely shasum-derived key, both writes land on the SAME path (harmless by itself,
    /// since the bytes are identical), but the LOSER's best-effort cleanup then deletes "its own"
    /// key — which is ALSO the winner's key, since they collided — leaving the winning row pointing
    /// at a now-missing file. Every future install of that version 500s, and republishing is refused
    /// as a duplicate; the only way out is to unpublish. Fixed by folding a fresh random `attempt_id`
    /// (a `Uuid::new_v4()` generated on every call to `execute`) into the key, so identical bytes no
    /// longer imply identical keys: the loser's cleanup then only ever deletes ITS OWN,
    /// genuinely-unique file, never the winner's.
    ///
    /// Same interleaving technique as the different-bytes test above — A paused mid-`write()`, B run
    /// to full completion, then A released — but both tarballs are byte-for-byte identical here,
    /// which is exactly the scenario that test cannot exercise.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_concurrent_publishes_of_the_same_new_version_with_identical_bytes_do_not_corrupt_each_others_state(pool: sqlx::PgPool) {
        let (repository_id, publisher_id) = setup_version_race_fixtures(&pool).await;

        let storage_root = tempfile::tempdir().unwrap();
        let real_storage: Arc<dyn StorageBackendPort> =
            Arc::new(artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend::new(storage_root.path().to_path_buf()));
        let entered_write = Arc::new(tokio::sync::Notify::new());
        let release_write = Arc::new(tokio::sync::Notify::new());
        let pausing_storage = Arc::new(PausingStorage::new(real_storage.clone(), entered_write.clone(), release_write.clone()));
        let written_key_a = pausing_storage.written_key.clone();
        let paused_storage: Arc<dyn StorageBackendPort> = pausing_storage;

        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        // Byte-for-byte IDENTICAL for both publishers — the whole point of this test. A real-world
        // trigger: `npm pack` is deterministic for a given commit, so a double-triggered CI run
        // publishing the same commit twice concurrently produces exactly this.
        let tarball = Bytes::from_static(b"identical-tarball-bytes-raced-by-two-publishers");

        let packages_a = Arc::new(artiferris_infrastructure::postgres::npm_package_repository::PostgresNpmPackageRepository::new(pool.clone()));
        let use_case_a =
            PublishNpmPackageUseCase::new(packages_a, paused_storage, Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let name_a = name.clone();
        let version_a = version.clone();
        let tarball_a_for_task = tarball.clone();
        let handle_a =
            tokio::spawn(async move { use_case_a.execute(repository_id, &name_a, &version_a, serde_json::json!({"from": "A"}), tarball_a_for_task, &[], publisher_id).await });

        // Wait until A is paused inside write() — it has already passed its own early find_version
        // check, but hasn't written or inserted anything yet.
        entered_write.notified().await;

        let packages_b = Arc::new(artiferris_infrastructure::postgres::npm_package_repository::PostgresNpmPackageRepository::new(pool.clone()));
        let use_case_b = PublishNpmPackageUseCase::new(
            packages_b,
            real_storage.clone(),
            Arc::new(FakeRepositories::new()),
            Arc::new(FakeRepositoryQuotaLock::new()),
            Arc::new(FakeEvents::new()),
        );
        // B runs to full completion here, for real, with the SAME bytes A is about to write, while A
        // is still paused mid-write — a genuine concurrent overlap, not a sequential retry.
        let result_b = use_case_b.execute(repository_id, &name, &version, serde_json::json!({"from": "B"}), tarball.clone(), &[], publisher_id).await;
        assert!(result_b.is_ok(), "publish B, racing A with identical bytes, must succeed, got {result_b:?}");

        // Only now let A proceed. It writes to its own key and then loses at the `insert_version`
        // unique constraint against the row B already committed.
        release_write.notify_one();
        let result_a = handle_a.await.unwrap();
        assert!(
            matches!(result_a, Err(ApplicationError::PackageVersionExists)),
            "publish A, unblocked only after B committed its version row, must be rejected as already existing, got {result_a:?}"
        );

        // Read the winner's storage key back off the PERSISTED ROW.
        let expected_shasum = hex::encode(Sha1::digest(&tarball));
        let row = sqlx::query!(
            "SELECT npv.shasum, npv.tarball_size_bytes, npv.tarball_storage_key FROM npm_package_versions npv \
             JOIN npm_packages np ON np.id = npv.npm_package_id \
             WHERE np.package_repository_id = $1 AND npv.version = '1.0.0'",
            repository_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.shasum, expected_shasum, "the persisted shasum must match the (shared) tarball content");
        assert_eq!(row.tarball_size_bytes as usize, tarball.len(), "the persisted size must match the bytes actually on disk");

        // The critical assertion, and the one that fails under the pre-fix (shasum-only) key: the
        // winner's own file, at the winner's own persisted key, must still be there and intact — NOT
        // deleted by the loser's cleanup, which would happen if the two keys collided.
        let on_disk = real_storage.read(repository_id, &row.tarball_storage_key).await.unwrap();
        assert_eq!(on_disk, tarball.to_vec(), "the winner's file must survive the loser's cleanup — it must NOT have been deleted");

        // The loser's own key (what it actually attempted to write, captured by the test decorator)
        // must be DIFFERENT from the winner's persisted key, despite byte-for-byte identical content
        // — that's the whole point of the per-attempt random component.
        let key_a = written_key_a.lock().unwrap().clone().expect("publish A must have reached write() before being rejected");
        assert_ne!(
            key_a, row.tarball_storage_key,
            "publish A and publish B wrote IDENTICAL bytes but must still land on DIFFERENT storage keys, or the collision this test guards against is back"
        );

        // The loser's own (genuinely distinct) file must have been cleaned up — and, since it's a
        // different path than the winner's, this cleanup cannot have touched the winner's file
        // (already confirmed present and correct above).
        let loser_still_on_disk = real_storage.read(repository_id, &key_a).await;
        assert!(loser_still_on_disk.is_err(), "publish A's orphaned tarball must have been cleaned up after it lost the race, found: {loser_still_on_disk:?}");
    }

    fn tarball_files_under(root: &std::path::Path) -> usize {
        std::fs::read_dir(root).unwrap().flatten().map(|entry| if entry.path().is_dir() { tarball_files_under(&entry.path()) } else { usize::from(entry.path().extension().is_some_and(|ext| ext == "tgz")) }).sum()
    }

    /// The package this publish found is deleted, by unpublishing its only version, while the publish is still writing its tarball.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_publish_survives_its_package_being_unpublished_out_from_under_it_and_leaks_no_tarball(pool: sqlx::PgPool) {
        use crate::use_cases::npm_unpublish::UnpublishNpmPackageUseCase;
        use artiferris_infrastructure::postgres::npm_package_repository::PostgresNpmPackageRepository;

        let (repository_id, publisher_id) = setup_version_race_fixtures(&pool).await;
        let storage_root = tempfile::tempdir().unwrap();
        let real_storage: Arc<dyn StorageBackendPort> = Arc::new(artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend::new(storage_root.path().to_path_buf()));
        let packages = Arc::new(PostgresNpmPackageRepository::new(pool.clone()));
        let name = NpmPackageName::parse("left-pad").unwrap();
        let publish_first = PublishNpmPackageUseCase::new(packages.clone(), real_storage.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        publish_first.execute(repository_id, &name, &NpmVersion::parse("1.0.0").unwrap(), serde_json::json!({}), Bytes::from_static(b"first"), &[], publisher_id).await.unwrap();

        let entered_write = Arc::new(tokio::sync::Notify::new());
        let release_write = Arc::new(tokio::sync::Notify::new());
        let pausing_storage: Arc<dyn StorageBackendPort> = Arc::new(PausingStorage::new(real_storage.clone(), entered_write.clone(), release_write.clone()));
        let publish_second = PublishNpmPackageUseCase::new(packages.clone(), pausing_storage, Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let second = {
            let name = name.clone();
            tokio::spawn(async move { publish_second.execute(repository_id, &name, &NpmVersion::parse("2.0.0").unwrap(), serde_json::json!({}), Bytes::from_static(b"second"), &[], publisher_id).await })
        };
        // The second publish has found the package and is about to write its tarball.
        entered_write.notified().await;
        UnpublishNpmPackageUseCase::new(packages.clone(), real_storage.clone(), Arc::new(FakeEvents::new()))
            .execute_version(repository_id, &name, &NpmVersion::parse("1.0.0").unwrap(), publisher_id)
            .await
            .unwrap();
        assert!(packages.find_package(repository_id, &name).await.unwrap().is_none(), "the unpublish took the package with it");

        release_write.notify_one();
        second.await.unwrap().expect("the publish must not fail because its package was deleted meanwhile");

        let package = packages.find_package(repository_id, &name).await.unwrap().expect("the publish recreated the package");
        let version = packages.find_version(package.id, &NpmVersion::parse("2.0.0").unwrap()).await.unwrap().unwrap();
        assert_eq!(real_storage.read(repository_id, &version.tarball_storage_key).await.unwrap(), b"second");
        let tags = packages.list_dist_tags(package.id).await.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].version.as_str(), "2.0.0");
        assert_eq!(tarball_files_under(storage_root.path()), 1, "only the live version's tarball is left");
    }

    #[tokio::test]
    async fn a_refused_publish_removes_the_tarball_it_wrote() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), storage.clone(), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        *packages.insert_version_conflicts.lock().unwrap() = true;

        let err = publish_with(&use_case, Uuid::new_v4(), "1.0.0", serde_json::json!({}), &[]).await.unwrap_err();

        assert!(matches!(err, ApplicationError::PackageVersionExists), "{err:?}");
        assert!(storage.written.lock().unwrap().is_empty(), "the tarball written for the refused publish is gone");
    }

    async fn publish_with(use_case: &PublishNpmPackageUseCase, repository_id: Uuid, version: &str, manifest: serde_json::Value, tags: &[&str]) -> Result<Uuid, ApplicationError> {
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        use_case
            .execute(repository_id, &NpmPackageName::parse("left-pad").unwrap(), &NpmVersion::parse(version).unwrap(), manifest, Bytes::from_static(b"tarball"), &tags, Uuid::new_v4())
            .await
    }

    fn tags_of(packages: &FakePackages, repository_id: Uuid) -> Vec<(String, String)> {
        let package = packages.packages.lock().unwrap().get(&(repository_id, "left-pad".to_string())).cloned().expect("package");
        let mut tags: Vec<(String, String)> =
            packages.dist_tags.lock().unwrap().iter().filter(|((id, _), _)| *id == package.id).map(|((_, tag), version)| (tag.clone(), version.as_str())).collect();
        tags.sort();
        tags
    }

    #[tokio::test]
    async fn a_requested_dist_tag_is_set_and_latest_is_left_alone() {
        let packages = Arc::new(FakePackages::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), Arc::new(FakeStorage::new()), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let repository_id = Uuid::new_v4();
        publish_with(&use_case, repository_id, "2.0.0", serde_json::json!({}), &[]).await.unwrap();

        // `npm publish --tag maintenance` of a backport that is lower than latest.
        publish_with(&use_case, repository_id, "1.5.1", serde_json::json!({}), &["maintenance"]).await.unwrap();

        assert_eq!(tags_of(&packages, repository_id), vec![("latest".to_string(), "2.0.0".to_string()), ("maintenance".to_string(), "1.5.1".to_string())]);
    }

    #[tokio::test]
    async fn a_version_with_build_metadata_is_not_a_prerelease() {
        let packages = Arc::new(FakePackages::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), Arc::new(FakeStorage::new()), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let repository_id = Uuid::new_v4();

        publish_with(&use_case, repository_id, "1.0.0+build-5", serde_json::json!({}), &[]).await.unwrap();

        assert_eq!(tags_of(&packages, repository_id), vec![("latest".to_string(), "1.0.0+build-5".to_string())]);
    }

    #[tokio::test]
    async fn an_invalid_dist_tag_is_rejected_before_anything_is_stored() {
        let packages = Arc::new(FakePackages::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), Arc::new(FakeStorage::new()), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let repository_id = Uuid::new_v4();

        for bad in ["1.2.3", "a b", ""] {
            let err = publish_with(&use_case, repository_id, "1.0.0", serde_json::json!({}), &[bad]).await.unwrap_err();
            assert!(matches!(err, ApplicationError::InvalidNpmPayload(_)), "{bad:?}: {err:?}");
        }
        assert!(packages.packages.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_stored_manifest_carries_the_real_name_and_version() {
        let packages = Arc::new(FakePackages::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), Arc::new(FakeStorage::new()), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let repository_id = Uuid::new_v4();

        publish_with(&use_case, repository_id, "1.0.0", serde_json::json!({ "name": "somebody-elses-package", "version": "9.9.9", "description": "kept" }), &[]).await.unwrap();

        let stored = packages.versions.lock().unwrap().values().next().unwrap().manifest.clone();
        assert_eq!(stored["name"], "left-pad");
        assert_eq!(stored["version"], "1.0.0");
        assert_eq!(stored["description"], "kept");
    }

    #[tokio::test]
    async fn an_oversized_manifest_is_rejected() {
        let use_case = use_case();

        let err = publish_with(&use_case, Uuid::new_v4(), "1.0.0", serde_json::json!({ "readme": "x".repeat(MAX_MANIFEST_BYTES) }), &[]).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InvalidNpmPayload(_)), "{err:?}");
        let err = publish_with(&use_case, Uuid::new_v4(), "1.0.0", serde_json::json!(["not", "an", "object"]), &[]).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidNpmPayload(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_version_that_was_unpublished_cannot_be_published_again() {
        let packages = Arc::new(FakePackages::new());
        let use_case = PublishNpmPackageUseCase::new(packages.clone(), Arc::new(FakeStorage::new()), Arc::new(FakeRepositories::new()), Arc::new(FakeRepositoryQuotaLock::new()), Arc::new(FakeEvents::new()));
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        packages.unpublished.lock().unwrap().insert((repository_id, name.as_str().to_string(), "1.0.0".to_string()));

        let err = publish_with(&use_case, repository_id, "1.0.0", serde_json::json!({}), &[]).await.unwrap_err();

        assert!(matches!(err, ApplicationError::PackageVersionExists), "{err:?}");
        publish_with(&use_case, repository_id, "1.0.1", serde_json::json!({}), &[]).await.unwrap();
    }
}
