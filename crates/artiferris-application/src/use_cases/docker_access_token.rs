use std::sync::Arc;

use chrono::Utc;
use artiferris_domain::api_token::ApiTokenRepositoryPort;
use artiferris_domain::docker_registry::{DockerGrantedScope, DockerScopeRequest, DockerTokenIssuerPort};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary};
use artiferris_domain::permission::{PermissionQueryPort, Role, organization_admin_bypass_role};
use artiferris_domain::user::{User, UserRepositoryPort};
use uuid::Uuid;

use crate::error::ApplicationError;
use crate::use_cases::api_token::hash_api_token;
use crate::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase;

pub struct IssueDockerAccessTokenUseCase {
    api_tokens: Arc<dyn ApiTokenRepositoryPort>,
    users: Arc<dyn UserRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    permissions: Arc<dyn PermissionQueryPort>,
    token_issuer: Arc<dyn DockerTokenIssuerPort>,
    resolve_personal_repository: Arc<ResolvePersonalRepositoryUseCase>,
}

impl IssueDockerAccessTokenUseCase {
    pub fn new(
        api_tokens: Arc<dyn ApiTokenRepositoryPort>,
        users: Arc<dyn UserRepositoryPort>,
        repositories: Arc<dyn PackageRepositoryQueryPort>,
        permissions: Arc<dyn PermissionQueryPort>,
        token_issuer: Arc<dyn DockerTokenIssuerPort>,
        resolve_personal_repository: Arc<ResolvePersonalRepositoryUseCase>,
    ) -> Self {
        Self { api_tokens, users, repositories, permissions, token_issuer, resolve_personal_repository }
    }

    /// `password` is the caller's ArtiFerris API token — the Basic-auth username is never checked.
    /// `organization_id` is the registry subdomain requested against, not necessarily the user's own.
    pub async fn execute(&self, organization_id: Uuid, password: &str, scope: Option<&str>) -> Result<String, ApplicationError> {
        let hash = hash_api_token(password);
        let token = self.api_tokens.find_by_hash(&hash).await?.ok_or(ApplicationError::InvalidCredentials)?;
        if !token.is_active() {
            return Err(ApplicationError::InactiveApiToken);
        }
        let user = self.users.find_by_id(token.user_id).await?.ok_or(ApplicationError::InvalidCredentials)?;
        if token.created_at < user.tokens_valid_after {
            return Err(ApplicationError::InactiveApiToken);
        }
        let _ = self.api_tokens.touch_last_used_at(token.id, Utc::now()).await;

        let granted_scope = match scope.and_then(DockerScopeRequest::parse) {
            None => None,
            Some(requested) => Some(self.authorize_scope(organization_id, &user, requested).await?),
        };

        Ok(self.token_issuer.issue_for_api_token(token.id, user.id, user.organization_id, user.is_super_admin, granted_scope)?)
    }

    /// Whether the API token an access token was exchanged for is still there: not revoked, not expired.
    pub async fn api_token_is_active(&self, user_id: Uuid, api_token_id: Uuid) -> Result<bool, ApplicationError> {
        Ok(self.api_tokens.list_for_user(user_id).await?.iter().any(|token| token.id == api_token_id && token.is_active()))
    }

    /// A docker client requests the full `u/{username}/{repo}/{image}` scope. It is detected from the scope itself and
    /// authorized against the personal org, not the Host-resolved one.
    async fn authorize_scope(&self, organization_id: Uuid, user: &User, requested: DockerScopeRequest) -> Result<DockerGrantedScope, ApplicationError> {
        let Some((username, repo_name, bare_name)) = personal_scope_parts(&requested.name) else {
            return self.authorize(organization_id, user, &requested).await;
        };
        let repository = self.resolve_personal_repository.execute(&username, &repo_name).await?;
        let stripped = DockerScopeRequest { resource_type: requested.resource_type, name: bare_name, actions: requested.actions };
        self.authorize_against(repository, user, &stripped).await
    }

    /// Never errors for an under-authorized request — mirrors real Docker registries by returning a reduced scope instead, without revealing whether a missing repository exists.
    async fn authorize(&self, organization_id: Uuid, user: &User, requested: &DockerScopeRequest) -> Result<DockerGrantedScope, ApplicationError> {
        let repository = self.repositories.find_by_org_and_name(organization_id, requested.artiferris_repository_name()).await?;
        self.authorize_against(repository, user, requested).await
    }

    async fn authorize_against(&self, repository: Option<PackageRepositorySummary>, user: &User, requested: &DockerScopeRequest) -> Result<DockerGrantedScope, ApplicationError> {
        let (actions, granted_repository_id) = match &repository {
            None => (vec![], None),
            Some(repository) => {
                let role = if user.is_super_admin {
                    Some(Role::Admin)
                } else if let Some(role) = organization_admin_bypass_role(user.is_organization_admin, user.organization_id, repository.organization_id) {
                    Some(role)
                } else {
                    self.permissions.find_role(user.id, repository.id).await?
                };
                let role = match role {
                    Some(r) => Some(r),
                    None if repository.is_public => Some(Role::Read),
                    None => None,
                };
                let actions =
                    requested.actions.iter().filter(|action| role.is_some_and(|role| role.satisfies(required_role_for_action(action)))).cloned().collect();
                (actions, Some(repository.id))
            }
        };

        Ok(DockerGrantedScope { resource_type: requested.resource_type.clone(), name: requested.name.clone(), actions, granted_repository_id })
    }
}

/// Unrecognized actions fail closed to write access rather than silently granting them.
fn required_role_for_action(action: &str) -> Role {
    if action == "pull" { Role::Read } else { Role::Write }
}

/// Splits `u/{username}/{repo}/{image...}` into username, repo name and the bare `{repo}/{image...}` name. `None` for
/// any other shape.
fn personal_scope_parts(name: &str) -> Option<(String, String, String)> {
    let mut parts = name.splitn(4, '/');
    if parts.next()? != "u" {
        return None;
    }
    let username = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next().filter(|s| !s.is_empty())?;
    let rest = parts.next().unwrap_or("");
    let bare_name = if rest.is_empty() { repo.to_string() } else { format!("{repo}/{rest}") };
    Some((username.to_string(), repo.to_string(), bare_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use artiferris_domain::api_token::ApiToken;
    use artiferris_domain::docker_registry::DockerAccessClaims;
    use artiferris_domain::error::{DomainError, EventStoreError};
    use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryFormat, RepositoryType};
    use artiferris_domain::permission::Role;
    use artiferris_domain::user::{User, Username};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    struct FakeApiTokens(Mutex<HashMap<String, ApiToken>>);
    #[async_trait]
    impl ApiTokenRepositoryPort for FakeApiTokens {
        async fn insert(&self, token: &ApiToken) -> Result<(), DomainError> {
            self.0.lock().unwrap().insert(token.token_hash.clone(), token.clone());
            Ok(())
        }
        async fn list_for_user(&self, _user_id: Uuid) -> Result<Vec<ApiToken>, DomainError> { Ok(vec![]) }
        async fn list_with_owners(&self, _organization_id: Option<Uuid>, _limit: i64, _offset: i64) -> Result<Vec<artiferris_domain::api_token::ApiTokenWithOwner>, DomainError> { Ok(vec![]) }
        async fn find_with_owner(&self, _id: Uuid) -> Result<Option<artiferris_domain::api_token::ApiTokenWithOwner>, DomainError> { Ok(None) }
        async fn find_by_hash(&self, token_hash: &str) -> Result<Option<ApiToken>, DomainError> {
            Ok(self.0.lock().unwrap().get(token_hash).cloned())
        }
        async fn touch_last_used_at(&self, _id: Uuid, _used_at: chrono::DateTime<chrono::Utc>) -> Result<(), DomainError> { Ok(()) }
        async fn revoke(&self, _id: Uuid, _user_id: Uuid) -> Result<bool, DomainError> { Ok(true) }
        async fn revoke_any(&self, _id: Uuid, _audit: Option<&artiferris_domain::audit::SecurityAuditRecord>) -> Result<(), DomainError> { Ok(()) }
    }

    struct FakeUsers(Mutex<HashMap<Uuid, User>>);
    #[async_trait]
    impl UserRepositoryPort for FakeUsers {
        async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, DomainError> { Ok(self.0.lock().unwrap().get(&id).cloned()) }
        async fn find_by_username(&self, username: &Username) -> Result<Option<User>, DomainError> {
            Ok(self.0.lock().unwrap().values().find(|u| &u.username == username).cloned())
        }
        async fn find_by_email(&self, _email: &str) -> Result<Option<User>, DomainError> { Ok(None) }
        async fn list_all(&self) -> Result<Vec<User>, DomainError> { Ok(vec![]) }
        async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<User>, DomainError> {
            Ok(self.0.lock().unwrap().values().filter(|u| ids.contains(&u.id)).cloned().collect())
        }
        async fn count_by_organization(&self, _organization_id: Uuid) -> Result<i64, DomainError> { Ok(0) }
        async fn search_by_organization(&self, _organization_id: Uuid, _query: &str, _limit: i64) -> Result<Vec<User>, DomainError> { Ok(vec![]) }
        async fn search_all_organizations(&self, _query: &str, _limit: i64) -> Result<Vec<User>, DomainError> { Ok(vec![]) }
        async fn insert(&self, user: &User) -> Result<(), DomainError> {
            self.0.lock().unwrap().insert(user.id, user.clone());
            Ok(())
        }
        async fn delete(&self, _id: Uuid) -> Result<(), DomainError> { Ok(()) }
        async fn update_password(&self, _id: Uuid, _new_password_hash: String, _audit: Option<&artiferris_domain::audit::AuditRecord>) -> Result<(), DomainError> { Ok(()) }
        async fn set_super_admin(&self, _id: Uuid, _is_super_admin: bool) -> Result<(), DomainError> { Ok(()) }
        async fn set_organization_admin(&self, _id: Uuid, _is_organization_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> { Ok(()) }
        async fn delete_unless_last_super_admin(&self, _id: Uuid, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> { Ok(true) }
        async fn set_super_admin_unless_last(&self, _id: Uuid, _is_super_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> { Ok(true) }
    }

    struct FakeRepositories(Mutex<HashMap<(Uuid, String), PackageRepositorySummary>>);
    #[async_trait]
    impl PackageRepositoryQueryPort for FakeRepositories {
        async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
            Ok(self.0.lock().unwrap().values().find(|r| r.id == id).cloned())
        }
        async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
            Ok(self.0.lock().unwrap().get(&(organization_id, name.to_string())).cloned())
        }
        async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError> { Ok(self.0.lock().unwrap().values().cloned().collect()) }
        async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
            Ok(self.0.lock().unwrap().values().filter(|r| r.organization_id == organization_id).cloned().collect())
        }
    }

    struct FakeOrganizations(Mutex<HashMap<artiferris_domain::organization::OrganizationSlug, artiferris_domain::organization::Organization>>);
    #[async_trait]
    impl artiferris_domain::organization::OrganizationRepositoryPort for FakeOrganizations {
        async fn create(&self, org: &artiferris_domain::organization::Organization) -> Result<(), DomainError> {
            self.0.lock().unwrap().insert(org.slug.clone(), org.clone());
            Ok(())
        }
        async fn find_by_id(&self, id: Uuid) -> Result<Option<artiferris_domain::organization::Organization>, DomainError> {
            Ok(self.0.lock().unwrap().values().find(|o| o.id == id).cloned())
        }
        async fn find_by_slug(&self, slug: &artiferris_domain::organization::OrganizationSlug) -> Result<Option<artiferris_domain::organization::Organization>, DomainError> {
            Ok(self.0.lock().unwrap().get(slug).cloned())
        }
        async fn find_public(&self) -> Result<artiferris_domain::organization::Organization, DomainError> {
            unreachable!("not exercised by this use case's tests")
        }
        async fn list_all(&self) -> Result<Vec<artiferris_domain::organization::Organization>, DomainError> { Ok(vec![]) }
    }

    struct FakePermissions(Mutex<HashMap<(Uuid, Uuid), Role>>);
    #[async_trait]
    impl PermissionQueryPort for FakePermissions {
        async fn find_role(&self, user_id: Uuid, repository_id: Uuid) -> Result<Option<Role>, EventStoreError> {
            Ok(self.0.lock().unwrap().get(&(user_id, repository_id)).copied())
        }
        async fn list_for_repository(&self, _repository_id: Uuid) -> Result<Vec<(Uuid, Role)>, EventStoreError> { Ok(vec![]) }
        async fn list_for_user(&self, _user_id: Uuid) -> Result<Vec<(Uuid, Role)>, EventStoreError> { Ok(vec![]) }
        async fn list_all(&self) -> Result<Vec<(Uuid, Uuid, Role)>, EventStoreError> { unreachable!("not exercised by this use case's tests") }
        async fn count_all(&self) -> Result<usize, EventStoreError> { unreachable!("not exercised by this use case's tests") }
        async fn count_for_repositories(&self, _repository_ids: &[Uuid]) -> Result<usize, EventStoreError> { unreachable!("not exercised by this use case's tests") }
    }

    struct FakeTokenIssuer(Mutex<Option<(Uuid, Option<DockerGrantedScope>)>>);
    #[async_trait]
    impl DockerTokenIssuerPort for FakeTokenIssuer {
        fn issue(&self, user_id: Uuid, _organization_id: Uuid, _is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError> {
            *self.0.lock().unwrap() = Some((user_id, granted_scope));
            Ok("fake-jwt".to_string())
        }
        fn issue_for_api_token(&self, _api_token_id: Uuid, user_id: Uuid, organization_id: Uuid, is_super_admin: bool, granted_scope: Option<DockerGrantedScope>) -> Result<String, DomainError> {
            self.issue(user_id, organization_id, is_super_admin, granted_scope)
        }
        fn verify(&self, _token: &str) -> Result<DockerAccessClaims, DomainError> {
            unreachable!("not exercised by this use case's tests")
        }
    }

    fn active_token(user_id: Uuid) -> ApiToken {
        ApiToken {
            id: Uuid::new_v4(),
            user_id,
            token_hash: hash_api_token("plaintext-token"),
            label: "ci".into(),
            created_at: chrono::Utc::now(),
            last_used_at: None,
            revoked_at: None,
            expires_at: None,
        }
    }

    const ORG_ID: Uuid = Uuid::from_u128(1);

    fn regular_user(id: Uuid) -> User {
        User { id, username: Username::parse("alice").unwrap(), password_hash: "irrelevant".into(), is_super_admin: false, is_organization_admin: false, organization_id: ORG_ID, created_at: chrono::Utc::now(), tokens_valid_after: chrono::Utc::now(), email: None }
    }

    struct Harness {
        use_case: IssueDockerAccessTokenUseCase,
        repositories: Arc<FakeRepositories>,
        organizations: Arc<FakeOrganizations>,
        permissions: Arc<FakePermissions>,
        issuer: Arc<FakeTokenIssuer>,
        user_id: Uuid,
    }

    fn harness(user: User) -> Harness {
        let token = active_token(user.id);
        harness_with_token(user, token)
    }

    /// Like `harness`, but with a chosen `ApiToken`, to exercise expiry and `tokens_valid_after` ordering.
    fn harness_with_token(user: User, token: ApiToken) -> Harness {
        let user_id = user.id;
        let api_tokens = Arc::new(FakeApiTokens(Mutex::new(HashMap::from([(token.token_hash.clone(), token)]))));
        let users = Arc::new(FakeUsers(Mutex::new(HashMap::from([(user_id, user)]))));
        let repositories = Arc::new(FakeRepositories(Mutex::new(HashMap::new())));
        let organizations = Arc::new(FakeOrganizations(Mutex::new(HashMap::new())));
        let permissions = Arc::new(FakePermissions(Mutex::new(HashMap::new())));
        let issuer = Arc::new(FakeTokenIssuer(Mutex::new(None)));
        let resolve_personal_repository = Arc::new(ResolvePersonalRepositoryUseCase::new(users.clone(), organizations.clone(), repositories.clone()));
        let use_case =
            IssueDockerAccessTokenUseCase::new(api_tokens, users, repositories.clone(), permissions.clone(), issuer.clone(), resolve_personal_repository);
        Harness { use_case, repositories, organizations, permissions, issuer, user_id }
    }

    /// Seeds a personal organization and a repository in it, in memory.
    fn seed_personal_repository(h: &Harness, user_id: Uuid, repo_name: &str) -> PackageRepositorySummary {
        let personal_org_id = Uuid::new_v4();
        h.organizations.0.lock().unwrap().insert(
            crate::use_cases::personal_repository::personal_organization_slug(user_id),
            artiferris_domain::organization::Organization {
                id: personal_org_id,
                slug: crate::use_cases::personal_repository::personal_organization_slug(user_id),
                display_name: "alice".to_string(),
                is_public: false,
                is_personal: true,
                created_at: chrono::Utc::now(),
            },
        );
        let repo = PackageRepositorySummary {
            id: Uuid::new_v4(),
            organization_id: personal_org_id,
            name: repo_name.to_string(),
            format: RepositoryFormat::Docker,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
            group_members: vec![],
        };
        h.repositories.0.lock().unwrap().insert((personal_org_id, repo_name.to_string()), repo.clone());
        repo
    }

    #[tokio::test]
    async fn rejects_an_unknown_token() {
        let h = harness(regular_user(Uuid::new_v4()));
        let result = h.use_case.execute(ORG_ID, "wrong-token", None).await;
        assert!(matches!(result, Err(ApplicationError::InvalidCredentials)));
    }

    /// A token minted before a password change must not survive it.
    #[tokio::test]
    async fn a_token_created_before_tokens_valid_after_is_rejected() {
        let user_id = Uuid::new_v4();
        let mut user = regular_user(user_id);
        user.tokens_valid_after = chrono::Utc::now() + chrono::Duration::seconds(60);
        let h = harness(user);

        let result = h.use_case.execute(ORG_ID, "plaintext-token", None).await;

        assert!(matches!(result, Err(ApplicationError::InactiveApiToken)));
    }

    #[tokio::test]
    async fn a_token_created_after_tokens_valid_after_still_works() {
        let user_id = Uuid::new_v4();
        let mut user = regular_user(user_id);
        user.tokens_valid_after = chrono::Utc::now() - chrono::Duration::seconds(60);
        let h = harness(user);

        let result = h.use_case.execute(ORG_ID, "plaintext-token", None).await;

        assert!(result.is_ok());
    }

    /// An expired token is rejected whatever `tokens_valid_after` says.
    #[tokio::test]
    async fn an_expired_token_is_rejected_even_when_tokens_valid_after_is_satisfied() {
        let user_id = Uuid::new_v4();
        let mut user = regular_user(user_id);
        user.tokens_valid_after = chrono::Utc::now() - chrono::Duration::days(365);
        let mut token = active_token(user_id);
        token.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
        let h = harness_with_token(user, token);

        let result = h.use_case.execute(ORG_ID, "plaintext-token", None).await;

        assert!(matches!(result, Err(ApplicationError::InactiveApiToken)));
    }

    #[tokio::test]
    async fn issues_a_token_with_no_granted_scope_when_none_was_requested() {
        let h = harness(regular_user(Uuid::new_v4()));
        h.use_case.execute(ORG_ID, "plaintext-token", None).await.unwrap();
        let (issued_for, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        assert_eq!(issued_for, h.user_id);
        assert!(granted.is_none());
    }

    #[tokio::test]
    async fn a_reader_requesting_pull_and_push_is_only_granted_pull() {
        let h = harness(regular_user(Uuid::new_v4()));
        let repo = PackageRepositorySummary { id: Uuid::new_v4(), organization_id: ORG_ID, name: "myrepo".into(), format: RepositoryFormat::Docker, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![] };
        h.repositories.0.lock().unwrap().insert((ORG_ID, "myrepo".into()), repo.clone());
        h.permissions.0.lock().unwrap().insert((h.user_id, repo.id), Role::Read);

        h.use_case.execute(ORG_ID, "plaintext-token", Some("repository:myrepo/myimage:pull,push")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        assert_eq!(granted.unwrap().actions, vec!["pull".to_string()]);
    }

    /// A public repository gives an anonymous or ungranted caller Read.
    #[tokio::test]
    async fn a_public_repository_grants_an_implicit_read_with_no_explicit_permission() {
        let h = harness(regular_user(Uuid::new_v4()));
        let repo = PackageRepositorySummary { id: Uuid::new_v4(), organization_id: ORG_ID, name: "myrepo".into(), format: RepositoryFormat::Docker, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: true, group_members: vec![] };
        h.repositories.0.lock().unwrap().insert((ORG_ID, "myrepo".into()), repo.clone());

        h.use_case.execute(ORG_ID, "plaintext-token", Some("repository:myrepo/myimage:pull,push")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        let actions = granted.unwrap().actions;
        assert!(actions.contains(&"pull".to_string()), "a public repository must grant at least Read/pull with no explicit permission");
        assert!(!actions.contains(&"push".to_string()), "public must never imply Write/push");
    }

    #[tokio::test]
    async fn a_writer_requesting_pull_and_push_is_granted_both() {
        let h = harness(regular_user(Uuid::new_v4()));
        let repo = PackageRepositorySummary { id: Uuid::new_v4(), organization_id: ORG_ID, name: "myrepo".into(), format: RepositoryFormat::Docker, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![] };
        h.repositories.0.lock().unwrap().insert((ORG_ID, "myrepo".into()), repo.clone());
        h.permissions.0.lock().unwrap().insert((h.user_id, repo.id), Role::Write);

        h.use_case.execute(ORG_ID, "plaintext-token", Some("repository:myrepo/myimage:pull,push")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        assert_eq!(granted.unwrap().actions, vec!["pull".to_string(), "push".to_string()]);
    }

    #[tokio::test]
    async fn a_nonexistent_repository_grants_no_actions_without_erroring() {
        let h = harness(regular_user(Uuid::new_v4()));
        h.use_case.execute(ORG_ID, "plaintext-token", Some("repository:no-such-repo/myimage:pull")).await.unwrap();
        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        let granted = granted.unwrap();
        assert!(granted.actions.is_empty());
        assert!(granted.granted_repository_id.is_none());
    }

    #[tokio::test]
    async fn a_super_admin_is_granted_every_requested_action_without_an_explicit_role() {
        let user_id = Uuid::new_v4();
        let mut admin = regular_user(user_id);
        admin.is_super_admin = true;
        let h = harness(admin);
        let repo = PackageRepositorySummary { id: Uuid::new_v4(), organization_id: ORG_ID, name: "myrepo".into(), format: RepositoryFormat::Docker, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![] };
        h.repositories.0.lock().unwrap().insert((ORG_ID, "myrepo".into()), repo);

        h.use_case.execute(ORG_ID, "plaintext-token", Some("repository:myrepo/myimage:pull,push")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        assert_eq!(granted.unwrap().actions, vec!["pull".to_string(), "push".to_string()]);
    }

    /// An org admin gets implicit Admin on any repository of their own organization.
    #[tokio::test]
    async fn an_organization_admin_is_granted_every_requested_action_on_a_repository_in_their_own_organization_without_an_explicit_role() {
        let user_id = Uuid::new_v4();
        let mut org_admin = regular_user(user_id);
        org_admin.is_organization_admin = true;
        let h = harness(org_admin);
        let repo = PackageRepositorySummary { id: Uuid::new_v4(), organization_id: ORG_ID, name: "myrepo".into(), format: RepositoryFormat::Docker, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![] };
        h.repositories.0.lock().unwrap().insert((ORG_ID, "myrepo".into()), repo);

        h.use_case.execute(ORG_ID, "plaintext-token", Some("repository:myrepo/myimage:pull,push")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        assert_eq!(granted.unwrap().actions, vec!["pull".to_string(), "push".to_string()]);
    }

    #[tokio::test]
    async fn an_organization_admin_of_a_different_organization_gets_no_implicit_role() {
        let user_id = Uuid::new_v4();
        let mut org_admin = regular_user(user_id);
        org_admin.is_organization_admin = true;
        org_admin.organization_id = Uuid::from_u128(999);
        let h = harness(org_admin);
        let repo = PackageRepositorySummary { id: Uuid::new_v4(), organization_id: ORG_ID, name: "myrepo".into(), format: RepositoryFormat::Docker, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, quota_bytes: None, retention_keep_last_n: None, is_public: false, group_members: vec![] };
        h.repositories.0.lock().unwrap().insert((ORG_ID, "myrepo".into()), repo);

        h.use_case.execute(ORG_ID, "plaintext-token", Some("repository:myrepo/myimage:pull")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        assert!(granted.unwrap().actions.is_empty());
    }

    #[tokio::test]
    async fn a_token_scoped_to_one_organizations_repository_carries_that_repositorys_id() {
        let h = harness(regular_user(Uuid::new_v4()));
        let other_org_id = Uuid::from_u128(2);

        let repo_in_org_a = PackageRepositorySummary {
            id: Uuid::new_v4(),
            organization_id: ORG_ID,
            name: "backend".into(),
            format: RepositoryFormat::Docker,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
            group_members: vec![],
        };
        let repo_in_org_b = PackageRepositorySummary {
            id: Uuid::new_v4(),
            organization_id: other_org_id,
            name: "backend".into(),
            format: RepositoryFormat::Docker,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
            group_members: vec![],
        };
        h.repositories.0.lock().unwrap().insert((ORG_ID, "backend".into()), repo_in_org_a.clone());
        h.repositories.0.lock().unwrap().insert((other_org_id, "backend".into()), repo_in_org_b.clone());
        h.permissions.0.lock().unwrap().insert((h.user_id, repo_in_org_a.id), Role::Read);

        h.use_case.execute(ORG_ID, "plaintext-token", Some("repository:backend/image:pull")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        assert_eq!(granted.unwrap().granted_repository_id, Some(repo_in_org_a.id));
    }

    #[test]
    fn personal_scope_parts_splits_out_username_and_repo_and_bares_the_rest() {
        assert_eq!(personal_scope_parts("u/alice/my-lib/myimage"), Some(("alice".to_string(), "my-lib".to_string(), "my-lib/myimage".to_string())));
        assert_eq!(
            personal_scope_parts("u/alice/my-lib/library/myimage"),
            Some(("alice".to_string(), "my-lib".to_string(), "my-lib/library/myimage".to_string()))
        );
    }

    #[test]
    fn personal_scope_parts_rejects_anything_not_shaped_like_u_username_repo() {
        assert_eq!(personal_scope_parts("myrepo/myimage"), None, "an ordinary repository scope must not be mistaken for a personal one");
        assert_eq!(personal_scope_parts("u/alice"), None, "missing the repo segment");
        assert_eq!(personal_scope_parts("u//my-lib/myimage"), None, "empty username segment");
        assert_eq!(personal_scope_parts("u/alice//myimage"), None, "empty repo segment");
    }

    /// The full `u/{username}/{repo}/{image}` path must authorize.
    #[tokio::test]
    async fn a_reader_requesting_pull_and_push_for_a_personal_repositorys_unstripped_scope_is_only_granted_pull() {
        let h = harness(regular_user(Uuid::new_v4()));
        let repo = seed_personal_repository(&h, h.user_id, "my-lib");
        h.permissions.0.lock().unwrap().insert((h.user_id, repo.id), Role::Read);

        h.use_case.execute(Uuid::new_v4(), "plaintext-token", Some("repository:u/alice/my-lib/myimage:pull,push")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        let granted = granted.unwrap();
        assert_eq!(granted.actions, vec!["pull".to_string()]);
        assert_eq!(granted.name, "my-lib/myimage");
        assert_eq!(granted.granted_repository_id, Some(repo.id));
    }

    /// An unresolvable personal repository grants no actions and raises no error.
    #[tokio::test]
    async fn an_unresolvable_personal_repository_scope_grants_no_actions_without_erroring() {
        let h = harness(regular_user(Uuid::new_v4()));

        h.use_case.execute(Uuid::new_v4(), "plaintext-token", Some("repository:u/nobody/does-not-exist/myimage:pull")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        let granted = granted.unwrap();
        assert!(granted.actions.is_empty());
        assert!(granted.granted_repository_id.is_none());
    }

    /// The Host-resolved `organization_id` is ignored for a personal scope.
    #[tokio::test]
    async fn a_personal_scope_ignores_the_requested_organization_id() {
        let h = harness(regular_user(Uuid::new_v4()));
        let repo = seed_personal_repository(&h, h.user_id, "my-lib");
        h.permissions.0.lock().unwrap().insert((h.user_id, repo.id), Role::Write);
        let unrelated_requested_org = Uuid::new_v4();

        h.use_case.execute(unrelated_requested_org, "plaintext-token", Some("repository:u/alice/my-lib/myimage:pull,push")).await.unwrap();

        let (_, granted) = h.issuer.0.lock().unwrap().clone().unwrap();
        let mut actions = granted.unwrap().actions;
        actions.sort();
        assert_eq!(actions, vec!["pull".to_string(), "push".to_string()]);
    }
}
