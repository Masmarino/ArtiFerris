//! Per-client request budgets for traffic nobody signed in for, shared between the instances of a deployment.
//!
//! Each instance counts the requests it serves in memory and decides from that count plus what the others last
//! reported, so a request never waits for the database. A background loop (`sync_once`) publishes the instance's own
//! counts and reads the others' every couple of seconds: the database sees a few batched statements, whatever the
//! traffic. A flood therefore overshoots the limit by at most what arrives between two syncs, and a store that does not
//! answer leaves every instance limiting on its own. Counters live in two sliding windows, so a restart keeps the
//! others' view of the budget and a burst at a window boundary is smoothed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use artiferris_domain::error::DomainError;
use artiferris_domain::rate_limit::{RateLimitCount, RateLimitKey, RateLimitStorePort};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Length of a counting window.
pub const WINDOW: Duration = Duration::from_secs(60);
/// How often an instance should call `sync_once`.
pub const SYNC_INTERVAL: Duration = Duration::from_secs(2);

/// What a refused client is told to wait, in seconds (`Retry-After`).
pub const RETRY_AFTER_SECONDS: u64 = 30;

/// Keys tracked at once. Past this, an unseen key is let through untracked rather than refused: an attacker cycling
/// addresses must not be able to lock real clients out by filling the table.
const MAX_KEYS: usize = 100_000;
/// Keys one sync publishes and refreshes; the rest wait for the next one.
const MAX_SYNC_KEYS: usize = 5_000;

/// Requests per minute and client for anonymous reads of the npm and Docker registries. Generous: a CI farm behind one
/// address pulls many images at once, and a Docker pull is dozens of requests.
pub const DEFAULT_ANONYMOUS_REGISTRY_READS_PER_MINUTE: usize = 1_200;

/// `ANONYMOUS_REGISTRY_READS_PER_MINUTE`: unset or empty keeps the default, `0` removes the limit, anything else is the
/// number of requests per minute and client. A typo fails startup instead of silently dropping the limit.
pub fn parse_anonymous_registry_reads_per_minute(raw: Option<&str>) -> Result<usize, String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(DEFAULT_ANONYMOUS_REGISTRY_READS_PER_MINUTE),
        Some(value) => value.parse::<usize>().map_err(|_| format!("ANONYMOUS_REGISTRY_READS_PER_MINUTE must be a whole number of requests per minute (0 turns the limit off), got {value:?}")),
    }
}

type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

#[derive(Default)]
struct Entry {
    window: u64,
    local: u32,
    remote: u32,
    previous_local: u32,
    previous_remote: u32,
    dirty: bool,
}

impl Entry {
    fn roll_to(&mut self, window: u64) {
        if self.window == window {
            return;
        }
        if self.window + 1 == window {
            self.previous_local = self.local;
            self.previous_remote = self.remote;
        } else {
            self.previous_local = 0;
            self.previous_remote = 0;
        }
        self.local = 0;
        self.remote = 0;
        self.window = window;
    }
}

pub struct RateLimiter {
    instance: Uuid,
    hash_secret: Vec<u8>,
    clock: Clock,
    entries: Mutex<HashMap<String, Entry>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(b"")
    }
}

impl RateLimiter {
    /// `hash_secret` keys the hash of client keys, so what the store holds cannot be turned back into addresses.
    /// Instances of one deployment must share it.
    pub fn new(hash_secret: &[u8]) -> Self {
        Self::with_clock(hash_secret, Arc::new(|| SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()))
    }

    fn with_clock(hash_secret: &[u8], clock: Clock) -> Self {
        Self { instance: Uuid::new_v4(), hash_secret: hash_secret.to_vec(), clock, entries: Mutex::new(HashMap::new()) }
    }

    /// Spends one request of `key`'s budget of `limit` per minute; `false` once it is used up. A refused request does
    /// not count. `limit == 0` means no limit.
    pub fn allow(&self, key: &str, limit: usize) -> bool {
        if limit == 0 {
            return true;
        }
        let now = (self.clock)();
        let window = now.as_secs() / WINDOW.as_secs();
        let elapsed = (now.as_secs_f64() % WINDOW.as_secs_f64()) / WINDOW.as_secs_f64();
        let mut entries = self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !entries.contains_key(key) && entries.len() >= MAX_KEYS {
            entries.retain(|_, entry| entry.window + 1 >= window);
            if entries.len() >= MAX_KEYS {
                return true;
            }
        }
        let entry = entries.entry(key.to_string()).or_insert_with(|| Entry { window, ..Entry::default() });
        entry.roll_to(window);
        let spent = f64::from(entry.local) + f64::from(entry.remote) + (f64::from(entry.previous_local) + f64::from(entry.previous_remote)) * (1.0 - elapsed);
        if spent >= limit as f64 {
            return false;
        }
        entry.local = entry.local.saturating_add(1);
        entry.dirty = true;
        true
    }

    fn hash(&self, key: &str) -> RateLimitKey {
        let digest = Sha256::new().chain_update(&self.hash_secret).chain_update(b"\0").chain_update(key.as_bytes()).finalize();
        let mut hashed = [0u8; 16];
        hashed.copy_from_slice(&digest[..16]);
        hashed
    }

    /// Publishes this instance's counts and takes in what the others have counted. Call it every `SYNC_INTERVAL`.
    /// On an error nothing is lost: the counts stay in memory and go out with the next call.
    pub async fn sync_once(&self, store: &dyn RateLimitStorePort) -> Result<(), DomainError> {
        let now = (self.clock)();
        let window = now.as_secs() / WINDOW.as_secs();
        let (publish, tracked, keys_by_hash) = {
            let mut entries = self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            entries.retain(|_, entry| entry.window + 1 >= window);
            let mut ordered: Vec<(&String, &mut Entry)> = entries.iter_mut().collect();
            ordered.sort_by_key(|(_, entry)| !entry.dirty);
            let mut publish: Vec<RateLimitCount> = Vec::new();
            let mut tracked: Vec<RateLimitKey> = Vec::new();
            let mut keys_by_hash: HashMap<RateLimitKey, String> = HashMap::new();
            for (key, entry) in ordered.into_iter().take(MAX_SYNC_KEYS) {
                let hashed = self.hash(key);
                if entry.dirty {
                    publish.push((hashed, entry.window, entry.local));
                    entry.dirty = false;
                }
                tracked.push(hashed);
                keys_by_hash.insert(hashed, key.clone());
            }
            (publish, tracked, keys_by_hash)
        };
        if tracked.is_empty() {
            return Ok(());
        }
        if let Err(e) = store.publish(self.instance, &publish).await {
            self.mark_dirty(&keys_by_hash, &publish);
            return Err(e);
        }
        let others = store.others(self.instance, &tracked, [window, window.saturating_sub(1)]).await?;
        let mut entries = self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for (hashed, counted_window, count) in others {
            let Some(entry) = keys_by_hash.get(&hashed).and_then(|key| entries.get_mut(key)) else { continue };
            if entry.window == counted_window {
                entry.remote = count;
            } else if entry.window == counted_window + 1 {
                entry.previous_remote = count;
            }
        }
        Ok(())
    }

    fn mark_dirty(&self, keys_by_hash: &HashMap<RateLimitKey, String>, published: &[RateLimitCount]) {
        let mut entries = self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for (hashed, _, _) in published {
            if let Some(entry) = keys_by_hash.get(hashed).and_then(|key| entries.get_mut(key)) {
                entry.dirty = true;
            }
        }
    }

    /// Removes the store's counters of windows nobody reads any more.
    pub async fn purge(&self, store: &dyn RateLimitStorePort) -> Result<u64, DomainError> {
        let window = (self.clock)().as_secs() / WINDOW.as_secs();
        store.purge_before(window.saturating_sub(2)).await
    }

    /// Keys tracked right now, for tests and metrics.
    pub fn tracked_keys(&self) -> usize {
        self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn clock_at(start_millis: u64) -> (Arc<AtomicU64>, Clock) {
        let millis = Arc::new(AtomicU64::new(start_millis));
        let handle = millis.clone();
        (millis, Arc::new(move || Duration::from_millis(handle.load(Ordering::SeqCst))))
    }

    /// A store two limiters of one deployment share, in memory.
    #[derive(Default)]
    struct SharedStore {
        counts: Mutex<HashMap<(RateLimitKey, u64, Uuid), u32>>,
        failing: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl RateLimitStorePort for SharedStore {
        async fn publish(&self, instance: Uuid, counts: &[RateLimitCount]) -> Result<(), DomainError> {
            if self.failing.load(Ordering::SeqCst) {
                return Err(DomainError::Infrastructure("store is down".to_string()));
            }
            let mut stored = self.counts.lock().unwrap();
            for (key, window, count) in counts {
                stored.insert((*key, *window, instance), *count);
            }
            Ok(())
        }
        async fn others(&self, instance: Uuid, keys: &[RateLimitKey], windows: [u64; 2]) -> Result<Vec<RateLimitCount>, DomainError> {
            if self.failing.load(Ordering::SeqCst) {
                return Err(DomainError::Infrastructure("store is down".to_string()));
            }
            let stored = self.counts.lock().unwrap();
            let mut sums: HashMap<(RateLimitKey, u64), u32> = HashMap::new();
            for ((key, window, owner), count) in stored.iter() {
                if *owner != instance && keys.contains(key) && windows.contains(window) {
                    *sums.entry((*key, *window)).or_default() += count;
                }
            }
            Ok(sums.into_iter().map(|((key, window), count)| (key, window, count)).collect())
        }
        async fn purge_before(&self, window: u64) -> Result<u64, DomainError> {
            let mut stored = self.counts.lock().unwrap();
            let before = stored.len();
            stored.retain(|(_, w, _), _| *w >= window);
            Ok((before - stored.len()) as u64)
        }
    }

    #[test]
    fn a_client_gets_its_budget_per_minute_and_a_refusal_costs_nothing() {
        let (_, clock) = clock_at(600_000);
        let limiter = RateLimiter::with_clock(b"s", clock);

        let allowed = (0..10).filter(|_| limiter.allow("a", 5)).count();

        assert_eq!(allowed, 5);
        assert!(limiter.allow("b", 5), "another client has its own budget");
    }

    #[test]
    fn zero_means_no_limit() {
        let limiter = RateLimiter::default();
        assert!((0..1000).all(|_| limiter.allow("a", 0)));
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn the_budget_comes_back_as_the_window_slides() {
        let (millis, clock) = clock_at(600_000);
        let limiter = RateLimiter::with_clock(b"s", clock);
        assert_eq!((0..10).filter(|_| limiter.allow("a", 10)).count(), 10);
        assert!(!limiter.allow("a", 10));

        millis.store(600_000 + 30_000, Ordering::SeqCst);
        assert!(!limiter.allow("a", 10), "half a window later half of the last window still counts");

        millis.store(600_000 + 90_000, Ordering::SeqCst);
        let allowed = (0..10).filter(|_| limiter.allow("a", 10)).count();
        assert!((4..=6).contains(&allowed), "half a window into the next one about half the budget is back, got {allowed}");

        millis.store(600_000 + 200_000, Ordering::SeqCst);
        assert_eq!((0..10).filter(|_| limiter.allow("a", 10)).count(), 10);
    }

    #[test]
    fn a_full_table_lets_an_unseen_client_through_instead_of_refusing_it() {
        let limiter = RateLimiter::default();
        {
            let mut entries = limiter.entries.lock().unwrap();
            for index in 0..MAX_KEYS {
                entries.insert(format!("k{index}"), Entry { window: u64::MAX / 2, ..Entry::default() });
            }
        }
        assert!(limiter.allow("someone-new", 1));
        assert!(limiter.allow("someone-new", 1), "untracked, so never limited");
        assert_eq!(limiter.tracked_keys(), MAX_KEYS);
    }

    #[tokio::test]
    async fn two_instances_share_one_budget_once_they_have_synced() {
        let (_, clock) = clock_at(600_000);
        let store = SharedStore::default();
        let first = RateLimiter::with_clock(b"same", clock.clone());
        let second = RateLimiter::with_clock(b"same", clock);

        assert_eq!((0..60).filter(|_| first.allow("client", 100)).count(), 60);
        first.sync_once(&store).await.unwrap();
        assert!(second.allow("client", 100), "an instance meeting a client for the first time has not heard of the others yet");
        second.sync_once(&store).await.unwrap();

        let allowed_on_second = 1 + (0..100).filter(|_| second.allow("client", 100)).count();
        assert_eq!(allowed_on_second, 40, "the first instance already spent 60 of the 100");
    }

    #[tokio::test]
    async fn a_restarted_instance_still_sees_what_the_others_counted() {
        let (_, clock) = clock_at(600_000);
        let store = SharedStore::default();
        let survivor = RateLimiter::with_clock(b"same", clock.clone());
        for _ in 0..90 {
            survivor.allow("client", 100);
        }
        survivor.sync_once(&store).await.unwrap();

        let restarted = RateLimiter::with_clock(b"same", clock);
        restarted.allow("client", 100);
        restarted.sync_once(&store).await.unwrap();

        assert_eq!((0..100).filter(|_| restarted.allow("client", 100)).count(), 9);
    }

    #[tokio::test]
    async fn the_store_never_sees_a_client_key_and_a_different_secret_gives_different_hashes() {
        let (_, clock) = clock_at(600_000);
        let store = SharedStore::default();
        let limiter = RateLimiter::with_clock(b"secret", clock.clone());
        limiter.allow("203.0.113.9", 10);
        limiter.sync_once(&store).await.unwrap();

        let stored = store.counts.lock().unwrap();
        let (key, _, _) = stored.keys().next().unwrap();
        assert_ne!(key.as_slice(), &b"203.0.113.9"[..16.min(11)]);
        assert_ne!(RateLimiter::with_clock(b"other", clock).hash("203.0.113.9"), *key);
    }

    #[tokio::test]
    async fn a_store_that_is_down_leaves_each_instance_limiting_alone_and_nothing_is_lost() {
        let (_, clock) = clock_at(600_000);
        let store = SharedStore::default();
        store.failing.store(true, Ordering::SeqCst);
        let limiter = RateLimiter::with_clock(b"s", clock);
        for _ in 0..30 {
            limiter.allow("client", 100);
        }

        assert!(limiter.sync_once(&store).await.is_err());
        assert_eq!((0..100).filter(|_| limiter.allow("client", 100)).count(), 70, "still limits on its own count");

        store.failing.store(false, Ordering::SeqCst);
        limiter.sync_once(&store).await.unwrap();
        let stored = store.counts.lock().unwrap();
        assert_eq!(stored.values().copied().max(), Some(100), "the counts published once the store is back are the full ones");
    }

    #[tokio::test]
    async fn purging_drops_old_windows_only() {
        let (millis, clock) = clock_at(600_000);
        let store = SharedStore::default();
        let limiter = RateLimiter::with_clock(b"s", clock);
        limiter.allow("old", 10);
        limiter.sync_once(&store).await.unwrap();
        millis.store(600_000 + 300_000, Ordering::SeqCst);
        limiter.allow("new", 10);
        limiter.sync_once(&store).await.unwrap();

        assert_eq!(limiter.purge(&store).await.unwrap(), 1);
        assert_eq!(store.counts.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_configured_limit_defaults_turns_off_and_refuses_a_typo() {
        assert_eq!(parse_anonymous_registry_reads_per_minute(None), Ok(DEFAULT_ANONYMOUS_REGISTRY_READS_PER_MINUTE));
        assert_eq!(parse_anonymous_registry_reads_per_minute(Some("  ")), Ok(DEFAULT_ANONYMOUS_REGISTRY_READS_PER_MINUTE));
        assert_eq!(parse_anonymous_registry_reads_per_minute(Some("0")), Ok(0));
        assert_eq!(parse_anonymous_registry_reads_per_minute(Some(" 300 ")), Ok(300));
        for bad in ["many", "-5", "1.5"] {
            assert!(parse_anonymous_registry_reads_per_minute(Some(bad)).is_err(), "{bad}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_instances_share_a_budget_through_postgres(pool: sqlx::PgPool) {
        let (_, clock) = clock_at(600_000);
        let store = artiferris_infrastructure::postgres::rate_limit_store::PostgresRateLimitStore::new(pool);
        let first = RateLimiter::with_clock(b"same", clock.clone());
        let second = RateLimiter::with_clock(b"same", clock);
        assert_eq!((0..70).filter(|_| first.allow("client", 100)).count(), 70);
        assert!(second.allow("client", 100));

        first.sync_once(&store).await.unwrap();
        second.sync_once(&store).await.unwrap();
        first.sync_once(&store).await.unwrap();

        let more_on_second = (0..100).filter(|_| second.allow("client", 100)).count();
        let more_on_first = (0..100).filter(|_| first.allow("client", 100)).count();
        assert_eq!(more_on_second, 29, "70 on the first instance plus the 1 here leave 29");
        assert_eq!(more_on_first, 29, "until the next sync an instance does not know what the other did meanwhile");

        first.sync_once(&store).await.unwrap();
        second.sync_once(&store).await.unwrap();
        first.sync_once(&store).await.unwrap();
        assert!(!first.allow("client", 100) && !second.allow("client", 100), "once synced, both see the budget spent");
    }
}
