use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use artiferris_domain::organization::{Organization, OrganizationRepositoryPort};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, RepositoryFormat, RepositoryType};
use artiferris_domain::permission::{PermissionQueryPort, Role};
use artiferris_domain::system_settings::{SystemSettings, SystemSettingsPort};
use artiferris_domain::user::UserRepositoryPort;
use uuid::Uuid;

use crate::error::ApplicationError;

#[derive(Debug, Clone)]
pub struct ExportedUser {
    pub id: Uuid,
    pub username: String,
    pub is_super_admin: bool,
    pub created_at: DateTime<Utc>,
    pub email: Option<String>,
}

#[derive(Clone)]
pub struct ExportedRepository {
    pub id: Uuid,
    pub name: String,
    pub format: RepositoryFormat,
    pub repo_type: RepositoryType,
    pub remote_url: Option<String>,
    pub remote_username: Option<String>,
    pub remote_password: Option<String>,
    pub group_members: Vec<Uuid>,
    pub quota_bytes: Option<i64>,
    pub retention_keep_last_n: Option<i32>,
}

impl std::fmt::Debug for ExportedRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportedRepository")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("format", &self.format)
            .field("repo_type", &self.repo_type)
            .field("remote_url", &self.remote_url)
            .field("remote_username", &self.remote_username)
            .field("remote_password", &self.remote_password.as_ref().map(|_| "[redacted]"))
            .field("group_members", &self.group_members)
            .field("quota_bytes", &self.quota_bytes)
            .field("retention_keep_last_n", &self.retention_keep_last_n)
            .finish()
    }
}

/// Export and import flatten everything into one organization: faithful only with a single real organization and no repository in a personal namespace (the export refuses those).
pub(super) fn ensure_single_tenant(organizations: &[Organization]) -> Result<(), ApplicationError> {
    if organizations.iter().any(|o| !o.is_public && !o.is_personal) {
        return Err(ApplicationError::MultiTenantInstance);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct ExportedPermission {
    pub user_id: Uuid,
    pub repository_id: Uuid,
    pub role: Role,
}

/// No packages/images (too large) and no credentials — this file gets downloaded to disk.
#[derive(Debug, Clone)]
pub struct ConfigurationExport {
    pub exported_at: DateTime<Utc>,
    pub users: Vec<ExportedUser>,
    pub repositories: Vec<ExportedRepository>,
    pub permissions: Vec<ExportedPermission>,
    pub system_settings: SystemSettings,
}

pub struct ExportConfigurationUseCase {
    users: Arc<dyn UserRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    permissions: Arc<dyn PermissionQueryPort>,
    system_settings: Arc<dyn SystemSettingsPort>,
    organizations: Arc<dyn OrganizationRepositoryPort>,
}

impl ExportConfigurationUseCase {
    pub fn new(
        users: Arc<dyn UserRepositoryPort>,
        repositories: Arc<dyn PackageRepositoryQueryPort>,
        permissions: Arc<dyn PermissionQueryPort>,
        system_settings: Arc<dyn SystemSettingsPort>,
        organizations: Arc<dyn OrganizationRepositoryPort>,
    ) -> Self {
        Self { users, repositories, permissions, system_settings, organizations }
    }

    pub async fn execute(&self) -> Result<ConfigurationExport, ApplicationError> {
        let organizations = self.organizations.list_all().await?;
        ensure_single_tenant(&organizations)?;
        let users = self.users.list_all().await?;
        let repositories = self.repositories.list_all().await?;
        let personal_organizations: HashSet<Uuid> = organizations.iter().filter(|o| o.is_personal).map(|o| o.id).collect();
        let personal_repositories = repositories.iter().filter(|r| personal_organizations.contains(&r.organization_id)).count();
        if personal_repositories > 0 {
            return Err(ApplicationError::PersonalRepositoriesNotExportable(personal_repositories));
        }

        let permissions: Vec<ExportedPermission> = self
            .permissions
            .list_all()
            .await?
            .into_iter()
            .map(|(user_id, repository_id, role)| ExportedPermission { user_id, repository_id, role })
            .collect();

        // A single-tenant instance has one real organization, the public one, so these are the settings import restores.
        let system_settings = self.system_settings.get(artiferris_domain::organization::PUBLIC_ORGANIZATION_ID).await?;

        Ok(ConfigurationExport {
            exported_at: Utc::now(),
            users: users
                .into_iter()
                .map(|u| ExportedUser { id: u.id, username: u.username.as_str().to_string(), is_super_admin: u.is_super_admin, created_at: u.created_at, email: u.email })
                .collect(),
            repositories: repositories
                .into_iter()
                .map(|r| ExportedRepository {
                    id: r.id,
                    name: r.name,
                    format: r.format,
                    repo_type: r.repo_type,
                    remote_url: r.remote_url,
                    // Never exported — see ImportReport.proxy_credentials_needed on the import side.
                    remote_username: None,
                    remote_password: None,
                    group_members: r.group_members,
                    quota_bytes: r.quota_bytes,
                    retention_keep_last_n: r.retention_keep_last_n,
                })
                .collect(),
            permissions,
            system_settings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::package_repository::PackageRepositorySummary;
    use artiferris_domain::user::{User, Username};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use crate::use_cases::admin_test_support::{FakeOrganizations, FakePermissionQuery, FakeRepositoryQuery, FakeSystemSettings, FakeUsers};

    #[tokio::test]
    async fn exports_users_repositories_permissions_and_settings() {
        let admin_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let repo_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![
            User { id: admin_id, username: Username::parse("admin").unwrap(), password_hash: "secret-hash".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: Some("admin@example.com".to_string()) },
            User { id: member_id, username: Username::parse("member").unwrap(), password_hash: "secret-hash".to_string(), is_super_admin: false, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None },
        ]));
        let repositories = Arc::new(FakeRepositoryQuery {
            repos: vec![PackageRepositorySummary {
                id: repo_id,
                organization_id: Uuid::new_v4(),
                name: "my-repo".to_string(),
                format: RepositoryFormat::Npm,
                repo_type: RepositoryType::Proxy,
                remote_url: Some("https://registry.npmjs.org".to_string()),
                remote_username: Some("svc-account".to_string()),
                remote_password: Some("s3cret-upstream-token".to_string()),
                group_members: vec![],
                quota_bytes: Some(1024),
                retention_keep_last_n: Some(5),
                is_public: false,
            }],
        });
        let permissions = Arc::new(FakePermissionQuery { entries: vec![(repo_id, member_id, Role::Write)] });
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });

        let organizations = Arc::new(FakeOrganizations::with(vec![
            FakeOrganizations::organization("public", true, false),
            FakeOrganizations::organization("alice", false, true),
        ]));

        let use_case = ExportConfigurationUseCase::new(users, repositories, permissions, settings, organizations);
        let export = use_case.execute().await.unwrap();

        assert_eq!(export.users.len(), 2);
        assert!(export.users.iter().any(|u| u.username == "admin" && u.is_super_admin));
        assert_eq!(export.repositories.len(), 1);
        assert_eq!(export.repositories[0].quota_bytes, Some(1024));
        assert_eq!(export.repositories[0].retention_keep_last_n, Some(5));
        assert_eq!(export.repositories[0].remote_username, None, "credentials must never leave via the downloadable export");
        assert_eq!(export.repositories[0].remote_password, None, "credentials must never leave via the downloadable export");
        assert_eq!(export.permissions.len(), 1);
        assert_eq!(export.permissions[0].user_id, member_id);
        assert_eq!(export.permissions[0].role, Role::Write);
        assert_eq!(export.system_settings, SystemSettings::defaults());
        assert_eq!(export.users.iter().find(|u| u.username == "admin").unwrap().email, Some("admin@example.com".to_string()));
    }

    #[tokio::test]
    async fn refuses_to_export_an_instance_with_a_real_organization_besides_the_public_one() {
        let organizations = Arc::new(FakeOrganizations::with(vec![
            FakeOrganizations::organization("public", true, false),
            FakeOrganizations::organization("acme", false, false),
        ]));
        let use_case = ExportConfigurationUseCase::new(
            Arc::new(FakeUsers::seeded(vec![])),
            Arc::new(FakeRepositoryQuery { repos: vec![] }),
            Arc::new(FakePermissionQuery { entries: vec![] }),
            Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }),
            organizations,
        );

        let err = use_case.execute().await.unwrap_err();

        assert!(matches!(err, ApplicationError::MultiTenantInstance), "got: {err:?}");
        assert!(err.to_string().contains("single-tenant"));
    }

    #[tokio::test]
    async fn refuses_to_export_repositories_that_sit_in_a_personal_namespace() {
        let alice = FakeOrganizations::organization("alice", false, true);
        let public = FakeOrganizations::organization("public", true, false);
        let repository = |name: &str, organization_id: Uuid| PackageRepositorySummary {
            id: Uuid::new_v4(),
            organization_id,
            name: name.to_string(),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
        };
        let use_case = ExportConfigurationUseCase::new(
            Arc::new(FakeUsers::seeded(vec![])),
            Arc::new(FakeRepositoryQuery { repos: vec![repository("shared", public.id), repository("alice-project", alice.id)] }),
            Arc::new(FakePermissionQuery { entries: vec![] }),
            Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }),
            Arc::new(FakeOrganizations::with(vec![public, alice])),
        );

        let err = use_case.execute().await.unwrap_err();

        assert!(matches!(err, ApplicationError::PersonalRepositoriesNotExportable(1)), "got: {err:?}");
        assert!(err.to_string().contains("personal"));
    }
}
