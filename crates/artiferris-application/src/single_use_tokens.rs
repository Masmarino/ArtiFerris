//! Remembers which short-lived tokens have already been spent. In memory and per instance, like `LoginThrottle`: a restart forgets, which only matters for the few minutes a token lives.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_REMEMBERED: usize = 10_000;
/// A person completes a login or two in a token's lifetime, so this only ever trims a script.
const MAX_REMEMBERED_PER_USER: usize = 16;

#[derive(Clone)]
pub struct SingleUseTokens {
    lifetime: Duration,
    max_remembered: usize,
    max_per_user: usize,
    spent: Arc<Mutex<Spent>>,
}

#[derive(Default)]
struct Spent {
    digests: HashSet<[u8; 32]>,
    /// Each user's spent tokens, oldest first.
    by_user: HashMap<Uuid, VecDeque<([u8; 32], Instant)>>,
}

impl Spent {
    fn forget_oldest_of(&mut self, user_id: Uuid) {
        let Some(entries) = self.by_user.get_mut(&user_id) else { return };
        if let Some((digest, _)) = entries.pop_front() {
            self.digests.remove(&digest);
        }
        if entries.is_empty() {
            self.by_user.remove(&user_id);
        }
    }

    fn forget_expired(&mut self, lifetime: Duration) {
        let digests = &mut self.digests;
        self.by_user.retain(|_, entries| {
            while entries.front().is_some_and(|(_, at)| at.elapsed() >= lifetime) {
                if let Some((digest, _)) = entries.pop_front() {
                    digests.remove(&digest);
                }
            }
            !entries.is_empty()
        });
    }

    fn forget_oldest(&mut self) {
        let oldest_owner = self.by_user.iter().filter_map(|(user_id, entries)| entries.front().map(|(_, at)| (*user_id, *at))).min_by_key(|(_, at)| *at).map(|(user_id, _)| user_id);
        if let Some(user_id) = oldest_owner {
            self.forget_oldest_of(user_id);
        }
    }
}

impl SingleUseTokens {
    /// `lifetime` must outlast the tokens being tracked, or a spent one could be replayed once it's forgotten.
    pub fn new(lifetime: Duration) -> Self {
        Self::with_limits(lifetime, MAX_REMEMBERED, MAX_REMEMBERED_PER_USER)
    }

    fn with_limits(lifetime: Duration, max_remembered: usize, max_per_user: usize) -> Self {
        Self { lifetime, max_remembered, max_per_user, spent: Arc::new(Mutex::new(Spent::default())) }
    }

    /// `true` the first time a token is seen, `false` for a replay. A full table never refuses a first use: it forgets the user's own oldest beyond
    /// their share, then expired entries, then the oldest of anyone.
    pub fn consume(&self, user_id: Uuid, token: &str) -> bool {
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut spent = self.spent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if spent.digests.contains(&digest) {
            return false;
        }
        while spent.by_user.get(&user_id).is_some_and(|entries| entries.len() >= self.max_per_user) {
            spent.forget_oldest_of(user_id);
        }
        if spent.digests.len() >= self.max_remembered {
            spent.forget_expired(self.lifetime);
        }
        while spent.digests.len() >= self.max_remembered {
            spent.forget_oldest();
        }
        spent.digests.insert(digest);
        spent.by_user.entry(user_id).or_default().push_back((digest, Instant::now()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn a_token_can_be_consumed_once() {
        let tokens = SingleUseTokens::new(Duration::from_secs(60));
        let user = user();
        assert!(tokens.consume(user, "abc"));
        assert!(!tokens.consume(user, "abc"));
        assert!(tokens.consume(user, "def"));
    }

    #[test]
    fn concurrent_consumers_of_one_token_yield_exactly_one_winner() {
        let tokens = SingleUseTokens::new(Duration::from_secs(60));
        let user = user();
        let handles: Vec<_> = (0..32)
            .map(|_| {
                let tokens = tokens.clone();
                std::thread::spawn(move || tokens.consume(user, "shared"))
            })
            .collect();

        assert_eq!(handles.into_iter().map(|h| h.join().unwrap()).filter(|won| *won).count(), 1);
    }

    #[test]
    fn a_full_table_still_lets_a_first_use_through_by_forgetting_the_oldest_entry() {
        let tokens = SingleUseTokens::with_limits(Duration::from_secs(60), 100, 100);
        let user = user();
        for i in 0..100 {
            assert!(tokens.consume(user, &format!("token-{i}")));
        }

        assert!(tokens.consume(Uuid::new_v4(), "one-more"), "a live entry must not block a legitimate first use");
        assert!(!tokens.consume(user, "token-99"), "the newest entries stay spent");
        assert!(tokens.consume(user, "token-0"), "the oldest was the one forgotten");
    }

    #[test]
    fn the_table_never_grows_past_its_cap() {
        let tokens = SingleUseTokens::with_limits(Duration::from_secs(60), 50, 10);
        for i in 0..500 {
            tokens.consume(Uuid::new_v4(), &format!("token-{i}"));
        }

        let spent = tokens.spent.lock().unwrap();
        assert_eq!(spent.digests.len(), 50);
        assert_eq!(spent.by_user.values().map(VecDeque::len).sum::<usize>(), 50);
    }

    #[test]
    fn one_user_can_only_fill_their_own_share_and_leaves_everyone_elses_entries_alone() {
        let tokens = SingleUseTokens::with_limits(Duration::from_secs(60), 1_000, 4);
        let victim = user();
        let flooder = user();
        assert!(tokens.consume(victim, "victim-token"));
        for i in 0..200 {
            assert!(tokens.consume(flooder, &format!("flood-{i}")));
        }

        assert!(!tokens.consume(victim, "victim-token"), "still remembered as spent");
        assert_eq!(tokens.spent.lock().unwrap().by_user[&flooder].len(), 4);
        assert!(!tokens.consume(flooder, "flood-199"));
        assert!(tokens.consume(flooder, "flood-0"), "beyond their share the flooder's own oldest are forgotten");
    }

    #[test]
    fn expired_entries_are_dropped_before_a_live_one_is_evicted() {
        let tokens = SingleUseTokens::with_limits(Duration::from_millis(40), 3, 3);
        let old_user = user();
        tokens.consume(old_user, "old-1");
        tokens.consume(old_user, "old-2");
        std::thread::sleep(Duration::from_millis(60));
        let live_user = user();
        tokens.consume(live_user, "live-1");

        assert!(tokens.consume(live_user, "live-2"));

        assert!(!tokens.consume(live_user, "live-1"), "the live entry survived");
        assert!(tokens.consume(old_user, "old-1"), "the expired ones made the room");
    }
}
