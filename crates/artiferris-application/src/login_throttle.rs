//! Rate limiting for login attempts. In-memory only, per-instance — fine for a self-hosted tool, not a distributed one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_LOGIN_ATTEMPTS: usize = 10;
pub const LOGIN_ATTEMPT_WINDOW: Duration = Duration::from_secs(300);
/// Evicts the quietest entry once reached, bounding memory against an attacker cycling through unbounded distinct usernames.
pub const MAX_TRACKED_USERNAMES: usize = 10_000;
/// Longest username kept in a throttle key or an audit entry. Real ones are far shorter; this covers an email-style LDAP login.
pub const MAX_LOGIN_IDENTIFIER_LEN: usize = 254;
/// `blocked_usernames` never returns more than this many entries, however many keys are blocked.
const MAX_LISTED_BLOCKED: usize = 1_000;

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
pub struct LoginThrottle {
    attempts: Arc<Mutex<HashMap<String, TrackedKey>>>,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginThrottle {
    pub fn new() -> Self {
        Self { attempts: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Expired entries are pruned lazily here, not by a background sweeper.
    pub fn is_throttled(&self, key: &str, max_attempts: usize, window: Duration) -> bool {
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

    /// Counts the attempt against every budget in one step, or counts nothing and returns `false` if any of them is already full.
    /// Reserving before the password check (rather than recording a failure after it) is what stops a burst of parallel requests from all passing the check first.
    /// Give back a reservation with `release`, or wipe a key with `clear`, once the attempt turns out fine.
    pub fn reserve_all(&self, budgets: &[(&str, usize, Duration)]) -> bool {
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

    pub fn reserve(&self, key: &str, max_attempts: usize, window: Duration) -> bool {
        self.reserve_all(&[(key, max_attempts, window)])
    }

    /// Gives back one earlier `reserve`, for a key that must not be charged for a successful attempt (a shared IP).
    pub fn release(&self, key: &str) {
        let mut attempts = self.lock();
        if let Some(tracked) = attempts.get_mut(key) {
            tracked.timestamps.pop();
            if tracked.timestamps.is_empty() {
                attempts.remove(key);
            }
        }
    }

    pub fn record_failure(&self, key: &str, max_attempts: usize, window: Duration) {
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

    pub fn clear(&self, key: &str) {
        self.lock().remove(key);
    }

    /// Wipes every login key that counts `username`: the shared one and each organization's own.
    pub fn clear_username(&self, username: &str) {
        let shared = shared_username_key(username);
        self.lock().retain(|key, _| key != &shared && !is_organization_key_of(key, username));
    }

    /// Reports every currently-blocked key against the given limits — a caller with several
    /// orgs' worth of tracked keys would call this once per distinct limit it cares about.
    pub fn blocked_usernames(&self, max_attempts: usize, window: Duration) -> Vec<BlockedUsername> {
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
                blocked.push(BlockedUsername { username: username.clone(), remaining_seconds: remaining.as_secs() });
            }
            true
        });
        blocked.sort_by_key(|b| std::cmp::Reverse(b.remaining_seconds));
        blocked.truncate(MAX_LISTED_BLOCKED);
        blocked
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TrackedKey>> {
        self.attempts.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Makes space for `key` if it's new and the map is full, by dropping the quietest key that isn't blocked (judged against its OWN threshold, so
/// flooding fresh keys can't unblock a victim, B-3). `false` if every tracked key is blocked and there's no room.
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

    #[test]
    fn a_fresh_username_is_not_throttled() {
        let throttle = LoginThrottle::new();
        assert!(!throttle.is_throttled("florian", MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW));
    }

    #[test]
    fn stays_below_the_threshold_until_the_limit_is_reached() {
        let throttle = LoginThrottle::new();
        throttle.record_failure("florian", 3, LOGIN_ATTEMPT_WINDOW);
        throttle.record_failure("florian", 3, LOGIN_ATTEMPT_WINDOW);
        assert!(!throttle.is_throttled("florian", 3, LOGIN_ATTEMPT_WINDOW));
        throttle.record_failure("florian", 3, LOGIN_ATTEMPT_WINDOW);
        assert!(throttle.is_throttled("florian", 3, LOGIN_ATTEMPT_WINDOW));
    }

    #[test]
    fn throttling_is_scoped_to_one_username() {
        let throttle = LoginThrottle::new();
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW);
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW);
        assert!(throttle.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW));
        assert!(!throttle.is_throttled("someone-else", 2, LOGIN_ATTEMPT_WINDOW));
    }

    #[test]
    fn clearing_resets_the_tally() {
        let throttle = LoginThrottle::new();
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW);
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW);
        assert!(throttle.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW));
        throttle.clear("florian");
        assert!(!throttle.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW));
    }

    #[test]
    fn lists_blocked_usernames_but_not_ones_below_the_threshold() {
        let throttle = LoginThrottle::new();
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW);
        throttle.record_failure("florian", 2, LOGIN_ATTEMPT_WINDOW);
        throttle.record_failure("almost-blocked", 2, LOGIN_ATTEMPT_WINDOW);

        let blocked = throttle.blocked_usernames(2, LOGIN_ATTEMPT_WINDOW);

        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].username, "florian");
        assert!(blocked[0].remaining_seconds > 0 && blocked[0].remaining_seconds <= LOGIN_ATTEMPT_WINDOW.as_secs());
    }

    #[test]
    fn blocked_usernames_is_empty_when_nobody_is_throttled() {
        let throttle = LoginThrottle::new();
        throttle.record_failure("florian", MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW);
        assert!(throttle.blocked_usernames(MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW).is_empty());
    }

    #[test]
    fn a_username_drops_off_blocked_usernames_once_its_window_expires() {
        let throttle = LoginThrottle::new();
        throttle.record_failure("florian", 2, Duration::from_millis(30));
        throttle.record_failure("florian", 2, Duration::from_millis(30));
        assert_eq!(throttle.blocked_usernames(2, Duration::from_millis(30)).len(), 1);

        std::thread::sleep(Duration::from_millis(60));

        assert!(throttle.blocked_usernames(2, Duration::from_millis(30)).is_empty());
    }

    #[test]
    fn failures_older_than_the_window_are_pruned() {
        let throttle = LoginThrottle::new();
        throttle.record_failure("florian", 2, Duration::from_millis(30));
        throttle.record_failure("florian", 2, Duration::from_millis(30));
        assert!(throttle.is_throttled("florian", 2, Duration::from_millis(30)));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!throttle.is_throttled("florian", 2, Duration::from_millis(30)));
    }

    #[test]
    fn eviction_never_unblocks_an_actively_blocked_key() {
        let throttle = LoginThrottle::new();
        // Block "victim" first, at a small cap so it's easy to trip.
        throttle.record_failure("victim", 2, LOGIN_ATTEMPT_WINDOW);
        throttle.record_failure("victim", 2, LOGIN_ATTEMPT_WINDOW);
        assert!(throttle.is_throttled("victim", 2, LOGIN_ATTEMPT_WINDOW));

        // Flood past the cap with distinct fresh keys, each under its own limit (not blocked).
        for i in 0..MAX_TRACKED_USERNAMES {
            throttle.record_failure(&format!("attacker-{i}"), MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW);
        }

        assert!(throttle.is_throttled("victim", 2, LOGIN_ATTEMPT_WINDOW), "an actively-blocked key must never be evicted to make room for new attempts");
    }

    #[test]
    fn the_tracked_username_map_never_grows_past_the_cap() {
        let throttle = LoginThrottle::new();
        for i in 0..(MAX_TRACKED_USERNAMES + 50) {
            throttle.record_failure(&format!("attacker-{i}"), MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW);
            assert!(throttle.len() <= MAX_TRACKED_USERNAMES);
        }
        assert_eq!(throttle.len(), MAX_TRACKED_USERNAMES);

        assert!(!throttle.is_throttled("attacker-0", MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW));
    }

    #[test]
    fn reserving_stops_at_the_limit() {
        let throttle = LoginThrottle::new();
        assert!(throttle.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW));
        assert!(throttle.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW));
        assert!(!throttle.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW));
        assert!(throttle.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW));
    }

    #[test]
    fn a_burst_of_concurrent_reservations_never_exceeds_the_limit() {
        let throttle = LoginThrottle::new();
        let handles: Vec<_> = (0..64)
            .map(|_| {
                let throttle = throttle.clone();
                std::thread::spawn(move || throttle.reserve("florian", 10, LOGIN_ATTEMPT_WINDOW))
            })
            .collect();

        let granted = handles.into_iter().map(|h| h.join().unwrap()).filter(|granted| *granted).count();

        assert_eq!(granted, 10);
    }

    #[test]
    fn a_full_budget_among_several_reserves_nothing_from_the_others() {
        let throttle = LoginThrottle::new();
        throttle.reserve("ip", 1, LOGIN_ATTEMPT_WINDOW);

        assert!(!throttle.reserve_all(&[("user", 5, LOGIN_ATTEMPT_WINDOW), ("ip", 1, LOGIN_ATTEMPT_WINDOW)]));

        assert!(!throttle.is_throttled("user", 1, LOGIN_ATTEMPT_WINDOW), "the refused request must not have been charged to the other key");
        assert_eq!(throttle.len(), 1);
    }

    #[test]
    fn releasing_gives_one_reservation_back() {
        let throttle = LoginThrottle::new();
        throttle.reserve("ip", 2, LOGIN_ATTEMPT_WINDOW);
        throttle.reserve("ip", 2, LOGIN_ATTEMPT_WINDOW);
        assert!(throttle.is_throttled("ip", 2, LOGIN_ATTEMPT_WINDOW));

        throttle.release("ip");

        assert!(!throttle.is_throttled("ip", 2, LOGIN_ATTEMPT_WINDOW));
        throttle.release("ip");
        assert_eq!(throttle.len(), 0);
    }

    #[test]
    fn reserving_never_evicts_an_actively_blocked_key() {
        let throttle = LoginThrottle::new();
        throttle.reserve("victim", 1, LOGIN_ATTEMPT_WINDOW);
        for i in 0..MAX_TRACKED_USERNAMES {
            throttle.reserve(&format!("attacker-{i}"), MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW);
        }

        assert!(throttle.is_throttled("victim", 1, LOGIN_ATTEMPT_WINDOW));
        assert!(throttle.len() <= MAX_TRACKED_USERNAMES);
    }

    #[test]
    fn the_blocked_listing_is_capped() {
        let throttle = LoginThrottle::new();
        for i in 0..(MAX_LISTED_BLOCKED + 50) {
            throttle.reserve(&format!("blocked-{i}"), 1, LOGIN_ATTEMPT_WINDOW);
        }

        assert_eq!(throttle.blocked_usernames(1, LOGIN_ATTEMPT_WINDOW).len(), MAX_LISTED_BLOCKED);
    }

    #[test]
    fn a_shorter_window_from_another_caller_does_not_erase_earlier_failures() {
        let throttle = LoginThrottle::new();
        for _ in 0..3 {
            assert!(throttle.reserve("victim", 3, LOGIN_ATTEMPT_WINDOW));
        }
        std::thread::sleep(Duration::from_millis(20));

        assert!(!throttle.reserve("victim", 3, Duration::from_millis(5)), "a caller with a 5 ms window must still see the lock");
        assert!(throttle.is_throttled("victim", 3, Duration::from_millis(5)));
        assert_eq!(throttle.blocked_usernames(3, Duration::from_millis(5)).len(), 1);
    }

    #[test]
    fn a_longer_window_from_a_later_caller_keeps_failures_longer() {
        let throttle = LoginThrottle::new();
        throttle.reserve("key", 2, Duration::from_millis(5));
        throttle.reserve("key", 2, Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(20));

        assert!(throttle.is_throttled("key", 2, Duration::from_millis(5)), "the longer window seen once keeps counting");
    }

    #[test]
    fn a_key_whose_failures_all_expired_takes_the_new_window() {
        let throttle = LoginThrottle::new();
        throttle.reserve("key", 1, Duration::from_millis(50));
        std::thread::sleep(Duration::from_millis(100));
        assert!(throttle.reserve("key", 1, Duration::from_millis(10)));
        std::thread::sleep(Duration::from_millis(30));

        assert!(throttle.reserve("key", 1, Duration::from_millis(10)), "a key that emptied out must not keep an old, longer window");
    }

    #[test]
    fn clearing_a_username_wipes_its_shared_and_every_organization_key_and_nothing_else() {
        let throttle = LoginThrottle::new();
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        for key in [shared_username_key("alice"), organization_username_key(first, "alice"), organization_username_key(second, "alice"), shared_username_key("bob"), organization_username_key(first, "bob"), organization_username_key(first, "x:alice")] {
            throttle.record_failure(&key, 1, LOGIN_ATTEMPT_WINDOW);
        }

        throttle.clear_username("alice");

        assert!(!throttle.is_throttled(&shared_username_key("alice"), 1, LOGIN_ATTEMPT_WINDOW));
        assert!(!throttle.is_throttled(&organization_username_key(first, "alice"), 1, LOGIN_ATTEMPT_WINDOW));
        assert!(!throttle.is_throttled(&organization_username_key(second, "alice"), 1, LOGIN_ATTEMPT_WINDOW));
        assert!(throttle.is_throttled(&shared_username_key("bob"), 1, LOGIN_ATTEMPT_WINDOW));
        assert!(throttle.is_throttled(&organization_username_key(first, "bob"), 1, LOGIN_ATTEMPT_WINDOW));
        assert!(throttle.is_throttled(&organization_username_key(first, "x:alice"), 1, LOGIN_ATTEMPT_WINDOW));
    }

    #[test]
    fn the_shared_and_per_organization_keys_never_collide() {
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());

        assert_eq!(shared_username_key("alice"), "login-user:alice");
        assert_ne!(organization_username_key(first, "alice"), organization_username_key(second, "alice"));
        assert_ne!(organization_username_key(first, "alice"), shared_username_key("alice"));
    }

    #[test]
    fn a_login_identifier_is_cut_to_the_maximum_length() {
        assert_eq!(bounded_identifier(&"a".repeat(10_000)).chars().count(), MAX_LOGIN_IDENTIFIER_LEN);
        assert_eq!(bounded_identifier("florian"), "florian");
        assert_eq!(bounded_identifier(&"é".repeat(300)).chars().count(), MAX_LOGIN_IDENTIFIER_LEN);
    }
}
