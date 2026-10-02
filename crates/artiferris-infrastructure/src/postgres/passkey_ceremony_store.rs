use artiferris_domain::error::DomainError;
use artiferris_domain::webauthn::{CeremonyKind, PasskeyCeremonyStorePort};
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error_ext::InfraErr;

pub struct PostgresPasskeyCeremonyStore {
    pool: PgPool,
}

impl PostgresPasskeyCeremonyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn kind_code(kind: CeremonyKind) -> i16 {
    match kind {
        CeremonyKind::Registration => 0,
        CeremonyKind::Authentication => 1,
    }
}

fn kind_of(code: i16) -> Option<CeremonyKind> {
    match code {
        0 => Some(CeremonyKind::Registration),
        1 => Some(CeremonyKind::Authentication),
        _ => None,
    }
}

#[async_trait]
impl PasskeyCeremonyStorePort for PostgresPasskeyCeremonyStore {
    async fn insert(&self, user_id: Uuid, kind: CeremonyKind, state: Vec<u8>, ttl: chrono::Duration, max_per_user: usize, max_total: usize) -> Result<Uuid, DomainError> {
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("DELETE FROM passkey_ceremonies WHERE expires_at < clock_timestamp()").execute(&mut *tx).await.infra_err()?;
        // Room for the new one: keep the user's newest `max_per_user - 1`, then the newest `max_total - 1` overall.
        sqlx::query!(
            "DELETE FROM passkey_ceremonies WHERE user_id = $1 AND id NOT IN (SELECT id FROM passkey_ceremonies WHERE user_id = $1 ORDER BY created_at DESC, id LIMIT $2)",
            user_id,
            max_per_user.saturating_sub(1) as i64,
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        sqlx::query!(
            "DELETE FROM passkey_ceremonies WHERE id NOT IN (SELECT id FROM passkey_ceremonies ORDER BY created_at DESC, id LIMIT $1)",
            max_total.saturating_sub(1) as i64,
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        sqlx::query!(
            "INSERT INTO passkey_ceremonies (id, user_id, kind, state, expires_at) VALUES ($1, $2, $3, $4, clock_timestamp() + make_interval(secs => $5))",
            id,
            user_id,
            kind_code(kind),
            state,
            ttl.num_milliseconds() as f64 / 1000.0,
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        tx.commit().await.infra_err()?;
        Ok(id)
    }

    async fn take(&self, challenge_id: Uuid, user_id: Uuid) -> Result<Option<(CeremonyKind, Vec<u8>)>, DomainError> {
        // One statement, so of any number of instances answering the same finish only one gets the ceremony.
        let row = sqlx::query!("DELETE FROM passkey_ceremonies WHERE id = $1 AND expires_at >= clock_timestamp() RETURNING user_id, kind, state", challenge_id).fetch_optional(&self.pool).await.infra_err()?;
        Ok(row.filter(|row| row.user_id == user_id).and_then(|row| Some((kind_of(row.kind)?, row.state))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minutes(n: i64) -> chrono::Duration {
        chrono::Duration::minutes(n)
    }

    async fn insert(store: &PostgresPasskeyCeremonyStore, user: Uuid, state: &[u8], max_per_user: usize, max_total: usize) -> Uuid {
        store.insert(user, CeremonyKind::Registration, state.to_vec(), minutes(5), max_per_user, max_total).await.unwrap()
    }

    async fn stored(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM passkey_ceremonies").fetch_one(pool).await.unwrap()
    }

    #[sqlx::test]
    async fn a_ceremony_started_on_one_instance_is_finished_on_another(pool: PgPool) {
        let (first, second) = (PostgresPasskeyCeremonyStore::new(pool.clone()), PostgresPasskeyCeremonyStore::new(pool));
        let user = Uuid::new_v4();
        let id = first.insert(user, CeremonyKind::Authentication, b"state".to_vec(), minutes(5), 5, 100).await.unwrap();

        assert_eq!(second.take(id, user).await.unwrap(), Some((CeremonyKind::Authentication, b"state".to_vec())));
        assert_eq!(first.take(id, user).await.unwrap(), None, "single use");
    }

    #[sqlx::test]
    async fn concurrent_finishes_of_one_ceremony_yield_exactly_one_winner(pool: PgPool) {
        let user = Uuid::new_v4();
        let id = insert(&PostgresPasskeyCeremonyStore::new(pool.clone()), user, b"state", 5, 100).await;
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let store = PostgresPasskeyCeremonyStore::new(pool.clone());
                tokio::spawn(async move { store.take(id, user).await.unwrap().is_some() })
            })
            .collect();

        let mut winners = 0;
        for handle in handles {
            winners += usize::from(handle.await.unwrap());
        }
        assert_eq!(winners, 1);
    }

    #[sqlx::test]
    async fn another_users_finish_gets_nothing_and_burns_the_ceremony(pool: PgPool) {
        let store = PostgresPasskeyCeremonyStore::new(pool);
        let owner = Uuid::new_v4();
        let id = insert(&store, owner, b"state", 5, 100).await;

        assert_eq!(store.take(id, Uuid::new_v4()).await.unwrap(), None);
        assert_eq!(store.take(id, owner).await.unwrap(), None);
    }

    #[sqlx::test]
    async fn the_oldest_ceremony_of_a_user_is_dropped_past_the_per_user_cap_and_others_are_left_alone(pool: PgPool) {
        let store = PostgresPasskeyCeremonyStore::new(pool.clone());
        let (user, victim) = (Uuid::new_v4(), Uuid::new_v4());
        let victims = insert(&store, victim, b"v", 2, 100).await;
        let first = insert(&store, user, b"1", 2, 100).await;
        let second = insert(&store, user, b"2", 2, 100).await;
        let third = insert(&store, user, b"3", 2, 100).await;

        assert_eq!(stored(&pool).await, 3);
        assert_eq!(store.take(first, user).await.unwrap(), None, "the oldest made room");
        assert!(store.take(second, user).await.unwrap().is_some());
        assert!(store.take(third, user).await.unwrap().is_some());
        assert!(store.take(victims, victim).await.unwrap().is_some());
    }

    #[sqlx::test]
    async fn the_store_never_grows_past_the_global_cap(pool: PgPool) {
        let store = PostgresPasskeyCeremonyStore::new(pool.clone());
        let mut ids = Vec::new();
        for n in 0..5u8 {
            let user = Uuid::new_v4();
            ids.push((user, insert(&store, user, &[n], 5, 3).await));
        }

        assert_eq!(stored(&pool).await, 3);
        assert_eq!(store.take(ids[0].1, ids[0].0).await.unwrap(), None);
        assert!(store.take(ids[4].1, ids[4].0).await.unwrap().is_some(), "the newest survive");
    }

    #[sqlx::test]
    async fn an_expired_ceremony_cannot_be_taken_and_is_swept_on_the_next_insert(pool: PgPool) {
        let store = PostgresPasskeyCeremonyStore::new(pool.clone());
        let user = Uuid::new_v4();
        let id = store.insert(user, CeremonyKind::Registration, b"old".to_vec(), chrono::Duration::milliseconds(-1), 5, 100).await.unwrap();
        assert_eq!(store.take(id, user).await.unwrap(), None);

        store.insert(user, CeremonyKind::Registration, b"old".to_vec(), chrono::Duration::milliseconds(-1), 5, 100).await.unwrap();
        insert(&store, user, b"new", 5, 100).await;

        assert_eq!(stored(&pool).await, 1, "only the entry just inserted is left");
    }
}
