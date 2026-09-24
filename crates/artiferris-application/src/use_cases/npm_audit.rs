use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use artiferris_domain::error::DomainError;
use artiferris_domain::npm_audit::{NpmAdvisory, NpmAuditPort};
use artiferris_domain::npm_package::{NpmPackageName, NpmPackageRepositoryPort};
use tokio::sync::{OwnedMutexGuard, Semaphore, SemaphorePermit};
use uuid::Uuid;

use crate::error::ApplicationError;

const AUDIT_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// Results for repositories anyone can read: the only ones an anonymous caller ever gets.
const MAX_PUBLIC_CACHED_AUDITS: usize = 200;
const MAX_OTHER_CACHED_AUDITS: usize = 800;
/// What one account can hold of the public partition, so that a few accounts cannot push every other result out.
const MAX_PUBLIC_CACHED_AUDITS_PER_USER: usize = 20;
/// Audits running against npm at the same time.
const MAX_OUTBOUND_AUDITS: usize = 4;
/// A request that waits longer than this for its turn gets a "busy" answer.
const QUEUE_TIMEOUT: Duration = Duration::from_secs(10);
/// Only the newest versions of a package are sent to npm.
pub const MAX_AUDITED_VERSIONS: usize = 200;

type CacheKey = (Uuid, String);

struct CachedAudit {
    /// The versions that were checked: a new publication makes the entry stale.
    versions: Vec<String>,
    checked_at: Instant,
    advisories: Vec<NpmAdvisory>,
    requested_by: Uuid,
    public: bool,
}

struct Limits {
    public_entries: usize,
    other_entries: usize,
    public_entries_per_user: usize,
    queue_timeout: Duration,
}

/// Who asks for an audit, and whether the repository is readable by anyone.
pub struct AuditRequest {
    pub user_id: Uuid,
    pub repository_is_public: bool,
}

enum Turn<'a> {
    Cached(Vec<NpmAdvisory>),
    Go(OwnedMutexGuard<()>, SemaphorePermit<'a>),
}

/// A package that doesn't exist audits clean rather than erroring.
pub struct AuditNpmPackageUseCase {
    packages: Arc<dyn NpmPackageRepositoryPort>,
    audit: Arc<dyn NpmAuditPort>,
    cache: Mutex<HashMap<CacheKey, CachedAudit>>,
    /// One lock per package being audited, so that concurrent requests for it wait for the first result instead of each calling npm.
    in_flight: Mutex<HashMap<CacheKey, Arc<tokio::sync::Mutex<()>>>>,
    ttl: Duration,
    limits: Limits,
    outbound: Semaphore,
}

impl AuditNpmPackageUseCase {
    pub fn new(packages: Arc<dyn NpmPackageRepositoryPort>, audit: Arc<dyn NpmAuditPort>) -> Self {
        Self::with_settings(
            packages,
            audit,
            AUDIT_CACHE_TTL,
            Limits {
                public_entries: MAX_PUBLIC_CACHED_AUDITS,
                other_entries: MAX_OTHER_CACHED_AUDITS,
                public_entries_per_user: MAX_PUBLIC_CACHED_AUDITS_PER_USER,
                queue_timeout: QUEUE_TIMEOUT,
            },
        )
    }

    fn with_settings(packages: Arc<dyn NpmPackageRepositoryPort>, audit: Arc<dyn NpmAuditPort>, ttl: Duration, limits: Limits) -> Self {
        Self { packages, audit, cache: Mutex::new(HashMap::new()), in_flight: Mutex::new(HashMap::new()), ttl, limits, outbound: Semaphore::new(MAX_OUTBOUND_AUDITS) }
    }

    /// A recent result, if there is one: for callers who may not trigger a call to npm.
    pub fn cached(&self, repository_id: Uuid, name: &NpmPackageName) -> Option<Vec<NpmAdvisory>> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.get(&(repository_id, name.as_str().to_string())).filter(|entry| entry.checked_at.elapsed() < self.ttl).map(|entry| entry.advisories.clone())
    }

    fn fresh(&self, key: &CacheKey, versions: &[String]) -> Option<Vec<NpmAdvisory>> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.get(key).filter(|entry| entry.checked_at.elapsed() < self.ttl && entry.versions == versions).map(|entry| entry.advisories.clone())
    }

    pub async fn execute(&self, repository_id: Uuid, name: &NpmPackageName, request: AuditRequest) -> Result<Vec<NpmAdvisory>, ApplicationError> {
        let Some(package) = self.packages.find_package(repository_id, name).await? else {
            return Ok(Vec::new());
        };
        let versions = self.packages.list_latest_versions_for_packages(&[package.id], MAX_AUDITED_VERSIONS as i64).await?;
        let mut versions: Vec<_> = versions.into_iter().map(|v| v.version).collect();
        versions.sort_by_key(|v| v.as_str());
        let listed: Vec<String> = versions.iter().map(|v| v.as_str()).collect();
        let key = (repository_id, name.as_str().to_string());
        if let Some(advisories) = self.fresh(&key, &listed) {
            return Ok(advisories);
        }
        let flight = self.in_flight.lock().unwrap_or_else(|p| p.into_inner()).entry(key.clone()).or_default().clone();
        let result = self.audit_when_its_turn(&key, name, &versions, listed, &request, &flight).await;
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|p| p.into_inner());
        if Arc::strong_count(&flight) <= 2 {
            in_flight.remove(&key);
        }
        result
    }

    async fn audit_when_its_turn(
        &self,
        key: &CacheKey,
        name: &NpmPackageName,
        versions: &[artiferris_domain::npm_package::NpmVersion],
        listed: Vec<String>,
        request: &AuditRequest,
        flight: &Arc<tokio::sync::Mutex<()>>,
    ) -> Result<Vec<NpmAdvisory>, ApplicationError> {
        let turn = tokio::time::timeout(self.limits.queue_timeout, async {
            let flight_guard = flight.clone().lock_owned().await;
            // Whoever held the lock before may have just stored the result.
            if let Some(advisories) = self.fresh(key, &listed) {
                return Turn::Cached(advisories);
            }
            Turn::Go(flight_guard, self.outbound.acquire().await.expect("the semaphore is never closed"))
        })
        .await;
        let (_flight_guard, _slot) = match turn {
            Ok(Turn::Cached(advisories)) => return Ok(advisories),
            Ok(Turn::Go(flight_guard, slot)) => (flight_guard, slot),
            Err(_) => return Err(DomainError::Busy("npm audits are waiting for a free slot".to_string()).into()),
        };
        let advisories = self.audit.check(name, versions).await?;
        self.store(key.clone(), CachedAudit { versions: listed, checked_at: Instant::now(), advisories: advisories.clone(), requested_by: request.user_id, public: request.repository_is_public });
        Ok(advisories)
    }

    /// Room is made by dropping expired entries, then the oldest ones of the same partition: a full cache never refuses a result.
    fn store(&self, key: CacheKey, entry: CachedAudit) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.remove(&key);
        let (cap, public) = if entry.public { (self.limits.public_entries, true) } else { (self.limits.other_entries, false) };
        let in_partition = move |e: &CachedAudit| e.public == public;
        if cache.values().filter(|e| in_partition(e)).count() >= cap {
            cache.retain(|_, e| !in_partition(e) || e.checked_at.elapsed() < self.ttl);
        }
        if public {
            let user = entry.requested_by;
            while cache.values().filter(|e| in_partition(e) && e.requested_by == user).count() >= self.limits.public_entries_per_user {
                if !evict_oldest(&mut cache, |e| in_partition(e) && e.requested_by == user) {
                    break;
                }
            }
        }
        while cache.values().filter(|e| in_partition(e)).count() >= cap {
            if !evict_oldest(&mut cache, in_partition) {
                break;
            }
        }
        cache.insert(key, entry);
    }
}

fn evict_oldest(cache: &mut HashMap<CacheKey, CachedAudit>, matching: impl Fn(&CachedAudit) -> bool) -> bool {
    let oldest = cache.iter().filter(|(_, e)| matching(e)).min_by_key(|(_, e)| e.checked_at).map(|(key, _)| key.clone());
    oldest.is_some_and(|key| cache.remove(&key).is_some())
}

/// Unlike `AuditNpmPackageUseCase`, forwards the client's request straight to npm's advisory database rather than looking anything up in ArtiFerris's own storage.
pub struct BulkAuditNpmPackagesUseCase {
    audit: Arc<dyn NpmAuditPort>,
    outbound: Semaphore,
}

impl BulkAuditNpmPackagesUseCase {
    pub fn new(audit: Arc<dyn NpmAuditPort>) -> Self {
        Self { audit, outbound: Semaphore::new(MAX_OUTBOUND_AUDITS) }
    }

    pub async fn execute(&self, packages: &HashMap<String, Vec<String>>) -> Result<serde_json::Value, ApplicationError> {
        let _slot = self.outbound.acquire().await.expect("the semaphore is never closed");
        Ok(self.audit.check_bulk_raw(packages).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::npm_test_support::{FakeNpmAudit, FakePackages};
    use chrono::Utc;
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageOrigin, NpmPackageVersion, NpmVersion};
    use artiferris_domain::npm_audit::NpmAuditPort;

    fn anyone() -> AuditRequest {
        AuditRequest { user_id: Uuid::nil(), repository_is_public: true }
    }

    fn limits() -> Limits {
        Limits { public_entries: MAX_PUBLIC_CACHED_AUDITS, other_entries: MAX_OTHER_CACHED_AUDITS, public_entries_per_user: MAX_PUBLIC_CACHED_AUDITS_PER_USER, queue_timeout: QUEUE_TIMEOUT }
    }

    fn sample_advisory(id: i64) -> NpmAdvisory {
        NpmAdvisory {
            id,
            url: format!("https://github.com/advisories/GHSA-{id}"),
            title: "Prototype Pollution".to_string(),
            severity: "critical".to_string(),
            vulnerable_versions: "<1.0.0".to_string(),
            cwe: vec!["CWE-1321".to_string()],
            cvss_score: Some(9.8),
        }
    }

    #[tokio::test]
    async fn audits_every_stored_version_of_the_package() {
        let packages = Arc::new(FakePackages::new());
        let audit = Arc::new(FakeNpmAudit::new(vec![sample_advisory(1)]));
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
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
        packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("1.0.0").unwrap(),
                manifest: serde_json::json!({}),
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

        let use_case = AuditNpmPackageUseCase::new(packages, audit.clone());
        let advisories = use_case.execute(repository_id, &name, anyone()).await.unwrap();

        assert_eq!(advisories.len(), 1);
        assert_eq!(advisories[0].id, 1);
        assert_eq!(audit.checked_versions(), vec!["1.0.0".to_string()]);
    }

    #[tokio::test]
    async fn an_unknown_package_audits_clean_without_calling_the_audit_port() {
        let packages = Arc::new(FakePackages::new());
        let audit = Arc::new(FakeNpmAudit::new(vec![sample_advisory(1)]));

        let use_case = AuditNpmPackageUseCase::new(packages, audit.clone());
        let advisories = use_case.execute(Uuid::new_v4(), &NpmPackageName::parse("left-pad").unwrap(), anyone()).await.unwrap();

        assert!(advisories.is_empty());
        assert!(audit.checked_versions().is_empty());
    }

    #[tokio::test]
    async fn bulk_audit_forwards_exactly_what_the_client_sent_to_the_audit_port() {
        let audit = Arc::new(FakeNpmAudit::new(vec![sample_advisory(1)]));
        let mut packages = HashMap::new();
        packages.insert("left-pad".to_string(), vec!["1.0.0".to_string(), "1.1.0".to_string()]);
        packages.insert("minimist".to_string(), vec!["0.0.8".to_string()]);

        let use_case = BulkAuditNpmPackagesUseCase::new(audit.clone());
        let result = use_case.execute(&packages).await.unwrap();

        assert_eq!(audit.checked_packages(), Some(packages));
        assert!(result.get("fake-package").is_some());
    }

    #[tokio::test]
    async fn bulk_audits_running_against_npm_at_once_are_capped() {
        let audit = Arc::new(FakeNpmAudit::new(vec![]));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        audit.set_bulk_gate(gate.clone());
        let use_case = Arc::new(BulkAuditNpmPackagesUseCase::new(audit.clone()));
        let calls: Vec<_> = (0..MAX_OUTBOUND_AUDITS + 3)
            .map(|_| {
                let use_case = use_case.clone();
                tokio::spawn(async move { use_case.execute(&HashMap::from([("left-pad".to_string(), vec!["1.0.0".to_string()])])).await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert_eq!(audit.bulk_in_flight(), MAX_OUTBOUND_AUDITS, "the rest wait for a slot");
        gate.add_permits(MAX_OUTBOUND_AUDITS + 3);
        for call in calls {
            call.await.unwrap().unwrap();
        }
        assert_eq!(audit.bulk_max_in_flight(), MAX_OUTBOUND_AUDITS);
    }

    async fn package_with_versions(versions: &[&str]) -> (Arc<FakePackages>, Arc<FakeNpmAudit>, Uuid, NpmPackageName, Uuid) {
        let packages = Arc::new(FakePackages::new());
        let audit = Arc::new(FakeNpmAudit::new(vec![sample_advisory(1)]));
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repository_id, name: name.clone(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        packages.create_package(&package).await.unwrap();
        for version in versions {
            publish(&packages, package.id, version).await;
        }
        (packages, audit, repository_id, name, package.id)
    }

    async fn publish(packages: &FakePackages, package_id: Uuid, version: &str) {
        publish_at(packages, package_id, version, Utc::now()).await;
    }

    async fn publish_at(packages: &FakePackages, package_id: Uuid, version: &str, published_at: chrono::DateTime<Utc>) {
        packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package_id,
                version: NpmVersion::parse(version).unwrap(),
                manifest: serde_json::json!({}),
                shasum: "s".to_string(),
                integrity: "i".to_string(),
                tarball_storage_key: "k".to_string(),
                tarball_size_bytes: 1,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at,
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_second_audit_of_the_same_versions_does_not_call_npm_again() {
        let (packages, audit, repository_id, name, _) = package_with_versions(&["1.0.0"]).await;
        let use_case = AuditNpmPackageUseCase::new(packages, audit.clone());

        assert!(use_case.cached(repository_id, &name).is_none(), "nothing yet for a caller who may not trigger an audit");
        let first = use_case.execute(repository_id, &name, anyone()).await.unwrap();
        let second = use_case.execute(repository_id, &name, anyone()).await.unwrap();

        assert_eq!((first.len(), second.len(), audit.check_calls()), (1, 1, 1));
        assert_eq!(use_case.cached(repository_id, &name).map(|a| a.len()), Some(1));
    }

    #[tokio::test]
    async fn a_new_version_or_an_expired_entry_is_audited_again_but_the_cache_is_per_repository() {
        let (packages, audit, repository_id, name, package_id) = package_with_versions(&["1.0.0"]).await;
        let use_case = AuditNpmPackageUseCase::with_settings(packages.clone(), audit.clone(), Duration::from_millis(300), limits());
        use_case.execute(repository_id, &name, anyone()).await.unwrap();

        publish(&packages, package_id, "1.1.0").await;
        use_case.execute(repository_id, &name, anyone()).await.unwrap();
        assert_eq!(audit.check_calls(), 2, "a publication changes what there is to audit");

        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(use_case.cached(repository_id, &name).is_none());
        use_case.execute(repository_id, &name, anyone()).await.unwrap();
        assert_eq!(audit.check_calls(), 3);
        assert!(use_case.cached(Uuid::new_v4(), &name).is_none(), "another repository's entry is not this one's");
    }

    /// Blocks every `check` until the test opens the gate.
    struct GatedAudit {
        gate: Semaphore,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl GatedAudit {
        fn closed() -> Arc<Self> {
            Arc::new(Self { gate: Semaphore::new(0), calls: std::sync::atomic::AtomicUsize::new(0) })
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl NpmAuditPort for GatedAudit {
        async fn check(&self, _name: &NpmPackageName, _versions: &[NpmVersion]) -> Result<Vec<NpmAdvisory>, artiferris_domain::error::DomainError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.gate.acquire().await.unwrap().forget();
            Ok(vec![sample_advisory(1)])
        }

        async fn check_bulk_raw(&self, _packages: &HashMap<String, Vec<String>>) -> Result<serde_json::Value, artiferris_domain::error::DomainError> {
            unreachable!()
        }
    }

    async fn wait_until(what: &str, condition: impl Fn() -> bool) {
        for _ in 0..500 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn add_package(packages: &FakePackages, repository_id: Uuid, name: &str) -> NpmPackageName {
        let parsed = NpmPackageName::parse(name).unwrap();
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repository_id, name: parsed.clone(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        packages.create_package(&package).await.unwrap();
        publish(packages, package.id, "1.0.0").await;
        parsed
    }

    #[tokio::test]
    async fn concurrent_audits_of_one_package_make_a_single_call_to_npm() {
        let (packages, _, repository_id, name, _) = package_with_versions(&["1.0.0"]).await;
        let audit = GatedAudit::closed();
        let use_case = Arc::new(AuditNpmPackageUseCase::new(packages, audit.clone()));

        let mut requests = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let (use_case, name) = (use_case.clone(), name.clone());
            requests.spawn(async move { use_case.execute(repository_id, &name, anyone()).await });
        }
        wait_until("the first call to start", || audit.calls() == 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(audit.calls(), 1, "the others wait for it instead of queueing their own call");
        audit.gate.add_permits(1);

        while let Some(result) = requests.join_next().await {
            assert_eq!(result.unwrap().unwrap().len(), 1);
        }
        assert_eq!(audit.calls(), 1);
        assert!(use_case.in_flight.lock().unwrap().is_empty(), "the per-package locks are cleaned up");
    }

    #[tokio::test]
    async fn a_request_that_waits_too_long_for_a_slot_gets_a_busy_answer() {
        let packages = Arc::new(FakePackages::new());
        let repository_id = Uuid::new_v4();
        let audit = GatedAudit::closed();
        let use_case = Arc::new(AuditNpmPackageUseCase::with_settings(packages.clone(), audit.clone(), AUDIT_CACHE_TTL, Limits { queue_timeout: Duration::from_millis(150), ..limits() }));
        let mut stuck = tokio::task::JoinSet::new();
        for i in 0..MAX_OUTBOUND_AUDITS {
            let name = add_package(&packages, repository_id, &format!("stuck-{i}")).await;
            let use_case = use_case.clone();
            stuck.spawn(async move { use_case.execute(repository_id, &name, anyone()).await });
        }
        wait_until("every slot to be taken", || audit.calls() == MAX_OUTBOUND_AUDITS).await;
        let waiting = add_package(&packages, repository_id, "waiting").await;

        let started = Instant::now();
        let result = use_case.execute(repository_id, &waiting, anyone()).await;

        assert!(matches!(result, Err(ApplicationError::Domain(DomainError::Busy(_)))), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(use_case.in_flight.lock().unwrap().len() <= MAX_OUTBOUND_AUDITS, "the abandoned wait leaves no lock behind");
        audit.gate.add_permits(MAX_OUTBOUND_AUDITS);
        while stuck.join_next().await.is_some() {}
    }

    #[tokio::test]
    async fn a_full_cache_drops_its_oldest_entry_instead_of_refusing_the_new_one() {
        let packages = Arc::new(FakePackages::new());
        let audit = Arc::new(FakeNpmAudit::new(vec![sample_advisory(1)]));
        let repository_id = Uuid::new_v4();
        let use_case = AuditNpmPackageUseCase::with_settings(packages.clone(), audit.clone(), AUDIT_CACHE_TTL, Limits { public_entries: 3, public_entries_per_user: 3, ..limits() });
        let mut names = Vec::new();
        for i in 0..4 {
            let name = add_package(&packages, repository_id, &format!("pkg-{i}")).await;
            use_case.execute(repository_id, &name, anyone()).await.unwrap();
            names.push(name);
        }

        assert!(use_case.cached(repository_id, &names[0]).is_none(), "the oldest one made room");
        assert!(names[1..].iter().all(|name| use_case.cached(repository_id, name).is_some()), "the newest result is cached, like the ones before it");
        assert_eq!(use_case.cache.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn results_for_other_repositories_cannot_push_the_public_ones_out() {
        let packages = Arc::new(FakePackages::new());
        let audit = Arc::new(FakeNpmAudit::new(vec![sample_advisory(1)]));
        let public_repository = Uuid::new_v4();
        let private_repository = Uuid::new_v4();
        let use_case = AuditNpmPackageUseCase::with_settings(packages.clone(), audit, AUDIT_CACHE_TTL, Limits { other_entries: 2, ..limits() });
        let public_name = add_package(&packages, public_repository, "public-one").await;
        use_case.execute(public_repository, &public_name, anyone()).await.unwrap();

        for i in 0..5 {
            let name = add_package(&packages, private_repository, &format!("private-{i}")).await;
            use_case.execute(private_repository, &name, AuditRequest { user_id: Uuid::new_v4(), repository_is_public: false }).await.unwrap();
        }

        assert!(use_case.cached(public_repository, &public_name).is_some());
        assert_eq!(use_case.cache.lock().unwrap().len(), 3, "the public entry and the two newest private ones");
    }

    #[tokio::test]
    async fn one_account_cannot_fill_the_public_partition_alone() {
        let packages = Arc::new(FakePackages::new());
        let audit = Arc::new(FakeNpmAudit::new(vec![sample_advisory(1)]));
        let repository_id = Uuid::new_v4();
        let use_case = AuditNpmPackageUseCase::with_settings(packages.clone(), audit, AUDIT_CACHE_TTL, Limits { public_entries: 10, public_entries_per_user: 2, ..limits() });
        let (greedy, other) = (Uuid::new_v4(), Uuid::new_v4());
        let others_name = add_package(&packages, repository_id, "others").await;
        use_case.execute(repository_id, &others_name, AuditRequest { user_id: other, repository_is_public: true }).await.unwrap();

        for i in 0..6 {
            let name = add_package(&packages, repository_id, &format!("greedy-{i}")).await;
            use_case.execute(repository_id, &name, AuditRequest { user_id: greedy, repository_is_public: true }).await.unwrap();
        }

        assert!(use_case.cached(repository_id, &others_name).is_some());
        assert_eq!(use_case.cache.lock().unwrap().values().filter(|e| e.requested_by == greedy).count(), 2);
    }

    #[tokio::test]
    async fn only_the_newest_versions_are_read_and_sent_to_npm() {
        let (packages, audit, repository_id, name, package_id) = package_with_versions(&[]).await;
        for i in 0..300 {
            publish_at(&packages, package_id, &format!("1.0.{i}"), Utc::now() - chrono::Duration::seconds(1000 - i)).await;
        }
        let use_case = AuditNpmPackageUseCase::new(packages.clone(), audit.clone());

        use_case.execute(repository_id, &name, anyone()).await.unwrap();

        let checked = audit.checked_versions();
        assert_eq!(checked.len(), MAX_AUDITED_VERSIONS);
        assert!(checked.contains(&"1.0.299".to_string()) && !checked.contains(&"1.0.0".to_string()), "the newest ones");
        assert!(packages.capped_summaries_returned.load(std::sync::atomic::Ordering::Relaxed) <= MAX_AUDITED_VERSIONS, "the cap is applied by the query, not after loading every version");
    }
}
