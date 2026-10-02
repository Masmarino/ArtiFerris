use std::time::Duration;

use artiferris_application::login_throttle::username_in_key;
use artiferris_domain::error::DomainError;
use artiferris_domain::login_attempts::{AttemptBudget, BlockedKey, LoginAttemptStorePort};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};

use crate::error_ext::InfraErr;

/// Advisory locks are numbered by a pair; this is the first of the pair for the login throttle, so its locks cannot
/// meet another feature's.
const LOCK_CLASS: i32 = 0x4c47_5448;
/// A row outlives its window by this much before it is swept, so a caller with a slightly longer window still sees it.
const SWEEP_GRACE_SECONDS: f64 = 600.0;

pub struct PostgresLoginAttemptStore {
    pool: PgPool,
    hash_secret: Vec<u8>,
}

impl PostgresLoginAttemptStore {
    /// `hash_secret` keys the hash of throttle keys. Instances of one deployment must share it.
    pub fn new(pool: PgPool, hash_secret: &[u8]) -> Self {
        Self { pool, hash_secret: hash_secret.to_vec() }
    }

    fn hash(&self, key: &str) -> Vec<u8> {
        Sha256::new().chain_update(&self.hash_secret).chain_update(b"\0").chain_update(key.as_bytes()).finalize()[..16].to_vec()
    }

    fn lock_id(hash: &[u8]) -> i32 {
        i32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]])
    }

    /// Serializes everything that counts under these keys until the transaction ends, so that two instances cannot both
    /// find room for the last attempt. Taken in hash order so that two transactions cannot wait on each other.
    async fn lock(tx: &mut Transaction<'_, Postgres>, hashes: &[Vec<u8>]) -> Result<(), DomainError> {
        let mut ids: Vec<i32> = hashes.iter().map(|hash| Self::lock_id(hash)).collect();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            sqlx::query!("SELECT pg_advisory_xact_lock($1, $2)", LOCK_CLASS, id).execute(&mut **tx).await.infra_err()?;
        }
        Ok(())
    }

    /// Attempts counted under the key: judged against the longer of the window each was counted with and `window`.
    async fn count(tx: &mut Transaction<'_, Postgres>, hash: &[u8], window: Duration) -> Result<i64, DomainError> {
        let count = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM login_attempts WHERE key_hash = $1 AND attempted_at > now() - make_interval(secs => GREATEST(window_seconds::float8, $2))"#,
            hash,
            window.as_secs_f64(),
        )
        .fetch_one(&mut **tx)
        .await
        .infra_err()?;
        Ok(count)
    }

    async fn insert(tx: &mut Transaction<'_, Postgres>, key: &str, hash: &[u8], window: Duration) -> Result<(), DomainError> {
        let username = username_in_key(key);
        let window_seconds = i32::try_from(window.as_secs()).unwrap_or(i32::MAX);
        sqlx::query!(
            "INSERT INTO login_attempts (key_hash, username, display_key, window_seconds, expires_at) VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))",
            hash,
            username,
            username.map(|_| key),
            window_seconds,
            f64::from(window_seconds) + SWEEP_GRACE_SECONDS,
        )
        .execute(&mut **tx)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn sweep(tx: &mut Transaction<'_, Postgres>) -> Result<(), DomainError> {
        sqlx::query!("DELETE FROM login_attempts WHERE expires_at < now()").execute(&mut **tx).await.infra_err()?;
        Ok(())
    }
}

#[async_trait]
impl LoginAttemptStorePort for PostgresLoginAttemptStore {
    async fn reserve_all(&self, budgets: &[AttemptBudget<'_>]) -> Result<bool, DomainError> {
        let hashes: Vec<Vec<u8>> = budgets.iter().map(|(key, ..)| self.hash(key)).collect();
        let mut tx = self.pool.begin().await.infra_err()?;
        Self::sweep(&mut tx).await?;
        Self::lock(&mut tx, &hashes).await?;
        for ((_, max_attempts, window), hash) in budgets.iter().zip(&hashes) {
            if Self::count(&mut tx, hash, *window).await? >= *max_attempts as i64 {
                tx.rollback().await.infra_err()?;
                return Ok(false);
            }
        }
        for ((key, _, window), hash) in budgets.iter().zip(&hashes) {
            Self::insert(&mut tx, key, hash, *window).await?;
        }
        tx.commit().await.infra_err()?;
        Ok(true)
    }

    async fn is_throttled(&self, key: &str, max_attempts: usize, window: Duration) -> Result<bool, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        let throttled = Self::count(&mut tx, &self.hash(key), window).await? >= max_attempts as i64;
        tx.rollback().await.infra_err()?;
        Ok(throttled)
    }

    async fn record_failure(&self, key: &str, max_attempts: usize, window: Duration) -> Result<(), DomainError> {
        let hash = self.hash(key);
        let mut tx = self.pool.begin().await.infra_err()?;
        Self::sweep(&mut tx).await?;
        Self::lock(&mut tx, std::slice::from_ref(&hash)).await?;
        if Self::count(&mut tx, &hash, window).await? <= max_attempts as i64 {
            Self::insert(&mut tx, key, &hash, window).await?;
        }
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn release(&self, key: &str) -> Result<(), DomainError> {
        sqlx::query!(
            "DELETE FROM login_attempts WHERE id = (SELECT id FROM login_attempts WHERE key_hash = $1 ORDER BY attempted_at DESC, id DESC LIMIT 1)",
            self.hash(key),
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn clear(&self, key: &str) -> Result<(), DomainError> {
        sqlx::query!("DELETE FROM login_attempts WHERE key_hash = $1", self.hash(key)).execute(&self.pool).await.infra_err()?;
        Ok(())
    }

    async fn clear_username(&self, username: &str) -> Result<(), DomainError> {
        sqlx::query!("DELETE FROM login_attempts WHERE username = $1", username).execute(&self.pool).await.infra_err()?;
        Ok(())
    }

    async fn blocked(&self, max_attempts: usize, window: Duration, limit: usize) -> Result<Vec<BlockedKey>, DomainError> {
        let rows = sqlx::query!(
            r#"SELECT display_key AS "display_key!",
                      GREATEST(0, EXTRACT(EPOCH FROM (MIN(attempted_at + make_interval(secs => GREATEST(window_seconds::float8, $2))) - now())))::float8 AS "remaining!"
               FROM login_attempts
               WHERE display_key IS NOT NULL AND attempted_at > now() - make_interval(secs => GREATEST(window_seconds::float8, $2))
               GROUP BY key_hash, display_key
               HAVING COUNT(*) >= $1
               ORDER BY 2 DESC
               LIMIT $3"#,
            max_attempts as i64,
            window.as_secs_f64(),
            limit as i64,
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        Ok(rows.into_iter().map(|row| BlockedKey { key: row.display_key, remaining_seconds: row.remaining as u64 }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_application::login_throttle::{organization_username_key, shared_username_key, LoginThrottle, LOGIN_ATTEMPT_WINDOW};
    use std::sync::Arc;

    fn throttles(pool: PgPool) -> (LoginThrottle, LoginThrottle) {
        let store = |pool: PgPool| LoginThrottle::new(Arc::new(PostgresLoginAttemptStore::new(pool, b"secret")));
        (store(pool.clone()), store(pool))
    }

    #[sqlx::test]
    async fn two_instances_share_one_budget(pool: PgPool) {
        let (first, second) = throttles(pool);

        assert!(first.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
        assert!(second.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
        assert!(!first.reserve("florian", 2, LOGIN_ATTEMPT_WINDOW).await, "N instances must not mean N times the limit");
        assert!(second.is_throttled("florian", 2, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[sqlx::test]
    async fn a_burst_of_concurrent_reservations_across_instances_never_exceeds_the_limit(pool: PgPool) {
        let (first, second) = throttles(pool);
        let handles: Vec<_> = (0..40)
            .map(|n| {
                let throttle = if n % 2 == 0 { first.clone() } else { second.clone() };
                tokio::spawn(async move { throttle.reserve("florian", 10, LOGIN_ATTEMPT_WINDOW).await })
            })
            .collect();

        let mut granted = 0;
        for handle in handles {
            granted += usize::from(handle.await.unwrap());
        }

        assert_eq!(granted, 10);
    }

    #[sqlx::test]
    async fn a_full_budget_among_several_reserves_nothing_from_the_others(pool: PgPool) {
        let (throttle, _) = throttles(pool);
        throttle.reserve("ip", 1, LOGIN_ATTEMPT_WINDOW).await;

        assert!(!throttle.reserve_all(&[("user", 5, LOGIN_ATTEMPT_WINDOW), ("ip", 1, LOGIN_ATTEMPT_WINDOW)]).await);

        assert!(!throttle.is_throttled("user", 1, LOGIN_ATTEMPT_WINDOW).await, "the refused request must not have been charged to the other key");
    }

    #[sqlx::test]
    async fn releasing_gives_one_reservation_back_and_clearing_wipes_the_key(pool: PgPool) {
        let (throttle, other) = throttles(pool);
        throttle.reserve("ip", 2, LOGIN_ATTEMPT_WINDOW).await;
        throttle.reserve("ip", 2, LOGIN_ATTEMPT_WINDOW).await;
        assert!(other.is_throttled("ip", 2, LOGIN_ATTEMPT_WINDOW).await);

        throttle.release("ip").await;
        assert!(!other.is_throttled("ip", 2, LOGIN_ATTEMPT_WINDOW).await);

        throttle.reserve("ip", 2, LOGIN_ATTEMPT_WINDOW).await;
        other.clear("ip").await;
        assert!(throttle.reserve("ip", 1, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[sqlx::test]
    async fn recording_failures_stops_just_past_the_threshold(pool: PgPool) {
        let (throttle, _) = throttles(pool);
        for _ in 0..10 {
            throttle.record_failure("mfa", 3, LOGIN_ATTEMPT_WINDOW).await;
        }

        assert!(throttle.is_throttled("mfa", 3, LOGIN_ATTEMPT_WINDOW).await);
        throttle.release("mfa").await;
        assert!(throttle.is_throttled("mfa", 3, LOGIN_ATTEMPT_WINDOW).await, "a key is only ever recorded one past its threshold");
    }

    #[sqlx::test]
    async fn a_shorter_window_from_another_caller_does_not_erase_earlier_failures(pool: PgPool) {
        let (throttle, _) = throttles(pool);
        for _ in 0..3 {
            assert!(throttle.reserve("victim", 3, LOGIN_ATTEMPT_WINDOW).await);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(!throttle.reserve("victim", 3, Duration::from_millis(5)).await, "a caller with a 5 ms window must still see the lock");
        assert_eq!(throttle.blocked_usernames(3, Duration::from_millis(5)).await.len(), 0, "only keys that name a user are listed");
    }

    #[sqlx::test]
    async fn failures_older_than_the_window_stop_counting(pool: PgPool) {
        let (throttle, _) = throttles(pool);
        throttle.record_failure("florian", 2, Duration::from_secs(1)).await;
        throttle.record_failure("florian", 2, Duration::from_secs(1)).await;
        assert!(throttle.is_throttled("florian", 2, Duration::from_secs(1)).await);

        tokio::time::sleep(Duration::from_millis(1_200)).await;

        assert!(!throttle.is_throttled("florian", 2, Duration::from_secs(1)).await);
    }

    #[sqlx::test]
    async fn lists_the_blocked_usernames_and_clearing_a_username_wipes_its_shared_and_organization_keys(pool: PgPool) {
        let (throttle, other) = throttles(pool);
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        for key in [shared_username_key("alice"), organization_username_key(first, "alice"), organization_username_key(second, "alice"), shared_username_key("bob"), organization_username_key(first, "x:alice")] {
            throttle.record_failure(&key, 1, LOGIN_ATTEMPT_WINDOW).await;
            throttle.record_failure(&key, 1, LOGIN_ATTEMPT_WINDOW).await;
        }
        throttle.record_failure("login-ip:1.2.3.4", 1, LOGIN_ATTEMPT_WINDOW).await;
        throttle.record_failure("login-ip:1.2.3.4", 1, LOGIN_ATTEMPT_WINDOW).await;

        let blocked = other.blocked_usernames(2, LOGIN_ATTEMPT_WINDOW).await;
        assert_eq!(blocked.len(), 5, "the address key is not listed: {blocked:?}");
        assert!(blocked.iter().any(|b| b.username == shared_username_key("alice") && b.remaining_seconds > 0 && b.remaining_seconds <= LOGIN_ATTEMPT_WINDOW.as_secs()));

        other.clear_username("alice").await;

        assert!(!throttle.is_throttled(&shared_username_key("alice"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(!throttle.is_throttled(&organization_username_key(first, "alice"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(!throttle.is_throttled(&organization_username_key(second, "alice"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(throttle.is_throttled(&shared_username_key("bob"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(throttle.is_throttled(&organization_username_key(first, "x:alice"), 1, LOGIN_ATTEMPT_WINDOW).await);
        assert!(throttle.is_throttled("login-ip:1.2.3.4", 1, LOGIN_ATTEMPT_WINDOW).await);
    }

    #[sqlx::test]
    async fn the_table_holds_no_client_address_and_expired_rows_are_swept(pool: PgPool) {
        let (throttle, _) = throttles(pool.clone());
        throttle.reserve("login-ip:203.0.113.9", 5, LOGIN_ATTEMPT_WINDOW).await;
        let stored: Vec<Option<String>> = sqlx::query_scalar("SELECT display_key FROM login_attempts").fetch_all(&pool).await.unwrap();
        assert_eq!(stored, vec![None]);
        let key_hash: Vec<u8> = sqlx::query_scalar("SELECT key_hash FROM login_attempts").fetch_one(&pool).await.unwrap();
        assert!(!key_hash.windows(4).any(|window| window == b"203."), "the key is stored hashed");

        sqlx::query("UPDATE login_attempts SET expires_at = now() - interval '1 second'").execute(&pool).await.unwrap();
        throttle.reserve("another", 5, LOGIN_ATTEMPT_WINDOW).await;

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_attempts").fetch_one(&pool).await.unwrap();
        assert_eq!(remaining, 1, "the expired row is gone");
    }
}
