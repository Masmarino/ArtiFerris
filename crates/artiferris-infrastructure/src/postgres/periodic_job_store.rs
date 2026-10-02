use std::time::Duration;

use artiferris_domain::error::DomainError;
use artiferris_domain::periodic_job::PeriodicJobPort;
use async_trait::async_trait;
use sqlx::PgPool;

use crate::error_ext::InfraErr;

/// An instance's timer drifts by a few milliseconds, so a job counts as due a little before its full interval has
/// passed; without the slack the instance that ran it last time could miss by a hair and hand a whole round to another.
const DUE_AFTER_FRACTION: f64 = 0.9;

pub struct PostgresPeriodicJobStore {
    pool: PgPool,
}

impl PostgresPeriodicJobStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PeriodicJobPort for PostgresPeriodicJobStore {
    async fn claim(&self, name: &str, interval: Duration) -> Result<bool, DomainError> {
        let claimed = sqlx::query!(
            "INSERT INTO periodic_job_runs (name, last_started_at) VALUES ($1, clock_timestamp())
             ON CONFLICT (name) DO UPDATE SET last_started_at = clock_timestamp()
             WHERE periodic_job_runs.last_started_at <= clock_timestamp() - make_interval(secs => $2)",
            name,
            interval.as_secs_f64() * DUE_AFTER_FRACTION,
        )
        .execute(&self.pool)
        .await
        .infra_err()?
        .rows_affected();
        Ok(claimed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn of_two_instances_asking_at_once_only_one_runs_the_job(pool: PgPool) {
        let (first, second) = (PostgresPeriodicJobStore::new(pool.clone()), PostgresPeriodicJobStore::new(pool));
        let interval = Duration::from_secs(3600);

        assert!(first.claim("retention", interval).await.unwrap());
        assert!(!second.claim("retention", interval).await.unwrap());
        assert!(!first.claim("retention", interval).await.unwrap(), "not even the one that ran it");
    }

    #[sqlx::test]
    async fn concurrent_claims_yield_exactly_one_winner(pool: PgPool) {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let store = PostgresPeriodicJobStore::new(pool.clone());
                tokio::spawn(async move { store.claim("retention", Duration::from_secs(3600)).await.unwrap() })
            })
            .collect();

        let mut winners = 0;
        for handle in handles {
            winners += usize::from(handle.await.unwrap());
        }
        assert_eq!(winners, 1);
    }

    #[sqlx::test]
    async fn jobs_do_not_take_turns_with_each_other(pool: PgPool) {
        let store = PostgresPeriodicJobStore::new(pool);

        assert!(store.claim("retention", Duration::from_secs(3600)).await.unwrap());
        assert!(store.claim("uploads", Duration::from_secs(3600)).await.unwrap());
    }

    #[sqlx::test]
    async fn the_job_is_due_again_once_its_interval_has_passed_even_a_hair_early(pool: PgPool) {
        let store = PostgresPeriodicJobStore::new(pool.clone());
        assert!(store.claim("retention", Duration::from_secs(3600)).await.unwrap());

        sqlx::query("UPDATE periodic_job_runs SET last_started_at = now() - interval '3500 seconds'").execute(&pool).await.unwrap();
        assert!(store.claim("retention", Duration::from_secs(3600)).await.unwrap(), "3500 s of 3600 s is within the slack");

        sqlx::query("UPDATE periodic_job_runs SET last_started_at = now() - interval '1800 seconds'").execute(&pool).await.unwrap();
        assert!(!store.claim("retention", Duration::from_secs(3600)).await.unwrap(), "half an interval is too early");
    }
}
