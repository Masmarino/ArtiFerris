use async_trait::async_trait;
use artiferris_domain::error::DomainError;
use artiferris_domain::rate_limit::{RateLimitCount, RateLimitKey, RateLimitStorePort};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error_ext::InfraErr;

pub struct PostgresRateLimitStore {
    pool: PgPool,
}

impl PostgresRateLimitStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn key_of(bytes: Vec<u8>) -> Option<RateLimitKey> {
    RateLimitKey::try_from(bytes.as_slice()).ok()
}

#[async_trait]
impl RateLimitStorePort for PostgresRateLimitStore {
    async fn publish(&self, instance: Uuid, counts: &[RateLimitCount]) -> Result<(), DomainError> {
        if counts.is_empty() {
            return Ok(());
        }
        let keys: Vec<Vec<u8>> = counts.iter().map(|(key, _, _)| key.to_vec()).collect();
        let windows: Vec<i64> = counts.iter().map(|(_, window, _)| *window as i64).collect();
        let totals: Vec<i32> = counts.iter().map(|(_, _, count)| i32::try_from(*count).unwrap_or(i32::MAX)).collect();
        sqlx::query!(
            "INSERT INTO rate_limit_counters (key_hash, window_start, instance_id, count) \
             SELECT key_hash, window_start, $4, count FROM UNNEST($1::bytea[], $2::bigint[], $3::int[]) AS t(key_hash, window_start, count) \
             ON CONFLICT (key_hash, window_start, instance_id) DO UPDATE SET count = EXCLUDED.count",
            &keys,
            &windows,
            &totals,
            instance,
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn others(&self, instance: Uuid, keys: &[RateLimitKey], windows: [u64; 2]) -> Result<Vec<RateLimitCount>, DomainError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<Vec<u8>> = keys.iter().map(|key| key.to_vec()).collect();
        let windows: Vec<i64> = windows.iter().map(|window| *window as i64).collect();
        let rows = sqlx::query!(
            r#"SELECT key_hash, window_start, SUM(count)::bigint AS "total!" FROM rate_limit_counters
               WHERE instance_id <> $1 AND window_start = ANY($2) AND key_hash = ANY($3)
               GROUP BY key_hash, window_start"#,
            instance,
            &windows,
            &keys,
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        Ok(rows.into_iter().filter_map(|row| Some((key_of(row.key_hash)?, row.window_start as u64, u32::try_from(row.total).unwrap_or(u32::MAX)))).collect())
    }

    async fn purge_before(&self, window: u64) -> Result<u64, DomainError> {
        let result = sqlx::query!("DELETE FROM rate_limit_counters WHERE window_start < $1", window as i64).execute(&self.pool).await.infra_err()?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: RateLimitKey = [1; 16];
    const KEY_B: RateLimitKey = [2; 16];

    #[sqlx::test]
    async fn instances_read_each_others_totals_and_never_their_own(pool: PgPool) {
        let store = PostgresRateLimitStore::new(pool);
        let (first, second, third) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        store.publish(first, &[(KEY_A, 10, 5), (KEY_B, 10, 1)]).await.unwrap();
        store.publish(second, &[(KEY_A, 10, 7), (KEY_A, 9, 3)]).await.unwrap();

        let seen_by_third = store.others(third, &[KEY_A], [10, 9]).await.unwrap();
        let mut seen_by_third: Vec<_> = seen_by_third.into_iter().collect();
        seen_by_third.sort();
        assert_eq!(seen_by_third, vec![(KEY_A, 9, 3), (KEY_A, 10, 12)], "summed over the other instances, one row per window");

        let seen_by_first = store.others(first, &[KEY_A, KEY_B], [10, 9]).await.unwrap();
        assert!(seen_by_first.iter().all(|(key, _, _)| *key != KEY_B), "the first instance does not hear itself");
        assert!(seen_by_first.contains(&(KEY_A, 10, 7)));
    }

    #[sqlx::test]
    async fn publishing_again_replaces_an_instances_total_instead_of_adding_to_it(pool: PgPool) {
        let store = PostgresRateLimitStore::new(pool);
        let (writer, reader) = (Uuid::new_v4(), Uuid::new_v4());

        store.publish(writer, &[(KEY_A, 10, 5)]).await.unwrap();
        store.publish(writer, &[(KEY_A, 10, 8)]).await.unwrap();

        assert_eq!(store.others(reader, &[KEY_A], [10, 9]).await.unwrap(), vec![(KEY_A, 10, 8)]);
    }

    #[sqlx::test]
    async fn purging_removes_older_windows_only(pool: PgPool) {
        let store = PostgresRateLimitStore::new(pool);
        let (writer, reader) = (Uuid::new_v4(), Uuid::new_v4());
        store.publish(writer, &[(KEY_A, 8, 1), (KEY_A, 9, 1), (KEY_A, 10, 1)]).await.unwrap();

        assert_eq!(store.purge_before(9).await.unwrap(), 1);

        let mut left = store.others(reader, &[KEY_A], [10, 9]).await.unwrap();
        left.sort();
        assert_eq!(left, vec![(KEY_A, 9, 1), (KEY_A, 10, 1)]);
    }

    #[sqlx::test]
    async fn nothing_to_publish_or_read_is_a_no_op(pool: PgPool) {
        let store = PostgresRateLimitStore::new(pool);
        store.publish(Uuid::new_v4(), &[]).await.unwrap();
        assert!(store.others(Uuid::new_v4(), &[], [1, 0]).await.unwrap().is_empty());
    }
}
