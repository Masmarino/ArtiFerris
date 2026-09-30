use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use artiferris_domain::audit::{AdminAuditEvent, AuditPage, AuditQueryFilter, EventPublisherPort, NpmPackageEvent, SecurityEvent};
use artiferris_domain::error::{DomainError, EventStoreError};
use artiferris_domain::npm_audit::{NpmAdvisory, NpmAuditPort};
use artiferris_domain::npm_package::{NpmDistTag, NpmPackage, NpmPackageName, NpmPackageRepositoryPort, NpmPackageVersion, NpmVersion, UnpublishVersionOutcome, UnpublishedPackage, UnpublishedVersion};
use artiferris_domain::npm_remote::RemoteNpmRegistryPort;
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary, RepositoryLockGuard, RepositoryQuotaLockPort};
use artiferris_domain::storage::{StorageBackendPort, StorageError};
use uuid::Uuid;

pub struct FakePackages {
    pub packages: Mutex<HashMap<(Uuid, String), NpmPackage>>,
    pub versions: Mutex<HashMap<(Uuid, String), NpmPackageVersion>>,
    pub dist_tags: Mutex<HashMap<(Uuid, String), NpmVersion>>,
    /// Test hook fired once after the first `list_dist_tags_for_packages` result is computed, to simulate a dist-tag
    /// landing after a stale snapshot.
    pub after_first_list_dist_tags_for_packages: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// How many versions were read together with their full manifest, for tests that assert a listing doesn't load them all.
    pub manifests_read: AtomicUsize,
    /// How many version rows the capped listing handed back, for tests that assert the cap is applied before the rows are loaded.
    pub capped_summaries_returned: AtomicUsize,
    pub unpublished: Mutex<std::collections::HashSet<(Uuid, String, String)>>,
    /// Makes `insert_version` store the row and then report a unique violation, as if a concurrent request had inserted it first.
    pub insert_version_conflicts: Mutex<bool>,
    /// Makes `search` fail for this repository.
    pub failing_search_repository: Mutex<Option<Uuid>>,
}

impl FakePackages {
    pub fn new() -> Self {
        Self {
            packages: Mutex::new(HashMap::new()),
            versions: Mutex::new(HashMap::new()),
            dist_tags: Mutex::new(HashMap::new()),
            after_first_list_dist_tags_for_packages: Mutex::new(None),
            manifests_read: AtomicUsize::new(0),
            capped_summaries_returned: AtomicUsize::new(0),
            unpublished: Mutex::new(std::collections::HashSet::new()),
            insert_version_conflicts: Mutex::new(false),
            failing_search_repository: Mutex::new(None),
        }
    }
}

#[async_trait]
impl NpmPackageRepositoryPort for FakePackages {
    async fn find_package(&self, repository_id: Uuid, name: &artiferris_domain::npm_package::NpmPackageName) -> Result<Option<NpmPackage>, DomainError> {
        Ok(self.packages.lock().unwrap().get(&(repository_id, name.as_str().to_string())).cloned())
    }
    async fn find_by_id(&self, id: Uuid) -> Result<Option<NpmPackage>, DomainError> {
        Ok(self.packages.lock().unwrap().values().find(|p| p.id == id).cloned())
    }
    async fn create_package(&self, package: &NpmPackage) -> Result<Uuid, DomainError> {
        let mut packages = self.packages.lock().unwrap();
        let existing = packages.entry((package.package_repository_id, package.name.as_str().to_string())).or_insert_with(|| package.clone());
        Ok(existing.id)
    }
    async fn touch_metadata_fetched_at(&self, npm_package_id: Uuid, fetched_at: DateTime<Utc>) -> Result<(), DomainError> {
        let mut packages = self.packages.lock().unwrap();
        if let Some(p) = packages.values_mut().find(|p| p.id == npm_package_id) {
            p.metadata_fetched_at = Some(fetched_at);
        }
        Ok(())
    }
    async fn set_cached_metadata(&self, npm_package_id: Uuid, metadata: serde_json::Value) -> Result<(), DomainError> {
        let mut packages = self.packages.lock().unwrap();
        if let Some(p) = packages.values_mut().find(|p| p.id == npm_package_id) {
            p.cached_metadata = Some(metadata);
        }
        Ok(())
    }
    async fn list_versions(&self, npm_package_id: Uuid) -> Result<Vec<NpmPackageVersion>, DomainError> {
        let versions: Vec<NpmPackageVersion> = self.versions.lock().unwrap().values().filter(|v| v.npm_package_id == npm_package_id).cloned().collect();
        self.manifests_read.fetch_add(versions.len(), Ordering::Relaxed);
        Ok(versions)
    }
    async fn list_versions_for_packages(&self, npm_package_ids: &[Uuid]) -> Result<Vec<artiferris_domain::npm_package::NpmPackageVersionSummary>, DomainError> {
        Ok(self
            .versions
            .lock()
            .unwrap()
            .values()
            .filter(|v| npm_package_ids.contains(&v.npm_package_id))
            .map(|v| artiferris_domain::npm_package::NpmPackageVersionSummary {
                id: v.id,
                npm_package_id: v.npm_package_id,
                version: v.version.clone(),
                shasum: v.shasum.clone(),
                tarball_size_bytes: v.tarball_size_bytes,
                deprecated: v.deprecated,
                deprecated_message: v.deprecated_message.clone(),
                published_at: v.published_at,
            })
            .collect())
    }
    async fn list_latest_versions_for_packages(&self, npm_package_ids: &[Uuid], per_package: i64) -> Result<Vec<artiferris_domain::npm_package::NpmPackageVersionSummary>, DomainError> {
        let mut all = self.list_versions_for_packages(npm_package_ids).await?;
        all.sort_by_key(|v| (v.npm_package_id, std::cmp::Reverse(v.published_at)));
        let mut per_package_count: HashMap<Uuid, i64> = HashMap::new();
        all.retain(|v| {
            let count = per_package_count.entry(v.npm_package_id).or_default();
            *count += 1;
            *count <= per_package
        });
        self.capped_summaries_returned.fetch_add(all.len(), Ordering::Relaxed);
        Ok(all)
    }
    async fn find_version(&self, npm_package_id: Uuid, version: &NpmVersion) -> Result<Option<NpmPackageVersion>, DomainError> {
        self.manifests_read.fetch_add(1, Ordering::Relaxed);
        Ok(self.versions.lock().unwrap().get(&(npm_package_id, version.as_str())).cloned())
    }
    async fn insert_version(&self, version: &NpmPackageVersion) -> Result<(), DomainError> {
        self.versions.lock().unwrap().insert((version.npm_package_id, version.version.as_str()), version.clone());
        if *self.insert_version_conflicts.lock().unwrap() {
            return Err(DomainError::NpmVersionAlreadyExists);
        }
        Ok(())
    }
    async fn publish_version(&self, package: &NpmPackage, version: &NpmPackageVersion, dist_tags: &[String]) -> Result<Uuid, DomainError> {
        if self.was_unpublished(package.package_repository_id, &package.name, &version.version).await? {
            return Err(DomainError::NpmVersionAlreadyExists);
        }
        if let Some(existing) = self.find_package(package.package_repository_id, &package.name).await? {
            let versions = self.versions.lock().unwrap();
            let held: Vec<_> = versions.values().filter(|v| v.npm_package_id == existing.id).collect();
            if held.iter().any(|v| v.version.release() == version.version.release()) {
                return Err(DomainError::NpmVersionAlreadyExists);
            }
            if held.len() as i64 >= artiferris_domain::npm_package::MAX_VERSIONS_PER_PACKAGE {
                return Err(DomainError::NpmPackageLimit("too many versions".into()));
            }
        }
        let package_id = self.create_package(package).await?;
        let mut stored = version.clone();
        stored.npm_package_id = package_id;
        self.insert_version(&stored).await?;
        for tag in dist_tags {
            self.set_dist_tag(package_id, tag, &version.version).await?;
        }
        Ok(package_id)
    }
    async fn unpublish_version(&self, repository_id: Uuid, name: &artiferris_domain::npm_package::NpmPackageName, version: &NpmVersion) -> Result<UnpublishVersionOutcome, DomainError> {
        let Some(package) = self.find_package(repository_id, name).await? else {
            return Ok(UnpublishVersionOutcome::PackageNotFound);
        };
        let Some(removed) = self.versions.lock().unwrap().remove(&(package.id, version.as_str())) else {
            return Ok(UnpublishVersionOutcome::VersionNotFound);
        };
        self.unpublished.lock().unwrap().insert((repository_id, name.as_str().to_string(), version.as_str()));
        self.dist_tags.lock().unwrap().retain(|(id, _), tagged| !(*id == package.id && tagged == version));
        let package_deleted = !self.versions.lock().unwrap().values().any(|v| v.npm_package_id == package.id);
        if package_deleted {
            self.packages.lock().unwrap().retain(|_, p| p.id != package.id);
        }
        Ok(UnpublishVersionOutcome::Removed(UnpublishedVersion { package_id: package.id, tarball_storage_key: removed.tarball_storage_key, package_deleted }))
    }
    async fn unpublish_package(&self, repository_id: Uuid, name: &artiferris_domain::npm_package::NpmPackageName) -> Result<Option<UnpublishedPackage>, DomainError> {
        let Some(package) = self.find_package(repository_id, name).await? else {
            return Ok(None);
        };
        let versions: Vec<NpmPackageVersion> = self.versions.lock().unwrap().values().filter(|v| v.npm_package_id == package.id).cloned().collect();
        for version in &versions {
            self.unpublished.lock().unwrap().insert((repository_id, name.as_str().to_string(), version.version.as_str()));
        }
        self.packages.lock().unwrap().retain(|_, p| p.id != package.id);
        self.versions.lock().unwrap().retain(|_, v| v.npm_package_id != package.id);
        self.dist_tags.lock().unwrap().retain(|(id, _), _| *id != package.id);
        Ok(Some(UnpublishedPackage { package_id: package.id, tarball_storage_keys: versions.into_iter().map(|v| v.tarball_storage_key).collect() }))
    }
    async fn was_unpublished(&self, repository_id: Uuid, name: &artiferris_domain::npm_package::NpmPackageName, version: &NpmVersion) -> Result<bool, DomainError> {
        Ok(self.unpublished.lock().unwrap().iter().any(|(repo, package, unpublished)| {
            *repo == repository_id && package == name.as_str() && NpmVersion::parse(unpublished).is_ok_and(|unpublished| unpublished.release() == version.release())
        }))
    }
    async fn set_deprecated(&self, npm_package_id: Uuid, version: &NpmVersion, message: Option<&str>) -> Result<(), DomainError> {
        if let Some(v) = self.versions.lock().unwrap().get_mut(&(npm_package_id, version.as_str())) {
            v.deprecated = true;
            v.deprecated_message = message.map(str::to_string);
        }
        Ok(())
    }
    async fn list_dist_tags(&self, npm_package_id: Uuid) -> Result<Vec<NpmDistTag>, DomainError> {
        Ok(self.dist_tags.lock().unwrap().iter().filter(|((id, _), _)| *id == npm_package_id)
            .map(|((_, tag), version)| NpmDistTag { npm_package_id, tag: tag.clone(), version: version.clone() }).collect())
    }
    async fn list_dist_tags_for_packages(&self, npm_package_ids: &[Uuid]) -> Result<Vec<NpmDistTag>, DomainError> {
        let result = self.dist_tags.lock().unwrap().iter().filter(|((id, _), _)| npm_package_ids.contains(id))
            .map(|((id, tag), version)| NpmDistTag { npm_package_id: *id, tag: tag.clone(), version: version.clone() }).collect();
        // Fires at most once, after the result above was already computed — see field docs.
        if let Some(hook) = self.after_first_list_dist_tags_for_packages.lock().unwrap().take() {
            hook();
        }
        Ok(result)
    }
    async fn set_dist_tag(&self, npm_package_id: Uuid, tag: &str, version: &NpmVersion) -> Result<(), DomainError> {
        if !self.versions.lock().unwrap().contains_key(&(npm_package_id, version.as_str())) {
            return Err(DomainError::NpmVersionNotFound);
        }
        self.dist_tags.lock().unwrap().insert((npm_package_id, tag.to_string()), version.clone());
        Ok(())
    }
    async fn delete_dist_tag(&self, npm_package_id: Uuid, tag: &str) -> Result<(), DomainError> {
        self.dist_tags.lock().unwrap().remove(&(npm_package_id, tag.to_string()));
        Ok(())
    }
    async fn search(&self, repository_id: Uuid, query: &str, limit: i64) -> Result<Vec<NpmPackage>, DomainError> {
        if *self.failing_search_repository.lock().unwrap() == Some(repository_id) {
            return Err(DomainError::Infrastructure("search is down for this repository".into()));
        }
        Ok(self.packages.lock().unwrap().values()
            .filter(|p| p.package_repository_id == repository_id && p.name.as_str().contains(query))
            .take(limit.max(0) as usize).cloned().collect())
    }
    async fn list_packages_page(&self, repository_id: Uuid, after: Option<&str>, limit: i64) -> Result<Vec<NpmPackage>, DomainError> {
        let mut page: Vec<NpmPackage> = self.packages.lock().unwrap().values().filter(|p| p.package_repository_id == repository_id && after.is_none_or(|after| p.name.as_str() > after)).cloned().collect();
        page.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
        page.truncate(limit.max(0) as usize);
        Ok(page)
    }
}

pub struct FakeRemoteRegistry {
    pub metadata_response: Mutex<Option<serde_json::Value>>,
    /// Per-package-name overrides, checked before falling back to `metadata_response`.
    pub per_package: Mutex<HashMap<String, serde_json::Value>>,
    /// `None` mirrors a genuine upstream 404 for the tarball — not an error.
    pub tarball_response: Mutex<Option<Vec<u8>>>,
    pub tarball_fetches: AtomicUsize,
    pub metadata_fetches: AtomicUsize,
    /// How long every metadata fetch takes.
    pub metadata_delay: Mutex<std::time::Duration>,
    /// When set, a tarball fetch waits for a permit before it answers.
    pub tarball_gate: Mutex<Option<std::sync::Arc<tokio::sync::Semaphore>>>,
}

impl FakeRemoteRegistry {
    pub fn new() -> Self {
        Self {
            metadata_response: Mutex::new(Some(serde_json::json!({ "dist-tags": { "latest": "1.0.0" }, "versions": {} }))),
            per_package: Mutex::new(HashMap::new()),
            tarball_response: Mutex::new(Some(b"fake-tarball-bytes".to_vec())),
            tarball_fetches: AtomicUsize::new(0),
            metadata_fetches: AtomicUsize::new(0),
            metadata_delay: Mutex::new(std::time::Duration::ZERO),
            tarball_gate: Mutex::new(None),
        }
    }

    pub fn set_package(&self, name: &str, packument: serde_json::Value) {
        self.per_package.lock().unwrap().insert(name.to_string(), packument);
    }
}

#[async_trait]
impl RemoteNpmRegistryPort for FakeRemoteRegistry {
    async fn fetch_metadata(
        &self,
        _base_url: &str,
        package_name: &artiferris_domain::npm_package::NpmPackageName,
        _username: Option<&str>,
        _password: Option<&str>,
    ) -> Result<Option<serde_json::Value>, DomainError> {
        self.metadata_fetches.fetch_add(1, Ordering::SeqCst);
        let delay = *self.metadata_delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if let Some(packument) = self.per_package.lock().unwrap().get(package_name.as_str()) {
            return Ok(Some(packument.clone()));
        }
        // `None` here mirrors a genuine upstream 404 — not an error.
        Ok(self.metadata_response.lock().unwrap().clone())
    }
    async fn fetch_tarball(&self, _base_url: &str, _tarball_url: &str, _username: Option<&str>, _password: Option<&str>) -> Result<Option<Vec<u8>>, DomainError> {
        self.tarball_fetches.fetch_add(1, Ordering::SeqCst);
        let gate = self.tarball_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            let _ = gate.acquire().await;
        }
        Ok(self.tarball_response.lock().unwrap().clone())
    }
}

/// Returns a fixed set of advisories and records the versions/packages it was asked to check.
pub struct FakeNpmAudit {
    advisories: Vec<NpmAdvisory>,
    checked_versions: Mutex<Vec<String>>,
    check_calls: std::sync::atomic::AtomicUsize,
    checked_packages: Mutex<Option<HashMap<String, Vec<String>>>>,
    /// Overrides `check_bulk_raw`'s response when set.
    bulk_response: Mutex<Option<serde_json::Value>>,
    /// When set, `check_bulk_raw` waits for a permit before it answers.
    bulk_gate: Mutex<Option<std::sync::Arc<tokio::sync::Semaphore>>>,
    bulk_in_flight: AtomicUsize,
    bulk_max_in_flight: AtomicUsize,
}

impl FakeNpmAudit {
    pub fn new(advisories: Vec<NpmAdvisory>) -> Self {
        Self { advisories, checked_versions: Mutex::new(Vec::new()), check_calls: std::sync::atomic::AtomicUsize::new(0), checked_packages: Mutex::new(None), bulk_response: Mutex::new(None), bulk_gate: Mutex::new(None), bulk_in_flight: AtomicUsize::new(0), bulk_max_in_flight: AtomicUsize::new(0) }
    }

    pub fn checked_versions(&self) -> Vec<String> {
        self.checked_versions.lock().unwrap().clone()
    }

    pub fn check_calls(&self) -> usize {
        self.check_calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn checked_packages(&self) -> Option<HashMap<String, Vec<String>>> {
        self.checked_packages.lock().unwrap().clone()
    }

    pub fn set_bulk_gate(&self, gate: std::sync::Arc<tokio::sync::Semaphore>) {
        *self.bulk_gate.lock().unwrap() = Some(gate);
    }

    pub fn bulk_in_flight(&self) -> usize {
        self.bulk_in_flight.load(Ordering::SeqCst)
    }

    pub fn bulk_max_in_flight(&self) -> usize {
        self.bulk_max_in_flight.load(Ordering::SeqCst)
    }

    pub fn set_bulk_response(&self, response: serde_json::Value) {
        *self.bulk_response.lock().unwrap() = Some(response);
    }
}

#[async_trait]
impl NpmAuditPort for FakeNpmAudit {
    async fn check(&self, _name: &NpmPackageName, versions: &[NpmVersion]) -> Result<Vec<NpmAdvisory>, DomainError> {
        self.check_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self.checked_versions.lock().unwrap() = versions.iter().map(|v| v.as_str()).collect();
        Ok(self.advisories.clone())
    }

    async fn check_bulk_raw(&self, packages: &HashMap<String, Vec<String>>) -> Result<serde_json::Value, DomainError> {
        *self.checked_packages.lock().unwrap() = Some(packages.clone());
        let in_flight = self.bulk_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.bulk_max_in_flight.fetch_max(in_flight, Ordering::SeqCst);
        let gate = self.bulk_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            let _ = gate.acquire().await;
        }
        self.bulk_in_flight.fetch_sub(1, Ordering::SeqCst);
        if let Some(response) = self.bulk_response.lock().unwrap().clone() {
            return Ok(response);
        }
        Ok(serde_json::json!({ "fake-package": self.advisories }))
    }
}

/// Mirrors the Postgres adapter's "insert-only, latest wins by `scanned_at`" semantics.
pub struct FakeDependencyAuditResults {
    saved: Mutex<Vec<artiferris_domain::npm_audit::DependencyAuditResult>>,
}

impl FakeDependencyAuditResults {
    pub fn new() -> Self {
        Self { saved: Mutex::new(Vec::new()) }
    }

    pub fn saved(&self) -> Vec<artiferris_domain::npm_audit::DependencyAuditResult> {
        self.saved.lock().unwrap().clone()
    }
}

#[async_trait]
impl artiferris_domain::npm_audit::DependencyAuditRepositoryPort for FakeDependencyAuditResults {
    async fn save(&self, result: &artiferris_domain::npm_audit::DependencyAuditResult) -> Result<(), DomainError> {
        self.saved.lock().unwrap().push(result.clone());
        Ok(())
    }

    async fn find_latest_for_version(&self, npm_package_version_id: Uuid) -> Result<Option<artiferris_domain::npm_audit::DependencyAuditResult>, DomainError> {
        Ok(self
            .saved
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.npm_package_version_id == npm_package_version_id)
            .max_by_key(|r| r.scanned_at)
            .cloned())
    }

    async fn find_latest_for_versions(
        &self,
        npm_package_version_ids: &[Uuid],
    ) -> Result<Vec<artiferris_domain::npm_audit::DependencyAuditResult>, DomainError> {
        let saved = self.saved.lock().unwrap();
        Ok(npm_package_version_ids
            .iter()
            .filter_map(|id| saved.iter().filter(|r| r.npm_package_version_id == *id).max_by_key(|r| r.scanned_at).cloned())
            .collect())
    }
}

pub struct FakeRepositories {
    pub repositories: Mutex<HashMap<Uuid, PackageRepositorySummary>>,
}

impl FakeRepositories {
    pub fn new() -> Self {
        Self { repositories: Mutex::new(HashMap::new()) }
    }

    pub fn insert(&self, summary: PackageRepositorySummary) {
        self.repositories.lock().unwrap().insert(summary.id, summary);
    }
}

#[async_trait]
impl PackageRepositoryQueryPort for FakeRepositories {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repositories.lock().unwrap().get(&id).cloned())
    }
    async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repositories.lock().unwrap().values().find(|r| r.organization_id == organization_id && r.name == name).cloned())
    }
    async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
        let mut all: Vec<PackageRepositorySummary> = self.repositories.lock().unwrap().values().cloned().collect();
        all.sort_by_key(|repository| repository.id);
        Ok(all)
    }
    async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repositories.lock().unwrap().values().filter(|r| r.organization_id == organization_id).cloned().collect())
    }
}

/// A no-op guard: the fakes do not model concurrency.
struct FakeLockGuard;
impl RepositoryLockGuard for FakeLockGuard {}

pub struct FakeRepositoryQuotaLock;

impl FakeRepositoryQuotaLock {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RepositoryQuotaLockPort for FakeRepositoryQuotaLock {
    async fn acquire_repository_lock(&self, _repository_id: Uuid) -> Result<Box<dyn RepositoryLockGuard>, EventStoreError> {
        Ok(Box::new(FakeLockGuard))
    }
}

pub struct FakeStorage {
    pub written: Mutex<HashMap<(Uuid, String), Vec<u8>>>,
    /// Makes every `delete` fail, for tests of best-effort file removal.
    pub fail_deletes: Mutex<bool>,
    /// How many files were read into memory whole (rather than streamed).
    pub whole_reads: AtomicUsize,
}

impl FakeStorage {
    pub fn new() -> Self { Self { written: Mutex::new(HashMap::new()), fail_deletes: Mutex::new(false), whole_reads: AtomicUsize::new(0) } }
}

#[async_trait]
impl StorageBackendPort for FakeStorage {
    async fn write(&self, repository_id: Uuid, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.written.lock().unwrap().insert((repository_id, path.to_string()), data.to_vec());
        Ok(())
    }
    async fn read(&self, repository_id: Uuid, path: &str) -> Result<Vec<u8>, StorageError> {
        self.whole_reads.fetch_add(1, Ordering::SeqCst);
        self.written.lock().unwrap().get(&(repository_id, path.to_string())).cloned()
            .ok_or_else(|| StorageError::NotFound(path.to_string()))
    }
    async fn read_stream(&self, repository_id: Uuid, path: &str) -> Result<artiferris_domain::storage::ByteStream, StorageError> {
        let data = self.written.lock().unwrap().get(&(repository_id, path.to_string())).cloned().ok_or_else(|| StorageError::NotFound(path.to_string()))?;
        Ok(Box::pin(futures::stream::once(async move { Ok(bytes::Bytes::from(data)) })))
    }
    async fn delete(&self, repository_id: Uuid, path: &str) -> Result<(), StorageError> {
        if *self.fail_deletes.lock().unwrap() {
            return Err(StorageError::Io("disk on fire".to_string()));
        }
        self.written.lock().unwrap().remove(&(repository_id, path.to_string()));
        Ok(())
    }
    async fn delete_repository(&self, repository_id: Uuid) -> Result<(), StorageError> {
        self.written.lock().unwrap().retain(|(id, _), _| *id != repository_id);
        Ok(())
    }
    async fn used_bytes(&self, _repository_id: Uuid) -> Result<u64, StorageError> { Ok(0) }
    async fn is_healthy(&self) -> bool { true }
    async fn volume_space(&self) -> Result<artiferris_domain::storage::VolumeSpace, StorageError> {
        Ok(artiferris_domain::storage::VolumeSpace { total_bytes: 0, free_bytes: 0 })
    }
}

pub struct FakeEvents {
    pub npm_events: Mutex<Vec<(NpmPackageEvent, Uuid, Uuid, Option<Uuid>)>>,
}

impl FakeEvents {
    pub fn new() -> Self { Self { npm_events: Mutex::new(Vec::new()) } }
}

#[async_trait]
impl EventPublisherPort for FakeEvents {
    async fn publish_security_event(&self, _event: SecurityEvent, _actor_id: Option<Uuid>) -> Result<(), EventStoreError> { Ok(()) }
    async fn publish_admin_event(&self, _event: AdminAuditEvent, _actor_id: Option<Uuid>) -> Result<(), EventStoreError> { Ok(()) }
    async fn query_audit_log(&self, _filter: AuditQueryFilter) -> Result<AuditPage, EventStoreError> { Ok(AuditPage { entries: vec![], next_cursor: None }) }
    async fn publish_npm_event(&self, event: NpmPackageEvent, npm_package_id: Uuid, package_repository_id: Uuid, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
        self.npm_events.lock().unwrap().push((event, npm_package_id, package_repository_id, actor_id));
        Ok(())
    }
    async fn publish_docker_event(&self, _event: artiferris_domain::audit::DockerRegistryEvent, _package_repository_id: Uuid, _actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
        Ok(())
    }
}
