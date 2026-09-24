use std::sync::Arc;

use chrono::{DateTime, Utc};
use artiferris_domain::metrics_snapshot::{MetricsSnapshot, MetricsSnapshotRepositoryPort};
use artiferris_domain::user::UserRepositoryPort;
use uuid::Uuid;

use crate::error::ApplicationError;
use super::usage_metrics::GetUsageMetricsUseCase;

/// Called periodically (see main.rs's timer) so the admin UI can chart totals over time.
pub struct RecordMetricsSnapshotUseCase {
    users: Arc<dyn UserRepositoryPort>,
    usage_metrics: Arc<GetUsageMetricsUseCase>,
    snapshots: Arc<dyn MetricsSnapshotRepositoryPort>,
}

impl RecordMetricsSnapshotUseCase {
    pub fn new(users: Arc<dyn UserRepositoryPort>, usage_metrics: Arc<GetUsageMetricsUseCase>, snapshots: Arc<dyn MetricsSnapshotRepositoryPort>) -> Self {
        Self { users, usage_metrics, snapshots }
    }

    pub async fn execute(&self) -> Result<(), ApplicationError> {
        let total_users = self.users.list_all().await?.len() as i64;
        let usages = self.usage_metrics.execute().await?;
        let total_repositories = usages.len() as i64;
        let total_storage_bytes = usages.iter().map(|u| u.used_bytes as i64).sum();
        let snapshot = MetricsSnapshot {
            id: Uuid::new_v4(),
            recorded_at: Utc::now(),
            total_users,
            total_repositories,
            total_storage_bytes,
        };
        self.snapshots.save(&snapshot).await?;
        Ok(())
    }
}

pub struct GetMetricsHistoryUseCase {
    snapshots: Arc<dyn MetricsSnapshotRepositoryPort>,
}

impl GetMetricsHistoryUseCase {
    pub fn new(snapshots: Arc<dyn MetricsSnapshotRepositoryPort>) -> Self {
        Self { snapshots }
    }

    pub async fn execute(&self, since: DateTime<Utc>) -> Result<Vec<MetricsSnapshot>, ApplicationError> {
        Ok(self.snapshots.list_since(since).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use artiferris_domain::error::DomainError;
    use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryFormat, RepositoryType};
    use artiferris_domain::user::{User, Username};
    use std::sync::Mutex;
    use crate::use_cases::admin_test_support::{FakeDockerBlobs, FakeRepositoryQuery, FakeStorage, FakeUsers};

    struct FakeMetricsSnapshots {
        saved: Mutex<Vec<MetricsSnapshot>>,
    }

    impl FakeMetricsSnapshots {
        fn new() -> Self {
            Self { saved: Mutex::new(Vec::new()) }
        }
    }

    #[async_trait]
    impl MetricsSnapshotRepositoryPort for FakeMetricsSnapshots {
        async fn save(&self, snapshot: &MetricsSnapshot) -> Result<(), DomainError> {
            self.saved.lock().unwrap().push(snapshot.clone());
            Ok(())
        }
        async fn list_since(&self, since: DateTime<Utc>) -> Result<Vec<MetricsSnapshot>, DomainError> {
            Ok(self.saved.lock().unwrap().iter().filter(|s| s.recorded_at >= since).cloned().collect())
        }
    }

    #[tokio::test]
    async fn records_a_snapshot_of_current_totals() {
        let repo_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![
            User { id: Uuid::new_v4(), username: Username::parse("user-one").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: chrono::Utc::now(), tokens_valid_after: chrono::Utc::now(), email: None },
            User { id: Uuid::new_v4(), username: Username::parse("user-two").unwrap(), password_hash: "h".to_string(), is_super_admin: false, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: chrono::Utc::now(), tokens_valid_after: chrono::Utc::now(), email: None },
        ]));
        let repos = Arc::new(FakeRepositoryQuery {
            repos: vec![PackageRepositorySummary {
                id: repo_id,
                organization_id: Uuid::new_v4(),
                name: "my-repo".to_string(),
                format: RepositoryFormat::Docker,
                repo_type: RepositoryType::Hosted,
                remote_url: None,
                remote_username: None,
                remote_password: None,
                quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
            }],
        });
        let docker_blobs = Arc::new(FakeDockerBlobs { used_bytes_by_repository: std::collections::HashMap::from([(repo_id, 500u64)]) });
        let usage_metrics = Arc::new(GetUsageMetricsUseCase::new(repos, Arc::new(FakeStorage { healthy: true }), docker_blobs));
        let snapshots = Arc::new(FakeMetricsSnapshots::new());

        let use_case = RecordMetricsSnapshotUseCase::new(users, usage_metrics, snapshots.clone());
        use_case.execute().await.unwrap();

        let saved = snapshots.saved.lock().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].total_users, 2);
        assert_eq!(saved[0].total_repositories, 1);
        assert_eq!(saved[0].total_storage_bytes, 500);
    }

    #[tokio::test]
    async fn get_metrics_history_returns_snapshots_since_the_given_time() {
        let snapshots = Arc::new(FakeMetricsSnapshots::new());
        let now = chrono::Utc::now();
        snapshots
            .save(&MetricsSnapshot { id: Uuid::new_v4(), recorded_at: now - chrono::Duration::days(40), total_users: 1, total_repositories: 1, total_storage_bytes: 1 })
            .await
            .unwrap();
        snapshots
            .save(&MetricsSnapshot { id: Uuid::new_v4(), recorded_at: now - chrono::Duration::days(1), total_users: 2, total_repositories: 2, total_storage_bytes: 2 })
            .await
            .unwrap();

        let use_case = GetMetricsHistoryUseCase::new(snapshots);
        let history = use_case.execute(now - chrono::Duration::days(30)).await.unwrap();

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].total_users, 2);
    }
}
