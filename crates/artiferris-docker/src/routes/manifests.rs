use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerMediaType, validate_manifest_reference};
use artiferris_domain::package_repository::PackageRepositorySummary;
use uuid::Uuid;

use crate::auth::DockerAuthUser;
use crate::authz::{member_is_readable, require_docker_repository, require_granted_action_for_route, require_hosted, require_readable_repository_by_name, require_repository_by_name};
use crate::errors::{docker_authz_error, docker_error, docker_error_response};
use crate::state::DockerState;
use artiferris_application::body_budget::{BodyReservation, Refusal};
use artiferris_application::body_read::{BodyReadError, read_limited};

/// A manifest is normally a few KB.
pub const MANIFEST_BODY_LIMIT_BYTES: usize = 4 * 1024 * 1024;

/// Read after authorization, within `MANIFEST_BODY_LIMIT_BYTES` and the global body budget, which is charged twice the bytes
/// as they arrive (up to twice the declared `Content-Length`, or the limit if none) for as long as the returned guard lives.
async fn read_manifest_body(state: &DockerState, body: Body, content_length: Option<u64>, clients: &[String]) -> Result<(Bytes, BodyReservation), Response> {
    let declared = content_length.map(|length| usize::try_from(length).unwrap_or(usize::MAX));
    let expected = declared.unwrap_or(MANIFEST_BODY_LIMIT_BYTES);
    if expected > MANIFEST_BODY_LIMIT_BYTES {
        return Err(docker_error(StatusCode::PAYLOAD_TOO_LARGE, "SIZE_INVALID", "manifest is too large").into_response());
    }
    let busy = || (StatusCode::SERVICE_UNAVAILABLE, [(header::RETRY_AFTER, "1")], docker_error(StatusCode::SERVICE_UNAVAILABLE, "UNAVAILABLE", "the registry is busy, retry shortly").1).into_response();
    let mut reservation = state.guard.body_budget.admit(clients, expected * 2).map_err(|refusal| match refusal {
        Refusal::Busy => busy(),
        Refusal::TooManyFromOneClient => (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "1")], docker_error(StatusCode::TOO_MANY_REQUESTS, "TOOMANYREQUESTS", "too many uploads in flight from this client, retry shortly").1).into_response(),
    })?;
    match read_limited(body.into_data_stream(), MANIFEST_BODY_LIMIT_BYTES, state.guard.body_timeouts, declared, |received| reservation.grow_to(received * 2)).await {
        Ok(bytes) => Ok((bytes, reservation)),
        Err(BodyReadError::TimedOut) => Err(docker_error(StatusCode::REQUEST_TIMEOUT, "MANIFEST_INVALID", "the manifest took too long to arrive").into_response()),
        Err(BodyReadError::Busy) => Err(busy()),
        Err(_) => Err(docker_error(StatusCode::PAYLOAD_TOO_LARGE, "SIZE_INVALID", "manifest is too large or could not be read").into_response()),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn put_manifest(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name_str: String,
    reference: String,
    content_type: Option<String>,
    content_length: Option<u64>,
    user: DockerAuthUser,
    body: Body,
    location_base: String,
    client: String,
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
    let Ok(image_name) = DockerImageName::parse(&image_name_str) else {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    };
    // Must be rejected, not silently downgraded to DockerV2Manifest — would mis-store an OCI index as a single-image manifest.
    let Ok(media_type) = DockerMediaType::parse(content_type.as_deref().unwrap_or("")) else {
        return docker_error(StatusCode::BAD_REQUEST, "MANIFEST_INVALID", "missing or unrecognized manifest Content-Type").into_response();
    };
    if validate_manifest_reference(&reference).is_err() {
        return docker_error(StatusCode::BAD_REQUEST, "MANIFEST_INVALID", "the reference must be a tag or a digest").into_response();
    }
    let (body, _budget) = match read_manifest_body(&state, body, content_length, &[format!("client:{client}"), format!("user:{}", user.user_id)]).await {
        Ok(read) => read,
        Err(response) => return response,
    };

    match state.put_manifest.execute(repo.id, &image_name, &reference, media_type, &body, user.user_id).await {
        Ok(digest) => {
            // Only a tagged push is scanned — a digest-only push (multi-arch buildx) isn't user-visible.
            // Fire-and-forget: must not hold up the response.
            if Digest::parse(&reference).is_err() {
                let scan = state.scan_docker_image.clone();
                let scan_repo_id = repo.id;
                let scan_image_name = image_name.clone();
                let scan_tag = reference.clone();
                let scan_triggered_by = user.user_id;
                tokio::spawn(async move {
                    let _ = scan.execute(scan_repo_id, &scan_image_name, &scan_tag, scan_triggered_by).await;
                });
            }
            (
                StatusCode::CREATED,
                [
                    (header::LOCATION, format!("{location_base}/{image_name_str}/manifests/{}", digest.as_str())),
                    (HeaderName::from_static("docker-content-digest"), digest.as_str().to_string()),
                ],
            )
                .into_response()
        }
        Err(e) => docker_error_response(e).into_response(),
    }
}

pub async fn get_manifest(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name_str: String,
    reference: String,
    user: Option<DockerAuthUser>,
    // False for a HEAD probe, which asks whether a manifest exists without pulling it.
    count_pull: bool,
    // The requesting client's bucket: a client counts once per image per hour.
    client: &str,
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
    if validate_manifest_reference(&reference).is_err() {
        return docker_error(StatusCode::BAD_REQUEST, "TAG_INVALID", "the reference must be a tag or a digest").into_response();
    }

    // `user`, not `caller`: `caller` is `None` for a public top-level repository, and an authenticated caller must not
    // lose their organization's members because the wrapping group is public. `caller.is_some()` means exactly "this
    // token was verified against this top-level repository" (see `member_is_readable`).
    let caller_user = user.as_ref();
    let top_level_organization_id = repo.organization_id;
    let top_level_was_authorized = caller.is_some();
    let state_ref = &state;
    match state
        .get_manifest
        .execute(repo.id, &image_name, &reference, move |member: &PackageRepositorySummary| {
            member_is_readable(state_ref, caller_user, top_level_organization_id, top_level_was_authorized, member.id, member.organization_id, member.is_public)
        })
        .await
    {
        Ok(Some(manifest)) => {
            // A pull is a GET by tag on a hosted repository; a digest request is what a client follows a multi-architecture
            // tag with, so counting it too would count one pull twice.
            if count_pull
                && repo.repo_type == artiferris_domain::package_repository::RepositoryType::Hosted
                && Digest::parse(&reference).is_err()
                && state.guard.download_dedupe.first_in_hour(client, repo.id, image_name.as_str())
            {
                state.downloads.record(repo.id, artiferris_domain::package_repository::RepositoryFormat::Docker, image_name.as_str());
            }
            if repo.repo_type == artiferris_domain::package_repository::RepositoryType::Proxy {
                // Best-effort — a failed cache write must not fail the pull.
                let _ = state.cache_proxied_manifest.execute(&manifest).await;
            }
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(manifest.media_type.as_str()).unwrap_or(HeaderValue::from_static("application/octet-stream")));
            headers.insert(
                HeaderName::from_static("docker-content-digest"),
                HeaderValue::from_str(manifest.digest.as_str()).unwrap_or(HeaderValue::from_static("")),
            );
            headers.insert(header::ETAG, HeaderValue::from_str(&format!("\"{}\"", manifest.digest.as_str())).unwrap_or(HeaderValue::from_static("\"\"")));
            // Only a request BY digest is guaranteed immutable — a tag can be re-pushed at any time.
            if Digest::parse(&reference).is_ok() {
                for (name, value) in super::blobs::immutable_content_cache_headers(repo.is_public) {
                    headers.insert(name, HeaderValue::from_static(value));
                }
            }
            (StatusCode::OK, headers, manifest.body).into_response()
        }
        Ok(None) => docker_error(StatusCode::NOT_FOUND, "MANIFEST_UNKNOWN", "manifest not found").into_response(),
        Err(e) => docker_error_response(e).into_response(),
    }
}

pub async fn delete_manifest(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name_str: String,
    reference: String,
    user: DockerAuthUser,
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
    let Ok(image_name) = DockerImageName::parse(&image_name_str) else {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    };
    if validate_manifest_reference(&reference).is_err() {
        return docker_error(StatusCode::BAD_REQUEST, "TAG_INVALID", "the reference must be a tag or a digest").into_response();
    }

    // A tag reference is also valid here; resolve it to a digest first.
    let digest = match Digest::parse(&reference) {
        Ok(digest) => digest,
        // `require_hosted` above already excluded groups, so this policy never actually fires —
        // it's here so the call type-checks, and so it stays correct if that ever changes.
        Err(_) => match state
            .get_manifest
            .execute(repo.id, &image_name, &reference, |member: &PackageRepositorySummary| {
                member_is_readable(&state, Some(&user), repo.organization_id, true, member.id, member.organization_id, member.is_public)
            })
            .await
        {
            Ok(Some(manifest)) => manifest.digest,
            Ok(None) => return docker_error(StatusCode::NOT_FOUND, "MANIFEST_UNKNOWN", "manifest not found").into_response(),
            Err(e) => return docker_error_response(e).into_response(),
        },
    };

    match state.delete_manifest.execute(repo.id, &image_name, &digest, user.user_id).await {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
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

    use crate::route_test_support::{issue_test_token, seed_bare_user, seed_repository, test_state};
    use std::sync::Arc;

    async fn hosted_repo(pool: &sqlx::PgPool) -> (Uuid, String) {
        let repository_id = Uuid::new_v4();
        seed_repository(pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        (repository_id, format!("repo-{repository_id}"))
    }

    fn manifest_body(config_digest: &Digest) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "config": { "digest": config_digest.as_str() },
            "layers": []
        }))
        .unwrap()
    }

    async fn push_config_blob(app: &axum::Router, repo_name: &str, push_token: &str, config_bytes: &[u8]) -> Digest {
        let digest = Digest::of(config_bytes);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        digest
    }

    /// A manifest for a repository that does not exist: `require_readable_repository_by_name` returns
    /// `Err(StatusCode::NOT_FOUND)`, no body.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_authz_rejection_still_carries_the_oci_error_envelope(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/nonexistent-repo/myimage/manifests/latest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["errors"][0]["code"].is_string(), "an authz-layer rejection must still carry the OCI error envelope, got: {json}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pushing_a_manifest_by_tag_then_pulling_it_back_round_trips(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let config_digest = push_config_blob(&app, &repo_name, &push_token, b"round-trip-config-bytes").await;
        let body = manifest_body(&config_digest);

        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let returned = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        // Byte-exact: docker pull recomputes and compares Docker-Content-Digest.
        assert_eq!(returned.to_vec(), body);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_tag_reference_carries_an_etag_but_no_long_lived_cache_control_unlike_a_digest_reference(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        crate::route_test_support::mark_public(&pool, repository_id).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let config_digest = push_config_blob(&app, &repo_name, &push_token, b"cache-header-config-bytes").await;
        let body = manifest_body(&config_digest);

        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);
        let digest = put_response.headers().get(HeaderName::from_static("docker-content-digest")).unwrap().to_str().unwrap().to_string();

        let by_tag = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(by_tag.headers().get(header::ETAG).is_some(), "a tag reference must still carry an ETag for conditional requests");
        assert!(by_tag.headers().get(header::CACHE_CONTROL).is_none(), "a tag can be reassigned — must not be cached long-lived");

        let by_digest = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/{digest}"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(by_digest.headers().get(header::CACHE_CONTROL).unwrap(), "public, max-age=31536000, immutable");
        assert_eq!(by_digest.headers().get(header::ETAG).unwrap(), &format!("\"{digest}\""));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pushing_a_manifest_over_the_repositorys_quota_is_rejected(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let config_digest = push_config_blob(&app, &repo_name, &push_token, b"a config blob larger than the quota").await;
        // The quota shrinks after the upload; a blob that does not fit is refused at upload.
        crate::route_test_support::set_quota(&pool, repository_id, 5).await;
        let body = manifest_body(&config_digest);

        let put_response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(put_response.status(), StatusCode::INSUFFICIENT_STORAGE);
    }

    /// Regression: a malformed tag bubbled up as a bare `DomainError::Validation`, which `docker_error_response` has no
    /// arm for, so it became a 500. It must be a 400 like every other malformed payload.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pushing_a_manifest_with_a_malformed_tag_is_a_400_not_a_500(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let config_digest = push_config_blob(&app, &repo_name, &push_token, b"malformed-tag-config-bytes").await;
        let body = manifest_body(&config_digest);

        // "!" is outside the OCI tag grammar `[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}`.
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest!"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "a malformed tag is a client input error, not a server error");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["errors"][0]["code"], "MANIFEST_INVALID", "got {json}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pushing_without_push_scope_is_forbidden(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let config_digest = Digest::of(b"unused-config-bytes");

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(manifest_body(&config_digest)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pushing_a_manifest_with_missing_or_unrecognized_content_type_is_rejected(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let config_digest = Digest::of(b"unused-config-bytes");
        let body = manifest_body(&config_digest);

        let no_content_type = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(no_content_type.status(), StatusCode::BAD_REQUEST);

        let garbage_content_type = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/x-not-a-real-manifest-type")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(garbage_content_type.status(), StatusCode::BAD_REQUEST);
    }

    /// Without its own cap a manifest would ride on the router's 2 GB blob-sized `DefaultBodyLimit`, letting a pusher force a huge JSON parse.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pushing_an_oversized_manifest_is_rejected_before_it_would_ever_be_parsed(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        // Bigger than the manifest limit, well under the blob limit — a 413 proves the manifest-specific cap stopped it.
        let oversized = vec![b' '; 11 * 1024 * 1024];

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(oversized))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pulling_a_missing_manifest_is_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/nonexistent"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn deleting_a_pushed_manifest_by_tag_then_pulling_it_is_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push", "pull"]);
        let app = crate::router(state);
        let config_digest = push_config_blob(&app, &repo_name, &push_token, b"delete-round-trip-config-bytes").await;

        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(manifest_body(&config_digest)))
                    .unwrap(),
            )
            .await
            .unwrap();

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::ACCEPTED);

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn manifest_routes_404_on_a_non_docker_format_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        let state = test_state(pool.clone(), dir.path()).await;
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // Deleting a manifest list must not touch its members' blob refs — it never incremented them.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn deleting_a_manifest_list_does_not_touch_its_members_blob_references(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push", "pull"]);
        let app = crate::router(state);

        let config_a = push_config_blob(&app, &repo_name, &push_token, b"member-a-config-bytes").await;
        let body_a = manifest_body(&config_a);
        let digest_a = Digest::of(&body_a);
        let put_a = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/{}", digest_a.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body_a))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_a.status(), StatusCode::CREATED);

        let config_b = push_config_blob(&app, &repo_name, &push_token, b"member-b-config-bytes").await;
        let body_b = manifest_body(&config_b);
        let digest_b = Digest::of(&body_b);
        let put_b = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/{}", digest_b.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body_b))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_b.status(), StatusCode::CREATED);

        let index_body = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                { "digest": digest_a.as_str(), "mediaType": "application/vnd.docker.distribution.manifest.v2+json" },
                { "digest": digest_b.as_str(), "mediaType": "application/vnd.docker.distribution.manifest.v2+json" },
            ]
        }))
        .unwrap();
        let put_index = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.oci.image.index.v1+json")
                    .body(Body::from(index_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_index.status(), StatusCode::CREATED);

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            delete_response.status(),
            StatusCode::ACCEPTED,
            "deleting the index must succeed and must NOT decrement member blob refs it never incremented"
        );

        for member_config_digest in [&config_a, &config_b] {
            let blob_response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(format!("/{repo_name}/myimage/blobs/{}", member_config_digest.as_str()))
                        .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                blob_response.status(),
                StatusCode::OK,
                "member manifest's blob must survive deleting the index that pointed at the member"
            );
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_in_the_requested_organization_is_reachable_via_its_own_subdomain(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::route_test_support::test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&artiferris_domain::organization::Organization {
                id: acme_id,
                slug: artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        crate::route_test_support::seed_repository(&pool, acme_id, repo_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let acme_user_id = crate::route_test_support::seed_user_with_active_token(&pool, acme_id, "acme-plaintext-token").await;
        let token = crate::route_test_support::issue_test_token_for_org(&state, acme_user_id, acme_id, false, repo_id, &repo_name, "myimage", &["push", "pull"]);
        let app = crate::router(state);

        let config_bytes = b"acme-own-subdomain-config-bytes";
        let config_digest = Digest::of(config_bytes);
        let blob_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blob_response.status(), StatusCode::CREATED);

        let body = manifest_body(&config_digest);
        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let returned = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), body);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_manifest_in_one_organizations_repository_is_not_reachable_from_another_organization(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::route_test_support::test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&artiferris_domain::organization::Organization {
                id: acme_id,
                slug: artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let other_id = Uuid::new_v4();
        state
            .organizations
            .create(&artiferris_domain::organization::Organization {
                id: other_id,
                slug: artiferris_domain::organization::OrganizationSlug::parse("other").unwrap(),
                display_name: "Other".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        crate::route_test_support::seed_repository(&pool, acme_id, repo_id, "docker", "hosted").await;
        let other_user_id = crate::route_test_support::seed_user_with_active_token(&pool, other_id, "plaintext-token").await;
        let token = crate::route_test_support::issue_test_token(&state, other_user_id, repo_id, &format!("repo-{repo_id}"), "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/repo-{repo_id}/myimage/manifests/latest"))
                    .header("host", "other.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// This token's embedded organization differs but the request targets the repository's owning organization, so only
    /// an explicit organization check can reject it.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_token_holders_own_organization_must_match_even_with_a_valid_granted_scope(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::route_test_support::test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&artiferris_domain::organization::Organization {
                id: acme_id,
                slug: artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let other_id = Uuid::new_v4();
        state
            .organizations
            .create(&artiferris_domain::organization::Organization {
                id: other_id,
                slug: artiferris_domain::organization::OrganizationSlug::parse("other").unwrap(),
                display_name: "Other".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        crate::route_test_support::seed_repository(&pool, acme_id, repo_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let app = crate::router(state.clone());

        let acme_user_id = crate::route_test_support::seed_user_with_active_token(&pool, acme_id, "acme-owns-this-manifest").await;
        let acme_token = crate::route_test_support::issue_test_token_for_org(&state, acme_user_id, acme_id, false, repo_id, &repo_name, "myimage", &["push", "pull"]);
        let config_bytes = b"cross-org-replay-config-bytes";
        let config_digest = Digest::of(config_bytes);
        let blob_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {acme_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blob_response.status(), StatusCode::CREATED);
        let manifest_bytes = manifest_body(&config_digest);
        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {acme_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(manifest_bytes.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let acme_get = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {acme_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(acme_get.status(), StatusCode::OK, "sanity check: the pushed manifest must be fetchable by its own organization");

        // Holder's org is "other", but the granted scope is a real "pull" on acme's repository — a stale cross-org grant.
        let other_token = crate::route_test_support::issue_test_token_for_org(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, other_id, false, repo_id, &repo_name, "myimage", &["pull"]);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {other_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a valid granted scope on a manifest that genuinely exists must not be enough to cross an organization boundary"
        );
    }


    async fn attach_group_member(pool: &sqlx::PgPool, group_id: Uuid, member_id: Uuid) {
        use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort};
        let store = artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let (version, _) = store.load(group_id).await.unwrap();
        store
            .append(
                group_id,
                version,
                vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: group_id, member_repository_id: member_id, position: 0 }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
    }

    async fn mark_repository_public(pool: &sqlx::PgPool, repository_id: Uuid) {
        use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort};
        let store = artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let (version, _) = store.load(repository_id).await.unwrap();
        store
            .append(repository_id, version, vec![PackageRepositoryEvent::VisibilityChanged { repository_id, is_public: true }], Uuid::new_v4())
            .await
            .unwrap();
    }

    /// Once the top-level group is public no token is needed, so an anonymous puller would reach a private member's
    /// manifest through it. The closing assertion pulls the same manifest with a real token and must succeed: that
    /// proves the 404 comes from the per-member policy, and pins the identity the policy runs as (a public top level
    /// yields no `caller`, so it uses the raw authenticated user).
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_group_member_is_not_reachable_through_a_public_group_by_an_anonymous_caller(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (member_id, member_name) = hosted_repo(&pool).await;
        let group_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, group_id, "docker", "group").await;
        let group_name = format!("repo-{group_id}");
        attach_group_member(&pool, group_id, member_id).await;
        mark_repository_public(&pool, group_id).await;

        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, member_id, &member_name, "myimage", &["push"]);
        let reader = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        crate::route_test_support::seed_permission(&pool, reader, member_id, "read").await;
        let pull_token = issue_test_token(&state, reader, group_id, &group_name, "myimage", &["pull"]);
        let app = crate::router(state);

        let config_digest = push_config_blob(&app, &member_name, &push_token, b"group-member-config-bytes").await;
        let body = manifest_body(&config_digest);
        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{member_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let anonymous = app
            .clone()
            .oneshot(Request::builder().method("GET").uri(format!("/{group_name}/myimage/manifests/latest")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            anonymous.status(),
            StatusCode::NOT_FOUND,
            "a public group must not hand a private member's manifest to a caller who presented no token at all"
        );

        let authenticated = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{group_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authenticated.status(), StatusCode::OK, "a caller with a Read grant on the member must still resolve through the group");
        let returned = axum::body::to_bytes(authenticated.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), body);
    }

    /// Regression: a personal organization's id never equals a real user's `organization_id`, so comparing every member
    /// to the caller's organization locked alice out of her own group's members. The policy also accepts a member
    /// sharing the organization of a top-level repository the token was verified against, which makes a personal group
    /// work.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_personal_group_members_own_owner_can_pull_it_through_the_group(pool: sqlx::PgPool) {
        use artiferris_domain::package_repository::RepositoryFormat;
        use crate::route_test_support::{create_personal_group_over_a_personal_member, seed_named_user_with_active_token};

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let (group_id, member_id) = create_personal_group_over_a_personal_member(&pool, alice_id, "my-group", "my-image", RepositoryFormat::Docker).await;

        let push_token = issue_test_token(&state, alice_id, member_id, "my-image", "myimage", &["push", "pull"]);
        let group_pull_token = issue_test_token(&state, alice_id, group_id, "my-group", "myimage", &["pull"]);
        let app = crate::router(state);

        let config_bytes = b"personal-group-member-config-bytes";
        let config_digest = Digest::of(config_bytes);
        let blob_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/u/alice/my-image/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blob_response.status(), StatusCode::CREATED);

        let body = manifest_body(&config_digest);
        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/u/alice/my-image/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let through_group = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/alice/my-group/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {group_pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            through_group.status(),
            StatusCode::OK,
            "a user must be able to pull their own personal hosted project through their own personal group project"
        );
        let returned = axum::body::to_bytes(through_group.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), body);

        // The same blob must come back through the group: `get_blob` carries its own copy of the policy.
        let blob_through_group = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/u/alice/my-group/myimage/blobs/{}", config_digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {group_pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blob_through_group.status(), StatusCode::OK, "the blob route must apply the same per-member policy as the manifest route");
        let blob_bytes = axum::body::to_bytes(blob_through_group.into_body(), usize::MAX).await.unwrap();
        assert_eq!(blob_bytes.to_vec(), config_bytes.to_vec());

        // The other half: the relaxation must not hand the member to a caller never authorized against the group.
        // Mallory holds no grant, so the real `/v2/token` endpoint issues her a scope without actions and the top-level
        // gate rejects her before traversal. Going through that endpoint, not `issue_test_token`, is the point.
        seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "mallory", "mallory-token").await;
        let mut basic = axum::http::HeaderMap::new();
        axum_extra::headers::HeaderMapExt::typed_insert(&mut basic, axum_extra::headers::Authorization::basic("ignored", "mallory-token"));
        let basic_value = basic.get(axum::http::header::AUTHORIZATION).unwrap().clone();
        let token_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/token?scope=repository:u/alice/my-group/myimage:pull")
                    .header(axum::http::header::AUTHORIZATION, basic_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(token_response.status(), StatusCode::OK);
        let token_bytes = axum::body::to_bytes(token_response.into_body(), usize::MAX).await.unwrap();
        let token_json: serde_json::Value = serde_json::from_slice(&token_bytes).unwrap();
        let mallory_token = token_json["token"].as_str().unwrap().to_string();

        let stranger = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/alice/my-group/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {mallory_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stranger.status(), StatusCode::NOT_FOUND, "a caller with no grant on the personal group must not reach its private member");

        let anonymous = app
            .oneshot(Request::builder().method("GET").uri("/u/alice/my-group/myimage/manifests/latest").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(anonymous.status(), StatusCode::NOT_FOUND, "an anonymous caller must not reach a private personal member through the group");
    }

    async fn send(app: &axum::Router, method: &str, uri: &str, token: &str, body: Body) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(Request::builder().method(method).uri(uri).header(axum::http::header::AUTHORIZATION, format!("Bearer {token}")).body(body).unwrap())
            .await
            .unwrap()
    }

    async fn put_manifest_request(app: &axum::Router, uri: &str, token: &str, body: Vec<u8>) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(uri)
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_repositorys_manifest_by_digest_is_not_cacheable_by_a_shared_cache(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push", "pull"]);
        let app = crate::router(state);
        let config_digest = push_config_blob(&app, &repo_name, &token, b"private-config").await;
        let put = put_manifest_request(&app, &format!("/{repo_name}/myimage/manifests/latest"), &token, manifest_body(&config_digest)).await;
        let digest = put.headers().get(HeaderName::from_static("docker-content-digest")).unwrap().to_str().unwrap().to_string();

        let response = send(&app, "GET", &format!("/{repo_name}/myimage/manifests/{digest}"), &token, Body::empty()).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::CACHE_CONTROL).unwrap(), "private, no-store");
        assert_eq!(response.headers().get(header::VARY).unwrap(), "Authorization");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_reference_that_is_neither_a_tag_nor_a_digest_is_rejected_before_it_is_used(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push", "pull"]);
        let app = crate::router(state);

        for reference in ["a%23b", "a%3Fb", "..%2F..%2Fother%2Fmanifests%2Flatest", "sha256:zz"] {
            for method in ["GET", "DELETE"] {
                let response = send(&app, method, &format!("/{repo_name}/myimage/manifests/{reference}"), &token, Body::empty()).await;
                assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{method} {reference}");
            }
        }
    }

    /// A manifest body that declares itself too large is never read.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_manifest_body_is_read_only_for_an_authorized_caller_and_only_up_to_its_limit(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let (body, polled) = crate::route_test_support::probe_body();
        let declared_huge = Request::builder()
            .method("PUT")
            .uri(format!("/{repo_name}/myimage/manifests/latest"))
            .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
            .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
            .header(axum::http::header::CONTENT_LENGTH, (MANIFEST_BODY_LIMIT_BYTES + 1).to_string())
            .body(body)
            .unwrap();
        let response = app.clone().oneshot(declared_huge).await.unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(!polled.load(std::sync::atomic::Ordering::SeqCst), "a body that declares itself over the limit is refused unread");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_manifest_push_is_turned_away_while_the_body_budget_is_spent(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let mut state = test_state(pool.clone(), dir.path()).await;
        state.guard = Arc::new(artiferris_application::request_guard::RequestGuard { body_budget: artiferris_application::body_budget::BodyBudget::new(1024), ..Default::default() });
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let response = put_manifest_request(&app, &format!("/{repo_name}/myimage/manifests/latest"), &token, vec![b' '; 4096]).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(response.headers().get(header::RETRY_AFTER).is_some());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn one_user_cannot_have_more_than_a_few_manifest_bodies_in_flight_and_stalled_ones_pin_nothing(pool: sqlx::PgPool) {
        use futures_util::StreamExt;

        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let mut state = test_state(pool.clone(), dir.path()).await;
        let budget_bytes = 4 * MANIFEST_BODY_LIMIT_BYTES;
        state.guard = Arc::new(artiferris_application::request_guard::RequestGuard { body_budget: artiferris_application::body_budget::BodyBudget::new(budget_bytes), ..Default::default() });
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let guard = state.guard.clone();
        let app = crate::router(state);
        let stalled_request = || {
            let stalled = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"{"))]).chain(futures_util::stream::pending());
            Request::builder()
                .method("PUT")
                .uri(format!("/{repo_name}/myimage/manifests/latest"))
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                .header(axum::http::header::CONTENT_LENGTH, MANIFEST_BODY_LIMIT_BYTES.to_string())
                .body(Body::from_stream(stalled))
                .unwrap()
        };
        let stalled: Vec<_> = (0..artiferris_application::body_budget::MAX_BODIES_PER_CLIENT).map(|_| tokio::spawn(app.clone().oneshot(stalled_request()))).collect();
        // Each spawned request clears auth and DB work before registering with the body budget, slow on a contended
        // runner. Wait for every slot to be taken: a probe arriving while one is free is admitted as a fifth stalled
        // body and the test hangs on its idle timeout (408) instead of seeing the 429.
        for _ in 0..500 {
            if guard.body_budget.most_in_flight_from_one_client() == artiferris_application::body_budget::MAX_BODIES_PER_CLIENT {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let response = app.clone().oneshot(stalled_request()).await.unwrap();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(guard.body_budget.try_reserve(budget_bytes / 2).is_some(), "four bodies that sent one byte each hold next to nothing");
        for task in stalled {
            task.abort();
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_manifest_body_that_stalls_gets_408_and_gives_its_budget_back(pool: sqlx::PgPool) {
        use futures_util::StreamExt;

        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let mut state = test_state(pool.clone(), dir.path()).await;
        let budget_bytes = 4 * MANIFEST_BODY_LIMIT_BYTES;
        state.guard = Arc::new(artiferris_application::request_guard::RequestGuard {
            body_budget: artiferris_application::body_budget::BodyBudget::new(budget_bytes),
            body_timeouts: artiferris_application::body_read::BodyTimeouts { idle: std::time::Duration::from_millis(100), total: std::time::Duration::from_secs(5) },
            ..Default::default()
        });
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let guard = state.guard.clone();
        let app = crate::router(state);
        let stalled = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"{"))]).chain(futures_util::stream::pending());

        let request = Request::builder()
            .method("PUT")
            .uri(format!("/{repo_name}/myimage/manifests/latest"))
            .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
            .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
            .body(Body::from_stream(stalled))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        assert!(guard.body_budget.try_reserve(budget_bytes).is_some(), "the stalled request kept its reservation");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_manifest_that_lists_the_same_layer_twice_is_accepted(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let (repository_id, repo_name) = hosted_repo(&pool).await;
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let config = push_config_blob(&app, &repo_name, &token, b"config").await;
        let layer = push_config_blob(&app, &repo_name, &token, b"one layer used twice").await;
        let body = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "config": { "digest": config.as_str() },
            "layers": [{ "digest": layer.as_str() }, { "digest": layer.as_str() }, { "digest": layer.as_str() }]
        }))
        .unwrap();

        let response = put_manifest_request(&app, &format!("/{repo_name}/myimage/manifests/latest"), &token, body).await;

        assert_eq!(response.status(), StatusCode::CREATED);
        let counts: Vec<i64> = sqlx::query_scalar!("SELECT reference_count FROM docker_blobs ORDER BY digest").fetch_all(&pool).await.unwrap();
        assert_eq!(counts, vec![1, 1], "one reference per blob, however often the manifest lists it");
    }

    /// A group over one private member holding a pushed manifest; returns the group id, member id and their names.
    async fn group_over_a_private_member(pool: &sqlx::PgPool, state: &DockerState, app: &axum::Router) -> (Uuid, Uuid, String, String) {
        let (member_id, member_name) = hosted_repo(pool).await;
        let group_id = Uuid::new_v4();
        seed_repository(pool, PUBLIC_ORGANIZATION_ID, group_id, "docker", "group").await;
        attach_group_member(pool, group_id, member_id).await;
        let push_token = issue_test_token(state, seed_bare_user(pool, PUBLIC_ORGANIZATION_ID).await, member_id, &member_name, "myimage", &["push"]);
        let config = push_config_blob(app, &member_name, &push_token, b"group-member-config").await;
        put_manifest_request(app, &format!("/{member_name}/myimage/manifests/latest"), &push_token, manifest_body(&config)).await;
        (group_id, member_id, format!("repo-{group_id}"), member_name)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_group_reader_without_a_grant_on_a_private_member_cannot_read_through_the_group(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let app = crate::router(state.clone());
        let (group_id, _member_id, group_name, _) = group_over_a_private_member(&pool, &state, &app).await;
        let reader = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let token = issue_test_token(&state, reader, group_id, &group_name, "myimage", &["pull"]);

        let response = send(&app, "GET", &format!("/{group_name}/myimage/manifests/latest"), &token, Body::empty()).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_grant_on_the_member_is_checked_live_not_read_from_the_token(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let app = crate::router(state.clone());
        let (group_id, member_id, group_name, _) = group_over_a_private_member(&pool, &state, &app).await;
        let reader = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let token = issue_test_token(&state, reader, group_id, &group_name, "myimage", &["pull"]);
        let uri = format!("/{group_name}/myimage/manifests/latest");

        crate::route_test_support::seed_permission(&pool, reader, member_id, "read").await;
        assert_eq!(send(&app, "GET", &uri, &token, Body::empty()).await.status(), StatusCode::OK);

        sqlx::query!("DELETE FROM permission_projections WHERE user_id = $1 AND repository_id = $2", reader, member_id).execute(&pool).await.unwrap();
        assert_eq!(send(&app, "GET", &uri, &token, Body::empty()).await.status(), StatusCode::NOT_FOUND, "the same token stops reaching the member once the grant is gone");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_reads_every_member_of_their_organization_through_a_group(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let app = crate::router(state.clone());
        let (group_id, _member_id, group_name, _) = group_over_a_private_member(&pool, &state, &app).await;
        let admin = crate::route_test_support::seed_organization_admin_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "admin-api-token").await;
        let token = issue_test_token(&state, admin, group_id, &group_name, "myimage", &["pull"]);

        let response = send(&app, "GET", &format!("/{group_name}/myimage/manifests/latest"), &token, Body::empty()).await;

        assert_eq!(response.status(), StatusCode::OK);
    }
}
