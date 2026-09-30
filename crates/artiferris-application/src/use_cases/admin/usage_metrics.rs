use std::sync::Arc;

use artiferris_domain::docker_registry::DockerBlobStorePort;
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, RepositoryFormat};
use artiferris_domain::storage::StorageBackendPort;
use uuid::Uuid;

use crate::error::ApplicationError;

#[derive(Debug, Clone)]
pub struct RepositoryUsage {
    pub repository_id: Uuid,
    pub name: String,
    pub used_bytes: u64,
    /// `None` means unlimited.
    pub quota_bytes: Option<i64>,
}

pub struct GetUsageMetricsUseCase {
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    storage: Arc<dyn StorageBackendPort>,
    docker_blobs: Arc<dyn DockerBlobStorePort>,
}

impl GetUsageMetricsUseCase {
    pub fn new(
        repositories: Arc<dyn PackageRepositoryQueryPort>,
        storage: Arc<dyn StorageBackendPort>,
        docker_blobs: Arc<dyn DockerBlobStorePort>,
    ) -> Self {
        Self { repositories, storage, docker_blobs }
    }

    pub async fn execute(&self) -> Result<Vec<RepositoryUsage>, ApplicationError> {
        let repos = self.repositories.list_all().await?;

        let docker_ids: Vec<Uuid> = repos.iter().filter(|r| r.format == RepositoryFormat::Docker).map(|r| r.id).collect();
        let docker_usage = self.docker_blobs.used_bytes_for_repositories(&docker_ids).await?;

        let npm_repos: Vec<_> = repos.iter().filter(|r| r.format == RepositoryFormat::Npm).collect();
        let npm_bytes = futures::future::try_join_all(npm_repos.iter().map(|r| self.storage.used_bytes(r.id))).await?;
        let npm_usage: std::collections::HashMap<Uuid, u64> = npm_repos.iter().map(|r| r.id).zip(npm_bytes).collect();

        let usages = repos
            .into_iter()
            .map(|repo| {
                let used_bytes = match repo.format {
                    RepositoryFormat::Npm => npm_usage.get(&repo.id).copied().unwrap_or(0),
                    RepositoryFormat::Docker => docker_usage.get(&repo.id).copied().unwrap_or(0),
                };
                RepositoryUsage { repository_id: repo.id, name: repo.name, used_bytes, quota_bytes: repo.quota_bytes }
            })
            .collect();
        Ok(usages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryType};
    use crate::use_cases::admin_test_support::{FakeDockerBlobs, FakeRepositoryQuery, FakeStorage};

    #[tokio::test]
    async fn computes_usage_metrics_for_an_npm_repository_from_the_storage_backend() {
        let repo_id = Uuid::new_v4();
        let repos = Arc::new(FakeRepositoryQuery {
            repos: vec![PackageRepositorySummary {
                id: repo_id,
                organization_id: Uuid::new_v4(),
                name: "my-repo".to_string(),
                format: RepositoryFormat::Npm,
                repo_type: RepositoryType::Hosted,
                remote_url: None,
                remote_username: None,
                remote_password: None,
                quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
            }],
        });
        let docker_blobs = Arc::new(FakeDockerBlobs { used_bytes_by_repository: std::collections::HashMap::new() });
        let use_case = GetUsageMetricsUseCase::new(repos, Arc::new(FakeStorage { healthy: true }), docker_blobs);
        let usages = use_case.execute().await.unwrap();
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].repository_id, repo_id);
        assert_eq!(usages[0].used_bytes, repo_id.as_u128() as u64 % 1000);
    }

    #[tokio::test]
    async fn computes_usage_metrics_for_a_docker_repository_from_the_blob_store_not_the_storage_backend() {
        let repo_id = Uuid::new_v4();
        let repos = Arc::new(FakeRepositoryQuery {
            repos: vec![PackageRepositorySummary {
                id: repo_id,
                organization_id: Uuid::new_v4(),
                name: "my-image".to_string(),
                format: RepositoryFormat::Docker,
                repo_type: RepositoryType::Hosted,
                remote_url: None,
                remote_username: None,
                remote_password: None,
                quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![],
            }],
        });
        let docker_blobs =
            Arc::new(FakeDockerBlobs { used_bytes_by_repository: std::collections::HashMap::from([(repo_id, 4096u64)]) });
        let use_case = GetUsageMetricsUseCase::new(repos, Arc::new(FakeStorage { healthy: true }), docker_blobs);
        let usages = use_case.execute().await.unwrap();
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].used_bytes, 4096);
    }
}
