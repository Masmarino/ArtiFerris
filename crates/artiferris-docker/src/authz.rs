use axum::http::StatusCode;
use artiferris_application::authz_primitives::{self, OrganizationScoped, RepositoryAccessError};
use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryFormat};
use artiferris_domain::permission::{Role, organization_admin_bypass_role};
#[cfg(test)]
use artiferris_domain::package_repository::RepositoryType;
use uuid::Uuid;

use crate::auth::DockerAuthUser;
use crate::state::DockerState;

impl OrganizationScoped for DockerAuthUser {
    fn is_super_admin(&self) -> bool {
        self.is_super_admin
    }
    fn organization_id(&self) -> Uuid {
        self.organization_id
    }
}

fn map_access_error(e: RepositoryAccessError) -> StatusCode {
    match e {
        RepositoryAccessError::NotFound => StatusCode::NOT_FOUND,
        RepositoryAccessError::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
        RepositoryAccessError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn find_repository_by_name(state: &DockerState, organization_id: Uuid, name: &str) -> Result<PackageRepositorySummary, StatusCode> {
    authz_primitives::find_repository_by_name(&state.repositories, organization_id, name).await.map_err(map_access_error)
}

/// Resolves whether `repo`'s organization is personal — the single source of truth every caller
/// uses to decide whether a denied grant should read as 404 instead of 403 (see
/// `require_granted_action_for_route`). Derived from the resolved organization itself, not from
/// which route the request arrived through: a personal org's slug is a syntactically valid
/// subdomain label, so a caller can reach a personal repo via the plain org-based route too.
async fn resolve_is_personal(state: &DockerState, user: &DockerAuthUser, repo: &PackageRepositorySummary) -> Result<bool, StatusCode> {
    // A caller whose own org already matches can never be in a personal org — a real user's
    // organization_id is never a personal org's id — so this is the cheap, common case, no extra query.
    if require_same_organization(user, repo.organization_id).is_ok() {
        return Ok(false);
    }
    let organization = state.organizations.find_by_id(repo.organization_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::NOT_FOUND)?;
    if !organization.is_personal {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(true)
}

/// The write path's gate: `resolve_is_personal` applied to a `find_repository_by_name` result —
/// unlike `require_readable_repository_by_name` below, always requires a real caller, `is_public`
/// or not, since a write is never anonymous.
pub async fn require_repository_by_name(state: &DockerState, user: &DockerAuthUser, organization_id: Uuid, name: &str) -> Result<(PackageRepositorySummary, bool), StatusCode> {
    let repo = find_repository_by_name(state, organization_id, name).await?;
    let is_personal = resolve_is_personal(state, user, &repo).await?;
    Ok((repo, is_personal))
}

/// The read-path counterpart of `require_repository_by_name`: resolves the repository first, and
/// only requires a caller — is_personal resolution included — when it isn't public. `Ok((repo,
/// None))` means public, served with no grant check at all; `Ok((repo, Some((caller,
/// is_personal))))` means private, and the caller must still pass
/// `require_granted_action_for_route`. Handing the caller back alongside `is_personal` (rather than
/// making the route re-derive or unwrap `user`) is the same shape `artiferris-npm`'s counterpart uses.
pub async fn require_readable_repository_by_name<'a>(
    state: &DockerState,
    user: Option<&'a DockerAuthUser>,
    organization_id: Uuid,
    name: &str,
) -> Result<(PackageRepositorySummary, Option<(&'a DockerAuthUser, bool)>), StatusCode> {
    let repo = find_repository_by_name(state, organization_id, name).await?;
    if repo.is_public {
        return Ok((repo, None));
    }
    let user = user.ok_or(StatusCode::NOT_FOUND)?;
    let is_personal = resolve_is_personal(state, user, &repo).await?;
    Ok((repo, Some((user, is_personal))))
}

/// Resolves `/u/{username}/{repo}` via the shared `ResolvePersonalRepositoryUseCase` stored on
/// `DockerState`, translating its `ApplicationError`/`None` into the `StatusCode`s this crate's
/// handlers expect. A missing user, an unreserved personal namespace, and an unknown repo name are
/// all 404 — deliberately indistinguishable, same as the use case's own contract.
pub async fn resolve_personal_repository(state: &DockerState, username: &str, repo_name: &str) -> Result<PackageRepositorySummary, StatusCode> {
    state.resolve_personal_repository.execute(username, repo_name).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::NOT_FOUND)
}

pub fn require_same_organization(user: &DockerAuthUser, organization_id: Uuid) -> Result<(), StatusCode> {
    authz_primitives::require_same_organization(user, organization_id).map_err(map_access_error)
}

/// A non-docker-format repository is unreachable through any docker route.
pub fn require_docker_repository(repo: &PackageRepositorySummary) -> Result<(), StatusCode> {
    authz_primitives::require_format(repo, RepositoryFormat::Docker).map_err(map_access_error)
}

/// Writes only make sense against a hosted repository — proxy/group have no local storage.
pub fn require_hosted(repo: &PackageRepositorySummary) -> Result<(), StatusCode> {
    authz_primitives::require_hosted(repo).map_err(map_access_error)
}

/// Docker's per-member read policy for group traversal (C-1). A private member is checked on its own terms, live, like
/// npm: the caller's organization (or the group's own, for a personal namespace) and a grant of at least `Read` on it.
///
/// `top_level_*` describe the repository the request actually addressed. `top_level_was_authorized`
/// means the caller's token scope was verified against that exact repository — which
/// `IssueDockerAccessTokenUseCase` only grants off a live role on it. That branch is what makes a
/// PERSONAL group's members reachable: a personal org's id never equals a real user's
/// `organization_id`, so the caller-org test can never hold there, and members are same-org-as-the
/// group by construction (`create_repository`/`AddGroupMemberUseCase`). It only ever widens the
/// caller-org test — a public top-level repository is served with no token, so the flag is `false`
/// there and the caller-org rule stands alone.
///
/// Takes the member's fields by value so the returned future outlives the traversal's borrow. A failed lookup reads as not readable.
pub async fn member_is_readable(
    state: &DockerState,
    caller: Option<&DockerAuthUser>,
    top_level_organization_id: Uuid,
    top_level_was_authorized: bool,
    member_id: Uuid,
    member_organization_id: Uuid,
    member_is_public: bool,
) -> bool {
    if member_is_public {
        return true;
    }
    let Some(user) = caller else { return false };
    let in_scope = require_same_organization(user, member_organization_id).is_ok() || (top_level_was_authorized && member_organization_id == top_level_organization_id);
    in_scope && caller_can_read(state, user, member_id, member_organization_id).await
}

/// The caller's grants now, not as of the token: super-admin, admin of the member's organization, or a `Read` grant on it.
async fn caller_can_read(state: &DockerState, user: &DockerAuthUser, repository_id: Uuid, repository_organization_id: Uuid) -> bool {
    let Ok(Some(live)) = state.users.find_by_id(user.user_id).await else { return false };
    if live.is_super_admin || organization_admin_bypass_role(live.is_organization_admin, live.organization_id, repository_organization_id).is_some() {
        return true;
    }
    matches!(state.permissions.find_role(user.user_id, repository_id).await, Ok(Some(role)) if role.satisfies(Role::Read))
}

/// Checks the token's already-granted scope, not the image-name segment. `resolved_repository_id` also has to match, not just the name — names are only unique per-org, so a
/// name-only check would let a token for one org's "backend" repo validate against another org's same-named one.
pub fn require_granted_action(user: &DockerAuthUser, resolved_repository_id: Uuid, repository_name: &str, action: &str) -> Result<(), StatusCode> {
    let scope = user.granted_scope.as_ref().ok_or(StatusCode::FORBIDDEN)?;
    let scope_repository = scope.name.split('/').next().unwrap_or(&scope.name);
    if scope_repository != repository_name || !scope.actions.iter().any(|a| a == action) {
        return Err(StatusCode::FORBIDDEN);
    }
    if scope.granted_repository_id != Some(resolved_repository_id) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

/// `require_granted_action`, but for a personal repository reached via `/u/{username}/{repo}` — a
/// real org's cross-org boundary is fenced by `require_same_organization`, so a denied caller is
/// only ever a fellow org member; a personal org has no such fence, so the same 403 would leak a
/// private personal project's existence to any authenticated registry user. 404, not 403.
pub fn require_personal_granted_action(user: &DockerAuthUser, resolved_repository_id: Uuid, repository_name: &str, action: &str) -> Result<(), StatusCode> {
    require_granted_action(user, resolved_repository_id, repository_name, action).map_err(|status| match status {
        StatusCode::FORBIDDEN => StatusCode::NOT_FOUND,
        other => other,
    })
}

/// Picks between the two above, keyed on `require_repository_by_name`'s own `is_personal` result.
pub fn require_granted_action_for_route(user: &DockerAuthUser, resolved_repository_id: Uuid, repository_name: &str, action: &str, is_personal: bool) -> Result<(), StatusCode> {
    if is_personal {
        require_personal_granted_action(user, resolved_repository_id, repository_name, action)
    } else {
        require_granted_action(user, resolved_repository_id, repository_name, action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::docker_registry::DockerGrantedScope;

    fn user(is_super_admin: bool, organization_id: Uuid) -> DockerAuthUser {
        DockerAuthUser { user_id: Uuid::new_v4(), is_super_admin, organization_id, granted_scope: None }
    }

    fn repo(organization_id: Uuid, format: RepositoryFormat, repo_type: RepositoryType) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id: Uuid::new_v4(),
            organization_id,
            name: "widgets".to_string(),
            format,
            repo_type,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: Vec::new(),
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
        }
    }

    #[test]
    fn a_member_of_the_same_organization_passes() {
        let org = Uuid::new_v4();
        assert!(require_same_organization(&user(false, org), org).is_ok());
    }

    #[test]
    fn a_super_admin_passes_for_a_different_organization() {
        assert!(require_same_organization(&user(true, Uuid::new_v4()), Uuid::new_v4()).is_ok());
    }

    #[test]
    fn a_member_of_a_different_organization_is_rejected_with_not_found() {
        let org = Uuid::new_v4();
        let err = require_same_organization(&user(false, Uuid::new_v4()), org).unwrap_err();
        assert_eq!(err, StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_docker_format_repository_passes_require_docker_repository() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Docker, RepositoryType::Hosted);
        assert!(require_docker_repository(&r).is_ok());
    }

    #[test]
    fn an_npm_format_repository_is_unreachable_through_docker_routes() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Npm, RepositoryType::Hosted);
        let err = require_docker_repository(&r).unwrap_err();
        assert_eq!(err, StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_hosted_repository_passes_require_hosted() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Docker, RepositoryType::Hosted);
        assert!(require_hosted(&r).is_ok());
    }

    #[test]
    fn a_proxy_repository_is_rejected_by_require_hosted() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Docker, RepositoryType::Proxy);
        let err = require_hosted(&r).unwrap_err();
        assert_eq!(err, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn a_group_repository_is_rejected_by_require_hosted() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Docker, RepositoryType::Group);
        let err = require_hosted(&r).unwrap_err();
        assert_eq!(err, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn require_docker_repository_rejects_an_npm_format_repository() {
        let repo = PackageRepositorySummary {
            id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            name: "x".to_string(),
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
        assert_eq!(require_docker_repository(&repo).unwrap_err(), StatusCode::NOT_FOUND);
    }

    // require_repository_by_name needs a real repository lookup, so these run against Postgres.
    mod db {
        use super::*;
        use crate::route_test_support::{seed_repository, test_state};
        use artiferris_domain::organization::{Organization, OrganizationSlug};
        use sqlx::PgPool;

        async fn create_org(state: &DockerState, id: Uuid, slug: &str) {
            state
                .organizations
                .create(&Organization {
                    id,
                    slug: OrganizationSlug::parse(slug).unwrap(),
                    display_name: slug.to_string(),
                    is_public: false,
                    is_personal: false,
                    created_at: chrono::Utc::now(),
                })
                .await
                .unwrap();
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_repository_in_the_callers_own_organization_is_returned(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let org_id = Uuid::new_v4();
            create_org(&state, org_id, "acme").await;
            let repo_id = Uuid::new_v4();
            seed_repository(&pool, org_id, repo_id, "docker", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let caller = user(false, org_id);

            let (found, is_personal) = require_repository_by_name(&state, &caller, org_id, &repo_name).await.unwrap();
            assert_eq!(found.id, repo_id);
            assert!(!is_personal);
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_nonexistent_repository_is_not_found(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let caller = user(false, Uuid::new_v4());
            let err = require_repository_by_name(&state, &caller, Uuid::new_v4(), "does-not-exist").await.unwrap_err();
            assert_eq!(err, StatusCode::NOT_FOUND);
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_caller_from_a_different_organization_is_rejected_even_though_the_repository_exists(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let acme_id = Uuid::new_v4();
            create_org(&state, acme_id, "acme").await;
            let repo_id = Uuid::new_v4();
            seed_repository(&pool, acme_id, repo_id, "docker", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let other_org_caller = user(false, Uuid::new_v4());

            let err = require_repository_by_name(&state, &other_org_caller, acme_id, &repo_name).await.unwrap_err();
            assert_eq!(err, StatusCode::NOT_FOUND);
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_super_admin_can_reach_a_repository_outside_their_own_organization(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let acme_id = Uuid::new_v4();
            create_org(&state, acme_id, "acme").await;
            let repo_id = Uuid::new_v4();
            seed_repository(&pool, acme_id, repo_id, "docker", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let super_admin = user(true, Uuid::new_v4());

            let (found, is_personal) = require_repository_by_name(&state, &super_admin, acme_id, &repo_name).await.unwrap();
            assert_eq!(found.id, repo_id);
            assert!(!is_personal);
        }

        async fn create_personal_org(state: &DockerState, id: Uuid) {
            state
                .organizations
                .create(&Organization {
                    id,
                    slug: OrganizationSlug::parse(&format!("u{}", &id.simple().to_string()[..24])).unwrap(),
                    display_name: "alice".to_string(),
                    is_public: false,
                    is_personal: true,
                    created_at: chrono::Utc::now(),
                })
                .await
                .unwrap();
        }

        /// A personal repo's owner never has the personal org in their own `organization_id` JWT
        /// claim — access there is a direct `Permission` grant, never org membership. This must
        /// return `is_personal = true` regardless of the `organization_id` passed in — it's derived
        /// from the resolved organization, so it's the same whether that id came from `/u/...` or
        /// from a personal org's own (syntactically valid) subdomain.
        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_personal_repositorys_owner_is_not_rejected_by_the_organization_check(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let personal_org_id = Uuid::new_v4();
            create_personal_org(&state, personal_org_id).await;
            let repo_id = Uuid::new_v4();
            seed_repository(&pool, personal_org_id, repo_id, "docker", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let owner = user(false, Uuid::new_v4());

            let (found, is_personal) = require_repository_by_name(&state, &owner, personal_org_id, &repo_name).await.unwrap();
            assert_eq!(found.id, repo_id);
            assert!(is_personal);
        }
    }

    #[test]
    fn a_granted_scope_for_a_different_repository_id_is_rejected_even_with_a_matching_name() {
        let repo_a_id = Uuid::new_v4();
        let repo_b_id = Uuid::new_v4();
        let user = DockerAuthUser {
            user_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            is_super_admin: false,
            granted_scope: Some(DockerGrantedScope {
                resource_type: "repository".to_string(),
                name: "backend/image".to_string(),
                actions: vec!["pull".to_string()],
                granted_repository_id: Some(repo_a_id),
            }),
        };

        let result = require_granted_action(&user, repo_b_id, "backend", "pull");

        assert_eq!(result, Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn a_granted_scope_for_the_matching_repository_id_and_name_is_accepted() {
        let repo_id = Uuid::new_v4();
        let user = DockerAuthUser {
            user_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            is_super_admin: false,
            granted_scope: Some(DockerGrantedScope {
                resource_type: "repository".to_string(),
                name: "backend/image".to_string(),
                actions: vec!["pull".to_string()],
                granted_repository_id: Some(repo_id),
            }),
        };

        let result = require_granted_action(&user, repo_id, "backend", "pull");

        assert!(result.is_ok());
    }
}
