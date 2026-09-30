use std::net::SocketAddr;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use artiferris_application::use_cases::list_readable_repositories::ReadableRepositoriesCaller;
use artiferris_application::use_cases::list_repository_packages::{RepositoryPackageTree, RepositoryPackagesPage, VulnerabilitySummary, MAX_PACKAGES_PER_PAGE};
use artiferris_application::use_cases::seo::repository_path;
use artiferris_domain::organization::Organization;
use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryFormat, RepositoryType};
use artiferris_domain::public_catalog::{OwnerKind, OwnerRef};
use artiferris_domain::permission::{Role, public_repository_bypass_role};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth_middleware::AuthUser;
use crate::authz::{effective_repository_role, repository_roles, require_repository_role, require_same_organization, RepositoryRoles};
use crate::dto::{application_error_response, ErrorResponse};
use crate::organization_middleware::ResolvedOrganization;
use crate::state::AppState;

mod docker;
mod npm;
mod permissions;

/// The shared gate of every `/api/repositories/{id}/...` handler: same organization as the caller, unless (a) the
/// repository is in a personal organization and the caller's grant satisfies `minimum_role` on it, or (b) it is public
/// and the caller is a public-organization member wanting no more than `Read`. (b) is checked against
/// `public_repository_bypass_role`, never through the organization-blind `Permission` lookup that (a)'s
/// `effective_repository_role` falls to: gating that on `is_public` would let a stale cross-organization grant on any
/// public repository act at its full role. Personal organizations stay single-member because `GrantPermissionUseCase`
/// rejects a grantee whose `organization_id` differs from the repository's. Real organizations still 404 on
/// cross-organization access beyond those two cases.
async fn require_repository_access(
    state: &AppState,
    user: &AuthUser,
    repository_organization_id: Uuid,
    repository_id: Uuid,
    repository_is_public: bool,
    minimum_role: Role,
    action: &str,
) -> Result<(), StatusCode> {
    if require_same_organization(user, repository_organization_id).is_err() {
        let organization = state.organizations.find_by_id(repository_organization_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let personal_grant_satisfies = organization.is_some_and(|org| org.is_personal)
            && effective_repository_role(state, user, repository_id).await?.is_some_and(|role| role.satisfies(minimum_role));
        let public_read_satisfies = public_repository_bypass_role(user.organization_id, repository_is_public).is_some_and(|role| role.satisfies(minimum_role));
        if !(personal_grant_satisfies || public_read_satisfies) {
            return Err(StatusCode::NOT_FOUND);
        }
    }
    require_repository_role(state, user, repository_id, minimum_role, action).await
}

/// The read-path counterpart of `require_repository_access`, for informational detail and security-audit routes, never
/// for writes or `list_permissions` (which goes through `require_management_access`, never honoring the public bypass).
/// Viewing a public repository's audit or scan findings is package-content transparency, open like the package listing.
/// A public repository is readable by anyone; a private one needs the full authenticated gate, and an absent caller
/// 404s like a missing repository.
async fn require_readable_repository_access(
    state: &AppState,
    user: Option<&AuthUser>,
    repository_organization_id: Uuid,
    repository_id: Uuid,
    repository_is_public: bool,
    action: &str,
) -> Result<(), StatusCode> {
    if repository_is_public {
        return Ok(());
    }
    let user = user.ok_or(StatusCode::NOT_FOUND)?;
    require_repository_access(state, user, repository_organization_id, repository_id, repository_is_public, Role::Read, action).await
}

/// Maps `require_repository_access`'s `StatusCode` to the same error bodies every handler
/// already used for these three outcomes.
fn repository_access_error(status: StatusCode) -> (StatusCode, Json<ErrorResponse>) {
    let message = match status {
        StatusCode::NOT_FOUND => "repository not found",
        StatusCode::FORBIDDEN => "forbidden",
        _ => "internal error",
    };
    (status, Json(ErrorResponse::message(message.to_string())))
}

/// The lookup half of the `find_by_id`, map internal error, 404 if missing pattern of these handlers. It deliberately
/// does no access check: which gate follows (`require_repository_access`, `require_readable_repository_access` or the
/// stricter `require_management_access`) is a security choice that stays explicit at each call site.
async fn load_repository(state: &AppState, id: Uuid) -> Result<PackageRepositorySummary, (StatusCode, Json<ErrorResponse>)> {
    state
        .repositories
        .find_by_id(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::message("internal error".to_string()))))?
        .ok_or((StatusCode::NOT_FOUND, Json(ErrorResponse::message("repository not found".to_string()))))
}

/// Anonymous callers of the package and image pages share the per-IP budget of the other public endpoints; signed-in ones are not limited here.
const ANONYMOUS_DETAILS_PER_MINUTE: usize = 60;
/// A page load reads the repository, its package list and a few more, so this is looser than the details budget.
const ANONYMOUS_REPOSITORY_READS_PER_MINUTE: usize = 120;

fn require_anonymous_budget(
    state: &AppState,
    user: &Option<AuthUser>,
    headers: &HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    spend_anonymous_budget(state, user, headers, connect_info, "package-details", ANONYMOUS_DETAILS_PER_MINUTE)
}

fn require_anonymous_repository_reads_budget(
    state: &AppState,
    user: &Option<AuthUser>,
    headers: &HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    spend_anonymous_budget(state, user, headers, connect_info, "repository-reads", ANONYMOUS_REPOSITORY_READS_PER_MINUTE)
}

fn spend_anonymous_budget(
    state: &AppState,
    user: &Option<AuthUser>,
    headers: &HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    scope: &str,
    limit: usize,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if user.is_none() && !crate::routes::public_catalog::within_budget(state, headers, connect_info, scope, limit) {
        return Err((StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse::message("too many requests, try again shortly".to_string()))));
    }
    Ok(())
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me/repository", get(get_my_personal_repository).post(reserve_personal_repository))
        .route("/api/me/repository/projects", get(list_my_projects).post(create_user_project))
        .route("/api/repositories", get(list_repositories).post(create_repository))
        .route("/api/repositories/{id}", get(get_repository).patch(rename_repository).delete(delete_repository))
        .route("/api/repositories/by-owner/{username}/{repo_name}", get(get_repository_by_owner))
        .route("/api/repositories/by-org/{slug}/{repo_name}", get(get_repository_by_org))
        .route("/api/repositories/{id}/group-members", post(add_group_member))
        .route("/api/repositories/{id}/group-members/{member_id}", axum::routing::delete(remove_group_member))
        .route("/api/repositories/{id}/quota", put(set_repository_quota))
        .route("/api/repositories/{id}/retention", put(set_retention_policy))
        .route("/api/repositories/{id}/visibility", put(set_repository_visibility))
        .route("/api/repositories/{id}/permissions", get(permissions::list_permissions))
        .route("/api/repositories/{id}/permissions/{user_id}", put(permissions::grant_permission).delete(permissions::revoke_permission))
        .route("/api/repositories/{id}/packages", get(list_repository_packages))
        .route(
            "/api/repositories/{id}/packages/npm/{name}",
            get(npm::get_npm_package_details).delete(npm::delete_npm_package),
        )
        .route("/api/repositories/{id}/packages/npm/{name}/versions/{version}", axum::routing::delete(npm::delete_npm_package_version))
        .route("/api/repositories/{id}/packages/npm/{name}/audit", get(npm::audit_npm_package))
        .route(
            "/api/repositories/{id}/packages/npm/{name}/versions/{version}/dependency-audit",
            get(npm::get_dependency_audit).post(npm::scan_dependency_tree),
        )
        .route(
            "/api/repositories/{id}/packages/docker/{image}",
            get(docker::get_docker_image_details).delete(docker::delete_docker_image),
        )
        .route("/api/repositories/{id}/packages/docker/{image}/tags/{tag}", axum::routing::delete(docker::delete_docker_tag))
        .route(
            "/api/repositories/{id}/packages/docker/{image}/tags/{tag}/scan",
            get(docker::get_docker_image_scan).post(docker::scan_docker_image),
        )
}

#[derive(Serialize)]
struct RepositoryResponse {
    id: Uuid,
    name: String,
    format: RepositoryFormat,
    repo_type: RepositoryType,
    remote_url: Option<String>,
    /// Never the credentials themselves.
    remote_credentials_set: bool,
    group_members: Vec<Uuid>,
    /// `None` means unlimited.
    quota_bytes: Option<i64>,
    /// `None` disables automatic cleanup.
    retention_keep_last_n: Option<i32>,
    is_public: bool,
    /// `Admin` for a super-admin whatever the grants, to let the frontend choose which actions to offer. `None` only
    /// for an anonymous caller on a public repository.
    my_role: Option<Role>,
    /// Lets a super-admin's client-side organization filter work, same as the Users list.
    organization_id: Uuid,
    /// The owning organization's display name — already the username itself for a personal org.
    owner_name: String,
    owner_is_personal: bool,
    /// The path of this repository's public page (e.g. `/@alice/libs` or `/o/acme/libs`), `None` while
    /// the repository is private. Built the same way the SEO head injection builds it, so a shared link
    /// always matches the page it points to.
    public_path: Option<String>,
}

impl RepositoryResponse {
    fn new(s: PackageRepositorySummary, my_role: Option<Role>, owner: &Organization) -> Self {
        let public_path = s.is_public.then(|| repository_path(&owner_ref(owner), &s.name));
        Self {
            id: s.id,
            name: s.name,
            format: s.format,
            repo_type: s.repo_type,
            remote_url: s.remote_url,
            remote_credentials_set: s.remote_username.is_some() || s.remote_password.is_some(),
            group_members: s.group_members,
            quota_bytes: s.quota_bytes,
            retention_keep_last_n: s.retention_keep_last_n,
            is_public: s.is_public,
            my_role,
            organization_id: s.organization_id,
            owner_name: owner.display_name.clone(),
            owner_is_personal: owner.is_personal,
            public_path,
        }
    }
}

/// A personal organization's `slug` is an internal, unguessable identifier (`personal_organization_slug`), never the
/// path of `/@username`, which uses the display name (the username itself).
fn owner_ref(owner: &Organization) -> OwnerRef {
    if owner.is_personal {
        OwnerRef { kind: OwnerKind::Personal, slug: owner.display_name.clone() }
    } else {
        OwnerRef { kind: OwnerKind::Organization, slug: owner.slug.as_str().to_string() }
    }
}

/// What a caller whose only access is the implicit `Read` of a public repository gets: no storage quota,
/// no retention policy and no internal organization id.
#[derive(Serialize)]
struct PublicRepositoryResponse {
    id: Uuid,
    name: String,
    format: RepositoryFormat,
    repo_type: RepositoryType,
    remote_url: Option<String>,
    remote_credentials_set: bool,
    group_members: Vec<Uuid>,
    is_public: bool,
    my_role: Option<Role>,
    owner_name: String,
    owner_is_personal: bool,
    public_path: Option<String>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum RepositoryView {
    Full(RepositoryResponse),
    Public(PublicRepositoryResponse),
}

impl RepositoryView {
    fn for_caller(s: PackageRepositorySummary, roles: RepositoryRoles, owner: &Organization) -> Self {
        if roles.explicit.is_some() {
            return Self::Full(RepositoryResponse::new(s, roles.effective, owner));
        }
        let public_path = s.is_public.then(|| repository_path(&owner_ref(owner), &s.name));
        Self::Public(PublicRepositoryResponse {
            id: s.id,
            name: s.name,
            format: s.format,
            repo_type: s.repo_type,
            remote_url: s.remote_url,
            remote_credentials_set: s.remote_username.is_some() || s.remote_password.is_some(),
            group_members: s.group_members,
            is_public: s.is_public,
            my_role: roles.effective,
            owner_name: owner.display_name.clone(),
            owner_is_personal: owner.is_personal,
            public_path,
        })
    }
}

async fn repository_view(state: &AppState, user: &Option<AuthUser>, repo: PackageRepositorySummary) -> Result<Json<RepositoryView>, StatusCode> {
    let roles = match user {
        Some(user) => repository_roles(state, user, repo.id).await?,
        None => RepositoryRoles { effective: None, explicit: None },
    };
    let owner = state.organizations.find_by_id(repo.organization_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(RepositoryView::for_caller(repo, roles, &owner)))
}

async fn list_repositories(
    State(state): State<AppState>,
    user: AuthUser,
    resolved_org: ResolvedOrganization,
) -> Result<Json<Vec<RepositoryView>>, StatusCode> {
    let caller = ReadableRepositoriesCaller { user_id: user.id, organization_id: user.organization_id, is_super_admin: user.is_super_admin, is_organization_admin: user.is_organization_admin };
    let readable = state.list_readable_repositories.execute(&caller, resolved_org.0.id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        readable
            .into_iter()
            .map(|r| RepositoryView::for_caller(r.repository, RepositoryRoles { effective: Some(r.role), explicit: r.explicit.then_some(r.role) }, &r.owner))
            .collect(),
    ))
}

/// A super-admin can target any org via `organization_id`; anyone else always gets their own.
#[derive(Deserialize)]
struct OrgScopeParams {
    organization_id: Option<Uuid>,
}

fn target_organization_id(user: &AuthUser, resolved_org: &ResolvedOrganization, requested: Option<Uuid>) -> Uuid {
    if user.is_super_admin { requested.unwrap_or(resolved_org.0.id) } else { user.organization_id }
}

#[derive(Deserialize)]
struct CreateRepositoryRequest {
    name: String,
    format: RepositoryFormat,
    repo_type: RepositoryType,
    remote_url: Option<String>,
    /// Ignored for any repo_type other than `proxy`.
    #[serde(default)]
    remote_username: Option<String>,
    #[serde(default)]
    remote_password: Option<String>,
    /// For a `group` repository: member repositories in resolution order.
    #[serde(default)]
    group_members: Option<Vec<Uuid>>,
    /// `None` (the default) leaves the quota unlimited.
    #[serde(default)]
    quota_bytes: Option<i64>,
    /// `None` (the default) leaves automatic cleanup disabled.
    #[serde(default)]
    retention_keep_last_n: Option<i32>,
}

/// `create_repository`'s error body: the same `{"error": "..."}` JSON as `ErrorResponse` while `repository_id` is
/// `None`, which holds for every failure before the repository exists. After creation, the group-member, quota and
/// retention steps are each fallible: a failure there sets `repository_id`, so the caller never mistakes a partially
/// configured repository for a total failure. Kept local rather than changing `application_error_response` or
/// `ErrorResponse`, which other handlers depend on.
#[derive(Serialize)]
struct CreateRepositoryErrorBody {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository_id: Option<Uuid>,
}

fn create_repository_error(status: StatusCode, message: impl Into<String>, repository_id: Option<Uuid>) -> (StatusCode, Json<CreateRepositoryErrorBody>) {
    (status, Json(CreateRepositoryErrorBody { error: message.into(), code: None, repository_id }))
}

/// Reuses `application_error_response`'s status and message mapping without its `Json<ErrorResponse>` body type.
fn create_repository_application_error(
    context: &str,
    error: artiferris_application::error::ApplicationError,
    repository_id: Option<Uuid>,
) -> (StatusCode, Json<CreateRepositoryErrorBody>) {
    let (status, Json(ErrorResponse { error: message, code })) = application_error_response(context, error);
    (status, Json(CreateRepositoryErrorBody { error: message, code, repository_id }))
}

async fn create_repository(
    State(state): State<AppState>,
    user: AuthUser,
    resolved_org: ResolvedOrganization,
    Query(scope): Query<OrgScopeParams>,
    Json(body): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryResponse>), (StatusCode, Json<CreateRepositoryErrorBody>)> {
    // Non-super-admins must be an admin of the org their own subdomain resolves to.
    if !user.is_super_admin {
        require_same_organization(&user, resolved_org.0.id).map_err(|status| create_repository_error(status, "not found", None))?;
        if !user.is_organization_admin {
            return Err(create_repository_error(StatusCode::FORBIDDEN, "forbidden", None));
        }
    }
    let organization_id = target_organization_id(&user, &resolved_org, scope.organization_id);
    let group_members = body.group_members.unwrap_or_default();

    // Validate every member up front, before creating anything: no `repository_id` exists yet for an error in this
    // block.
    if !group_members.is_empty() {
        if body.repo_type != RepositoryType::Group {
            return Err(create_repository_error(StatusCode::BAD_REQUEST, "group_members is only valid for a group repository", None));
        }
        for member_id in &group_members {
            let member = state
                .repositories
                .find_by_id(*member_id)
                .await
                .map_err(|_| create_repository_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error", None))?
                .ok_or_else(|| {
                    create_repository_application_error(
                        "failed to create repository",
                        artiferris_domain::error::DomainError::UnknownGroupMember(*member_id).into(),
                        None,
                    )
                })?;
            // Checked up front, so a cross-organization member is rejected before the repository is created.
            if member.organization_id != organization_id {
                return Err(create_repository_application_error(
                    "failed to create repository",
                    artiferris_domain::error::DomainError::GroupMemberOrganizationMismatch(*member_id).into(),
                    None,
                ));
            }
            if member.format != body.format {
                return Err(create_repository_application_error(
                    "failed to create repository",
                    artiferris_domain::error::DomainError::GroupMemberFormatMismatch(*member_id).into(),
                    None,
                ));
            }
            // As in `add_group_member`: a group's (possibly broader) access must not transitively expose a member the
            // caller cannot read on their own.
            require_repository_access(&state, &user, member.organization_id, member.id, member.is_public, Role::Read, "create repository")
                .await
                .map_err(repository_access_error)
                .map_err(|(status, Json(ErrorResponse { error, .. }))| create_repository_error(status, error, None))?;
        }
    }

    let id = state
        .create_repository
        .execute(
            organization_id,
            &body.name,
            body.format,
            body.repo_type,
            body.remote_url.clone(),
            body.remote_username.clone(),
            body.remote_password.clone(),
            user.id,
        )
        .await
        .map_err(|e| create_repository_application_error("failed to create repository", e, None))?;

    // From here the repository exists: full transactional atomicity across the three steps below, each with its own
    // optimistic-concurrency version, is more than this warrants, so every error carries `id` and no caller is told
    // "creation failed" for a repository that now exists.
    for (position, member_id) in group_members.iter().enumerate() {
        state
            .add_group_member
            .execute(id, *member_id, position as i32, user.id)
            .await
            .map_err(|e| create_repository_application_error("failed to add group member", e, Some(id)))?;
    }
    if let Some(quota_bytes) = body.quota_bytes {
        state
            .set_repository_quota
            .execute(id, Some(quota_bytes), user.id)
            .await
            .map_err(|e| create_repository_application_error("failed to set repository quota", e, Some(id)))?;
    }
    if let Some(keep_last_n) = body.retention_keep_last_n {
        state
            .set_retention_policy
            .execute(id, Some(keep_last_n), user.id)
            .await
            .map_err(|e| create_repository_application_error("failed to set retention policy", e, Some(id)))?;
    }

    let created = state
        .repositories
        .find_by_id(id)
        .await
        .map_err(|_| create_repository_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error", Some(id)))?
        .ok_or_else(|| create_repository_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error", Some(id)))?;
    let owner = state
        .organizations
        .find_by_id(organization_id)
        .await
        .map_err(|_| create_repository_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error", Some(id)))?
        .ok_or_else(|| create_repository_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error", Some(id)))?;
    Ok((StatusCode::CREATED, Json(RepositoryResponse::new(created, Some(Role::Admin), &owner))))
}

/// A second call is a `409`, not a silent no-op, so the frontend can tell "already have one" from "just created one".
async fn reserve_personal_repository(State(state): State<AppState>, user: AuthUser) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state.reserve_personal_organization.execute(user.id).await.map_err(|e| application_error_response("failed to reserve personal repository", e))?;
    Ok(StatusCode::CREATED)
}

/// The read-only counterpart of `reserve_personal_repository`: 404 if not reserved, rather than a 409 the caller would
/// have to trigger to find out.
async fn get_my_personal_repository(State(state): State<AppState>, user: AuthUser) -> StatusCode {
    match state.find_my_personal_organization.execute(user.id).await {
        Ok(Some(_)) => StatusCode::OK,
        Ok(None) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
struct CreateUserProjectRequest {
    name: String,
    format: RepositoryFormat,
    repo_type: RepositoryType,
}

async fn create_user_project(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateUserProjectRequest>,
) -> Result<(StatusCode, Json<RepositoryResponse>), (StatusCode, Json<ErrorResponse>)> {
    let id = state
        .create_user_project
        .execute(user.id, &body.name, body.format, body.repo_type)
        .await
        .map_err(|e| application_error_response("failed to create project", e))?;
    let created = state
        .repositories
        .find_by_id(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::message("internal error".to_string()))))?
        .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::message("internal error".to_string()))))?;
    let owner = state
        .organizations
        .find_by_id(created.organization_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::message("internal error".to_string()))))?
        .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::message("internal error".to_string()))))?;
    // As in `create_repository`: the creator just got Admin from `CreateUserProjectUseCase::execute`, so there is no
    // need for another round trip to derive it.
    Ok((StatusCode::CREATED, Json(RepositoryResponse::new(created, Some(Role::Admin), &owner))))
}

/// Separate from `list_repositories`: it touches only the caller's own personal organization. An empty array, not a
/// 404, when no namespace is reserved: the caller simply has no projects, unlike `get_my_personal_repository`'s
/// existence check.
async fn list_my_projects(State(state): State<AppState>, user: AuthUser) -> Result<Json<Vec<RepositoryResponse>>, StatusCode> {
    let Some(organization_id) = state.find_my_personal_organization.execute(user.id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? else {
        return Ok(Json(Vec::new()));
    };
    let owner = state.organizations.find_by_id(organization_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let org_repos = state.repositories.list_by_organization(organization_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let roles: std::collections::HashMap<Uuid, Role> =
        state.permissions.list_for_user(user.id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.into_iter().collect();
    let mine = org_repos
        .into_iter()
        .filter_map(|repo| roles.get(&repo.id).map(|role| RepositoryResponse::new(repo, Some(*role), &owner)))
        .collect();
    Ok(Json(mine))
}

async fn get_repository(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<RepositoryView>, StatusCode> {
    require_anonymous_repository_reads_budget(&state, &user, &headers, connect_info).map_err(|(status, _)| status)?;
    let repo = state.repositories.find_by_id(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::NOT_FOUND)?;
    // 404 not 403 — a cross-org caller shouldn't learn the repo exists at all.
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, id, repo.is_public, "view repository").await?;
    repository_view(&state, &user, repo).await
}

/// The read path of the public web view: a personal project by owner username and name, behind the same anonymous-safe
/// gate as `get_repository`. It never resolves an organization-owned repository (npm and Docker address a personal
/// project as `@username/name` too).
async fn get_repository_by_owner(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path((username, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<RepositoryView>, StatusCode> {
    require_anonymous_repository_reads_budget(&state, &user, &headers, connect_info).map_err(|(status, _)| status)?;
    let repo = state
        .resolve_personal_repository
        .execute(&username, &repo_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    // 404 not 403 — same reasoning as get_repository: a caller who can't read this repository
    // shouldn't learn it exists at all.
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, repo.id, repo.is_public, "view repository by owner").await?;
    repository_view(&state, &user, repo).await
}

/// The organization-owned counterpart of `get_repository_by_owner`, for the public catalog's links. It never resolves a
/// personal organization (those use `by-owner`), and a repository the caller cannot read is a 404, like a missing one.
async fn get_repository_by_org(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path((slug, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<RepositoryView>, StatusCode> {
    require_anonymous_repository_reads_budget(&state, &user, &headers, connect_info).map_err(|(status, _)| status)?;
    let repo = state
        .resolve_organization_repository
        .execute(&slug, &repo_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, repo.id, repo.is_public, "view repository by organization").await?;
    repository_view(&state, &user, repo).await
}

#[derive(Deserialize)]
struct RenameRepositoryRequest {
    name: String,
}

async fn rename_repository(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RenameRepositoryRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "rename repository").await.map_err(repository_access_error)?;
    state.rename_repository.execute(id, &body.name, user.id).await.map_err(|e| application_error_response("failed to rename repository", e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_repository(State(state): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "delete repository").await.map_err(repository_access_error)?;
    state.delete_repository.execute(id, user.id).await.map_err(|e| application_error_response("failed to delete repository", e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AddGroupMemberRequest {
    member_repository_id: Uuid,
    position: i32,
}

async fn add_group_member(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<AddGroupMemberRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "add group member").await.map_err(repository_access_error)?;
    // Attaching a repository the caller cannot read would let anyone who can read the group read it too: the caller
    // must be able to read the member on their own first.
    let member = load_repository(&state, body.member_repository_id).await?;
    require_repository_access(&state, &user, member.organization_id, member.id, member.is_public, Role::Read, "add group member")
        .await
        .map_err(repository_access_error)?;
    state.add_group_member.execute(id, body.member_repository_id, body.position, user.id).await.map_err(|e| application_error_response("failed to add group member", e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_group_member(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, member_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "remove group member").await.map_err(repository_access_error)?;
    state.remove_group_member.execute(id, member_id, user.id).await.map_err(|e| application_error_response("failed to remove group member", e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SetRepositoryQuotaRequest {
    /// `None` (a JSON `null`) clears the quota back to unlimited.
    quota_bytes: Option<i64>,
}

async fn set_repository_quota(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetRepositoryQuotaRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "set repository quota").await.map_err(repository_access_error)?;
    state.set_repository_quota.execute(id, body.quota_bytes, user.id).await.map_err(|e| application_error_response("failed to set repository quota", e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SetRetentionPolicyRequest {
    /// `None` (a JSON `null`) disables automatic cleanup.
    keep_last_n_versions: Option<i32>,
}

async fn set_retention_policy(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetRetentionPolicyRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "set retention policy").await.map_err(repository_access_error)?;
    state
        .set_retention_policy
        .execute(id, body.keep_last_n_versions, user.id)
        .await
        .map_err(|e| application_error_response("failed to set retention policy", e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SetRepositoryVisibilityRequest {
    is_public: bool,
}

async fn set_repository_visibility(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetRepositoryVisibilityRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "set repository visibility").await.map_err(repository_access_error)?;
    state.set_repository_visibility.execute(id, body.is_public, user.id).await.map_err(|e| application_error_response("failed to set repository visibility", e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
#[serde(tag = "format", rename_all = "snake_case")]
enum RepositoryPackagesResponse {
    Npm { packages: Vec<NpmPackageTreeResponse>, next_after: Option<String> },
    Docker { images: Vec<DockerImageTreeResponse>, next_after: Option<String> },
}

#[derive(Deserialize)]
struct PackagesPageParams {
    /// At most 200, which is also what a request without the parameter gets.
    limit: Option<usize>,
    /// The `next_after` of the previous page.
    after: Option<String>,
}

#[derive(Serialize)]
struct NpmPackageTreeResponse {
    name: String,
    versions: Vec<NpmPackageVersionResponse>,
    /// The package has more versions than are listed.
    truncated: bool,
    vulnerability_summary: VulnerabilitySummaryResponse,
}

#[derive(Serialize)]
struct NpmPackageVersionResponse {
    version: String,
    published_at: DateTime<Utc>,
    size_bytes: i64,
    deprecated: bool,
}

#[derive(Serialize)]
struct DockerImageTreeResponse {
    image_name: String,
    tags: Vec<String>,
    /// The image has more tags than are listed.
    truncated: bool,
    vulnerability_summary: VulnerabilitySummaryResponse,
}

#[derive(Serialize)]
struct VulnerabilitySummaryResponse {
    critical: i64,
    high: i64,
    medium: i64,
    low: i64,
}

impl From<VulnerabilitySummary> for VulnerabilitySummaryResponse {
    fn from(s: VulnerabilitySummary) -> Self {
        Self { critical: s.critical, high: s.high, medium: s.medium, low: s.low }
    }
}

async fn list_repository_packages(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path(id): Path<Uuid>,
    Query(page): Query<PackagesPageParams>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<RepositoryPackagesResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_anonymous_repository_reads_budget(&state, &user, &headers, connect_info)?;
    let repo = load_repository(&state, id).await?;
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, id, repo.is_public, "browse repository packages").await.map_err(repository_access_error)?;
    let RepositoryPackagesPage { tree, next_after } = state
        .list_repository_packages
        .execute(id, repo.format, page.after.as_deref(), page.limit.unwrap_or(MAX_PACKAGES_PER_PAGE))
        .await
        .map_err(|e| application_error_response("failed to list repository packages", e))?;
    Ok(Json(match tree {
        RepositoryPackageTree::Npm(packages) => RepositoryPackagesResponse::Npm {
            next_after,
            packages: packages
                .into_iter()
                .map(|p| NpmPackageTreeResponse {
                    name: p.name,
                    versions: p
                        .versions
                        .into_iter()
                        .map(|v| NpmPackageVersionResponse {
                            version: v.version,
                            published_at: v.published_at,
                            size_bytes: v.size_bytes,
                            deprecated: v.deprecated,
                        })
                        .collect(),
                    truncated: p.truncated,
                    vulnerability_summary: p.vulnerability_summary.into(),
                })
                .collect(),
        },
        RepositoryPackageTree::Docker(images) => RepositoryPackagesResponse::Docker {
            next_after,
            images: images
                .into_iter()
                .map(|i| DockerImageTreeResponse { image_name: i.image_name, tags: i.tags, truncated: i.truncated, vulnerability_summary: i.vulnerability_summary.into() })
                .collect(),
        },
    }))
}


#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::config::Config;

    pub(crate) fn test_config() -> Config {
        Config {
            database_url: String::new(),
            jwt_secret: "test-secret".to_string(),
            secrets_encryption_key: "test-secrets-encryption-key".to_string(),
            storage_root: std::env::temp_dir().to_string_lossy().to_string(),
            bind_addr: "0.0.0.0:0".to_string(),
            cors_allowed_origin: None,
            docker_token_realm_override: None,
            public_url: "http://localhost:4200".to_string(),
            db_max_connections: artiferris_infrastructure::postgres::DEFAULT_DB_MAX_CONNECTIONS,
            artiferris_base_domain: "artiferris.localhost".to_string(),
            trusted_proxy_ips: std::collections::HashSet::new(),
            audit_retention_days: None,
        }
    }

    pub(super) async fn bearer(state: &AppState, organization_id: Uuid, username: &str, password: &str, is_super_admin: bool) -> String {
        state.create_user.execute(organization_id, username, password, is_super_admin).await.unwrap();
        state.authenticate_user.execute(username, password).await.unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_support::{bearer, test_config};
    use crate::{build_router, state::AppState};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;


    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn super_admin_can_create_a_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"my-npm-repo","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_super_admin_can_target_a_specific_organizations_repository_via_the_query_param(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        let app = build_router(state.clone());

        // No `host` header — this would otherwise resolve to the public organization.
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repositories?organization_id={acme_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"acme-repo","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let created_id = Uuid::parse_str(json["id"].as_str().unwrap()).unwrap();
        let created = state.repositories.find_by_id(created_id).await.unwrap().unwrap();
        assert_eq!(created.organization_id, acme_id, "?organization_id= must target that organization, not whichever one the request's domain resolves to");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_cannot_use_the_organization_id_query_param_to_create_a_repository_in_another_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let org_admin_id = state.create_user.execute(acme_id, "org-admin", "sup3r-s3cret!", false).await.unwrap();
        state.users.set_organization_admin(org_admin_id, true, None).await.unwrap();
        let token = state.authenticate_user.execute("org-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repositories?organization_id={other_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    // organization_id doesn't override the domain for a non-super-admin.
                    .header("host", "acme.artiferris.localhost")
                    .body(Body::from(r#"{"name":"escape-attempt","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let created_id = Uuid::parse_str(json["id"].as_str().unwrap()).unwrap();
        let created = state.repositories.find_by_id(created_id).await.unwrap().unwrap();
        assert_eq!(created.organization_id, acme_id, "an organization admin must not be able to use ?organization_id= to escape their own organization");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn renaming_a_repository_to_an_already_taken_name_returns_the_real_error_message(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "taken-name", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let other_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "other-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/repositories/{other_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"taken-name"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["error"].as_str().unwrap_or("").is_empty(), "the failure must carry a real message, not an empty body");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_group_repository_can_be_created_with_initial_members_in_order(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let member_a = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "member-a", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let member_b = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "member-b", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(format!(
                        r#"{{"name":"my-group","format":"npm","repo_type":"group","remote_url":null,"group_members":["{member_a}","{member_b}"]}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["group_members"], serde_json::json!([member_a, member_b]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_group_with_an_unknown_member_is_rejected_and_creates_nothing(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        let ghost_id = Uuid::new_v4();
        let app = build_router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(format!(
                        r#"{{"name":"my-group","format":"npm","repo_type":"group","remote_url":null,"group_members":["{ghost_id}"]}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

        let retry = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"my-group","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retry.status(), axum::http::StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_group_with_a_cross_organization_member_is_rejected_and_creates_nothing(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let acme_admin =
            state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        // A member repository in a DIFFERENT org than this request resolves to (public, since no Host header is set below).
        let cross_org_member =
            state.create_repository.execute(acme_id, "acme-member", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, acme_admin).await.unwrap();
        let app = build_router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(format!(
                        r#"{{"name":"my-group","format":"npm","repo_type":"group","remote_url":null,"group_members":["{cross_org_member}"]}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        // Proves no orphaned repository was left behind: the same name is still free.
        assert!(state.repositories.find_by_org_and_name(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-group").await.unwrap().is_none());

        let retry = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"my-group","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retry.status(), axum::http::StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_group_with_a_mismatched_format_member_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let docker_member =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "docker-member", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(format!(
                        r#"{{"name":"my-npm-group","format":"npm","repo_type":"group","remote_url":null,"group_members":["{docker_member}"]}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_proxy_with_remote_credentials_reports_them_set_but_never_returns_them(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        r#"{"name":"my-proxy","format":"npm","repo_type":"proxy","remote_url":"https://registry.example.com","remote_username":"svc-account","remote_password":"s3cret-token"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("s3cret-token"), "the raw password must never appear in the response body: {text}");
        assert!(!text.contains("svc-account"), "the raw username must never appear in the response body: {text}");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["remote_credentials_set"], true);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn quota_and_retention_can_be_set_at_creation_time(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        r#"{"name":"my-repo","format":"npm","repo_type":"hosted","remote_url":null,"quota_bytes":1000000,"retention_keep_last_n":5}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["quota_bytes"], 1000000);
        assert_eq!(json["retention_keep_last_n"], 5);
    }

    /// The steps after creation (group members, quota, retention) are each fallible. Members are validated up front, so
    /// `add_group_member` cannot fail after creation without a race, while `quota_bytes` has no up-front validation
    /// (the negative-quota check lives in `SetRepositoryQuotaUseCase`), so it is the smallest reliable way to fail a
    /// step after creation.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failure_setting_the_quota_after_creation_reports_the_repository_id_not_a_bare_failure(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        let app = build_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        r#"{"name":"my-repo","format":"npm","repo_type":"hosted","remote_url":null,"quota_bytes":-1}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["error"].as_str().unwrap_or("").is_empty(), "the failure must carry a real message, not an empty body");
        // The repository already exists when the quota step fails: the response must say so.
        let repository_id = Uuid::parse_str(
            json["repository_id"].as_str().expect("repository_id must be present once the repository has actually been created"),
        )
        .unwrap();
        let created = state.repositories.find_by_id(repository_id).await.unwrap();
        assert!(created.is_some(), "the repository reported in the error response must actually exist");
        assert_eq!(created.unwrap().name, "my-repo");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_duplicate_repository_name_keeps_its_specific_error_message(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        state
            .create_repository
            .execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "taken-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, Uuid::new_v4())
            .await
            .unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"taken-repo","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "repository name already taken");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_name_is_reusable_after_deletion(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await;
        let admin_id = state.users.find_by_username(&artiferris_domain::user::Username::parse("admin").unwrap()).await.unwrap().unwrap().id;
        let first_id = state
            .create_repository
            .execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "recyclable", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        state.delete_repository.execute(first_id, admin_id).await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"recyclable","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_non_admin_cannot_create_a_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "regular", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"my-npm-repo","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_admin_sets_and_clears_a_repository_quota(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/quota"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"quota_bytes":1000000}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);

        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["quota_bytes"], 1000000);

        let clear_response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/quota"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"quota_bytes":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(clear_response.status(), axum::http::StatusCode::NO_CONTENT);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_negative_quota_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/quota"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"quota_bytes":-1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_write_only_user_cannot_set_the_repository_quota(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let writer_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "writer", "sup3r-s3cret!", false).await.unwrap();
        state.grant_permission.execute(writer_id, repo_id, Role::Write, admin_id).await.unwrap();
        let writer_token = state.authenticate_user.execute("writer", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/quota"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {writer_token}"))
                    .body(Body::from(r#"{"quota_bytes":1000000}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_admin_sets_and_clears_a_retention_policy(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/retention"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"keep_last_n_versions":5}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);

        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["retention_keep_last_n"], 5);

        let clear_response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/retention"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"keep_last_n_versions":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(clear_response.status(), axum::http::StatusCode::NO_CONTENT);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_retention_policy_below_one_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/retention"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"keep_last_n_versions":0}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_write_only_user_cannot_set_the_retention_policy(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let writer_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "writer", "sup3r-s3cret!", false).await.unwrap();
        state.grant_permission.execute(writer_id, repo_id, Role::Write, admin_id).await.unwrap();
        let writer_token = state.authenticate_user.execute("writer", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/retention"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {writer_token}"))
                    .body(Body::from(r#"{"keep_last_n_versions":5}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_admin_sets_repository_visibility(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/visibility"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"is_public":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);

        let get_response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["is_public"], true);
    }

    /// A public proxy would be an anonymous relay to its upstream using the repository's own stored credentials.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn making_a_proxy_repository_public_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(
                Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                "my-proxy",
                RepositoryFormat::Npm,
                RepositoryType::Proxy,
                Some("https://registry.npmjs.org".to_string()),
                None,
                None,
                admin_id,
            )
            .await
            .unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/visibility"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"is_public":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    /// `resolve_in_group` authorizes only once, at the group: a public group would silently expose every member
    /// whatever its own visibility.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn making_a_group_repository_public_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let group_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "my-group", RepositoryFormat::Npm, RepositoryType::Group, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{group_id}/visibility"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"is_public":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn setting_visibility_on_a_repository_in_another_organization_returns_not_found(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        let other_token = bearer(&state, other_id, "other-user", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/visibility"))
                    .header("host", "other.artiferris.localhost")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {other_token}"))
                    .body(Body::from(r#"{"is_public":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_user_without_permission_cannot_view_a_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "private-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "outsider", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_view_a_public_repositorys_details(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let app = build_router(state);

        // No Authorization header at all.
        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["my_role"].is_null(), "an anonymous caller has no role of their own, not an implicit \"read\"");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_view_a_private_repositorys_details(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "private-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}")).body(Body::empty()).unwrap()).await.unwrap();

        // 404 not 401 — same "don't leak existence" contract as an authenticated cross-org caller.
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    /// Regression: `require_readable_repository_access`'s public branch grants access without consulting the caller's
    /// role, so an authenticated caller from an unrelated organization must see the repository, and
    /// `effective_repository_role` reports it honestly as an implicit `my_role: "read"` rather than 403 or `null`.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_authenticated_caller_from_an_unrelated_organization_can_view_a_public_repositorys_details(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let token = bearer(&state, other_id, "other-user", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}")).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK, "an authenticated but unrelated caller must still see a public repository's details");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["my_role"], "read", "a public repository grants an implicit Read to any caller, including one from an unrelated organization (B-43)");
    }

    /// The same case reached by a same-organization member with no explicit grant: it also gets the implicit public
    /// `Read`, since the bypass no longer depends on membership of the public organization.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_authenticated_same_organization_member_with_no_grant_can_view_a_public_repositorys_details(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let token = bearer(&state, acme_id, "acme-member", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}")).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK, "same-organization membership alone must not turn into a 403 just because there's no explicit grant");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["my_role"], "read", "a public repository grants an implicit Read to any caller, including a same-organization member with no explicit grant (B-43)");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn granting_read_access_allows_viewing_the_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "shared-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let member_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "member", "sup3r-s3cret!", false).await.unwrap();
        state.grant_permission.execute(member_id, repo_id, Role::Read, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("member", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_public_organization_member_can_view_a_public_repository_in_another_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "public-org-member", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK, "a public-organization member gets implicit Read on any public repository, no explicit grant needed");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_public_organization_member_cannot_write_to_a_public_repository_in_another_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "public-org-member", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"renamed-by-outsider"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // 404, not 403: `require_repository_access`'s cross-organization branch checks the bypass role against
        // `minimum_role` itself, so an insufficient bypass role reads as "not accessible", as for a
        // personal-organization owner.
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND, "visibility grants Read, never more — a public-organization member cannot rename a repository they don't own");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_public_organization_member_cannot_view_a_private_repository_in_another_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "public-org-member", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND, "being a public-organization member is not enough on its own — the repository must actually be public");
    }

    /// Regression: gating the public bypass on `is_public` rather than on the caller being a public-organization member
    /// would let this stale grant act at its full role on any public repository, not just Read.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_stale_cross_organization_permission_grant_does_not_work_even_on_a_public_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();

        // Grant as super-admin (only way to cross orgs), then demote — leaves a stale
        // out-of-org grant, at Admin, on a repository that is now public.
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "another-super-admin", "sup3r-s3cret!", true).await.unwrap();
        let other_user_id = state.create_user.execute(other_id, "other-user", "sup3r-s3cret!", true).await.unwrap();
        state.grant_permission.execute(other_user_id, repo_id, Role::Admin, admin_id).await.unwrap();
        state.set_super_admin.execute(other_user_id, false, admin_id).await.unwrap();
        let other_token = state.authenticate_user.execute("other-user", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {other_token}"))
                    .body(Body::from(r#"{"name":"renamed-via-stale-grant"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::NOT_FOUND,
            "a stale cross-org Admin grant must not work on a public repository just because it's public — the public bypass only ever grants Read, and only to a genuine public-organization member"
        );
    }

    /// Regression: a bypass that returns early and shadows the caller's real `Permission` row would silently downgrade
    /// this owner from Admin to Read when they make their own project public.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn making_a_personal_project_public_does_not_downgrade_its_owners_admin_grant(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"my-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(create_response.into_body(), usize::MAX).await.unwrap();
        let repo_id = Uuid::parse_str(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"].as_str().unwrap()).unwrap();

        let make_public_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/visibility"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"is_public":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(make_public_response.status(), axum::http::StatusCode::NO_CONTENT, "alice, as the owner, must still be able to make her own project public");

        let rename_response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"my-renamed-lib"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            rename_response.status(),
            axum::http::StatusCode::NO_CONTENT,
            "alice's own explicit Admin grant must survive her project becoming public — the public bypass (Read only) must never shadow a stronger explicit grant"
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_response_reports_its_owning_organizations_display_name(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        state.users.set_organization_admin(admin_id, true, None).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["owner_name"], "Acme Corp");
        assert_eq!(json["owner_is_personal"], false);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_personal_projects_repository_response_reports_the_owners_username(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"my-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(create_response.into_body(), usize::MAX).await.unwrap();
        let repo_id = Uuid::parse_str(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"].as_str().unwrap()).unwrap();

        let get_response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(get_response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["owner_name"], "alice");
        assert_eq!(json["owner_is_personal"], true);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_public_organizations_listing_includes_public_personal_projects_but_not_private_ones(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let create_public = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"name":"public-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(create_public.into_body(), usize::MAX).await.unwrap();
        let public_repo_id = Uuid::parse_str(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"].as_str().unwrap()).unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"name":"private-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{public_repo_id}/visibility"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"is_public":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        let bob_token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "bob", "sup3r-s3cret!", false).await;
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/api/repositories").header("authorization", format!("Bearer {bob_token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let repos = json.as_array().unwrap();
        let public_entry = repos.iter().find(|r| r["name"] == "public-lib");
        assert!(public_entry.is_some(), "the public organization's listing must include a public personal project, even without an explicit grant");
        let public_entry = public_entry.unwrap();
        assert_eq!(public_entry["owner_name"], "alice");
        assert_eq!(public_entry["owner_is_personal"], true);
        assert_eq!(public_entry["my_role"], "read");
        for hidden in ["quota_bytes", "retention_keep_last_n", "organization_id"] {
            assert!(public_entry.get(hidden).is_none(), "the implicit-Read entry of another user's project must not carry {hidden}");
        }
        assert!(repos.iter().all(|r| r["name"] != "private-lib"), "a private personal project must never show up in the public organization's listing");

        // The owner's own explicit Admin grant on her project must survive the same listing —
        // the implicit Read bypass must never shadow a stronger explicit grant here either.
        let self_view = app
            .clone()
            .oneshot(Request::builder().uri("/api/repositories").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let self_body = axum::body::to_bytes(self_view.into_body(), usize::MAX).await.unwrap();
        let self_json: serde_json::Value = serde_json::from_slice(&self_body).unwrap();
        let self_entry = self_json.as_array().unwrap().iter().find(|r| r["name"] == "public-lib").unwrap();
        assert_eq!(self_entry["my_role"], "admin", "the owner's real Admin grant must not be downgraded to Read just because her project is also public");
        assert!(self_entry.get("quota_bytes").is_some() && self_entry.get("organization_id").is_some(), "the owner keeps the full shape");

        // A caller from an unrelated organization must never see this: gating on the organization being viewed alone
        // would leak the repository's existence and its owner's name to any outsider hitting the public organization's
        // listing.
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let carol_token = bearer(&state, acme_id, "carol", "sup3r-s3cret!", false).await;
        let carol_no_host = app
            .clone()
            .oneshot(Request::builder().uri("/api/repositories").header("authorization", format!("Bearer {carol_token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let carol_no_host_body = axum::body::to_bytes(carol_no_host.into_body(), usize::MAX).await.unwrap();
        let carol_no_host_json: serde_json::Value = serde_json::from_slice(&carol_no_host_body).unwrap();
        assert_eq!(
            carol_no_host_json.as_array().unwrap().len(),
            0,
            "a caller from an unrelated real organization must not see a public personal project, even with no Host header (defaults to the public org's own listing)"
        );
        let carol_public_host = app
            .oneshot(
                Request::builder()
                    .uri("/api/repositories")
                    .header("host", "www.artiferris.localhost")
                    .header("authorization", format!("Bearer {carol_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let carol_public_host_body = axum::body::to_bytes(carol_public_host.into_body(), usize::MAX).await.unwrap();
        let carol_public_host_json: serde_json::Value = serde_json::from_slice(&carol_public_host_body).unwrap();
        assert_eq!(
            carol_public_host_json.as_array().unwrap().len(),
            0,
            "a caller from an unrelated real organization must not see a public personal project even when explicitly browsing the public organization's own listing"
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn adding_a_group_member_via_the_api(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let group_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "group-repo", RepositoryFormat::Npm, RepositoryType::Group, None, None, None, admin_id).await.unwrap();
        let member_repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "member-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repositories/{group_id}/group-members"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(format!(r#"{{"member_repository_id":"{member_repo_id}","position":0}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    }

    /// Write side: Admin on the group is not enough. Attaching a repository the caller cannot read would let everyone
    /// who reads the group read that member through it, so the member's own access is checked too. `carol` is
    /// deliberately not an organization admin, since the bypass gives an organization admin Admin on every repository
    /// of their organization.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn adding_a_group_member_the_caller_cannot_read_is_refused(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let org_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let admin_id = state.create_user.execute(org_id, "admin", "sup3r-s3cret!", true).await.unwrap();
        let group_id = state.create_repository.execute(org_id, "carols-group", RepositoryFormat::Npm, RepositoryType::Group, None, None, None, admin_id).await.unwrap();
        let private_id = state.create_repository.execute(org_id, "someone-elses-private", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();

        // Carol administers the group, and holds nothing at all on the private repository.
        let carol_id = state.create_user.execute(org_id, "carol", "sup3r-s3cret!", false).await.unwrap();
        state.grant_permission.execute(carol_id, group_id, Role::Admin, admin_id).await.unwrap();
        let carol_token = state.authenticate_user.execute("carol", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repositories/{group_id}/group-members"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {carol_token}"))
                    .body(Body::from(format!(r#"{{"member_repository_id":"{private_id}","position":0}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::FORBIDDEN,
            "Admin on the group must not be enough to attach a member the caller has no Read on"
        );
        let group = state.repositories.find_by_id(group_id).await.unwrap().unwrap();
        assert!(!group.group_members.contains(&private_id), "the refused member must not have been attached anyway");
    }

    /// `create_repository` gained the same member-readability check. No caller can reach it today without Read on every
    /// same-organization member, so it is defense in depth: this pins that it does not break the legitimate path.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_can_still_create_a_group_around_a_private_member_of_their_own_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        state.users.set_organization_admin(admin_id, true, None).await.unwrap();
        let member_id = state.create_repository.execute(acme_id, "private-member", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("host", "acme.artiferris.localhost")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(format!(
                        r#"{{"name":"my-group","format":"npm","repo_type":"group","remote_url":null,"group_members":["{member_id}"]}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn browsing_an_npm_repository_lists_its_packages_and_versions(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("1.0.0").unwrap(),
                manifest: serde_json::json!({}),
                shasum: "shasum".to_string(),
                integrity: "integrity".to_string(),
                tarball_storage_key: "key".to_string(),
                tarball_size_bytes: 42,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["format"], "npm");
        assert_eq!(json["packages"][0]["name"], "left-pad");
        assert_eq!(json["packages"][0]["versions"][0]["version"], "1.0.0");
        assert_eq!(json["packages"][0]["versions"][0]["size_bytes"], 42);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn browsing_a_docker_repository_lists_its_images_and_tags(pool: sqlx::PgPool) {
        use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerMediaType};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        let image_name = DockerImageName::parse("my-app").unwrap();
        let manifest = DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            image_name: image_name.clone(),
            digest: Digest::of(b"{}"),
            media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(),
            created_at: chrono::Utc::now(),
        };
        state.docker_manifests.insert_manifest(&manifest, &[]).await.unwrap();
        state.docker_manifests.set_tag(repo_id, &image_name, "latest", manifest.id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["format"], "docker");
        assert_eq!(json["images"][0]["image_name"], "my-app");
        assert_eq!(json["images"][0]["tags"], serde_json::json!(["latest"]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_user_without_permission_cannot_browse_a_repositorys_packages(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "private-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "outsider", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_browse_a_public_repositorys_packages(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let app = build_router(state);

        // No Authorization header at all.
        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_browse_a_private_repositorys_packages(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "private-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_in_one_organization_is_not_reachable_from_another_organizations_subdomain(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        let other_token = bearer(&state, other_id, "other-user", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("host", "other.artiferris.localhost")
                    .header("authorization", format!("Bearer {other_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_super_admin_can_still_reach_any_organizations_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        let super_admin_token = bearer(&state, other_id, "the-super-admin", "sup3r-s3cret!", true).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("host", "other.artiferris.localhost")
                    .header("authorization", format!("Bearer {super_admin_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn adding_a_group_member_to_a_repository_in_another_organization_returns_not_found(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(acme_id, "backend-group", RepositoryFormat::Npm, RepositoryType::Group, None, None, None, admin_id)
            .await
            .unwrap();
        let other_token = bearer(&state, other_id, "other-user", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repositories/{repo_id}/group-members"))
                    .header("host", "other.artiferris.localhost")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {other_token}"))
                    .body(Body::from(format!(r#"{{"member_repository_id":"{}","position":0}}"#, Uuid::new_v4())))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_sees_a_repository_in_their_org_they_never_created_or_were_granted_on(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let creator_id = state.create_user.execute(acme_id, "creator", "sup3r-s3cret!", false).await.unwrap();
        state.create_repository.execute(acme_id, "acme-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, creator_id).await.unwrap();
        let org_admin_id = state.create_user.execute(acme_id, "org-admin", "sup3r-s3cret!", false).await.unwrap();
        state.users.set_organization_admin(org_admin_id, true, None).await.unwrap();
        let token = state.authenticate_user.execute("org-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repositories")
                    .header("host", "acme.artiferris.localhost")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let repos = json.as_array().unwrap();
        assert_eq!(repos.len(), 1, "an organization admin must see every repository in their own organization, not just ones they created or were explicitly granted on");
        assert_eq!(repos[0]["name"], "acme-repo");
        assert_eq!(repos[0]["my_role"], "admin");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_does_not_see_another_organizations_repositories(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let other_creator = state.create_user.execute(other_id, "other-creator", "sup3r-s3cret!", false).await.unwrap();
        state.create_repository.execute(other_id, "other-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, other_creator).await.unwrap();
        let org_admin_id = state.create_user.execute(acme_id, "org-admin", "sup3r-s3cret!", false).await.unwrap();
        state.users.set_organization_admin(org_admin_id, true, None).await.unwrap();
        let token = state.authenticate_user.execute("org-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repositories")
                    .header("host", "acme.artiferris.localhost")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 0, "an organization admin must never see another organization's repositories");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_can_rename_a_repository_in_their_org_they_never_created_or_were_granted_on(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let creator_id = state.create_user.execute(acme_id, "creator", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "acme-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, creator_id).await.unwrap();
        let org_admin_id = state.create_user.execute(acme_id, "org-admin", "sup3r-s3cret!", false).await.unwrap();
        state.users.set_organization_admin(org_admin_id, true, None).await.unwrap();
        let token = state.authenticate_user.execute("org-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"renamed-repo"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_of_a_different_organization_cannot_rename_a_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let creator_id = state.create_user.execute(acme_id, "creator", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "acme-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, creator_id).await.unwrap();
        let org_admin_id = state.create_user.execute(other_id, "other-admin", "sup3r-s3cret!", false).await.unwrap();
        state.users.set_organization_admin(org_admin_id, true, None).await.unwrap();
        let token = state.authenticate_user.execute("other-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"renamed-repo"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::NOT_FOUND,
            "require_same_organization (checked before require_repository_role) must reject a different organization's admin without confirming the repository exists"
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_can_view_a_repository_they_never_created_or_were_granted_on(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let creator_id = state.create_user.execute(acme_id, "creator", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "acme-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, creator_id).await.unwrap();
        let org_admin_id = state.create_user.execute(acme_id, "org-admin", "sup3r-s3cret!", false).await.unwrap();
        state.users.set_organization_admin(org_admin_id, true, None).await.unwrap();
        let token = state.authenticate_user.execute("org-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK, "require_repository_role already grants an org-admin Read access here — the response body's own my_role computation must not re-check a raw permission grant and 403 anyway");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["my_role"], "admin");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn reserving_a_personal_repository_creates_a_hidden_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn reserving_twice_returns_conflict(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        for _ in 0..2 {
            app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/me/repository")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_project_after_reserving_succeeds_and_the_creator_can_manage_it(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"my-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let repo_id = created["id"].as_str().unwrap();
        assert_eq!(created["my_role"], "admin");

        // The creator's own permission grant on the repo (not organization membership —
        // a personal namespace's hidden organization never matches the creator's real
        // organization_id) must be enough to fetch it back through the ordinary route.
        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), axum::http::StatusCode::OK);

        let rename_response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"renamed-lib"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rename_response.status(), axum::http::StatusCode::NO_CONTENT);

        let renamed = state.repositories.find_by_id(Uuid::parse_str(repo_id).unwrap()).await.unwrap().unwrap();
        assert_eq!(renamed.name, "renamed-lib");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_personal_repository_is_not_reachable_by_another_user(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"name":"my-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(create_response.into_body(), usize::MAX).await.unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let repo_id = created["id"].as_str().unwrap();

        // Mallory has no grant on alice's personal repo and isn't a member of its hidden
        // organization — she must not even learn the repository exists.
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        state.create_user.execute(other_id, "mallory", "sup3r-s3cret!", false).await.unwrap();
        let mallory_token = state.authenticate_user.execute("mallory", "sup3r-s3cret!").await.unwrap();

        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("authorization", format!("Bearer {mallory_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), axum::http::StatusCode::NOT_FOUND);

        let patch_response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/repositories/{repo_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {mallory_token}"))
                    .body(Body::from(r#"{"name":"stolen-name"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(patch_response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn checking_for_an_unreserved_personal_repository_returns_not_found(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/me/repository")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn checking_for_a_reserved_personal_repository_returns_ok(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/me/repository")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn listing_my_projects_before_reserving_a_namespace_returns_an_empty_array(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/me/repository/projects")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!([]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn listing_my_projects_returns_only_my_own_personal_projects(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        for name in ["my-lib", "my-other-lib"] {
            app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/me/repository/projects")
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {alice_token}"))
                        .body(Body::from(format!(r#"{{"name":"{name}","format":"npm","repo_type":"hosted"}}"#)))
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/me/repository/projects")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let projects: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(projects.len(), 2);
        for project in &projects {
            assert_eq!(project["owner_name"], "alice");
            assert_eq!(project["owner_is_personal"], true);
            assert_eq!(project["my_role"], "admin");
        }

        // A second user with no personal projects of their own must see [], not alice's.
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        state.create_user.execute(other_id, "mallory", "sup3r-s3cret!", false).await.unwrap();
        let mallory_token = state.authenticate_user.execute("mallory", "sup3r-s3cret!").await.unwrap();

        let mallory_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/me/repository/projects")
                    .header("authorization", format!("Bearer {mallory_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(mallory_response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(mallory_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!([]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_visitor_can_resolve_a_public_personal_project_by_owner_and_name(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap())
            .await.unwrap();
        let create_response = app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"name":"my-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await.unwrap();
        let create_body = axum::body::to_bytes(create_response.into_body(), usize::MAX).await.unwrap();
        let created: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        let repo_id = created["id"].as_str().unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/repositories/{repo_id}/visibility"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"is_public":true}"#))
                    .unwrap(),
            )
            .await.unwrap();

        let response = app
            .oneshot(Request::builder().uri("/api/repositories/by-owner/alice/my-lib").body(Body::empty()).unwrap())
            .await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "my-lib");
        assert_eq!(json["owner_name"], "alice");
        assert_eq!(json["owner_is_personal"], true);
        assert_eq!(json["is_public"], true);
        assert!(json["my_role"].is_null());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn resolving_a_private_personal_project_anonymously_returns_not_found(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);
        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap())
            .await.unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"name":"my-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await.unwrap();

        let response = app
            .oneshot(Request::builder().uri("/api/repositories/by-owner/alice/my-lib").body(Body::empty()).unwrap())
            .await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_owner_can_resolve_their_own_private_project_by_owner_and_name(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);
        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap())
            .await.unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"name":"my-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repositories/by-owner/alice/my-lib")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["my_role"], "admin");
        assert_eq!(json["is_public"], false);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_different_authenticated_user_cannot_resolve_a_private_personal_project_by_owner_and_name(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());
        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap())
            .await.unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/repository/projects")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::from(r#"{"name":"my-lib","format":"npm","repo_type":"hosted"}"#))
                    .unwrap(),
            )
            .await.unwrap();
        let other_org_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        state.create_user.execute(other_org_id, "mallory", "sup3r-s3cret!", false).await.unwrap();
        let mallory_token = state.authenticate_user.execute("mallory", "sup3r-s3cret!").await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repositories/by-owner/alice/my-lib")
                    .header("authorization", format!("Bearer {mallory_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn resolving_an_unknown_username_returns_not_found(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app
            .oneshot(Request::builder().uri("/api/repositories/by-owner/nobody/my-lib").body(Body::empty()).unwrap())
            .await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn resolving_an_unknown_repo_name_returns_not_found(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);
        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap())
            .await.unwrap();

        let response = app
            .oneshot(Request::builder().uri("/api/repositories/by-owner/alice/does-not-exist").body(Body::empty()).unwrap())
            .await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    async fn organization_repository(state: &AppState, slug: &str, name: &str, is_public: bool) -> (Uuid, Uuid) {
        let organization = state.create_organization.execute(slug, "Acme Corp").await.unwrap();
        let admin = state.create_user.execute(organization, &format!("{slug}-admin"), "sup3r-s3cret!", false).await.unwrap();
        let repo = state.create_repository.execute(organization, name, RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        state.set_repository_visibility.execute(repo, is_public, admin).await.unwrap();
        (repo, admin)
    }

    async fn status_and_body(app: axum::Router, uri: &str) -> (axum::http::StatusCode, serde_json::Value) {
        let response = app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_visitor_can_resolve_a_public_organization_repository_by_slug_and_name(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        organization_repository(&state, "acme", "libs", true).await;

        let (status, json) = status_and_body(build_router(state), "/api/repositories/by-org/acme/libs").await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!((json["name"].as_str(), json["owner_name"].as_str(), json["owner_is_personal"].as_bool(), json["is_public"].as_bool()), (Some("libs"), Some("Acme Corp"), Some(false), Some(true)));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_organization_repository_is_not_found_by_slug_and_name_for_an_anonymous_visitor(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        organization_repository(&state, "acme", "internal", false).await;

        let (status, _) = status_and_body(build_router(state), "/api/repositories/by-org/acme/internal").await;

        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_organization_repository_is_visible_to_its_organization_admin_by_slug_and_name(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, admin) = organization_repository(&state, "acme", "internal", false).await;
        state.set_organization_admin.execute(admin, true, None).await.unwrap();
        let token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(Request::builder().uri("/api/repositories/by-org/acme/internal").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn unknown_organizations_and_repositories_are_not_found_by_slug_and_name(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        organization_repository(&state, "acme", "libs", true).await;
        let app = build_router(state);

        for uri in ["/api/repositories/by-org/nobody/libs", "/api/repositories/by-org/acme/nope"] {
            assert_eq!(status_and_body(app.clone(), uri).await.0, axum::http::StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_control_character_in_a_repository_name_is_not_found_rather_than_a_server_error(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        organization_repository(&state, "acme", "libs", true).await;
        let alice = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        state.reserve_personal_organization.execute(alice).await.unwrap();
        let app = build_router(state);

        for uri in ["/api/repositories/by-org/acme/%00", "/api/repositories/by-org/acme/li%00bs", "/api/repositories/by-owner/alice/%00", "/api/repositories/by-owner/alice/my%09lib"] {
            assert_eq!(status_and_body(app.clone(), uri).await.0, axum::http::StatusCode::NOT_FOUND, "{uri}");
        }
    }

    async fn limited_public_repository(state: &AppState) -> (Uuid, Uuid) {
        let (repo, admin) = organization_repository(state, "acme", "libs", true).await;
        state.set_repository_quota.execute(repo, Some(1_000_000), admin).await.unwrap();
        state.set_retention_policy.execute(repo, Some(5), admin).await.unwrap();
        (repo, admin)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_sees_neither_the_limits_nor_the_internal_organization_id_of_a_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (repo, _) = limited_public_repository(&state).await;
        let alice = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        state.reserve_personal_organization.execute(alice).await.unwrap();
        let personal = state.create_user_project.execute(alice, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap();
        state.set_repository_visibility.execute(personal, true, alice).await.unwrap();
        state.set_repository_quota.execute(personal, Some(5), alice).await.unwrap();
        let app = build_router(state);

        for uri in [format!("/api/repositories/{repo}"), "/api/repositories/by-org/acme/libs".to_string(), "/api/repositories/by-owner/alice/my-lib".to_string()] {
            let (status, json) = status_and_body(app.clone(), &uri).await;

            assert_eq!(status, axum::http::StatusCode::OK, "{uri}");
            for hidden in ["quota_bytes", "retention_keep_last_n", "organization_id"] {
                assert!(json.get(hidden).is_none(), "{uri} leaks {hidden}: {json}");
            }
            for shown in ["id", "name", "format", "repo_type", "is_public", "my_role", "owner_name", "owner_is_personal", "group_members", "remote_credentials_set", "remote_url"] {
                assert!(json.get(shown).is_some(), "{uri} lacks {shown}: {json}");
            }
            assert!(json["my_role"].is_null());
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_signed_in_caller_with_a_role_keeps_the_full_repository_shape(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (repo, admin) = limited_public_repository(&state).await;
        state.set_organization_admin.execute(admin, true, None).await.unwrap();
        let token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        for uri in [format!("/api/repositories/{repo}"), "/api/repositories/by-org/acme/libs".to_string()] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(&uri).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();

            assert_eq!((json["quota_bytes"].as_i64(), json["retention_keep_last_n"].as_i64(), json["my_role"].as_str()), (Some(1_000_000), Some(5), Some("admin")), "{uri}");
            assert!(json["organization_id"].is_string(), "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn public_path_points_at_the_organization_or_personal_page_and_is_absent_while_private(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (org_repo, _) = organization_repository(&state, "acme", "libs", true).await;
        let alice = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        state.reserve_personal_organization.execute(alice).await.unwrap();
        let personal_repo = state.create_user_project.execute(alice, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap();
        state.set_repository_visibility.execute(personal_repo, true, alice).await.unwrap();
        let app = build_router(state);

        let org_json = status_and_body(app.clone(), &format!("/api/repositories/{org_repo}")).await.1;
        let personal_json = status_and_body(app.clone(), &format!("/api/repositories/{personal_repo}")).await.1;

        assert_eq!(org_json["public_path"], "/o/acme/libs");
        assert_eq!(personal_json["public_path"], "/@alice/my-lib");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn public_path_is_absent_while_the_repository_is_private(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (repo, admin) = organization_repository(&state, "acme", "libs", false).await;
        state.set_organization_admin.execute(admin, true, None).await.unwrap();
        let token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let json = json_as(&app, &format!("/api/repositories/{repo}"), &token).await;

        assert!(json["public_path"].is_null(), "{json}");
    }

    async fn json_as(app: &axum::Router, uri: &str, token: &str) -> serde_json::Value {
        let response = app.clone().oneshot(Request::builder().uri(uri).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap()).await.unwrap();
        serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_signed_in_caller_who_only_has_the_implicit_public_read_gets_the_public_shape(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (repo, _) = limited_public_repository(&state).await;
        let acme = state.organizations.find_by_slug(&artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap()).await.unwrap().unwrap().id;
        let outsider = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "outsider", "sup3r-s3cret!", false).await;
        let colleague = bearer(&state, acme, "colleague", "sup3r-s3cret!", false).await;
        let app = build_router(state);

        for token in [&outsider, &colleague] {
            for uri in [format!("/api/repositories/{repo}"), "/api/repositories/by-org/acme/libs".to_string()] {
                let json = json_as(&app, &uri, token).await;

                for hidden in ["quota_bytes", "retention_keep_last_n", "organization_id"] {
                    assert!(json.get(hidden).is_none(), "{uri} leaks {hidden}: {json}");
                }
                assert_eq!(json["my_role"], "read", "{uri}: the role they really have is still reported");
            }
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_caller_with_an_explicit_grant_gets_the_full_shape_even_when_it_is_only_read(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (repo, admin) = limited_public_repository(&state).await;
        let acme = state.organizations.find_by_slug(&artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap()).await.unwrap().unwrap().id;
        let member = state.create_user.execute(acme, "member", "sup3r-s3cret!", false).await.unwrap();
        state.grant_permission.execute(member, repo, Role::Read, admin).await.unwrap();
        let token = state.authenticate_user.execute("member", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let json = json_as(&app, &format!("/api/repositories/{repo}"), &token).await;

        assert_eq!((json["quota_bytes"].as_i64(), json["my_role"].as_str()), (Some(1_000_000), Some("read")));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn anonymous_repository_reads_run_out_of_budget_but_signed_in_callers_do_not(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (repo, _) = limited_public_repository(&state).await;
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "reader", "sup3r-s3cret!", false).await;
        let app = build_router(state);
        let uris = [format!("/api/repositories/{repo}"), "/api/repositories/by-org/acme/libs".to_string(), format!("/api/repositories/{repo}/packages")];

        for i in 0..ANONYMOUS_REPOSITORY_READS_PER_MINUTE {
            let (status, _) = status_and_body(app.clone(), &uris[i % 3]).await;
            assert_eq!(status, axum::http::StatusCode::OK, "request {i}");
        }

        for uri in &uris {
            assert_eq!(status_and_body(app.clone(), uri).await.0, axum::http::StatusCode::TOO_MANY_REQUESTS, "{uri}");
        }
        let signed_in = app.clone().oneshot(Request::builder().uri(&uris[0]).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(signed_in.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_package_list_comes_in_pages_of_at_most_two_hundred(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName};

        let state = AppState::build(pool, &test_config());
        let (repo, _) = organization_repository(&state, "acme", "libs", true).await;
        for name in ["c", "a", "b"] {
            let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repo, name: NpmPackageName::parse(name).unwrap(), created_at: chrono::Utc::now(), updated_at: chrono::Utc::now(), metadata_fetched_at: None, cached_metadata: None };
            state.npm_packages.create_package(&package).await.unwrap();
        }
        let app = build_router(state);
        fn names(json: &serde_json::Value) -> Vec<&str> {
            json["packages"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect()
        }

        let (_, everything) = status_and_body(app.clone(), &format!("/api/repositories/{repo}/packages")).await;
        let (_, first) = status_and_body(app.clone(), &format!("/api/repositories/{repo}/packages?limit=2")).await;
        let (_, second) = status_and_body(app.clone(), &format!("/api/repositories/{repo}/packages?limit=2&after=b")).await;
        let (status, silly) = status_and_body(app.clone(), &format!("/api/repositories/{repo}/packages?limit=100000")).await;

        assert_eq!((names(&everything), everything["next_after"].is_null()), (vec!["a", "b", "c"], true), "no parameters: the same array as before");
        assert_eq!((names(&first), first["next_after"].as_str()), (vec!["a", "b"], Some("b")));
        assert_eq!((names(&second), second["next_after"].is_null()), (vec!["c"], true));
        assert_eq!((status, names(&silly).len()), (axum::http::StatusCode::OK, 3));
        assert_eq!(status_and_body(app, &format!("/api/repositories/{repo}/packages?limit=abc")).await.0, axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_personal_project_is_never_resolved_through_its_personal_organization_slug(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let alice = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let personal_org = state.reserve_personal_organization.execute(alice).await.unwrap();
        let project = state.create_user_project.execute(alice, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap();
        state.set_repository_visibility.execute(project, true, alice).await.unwrap();
        let slug = state.organizations.find_by_id(personal_org).await.unwrap().unwrap().slug;

        let (status, _) = status_and_body(build_router(state), &format!("/api/repositories/by-org/{}/my-lib", slug.as_str())).await;

        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_repository_named_after_a_catalog_is_refused_with_a_clear_message(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let token = bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "root", "sup3r-s3cret!", true).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/repositories")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(r#"{"name":"artiferris-npm","format":"npm","repo_type":"hosted","remote_url":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("reserved"), "got: {json}");
    }
}
