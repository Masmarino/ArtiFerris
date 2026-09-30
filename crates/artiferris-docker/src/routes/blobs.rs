use axum::body::{Body, HttpBody};
use axum::http::{HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};
use artiferris_application::body_read::with_idle_timeout;
use artiferris_domain::docker_registry::{ByteStream, Digest, DockerImageName, MAX_BLOB_BYTES};
use artiferris_domain::error::DomainError;
use artiferris_domain::package_repository::PackageRepositorySummary;
use futures_util::StreamExt;
use uuid::Uuid;

use crate::auth::DockerAuthUser;
use crate::authz::{member_is_readable, require_docker_repository, require_granted_action_for_route, require_hosted, require_readable_repository_by_name, require_repository_by_name};
use crate::errors::{docker_authz_error, docker_error, docker_error_response};
use crate::state::DockerState;

/// Parses the `start` of a `Content-Range: <start>-<end>` header. `Ok(None)`: no header (offset validation skipped).
/// `Ok(Some(n))`: a well-formed start `n`. `Err(())`: a header was sent but cannot be parsed, which must be rejected,
/// not treated as absent.
fn parse_content_range_start(header: Option<&str>) -> Result<Option<i64>, ()> {
    let Some(header) = header else { return Ok(None) };
    header.split('-').next().and_then(|s| s.trim().parse().ok()).map(Some).ok_or(())
}

/// Only ever built after the caller passed every authorization check.
pub(crate) fn body_stream(state: &DockerState, body: Body) -> ByteStream {
    let chunks = body.into_data_stream().map(|chunk| chunk.map_err(|_| DomainError::Validation("the upload body could not be read".to_string())));
    Box::pin(with_idle_timeout(chunks, state.guard.body_timeouts.idle, || DomainError::RequestTimeout))
}

/// Content fetched by digest never changes, but a shared cache may keep it only for a public repository.
pub(crate) fn immutable_content_cache_headers(repository_is_public: bool) -> [(header::HeaderName, &'static str); 2] {
    let cache_control = if repository_is_public { "public, max-age=31536000, immutable" } else { "private, no-store" };
    [(header::CACHE_CONTROL, cache_control), (header::VARY, "Authorization")]
}

/// `location_base` is `/v2/{repo}` or `/v2/u/{user}/{repo}`, the route the client came in through.
#[allow(clippy::too_many_arguments)]
pub async fn start_or_monolithic_upload(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name: String,
    digest_query: Option<String>,
    user: DockerAuthUser,
    body: Body,
    location_base: String,
) -> Response {
    let (repo, is_personal) = match require_repository_by_name(&state, &user, organization_id, &repository_name).await {
        Ok(found) => found,
        Err(status) => return docker_authz_error(status),
    };
    if let Err(status) = require_docker_repository(&repo) {
        return docker_authz_error(status);
    }
    if let Err(status) = require_granted_action_for_route(&user, repo.id, &repository_name, "push", is_personal) {
        return docker_authz_error(status);
    }
    if let Err(status) = require_hosted(&repo) {
        return docker_authz_error(status);
    }
    if DockerImageName::parse(&image_name).is_err() {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    }

    if let Some(digest_str) = digest_query {
        let Ok(digest) = Digest::parse(&digest_str) else {
            return docker_error(StatusCode::BAD_REQUEST, "DIGEST_INVALID", "invalid digest").into_response();
        };
        return match state.monolithic_upload.execute(repo.id, &digest, body_stream(&state, body), MAX_BLOB_BYTES).await {
            Ok(()) => (StatusCode::CREATED, [(header::LOCATION, format!("{location_base}/{image_name}/blobs/{}", digest.as_str()))]).into_response(),
            Err(e) => docker_error_response(e).into_response(),
        };
    }

    match state.start_upload.execute(repo.id).await {
        Ok(session) => (
            StatusCode::ACCEPTED,
            [
                (header::LOCATION, format!("{location_base}/{image_name}/blobs/uploads/{}", session.id)),
                (HeaderName::from_static("range"), "0-0".to_string()),
                (HeaderName::from_static("docker-upload-uuid"), session.id.to_string()),
            ],
        )
            .into_response(),
        Err(e) => docker_error_response(e).into_response(),
    }
}

/// `DELETE` on an upload: the client is giving up, so its staged bytes and its slot are freed right away.
pub async fn cancel_upload(state: DockerState, organization_id: Uuid, repository_name: String, image_name: String, upload_id: String, user: DockerAuthUser) -> Response {
    let (repo, is_personal) = match require_repository_by_name(&state, &user, organization_id, &repository_name).await {
        Ok(found) => found,
        Err(status) => return docker_authz_error(status),
    };
    if let Err(status) = require_docker_repository(&repo) {
        return docker_authz_error(status);
    }
    if let Err(status) = require_granted_action_for_route(&user, repo.id, &repository_name, "push", is_personal) {
        return docker_authz_error(status);
    }
    if let Err(status) = require_hosted(&repo) {
        return docker_authz_error(status);
    }
    if DockerImageName::parse(&image_name).is_err() {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    }
    let Ok(session_id) = Uuid::parse_str(&upload_id) else {
        return docker_error(StatusCode::BAD_REQUEST, "BLOB_UPLOAD_INVALID", "invalid upload id").into_response();
    };
    match state.patch_upload.cancel(session_id, repo.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => docker_error_response(e).into_response(),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn patch_chunk(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name: String,
    upload_id: String,
    content_range: Option<String>,
    user: DockerAuthUser,
    chunk: Body,
    location_base: String,
) -> Response {
    let (repo, is_personal) = match require_repository_by_name(&state, &user, organization_id, &repository_name).await {
        Ok(found) => found,
        Err(status) => return docker_authz_error(status),
    };
    if let Err(status) = require_docker_repository(&repo) {
        return docker_authz_error(status);
    }
    if let Err(status) = require_granted_action_for_route(&user, repo.id, &repository_name, "push", is_personal) {
        return docker_authz_error(status);
    }
    if let Err(status) = require_hosted(&repo) {
        return docker_authz_error(status);
    }
    if DockerImageName::parse(&image_name).is_err() {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    }
    let Ok(session_id) = Uuid::parse_str(&upload_id) else {
        return docker_error(StatusCode::BAD_REQUEST, "BLOB_UPLOAD_INVALID", "invalid upload id").into_response();
    };
    let expected_start = match parse_content_range_start(content_range.as_deref()) {
        Ok(start) => start,
        Err(()) => return docker_error(StatusCode::BAD_REQUEST, "BLOB_UPLOAD_INVALID", "malformed Content-Range header").into_response(),
    };

    match state.patch_upload.execute_stream(session_id, repo.id, body_stream(&state, chunk), expected_start, MAX_BLOB_BYTES).await {
        Ok(total_bytes) => {
            let range_end = if total_bytes > 0 { total_bytes - 1 } else { 0 };
            (
                StatusCode::ACCEPTED,
                [
                    (header::LOCATION, format!("{location_base}/{image_name}/blobs/uploads/{upload_id}")),
                    (HeaderName::from_static("range"), format!("0-{range_end}")),
                ],
            )
                .into_response()
        }
        Err(e) => docker_error_response(e).into_response(),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn complete_upload(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name: String,
    upload_id: String,
    digest_query: Option<String>,
    user: DockerAuthUser,
    final_chunk: Body,
    location_base: String,
) -> Response {
    let (repo, is_personal) = match require_repository_by_name(&state, &user, organization_id, &repository_name).await {
        Ok(found) => found,
        Err(status) => return docker_authz_error(status),
    };
    if let Err(status) = require_docker_repository(&repo) {
        return docker_authz_error(status);
    }
    if let Err(status) = require_granted_action_for_route(&user, repo.id, &repository_name, "push", is_personal) {
        return docker_authz_error(status);
    }
    if let Err(status) = require_hosted(&repo) {
        return docker_authz_error(status);
    }
    if DockerImageName::parse(&image_name).is_err() {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    }
    let Some(digest_str) = digest_query else {
        return docker_error(StatusCode::BAD_REQUEST, "DIGEST_INVALID", "digest query parameter is required").into_response();
    };
    let Ok(digest) = Digest::parse(&digest_str) else {
        return docker_error(StatusCode::BAD_REQUEST, "DIGEST_INVALID", "invalid digest").into_response();
    };
    let Ok(session_id) = Uuid::parse_str(&upload_id) else {
        return docker_error(StatusCode::BAD_REQUEST, "BLOB_UPLOAD_INVALID", "invalid upload id").into_response();
    };

    // Some clients send the final bytes directly here rather than a preceding PATCH.
    if !final_chunk.is_end_stream() {
        if let Err(e) = state.patch_upload.execute_stream(session_id, repo.id, body_stream(&state, final_chunk), None, MAX_BLOB_BYTES).await {
            return docker_error_response(e).into_response();
        }
    }

    match state.complete_upload.execute(session_id, repo.id, &digest).await {
        Ok(()) => (StatusCode::CREATED, [(header::LOCATION, format!("{location_base}/{image_name}/blobs/{}", digest.as_str()))]).into_response(),
        Err(e) => docker_error_response(e).into_response(),
    }
}

pub async fn get_blob(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name_str: String,
    digest_str: String,
    user: Option<DockerAuthUser>,
) -> Response {
    let (repo, caller) = match require_readable_repository_by_name(&state, user.as_ref(), organization_id, &repository_name).await {
        Ok(found) => found,
        Err(status) => return docker_authz_error(status),
    };
    if let Err(status) = require_docker_repository(&repo) {
        return docker_authz_error(status);
    }
    if let Some((caller, is_personal)) = caller {
        if let Err(status) = require_granted_action_for_route(caller, repo.id, &repository_name, "pull", is_personal) {
            return docker_authz_error(status);
        }
    }
    let Ok(image_name) = DockerImageName::parse(&image_name_str) else {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    };
    let Ok(digest) = Digest::parse(&digest_str) else {
        return docker_error(StatusCode::BAD_REQUEST, "DIGEST_INVALID", "invalid digest").into_response();
    };

    // `user`, not `caller`: see `member_is_readable` for the public-group case.
    let caller_user = user.as_ref();
    let top_level_organization_id = repo.organization_id;
    let top_level_was_authorized = caller.is_some();
    let state_ref = &state;
    match state
        .get_blob
        .execute_stream(repo.id, &image_name, &digest, move |member: &PackageRepositorySummary| {
            member_is_readable(state_ref, caller_user, top_level_organization_id, top_level_was_authorized, member.id, member.organization_id, member.is_public)
        })
        .await
    {
        Ok(Some(stream)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (HeaderName::from_static("docker-content-digest"), digest.as_str().to_string()),
                (header::ETAG, format!("\"{}\"", digest.as_str())),
            ],
            immutable_content_cache_headers(repo.is_public),
            Body::from_stream(stream),
        )
            .into_response(),
        Ok(None) => docker_error(StatusCode::NOT_FOUND, "BLOB_UNKNOWN", "blob not found").into_response(),
        Err(e) => docker_error_response(e).into_response(),
    }
}

/// Dedicated `HEAD` handler using `execute_exists`, so a push's existence check doesn't read an already-present blob off disk just to discard it. `Content-Length` must be set — real `docker push` errors on a HEAD without it.
pub async fn head_blob(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name_str: String,
    digest_str: String,
    user: Option<DockerAuthUser>,
) -> Response {
    let (repo, caller) = match require_readable_repository_by_name(&state, user.as_ref(), organization_id, &repository_name).await {
        Ok(found) => found,
        Err(status) => return docker_authz_error(status),
    };
    if let Err(status) = require_docker_repository(&repo) {
        return docker_authz_error(status);
    }
    if let Some((caller, is_personal)) = caller {
        if let Err(status) = require_granted_action_for_route(caller, repo.id, &repository_name, "pull", is_personal) {
            return docker_authz_error(status);
        }
    }
    let Ok(image_name) = DockerImageName::parse(&image_name_str) else {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    };
    let Ok(digest) = Digest::parse(&digest_str) else {
        return docker_error(StatusCode::BAD_REQUEST, "DIGEST_INVALID", "invalid digest").into_response();
    };

    // `user`, not `caller`: see `member_is_readable` for the public-group case.
    let caller_user = user.as_ref();
    let top_level_organization_id = repo.organization_id;
    let top_level_was_authorized = caller.is_some();
    let state_ref = &state;
    match state
        .get_blob
        .execute_exists(repo.id, &image_name, &digest, move |member: &PackageRepositorySummary| {
            member_is_readable(state_ref, caller_user, top_level_organization_id, top_level_was_authorized, member.id, member.organization_id, member.is_public)
        })
        .await
    {
        Ok(Some(size_bytes)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (header::CONTENT_LENGTH, size_bytes.to_string()),
                (HeaderName::from_static("docker-content-digest"), digest.as_str().to_string()),
            ],
        )
            .into_response(),
        Ok(None) => docker_error(StatusCode::NOT_FOUND, "BLOB_UNKNOWN", "blob not found").into_response(),
        Err(e) => docker_error_response(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::route_test_support::{issue_test_token, probe_body, seed_bare_user, seed_named_user_with_active_token, seed_repository, set_quota, test_state};
    use std::sync::atomic::Ordering;

    const REPO_NAME_PREFIX: &str = "repo-";

    async fn hosted_repo(pool: &sqlx::PgPool) -> (Uuid, String) {
        let repository_id = Uuid::new_v4();
        seed_repository(pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        (repository_id, format!("{REPO_NAME_PREFIX}{repository_id}"))
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn starting_an_upload_without_push_scope_is_forbidden(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_monolithic_upload_then_download_round_trips_the_same_bytes(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let bytes = b"layer-bytes".to_vec();
        let digest = artiferris_domain::docker_registry::Digest::of(&bytes);

        let push_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(bytes.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(push_response.status(), StatusCode::CREATED);

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/blobs/{}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.to_vec(), bytes);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn getting_a_blob_from_a_public_repository_carries_long_lived_cache_headers_since_a_digest_never_changes(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        crate::route_test_support::mark_public(&pool, repository_id).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let bytes = b"layer-bytes".to_vec();
        let digest = artiferris_domain::docker_registry::Digest::of(&bytes);

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(bytes.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/blobs/{}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.headers().get(axum::http::header::CACHE_CONTROL).unwrap(), "public, max-age=31536000, immutable");
        assert_eq!(get_response.headers().get(axum::http::header::ETAG).unwrap(), &format!("\"{}\"", digest.as_str()));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_monolithic_upload_with_a_wrong_digest_is_rejected(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest=sha256:{}", "0".repeat(64)))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(b"layer-bytes".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_chunked_upload_then_download_round_trips_the_concatenated_bytes(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let full_bytes = b"hello-world".to_vec();
        let digest = artiferris_domain::docker_registry::Digest::of(&full_bytes);

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(start_response.status(), StatusCode::ACCEPTED);
        let location = start_response.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string();
        // This harness mounts the router unnested, so strip the /v2 prefix.
        let upload_path = location.trim_start_matches(&format!("/v2/{repo_name}/")).to_string();

        let patch_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/{repo_name}/{upload_path}"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(b"hello-".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(patch_response.status(), StatusCode::ACCEPTED);

        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/{upload_path}?digest={}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(b"world".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/blobs/{}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.to_vec(), full_bytes);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_session_started_for_one_repository_cannot_be_completed_through_a_different_repositorys_authorization(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (victim_repository_id, victim_repo_name) = hosted_repo(&pool).await;
        let (attacker_repository_id, attacker_repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let victim_push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, victim_repository_id, &victim_repo_name, "myimage", &["push"]);
        let attacker_push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, attacker_repository_id, &attacker_repo_name, "otherimage", &["push"]);
        let app = crate::router(state);
        let bytes = b"stolen-bytes".to_vec();
        let digest = artiferris_domain::docker_registry::Digest::of(&bytes);

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{victim_repo_name}/myimage/blobs/uploads/"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {victim_push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(start_response.status(), StatusCode::ACCEPTED);
        let location = start_response.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string();
        let upload_path = location.trim_start_matches(&format!("/v2/{victim_repo_name}/")).to_string();
        let upload_id = upload_path.rsplit('/').next().unwrap().to_string();

        let patch_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/{attacker_repo_name}/otherimage/blobs/uploads/{upload_id}"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {attacker_push_token}"))
                    .body(Body::from(bytes.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(patch_response.status(), StatusCode::ACCEPTED, "an upload session belonging to a different repository must not accept chunks via this route");

        let put_response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{attacker_repo_name}/otherimage/blobs/uploads/{upload_id}?digest={}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {attacker_push_token}"))
                    .body(Body::from(bytes.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(put_response.status(), StatusCode::CREATED, "completing a foreign upload session through the wrong repository's authorization must not succeed");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_chunk_whose_content_range_matches_the_current_offset_is_accepted(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let location = start_response.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string();
        let upload_path = location.trim_start_matches(&format!("/v2/{repo_name}/")).to_string();

        let first_patch = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/{repo_name}/{upload_path}"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_RANGE, "0-5")
                    .body(Body::from(b"hello-".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first_patch.status(), StatusCode::ACCEPTED);

        // A retried chunk claiming to restart at 0 bytes when 6 are already staged.
        let retried_from_zero = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/{repo_name}/{upload_path}"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_RANGE, "0-4")
                    .body(Body::from(b"world".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retried_from_zero.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        let second_patch = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/{repo_name}/{upload_path}"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_RANGE, "6-10")
                    .body(Body::from(b"world".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second_patch.status(), StatusCode::ACCEPTED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_malformed_content_range_is_rejected_not_silently_ignored(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let location = start_response.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string();
        let upload_path = location.trim_start_matches(&format!("/v2/{repo_name}/")).to_string();

        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/{repo_name}/{upload_path}"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_RANGE, "garbage")
                    .body(Body::from(b"hello".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::ACCEPTED, "a malformed Content-Range must not silently skip offset validation");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "a malformed Content-Range must be rejected with a clear 400, not treated as absent");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn downloading_a_missing_blob_is_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/blobs/sha256:{}", "0".repeat(64)))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn head_request_for_an_existing_blob_reports_its_real_content_length(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let bytes = b"layer-bytes-for-head-check".to_vec();
        let digest = artiferris_domain::docker_registry::Digest::of(&bytes);

        let push_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(bytes.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(push_response.status(), StatusCode::CREATED);

        let head_response = app
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri(format!("/{repo_name}/myimage/blobs/{}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(head_response.status(), StatusCode::OK);
        let content_length = head_response.headers().get(axum::http::header::CONTENT_LENGTH).unwrap().to_str().unwrap().to_string();
        assert_eq!(content_length, bytes.len().to_string());
        let content_type = head_response.headers().get(axum::http::header::CONTENT_TYPE).unwrap().to_str().unwrap().to_string();
        assert_eq!(content_type, "application/octet-stream");
        let body = axum::body::to_bytes(head_response.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty(), "a HEAD response must carry no body");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn head_request_for_a_missing_blob_is_not_found_with_no_body(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri(format!("/{repo_name}/myimage/blobs/sha256:{}", "0".repeat(64)))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn blob_routes_404_on_a_non_docker_format_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "npm", "hosted").await;
        let repo_name = format!("{REPO_NAME_PREFIX}{repository_id}");
        let state = test_state(pool.clone(), dir.path()).await;
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/blobs/sha256:{}", "0".repeat(64)))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    async fn send(app: &axum::Router, method: &str, uri: &str, token: &str, body: Body) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(Request::builder().method(method).uri(uri).header(axum::http::header::AUTHORIZATION, format!("Bearer {token}")).body(body).unwrap())
            .await
            .unwrap()
    }

    fn location_of(response: &axum::http::Response<Body>) -> String {
        response.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_repositorys_blob_is_never_marked_cacheable_by_a_shared_cache(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push", "pull"]);
        let app = crate::router(state);
        let digest = artiferris_domain::docker_registry::Digest::of(b"private-layer");
        send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/?digest={}", digest.as_str()), &token, Body::from(b"private-layer".to_vec())).await;

        let response = send(&app, "GET", &format!("/{repo_name}/myimage/blobs/{}", digest.as_str()), &token, Body::empty()).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(axum::http::header::CACHE_CONTROL).unwrap(), "private, no-store");
        assert_eq!(response.headers().get(axum::http::header::VARY).unwrap(), "Authorization");
    }

    /// The payload of a caller who may not push is never read.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_upload_body_from_a_caller_without_push_scope_is_never_read(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let pull_only = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let digest = artiferris_domain::docker_registry::Digest::of(b"x");
        let upload_id = Uuid::new_v4();

        for (method, uri) in [
            ("POST", format!("/{repo_name}/myimage/blobs/uploads/")),
            ("POST", format!("/{repo_name}/myimage/blobs/uploads/?digest={}", digest.as_str())),
            ("PATCH", format!("/{repo_name}/myimage/blobs/uploads/{upload_id}")),
            ("PUT", format!("/{repo_name}/myimage/blobs/uploads/{upload_id}?digest={}", digest.as_str())),
            ("PUT", format!("/{repo_name}/myimage/manifests/latest")),
        ] {
            let (body, polled) = probe_body();
            let response = send(&app, method, &uri, &pull_only, body).await;

            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
            assert!(!polled.load(Ordering::SeqCst), "{method} {uri} read the body of a caller that was refused");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_upload_body_for_an_unknown_repository_or_without_a_token_is_never_read(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let (body, polled) = probe_body();
        let response = send(&app, "PATCH", &format!("/no-such-repo/myimage/blobs/uploads/{}", Uuid::new_v4()), &token, body).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(!polled.load(Ordering::SeqCst));

        let (body, polled) = probe_body();
        let response = app.clone().oneshot(Request::builder().method("POST").uri(format!("/{repo_name}/myimage/blobs/uploads/")).body(body).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!polled.load(Ordering::SeqCst));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_blob_upload_over_the_quota_is_rejected_and_leaves_nothing_behind(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        set_quota(&pool, repository_id, 20).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let big = vec![7u8; 40];
        let digest = artiferris_domain::docker_registry::Digest::of(&big);

        let monolithic = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/?digest={}", digest.as_str()), &token, Body::from(big.clone())).await;
        assert_eq!(monolithic.status(), StatusCode::INSUFFICIENT_STORAGE);

        let start = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/"), &token, Body::empty()).await;
        let path = location_of(&start).trim_start_matches("/v2").to_string();
        let chunk = send(&app, "PATCH", &path, &token, Body::from(big)).await;
        assert_eq!(chunk.status(), StatusCode::INSUFFICIENT_STORAGE);

        let stored: i64 = sqlx::query_scalar!("SELECT count(*) FROM docker_repository_blobs WHERE package_repository_id = $1", repository_id).fetch_one(&pool).await.unwrap().unwrap();
        assert_eq!(stored, 0);
    }

    /// Blobs uploaded but never referenced by a manifest still fill the disk.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn uploaded_blobs_no_manifest_references_count_against_the_quota(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        set_quota(&pool, repository_id, 50).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let first = vec![1u8; 30];
        let second = vec![2u8; 30];

        let response = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/?digest={}", artiferris_domain::docker_registry::Digest::of(&first).as_str()), &token, Body::from(first)).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let response = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/?digest={}", artiferris_domain::docker_registry::Digest::of(&second).as_str()), &token, Body::from(second)).await;

        assert_eq!(response.status(), StatusCode::INSUFFICIENT_STORAGE);
    }

    /// Parallel sessions must not each get the whole quota.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn open_upload_sessions_share_the_quota(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        set_quota(&pool, repository_id, 50).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let mut paths = Vec::new();
        for _ in 0..2 {
            let start = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/"), &token, Body::empty()).await;
            paths.push(location_of(&start).trim_start_matches("/v2").to_string());
        }

        let first = send(&app, "PATCH", &paths[0], &token, Body::from(vec![0u8; 30])).await;
        let second = send(&app, "PATCH", &paths[1], &token, Body::from(vec![0u8; 30])).await;

        assert_eq!(first.status(), StatusCode::ACCEPTED);
        assert_eq!(second.status(), StatusCode::INSUFFICIENT_STORAGE);
    }

    /// Both requests see the whole quota when they start; only one of them may keep its bytes.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn chunks_streaming_at_the_same_time_cannot_overshoot_the_quota(pool: sqlx::PgPool) {
        use futures_util::StreamExt;

        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        set_quota(&pool, repository_id, 50).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let mut paths = Vec::new();
        for _ in 0..2 {
            let start = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/"), &token, Body::empty()).await;
            paths.push(location_of(&start).trim_start_matches("/v2").to_string());
        }
        // Neither body ends before both have delivered their bytes, so both requests are streaming at once.
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let racing_body = || {
            let barrier = barrier.clone();
            let wait = futures_util::stream::once(async move {
                barrier.wait().await;
            })
            .filter_map(|()| async { None::<Result<axum::body::Bytes, std::io::Error>> });
            Body::from_stream(futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![0u8; 30]))]).chain(wait))
        };

        let (first, second) = tokio::join!(send(&app, "PATCH", &paths[0], &token, racing_body()), send(&app, "PATCH", &paths[1], &token, racing_body()));

        let mut statuses = [first.status(), second.status()];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::ACCEPTED, StatusCode::INSUFFICIENT_STORAGE]);
        let staged: i64 = sqlx::query_scalar!("SELECT COALESCE(SUM(bytes_received), 0)::BIGINT FROM docker_blob_uploads WHERE package_repository_id = $1", repository_id).fetch_one(&pool).await.unwrap().unwrap();
        assert_eq!(staged, 30, "the refused chunk was taken back");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn cancelling_an_upload_frees_its_slot_and_its_staged_bytes(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let start = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/"), &token, Body::empty()).await;
        let path = location_of(&start).trim_start_matches("/v2").to_string();
        send(&app, "PATCH", &path, &token, Body::from(vec![1u8; 10])).await;

        let cancelled = send(&app, "DELETE", &path, &token, Body::empty()).await;
        assert_eq!(cancelled.status(), StatusCode::NO_CONTENT);

        let again = send(&app, "DELETE", &path, &token, Body::empty()).await;
        assert_eq!(again.status(), StatusCode::NOT_FOUND);
        let open: i64 = sqlx::query_scalar!("SELECT count(*) FROM docker_blob_uploads WHERE package_repository_id = $1", repository_id).fetch_one(&pool).await.unwrap().unwrap();
        assert_eq!(open, 0);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_name_with_a_control_character_is_not_found_rather_than_a_server_error(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push", "pull"]);
        let app = crate::router(state);

        for name in ["repo%00x", "repo%0Ax", "repo%7Fx"] {
            let start = send(&app, "POST", &format!("/{name}/myimage/blobs/uploads/"), &token, Body::empty()).await;
            assert_eq!(start.status(), StatusCode::NOT_FOUND, "{name}");
            let pull = send(&app, "GET", &format!("/{name}/myimage/blobs/sha256:{}", "a".repeat(64)), &token, Body::empty()).await;
            assert_eq!(pull.status(), StatusCode::NOT_FOUND, "{name}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_chunk_that_stalls_gets_408_and_leaves_the_session_usable(pool: sqlx::PgPool) {
        use futures_util::StreamExt;

        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let mut state = test_state(pool.clone(), dir.path()).await;
        state.guard = std::sync::Arc::new(artiferris_application::request_guard::RequestGuard {
            body_timeouts: artiferris_application::body_read::BodyTimeouts { idle: std::time::Duration::from_millis(100), total: std::time::Duration::from_secs(5) },
            ..Default::default()
        });
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let start = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/"), &token, Body::empty()).await;
        let path = location_of(&start).trim_start_matches("/v2").to_string();
        let stalled = futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"partial"))]).chain(futures_util::stream::pending());

        let response = send(&app, "PATCH", &path, &token, Body::from_stream(stalled)).await;
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);

        let retry = send(&app, "PATCH", &path, &token, Body::from(vec![1u8; 10])).await;
        assert_eq!(retry.status(), StatusCode::ACCEPTED);
        assert_eq!(retry.headers().get("range").unwrap(), "0-9", "the stalled chunk left no bytes behind");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_refuses_more_open_uploads_than_the_cap(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        for _ in 0..artiferris_domain::docker_registry::MAX_OPEN_UPLOADS_PER_REPOSITORY {
            let start = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/"), &token, Body::empty()).await;
            assert_eq!(start.status(), StatusCode::ACCEPTED);
        }
        let refused = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/"), &token, Body::empty()).await;

        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    /// Whatever order a chunk and the completion land in, the blob stored under a digest hashes to that digest.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_chunk_arriving_during_completion_can_never_change_what_gets_stored(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push", "pull"]);
        let app = crate::router(state);
        let content = vec![9u8; 512 * 1024];
        let digest = artiferris_domain::docker_registry::Digest::of(&content);

        for round in 0..8 {
            let start = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/"), &token, Body::empty()).await;
            let path = location_of(&start).trim_start_matches("/v2").to_string();
            send(&app, "PATCH", &path, &token, Body::from(content.clone())).await;

            let complete_uri = format!("{path}?digest={}", digest.as_str());
            let complete = send(&app, "PUT", &complete_uri, &token, Body::empty());
            let late_chunk = send(&app, "PATCH", &path, &token, Body::from(vec![1u8; 1024]));
            let (complete, _late_chunk) = tokio::join!(complete, late_chunk);

            if complete.status() == StatusCode::CREATED {
                let stored = send(&app, "GET", &format!("/{repo_name}/myimage/blobs/{}", digest.as_str()), &token, Body::empty()).await;
                let bytes = axum::body::to_bytes(stored.into_body(), usize::MAX).await.unwrap();
                assert_eq!(artiferris_domain::docker_registry::Digest::of(&bytes), digest, "round {round}: the blob stored under a digest must hash to it");
            } else {
                assert_eq!(complete.status(), StatusCode::BAD_REQUEST, "round {round}: the chunk landed first, so the client's digest no longer matches");
            }
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_rejected_upload_leaves_no_session_and_no_staging_file_behind(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let response = send(&app, "POST", &format!("/{repo_name}/myimage/blobs/uploads/?digest=sha256:{}", "0".repeat(64)), &token, Body::from(b"not that digest".to_vec())).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let leftovers: Vec<_> = walk(dir.path()).into_iter().filter(|path| path.to_string_lossy().contains(".tmp-")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert!(walk(&dir.path().join("uploads")).is_empty(), "a rejected upload's session and staging file are discarded");
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else { return found };
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                found.extend(walk(&entry.path()));
            } else {
                found.push(entry.path());
            }
        }
        found
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_hostile_image_name_never_reaches_a_location_header(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let response = send(&app, "POST", &format!("/{repo_name}/My%20Image%0d%0aX-Injected:%201/blobs/uploads/"), &token, Body::empty()).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get("x-injected").is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_user_who_is_not_a_member_of_the_personal_namespace_cannot_use_its_upload_urls(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let stranger = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let token = state.token_issuer.issue(stranger, PUBLIC_ORGANIZATION_ID, false, None).unwrap();
        let app = crate::router(state);

        let (body, polled) = probe_body();
        let response = send(&app, "PATCH", &format!("/u/alice/my-image/myimage/blobs/uploads/{}", Uuid::new_v4()), &token, body).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(!polled.load(Ordering::SeqCst));
    }
}
