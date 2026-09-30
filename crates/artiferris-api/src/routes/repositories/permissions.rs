use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use artiferris_domain::permission::Role;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth_middleware::AuthUser;
use crate::authz::require_management_access;
use crate::dto::{application_error_response, ErrorResponse};
use crate::state::AppState;

use super::{load_repository, repository_access_error, require_repository_access};

#[derive(Serialize)]
pub(super) struct PermissionEntryResponse {
    user_id: Uuid,
    username: String,
    role: Role,
}

pub(super) async fn list_permissions(State(state): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<Json<Vec<PermissionEntryResponse>>, StatusCode> {
    let repo = state.repositories.find_by_id(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::NOT_FOUND)?;
    require_management_access(&state, &user, repo.organization_id, id, Role::Read, "view permissions").await?;
    let entries = state.permissions.list_for_repository(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let user_ids: Vec<Uuid> = entries.iter().map(|(user_id, _)| *user_id).collect();
    let users = state.users.find_by_ids(&user_ids).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let username_by_id: std::collections::HashMap<Uuid, String> = users.into_iter().map(|u| (u.id, u.username.as_str().to_string())).collect();
    let result = entries
        .into_iter()
        .map(|(user_id, role)| {
            let username = username_by_id.get(&user_id).cloned().unwrap_or_else(|| "unknown".to_string());
            PermissionEntryResponse { user_id, username, role }
        })
        .collect();
    Ok(Json(result))
}

#[derive(Deserialize)]
pub(super) struct GrantPermissionRequest {
    role: Role,
}

pub(super) async fn grant_permission(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, target_user_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<GrantPermissionRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "grant permission").await.map_err(repository_access_error)?;
    state.grant_permission.execute(target_user_id, id, body.role, user.id).await.map_err(|e| application_error_response("failed to grant permission", e))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn revoke_permission(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, target_user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Admin, "revoke permission").await.map_err(repository_access_error)?;
    state.revoke_permission.execute(target_user_id, id, user.id).await.map_err(|e| application_error_response("failed to revoke permission", e))?;
    Ok(StatusCode::NO_CONTENT)
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_support::test_config;
    use crate::{build_router, state::AppState};
    use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;


    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn listing_permissions_includes_a_granted_user(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "shared-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let member_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "member", "sup3r-s3cret!", false).await.unwrap();
        state.grant_permission.execute(member_id, repo_id, Role::Write, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/permissions"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entries = json.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["username"], "member");
        assert_eq!(entries[0]["role"], "write");
    }

    /// The public-organization bypass only unlocks reading package content: a member of an unrelated organization must
    /// not enumerate who has access to a repository because it is `is_public`.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_public_organization_member_cannot_list_permissions_on_a_public_repository_in_another_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "shared-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.grant_permission.execute(admin_id, repo_id, Role::Admin, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "public-member", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("public-member", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/permissions"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND, "must not confirm the repository exists to a caller with no real access");
    }

    /// The same-organization variant: even a member of the repository's own organization must not see the permission
    /// roster because the repository is public. It has always required a real grant or organization admin, and a public
    /// repository of the default public organization must not reopen it for every self-registered member.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_same_organization_member_without_a_grant_cannot_list_permissions_on_a_public_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "public-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "ungranted-member", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("ungranted-member", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/permissions"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    /// `require_management_access`'s personal-organization branch, which the two denials above do not exercise: a
    /// personal project's owner holds their own explicit `Admin` grant from creation and must still list it.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_owner_can_list_permissions_on_their_own_personal_project(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let alice_token = state.authenticate_user.execute("alice", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        app.clone()
            .oneshot(Request::builder().method("POST").uri("/api/me/repository").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap())
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
        let create_body = axum::body::to_bytes(create_response.into_body(), usize::MAX).await.unwrap();
        let created: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        let repo_id = created["id"].as_str().unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/permissions"))
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entries = json.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["username"], "alice");
        assert_eq!(entries[0]["role"], "admin");
    }
}
