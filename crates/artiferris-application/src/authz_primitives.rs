use std::sync::Arc;
use uuid::Uuid;

use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary, RepositoryFormat, RepositoryType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryAccessError {
    NotFound,
    MethodNotAllowed,
    Internal,
}

/// The minimal shape every crate's `AuthUser` already has, so these primitives stay generic (npm and docker cannot
/// depend on api).
pub trait OrganizationScoped {
    fn is_super_admin(&self) -> bool;
    fn organization_id(&self) -> Uuid;
}

pub fn require_same_organization<U: OrganizationScoped>(user: &U, organization_id: Uuid) -> Result<(), RepositoryAccessError> {
    if user.is_super_admin() || user.organization_id() == organization_id {
        Ok(())
    } else {
        Err(RepositoryAccessError::NotFound)
    }
}

pub async fn find_repository_by_name(
    repositories: &Arc<dyn PackageRepositoryQueryPort>,
    organization_id: Uuid,
    name: &str,
) -> Result<PackageRepositorySummary, RepositoryAccessError> {
    // Postgres refuses NUL in text, which would turn a bad URL into a 500.
    if name.chars().any(char::is_control) {
        return Err(RepositoryAccessError::NotFound);
    }
    repositories
        .find_by_org_and_name(organization_id, name)
        .await
        .map_err(|_| RepositoryAccessError::Internal)?
        .ok_or(RepositoryAccessError::NotFound)
}

pub fn require_format(repo: &PackageRepositorySummary, format: RepositoryFormat) -> Result<(), RepositoryAccessError> {
    if repo.format != format {
        return Err(RepositoryAccessError::NotFound);
    }
    Ok(())
}

pub fn require_hosted(repo: &PackageRepositorySummary) -> Result<(), RepositoryAccessError> {
    if repo.repo_type != RepositoryType::Hosted {
        return Err(RepositoryAccessError::MethodNotAllowed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::npm_test_support::FakeRepositories;

    struct TestUser {
        is_super_admin: bool,
        organization_id: Uuid,
    }

    impl OrganizationScoped for TestUser {
        fn is_super_admin(&self) -> bool {
            self.is_super_admin
        }
        fn organization_id(&self) -> Uuid {
            self.organization_id
        }
    }

    fn repo(org: Uuid, format: RepositoryFormat, repo_type: RepositoryType) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id: Uuid::new_v4(),
            organization_id: org,
            name: "some-repo".to_string(),
            format,
            repo_type,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
        }
    }

    #[test]
    fn a_super_admin_passes_regardless_of_organization() {
        let user = TestUser { is_super_admin: true, organization_id: Uuid::new_v4() };
        assert!(require_same_organization(&user, Uuid::new_v4()).is_ok());
    }

    #[test]
    fn a_member_of_the_same_organization_passes() {
        let org = Uuid::new_v4();
        let user = TestUser { is_super_admin: false, organization_id: org };
        assert!(require_same_organization(&user, org).is_ok());
    }

    #[test]
    fn a_member_of_a_different_organization_gets_not_found() {
        let user = TestUser { is_super_admin: false, organization_id: Uuid::new_v4() };
        let err = require_same_organization(&user, Uuid::new_v4()).unwrap_err();
        assert_eq!(err, RepositoryAccessError::NotFound);
    }

    #[test]
    fn require_format_rejects_a_mismatched_format() {
        let repo = repo(Uuid::new_v4(), RepositoryFormat::Docker, RepositoryType::Hosted);
        let err = require_format(&repo, RepositoryFormat::Npm).unwrap_err();
        assert_eq!(err, RepositoryAccessError::NotFound);
    }

    #[test]
    fn require_format_accepts_a_matching_format() {
        let repo = repo(Uuid::new_v4(), RepositoryFormat::Npm, RepositoryType::Hosted);
        assert!(require_format(&repo, RepositoryFormat::Npm).is_ok());
    }

    #[test]
    fn require_hosted_rejects_a_proxy_repository() {
        let repo = repo(Uuid::new_v4(), RepositoryFormat::Npm, RepositoryType::Proxy);
        let err = require_hosted(&repo).unwrap_err();
        assert_eq!(err, RepositoryAccessError::MethodNotAllowed);
    }

    #[tokio::test]
    async fn find_repository_by_name_finds_an_existing_repository() {
        let org = Uuid::new_v4();
        let store = FakeRepositories::new();
        store.insert(repo(org, RepositoryFormat::Npm, RepositoryType::Hosted));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let found = find_repository_by_name(&repositories, org, "some-repo").await.unwrap();
        assert_eq!(found.organization_id, org);
    }

    /// Fails the test if the lookup is reached: a control character never gets as far as the database.
    struct UnreachableRepositories;

    #[async_trait::async_trait]
    impl PackageRepositoryQueryPort for UnreachableRepositories {
        async fn find_by_id(&self, _id: Uuid) -> Result<Option<PackageRepositorySummary>, artiferris_domain::error::EventStoreError> {
            unreachable!()
        }
        async fn find_by_org_and_name(&self, _organization_id: Uuid, _name: &str) -> Result<Option<PackageRepositorySummary>, artiferris_domain::error::EventStoreError> {
            Err(artiferris_domain::error::EventStoreError::Storage("invalid byte sequence".to_string()))
        }
        async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, artiferris_domain::error::EventStoreError> {
            unreachable!()
        }
        async fn list_by_organization(&self, _organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, artiferris_domain::error::EventStoreError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn a_repository_name_with_a_control_character_is_not_found_instead_of_an_internal_error() {
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(UnreachableRepositories);
        for name in ["a\0b", "a\nb", "\u{7f}"] {
            let err = find_repository_by_name(&repositories, Uuid::new_v4(), name).await.unwrap_err();
            assert_eq!(err, RepositoryAccessError::NotFound, "{name:?}");
        }
    }

    #[tokio::test]
    async fn find_repository_by_name_returns_not_found_for_an_unknown_name() {
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(FakeRepositories::new());
        let err = find_repository_by_name(&repositories, Uuid::new_v4(), "ghost").await.unwrap_err();
        assert_eq!(err, RepositoryAccessError::NotFound);
    }
}
