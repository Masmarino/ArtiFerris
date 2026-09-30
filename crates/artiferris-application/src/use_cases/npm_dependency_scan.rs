use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::sync::Semaphore;
use artiferris_domain::error::DomainError;
use artiferris_domain::npm_audit::{parse_advisories, DependencyAuditFinding, DependencyAuditRepositoryPort, DependencyAuditResult, NpmAuditPort};
use artiferris_domain::npm_package::{NpmPackageName, NpmPackageRepositoryPort, NpmVersion};
use artiferris_domain::npm_remote::RemoteNpmRegistryPort;
use uuid::Uuid;

use crate::error::ApplicationError;

const PUBLIC_REGISTRY_BASE_URL: &str = "https://registry.npmjs.org";

const MAX_DEPTH: usize = 10;

/// Once hit, the walk stops and the result is marked `truncated`, not silently partial.
const MAX_PACKAGES: usize = 500;

/// Scans that run at once: each holds packuments in memory, over a dependency list the publisher chooses.
const MAX_CONCURRENT_SCANS: usize = 2;

/// Limits of one scan. The publisher chooses the dependency names, so every one of them is bounded.
#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    /// Requests to the public registry, whatever they come back with: unknown names cost one too.
    pub max_requests: usize,
    /// Dependencies read from one manifest; more than this marks the scan truncated.
    pub max_dependencies_per_manifest: usize,
    /// Wall-clock time for the walk. Past it the scan audits what it has and is marked truncated.
    pub deadline: Duration,
    /// How long a manual rescan waits for a scan slot before it gives up.
    pub slot_wait: Duration,
    /// Minimum time between two manual rescans by the same user in the same repository.
    pub manual_interval: Duration,
    pub concurrent_scans: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_requests: 500,
            max_dependencies_per_manifest: 1000,
            deadline: Duration::from_secs(120),
            slot_wait: Duration::from_secs(10),
            manual_interval: Duration::from_secs(30),
            concurrent_scans: MAX_CONCURRENT_SCANS,
        }
    }
}

/// Past this many bytes of trimmed packuments, a scan uses them but stops keeping them.
const MAX_PACKUMENT_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Walks a published version's `dependencies` transitively against the public npm registry, then audits the whole collected set in one bulk call. Persists the result so a page view never waits for a live, potentially multi-second walk.
pub struct ScanDependencyTreeUseCase {
    packages: Arc<dyn NpmPackageRepositoryPort>,
    remote_registry: Arc<dyn RemoteNpmRegistryPort>,
    audit: Arc<dyn NpmAuditPort>,
    results: Arc<dyn DependencyAuditRepositoryPort>,
    scans: Arc<Semaphore>,
    limits: ScanLimits,
    /// When each (user, repository) last asked for a manual rescan.
    manual_scans: Mutex<HashMap<(Uuid, Uuid), Instant>>,
}

impl ScanDependencyTreeUseCase {
    pub fn new(
        packages: Arc<dyn NpmPackageRepositoryPort>,
        remote_registry: Arc<dyn RemoteNpmRegistryPort>,
        audit: Arc<dyn NpmAuditPort>,
        results: Arc<dyn DependencyAuditRepositoryPort>,
    ) -> Self {
        let limits = ScanLimits::default();
        Self { packages, remote_registry, audit, results, scans: Arc::new(Semaphore::new(limits.concurrent_scans)), limits, manual_scans: Mutex::new(HashMap::new()) }
    }

    pub fn with_limits(mut self, limits: ScanLimits) -> Self {
        self.scans = Arc::new(Semaphore::new(limits.concurrent_scans));
        self.limits = limits;
        self
    }

    /// A rescan asked for by `requested_by`: at most one per `manual_interval` for each user and repository, and it gives up
    /// after `slot_wait` if all the scan slots stay taken.
    pub async fn execute(&self, requested_by: Uuid, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<DependencyAuditResult, ApplicationError> {
        self.check_rescan_rate(requested_by, repository_id)?;
        let _turn = match tokio::time::timeout(self.limits.slot_wait, self.scans.acquire()).await {
            Ok(turn) => turn.map_err(|e| DomainError::Infrastructure(e.to_string()))?,
            Err(_) => return Err(ApplicationError::DependencyScanBusy),
        };
        self.scan(repository_id, name, version).await
    }

    fn check_rescan_rate(&self, requested_by: Uuid, repository_id: Uuid) -> Result<(), ApplicationError> {
        let now = Instant::now();
        let mut recent = self.manual_scans.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recent.retain(|_, asked_at| now.duration_since(*asked_at) < self.limits.manual_interval);
        if recent.contains_key(&(requested_by, repository_id)) {
            return Err(ApplicationError::DependencyScanRateLimited);
        }
        recent.insert((requested_by, repository_id), now);
        Ok(())
    }

    /// For the scan after a publish: `None`, doing nothing, when scans are already running. The manual rescan covers it.
    pub async fn execute_unless_busy(&self, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<Option<DependencyAuditResult>, ApplicationError> {
        let Ok(_turn) = self.scans.try_acquire() else { return Ok(None) };
        Ok(Some(self.scan(repository_id, name, version).await?))
    }

    async fn scan(&self, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<DependencyAuditResult, ApplicationError> {
        let package = self.packages.find_package(repository_id, name).await?.ok_or(ApplicationError::NpmPackageNotFound)?;
        let root_version = self.packages.find_version(package.id, version).await?.ok_or(ApplicationError::NpmVersionNotFound)?;

        let mut visited: HashSet<(String, String)> = HashSet::new();
        let mut to_audit: HashMap<String, Vec<String>> = HashMap::new();
        let mut packument_cache: HashMap<String, Option<Arc<serde_json::Value>>> = HashMap::new();
        let mut cached_bytes = 0usize;
        let mut truncated = false;

        let deadline = tokio::time::Instant::now() + self.limits.deadline;
        let mut requests = 0usize;
        let (root_dependencies, root_overflow) = extract_dependencies(&root_version.manifest, self.limits.max_dependencies_per_manifest);
        truncated |= root_overflow;
        let mut queue: VecDeque<(String, String, usize)> = root_dependencies.into_iter().map(|(dep_name, range)| (dep_name, range, 1)).collect();

        while let Some((dep_name, range, depth)) = queue.pop_front() {
            if depth > MAX_DEPTH {
                truncated = true;
                continue;
            }
            if visited.len() >= MAX_PACKAGES || tokio::time::Instant::now() >= deadline {
                truncated = true;
                break;
            }

            let packument = match packument_cache.get(&dep_name) {
                Some(cached) => cached.clone(),
                None => {
                    let fetched = match NpmPackageName::parse(&dep_name) {
                        Ok(parsed) if self.packages.find_package(repository_id, &parsed).await?.is_some() => None,
                        // `.ok()` turns a fetch error into "unresolvable, skip it"; `.flatten()`
                        // additionally folds a genuine upstream 404 (`Ok(None)`) into the same
                        // "skip it" outcome, since either way there's no packument to resolve against.
                        Ok(parsed) => {
                            if requests >= self.limits.max_requests {
                                truncated = true;
                                break;
                            }
                            requests += 1;
                            match tokio::time::timeout_at(deadline, self.remote_registry.fetch_metadata(PUBLIC_REGISTRY_BASE_URL, &parsed, None, None)).await {
                                Ok(fetched) => fetched.ok().flatten().map(|full| slim_packument(&full)),
                                Err(_) => {
                                    truncated = true;
                                    break;
                                }
                            }
                        }
                        Err(_) => None,
                    };
                    let fetched = fetched.map(Arc::new);
                    let size = fetched.as_ref().map_or(0, |p| p.to_string().len());
                    if cached_bytes + size <= MAX_PACKUMENT_CACHE_BYTES {
                        cached_bytes += size;
                        packument_cache.insert(dep_name.clone(), fetched.clone());
                    }
                    fetched
                }
            };
            let Some(packument) = packument else { continue };
            let Some(versions) = packument.get("versions").and_then(|v| v.as_object()) else { continue };
            let Some((resolved_version, resolved_manifest)) = resolve_range(&range, versions) else { continue };

            if !visited.insert((dep_name.clone(), resolved_version.clone())) {
                continue;
            }

            to_audit.entry(dep_name.clone()).or_default().push(resolved_version.clone());

            let (children, overflow) = extract_dependencies(resolved_manifest, self.limits.max_dependencies_per_manifest);
            truncated |= overflow;
            for (child_name, child_range) in children {
                queue.push_back((child_name, child_range, depth + 1));
            }
        }

        let raw = if to_audit.is_empty() { serde_json::json!({}) } else { self.audit.check_bulk_raw(&to_audit).await? };
        let advisories_by_name = parse_advisories(&raw);

        let mut findings = Vec::new();
        for (dep_name, dep_version) in &visited {
            let Some(advisories) = advisories_by_name.get(dep_name) else { continue };
            for advisory in advisories {
                if version_satisfies_range(dep_version, &advisory.vulnerable_versions) {
                    findings.push(DependencyAuditFinding {
                        dependency_name: dep_name.clone(),
                        dependency_version: dep_version.clone(),
                        advisory: advisory.clone(),
                    });
                }
            }
        }
        findings.sort_by(|a, b| a.dependency_name.cmp(&b.dependency_name).then(a.dependency_version.cmp(&b.dependency_version)).then(a.advisory.id.cmp(&b.advisory.id)));

        let result = DependencyAuditResult {
            id: Uuid::new_v4(),
            npm_package_version_id: root_version.id,
            scanned_at: Utc::now(),
            packages_scanned: visited.len() as i32,
            truncated,
            findings,
        };
        self.results.save(&result).await?;
        Ok(result)
    }
}

/// Never triggers a fresh walk itself.
pub struct GetDependencyAuditResultUseCase {
    packages: Arc<dyn NpmPackageRepositoryPort>,
    results: Arc<dyn DependencyAuditRepositoryPort>,
}

impl GetDependencyAuditResultUseCase {
    pub fn new(packages: Arc<dyn NpmPackageRepositoryPort>, results: Arc<dyn DependencyAuditRepositoryPort>) -> Self {
        Self { packages, results }
    }

    pub async fn execute(
        &self,
        repository_id: Uuid,
        name: &NpmPackageName,
        version: &NpmVersion,
    ) -> Result<Option<DependencyAuditResult>, ApplicationError> {
        let Some(package) = self.packages.find_package(repository_id, name).await? else { return Ok(None) };
        let Some(root_version) = self.packages.find_version(package.id, version).await? else { return Ok(None) };
        Ok(self.results.find_latest_for_version(root_version.id).await?)
    }
}

/// Keeps only each version's `dependencies`, all the walk reads; a full packument can be tens of MB.
fn slim_packument(packument: &serde_json::Value) -> serde_json::Value {
    let versions: serde_json::Map<String, serde_json::Value> = packument
        .get("versions")
        .and_then(|v| v.as_object())
        .map(|versions| {
            versions
                .iter()
                .map(|(version, manifest)| {
                    let dependencies = manifest.get("dependencies").filter(|d| d.is_object()).cloned().unwrap_or(serde_json::Value::Null);
                    (version.clone(), serde_json::json!({ "dependencies": dependencies }))
                })
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({ "versions": versions })
}

/// At most `max` of the manifest's dependencies, and whether it listed more.
fn extract_dependencies(manifest: &serde_json::Value, max: usize) -> (Vec<(String, String)>, bool) {
    let Some(deps) = manifest.get("dependencies").and_then(|d| d.as_object()) else {
        return (Vec::new(), false);
    };
    let dependencies = deps.iter().filter_map(|(k, v)| v.as_str().map(|range| (k.clone(), range.to_string()))).take(max).collect();
    (dependencies, deps.len() > max)
}

/// node-semver treats a bare `"1.2.3"` as an exact match, not a caret range like Rust's `semver` crate — force an explicit `=` on. `x`/`X` wildcards normalize to `*`.
fn normalize_comparator(part: &str) -> String {
    let part = part.replace(['x', 'X'], "*");
    if part.starts_with(['^', '~', '>', '<', '=']) {
        part
    } else {
        format!("={part}")
    }
}

/// Unsupported syntax (hyphen ranges, git/file/tag references) returns `None` — the caller skips it.
fn parse_range_clause(clause: &str) -> Option<semver::VersionReq> {
    let clause = clause.trim();
    if clause.is_empty() || clause == "*" || clause == "latest" {
        return semver::VersionReq::parse("*").ok();
    }
    let normalized = clause.split_whitespace().map(normalize_comparator).collect::<Vec<_>>().join(", ");
    semver::VersionReq::parse(&normalized).ok()
}

/// Highest published version satisfying `range`, with its manifest for recursion.
fn resolve_range<'a>(range: &str, versions: &'a serde_json::Map<String, serde_json::Value>) -> Option<(String, &'a serde_json::Value)> {
    let mut best: Option<(semver::Version, String, &serde_json::Value)> = None;
    for clause in range.split("||") {
        let Some(req) = parse_range_clause(clause) else { continue };
        for (version_str, manifest) in versions {
            let Ok(parsed) = semver::Version::parse(version_str) else { continue };
            if req.matches(&parsed) && best.as_ref().is_none_or(|(current_best, _, _)| parsed > *current_best) {
                best = Some((parsed, version_str.clone(), manifest));
            }
        }
    }
    best.map(|(_, version_str, manifest)| (version_str, manifest))
}

fn version_satisfies_range(version_str: &str, range: &str) -> bool {
    let Ok(version) = semver::Version::parse(version_str) else { return false };
    range.split("||").filter_map(parse_range_clause).any(|req| req.matches(&version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::npm_test_support::{FakeDependencyAuditResults, FakeNpmAudit, FakePackages, FakeRemoteRegistry};
    use artiferris_domain::npm_audit::NpmAdvisory;
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageOrigin, NpmPackageVersion};

    fn manifest_with_deps(deps: &[(&str, &str)]) -> serde_json::Value {
        let mut deps_obj = serde_json::Map::new();
        for (name, range) in deps {
            deps_obj.insert(name.to_string(), serde_json::Value::String(range.to_string()));
        }
        serde_json::json!({ "dependencies": deps_obj })
    }

    fn packument(versions: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut versions_obj = serde_json::Map::new();
        for (v, manifest) in versions {
            versions_obj.insert(v.to_string(), manifest.clone());
        }
        serde_json::json!({ "dist-tags": {}, "versions": versions_obj })
    }

    fn sample_advisory(id: i64, vulnerable_versions: &str) -> NpmAdvisory {
        NpmAdvisory {
            id,
            url: format!("https://github.com/advisories/GHSA-{id}"),
            title: "Prototype Pollution".to_string(),
            severity: "critical".to_string(),
            vulnerable_versions: vulnerable_versions.to_string(),
            cwe: vec!["CWE-1321".to_string()],
            cvss_score: Some(9.8),
        }
    }

    struct Harness {
        packages: Arc<FakePackages>,
        remote: Arc<FakeRemoteRegistry>,
        audit: Arc<FakeNpmAudit>,
        results: Arc<FakeDependencyAuditResults>,
        repository_id: Uuid,
        name: NpmPackageName,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                packages: Arc::new(FakePackages::new()),
                remote: Arc::new(FakeRemoteRegistry::new()),
                audit: Arc::new(FakeNpmAudit::new(vec![])),
                results: Arc::new(FakeDependencyAuditResults::new()),
                repository_id: Uuid::new_v4(),
                name: NpmPackageName::parse("root-pkg").unwrap(),
            }
        }

        async fn seed_root(&self, manifest: serde_json::Value) -> Uuid {
            let package =
                NpmPackage { id: Uuid::new_v4(), package_repository_id: self.repository_id, name: self.name.clone(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
            self.packages.create_package(&package).await.unwrap();
            let version_id = Uuid::new_v4();
            self.packages
                .insert_version(&NpmPackageVersion {
                    id: version_id,
                    npm_package_id: package.id,
                    version: NpmVersion::parse("1.0.0").unwrap(),
                    manifest,
                    shasum: "s".to_string(),
                    integrity: "i".to_string(),
                    tarball_storage_key: "k".to_string(),
                    tarball_size_bytes: 1,
                    deprecated: false,
                    deprecated_message: None,
                    published_by: None,
                    published_at: Utc::now(),
                    origin: NpmPackageOrigin::Local,
                })
                .await
                .unwrap();
            version_id
        }

        fn use_case(&self) -> ScanDependencyTreeUseCase {
            ScanDependencyTreeUseCase::new(self.packages.clone(), self.remote.clone(), self.audit.clone(), self.results.clone())
        }
    }

    #[tokio::test]
    async fn a_direct_dependency_with_a_known_advisory_is_reported() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[("minimist", "^0.0.8")])).await;
        h.remote.set_package("minimist", packument(&[("0.0.8", serde_json::json!({}))]));
        h.audit.set_bulk_response(serde_json::json!({ "minimist": [sample_advisory(1, "<0.2.4")] }));

        let result = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert_eq!(result.packages_scanned, 1);
        assert!(!result.truncated);
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].dependency_name, "minimist");
        assert_eq!(result.findings[0].dependency_version, "0.0.8");
        assert_eq!(h.audit.checked_packages(), Some(HashMap::from([("minimist".to_string(), vec!["0.0.8".to_string()])])));
        assert_eq!(h.results.saved().len(), 1);
    }

    #[tokio::test]
    async fn a_transitive_dependency_two_levels_deep_is_walked_and_audited() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[("a", "^1.0.0")])).await;
        h.remote.set_package("a", packument(&[("1.0.0", manifest_with_deps(&[("b", "^2.0.0")]))]));
        h.remote.set_package("b", packument(&[("2.0.0", serde_json::json!({}))]));
        h.audit.set_bulk_response(serde_json::json!({ "b": [sample_advisory(2, "<3.0.0")] }));

        let result = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert_eq!(result.packages_scanned, 2);
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].dependency_name, "b");
        let checked = h.audit.checked_packages().unwrap();
        assert_eq!(checked.get("a"), Some(&vec!["1.0.0".to_string()]));
        assert_eq!(checked.get("b"), Some(&vec!["2.0.0".to_string()]));
    }

    #[tokio::test]
    async fn a_dependency_cycle_does_not_loop_forever_and_each_pair_is_visited_once() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[("a", "^1.0.0")])).await;
        h.remote.set_package("a", packument(&[("1.0.0", manifest_with_deps(&[("b", "^1.0.0")]))]));
        h.remote.set_package("b", packument(&[("1.0.0", manifest_with_deps(&[("a", "^1.0.0")]))]));

        let result = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert_eq!(result.packages_scanned, 2);
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn an_unresolvable_range_is_skipped_without_failing_the_scan() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[("a", "^9.0.0")])).await;
        h.remote.set_package("a", packument(&[("1.0.0", serde_json::json!({}))]));

        let result = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert_eq!(result.packages_scanned, 0);
        assert!(result.findings.is_empty());
    }

    #[tokio::test]
    async fn a_package_missing_from_the_public_registry_is_skipped_without_failing_the_scan() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[("totally-unpublished-pkg", "^1.0.0")])).await;
        *h.remote.metadata_response.lock().unwrap() = None;

        let result = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert_eq!(result.packages_scanned, 0);
    }

    #[tokio::test]
    async fn a_chain_deeper_than_the_depth_cap_is_reported_as_truncated() {
        let h = Harness::new();
        let chain_len = 15;
        h.seed_root(manifest_with_deps(&[("dep-0", "^1.0.0")])).await;
        for i in 0..chain_len {
            let next = format!("dep-{}", i + 1);
            let manifest = if i + 1 < chain_len { manifest_with_deps(&[(next.as_str(), "^1.0.0")]) } else { serde_json::json!({}) };
            h.remote.set_package(&format!("dep-{i}"), packument(&[("1.0.0", manifest)]));
        }

        let result = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert!(result.truncated);
        assert_eq!(result.packages_scanned, MAX_DEPTH as i32);
    }

    #[tokio::test]
    async fn more_unique_packages_than_the_count_cap_is_reported_as_truncated() {
        let h = Harness::new();
        let extra = MAX_PACKAGES + 5;
        let deps: Vec<(String, String)> = (0..extra).map(|i| (format!("dep-{i}"), "^1.0.0".to_string())).collect();
        let deps_refs: Vec<(&str, &str)> = deps.iter().map(|(n, r)| (n.as_str(), r.as_str())).collect();
        h.seed_root(manifest_with_deps(&deps_refs)).await;
        for (name, _) in &deps {
            h.remote.set_package(name, packument(&[("1.0.0", serde_json::json!({}))]));
        }

        let result = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert!(result.truncated);
        assert_eq!(result.packages_scanned, MAX_PACKAGES as i32);
    }

    #[tokio::test]
    async fn scanning_an_unknown_package_fails_with_not_found() {
        let h = Harness::new();
        let err = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::NpmPackageNotFound));
    }

    #[tokio::test]
    async fn scanning_an_unknown_version_fails_with_not_found() {
        let h = Harness::new();
        h.seed_root(serde_json::json!({})).await;
        let err = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("9.9.9").unwrap()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::NpmVersionNotFound));
    }

    #[tokio::test]
    async fn get_dependency_audit_result_returns_none_when_never_scanned() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[])).await;
        let use_case = GetDependencyAuditResultUseCase::new(h.packages.clone(), h.results.clone());

        let result = use_case.execute(h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_dependency_audit_result_returns_the_last_scan() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[("minimist", "^0.0.8")])).await;
        h.remote.set_package("minimist", packument(&[("0.0.8", serde_json::json!({}))]));

        h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();
        let use_case = GetDependencyAuditResultUseCase::new(h.packages.clone(), h.results.clone());
        let result = use_case.execute(h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap().packages_scanned, 1);
    }

    #[test]
    fn resolve_range_treats_a_bare_version_as_an_exact_match() {
        let versions = versions_map(&[("1.0.0", serde_json::json!({})), ("1.2.3", serde_json::json!({})), ("2.0.0", serde_json::json!({}))]);
        let (resolved, _) = resolve_range("1.2.3", &versions).unwrap();
        assert_eq!(resolved, "1.2.3");
    }

    #[test]
    fn resolve_range_picks_the_highest_version_matching_a_caret_range() {
        let versions = versions_map(&[("1.0.0", serde_json::json!({})), ("1.5.0", serde_json::json!({})), ("2.0.0", serde_json::json!({}))]);
        let (resolved, _) = resolve_range("^1.0.0", &versions).unwrap();
        assert_eq!(resolved, "1.5.0");
    }

    #[test]
    fn resolve_range_supports_tilde_ranges() {
        let versions = versions_map(&[("1.2.0", serde_json::json!({})), ("1.2.9", serde_json::json!({})), ("1.3.0", serde_json::json!({}))]);
        let (resolved, _) = resolve_range("~1.2.0", &versions).unwrap();
        assert_eq!(resolved, "1.2.9");
    }

    #[test]
    fn resolve_range_supports_or_combined_ranges() {
        let versions = versions_map(&[("1.0.0", serde_json::json!({})), ("3.0.0", serde_json::json!({}))]);
        let (resolved, _) = resolve_range("^1.0.0 || ^3.0.0", &versions).unwrap();
        assert_eq!(resolved, "3.0.0");
    }

    #[test]
    fn resolve_range_returns_none_when_nothing_matches() {
        let versions = versions_map(&[("1.0.0", serde_json::json!({}))]);
        assert!(resolve_range("^2.0.0", &versions).is_none());
    }

    #[test]
    fn version_satisfies_range_matches_a_vulnerable_versions_string() {
        assert!(version_satisfies_range("0.0.8", "<0.2.4"));
        assert!(!version_satisfies_range("1.2.8", "<0.2.4"));
    }

    fn versions_map(versions: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        for (v, m) in versions {
            map.insert(v.to_string(), m.clone());
        }
        map
    }

    #[tokio::test]
    async fn a_scan_after_a_publish_gives_way_when_scans_are_already_running() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[])).await;
        let use_case = h.use_case();
        let version = NpmVersion::parse("1.0.0").unwrap();
        let running = use_case.scans.clone().try_acquire_many_owned(MAX_CONCURRENT_SCANS as u32).unwrap();

        assert!(use_case.execute_unless_busy(h.repository_id, &h.name, &version).await.unwrap().is_none());
        assert!(h.results.saved().is_empty());

        drop(running);
        assert!(use_case.execute_unless_busy(h.repository_id, &h.name, &version).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_dependency_this_repository_publishes_itself_is_never_looked_up_on_the_public_registry() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[("corp-internal", "^1.0.0")])).await;
        let internal = NpmPackage { id: Uuid::new_v4(), package_repository_id: h.repository_id, name: NpmPackageName::parse("corp-internal").unwrap(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        h.packages.create_package(&internal).await.unwrap();
        h.remote.set_package("corp-internal", packument(&[("1.0.0", serde_json::json!({}))]));

        let result = h.use_case().execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert_eq!(result.packages_scanned, 0);
        assert_eq!(h.audit.checked_packages(), None, "nothing of it goes to the advisory service either");
    }

    #[test]
    fn a_trimmed_packument_keeps_the_dependencies_and_drops_the_rest() {
        let full = serde_json::json!({
            "name": "a", "readme": "x".repeat(1000),
            "versions": { "1.0.0": { "name": "a", "description": "big", "dependencies": { "b": "^1" }, "dist": { "tarball": "t" } }, "2.0.0": { "name": "a" } }
        });

        let slim = slim_packument(&full);

        assert_eq!(slim, serde_json::json!({ "versions": { "1.0.0": { "dependencies": { "b": "^1" } }, "2.0.0": { "dependencies": null } } }));
    }

    fn many_unknown_dependencies(count: usize) -> serde_json::Value {
        let names: Vec<String> = (0..count).map(|i| format!("no-such-package-{i}")).collect();
        let deps: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "^1.0.0")).collect();
        manifest_with_deps(&deps)
    }

    fn limits() -> ScanLimits {
        ScanLimits::default()
    }

    #[tokio::test]
    async fn dependency_names_that_resolve_to_nothing_still_count_against_the_request_budget() {
        let h = Harness::new();
        h.seed_root(many_unknown_dependencies(200)).await;
        *h.remote.metadata_response.lock().unwrap() = None;
        let use_case = h.use_case().with_limits(ScanLimits { max_requests: 25, ..limits() });

        let result = use_case.execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert_eq!(h.remote.metadata_fetches.load(std::sync::atomic::Ordering::SeqCst), 25, "one request per unknown name, up to the budget");
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn a_manifest_with_more_dependencies_than_the_cap_is_read_only_up_to_it() {
        let h = Harness::new();
        h.seed_root(many_unknown_dependencies(50)).await;
        *h.remote.metadata_response.lock().unwrap() = None;
        let use_case = h.use_case().with_limits(ScanLimits { max_dependencies_per_manifest: 10, ..limits() });

        let result = use_case.execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert_eq!(h.remote.metadata_fetches.load(std::sync::atomic::Ordering::SeqCst), 10);
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn a_walk_that_outlasts_its_deadline_stops_and_is_marked_truncated() {
        let h = Harness::new();
        h.seed_root(many_unknown_dependencies(100)).await;
        *h.remote.metadata_response.lock().unwrap() = None;
        *h.remote.metadata_delay.lock().unwrap() = Duration::from_millis(40);
        let use_case = h.use_case().with_limits(ScanLimits { deadline: Duration::from_millis(150), ..limits() });

        let started = Instant::now();
        let result = use_case.execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await.unwrap();

        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
        assert!(h.remote.metadata_fetches.load(std::sync::atomic::Ordering::SeqCst) < 100);
        assert!(result.truncated);
        assert_eq!(h.results.saved().len(), 1, "what was found is still saved");
    }

    #[tokio::test]
    async fn a_manual_rescan_gives_up_when_every_scan_slot_stays_taken() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[])).await;
        let use_case = h.use_case().with_limits(ScanLimits { slot_wait: Duration::from_millis(50), ..limits() });
        let _running = use_case.scans.clone().try_acquire_many_owned(MAX_CONCURRENT_SCANS as u32).unwrap();

        let result = use_case.execute(Uuid::new_v4(), h.repository_id, &h.name, &NpmVersion::parse("1.0.0").unwrap()).await;

        assert!(matches!(result, Err(ApplicationError::DependencyScanBusy)), "{result:?}");
    }

    #[tokio::test]
    async fn one_user_cannot_ask_for_a_rescan_of_the_same_repository_over_and_over() {
        let h = Harness::new();
        h.seed_root(manifest_with_deps(&[])).await;
        let use_case = h.use_case();
        let version = NpmVersion::parse("1.0.0").unwrap();
        let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());

        use_case.execute(alice, h.repository_id, &h.name, &version).await.unwrap();
        let again = use_case.execute(alice, h.repository_id, &h.name, &version).await;
        let someone_else = use_case.execute(bob, h.repository_id, &h.name, &version).await;
        let other_repository = use_case.execute(alice, Uuid::new_v4(), &h.name, &version).await;

        assert!(matches!(again, Err(ApplicationError::DependencyScanRateLimited)), "{again:?}");
        assert!(someone_else.is_ok());
        assert!(!matches!(other_repository, Err(ApplicationError::DependencyScanRateLimited)), "another repository has its own allowance");
    }
}
