use std::future::Future;

use axum::http::StatusCode;
use artiferris_application::authz_primitives::{self, OrganizationScoped, RepositoryAccessError};
use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryFormat};
#[cfg(test)]
use artiferris_domain::package_repository::RepositoryType;
use artiferris_domain::permission::{Role, organization_admin_bypass_role};
use uuid::Uuid;

use crate::auth::NpmAuthUser;
use crate::state::NpmState;

impl OrganizationScoped for NpmAuthUser {
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

async fn find_repository_by_name(state: &NpmState, organization_id: Uuid, name: &str) -> Result<PackageRepositorySummary, StatusCode> {
    authz_primitives::find_repository_by_name(&state.repositories, organization_id, name).await.map_err(map_access_error)
}

pub async fn require_repository_by_name(state: &NpmState, user: &NpmAuthUser, organization_id: Uuid, name: &str) -> Result<PackageRepositorySummary, StatusCode> {
    let repo = find_repository_by_name(state, organization_id, name).await?;
    // The repository was looked up within organization_id, but the caller's own organization
    // must also match — otherwise a stale cross-organization permission grant stays usable.
    // 404, not 403, so the caller can't tell "wrong org" from "doesn't exist".
    require_same_organization(user, repo.organization_id)?;
    Ok(repo)
}

/// Shared branching shape behind the org and personal read-path helpers below: resolve the
/// repository, then run `verify` (and require a caller) only when it's private. `resolve`/`verify`
/// are injected because the two paths resolve and authorize differently — only this branch is
/// shared. The returned `Option` is `None` only for a public repo, where `verify` never ran.
async fn require_readable_repository<'a, R, RFut, V, VFut>(user: Option<&'a NpmAuthUser>, resolve: R, verify: V) -> Result<(PackageRepositorySummary, Option<&'a NpmAuthUser>), StatusCode>
where
    R: FnOnce() -> RFut,
    RFut: Future<Output = Result<PackageRepositorySummary, StatusCode>>,
    // By value, not `&PackageRepositorySummary` — a plain (non-higher-ranked) `VFut` can't borrow a
    // value that only lives inside this function's own stack frame.
    V: FnOnce(&'a NpmAuthUser, Uuid, Uuid) -> VFut,
    VFut: Future<Output = Result<(), StatusCode>>,
{
    let repo = resolve().await?;
    if repo.is_public {
        return Ok((repo, None));
    }
    // No Authorization header at all on a private repository is indistinguishable from a
    // nonexistent one — same 404-not-401 discipline as everywhere else in this file.
    let user = user.ok_or(StatusCode::NOT_FOUND)?;
    verify(user, repo.id, repo.organization_id).await?;
    Ok((repo, Some(user)))
}

/// The read-path counterpart of `require_repository_by_name`, resolving by org+name. The role
/// check itself stays deferred to the caller, same as before this was extracted.
pub async fn require_readable_repository_by_name<'a>(
    state: &NpmState,
    user: Option<&'a NpmAuthUser>,
    organization_id: Uuid,
    name: &str,
) -> Result<(PackageRepositorySummary, Option<&'a NpmAuthUser>), StatusCode> {
    require_readable_repository(user, || find_repository_by_name(state, organization_id, name), |user, _repo_id, repo_organization_id| async move {
        require_same_organization(user, repo_organization_id)
    })
    .await
}

/// The personal-namespace counterpart, resolving via `/u/{username}/{repo}`. Unlike the org path,
/// it runs the full role check here rather than deferring it — a personal org's id never equals a
/// real user's `organization_id`, so the org-membership check would always reject even the owner.
pub async fn require_readable_personal_repository_by_name<'a>(
    state: &NpmState,
    user: Option<&'a NpmAuthUser>,
    username: &str,
    repo_name: &str,
) -> Result<(PackageRepositorySummary, Option<&'a NpmAuthUser>), StatusCode> {
    require_readable_repository(
        user,
        // The format check runs here, inside resolution, not after this function returns — it must
        // reject a non-npm repository before `verify` below ever runs a role check against the
        // database, so a DB error there can't turn a 404 into a 500.
        || async move {
            let repo = resolve_personal_repository(state, username, repo_name).await?;
            require_npm_format_repository(&repo)?;
            Ok(repo)
        },
        |user, repo_id, repo_organization_id| require_personal_repository_role(state, user, repo_id, repo_organization_id, Role::Read),
    )
    .await
}

/// Delegates to the shared primitive — kept here so npm's routes keep importing `require_same_organization` from `crate::authz` unchanged.
pub fn require_same_organization(user: &NpmAuthUser, organization_id: Uuid) -> Result<(), StatusCode> {
    authz_primitives::require_same_organization(user, organization_id).map_err(map_access_error)
}

/// A non-npm-format repository is unreachable through any npm route — 404, same as nonexistent.
/// Checks FORMAT only (npm vs docker); it does NOT check repo_type — see `require_hosted` for the
/// hosted/proxy/group distinction (B-19: this function was previously misleadingly named
/// `require_npm_hosted_repository`, implying a repo_type check it never performed).
pub fn require_npm_format_repository(repo: &PackageRepositorySummary) -> Result<(), StatusCode> {
    authz_primitives::require_format(repo, RepositoryFormat::Npm).map_err(map_access_error)
}

/// Writes only make sense against a hosted repository — proxy/group have no local storage.
pub fn require_hosted(repo: &PackageRepositorySummary) -> Result<(), StatusCode> {
    authz_primitives::require_hosted(repo).map_err(map_access_error)
}

/// Resolves `/u/{username}/{repo}` via Task 8's shared `ResolvePersonalRepositoryUseCase`
/// (stored on `NpmState`, same as every other application-layer use case this crate wires up),
/// translating its `ApplicationError`/`None` into the `StatusCode`s this crate's handlers expect.
/// A missing user, an unreserved personal namespace, and an unknown repo name are all 404 —
/// deliberately indistinguishable, same as the use case's own contract.
pub async fn resolve_personal_repository(state: &NpmState, username: &str, repo_name: &str) -> Result<PackageRepositorySummary, StatusCode> {
    state
        .resolve_personal_repository
        .execute(username, repo_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)
}

/// `require_repository_role`, but for a personal repository reached via `/u/{username}/{repo}`.
/// A personal org's only conceptual member is its owner, who already holds an explicit
/// `Permission::Admin` grant (Task 6) — the org-admin bypass never legitimately fires here, so a
/// denied caller always falls through to the plain permission-grant lookup. That denial must
/// still read as 404, not 403: an unauthorized caller must not learn a private personal project
/// exists at all, the same privacy goal `artiferris_api`'s personal-repository gate has.
pub async fn require_personal_repository_role(state: &NpmState, user: &NpmAuthUser, repository_id: Uuid, repository_organization_id: Uuid, minimum_role: Role) -> Result<(), StatusCode> {
    require_repository_role(state, user, repository_id, repository_organization_id, minimum_role).await.map_err(|status| match status {
        StatusCode::FORBIDDEN => StatusCode::NOT_FOUND,
        other => other,
    })
}

/// npm's per-member read policy for group traversal (C-1). Reading *through* a group must never be
/// broader than reading a member directly: the group's own (possibly public, possibly widely
/// granted) access stops at its boundary, and every member is re-checked on its own terms.
///
/// Takes the member's fields by value rather than a `&PackageRepositorySummary` on purpose — the
/// `authorize_member` hook it feeds (`artiferris_application::use_cases::group_resolve`) requires a
/// future that outlives the borrow the traversal hands it.
pub async fn member_is_readable(state: &NpmState, caller: Option<&NpmAuthUser>, member_id: Uuid, member_organization_id: Uuid, member_is_public: bool) -> bool {
    if member_is_public {
        return true;
    }
    let Some(user) = caller else {
        return false;
    };
    // `require_repository_role`'s grant lookup keys on (user id, repository id) alone — it's
    // organization-blind — so a stale cross-organization grant would otherwise be usable here. The
    // top-level repository gets this check in `require_readable_repository_by_name`, but that's
    // skipped entirely when the top level is public, so a member reached through a public group
    // would never get it. Re-assert it per member.
    //
    // A personal organization is the exception: its id never equals a real user's
    // `organization_id`, so an exact match can never hold there and demanding one locks a user out
    // of their own personal group's members. There the explicit grant IS the access model — same
    // branch `artiferris-api`'s `require_repository_access` takes — so fall through to the grant
    // lookup instead of rejecting.
    if require_same_organization(user, member_organization_id).is_err() && !organization_is_personal(state, member_organization_id).await {
        return false;
    }
    require_repository_role(state, user, member_id, member_organization_id, Role::Read).await.is_ok()
}

/// Only ever consulted once the cheap org-equality check has already failed — a real user's
/// `organization_id` is never a personal org's id, so a match rules a personal org out and saves
/// the query. Same shape as `artiferris-docker`'s `resolve_is_personal`. A lookup failure reads as
/// "not personal", which fails closed.
async fn organization_is_personal(state: &NpmState, organization_id: Uuid) -> bool {
    matches!(state.organizations.find_by_id(organization_id).await, Ok(Some(organization)) if organization.is_personal)
}

pub async fn require_repository_role(
    state: &NpmState,
    user: &NpmAuthUser,
    repository_id: Uuid,
    repository_organization_id: Uuid,
    minimum_role: Role,
) -> Result<(), StatusCode> {
    if user.is_super_admin {
        return Ok(());
    }
    if organization_admin_bypass_role(user.is_organization_admin, user.organization_id, repository_organization_id).is_some() {
        return Ok(());
    }
    let role = state.permissions.find_role(user.id, repository_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match role {
        Some(role) if role.satisfies(minimum_role) => Ok(()),
        _ => Err(StatusCode::FORBIDDEN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(is_super_admin: bool, organization_id: Uuid) -> NpmAuthUser {
        NpmAuthUser { id: Uuid::new_v4(), is_super_admin, is_organization_admin: false, organization_id }
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
    fn an_npm_format_repository_passes_require_npm_format_repository() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Npm, RepositoryType::Hosted);
        assert!(require_npm_format_repository(&r).is_ok());
    }

    #[test]
    fn an_npm_format_group_repository_passes_require_npm_format_repository() {
        // Documents that require_npm_format_repository checks FORMAT only, not repo_type —
        // require_hosted is the separate check for hosted-vs-proxy-vs-group (B-19).
        let repo = repo(Uuid::new_v4(), RepositoryFormat::Npm, RepositoryType::Group);
        assert!(require_npm_format_repository(&repo).is_ok());
    }

    #[test]
    fn a_docker_format_repository_is_unreachable_through_npm_routes() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Docker, RepositoryType::Hosted);
        let err = require_npm_format_repository(&r).unwrap_err();
        assert_eq!(err, StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_hosted_repository_passes_require_hosted() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Npm, RepositoryType::Hosted);
        assert!(require_hosted(&r).is_ok());
    }

    #[test]
    fn a_proxy_repository_is_rejected_by_require_hosted() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Npm, RepositoryType::Proxy);
        let err = require_hosted(&r).unwrap_err();
        assert_eq!(err, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn a_group_repository_is_rejected_by_require_hosted() {
        let r = repo(Uuid::new_v4(), RepositoryFormat::Npm, RepositoryType::Group);
        let err = require_hosted(&r).unwrap_err();
        assert_eq!(err, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn require_npm_format_repository_rejects_a_docker_format_repository() {
        let repo = PackageRepositorySummary {
            id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            name: "x".to_string(),
            format: RepositoryFormat::Docker,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
        };
        assert_eq!(require_npm_format_repository(&repo).unwrap_err(), StatusCode::NOT_FOUND);
    }

    // require_repository_by_name needs a real repository lookup, so these run against Postgres.
    mod db {
        use super::*;
        use artiferris_application::use_cases::api_token::{CreateApiTokenUseCase, ListApiTokensUseCase, RevokeApiTokenUseCase};
        use artiferris_application::use_cases::npm_audit::BulkAuditNpmPackagesUseCase;
        use artiferris_application::use_cases::npm_dependency_scan::ScanDependencyTreeUseCase;
        use artiferris_application::use_cases::npm_deprecate::DeprecateNpmVersionUseCase;
        use artiferris_application::use_cases::npm_dist_tags::{DeleteDistTagUseCase, ListDistTagsUseCase, SetDistTagUseCase};
        use artiferris_application::use_cases::npm_download::DownloadNpmTarballUseCase;
        use artiferris_application::use_cases::npm_metadata::GetNpmPackageMetadataUseCase;
        use artiferris_application::use_cases::npm_publish::PublishNpmPackageUseCase;
        use artiferris_application::use_cases::npm_search::SearchNpmPackagesUseCase;
        use artiferris_application::use_cases::npm_unpublish::UnpublishNpmPackageUseCase;
        use artiferris_application::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase;
        use artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend;
        use artiferris_infrastructure::http_npm_audit_client::HttpNpmAuditClient;
        use artiferris_infrastructure::http_remote_npm_registry::HttpRemoteNpmRegistry;
        use artiferris_infrastructure::postgres::api_token_repository::PostgresApiTokenRepository;
        use artiferris_infrastructure::postgres::npm_dependency_audit_repository::PostgresDependencyAuditRepository;
        use artiferris_infrastructure::postgres::npm_package_repository::PostgresNpmPackageRepository;
        use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
        use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
        use artiferris_infrastructure::postgres::permission_store::PostgresPermissionStore;
        use artiferris_infrastructure::postgres::user_repository::PostgresUserRepository;
        use artiferris_infrastructure::postgres::event_publisher::PostgresEventPublisher;
        use artiferris_domain::organization::{Organization, OrganizationSlug};
        use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort};
        use sqlx::PgPool;
        use std::sync::Arc;

        async fn test_state(pool: PgPool, root: &std::path::Path) -> NpmState {
            let users = Arc::new(PostgresUserRepository::new(pool.clone()));
            let repositories = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
            let permissions = Arc::new(PostgresPermissionStore::new(pool.clone()));
            let api_tokens: Arc<dyn artiferris_domain::api_token::ApiTokenRepositoryPort> = Arc::new(PostgresApiTokenRepository::new(pool.clone()));
            let organizations: Arc<dyn artiferris_domain::organization::OrganizationRepositoryPort> = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
            let npm_packages: Arc<dyn artiferris_domain::npm_package::NpmPackageRepositoryPort> = Arc::new(PostgresNpmPackageRepository::new(pool.clone()));
            let npm_audit: Arc<dyn artiferris_domain::npm_audit::NpmAuditPort> = Arc::new(HttpNpmAuditClient::new());
            let dependency_audits: Arc<dyn artiferris_domain::npm_audit::DependencyAuditRepositoryPort> = Arc::new(PostgresDependencyAuditRepository::new(pool.clone()));
            let storage: Arc<dyn artiferris_domain::storage::StorageBackendPort> = Arc::new(FilesystemStorageBackend::new(root.to_path_buf()));
            let events: Arc<dyn artiferris_domain::audit::EventPublisherPort> = Arc::new(PostgresEventPublisher::new(pool.clone()));
            let remote_registry: Arc<dyn artiferris_domain::npm_remote::RemoteNpmRegistryPort> = Arc::new(HttpRemoteNpmRegistry::new());

            NpmState {
                users: users.clone(),
                repositories: repositories.clone(),
                permissions: permissions.clone(),
                api_tokens: api_tokens.clone(),
                organizations: organizations.clone(),
                artiferris_base_domain: "artiferris.localhost".to_string(),
            public_scheme: "http".to_string(),
            guard: std::sync::Arc::new(artiferris_application::request_guard::RequestGuard::default()),
                publish: Arc::new(PublishNpmPackageUseCase::new(npm_packages.clone(), storage.clone(), repositories.clone(), repositories.clone(), events.clone())),
                metadata: Arc::new(GetNpmPackageMetadataUseCase::new(npm_packages.clone(), repositories.clone(), remote_registry.clone())),
                download: Arc::new(DownloadNpmTarballUseCase::new(npm_packages.clone(), storage.clone(), remote_registry.clone(), repositories.clone())),
                downloads: Arc::new(artiferris_domain::download_stats::NoopDownloadRecorder),
                unpublish: Arc::new(UnpublishNpmPackageUseCase::new(npm_packages.clone(), storage.clone(), events.clone())),
                deprecate: Arc::new(DeprecateNpmVersionUseCase::new(npm_packages.clone(), events.clone())),
                set_dist_tag: Arc::new(SetDistTagUseCase::new(npm_packages.clone(), events.clone())),
                delete_dist_tag: Arc::new(DeleteDistTagUseCase::new(npm_packages.clone())),
                list_dist_tags: Arc::new(ListDistTagsUseCase::new(npm_packages.clone())),
                search: Arc::new(SearchNpmPackagesUseCase::new(npm_packages.clone())),
                bulk_audit: Arc::new(BulkAuditNpmPackagesUseCase::new(npm_audit.clone())),
                scan_dependency_tree: Arc::new(ScanDependencyTreeUseCase::new(npm_packages.clone(), remote_registry.clone(), npm_audit.clone(), dependency_audits.clone())),
                create_api_token: Arc::new(CreateApiTokenUseCase::new(api_tokens.clone())),
                list_api_tokens: Arc::new(ListApiTokensUseCase::new(api_tokens.clone())),
                revoke_api_token: Arc::new(RevokeApiTokenUseCase::new(api_tokens.clone())),
                resolve_personal_repository: Arc::new(ResolvePersonalRepositoryUseCase::new(users.clone(), organizations.clone(), repositories.clone())),
            }
        }

        /// Every repository seeded below needs a real organization row (foreign key).
        async fn create_org(state: &NpmState, id: Uuid, slug: &str) {
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

        async fn seed_repository(pool: &PgPool, organization_id: Uuid, id: Uuid, format: &str, repo_type: &str) {
            sqlx::query!(
                "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, version, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, NULL, 1, now(), now())",
                id,
                organization_id,
                format!("repo-{id}"),
                format,
                repo_type,
            )
            .execute(pool)
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
            seed_repository(&pool, org_id, repo_id, "npm", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let caller = user(false, org_id);

            let found = require_repository_by_name(&state, &caller, org_id, &repo_name).await.unwrap();
            assert_eq!(found.id, repo_id);
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
            seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
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
            seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let super_admin = user(true, Uuid::new_v4());

            let found = require_repository_by_name(&state, &super_admin, acme_id, &repo_name).await.unwrap();
            assert_eq!(found.id, repo_id);
        }

        async fn mark_repository_public(pool: &PgPool, repository_id: Uuid) {
            let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
            let (version, _) = store.load(repository_id).await.unwrap();
            store
                .append(repository_id, version, vec![PackageRepositoryEvent::VisibilityChanged { repository_id, is_public: true }], Uuid::new_v4())
                .await
                .unwrap();
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_public_repository_is_readable_by_no_caller_at_all(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let acme_id = Uuid::new_v4();
            create_org(&state, acme_id, "acme").await;
            let repo_id = Uuid::new_v4();
            seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
            mark_repository_public(&pool, repo_id).await;
            let repo_name = format!("repo-{repo_id}");

            let (found, caller) = require_readable_repository_by_name(&state, None, acme_id, &repo_name).await.unwrap();
            assert_eq!(found.id, repo_id);
            assert!(caller.is_none(), "a public repository needs no role check, so no caller is handed back");
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_private_repository_rejects_a_caller_with_no_authorization_header_at_all(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let acme_id = Uuid::new_v4();
            create_org(&state, acme_id, "acme").await;
            let repo_id = Uuid::new_v4();
            seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
            let repo_name = format!("repo-{repo_id}");

            let err = require_readable_repository_by_name(&state, None, acme_id, &repo_name).await.unwrap_err();
            assert_eq!(err, StatusCode::NOT_FOUND);
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_nonexistent_repository_and_a_private_one_are_both_not_found_to_an_anonymous_caller(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let acme_id = Uuid::new_v4();
            create_org(&state, acme_id, "acme").await;

            let missing = require_readable_repository_by_name(&state, None, acme_id, "does-not-exist").await.unwrap_err();

            let repo_id = Uuid::new_v4();
            seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let private = require_readable_repository_by_name(&state, None, acme_id, &repo_name).await.unwrap_err();

            assert_eq!(missing, StatusCode::NOT_FOUND);
            assert_eq!(missing, private, "a private repository must be indistinguishable from a nonexistent one to an anonymous caller");
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_private_repository_still_requires_the_same_organization_when_a_caller_is_present(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let acme_id = Uuid::new_v4();
            create_org(&state, acme_id, "acme").await;
            let repo_id = Uuid::new_v4();
            seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let other_org_caller = user(false, Uuid::new_v4());

            let err = require_readable_repository_by_name(&state, Some(&other_org_caller), acme_id, &repo_name).await.unwrap_err();
            assert_eq!(err, StatusCode::NOT_FOUND);
        }

        #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
        async fn a_private_repository_hands_the_caller_back_for_a_role_check(pool: PgPool) {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(pool.clone(), dir.path()).await;
            let acme_id = Uuid::new_v4();
            create_org(&state, acme_id, "acme").await;
            let repo_id = Uuid::new_v4();
            seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
            let repo_name = format!("repo-{repo_id}");
            let caller = user(false, acme_id);

            let (found, returned_caller) = require_readable_repository_by_name(&state, Some(&caller), acme_id, &repo_name).await.unwrap();
            assert_eq!(found.id, repo_id);
            assert_eq!(returned_caller.unwrap().id, caller.id);
        }
    }
}
