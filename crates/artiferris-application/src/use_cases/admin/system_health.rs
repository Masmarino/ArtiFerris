use std::sync::Arc;
use std::time::Instant;

use artiferris_domain::health::{ComponentHealth, DatabaseHealth, HealthCheckPort};
use artiferris_domain::storage::StorageBackendPort;

#[derive(Debug, Clone, PartialEq)]
pub struct StorageHealth {
    pub status: ComponentHealth,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthStatus {
    pub database: DatabaseHealth,
    pub storage: StorageHealth,
    pub uptime_seconds: u64,
}

pub struct GetHealthStatusUseCase {
    database: Arc<dyn HealthCheckPort>,
    storage: Arc<dyn StorageBackendPort>,
    started_at: Instant,
}

impl GetHealthStatusUseCase {
    pub fn new(database: Arc<dyn HealthCheckPort>, storage: Arc<dyn StorageBackendPort>, started_at: Instant) -> Self {
        Self { database, storage, started_at }
    }

    pub async fn execute(&self) -> HealthStatus {
        let database = self.database.check().await;

        let storage_up = self.storage.is_healthy().await;
        let space = self.storage.volume_space().await.unwrap_or(artiferris_domain::storage::VolumeSpace { total_bytes: 0, free_bytes: 0 });
        let storage = StorageHealth {
            status: if storage_up { ComponentHealth::Up } else { ComponentHealth::Down("storage backend unreachable".to_string()) },
            used_bytes: space.total_bytes.saturating_sub(space.free_bytes),
            free_bytes: space.free_bytes,
            total_bytes: space.total_bytes,
        };

        HealthStatus { database, storage, uptime_seconds: self.started_at.elapsed().as_secs() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::use_cases::admin_test_support::FakeStorage;

    struct FakeHealthCheck {
        healthy: bool,
    }

    #[async_trait]
    impl HealthCheckPort for FakeHealthCheck {
        async fn check(&self) -> DatabaseHealth {
            let status = if self.healthy { ComponentHealth::Up } else { ComponentHealth::Down("db down".to_string()) };
            DatabaseHealth {
                status,
                response_time_ms: 1,
                active_connections: 2,
                max_connections: 10,
                server_version: self.healthy.then(|| "16.0".to_string()),
            }
        }
    }

    #[tokio::test]
    async fn reports_health_of_dependencies() {
        let use_case = GetHealthStatusUseCase::new(
            Arc::new(FakeHealthCheck { healthy: true }),
            Arc::new(FakeStorage { healthy: false }),
            Instant::now(),
        );
        let status = use_case.execute().await;
        assert_eq!(status.database.status, ComponentHealth::Up);
        assert_eq!(status.database.server_version, Some("16.0".to_string()));
        assert_eq!(status.storage.status, ComponentHealth::Down("storage backend unreachable".to_string()));
        assert_eq!(status.storage.total_bytes, 1000);
        assert_eq!(status.storage.free_bytes, 400);
        assert_eq!(status.storage.used_bytes, 600);
    }

    #[tokio::test]
    async fn reports_uptime_elapsed_since_the_use_case_was_built() {
        let started_at = Instant::now() - std::time::Duration::from_secs(120);
        let use_case = GetHealthStatusUseCase::new(Arc::new(FakeHealthCheck { healthy: true }), Arc::new(FakeStorage { healthy: true }), started_at);

        let status = use_case.execute().await;

        assert!(status.uptime_seconds >= 120, "got {}", status.uptime_seconds);
    }
}
