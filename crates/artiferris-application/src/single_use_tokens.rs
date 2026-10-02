//! Remembers which short-lived tokens have already been spent. The record lives in a `SingleUseTokenStorePort`: the
//! database in a deployment, so that every instance refuses a replay, or `InMemorySingleUseTokenStore` for tests and
//! one-off tools.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use artiferris_domain::error::DomainError;
use artiferris_domain::single_use_token::SingleUseTokenStorePort;
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_REMEMBERED: usize = 10_000;
/// A person completes a login or two in a token's lifetime, so this only ever trims a script.
const MAX_REMEMBERED_PER_USER: usize = 16;

#[derive(Clone)]
pub struct SingleUseTokens {
    lifetime: Duration,
    store: Arc<dyn SingleUseTokenStorePort>,
}

impl SingleUseTokens {
    /// `lifetime` must outlast the tokens being tracked, or a spent one could be replayed once it's forgotten.
    pub fn new(lifetime: Duration, store: Arc<dyn SingleUseTokenStorePort>) -> Self {
        Self { lifetime, store }
    }

    /// Per instance and forgotten on restart: for tests.
    pub fn in_memory(lifetime: Duration) -> Self {
        Self::new(lifetime, Arc::new(InMemorySingleUseTokenStore::default()))
    }

    /// `true` the first time a token is seen, `false` for a replay.
    pub async fn consume(&self, user_id: Uuid, token: &str) -> Result<bool, DomainError> {
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        self.store.consume(user_id, digest, self.lifetime, MAX_REMEMBERED_PER_USER).await
    }
}

pub struct InMemorySingleUseTokenStore {
    max_remembered: usize,
    spent: Mutex<Spent>,
}

impl Default for InMemorySingleUseTokenStore {
    fn default() -> Self {
        Self::with_limit(MAX_REMEMBERED)
    }
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

impl InMemorySingleUseTokenStore {
    fn with_limit(max_remembered: usize) -> Self {
        Self { max_remembered, spent: Mutex::new(Spent::default()) }
    }
}

#[async_trait]
impl SingleUseTokenStorePort for InMemorySingleUseTokenStore {
    /// A full table never refuses a first use: it forgets the user's own oldest beyond their share, then expired
    /// entries, then the oldest of anyone.
    async fn consume(&self, user_id: Uuid, digest: [u8; 32], lifetime: Duration, max_per_user: usize) -> Result<bool, DomainError> {
        let mut spent = self.spent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if spent.digests.contains(&digest) {
            return Ok(false);
        }
        while spent.by_user.get(&user_id).is_some_and(|entries| entries.len() >= max_per_user) {
            spent.forget_oldest_of(user_id);
        }
        if spent.digests.len() >= self.max_remembered {
            spent.forget_expired(lifetime);
        }
        while spent.digests.len() >= self.max_remembered {
            spent.forget_oldest();
        }
        spent.digests.insert(digest);
        spent.by_user.entry(user_id).or_default().push_back((digest, Instant::now()));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIFETIME: Duration = Duration::from_secs(60);

    fn user() -> Uuid {
        Uuid::new_v4()
    }

    fn digest(token: &str) -> [u8; 32] {
        Sha256::digest(token.as_bytes()).into()
    }

    async fn consume(store: &InMemorySingleUseTokenStore, user_id: Uuid, token: &str, lifetime: Duration, max_per_user: usize) -> bool {
        store.consume(user_id, digest(token), lifetime, max_per_user).await.unwrap()
    }

    #[tokio::test]
    async fn a_token_can_be_consumed_once() {
        let tokens = SingleUseTokens::in_memory(LIFETIME);
        let user = user();
        assert!(tokens.consume(user, "abc").await.unwrap());
        assert!(!tokens.consume(user, "abc").await.unwrap());
        assert!(tokens.consume(user, "def").await.unwrap());
    }

    #[tokio::test]
    async fn instances_sharing_a_store_see_each_others_spent_tokens() {
        let store: Arc<dyn SingleUseTokenStorePort> = Arc::new(InMemorySingleUseTokenStore::default());
        let (first, second) = (SingleUseTokens::new(LIFETIME, store.clone()), SingleUseTokens::new(LIFETIME, store));
        let user = user();

        assert!(first.consume(user, "abc").await.unwrap());
        assert!(!second.consume(user, "abc").await.unwrap());
    }

    #[tokio::test]
    async fn concurrent_consumers_of_one_token_yield_exactly_one_winner() {
        let tokens = SingleUseTokens::in_memory(LIFETIME);
        let user = user();
        let handles: Vec<_> = (0..32)
            .map(|_| {
                let tokens = tokens.clone();
                tokio::spawn(async move { tokens.consume(user, "shared").await.unwrap() })
            })
            .collect();

        let mut winners = 0;
        for handle in handles {
            winners += usize::from(handle.await.unwrap());
        }
        assert_eq!(winners, 1);
    }

    #[tokio::test]
    async fn a_full_table_still_lets_a_first_use_through_by_forgetting_the_oldest_entry() {
        let store = InMemorySingleUseTokenStore::with_limit(100);
        let user = user();
        for i in 0..100 {
            assert!(consume(&store, user, &format!("token-{i}"), LIFETIME, 100).await);
        }

        assert!(consume(&store, Uuid::new_v4(), "one-more", LIFETIME, 100).await, "a live entry must not block a legitimate first use");
        assert!(!consume(&store, user, "token-99", LIFETIME, 100).await, "the newest entries stay spent");
        assert!(consume(&store, user, "token-0", LIFETIME, 100).await, "the oldest was the one forgotten");
    }

    #[tokio::test]
    async fn the_table_never_grows_past_its_cap() {
        let store = InMemorySingleUseTokenStore::with_limit(50);
        for i in 0..500 {
            consume(&store, Uuid::new_v4(), &format!("token-{i}"), LIFETIME, 10).await;
        }

        let spent = store.spent.lock().unwrap();
        assert_eq!(spent.digests.len(), 50);
        assert_eq!(spent.by_user.values().map(VecDeque::len).sum::<usize>(), 50);
    }

    #[tokio::test]
    async fn one_user_can_only_fill_their_own_share_and_leaves_everyone_elses_entries_alone() {
        let store = InMemorySingleUseTokenStore::with_limit(1_000);
        let victim = user();
        let flooder = user();
        assert!(consume(&store, victim, "victim-token", LIFETIME, 4).await);
        for i in 0..200 {
            assert!(consume(&store, flooder, &format!("flood-{i}"), LIFETIME, 4).await);
        }

        assert!(!consume(&store, victim, "victim-token", LIFETIME, 4).await, "still remembered as spent");
        assert_eq!(store.spent.lock().unwrap().by_user[&flooder].len(), 4);
        assert!(!consume(&store, flooder, "flood-199", LIFETIME, 4).await);
        assert!(consume(&store, flooder, "flood-0", LIFETIME, 4).await, "beyond their share the flooder's own oldest are forgotten");
    }

    #[tokio::test]
    async fn expired_entries_are_dropped_before_a_live_one_is_evicted() {
        let store = InMemorySingleUseTokenStore::with_limit(3);
        let lifetime = Duration::from_millis(40);
        let old_user = user();
        consume(&store, old_user, "old-1", lifetime, 3).await;
        consume(&store, old_user, "old-2", lifetime, 3).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let live_user = user();
        consume(&store, live_user, "live-1", lifetime, 3).await;

        assert!(consume(&store, live_user, "live-2", lifetime, 3).await);

        assert!(!consume(&store, live_user, "live-1", lifetime, 3).await, "the live entry survived");
        assert!(consume(&store, old_user, "old-1", lifetime, 3).await, "the expired ones made the room");
    }
}
