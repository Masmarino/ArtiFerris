use std::time::Duration;

use artiferris_domain::error::DomainError;
use artiferris_domain::single_use_token::SingleUseTokenStorePort;
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error_ext::InfraErr;

pub struct PostgresSingleUseTokenStore {
    pool: PgPool,
}

impl PostgresSingleUseTokenStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SingleUseTokenStorePort for PostgresSingleUseTokenStore {
    async fn consume(&self, user_id: Uuid, digest: [u8; 32], lifetime: Duration, max_per_user: usize) -> Result<bool, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        // Expired rows are swept here rather than by a background task: a write is what makes more of them.
        sqlx::query!("DELETE FROM spent_single_use_tokens WHERE expires_at < now()").execute(&mut *tx).await.infra_err()?;
        let inserted = sqlx::query!(
            "INSERT INTO spent_single_use_tokens (digest, user_id, expires_at) VALUES ($1, $2, now() + make_interval(secs => $3)) ON CONFLICT (digest) DO NOTHING",
            digest.as_slice(),
            user_id,
            lifetime.as_secs_f64(),
        )
        .execute(&mut *tx)
        .await
        .infra_err()?
        .rows_affected();
        if inserted == 1 {
            sqlx::query!(
                "DELETE FROM spent_single_use_tokens WHERE user_id = $1 AND digest NOT IN (SELECT digest FROM spent_single_use_tokens WHERE user_id = $1 ORDER BY spent_at DESC, digest LIMIT $2)",
                user_id,
                max_per_user as i64,
            )
            .execute(&mut *tx)
            .await
            .infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(inserted == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[sqlx::test]
    async fn a_token_is_spent_once_even_through_another_store_on_the_same_database(pool: PgPool) {
        let (first, second) = (PostgresSingleUseTokenStore::new(pool.clone()), PostgresSingleUseTokenStore::new(pool));
        let user = Uuid::new_v4();

        assert!(first.consume(user, digest(1), Duration::from_secs(60), 16).await.unwrap());
        assert!(!second.consume(user, digest(1), Duration::from_secs(60), 16).await.unwrap(), "another instance must see it as spent");
        assert!(second.consume(user, digest(2), Duration::from_secs(60), 16).await.unwrap());
    }

    #[sqlx::test]
    async fn concurrent_consumers_of_one_token_yield_exactly_one_winner(pool: PgPool) {
        let user = Uuid::new_v4();
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let store = PostgresSingleUseTokenStore::new(pool.clone());
                tokio::spawn(async move { store.consume(user, digest(7), Duration::from_secs(60), 16).await.unwrap() })
            })
            .collect();

        let mut winners = 0;
        for handle in handles {
            winners += usize::from(handle.await.unwrap());
        }
        assert_eq!(winners, 1);
    }

    #[sqlx::test]
    async fn a_user_only_keeps_their_own_share_and_leaves_other_users_alone(pool: PgPool) {
        let store = PostgresSingleUseTokenStore::new(pool);
        let (victim, flooder) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(store.consume(victim, digest(200), Duration::from_secs(60), 4).await.unwrap());
        for n in 0..20u8 {
            assert!(store.consume(flooder, digest(n), Duration::from_secs(60), 4).await.unwrap());
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        assert!(!store.consume(victim, digest(200), Duration::from_secs(60), 4).await.unwrap(), "the victim's entry is untouched");
        assert!(!store.consume(flooder, digest(19), Duration::from_secs(60), 4).await.unwrap(), "the newest of the flooder stay spent");
        assert!(store.consume(flooder, digest(0), Duration::from_secs(60), 4).await.unwrap(), "beyond their share the flooder's oldest are forgotten");
    }

    #[sqlx::test]
    async fn an_expired_entry_is_swept_and_the_token_could_be_spent_again(pool: PgPool) {
        let store = PostgresSingleUseTokenStore::new(pool.clone());
        let user = Uuid::new_v4();
        assert!(store.consume(user, digest(9), Duration::from_millis(1), 16).await.unwrap());
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(store.consume(user, digest(10), Duration::from_secs(60), 16).await.unwrap());

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM spent_single_use_tokens").fetch_one(&pool).await.unwrap();
        assert_eq!(remaining, 1, "the expired entry is gone");
    }
}
