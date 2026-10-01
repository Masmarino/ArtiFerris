use std::sync::Arc;

use artiferris_domain::package_repository::PackageRepositoryQueryPort;
use artiferris_domain::permission::PermissionQueryPort;
use artiferris_domain::user::UserRepositoryPort;

use crate::error::ApplicationError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminStats {
    pub total_users: usize,
    pub total_repositories: usize,
    pub total_active_permissions: usize,
}

pub struct GetAdminStatsUseCase {
    users: Arc<dyn UserRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    permissions: Arc<dyn PermissionQueryPort>,
}

impl GetAdminStatsUseCase {
    pub fn new(users: Arc<dyn UserRepositoryPort>, repositories: Arc<dyn PackageRepositoryQueryPort>, permissions: Arc<dyn PermissionQueryPort>) -> Self {
        Self { users, repositories, permissions }
    }

    pub async fn execute(&self) -> Result<AdminStats, ApplicationError> {
        let total_users = self.users.list_all().await?.len();
        let total_repositories = self.repositories.list_all().await?.len();
        let total_active_permissions = self.permissions.count_all().await?;
        Ok(AdminStats { total_users, total_repositories, total_active_permissions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryFormat, RepositoryType};
    use artiferris_domain::permission::Role;
    use artiferris_domain::user::{User, Username};
    use uuid::Uuid;
    use crate::use_cases::admin_test_support::{FakePermissionQuery, FakeRepositoryQuery, FakeUsers};

    #[tokio::test]
    async fn computes_totals_across_users_repositories_and_permissions() {
        let user_id = Uuid::new_v4();
        let other_user_id = Uuid::new_v4();
        let repo_a = Uuid::new_v4();
        let repo_b = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![
            User { id: user_id, username: Username::parse("florian").unwrap(), password_hash: "hash".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: chrono::Utc::now(), tokens_valid_after: chrono::Utc::now(), email: None },
            User { id: other_user_id, username: Username::parse("regular").unwrap(), password_hash: "hash".to_string(), is_super_admin: false, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: chrono::Utc::now(), tokens_valid_after: chrono::Utc::now(), email: None },
        ]));
        let repos = Arc::new(FakeRepositoryQuery {
            repos: vec![
                PackageRepositorySummary { id: repo_a, organization_id: Uuid::new_v4(), name: "repo-a".to_string(), format: RepositoryFormat::Npm, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![] },
                PackageRepositorySummary { id: repo_b, organization_id: Uuid::new_v4(), name: "repo-b".to_string(), format: RepositoryFormat::Npm, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![] },
            ],
        });
        let permissions = Arc::new(FakePermissionQuery {
            entries: vec![(repo_a, user_id, Role::Write), (repo_a, other_user_id, Role::Read), (repo_b, user_id, Role::Admin)],
        });

        let use_case = GetAdminStatsUseCase::new(users, repos, permissions);
        let stats = use_case.execute().await.unwrap();

        assert_eq!(stats.total_users, 2);
        assert_eq!(stats.total_repositories, 2);
        assert_eq!(stats.total_active_permissions, 3);
    }

    #[tokio::test]
    async fn reports_zero_totals_on_a_fresh_instance() {
        let use_case = GetAdminStatsUseCase::new(
            Arc::new(FakeUsers::seeded(vec![])),
            Arc::new(FakeRepositoryQuery { repos: vec![] }),
            Arc::new(FakePermissionQuery { entries: vec![] }),
        );
        let stats = use_case.execute().await.unwrap();
        assert_eq!(stats.total_users, 0);
        assert_eq!(stats.total_repositories, 0);
        assert_eq!(stats.total_active_permissions, 0);
    }
}
