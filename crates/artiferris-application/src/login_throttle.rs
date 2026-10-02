//! Rate limiting for login attempts. The counts live in a `LoginAttemptStorePort`: the database in a deployment, so
//! that several instances share one budget per key, or `InMemoryLoginAttemptStore` for tests and one-off tools.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use artiferris_domain::error::DomainError;
use artiferris_domain::login_attempts::{AttemptBudget, BlockedKey, LoginAttemptStorePort};
use async_trait::async_trait;

pub const MAX_LOGIN_ATTEMPTS: usize = 10;
pub const LOGIN_ATTEMPT_WINDOW: Duration = Duration::from_secs(300);
/// Evicts the quietest entry once reached, bounding memory against an attacker cycling through unbounded distinct usernames.
pub const MAX_TRACKED_USERNAMES: usize = 10_000;
/// Longest username kept in a throttle key or an audit entry. Real ones are far shorter; this covers an email-style LDAP login.
pub const MAX_LOGIN_IDENTIFIER_LEN: usize = 254;
/// `blocked_usernames` never returns more than this many entries, however many keys are blocked.
pub const MAX_LISTED_BLOCKED: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedUsername {
    pub username: String,
    pub remaining_seconds: u64,
}

/// A key's timestamps plus the threshold and window it was last used with. Eviction judges each key against its own threshold, and a later caller's shorter window can't shorten how long earlier failures count.
struct TrackedKey {
    timestamps: Vec<Instant>,
    max_attempts: usize,
    window: Duration,
}

impl TrackedKey {
    fn new(max_attempts: usize, window: Duration) -> Self {
        Self { timestamps: Vec::new(), max_attempts, window }
    }

    /// Drops what's older than the longer of the stored and given windows.
    fn prune(&mut self, window: Duration) {
        let effective = self.window.max(window);
        prune(&mut self.timestamps, effective);
        self.window = if self.timestamps.is_empty() { window } else { effective };
    }
}

/// The key every login on the instance counts a username under, whichever organization's host it came in on.
pub fn shared_username_key(username: &str) -> String {
    format!("login-user:{username}")
}

/// An organization's own login policy counts here, a key only a login made on that organization's host can reach.
pub fn organization_username_key(organization_id: uuid::Uuid, username: &str) -> String {
    format!("login-org-user:{organization_id}:{username}")
}

#[derive(Clone)]
pub struct InMemoryLoginAttemptStore {
    attempts: Arc<Mutex<HashMap<String, TrackedKey>>>,
}

impl Default for InMemoryLoginAttemptStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryLoginAttemptStore {
    pub fn new() -> Self {
        Self { attempts: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Expired entries are pruned lazily here, not by a background sweeper.
    fn is_throttled_now(&self, key: &str, max_attempts: usize, window: Duration) -> bool {
        let mut attempts = self.lock();
        let Some(tracked) = attempts.get_mut(key) else {
            return false;
        };
        tracked.prune(window);
        if tracked.timestamps.is_empty() {
            attempts.remove(key);
            return false;
        }
        tracked.timestamps.len() >= max_attempts
    }

    /// Counts the attempt against every budget in one step, or counts nothing and returns `false` if one is full.
    /// Reserving before the password check is what stops parallel requests from all passing it. `release` gives a
    /// reservation back, `clear` wipes a key.
    fn reserve_all_now(&self, budgets: &[AttemptBudget<'_>]) -> bool {
        let mut attempts = self.lock();
        for &(key, max_attempts, window) in budgets {
            if let Some(tracked) = attempts.get_mut(key) {
                tracked.prune(window);
                if tracked.timestamps.len() >= max_attempts {
                    return false;
                }
            }
        }
        for &(key, max_attempts, window) in budgets {
            if !make_room(&mut attempts, key) {
                continue; // every tracked key is actively blocked; let this one through untracked rather than unblocking another
            }
            let tracked = attempts.entry(key.to_string()).or_insert_with(|| TrackedKey::new(max_attempts, window));
            tracked.max_attempts = max_attempts;
            tracked.prune(window);
            tracked.timestamps.push(Instant::now());
        }
        true
    }

    /// Gives back one earlier `reserve`, for a key that must not be charged for a successful attempt (a shared IP).
    fn release_now(&self, key: &str) {
        let mut attempts = self.lock();
        if let Some(tracked) = attempts.get_mut(key) {
            tracked.timestamps.pop();
            if tracked.timestamps.is_empty() {
                attempts.remove(key);
            }
        }
    }

    fn record_failure_now(&self, key: &str, max_attempts: usize, window: Duration) {
        let mut attempts = self.lock();
        if !make_room(&mut attempts, key) {
            return; // every tracked key is actively blocked; drop this attempt rather than unblocking one
        }
        let tracked = attempts.entry(key.to_string()).or_insert_with(|| TrackedKey::new(max_attempts, window));
        tracked.max_attempts = max_attempts;
        tracked.prune(window);
        if tracked.timestamps.len() <= max_attempts {
            tracked.timestamps.push(Instant::now());
        }
    }

    fn clear_now(&self, key: &str) {
        self.lock().remove(key);
    }

    /// Wipes every login key that counts `username`: the shared one and each organization's own.
    fn clear_username_now(&self, username: &str) {
        let shared = shared_username_key(username);
        self.lock().retain(|key, _| key != &shared && !is_organization_key_of(key, username));
    }

    /// Every currently blocked key against the given limits.
    fn blocked_now(&self, max_attempts: usize, window: Duration) -> Vec<BlockedKey> {
        let mut attempts = self.lock();
        let now = Instant::now();
        let mut blocked = Vec::new();
        attempts.retain(|username, tracked| {
            tracked.prune(window);
            if tracked.timestamps.is_empty() {
                return false;
            }
            if tracked.timestamps.len() >= max_attempts {
                let oldest = *tracked.timestamps.iter().min().expect("just checked non-empty");
                let remaining = tracked.window.saturating_sub(now.duration_since(oldest));
                blocked.push(BlockedKey { key: username.clone(), remaining_seconds: remaining.as_secs() });
            }
            true
        });
        blocked.sort_by_key(|b| std::cmp::Reverse(b.remaining_seconds));
        blocked
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TrackedKey>> {
        self.attempts.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl LoginAttemptStorePort for InMemoryLoginAttemptStore {
    async fn reserve_all(&self, budgets: &[AttemptBudget<'_>]) -> Result<bool, DomainError> {
        Ok(self.reserve_all_now(budgets))
    }

    async fn is_throttled(&self, key: &str, max_attempts: usize, window: Duration) -> Result<bool, DomainError> {
        Ok(self.is_throttled_now(key, max_attempts, window))
    }

    async fn record_failure(&self, key: &str, max_attempts: usize, window: Duration) -> Result<(), DomainError> {
        self.record_failure_now(key, max_attempts, window);
        Ok(())
    }

    async fn release(&self, key: &str) -> Result<(), DomainError> {
        self.release_now(key);
        Ok(())
    }

    async fn clear(&self, key: &str) -> Result<(), DomainError> {
        self.clear_now(key);
        Ok(())
    }

    async fn clear_username(&self, username: &str) -> Result<(), DomainError> {
        self.clear_username_now(username);
        Ok(())
    }

    async fn blocked(&self, max_attempts: usize, window: Duration, limit: usize) -> Result<Vec<BlockedKey>, DomainError> {
        let mut blocked = self.blocked_now(max_attempts, window);
        blocked.truncate(limit);
        Ok(blocked)
    }
}

/// What the handlers count attempts with. A store that fails closes the door: a login that cannot be counted is
/// refused rather than let through uncounted, and the failure is logged.
#[derive(Clone)]
pub struct LoginThrottle {
    store: Arc<dyn LoginAttemptStorePort>,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl LoginThrottle {
    pub fn new(store: Arc<dyn LoginAttemptStorePort>) -> Self {
        Self { store }
    }

    /// Per instance and forgotten on restart: for tests, and for budgets that only need to hold on one instance.
    pub fn in_memory() -> Self {
        Self::new(Arc::new(InMemoryLoginAttemptStore::new()))
    }

    pub async fn is_throttled(&self, key: &str, max_attempts: usize, window: Duration) -> bool {
        self.store.is_throttled(key, max_attempts, window).await.unwrap_or_else(|e| {
            tracing::error!("login throttle store failed, treating the key as throttled: {e}");
            true
        })
    }

    /// Counts the attempt against every budget in one step, or counts nothing and returns `false` if one is full.
    /// Reserving before the password check is what stops parallel requests from all passing it. `release` gives a
    /// reservation back, `clear` wipes a key.
    pub async fn reserve_all(&self, budgets: &[AttemptBudget<'_>]) -> bool {
        self.store.reserve_all(budgets).await.unwrap_or_else(|e| {
            tracing::error!("login throttle store failed, refusing the attempt: {e}");
            false
        })
    }

    pub async fn reserve(&self, key: &str, max_attempts: usize, window: Duration) -> bool {
        self.reserve_all(&[(key, max_attempts, window)]).await
    }

    /// Gives back one earlier `reserve`, for a key that must not be charged for a successful attempt (a shared IP).
    pub async fn release(&self, key: &str) {
        if let Err(e) = self.store.release(key).await {
            tracing::error!("login throttle store failed to release an attempt: {e}");
        }
    }

    pub async fn record_failure(&self, key: &str, max_attempts: usize, window: Duration) {
        if let Err(e) = self.store.record_failure(key, max_attempts, window).await {
            tracing::error!("login throttle store failed to record a failure: {e}");
        }
    }

    pub async fn clear(&self, key: &str) {
        if let Err(e) = self.store.clear(key).await {
            tracing::error!("login throttle store failed to clear a key: {e}");
        }
    }

    /// Wipes every login key that counts `username`: the shared one and each organization's own.
    pub async fn clear_username(&self, username: &str) {
        if let Err(e) = self.store.clear_username(username).await {
            tracing::error!("login throttle store failed to clear a username: {e}");
        }
    }

    /// Every currently blocked key against the given limits.
    pub async fn blocked_usernames(&self, max_attempts: usize, window: Duration) -> Vec<BlockedUsername> {
        match self.store.blocked(max_attempts, window, MAX_LISTED_BLOCKED).await {
            Ok(blocked) => blocked.into_iter().map(|b| BlockedUsername { username: b.key, remaining_seconds: b.remaining_seconds }).collect(),
            Err(e) => {
                tracing::error!("login throttle store failed to list the blocked keys: {e}");
                Vec::new()
            }
        }
    }
}

/// Makes room for a new `key` when the map is full by dropping the quietest key that is not blocked, judged against its
/// own threshold, so flooding fresh keys cannot unblock a victim. `false` if every key is blocked.
fn make_room(attempts: &mut HashMap<String, TrackedKey>, key: &str) -> bool {
    if attempts.contains_key(key) || attempts.len() < MAX_TRACKED_USERNAMES {
        return true;
    }
    let evict_key = attempts
        .iter()
        .filter(|(_, tracked)| tracked.timestamps.len() < tracked.max_attempts)
        .min_by_key(|(_, tracked)| tracked.timestamps.last())
        .map(|(key, _)| key.clone());
    match evict_key {
        Some(evict_key) => {
            attempts.remove(&evict_key);
            true
        }
        None => false,
    }
}

/// The login name a throttle key counts, for the keys that count one (the shared one and an organization's own).
pub fn username_in_key(key: &str) -> Option<&str> {
    const SHARED_PREFIX: &str = "login-user:";
    const ORGANIZATION_PREFIX: &str = "login-org-user:";
    const UUID_LEN: usize = 36;
    key.strip_prefix(SHARED_PREFIX).or_else(|| key.strip_prefix(ORGANIZATION_PREFIX).and_then(|rest| rest.get(UUID_LEN..)).and_then(|rest| rest.strip_prefix(':')))
}

fn is_organization_key_of(key: &str, username: &str) -> bool {
    const PREFIX: &str = "login-org-user:";
    const UUID_LEN: usize = 36;
    key.strip_prefix(PREFIX).and_then(|rest| rest.get(UUID_LEN..)).and_then(|rest| rest.strip_prefix(':')).is_some_and(|typed| typed == username)
}

/// Cuts an attacker-supplied login name down to `MAX_LOGIN_IDENTIFIER_LEN` characters before it becomes a throttle key or audit field.
pub fn bounded_identifier(raw: &str) -> String {
    raw.chars().take(MAX_LOGIN_IDENTIFIER_LEN).collect()
}

fn prune(timestamps: &mut Vec<Instant>, window: Duration) {
    let now = Instant::now();
    timestamps.retain(|at| now.duration_since(*at) < window);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted_throttle() -> (LoginThrottle, Arc<InMemoryLoginAttemptStore>) {
        let store = Arc::new(InMemoryLoginAttemptStore::new());
        (LoginThrottle::new(store.clone()), store)
    }

    #[tokio::test]
    async fn a_fresh_username_is_not_throttled() {
        let throttle = LoginThrottle::in_memory();
        assert!(!throttle.is_throttled("florian", MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[tokio::test]
    async fn stays_below_the_threshold_until_the_limit_is_reached() {
        let throttle = LoginThrottle::in_memory();
        throttle.record_failure("florian", 3, LOGIN_ATTEMPT_WINDOW).await;
        throttle.record_failure("florian", 3, LOGIN_ATTEMPT_WINDOW).await;
        assert!(!throttle.is_throttled("florian", 3, LOGIN_ATTEMPT_WINDOW).await);
        throttle.record_failure("florian", 3, LOGIN_ATTEMPT_WINDOW).await;
        assert!(throttle.is_throttled("florian", 3, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[tokio::test]
    async fn throttling_is_scoped_to_one_username() {
        let throttle = LoginThrottle::in_memory();
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW).await;
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW).await;
        assert!(throttle.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
        assert!(!throttle.is_throttled("someone-else", 2, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[tokio::test]
    async fn clearing_resets_the_tally() {
        let throttle = LoginThrottle::in_memory();
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW).await;
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW).await;
        assert!(throttle.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
        throttle.clear("florian").await;
        assert!(!throttle.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[tokio::test]
    async fn lists_blocked_usernames_but_not_ones_below_the_threshold() {
        let throttle = LoginThrottle::in_memory();
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW).await;
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW).await;
        throttle.record_failure("almost-blocked", 2, LOGIN_ATTEMPT_WINDOW).await;

        let blocked = throttle.blocked_usernames(2, LOGIN_ATTEMPT_WINDOW).await;

        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].username, "florian");
        assert!(blocked[0].remaining_seconds > 0 && blocked[0].remaining_seconds <= LOGIN_ATTEMPT_WINDOW.as_secs());
    }

    #[tokio::test]
    async fn blocked_usernames_is_empty_when_nobody_is_throttled() {
        let throttle = LoginThrottle::in_memory();
        throttle.record_failure("florian", MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW).await;
        assert!(throttle.blocked_usernames(MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW).await.is_empty());
    }

    #[tokio::test]
    async fn a_username_drops_off_blocked_usernames_once_its_window_expires() {
        let throttle = LoginThrottle::in_memory();
        throttle.record_failure("florian", 2, Duration::from_millis(30)).await;
        throttle.record_failure("florian", 2, Duration::from_millis(30)).await;
        assert_eq!(throttle.blocked_usernames(2, Duration::from_millis(30)).await.len(), 1);

        tokio::time::sleep(Duration::from_millis(60)).await;

        assert!(throttle.blocked_usernames(2, Duration::from_millis(30)).await.is_empty());
    }

    #[tokio::test]
    async fn failures_older_than_the_window_are_pruned() {
        let throttle = LoginThrottle::in_memory();
        throttle.record_failure("florian", 2, Duration::from_millis(30)).await;
        throttle.record_failure("florian", 2, Duration::from_millis(30)).await;
        assert!(throttle.is_throttled("florian", 2, Duration::from_millis(30)).await);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(!throttle.is_throttled("florian", 2, Duration::from_millis(30)).await);
    }

    #[tokio::test]
    async fn eviction_never_unblocks_an_actively_blocked_key() {
        let throttle = LoginThrottle::in_memory();
        throttle.record_failure("victim", 2, LOGIN_ATTEMPT_WINDOW).await;
        throttle.record_failure("victim", 2, LOGIN_ATTEMPT_WINDOW).await;
        assert!(throttle.is_throttled("victim", 2, LOGIN_ATTEMPT_WINDOW).await);

        for i in 0..MAX_TRACKED_USERNAMES {
            throttle.record_failure(&format!("attacker-{i}"), MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW).await;
        }

        assert!(throttle.is_throttled("victim", 2, LOGIN_ATTEMPT_WINDOW).await, "an actively-blocked key must never be evicted to make room for new attempts");
    }

    #[tokio::test]
    async fn the_tracked_username_map_never_grows_past_the_cap() {
        let (throttle, store) = counted_throttle();
        for i in 0..(MAX_TRACKED_USERNAMES + 50) {
            throttle.record_failure(&format!("attacker-{i}"), MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW).await;
            assert!(store.len() <= MAX_TRACKED_USERNAMES);
        }
        assert_eq!(store.len(), MAX_TRACKED_USERNAMES);

        assert!(!throttle.is_throttled("attacker-0", MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[tokio::test]
    async fn reserving_stops_at_the_limit() {
        let throttle = LoginThrottle::in_memory();
        assert!(throttle.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
        assert!(throttle.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
        assert!(!throttle.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
        assert!(throttle.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[tokio::test]
    async fn a_burst_of_concurrent_reservations_never_exceeds_the_limit() {
        let throttle = LoginThrottle::in_memory();
        let handles: Vec<_> = (0..64)
            .map(|_| {
                let throttle = throttle.clone();
                tokio::spawn(async move { throttle.reserve("florian", 10, LOGIN_ATTEMPT_WINDOW).await })
            })
            .collect();

        let mut granted = 0;
        for handle in handles {
            granted += usize::from(handle.await.unwrap());
        }

        assert_eq!(granted, 10);
    }

    #[tokio::test]
    async fn a_full_budget_among_several_reserves_nothing_from_the_others() {
        let (throttle, store) = counted_throttle();
        throttle.reserve("ip", 1, LOGIN_ATTEMPT_WINDOW).await;

        assert!(!throttle.reserve_all(&[("user", 5, LOGIN_ATTEMPT_WINDOW), ("ip", 1, LOGIN_ATTEMPT_WINDOW)]).await);

        assert!(!throttle.is_throttled("user", 1, LOGIN_ATTEMPT_WINDOW).await, "the refused request must not have been charged to the other key");
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn releasing_gives_one_reservation_back() {
        let (throttle, store) = counted_throttle();
        throttle.reserve("ip", 2, LOGIN_ATTEMPT_WINDOW).await;
        throttle.reserve("ip", 2, LOGIN_ATTEMPT_WINDOW).await;
        assert!(throttle.is_throttled("ip", 2, LOGIN_ATTEMPT_WINDOW).await);

        throttle.release("ip").await;

        assert!(!throttle.is_throttled("ip", 2, LOGIN_ATTEMPT_WINDOW).await);
        throttle.release("ip").await;
        assert_eq!(store.len(), 0);
    }

    #[tokio::test]
    async fn reserving_never_evicts_an_actively_blocked_key() {
        let (throttle, store) = counted_throttle();
        throttle.reserve("victim", 1, LOGIN_ATTEMPT_WINDOW).await;
        for i in 0..MAX_TRACKED_USERNAMES {
            throttle.reserve(&format!("attacker-{i}"), MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW).await;
        }

        assert!(throttle.is_throttled("victim", 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(store.len() <= MAX_TRACKED_USERNAMES);
    }

    #[tokio::test]
    async fn the_blocked_listing_is_capped() {
        let throttle = LoginThrottle::in_memory();
        for i in 0..(MAX_LISTED_BLOCKED + 50) {
            throttle.reserve(&format!("blocked-{i}"), 1, LOGIN_ATTEMPT_WINDOW).await;
        }

        assert_eq!(throttle.blocked_usernames(1, LOGIN_ATTEMPT_WINDOW).await.len(), MAX_LISTED_BLOCKED);
    }

    #[tokio::test]
    async fn a_shorter_window_from_another_caller_does_not_erase_earlier_failures() {
        let throttle = LoginThrottle::in_memory();
        for _ in 0..3 {
            assert!(throttle.reserve("victim", 3, LOGIN_ATTEMPT_WINDOW).await);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(!throttle.reserve("victim", 3, Duration::from_millis(5)).await, "a caller with a 5 ms window must still see the lock");
        assert!(throttle.is_throttled("victim", 3, Duration::from_millis(5)).await);
        assert_eq!(throttle.blocked_usernames(3, Duration::from_millis(5)).await.len(), 1);
    }

    #[tokio::test]
    async fn a_longer_window_from_a_later_caller_keeps_failures_longer() {
        let throttle = LoginThrottle::in_memory();
        throttle.reserve("key", 2, Duration::from_millis(5)).await;
        throttle.reserve("key", 2, Duration::from_secs(60)).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(throttle.is_throttled("key", 2, Duration::from_millis(5)).await, "the longer window seen once keeps counting");
    }

    #[tokio::test]
    async fn a_key_whose_failures_all_expired_takes_the_new_window() {
        let throttle = LoginThrottle::in_memory();
        throttle.reserve("key", 1, Duration::from_millis(50)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(throttle.reserve("key", 1, Duration::from_millis(10)).await);
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert!(throttle.reserve("key", 1, Duration::from_millis(10)).await, "a key that emptied out must not keep an old, longer window");
    }

    #[tokio::test]
    async fn clearing_a_username_wipes_its_shared_and_every_organization_key_and_nothing_else() {
        let throttle = LoginThrottle::in_memory();
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        for key in [shared_username_key("alice"), organization_username_key(first, "alice"), organization_username_key(second, "alice"), shared_username_key("bob"), organization_username_key(first, "bob"), organization_username_key(first, "x:alice")] {
            throttle.record_failure(&key, 1, LOGIN_ATTEMPT_WINDOW).await;
        }

        throttle.clear_username("alice").await;

        assert!(!throttle.is_throttled(&shared_username_key("alice"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(!throttle.is_throttled(&organization_username_key(first, "alice"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(!throttle.is_throttled(&organization_username_key(second, "alice"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(throttle.is_throttled(&shared_username_key("bob"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(throttle.is_throttled(&organization_username_key(first, "bob"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(throttle.is_throttled(&organization_username_key(first, "x:alice"), 1, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[tokio::test]
    async fn the_shared_and_per_organization_keys_never_collide() {
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());

        assert_eq!(shared_username_key("alice"), "login-user:alice");
        assert_ne!(organization_username_key(first, "alice"), organization_username_key(second, "alice"));
        assert_ne!(organization_username_key(first, "alice"), shared_username_key("alice"));
    }

    #[test]
    fn the_username_is_read_back_from_the_keys_that_count_one() {
        let organization = uuid::Uuid::new_v4();

        assert_eq!(username_in_key(&shared_username_key("alice")), Some("alice"));
        assert_eq!(username_in_key(&organization_username_key(organization, "x:alice")), Some("x:alice"));
        assert_eq!(username_in_key("login-ip:1.2.3.4"), None);
        assert_eq!(username_in_key("alice"), None);
    }

    #[tokio::test]
    async fn a_login_identifier_is_cut_to_the_maximum_length() {
        assert_eq!(bounded_identifier(&"a".repeat(10_000)).chars().count(), MAX_LOGIN_IDENTIFIER_LEN);
        assert_eq!(bounded_identifier("florian"), "florian");
        assert_eq!(bounded_identifier(&"é".repeat(300)).chars().count(), MAX_LOGIN_IDENTIFIER_LEN);
    }
}
