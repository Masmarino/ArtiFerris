//! A short-TTL cache of each user's `tokens_valid_after`, so the Docker data-plane auth extractor rejects a token
//! issued before a password change or deactivation without a DB round trip per request, only on a miss or a stale
//! entry.
//!
//! The tradeoff is deliberate: blob and manifest GET/PUT is the highest-traffic path, so the live per-request lookup
//! `artiferris-api` does is too expensive here. The revocation delay is bounded by the TTL (30 s in production) instead
//! of the token's 5-minute lifetime.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use artiferris_domain::error::DomainError;
use artiferris_domain::user::UserRepositoryPort;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Above this many live entries, an insert first drops everything already past its TTL. Without it
/// the map would keep a row per user that ever pulled an image, for the lifetime of the process.
const PRUNE_THRESHOLD: usize = 10_000;

/// A user's `tokens_valid_after` as last read, and when it was read.
type CachedValidAfter = (DateTime<Utc>, Instant);

#[derive(Clone)]
pub struct TokensValidAfterCache {
    ttl: Duration,
    entries: Arc<Mutex<HashMap<Uuid, CachedValidAfter>>>,
    /// API token id to whether it was still active when last read.
    api_tokens: Arc<Mutex<HashMap<Uuid, (bool, Instant)>>>,
}

impl TokensValidAfterCache {
    pub fn new(ttl: Duration) -> Self {
        Self { ttl, entries: Arc::new(Mutex::new(HashMap::new())), api_tokens: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Whether the API token behind an access token is still active. `load` only runs on a miss or a stale entry.
    pub async fn is_api_token_active<E>(&self, api_token_id: Uuid, load: impl Future<Output = Result<bool, E>>) -> Result<bool, E> {
        let cached = self.api_tokens.lock().unwrap_or_else(|p| p.into_inner()).get(&api_token_id).copied();
        if let Some((active, _)) = cached.filter(|(_, cached_at)| cached_at.elapsed() < self.ttl) {
            return Ok(active);
        }
        let active = load.await?;
        let mut api_tokens = self.api_tokens.lock().unwrap_or_else(|p| p.into_inner());
        if api_tokens.len() >= PRUNE_THRESHOLD {
            api_tokens.retain(|_, (_, cached_at)| cached_at.elapsed() < self.ttl);
        }
        api_tokens.insert(api_token_id, (active, Instant::now()));
        Ok(active)
    }

    /// `true` if `issued_at` is at or after the user's current `tokens_valid_after`, refreshing from `users` on a miss
    /// or an entry older than `ttl`. Fails closed: a user that no longer exists is an error, matching
    /// `artiferris-api`'s extractor. Misses are not cached, so a deleted user's unexpired token costs one lookup per
    /// request.
    pub async fn is_valid(
        &self,
        users: &Arc<dyn UserRepositoryPort>,
        user_id: Uuid,
        issued_at: DateTime<Utc>,
    ) -> Result<bool, DomainError> {
        let cached = {
            let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            entries.get(&user_id).copied()
        };

        let tokens_valid_after = match cached {
            Some((valid_after, cached_at)) if cached_at.elapsed() < self.ttl => valid_after,
            _ => {
                let user = users
                    .find_by_id(user_id)
                    .await?
                    .ok_or_else(|| DomainError::Infrastructure(format!("no such user: {user_id}")))?;
                self.store(user_id, user.tokens_valid_after);
                user.tokens_valid_after
            }
        };

        // Second-granularity: the token's `iat` has no sub-second precision, unlike tokens_valid_after.
        Ok(issued_at.timestamp() >= tokens_valid_after.timestamp())
    }

    fn store(&self, user_id: Uuid, tokens_valid_after: DateTime<Utc>) {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if entries.len() >= PRUNE_THRESHOLD {
            entries.retain(|_, (_, cached_at)| cached_at.elapsed() < self.ttl);
        }
        entries.insert(user_id, (tokens_valid_after, Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::user::{User, Username};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn user_with_tokens_valid_after(id: Uuid, valid_after: DateTime<Utc>) -> User {
        User {
            id,
            username: Username::parse("alice").unwrap(),
            password_hash: "irrelevant".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::new_v4(),
            created_at: Utc::now(),
            tokens_valid_after: valid_after,
            email: None,
        }
    }

    /// Counts `find_by_id` calls: the tests assert on it directly rather than inferring caching from timing.
    struct FakeUsers {
        user: Mutex<Option<User>>,
        lookups: AtomicUsize,
    }

    impl FakeUsers {
        fn holding(user: Option<User>) -> Arc<Self> {
            Arc::new(Self { user: Mutex::new(user), lookups: AtomicUsize::new(0) })
        }

        fn set_tokens_valid_after(&self, valid_after: DateTime<Utc>) {
            if let Some(user) = self.user.lock().unwrap().as_mut() {
                user.tokens_valid_after = valid_after;
            }
        }

        fn lookups(&self) -> usize {
            self.lookups.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl UserRepositoryPort for FakeUsers {
        async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, DomainError> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            Ok(self.user.lock().unwrap().clone().filter(|u| u.id == id))
        }
        async fn find_by_username(&self, _username: &Username) -> Result<Option<User>, DomainError> {
            unimplemented!()
        }
        async fn find_by_email(&self, _email: &str) -> Result<Option<User>, DomainError> {
            unimplemented!()
        }
        async fn list_all(&self) -> Result<Vec<User>, DomainError> {
            unimplemented!()
        }
        async fn find_by_ids(&self, _ids: &[Uuid]) -> Result<Vec<User>, DomainError> {
            unimplemented!()
        }
        async fn count_by_organization(&self, _organization_id: Uuid) -> Result<i64, DomainError> {
            unimplemented!()
        }
        async fn search_by_organization(&self, _organization_id: Uuid, _query: &str, _limit: i64) -> Result<Vec<User>, DomainError> {
            unimplemented!()
        }
        async fn search_all_organizations(&self, _query: &str, _limit: i64) -> Result<Vec<User>, DomainError> {
            unimplemented!()
        }
        async fn insert(&self, _user: &User) -> Result<(), DomainError> {
            unimplemented!()
        }
        async fn delete(&self, _id: Uuid) -> Result<(), DomainError> {
            unimplemented!()
        }
        async fn update_password(&self, _id: Uuid, _new_password_hash: String, _audit: Option<&artiferris_domain::audit::AuditRecord>) -> Result<(), DomainError> {
            unimplemented!()
        }
        async fn set_super_admin(&self, _id: Uuid, _is_super_admin: bool) -> Result<(), DomainError> {
            unimplemented!()
        }
        async fn delete_unless_last_super_admin(&self, _id: Uuid, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> {
            unimplemented!()
        }
        async fn set_super_admin_unless_last(&self, _id: Uuid, _is_super_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> {
            unimplemented!()
        }
        async fn set_organization_admin(&self, _id: Uuid, _is_organization_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            unimplemented!()
        }
    }

    fn seeded(valid_after: DateTime<Utc>) -> (Uuid, Arc<FakeUsers>, Arc<dyn UserRepositoryPort>) {
        let user_id = Uuid::new_v4();
        let fake = FakeUsers::holding(Some(user_with_tokens_valid_after(user_id, valid_after)));
        let port: Arc<dyn UserRepositoryPort> = fake.clone();
        (user_id, fake, port)
    }

    #[tokio::test]
    async fn a_token_issued_after_tokens_valid_after_is_valid() {
        let cache = TokensValidAfterCache::new(Duration::from_secs(30));
        let (user_id, _fake, users) = seeded(Utc::now() - chrono::Duration::hours(1));

        assert!(cache.is_valid(&users, user_id, Utc::now()).await.unwrap());
    }

    #[tokio::test]
    async fn a_token_issued_before_tokens_valid_after_is_invalid() {
        let cache = TokensValidAfterCache::new(Duration::from_secs(30));
        let valid_after = Utc::now();
        let (user_id, _fake, users) = seeded(valid_after);

        assert!(!cache.is_valid(&users, user_id, valid_after - chrono::Duration::seconds(1)).await.unwrap());
    }

    #[tokio::test]
    async fn a_repeat_check_within_the_ttl_does_not_hit_the_repository_again() {
        let cache = TokensValidAfterCache::new(Duration::from_secs(30));
        let (user_id, fake, users) = seeded(Utc::now() - chrono::Duration::hours(1));

        for _ in 0..5 {
            assert!(cache.is_valid(&users, user_id, Utc::now()).await.unwrap());
        }

        assert_eq!(fake.lookups(), 1, "only the first check should have reached the repository");
    }

    /// The flip side: a revocation landing while an entry is warm must take effect once it goes stale.
    #[tokio::test]
    async fn a_bump_takes_effect_once_the_cached_entry_goes_stale() {
        let cache = TokensValidAfterCache::new(Duration::from_millis(50));
        let issued_at = Utc::now();
        let (user_id, fake, users) = seeded(issued_at - chrono::Duration::hours(1));

        assert!(cache.is_valid(&users, user_id, issued_at).await.unwrap());

        fake.set_tokens_valid_after(issued_at + chrono::Duration::seconds(1));
        assert!(cache.is_valid(&users, user_id, issued_at).await.unwrap(), "within the TTL the stale value is the accepted tradeoff");
        assert_eq!(fake.lookups(), 1);

        tokio::time::sleep(Duration::from_millis(80)).await;

        assert!(!cache.is_valid(&users, user_id, issued_at).await.unwrap(), "once stale, the refreshed value must reject the token");
        assert_eq!(fake.lookups(), 2, "the stale entry should have triggered exactly one refresh");
    }

    /// Fails closed, like `artiferris-api`'s extractor: a token for a user that no longer exists must not authenticate.
    #[tokio::test]
    async fn a_token_for_an_unknown_user_is_an_error_rather_than_valid() {
        let cache = TokensValidAfterCache::new(Duration::from_secs(30));
        let users: Arc<dyn UserRepositoryPort> = FakeUsers::holding(None);

        assert!(cache.is_valid(&users, Uuid::new_v4(), Utc::now()).await.is_err());
    }

    #[tokio::test]
    async fn an_api_token_check_within_the_ttl_is_served_from_the_cache() {
        let cache = TokensValidAfterCache::new(Duration::from_secs(30));
        let token_id = Uuid::new_v4();
        let loads = std::sync::atomic::AtomicUsize::new(0);
        let load = || async {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok::<_, DomainError>(true)
        };

        for _ in 0..5 {
            assert!(cache.is_api_token_active(token_id, load()).await.unwrap());
        }

        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_revoked_api_token_is_seen_once_the_cached_entry_goes_stale() {
        let cache = TokensValidAfterCache::new(Duration::from_millis(50));
        let token_id = Uuid::new_v4();

        assert!(cache.is_api_token_active(token_id, async { Ok::<_, DomainError>(true) }).await.unwrap());
        assert!(cache.is_api_token_active(token_id, async { Ok::<_, DomainError>(false) }).await.unwrap(), "within the TTL the warm answer stands");

        tokio::time::sleep(Duration::from_millis(80)).await;

        assert!(!cache.is_api_token_active(token_id, async { Ok::<_, DomainError>(false) }).await.unwrap());
    }

    #[tokio::test]
    async fn a_failed_api_token_lookup_is_an_error_and_is_not_cached() {
        let cache = TokensValidAfterCache::new(Duration::from_secs(30));
        let token_id = Uuid::new_v4();

        assert!(cache.is_api_token_active(token_id, async { Err::<bool, _>(DomainError::Infrastructure("down".to_string())) }).await.is_err());

        assert!(cache.is_api_token_active(token_id, async { Ok::<_, DomainError>(true) }).await.unwrap());
    }

    /// `iat` has whole seconds: a token minted in the same second as the bump must not be rejected on sub-second
    /// grounds, the same truncation `artiferris-api` applies.
    #[tokio::test]
    async fn a_token_minted_in_the_same_second_as_the_bump_is_still_valid() {
        let cache = TokensValidAfterCache::new(Duration::from_secs(30));
        let whole_second = DateTime::from_timestamp(Utc::now().timestamp(), 0).unwrap();
        let (user_id, _fake, users) = seeded(whole_second + chrono::Duration::milliseconds(400));

        assert!(cache.is_valid(&users, user_id, whole_second).await.unwrap());
    }
}
