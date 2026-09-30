use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use artiferris_domain::npm_package::{NpmPackageName, NpmPackageOrigin, NpmPackageRepositoryPort, NpmPackageVersion, NpmVersion};
use artiferris_domain::npm_remote::RemoteNpmRegistryPort;
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary};
use artiferris_domain::storage::{ByteStream, StorageBackendPort, StorageError};
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha512;
use uuid::Uuid;

use crate::error::ApplicationError;
use crate::keyed_locks::KeyedLocks;
use crate::use_cases::group_resolve::resolve_in_group;

/// Tarballs fetched from an upstream at the same time; each is held in memory (up to 200 MiB) while it is verified.
const MAX_CONCURRENT_PROXY_FILLS: usize = 4;

pub struct DownloadNpmTarballUseCase {
    packages: Arc<dyn NpmPackageRepositoryPort>,
    storage: Arc<dyn StorageBackendPort>,
    remote: Arc<dyn RemoteNpmRegistryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    /// One fetch per (repository, package, version) at a time; the others wait for it and read the result.
    fills: KeyedLocks<(Uuid, String, String)>,
    fill_slots: tokio::sync::Semaphore,
}

/// What a fetch from the upstream came to.
enum Fill {
    Fetched(Vec<u8>),
    /// Another request cached it while this one waited.
    AlreadyCached,
}

impl DownloadNpmTarballUseCase {
    pub fn new(
        packages: Arc<dyn NpmPackageRepositoryPort>,
        storage: Arc<dyn StorageBackendPort>,
        remote: Arc<dyn RemoteNpmRegistryPort>,
        repositories: Arc<dyn PackageRepositoryQueryPort>,
    ) -> Self {
        Self { packages, storage, remote, repositories, fills: KeyedLocks::new(), fill_slots: tokio::sync::Semaphore::new(MAX_CONCURRENT_PROXY_FILLS) }
    }

    /// Returns `None` if the version isn't known to this repository at all.
    pub async fn execute_hosted(
        &self,
        repository_id: Uuid,
        name: &NpmPackageName,
        version: &NpmVersion,
    ) -> Result<Option<Vec<u8>>, ApplicationError> {
        let Some(package) = self.packages.find_package(repository_id, name).await? else {
            return Ok(None);
        };
        let Some(npm_version) = self.packages.find_version(package.id, version).await? else {
            return Ok(None);
        };
        let bytes = self.storage.read(repository_id, &npm_version.tarball_storage_key).await?;
        Ok(Some(bytes))
    }

    /// `authorize_member` is the caller's read policy for every group member descended into; the top-level repository
    /// is checked by the caller.
    pub fn execute<'a, FAuthorize, FutAuthorize>(
        &'a self,
        repository_id: Uuid,
        name: &'a NpmPackageName,
        version: &'a NpmVersion,
        authorize_member: FAuthorize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Vec<u8>>, ApplicationError>> + Send + 'a>>
    where
        FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send + 'a,
        FutAuthorize: std::future::Future<Output = bool> + Send + 'a,
    {
        resolve_in_group(
            &self.repositories,
            repository_id,
            HashSet::new(),
            move |repository_id| self.execute_hosted(repository_id, name, version),
            move |repository_id, repo| async move {
                match self.fill_from_upstream(repository_id, repo, name, version).await? {
                    Some(Fill::Fetched(bytes)) => Ok(Some(bytes)),
                    Some(Fill::AlreadyCached) => self.execute_hosted(repository_id, name, version).await,
                    None => Ok(None),
                }
            },
            || ApplicationError::NpmPackageNotFound,
            authorize_member,
        )
    }

    /// Same as `execute_hosted`, chunked instead of buffered whole.
    pub async fn execute_hosted_stream(&self, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<Option<ByteStream>, ApplicationError> {
        let Some(package) = self.packages.find_package(repository_id, name).await? else {
            return Ok(None);
        };
        let Some(npm_version) = self.packages.find_version(package.id, version).await? else {
            return Ok(None);
        };
        let stream = self.storage.read_stream(repository_id, &npm_version.tarball_storage_key).await?;
        Ok(Some(stream))
    }

    /// Same as `execute`, chunked instead of buffered whole. A proxy cache miss still fetches and verifies the tarball in memory before it is stored.
    pub fn execute_stream<'a, FAuthorize, FutAuthorize>(
        &'a self,
        repository_id: Uuid,
        name: &'a NpmPackageName,
        version: &'a NpmVersion,
        authorize_member: FAuthorize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<ByteStream>, ApplicationError>> + Send + 'a>>
    where
        FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send + 'a,
        FutAuthorize: std::future::Future<Output = bool> + Send + 'a,
    {
        resolve_in_group(
            &self.repositories,
            repository_id,
            HashSet::new(),
            move |repository_id| self.execute_hosted_stream(repository_id, name, version),
            move |repository_id, repo| async move {
                if let Some(cached) = self.execute_hosted_stream(repository_id, name, version).await? {
                    return Ok(Some(cached));
                }
                match self.fill_from_upstream(repository_id, repo, name, version).await? {
                    Some(Fill::Fetched(bytes)) => Ok(Some(Box::pin(futures::stream::once(async move { Ok::<_, StorageError>(bytes::Bytes::from(bytes)) })) as ByteStream)),
                    Some(Fill::AlreadyCached) => self.execute_hosted_stream(repository_id, name, version).await,
                    None => Ok(None),
                }
            },
            || ApplicationError::NpmPackageNotFound,
            authorize_member,
        )
    }

    /// Fetches the tarball from the proxy's upstream, verifies it and caches it. `Ok(None)` if the upstream doesn't have it.
    async fn fill_from_upstream(
        &self,
        repository_id: Uuid,
        repo: PackageRepositorySummary,
        name: &NpmPackageName,
        version: &NpmVersion,
    ) -> Result<Option<Fill>, ApplicationError> {
        let _turn = self.fills.lock((repository_id, name.as_str().to_string(), version.as_str())).await;
        if let Some(package) = self.packages.find_package(repository_id, name).await? {
            if self.packages.find_version(package.id, version).await?.is_some() {
                return Ok(Some(Fill::AlreadyCached));
            }
        }
        let _slot = self.fill_slots.acquire().await.map_err(|_| artiferris_domain::error::DomainError::Infrastructure("the proxy fetch limiter is closed".to_string()))?;

        let Some(package) = self.packages.find_package(repository_id, name).await? else {
            return Ok(None);
        };
        let Some(cached) = package.cached_metadata.as_ref() else {
            return Ok(None);
        };
        let Some(tarball_url) = cached
            .get("versions")
            .and_then(|v| v.get(version.as_str()))
            .and_then(|v| v.get("dist"))
            .and_then(|d| d.get("tarball"))
            .and_then(|t| t.as_str())
        else {
            return Ok(None);
        };

        let remote_url = repo
            .remote_url
            .as_deref()
            .ok_or_else(|| ApplicationError::InvalidNpmPayload("proxy repository has no remote_url configured".into()))?;
        let Some(tarball_bytes) =
            self.remote.fetch_tarball(remote_url, tarball_url, repo.remote_username.as_deref(), repo.remote_password.as_deref()).await?
        else {
            return Ok(None);
        };

        let manifest = cached
            .get("versions")
            .and_then(|v| v.get(version.as_str()))
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Hashed server-side, never trusting the remote's shasum or integrity; off the async executor since it is
        // CPU-bound.
        let (shasum, integrity, tarball_bytes) = tokio::task::spawn_blocking(move || {
            let shasum = hex::encode(Sha1::digest(&tarball_bytes));
            let integrity = format!("sha512-{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, Sha512::digest(&tarball_bytes)));
            (shasum, integrity, tarball_bytes)
        })
        .await
        .map_err(|e| artiferris_domain::error::DomainError::Infrastructure(e.to_string()))?;

        check_advertised_checksums(&manifest, &shasum, &integrity)?;

        let filename = format!("{}-{}.tgz", name.as_str().rsplit('/').next().unwrap_or(name.as_str()), version.as_str());
        let storage_key = format!("{}/-/{filename}", name.as_str().trim_start_matches('@'));
        self.storage.write(repository_id, &storage_key, &tarball_bytes).await?;

        let npm_version = NpmPackageVersion {
            id: Uuid::new_v4(),
            npm_package_id: package.id,
            version: version.clone(),
            manifest,
            shasum,
            integrity,
            tarball_storage_key: storage_key,
            tarball_size_bytes: tarball_bytes.len() as i64,
            deprecated: false,
            deprecated_message: None,
            published_by: None,
            published_at: Utc::now(),
            origin: NpmPackageOrigin::ProxyCache,
        };
        match self.packages.insert_version(&npm_version).await {
            Ok(()) | Err(artiferris_domain::error::DomainError::NpmVersionAlreadyExists) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(Some(Fill::Fetched(tarball_bytes)))
    }
}

/// Compares the `dist.integrity` (sha512 entries only) and `dist.shasum` the upstream advertises with the fetched bytes'.
fn check_advertised_checksums(version_manifest: &serde_json::Value, shasum: &str, integrity: &str) -> Result<(), ApplicationError> {
    let dist = version_manifest.get("dist");
    if let Some(advertised) = dist.and_then(|d| d.get("integrity")).and_then(|i| i.as_str()) {
        let sha512_entries: Vec<&str> = advertised.split_whitespace().filter(|entry| entry.starts_with("sha512-")).collect();
        if !sha512_entries.is_empty() && !sha512_entries.iter().any(|entry| *entry == integrity) {
            return Err(ApplicationError::UpstreamIntegrityMismatch);
        }
    }
    if let Some(advertised) = dist.and_then(|d| d.get("shasum")).and_then(|s| s.as_str()) {
        if !advertised.eq_ignore_ascii_case(shasum) {
            return Err(ApplicationError::UpstreamIntegrityMismatch);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::npm_test_support::{FakePackages, FakeRemoteRegistry, FakeStorage};
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};
    use artiferris_domain::package_repository::RepositoryType;

    fn fake_tarball_integrity() -> String {
        format!("sha512-{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, Sha512::digest(b"fake-tarball-bytes")))
    }

    #[tokio::test]
    async fn downloading_an_already_stored_hosted_tarball_reads_it_back_unchanged() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: name.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        storage.write(repository_id, "left-pad/-/left-pad-1.0.0.tgz", b"real-bytes").await.unwrap();
        packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: version.clone(),
                manifest: serde_json::json!({}),
                shasum: "s".into(),
                integrity: "i".into(),
                tarball_storage_key: "left-pad/-/left-pad-1.0.0.tgz".into(),
                tarball_size_bytes: 10,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();

        let use_case = DownloadNpmTarballUseCase::new(
            packages,
            storage,
            Arc::new(FakeRemoteRegistry::new()),
            Arc::new(crate::use_cases::npm_test_support::FakeRepositories::new()),
        );
        let bytes = use_case.execute_hosted(repository_id, &name, &version).await.unwrap();
        assert_eq!(bytes.as_deref(), Some(b"real-bytes".as_slice()));
    }

    #[tokio::test]
    async fn execute_hosted_stream_yields_the_same_bytes_as_execute_hosted() {
        use futures::StreamExt;

        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: name.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        storage.write(repository_id, "left-pad/-/left-pad-1.0.0.tgz", b"streamed-bytes").await.unwrap();
        packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: version.clone(),
                manifest: serde_json::json!({}),
                shasum: "s".into(),
                integrity: "i".into(),
                tarball_storage_key: "left-pad/-/left-pad-1.0.0.tgz".into(),
                tarball_size_bytes: 14,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();

        let use_case = DownloadNpmTarballUseCase::new(
            packages,
            storage,
            Arc::new(FakeRemoteRegistry::new()),
            Arc::new(crate::use_cases::npm_test_support::FakeRepositories::new()),
        );
        let mut stream = use_case.execute_hosted_stream(repository_id, &name, &version).await.unwrap().unwrap();
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(collected, b"streamed-bytes");
    }

    #[tokio::test]
    async fn downloading_a_missing_hosted_version_returns_none() {
        let use_case = DownloadNpmTarballUseCase::new(
            Arc::new(FakePackages::new()),
            Arc::new(FakeStorage::new()),
            Arc::new(FakeRemoteRegistry::new()),
            Arc::new(crate::use_cases::npm_test_support::FakeRepositories::new()),
        );
        let result = use_case
            .execute_hosted(Uuid::new_v4(), &NpmPackageName::parse("left-pad").unwrap(), &NpmVersion::parse("1.0.0").unwrap())
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn downloading_via_a_proxy_repository_lazily_fetches_and_caches_the_tarball() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repositories = Arc::new(crate::use_cases::npm_test_support::FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();

        repositories.insert(artiferris_domain::package_repository::PackageRepositorySummary {
            id: repository_id,
            organization_id: Uuid::new_v4(),
            name: "proxy-repo".to_string(),
            format: artiferris_domain::package_repository::RepositoryFormat::Npm,
            repo_type: RepositoryType::Proxy,
            remote_url: Some("https://registry.example.com".to_string()),
            remote_username: None,
            remote_password: None,
            quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
        });

        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: name.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: Some(Utc::now()),
            cached_metadata: Some(serde_json::json!({
                "versions": {
                    "1.0.0": {
                        "dist": {
                            "tarball": "https://registry.example.com/left-pad/-/left-pad-1.0.0.tgz",
                            "integrity": fake_tarball_integrity()
                        }
                    }
                }
            })),
        };
        packages.create_package(&package).await.unwrap();

        let packages_inspect = packages.clone();
        let use_case = DownloadNpmTarballUseCase::new(packages, storage, Arc::new(FakeRemoteRegistry::new()), repositories);
        let bytes = use_case.execute(repository_id, &name, &version, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();
        assert_eq!(bytes.as_deref(), Some(b"fake-tarball-bytes".as_slice()));

        let expected_shasum = hex::encode(Sha1::digest(b"fake-tarball-bytes"));
        let expected_integrity = format!(
            "sha512-{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, Sha512::digest(b"fake-tarball-bytes"))
        );
        let stored_version = packages_inspect
            .versions
            .lock()
            .unwrap()
            .get(&(package.id, version.as_str()))
            .cloned()
            .expect("proxy-cached version must be persisted");
        assert_eq!(stored_version.shasum, expected_shasum);
        assert_eq!(stored_version.integrity, expected_integrity);
        assert_eq!(stored_version.origin, NpmPackageOrigin::ProxyCache);

        let bytes_again = use_case.execute_hosted(repository_id, &name, &version).await.unwrap();
        assert_eq!(bytes_again.as_deref(), Some(b"fake-tarball-bytes".as_slice()));
    }

    /// A first proxy fetch whose upstream tarball 404s is "not found": no error, no version row, nothing written.
    #[tokio::test]
    async fn a_first_time_tarball_fetch_that_404s_upstream_returns_not_found() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repositories = Arc::new(crate::use_cases::npm_test_support::FakeRepositories::new());
        let remote = Arc::new(FakeRemoteRegistry::new());
        *remote.tarball_response.lock().unwrap() = None;

        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();

        let repo = artiferris_domain::package_repository::PackageRepositorySummary {
            id: repository_id,
            organization_id: Uuid::new_v4(),
            name: "proxy-repo".to_string(),
            format: artiferris_domain::package_repository::RepositoryFormat::Npm,
            repo_type: RepositoryType::Proxy,
            remote_url: Some("https://registry.example.com".to_string()),
            remote_username: None,
            remote_password: None,
            quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
        };

        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: name.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: Some(Utc::now()),
            cached_metadata: Some(serde_json::json!({
                "versions": {
                    "1.0.0": {
                        "dist": {
                            "tarball": "https://registry.example.com/left-pad/-/left-pad-1.0.0.tgz",
                            "integrity": "sha512-fake"
                        }
                    }
                }
            })),
        };
        packages.create_package(&package).await.unwrap();

        let packages_inspect = packages.clone();
        let storage_inspect = storage.clone();
        let use_case = DownloadNpmTarballUseCase::new(packages, storage, remote, repositories);
        let result = use_case.fill_from_upstream(repository_id, repo, &name, &version).await.unwrap();

        assert!(result.is_none(), "a first-time upstream 404 for the tarball must be Ok(None), not an error");
        assert!(
            packages_inspect.versions.lock().unwrap().get(&(package.id, version.as_str())).is_none(),
            "no version row should be persisted for a tarball that doesn't exist upstream"
        );
        assert!(storage_inspect.written.lock().unwrap().is_empty(), "nothing should be written to storage for a 404'd tarball");
    }

    #[tokio::test]
    async fn a_cyclic_group_configuration_resolves_without_hanging() {
        let repo_a_id = Uuid::new_v4();
        let repo_b_id = Uuid::new_v4();

        let repositories = Arc::new(crate::use_cases::npm_test_support::FakeRepositories::new());
        repositories.insert(artiferris_domain::package_repository::PackageRepositorySummary {
            id: repo_a_id,
            organization_id: Uuid::new_v4(),
            name: "group-a".to_string(),
            format: artiferris_domain::package_repository::RepositoryFormat::Npm,
            repo_type: RepositoryType::Group,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![repo_b_id],
        });
        repositories.insert(artiferris_domain::package_repository::PackageRepositorySummary {
            id: repo_b_id,
            organization_id: Uuid::new_v4(),
            name: "group-b".to_string(),
            format: artiferris_domain::package_repository::RepositoryFormat::Npm,
            repo_type: RepositoryType::Group,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![repo_a_id],
        });

        let use_case = DownloadNpmTarballUseCase::new(
            Arc::new(FakePackages::new()),
            Arc::new(FakeStorage::new()),
            Arc::new(FakeRemoteRegistry::new()),
            repositories,
        );
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            use_case.execute(repo_a_id, &name, &version, |_repo: &PackageRepositorySummary| async { true }),
        )
            .await
            .expect("execute() must not hang on a cyclic group configuration");
        assert!(result.unwrap().is_none());
    }

    /// A proxy repository whose cached metadata for left-pad@1.0.0 carries `dist`, plus the use case wired to fakes.
    async fn proxy_with_dist(dist: serde_json::Value) -> (DownloadNpmTarballUseCase, Arc<FakePackages>, Uuid, NpmPackageName, NpmVersion) {
        let packages = Arc::new(FakePackages::new());
        let repositories = Arc::new(crate::use_cases::npm_test_support::FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(artiferris_domain::package_repository::PackageRepositorySummary {
            id: repository_id,
            organization_id: Uuid::new_v4(),
            name: "proxy-repo".to_string(),
            format: artiferris_domain::package_repository::RepositoryFormat::Npm,
            repo_type: RepositoryType::Proxy,
            remote_url: Some("https://registry.example.com".to_string()),
            remote_username: None,
            remote_password: None,
            quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
        });
        let name = NpmPackageName::parse("left-pad").unwrap();
        packages
            .create_package(&NpmPackage {
                id: Uuid::new_v4(),
                package_repository_id: repository_id,
                name: name.clone(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                metadata_fetched_at: Some(Utc::now()),
                cached_metadata: Some(serde_json::json!({ "versions": { "1.0.0": { "dist": dist } } })),
            })
            .await
            .unwrap();
        let use_case = DownloadNpmTarballUseCase::new(packages.clone(), Arc::new(FakeStorage::new()), Arc::new(FakeRemoteRegistry::new()), repositories);
        (use_case, packages, repository_id, name, NpmVersion::parse("1.0.0").unwrap())
    }

    #[tokio::test]
    async fn a_tarball_that_does_not_match_the_advertised_checksums_is_rejected_and_not_cached() {
        for dist in [
            serde_json::json!({ "tarball": "https://registry.example.com/x.tgz", "integrity": "sha512-AAAA" }),
            serde_json::json!({ "tarball": "https://registry.example.com/x.tgz", "shasum": "0000000000000000000000000000000000000000" }),
            serde_json::json!({ "tarball": "https://registry.example.com/x.tgz", "integrity": format!("sha1-zzz {}", "sha512-BBBB") }),
        ] {
            let (use_case, packages, repository_id, name, version) = proxy_with_dist(dist).await;

            let err = use_case.execute(repository_id, &name, &version, |_repo: &PackageRepositorySummary| async { true }).await.unwrap_err();

            assert!(matches!(err, ApplicationError::UpstreamIntegrityMismatch), "{err:?}");
            assert!(packages.versions.lock().unwrap().is_empty(), "a tampered tarball must never be cached");
        }
    }

    #[tokio::test]
    async fn a_tarball_matching_the_advertised_checksums_is_cached() {
        let shasum = hex::encode(Sha1::digest(b"fake-tarball-bytes"));
        let dist = serde_json::json!({ "tarball": "https://registry.example.com/x.tgz", "integrity": format!("sha1-zzz {}", fake_tarball_integrity()), "shasum": shasum.to_uppercase() });
        let (use_case, packages, repository_id, name, version) = proxy_with_dist(dist).await;

        let bytes = use_case.execute(repository_id, &name, &version, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        assert_eq!(bytes.as_deref(), Some(b"fake-tarball-bytes".as_slice()));
        assert_eq!(packages.versions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn losing_the_race_to_cache_a_version_is_not_an_error() {
        let (use_case, packages, repository_id, name, version) = proxy_with_dist(serde_json::json!({ "tarball": "https://registry.example.com/x.tgz" })).await;
        *packages.insert_version_conflicts.lock().unwrap() = true;

        let bytes = use_case.execute(repository_id, &name, &version, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        assert_eq!(bytes.as_deref(), Some(b"fake-tarball-bytes".as_slice()));
    }

    async fn read_all(mut stream: ByteStream) -> Vec<u8> {
        use futures::StreamExt;
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        collected
    }

    fn allow_all() -> impl Fn(&PackageRepositorySummary) -> std::future::Ready<bool> + Clone + Send {
        |_repo: &PackageRepositorySummary| std::future::ready(true)
    }

    async fn proxy_with_remote(remote: Arc<FakeRemoteRegistry>, storage: Arc<FakeStorage>) -> (Arc<DownloadNpmTarballUseCase>, Uuid, NpmPackageName, NpmVersion) {
        let packages = Arc::new(FakePackages::new());
        let repositories = Arc::new(crate::use_cases::npm_test_support::FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        repositories.insert(PackageRepositorySummary {
            id: repository_id, organization_id: Uuid::new_v4(), name: "proxy-repo".to_string(), format: artiferris_domain::package_repository::RepositoryFormat::Npm,
            repo_type: RepositoryType::Proxy, remote_url: Some("https://registry.example.com".to_string()), remote_username: None, remote_password: None,
            quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
        });
        packages
            .create_package(&NpmPackage {
                id: Uuid::new_v4(), package_repository_id: repository_id, name: name.clone(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: Some(Utc::now()),
                cached_metadata: Some(serde_json::json!({ "versions": { "1.0.0": { "dist": { "tarball": "https://registry.example.com/left-pad/-/left-pad-1.0.0.tgz" } } } })),
            })
            .await
            .unwrap();
        (Arc::new(DownloadNpmTarballUseCase::new(packages, storage, remote, repositories)), repository_id, name, NpmVersion::parse("1.0.0").unwrap())
    }

    #[tokio::test]
    async fn a_cached_proxy_tarball_is_streamed_from_storage_without_going_upstream_or_being_read_whole() {
        let remote = Arc::new(FakeRemoteRegistry::new());
        let storage = Arc::new(FakeStorage::new());
        let (use_case, repository_id, name, version) = proxy_with_remote(remote.clone(), storage.clone()).await;
        let first = use_case.execute_stream(repository_id, &name, &version, allow_all()).await.unwrap().unwrap();
        assert_eq!(read_all(first).await, b"fake-tarball-bytes");
        let reads_after_the_fill = storage.whole_reads.load(std::sync::atomic::Ordering::SeqCst);

        let second = use_case.execute_stream(repository_id, &name, &version, allow_all()).await.unwrap().unwrap();

        assert_eq!(read_all(second).await, b"fake-tarball-bytes");
        assert_eq!(remote.tarball_fetches.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(storage.whole_reads.load(std::sync::atomic::Ordering::SeqCst), reads_after_the_fill, "a cache hit must not read the tarball into memory");
    }

    #[tokio::test]
    async fn concurrent_pulls_of_an_uncached_proxy_tarball_reach_the_upstream_once() {
        let remote = Arc::new(FakeRemoteRegistry::new());
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *remote.tarball_gate.lock().unwrap() = Some(gate.clone());
        let (use_case, repository_id, name, version) = proxy_with_remote(remote.clone(), Arc::new(FakeStorage::new())).await;
        let pull = || {
            let use_case = use_case.clone();
            let (name, version) = (name.clone(), version.clone());
            tokio::spawn(async move { read_all(use_case.execute_stream(repository_id, &name, &version, allow_all()).await.unwrap().unwrap()).await })
        };

        let first = pull();
        while remote.tarball_fetches.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let others = [pull(), pull()];
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(remote.tarball_fetches.load(std::sync::atomic::Ordering::SeqCst), 1, "the others wait for the fetch in progress");

        gate.add_permits(1);
        assert_eq!(first.await.unwrap(), b"fake-tarball-bytes");
        for pulled in others {
            assert_eq!(pulled.await.unwrap(), b"fake-tarball-bytes");
        }
        assert_eq!(remote.tarball_fetches.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
