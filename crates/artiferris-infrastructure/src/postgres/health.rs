use async_trait::async_trait;
use artiferris_domain::health::{ComponentHealth, DatabaseHealth, HealthCheckPort};
use sqlx::PgPool;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct PostgresHealthCheck {
    pool: PgPool,
    max_connections: u32,
}

impl PostgresHealthCheck {
    pub fn new(pool: PgPool, max_connections: u32) -> Self {
        Self { pool, max_connections }
    }
}

#[async_trait]
impl HealthCheckPort for PostgresHealthCheck {
    async fn check(&self) -> DatabaseHealth {
        let start = Instant::now();
        let version: Result<(String,), _> = sqlx::query_as("SELECT current_setting('server_version')").fetch_one(&self.pool).await;
        let response_time_ms = start.elapsed().as_millis() as u64;

        let (status, server_version) = match version {
            Ok((version,)) => (ComponentHealth::Up, Some(version)),
            Err(e) => (ComponentHealth::Down(e.to_string()), None),
        };

        DatabaseHealth {
            status,
            response_time_ms,
            active_connections: self.pool.size(),
            max_connections: self.max_connections,
            server_version,
        }
    }
}

/// What `/readyz` asks. It never queues behind busy connections: under load a probe would time out and pull the only replica out of service.
pub struct PostgresReadiness {
    pool: PgPool,
    probe_timeout: Duration,
    /// How recent the last successful probe must be for a fully busy pool to count as up.
    busy_grace: Duration,
    last_success: Mutex<Option<Instant>>,
}

impl PostgresReadiness {
    pub fn new(pool: PgPool) -> Self {
        Self::with_limits(pool, Duration::from_secs(2), Duration::from_secs(30))
    }

    pub fn with_limits(pool: PgPool, probe_timeout: Duration, busy_grace: Duration) -> Self {
        Self { pool, probe_timeout, busy_grace, last_success: Mutex::new(None) }
    }

    /// Runs a query on an idle connection, or on a new one while the pool has room. A pool at its limit with every connection in use counts as up if a probe answered recently.
    pub async fn is_ready(&self) -> bool {
        if self.pool.is_closed() {
            return false;
        }
        if let Some(mut connection) = self.pool.try_acquire() {
            return self.probe(&mut connection).await;
        }
        if self.pool.size() < self.pool.options().get_max_connections() {
            return match tokio::time::timeout(self.probe_timeout, self.pool.acquire()).await {
                Ok(Ok(mut connection)) => self.probe(&mut connection).await,
                _ => false,
            };
        }
        self.last_success.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some_and(|at| at.elapsed() < self.busy_grace)
    }

    async fn probe(&self, connection: &mut sqlx::PgConnection) -> bool {
        let answered = matches!(tokio::time::timeout(self.probe_timeout, sqlx::query("SELECT 1").execute(connection)).await, Ok(Ok(_)));
        if answered {
            *self.last_success.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
        }
        answered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn reports_up_with_connection_and_version_details_when_postgres_is_reachable(pool: sqlx::PgPool) {
        let health = PostgresHealthCheck::new(pool, 10);
        let status = health.check().await;

        assert_eq!(status.status, ComponentHealth::Up);
        assert_eq!(status.max_connections, 10);
        assert!(status.server_version.is_some());
    }

    async fn single_connection_pool(pool: &PgPool) -> PgPool {
        sqlx::postgres::PgPoolOptions::new().max_connections(1).acquire_timeout(Duration::from_secs(30)).connect_with((*pool.connect_options()).clone()).await.unwrap()
    }

    #[sqlx::test]
    async fn readiness_is_true_with_a_free_connection(pool: sqlx::PgPool) {
        assert!(PostgresReadiness::new(pool).is_ready().await);
    }

    #[sqlx::test]
    async fn a_saturated_pool_is_still_ready_while_a_probe_succeeded_recently(pool: sqlx::PgPool) {
        let small = single_connection_pool(&pool).await;
        let readiness = PostgresReadiness::new(small.clone());
        assert!(readiness.is_ready().await);
        let _busy = small.acquire().await.unwrap();

        let started = Instant::now();
        assert!(readiness.is_ready().await, "every connection is in use, the database is not down");
        assert!(started.elapsed() < Duration::from_secs(1), "the probe must not queue behind the pool");
    }

    #[sqlx::test]
    async fn a_saturated_pool_with_no_recent_success_is_not_ready(pool: sqlx::PgPool) {
        let small = single_connection_pool(&pool).await;
        let never_probed = PostgresReadiness::new(small.clone());
        let stale = PostgresReadiness::with_limits(small.clone(), Duration::from_secs(2), Duration::ZERO);
        assert!(stale.is_ready().await);
        let _busy = small.acquire().await.unwrap();

        assert!(!never_probed.is_ready().await);
        assert!(!stale.is_ready().await, "the last success is older than the grace period");
    }

    #[sqlx::test]
    async fn readiness_recovers_after_the_pool_reaped_every_connection(pool: sqlx::PgPool) {
        let short_lived = sqlx::postgres::PgPoolOptions::new().max_connections(3).max_lifetime(Duration::from_millis(200)).connect_with((*pool.connect_options()).clone()).await.unwrap();
        let readiness = PostgresReadiness::with_limits(short_lived.clone(), Duration::from_secs(2), Duration::ZERO);
        assert!(readiness.is_ready().await);
        let deadline = Instant::now() + Duration::from_secs(10);
        while short_lived.size() > 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(short_lived.size(), 0, "the pool should have reaped its connection");

        assert!(readiness.is_ready().await, "with room in the pool the probe opens a connection itself");
    }

    #[tokio::test]
    async fn readiness_is_false_when_the_database_is_unreachable() {
        let options: sqlx::postgres::PgConnectOptions = "postgres://postgres:postgres@127.0.0.1:1/none".parse().unwrap();
        let unreachable = sqlx::postgres::PgPoolOptions::new().max_connections(2).acquire_timeout(Duration::from_secs(1)).connect_lazy_with(options);

        assert!(!PostgresReadiness::new(unreachable).is_ready().await);
    }

    #[sqlx::test]
    async fn a_closed_pool_is_not_ready(pool: sqlx::PgPool) {
        let readiness = PostgresReadiness::new(pool.clone());
        assert!(readiness.is_ready().await);
        pool.close().await;

        assert!(!readiness.is_ready().await);
    }
}
