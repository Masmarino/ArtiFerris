use axum::Router;
use axum::body::Body;
use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use std::future::Future;
use std::net::SocketAddr;

use uuid::Uuid;

use crate::auth::DockerAuthUser;
use crate::authz::resolve_personal_repository;
use crate::errors::docker_authz_error;
use crate::organization_resolution::ResolvedOrganization;
use crate::routes::blobs;
use crate::routes::manifests;
use crate::routes::path::{DockerOperation, parse_operation};
use crate::routes::tags::{self, TagsQueryParams};
use crate::state::DockerState;

pub fn router() -> Router<DockerState> {
    Router::new()
        .route(
            "/{repository}/{*rest}",
            get(handle_get).head(handle_head).post(handle_post).put(handle_put).patch(handle_patch).delete(handle_delete),
        )
        .route(
            "/u/{username}/{repo}/{*rest}",
            get(handle_get_personal).head(handle_head_personal).post(handle_post_personal).put(handle_put_personal).patch(handle_patch_personal).delete(handle_delete_personal),
        )
}

#[derive(Deserialize)]
pub struct BlobQueryParams {
    digest: Option<String>,
}

// `dispatch_*` holds the per-verb match once; each `handle_*` wrapper supplies its own `resolve_organization_id`
// closure. It returns `Result<Uuid, Response>` so each path keeps its own failure shape.
//
// Call it inside the matched `parse_operation` arm: the personal lookup hits the database, so calling it before the
// match would let an unhandled verb or shape reveal a private repository through 405 versus 404.

#[allow(clippy::too_many_arguments)]
async fn dispatch_get<F, Fut>(state: DockerState, repository: String, rest: String, user: Option<DockerAuthUser>, client: String, tags_query: TagsQueryParams, location_base: String, resolve_organization_id: F) -> Response
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Uuid, Response>>,
{
    match parse_operation(&rest) {
        Some(DockerOperation::Blob { image_name, digest }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            blobs::get_blob(state, organization_id, repository, image_name, digest, user).await
        }
        Some(DockerOperation::Manifest { image_name, reference }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            manifests::get_manifest(state, organization_id, repository, image_name, reference, user, true, &client).await
        }
        Some(DockerOperation::TagsList { image_name }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            tags::list_tags(state, organization_id, repository, image_name, user, tags_query, location_base).await
        }
        Some(_) => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Only the blob case gets the lightweight `head_blob` treatment; other HEAD requests fall back to the same GET handlers.
#[allow(clippy::too_many_arguments)]
async fn dispatch_head<F, Fut>(state: DockerState, repository: String, rest: String, user: Option<DockerAuthUser>, client: String, tags_query: TagsQueryParams, location_base: String, resolve_organization_id: F) -> Response
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Uuid, Response>>,
{
    match parse_operation(&rest) {
        Some(DockerOperation::Blob { image_name, digest }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            blobs::head_blob(state, organization_id, repository, image_name, digest, user).await
        }
        Some(DockerOperation::Manifest { image_name, reference }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            manifests::get_manifest(state, organization_id, repository, image_name, reference, user, false, &client).await
        }
        Some(DockerOperation::TagsList { image_name }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            tags::list_tags(state, organization_id, repository, image_name, user, tags_query, location_base).await
        }
        Some(_) => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Bodies reach the handlers unread and are only consumed after the repository, role and scope checks.
/// `location_base` is the path the request came in through, for `Location` headers.
#[allow(clippy::too_many_arguments)]
async fn dispatch_post<F, Fut>(
    state: DockerState,
    repository: String,
    rest: String,
    params: BlobQueryParams,
    user: DockerAuthUser,
    body: Body,
    location_base: String,
    resolve_organization_id: F,
) -> Response
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Uuid, Response>>,
{
    match parse_operation(&rest) {
        Some(DockerOperation::BlobUploadStart { image_name }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            blobs::start_or_monolithic_upload(state, organization_id, repository, image_name, params.digest, user, body, location_base).await
        }
        Some(_) => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_patch<F, Fut>(
    state: DockerState,
    repository: String,
    rest: String,
    headers: axum::http::HeaderMap,
    user: DockerAuthUser,
    chunk: Body,
    location_base: String,
    resolve_organization_id: F,
) -> Response
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Uuid, Response>>,
{
    match parse_operation(&rest) {
        Some(DockerOperation::BlobUploadChunk { image_name, upload_id }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            let content_range = headers.get(axum::http::header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
            blobs::patch_chunk(state, organization_id, repository, image_name, upload_id, content_range, user, chunk, location_base).await
        }
        Some(_) => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_put<F, Fut>(
    state: DockerState,
    repository: String,
    rest: String,
    params: BlobQueryParams,
    headers: axum::http::HeaderMap,
    user: DockerAuthUser,
    body: Body,
    location_base: String,
    client: String,
    resolve_organization_id: F,
) -> Response
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Uuid, Response>>,
{
    match parse_operation(&rest) {
        Some(DockerOperation::BlobUploadChunk { image_name, upload_id }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            blobs::complete_upload(state, organization_id, repository, image_name, upload_id, params.digest, user, body, location_base).await
        }
        Some(DockerOperation::Manifest { image_name, reference }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            let content_type = headers.get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
            let content_length = headers.get(axum::http::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
            manifests::put_manifest(state, organization_id, repository, image_name, reference, content_type, content_length, user, body, location_base, client).await
        }
        Some(_) => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn dispatch_delete<F, Fut>(state: DockerState, repository: String, rest: String, user: DockerAuthUser, resolve_organization_id: F) -> Response
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Uuid, Response>>,
{
    match parse_operation(&rest) {
        Some(DockerOperation::Manifest { image_name, reference }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            manifests::delete_manifest(state, organization_id, repository, image_name, reference, user).await
        }
        Some(DockerOperation::BlobUploadChunk { image_name, upload_id }) => {
            let organization_id = match resolve_organization_id().await {
                Ok(id) => id,
                Err(response) => return response,
            };
            blobs::cancel_upload(state, organization_id, repository, image_name, upload_id, user).await
        }
        Some(_) => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn client_of(state: &DockerState, headers: &axum::http::HeaderMap, connect_info: &Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> String {
    let forwarded: Vec<&str> = headers.get_all("x-forwarded-for").iter().filter_map(|value| value.to_str().ok()).collect();
    state.guard.client_bucket(connect_info.as_ref().ok().map(|ConnectInfo(addr)| addr.ip()), &forwarded)
}

/// A read that may not be made anonymously comes out of the dispatch as a bare 401, for a private repository and a
/// missing one alike. This adds the challenge, scoped to the repository the client named (`repo`, or
/// `u/{username}/{repo}`), so that it fetches a token with its credentials and retries: containerd only sends its pull
/// secret after a challenge, and gives up on a 404.
fn challenge_anonymous_read(state: &DockerState, headers: &axum::http::HeaderMap, scope_repository: &str, rest: &str, response: Response) -> Response {
    if response.status() != StatusCode::UNAUTHORIZED || response.headers().contains_key(axum::http::header::WWW_AUTHENTICATE) {
        return response;
    }
    let scope = parse_operation(rest).map(|operation| format!("repository:{scope_repository}/{}:pull", operation.image_name()));
    crate::auth::unauthorized(state, state.host_header(headers), scope.as_deref())
}

/// An unknown organization subdomain is answered, to an anonymous reader, like a private repository.
fn organization_for_read(resolved: Result<ResolvedOrganization, StatusCode>, anonymous: bool) -> Result<Uuid, StatusCode> {
    match resolved {
        Ok(ResolvedOrganization(organization)) => Ok(organization.id),
        Err(StatusCode::NOT_FOUND) if anonymous => Err(StatusCode::UNAUTHORIZED),
        Err(status) => Err(status),
    }
}

async fn handle_get(
    State(state): State<DockerState>,
    Path((repository, rest)): Path<(String, String)>,
    Query(tags_query): Query<TagsQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    resolved_org: Result<ResolvedOrganization, StatusCode>,
    user: Option<DockerAuthUser>,
) -> Response {
    let organization_id = organization_for_read(resolved_org, user.is_none());
    let client = client_of(&state, &headers, &connect_info);
    let location_base = format!("/v2/{repository}");
    let response = dispatch_get(state.clone(), repository.clone(), rest.clone(), user, client, tags_query, location_base, move || async move {
        organization_id.map_err(docker_authz_error)
    })
    .await;
    challenge_anonymous_read(&state, &headers, &repository, &rest, response)
}

async fn handle_head(
    State(state): State<DockerState>,
    Path((repository, rest)): Path<(String, String)>,
    Query(tags_query): Query<TagsQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    resolved_org: Result<ResolvedOrganization, StatusCode>,
    user: Option<DockerAuthUser>,
) -> Response {
    let organization_id = organization_for_read(resolved_org, user.is_none());
    let client = client_of(&state, &headers, &connect_info);
    let location_base = format!("/v2/{repository}");
    let response = dispatch_head(state.clone(), repository.clone(), rest.clone(), user, client, tags_query, location_base, move || async move {
        organization_id.map_err(docker_authz_error)
    })
    .await;
    challenge_anonymous_read(&state, &headers, &repository, &rest, response)
}

async fn handle_post(
    State(state): State<DockerState>,
    Path((repository, rest)): Path<(String, String)>,
    Query(params): Query<BlobQueryParams>,
    resolved_org: ResolvedOrganization,
    user: DockerAuthUser,
    body: Body,
) -> Response {
    let organization_id = resolved_org.0.id;
    let location_base = format!("/v2/{repository}");
    dispatch_post(state, repository, rest, params, user, body, location_base, || async move { Ok::<Uuid, Response>(organization_id) }).await
}

async fn handle_patch(
    State(state): State<DockerState>,
    Path((repository, rest)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    resolved_org: ResolvedOrganization,
    user: DockerAuthUser,
    chunk: Body,
) -> Response {
    let organization_id = resolved_org.0.id;
    let location_base = format!("/v2/{repository}");
    dispatch_patch(state, repository, rest, headers, user, chunk, location_base, || async move { Ok::<Uuid, Response>(organization_id) }).await
}

async fn handle_put(
    State(state): State<DockerState>,
    Path((repository, rest)): Path<(String, String)>,
    Query(params): Query<BlobQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    resolved_org: ResolvedOrganization,
    user: DockerAuthUser,
    body: Body,
) -> Response {
    let organization_id = resolved_org.0.id;
    let location_base = format!("/v2/{repository}");
    let client = client_of(&state, &headers, &connect_info);
    dispatch_put(state, repository, rest, params, headers, user, body, location_base, client, || async move { Ok::<Uuid, Response>(organization_id) }).await
}

async fn handle_delete(
    State(state): State<DockerState>,
    Path((repository, rest)): Path<(String, String)>,
    resolved_org: ResolvedOrganization,
    user: DockerAuthUser,
) -> Response {
    let organization_id = resolved_org.0.id;
    dispatch_delete(state, repository, rest, user, || async move { Ok::<Uuid, Response>(organization_id) }).await
}

// Personal counterparts of the handlers above, resolved with `resolve_personal_repository` since personal organizations
// have no subdomain; see `dispatch_*` for why that is a closure called inside each matched arm.
async fn resolve_personal_organization_id(state: &DockerState, username: &str, repo: &str) -> Result<Uuid, Response> {
    resolve_personal_repository(state, username, repo).await.map(|r| r.organization_id).map_err(docker_authz_error)
}

/// For reads: a personal project that does not exist is answered, to an anonymous reader, like a private one.
async fn resolve_personal_organization_id_for_read(state: &DockerState, username: &str, repo: &str, anonymous: bool) -> Result<Uuid, Response> {
    match resolve_personal_repository(state, username, repo).await {
        Ok(repository) => Ok(repository.organization_id),
        Err(StatusCode::NOT_FOUND) if anonymous => Err(docker_authz_error(StatusCode::UNAUTHORIZED)),
        Err(status) => Err(docker_authz_error(status)),
    }
}

async fn handle_get_personal(
    State(state): State<DockerState>,
    Path((username, repo, rest)): Path<(String, String, String)>,
    Query(tags_query): Query<TagsQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    user: Option<DockerAuthUser>,
) -> Response {
    let anonymous = user.is_none();
    let client = client_of(&state, &headers, &connect_info);
    let location_base = format!("/v2/u/{username}/{repo}");
    let response = dispatch_get(state.clone(), repo.clone(), rest.clone(), user, client, tags_query, location_base, || {
        resolve_personal_organization_id_for_read(&state, &username, &repo, anonymous)
    })
    .await;
    challenge_anonymous_read(&state, &headers, &format!("u/{username}/{repo}"), &rest, response)
}

async fn handle_head_personal(
    State(state): State<DockerState>,
    Path((username, repo, rest)): Path<(String, String, String)>,
    Query(tags_query): Query<TagsQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    user: Option<DockerAuthUser>,
) -> Response {
    let anonymous = user.is_none();
    let client = client_of(&state, &headers, &connect_info);
    let location_base = format!("/v2/u/{username}/{repo}");
    let response = dispatch_head(state.clone(), repo.clone(), rest.clone(), user, client, tags_query, location_base, || {
        resolve_personal_organization_id_for_read(&state, &username, &repo, anonymous)
    })
    .await;
    challenge_anonymous_read(&state, &headers, &format!("u/{username}/{repo}"), &rest, response)
}

async fn handle_post_personal(
    State(state): State<DockerState>,
    Path((username, repo, rest)): Path<(String, String, String)>,
    Query(params): Query<BlobQueryParams>,
    user: DockerAuthUser,
    body: Body,
) -> Response {
    let resolve_state = state.clone();
    let resolve_repo = repo.clone();
    let location_base = format!("/v2/u/{username}/{repo}");
    dispatch_post(state, repo, rest, params, user, body, location_base, || resolve_personal_organization_id(&resolve_state, &username, &resolve_repo)).await
}

async fn handle_patch_personal(
    State(state): State<DockerState>,
    Path((username, repo, rest)): Path<(String, String, String)>,
    headers: axum::http::HeaderMap,
    user: DockerAuthUser,
    chunk: Body,
) -> Response {
    let resolve_state = state.clone();
    let resolve_repo = repo.clone();
    let location_base = format!("/v2/u/{username}/{repo}");
    dispatch_patch(state, repo, rest, headers, user, chunk, location_base, || resolve_personal_organization_id(&resolve_state, &username, &resolve_repo)).await
}

async fn handle_put_personal(
    State(state): State<DockerState>,
    Path((username, repo, rest)): Path<(String, String, String)>,
    Query(params): Query<BlobQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    user: DockerAuthUser,
    body: Body,
) -> Response {
    let resolve_state = state.clone();
    let resolve_repo = repo.clone();
    let location_base = format!("/v2/u/{username}/{repo}");
    let client = client_of(&state, &headers, &connect_info);
    dispatch_put(state, repo, rest, params, headers, user, body, location_base, client, || resolve_personal_organization_id(&resolve_state, &username, &resolve_repo)).await
}

async fn handle_delete_personal(State(state): State<DockerState>, Path((username, repo, rest)): Path<(String, String, String)>, user: DockerAuthUser) -> Response {
    let resolve_state = state.clone();
    let resolve_repo = repo.clone();
    dispatch_delete(state, repo, rest, user, || resolve_personal_organization_id(&resolve_state, &username, &resolve_repo)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use axum_extra::headers::authorization::Authorization;
    use axum_extra::headers::HeaderMapExt;
    use artiferris_domain::docker_registry::Digest;
    use artiferris_domain::organization::{Organization, OrganizationSlug, PUBLIC_ORGANIZATION_ID};
    use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort, RepositoryFormat};
    use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    /// What an anonymous read of something private or missing must get: a 401 with the OCI envelope and a challenge
    /// scoped to the repository asked for.
    fn assert_read_challenge(response: &axum::http::Response<Body>, scope: &str) {
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let challenge = response.headers().get(axum::http::header::WWW_AUTHENTICATE).expect("a WWW-Authenticate challenge").to_str().unwrap();
        assert!(challenge.starts_with(r#"Bearer realm=""#), "{challenge}");
        assert!(challenge.contains(&format!(r#"scope="{scope}""#)), "{challenge}");
    }

    use crate::route_test_support::{create_personal_project, issue_test_token, seed_bare_user, seed_named_user_with_active_token, seed_permission, seed_repository, seed_user_with_active_token, test_state};

    /// Marks a repository public through the event store, as `PostgresPackageRepositoryStore`'s tests do.
    async fn mark_repository_public(pool: &sqlx::PgPool, repository_id: Uuid) {
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let (version, _) = store.load(repository_id).await.unwrap();
        store.append(repository_id, version, vec![PackageRepositoryEvent::VisibilityChanged { repository_id, is_public: true }], Uuid::new_v4()).await.unwrap();
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

    fn basic_auth_header(password: &str) -> axum::http::HeaderValue {
        let mut headers = axum::http::HeaderMap::new();
        headers.typed_insert(Authorization::basic("ignored", password));
        headers.get(axum::http::header::AUTHORIZATION).unwrap().clone()
    }

    /// A personal repository's own owner, holding a token whose granted scope's
    /// `granted_repository_id` matches, can push and then pull through `/u/{username}/{repo}/...`.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn fetches_a_manifest_for_a_personal_project(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        let push_token = issue_test_token(&state, alice_id, repo_id, "my-image", "myimage", &["push", "pull"]);
        let app = crate::router(state);

        let config_bytes = b"personal-project-config-bytes";
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

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/alice/my-image/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(get_response.status(), StatusCode::OK);
        let returned = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), body);
    }

    /// An unknown personal project, or someone else's, must 404 like the organization route, without reaching
    /// `require_granted_action`.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_request_for_an_unknown_personal_project_is_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let token = state.token_issuer.issue(seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, Uuid::new_v4(), false, None).unwrap();
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/nobody/does-not-exist/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Resolving a personal repository must not grant push by itself, and a denied caller gets the same 404 as a
    /// missing repository, not a 403 confirming alice's project exists.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pushing_to_a_personal_project_without_push_scope_is_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        let pull_only_token = issue_test_token(&state, alice_id, repo_id, "my-image", "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/u/alice/my-image/myimage/blobs/uploads/")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_only_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// A real stranger, with no grant at all rather than a narrower scope, must not tell alice's private project from a
    /// missing one.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_stranger_with_no_grant_cannot_reach_someone_elses_personal_project(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;

        let other_org_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: other_org_id,
                slug: OrganizationSlug::parse("other-corp").unwrap(),
                display_name: "Other Corp".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let mallory_id = seed_user_with_active_token(&pool, other_org_id, "mallory-token").await;
        let mallory_token = state.token_issuer.issue(mallory_id, other_org_id, false, None).unwrap();
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/alice/my-image/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {mallory_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// A personal project that exists but isn't docker-format is unreachable through any docker
    /// route, same as the org-based case.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_non_docker_personal_project_is_not_found_through_docker_routes(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib", RepositoryFormat::Npm).await;
        let token = issue_test_token(&state, alice_id, repo_id, "my-lib", "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/alice/my-lib/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// A real docker client asks for a token scoped to the full `u/{username}/{repo}/{image}` path, unstripped, never a
    /// pre-stripped `{repo}/{image}`. This drives the real `/v2/token` exchange and uses the token on the personal data
    /// routes, proving both match the same bare scope shape.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_real_clients_unstripped_personal_scope_authenticates_end_to_end(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        let app = crate::router(state);

        let token_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/token?scope=repository:u/alice/my-image/myimage:pull,push")
                    .header(axum::http::header::AUTHORIZATION, basic_auth_header("alice-token"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(token_response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(token_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let push_token = json["token"].as_str().unwrap().to_string();

        let config_bytes = b"real-client-config-bytes";
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

        let manifest = manifest_body(&config_digest);
        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/u/alice/my-image/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(manifest.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/alice/my-image/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let returned = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), manifest);
    }

    /// A personal organization's slug is a valid subdomain label, so a personal repository is reachable through the
    /// plain `/{repository}/{*rest}` route too: `require_repository_by_name` must recognize it as personal either way,
    /// so a denied caller gets 404, never a 403.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_personal_org_reached_via_its_own_derived_subdomain_still_remaps_forbidden_to_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        let repo_name = "my-image".to_string();
        let personal_host = format!("u{}.artiferris.localhost", &alice_id.simple().to_string()[..24]);
        let app = crate::router(state.clone());

        let owner_token = issue_test_token(&state, alice_id, repo_id, &repo_name, "myimage", &["pull"]);
        let owner_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/tags/list"))
                    .header("host", &personal_host)
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {owner_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(owner_response.status(), StatusCode::OK, "sanity check: the owner must still be able to reach her own project this way");

        let other_org_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: other_org_id,
                slug: OrganizationSlug::parse("other-corp").unwrap(),
                display_name: "Other Corp".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let mallory_id = seed_user_with_active_token(&pool, other_org_id, "mallory-token").await;
        let mallory_token = state.token_issuer.issue(mallory_id, other_org_id, false, None).unwrap();

        let stranger_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/tags/list"))
                    .header("host", &personal_host)
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {mallory_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            stranger_response.status(),
            StatusCode::NOT_FOUND,
            "a denied caller reaching a personal repo via its derived subdomain must get 404, not the 403 a real org's boundary would produce"
        );
    }

    /// The `Some(_) => METHOD_NOT_ALLOWED` arm must not depend on whether the named personal repository exists: that
    /// lookup happens only inside a correctly shaped operation's arm. A GET that parses to a write-shaped operation
    /// must 405 the same whether the repository is real (and the caller has no grant) or not.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_wrongly_shaped_personal_request_is_not_an_existence_oracle(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        create_personal_project(&pool, alice_id, "secret-lib", RepositoryFormat::Docker).await;

        let other_org_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: other_org_id,
                slug: OrganizationSlug::parse("other-corp-2").unwrap(),
                display_name: "Other Corp 2".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let mallory_id = seed_user_with_active_token(&pool, other_org_id, "mallory-token-2").await;
        let mallory_token = state.token_issuer.issue(mallory_id, other_org_id, false, None).unwrap();
        let app = crate::router(state);

        let existing_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/alice/secret-lib/myimage/blobs/uploads/")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {mallory_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let nonexistent_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/u/alice/does-not-exist/myimage/blobs/uploads/")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {mallory_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(existing_response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            existing_response.status(),
            nonexistent_response.status(),
            "a real, private personal repo and a nonexistent one must be indistinguishable for a wrongly-shaped request"
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_pull_a_manifest_from_a_public_organization_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        mark_repository_public(&pool, repository_id).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let config_bytes = b"public-org-repo-config-bytes";
        let config_digest = Digest::of(config_bytes);
        let blob_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
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
            .oneshot(Request::builder().method("GET").uri(format!("/{repo_name}/myimage/manifests/latest")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(get_response.status(), StatusCode::OK);
        let returned = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), body);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_pull_from_a_private_organization_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let config_bytes = b"private-org-repo-config-bytes";
        let config_digest = Digest::of(config_bytes);
        let blob_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blob_response.status(), StatusCode::CREATED);

        let put_response = app
            .clone()
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
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let get_response = app
            .oneshot(Request::builder().method("GET").uri(format!("/{repo_name}/myimage/manifests/latest")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_read_challenge(&get_response, &format!("repository:{repo_name}/myimage:pull"));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_pull_a_manifest_from_a_public_personal_project(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        mark_repository_public(&pool, repo_id).await;
        let push_token = issue_test_token(&state, alice_id, repo_id, "my-image", "myimage", &["push"]);
        let app = crate::router(state);

        let config_bytes = b"public-personal-project-config-bytes";
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

        let get_response =
            app.oneshot(Request::builder().method("GET").uri("/u/alice/my-image/myimage/manifests/latest").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(get_response.status(), StatusCode::OK);
        let returned = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), body);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_pull_from_a_private_personal_project(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        let push_token = issue_test_token(&state, alice_id, repo_id, "my-image", "myimage", &["push"]);
        let app = crate::router(state);

        let config_bytes = b"private-personal-project-config-bytes";
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

        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/u/alice/my-image/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(manifest_body(&config_digest)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let get_response =
            app.oneshot(Request::builder().method("GET").uri("/u/alice/my-image/myimage/manifests/latest").body(Body::empty()).unwrap()).await.unwrap();

        assert_read_challenge(&get_response, "repository:u/alice/my-image/myimage:pull");
    }

    /// The anonymous tests above pin that the public check fires before `user.ok_or`. This pins that it fires before
    /// `resolve_is_personal` too: an authenticated caller from another organization with no grant must still read a
    /// public repository. Reordering the checks would keep the other tests green while silently 404-ing this caller.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_authenticated_stranger_with_no_grant_can_still_pull_a_manifest_from_a_public_organization_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        mark_repository_public(&pool, repository_id).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state.clone());

        let config_bytes = b"public-org-repo-authenticated-stranger-config-bytes";
        let config_digest = Digest::of(config_bytes);
        let blob_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
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
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let other_org_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: other_org_id,
                slug: OrganizationSlug::parse("stranger-corp").unwrap(),
                display_name: "Stranger Corp".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let stranger_id = seed_user_with_active_token(&pool, other_org_id, "stranger-token").await;
        let stranger_token = state.token_issuer.issue(stranger_id, other_org_id, false, None).unwrap();

        let get_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {stranger_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(get_response.status(), StatusCode::OK);
        let returned = axum::body::to_bytes(get_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), body);
    }

    /// `is_public` only unlocks reads: a push still needs a real, authenticated caller, even against a public
    /// repository. The single highest-value guard of the feature.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_public_organization_repository_still_requires_authentication_for_a_push(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        mark_repository_public(&pool, repository_id).await;
        let app = crate::router(state);
        let body = manifest_body(&Digest::of(b"unused-config-bytes"));

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "a public repository unlocks reads only — writes still require a real, authenticated caller");
    }

    /// End to end: a real Docker client's handshake (challenge, `/token` with no Basic credentials, present the token)
    /// must succeed against a public repository, not only a request with no `Authorization` header.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_docker_client_can_pull_a_manifest_from_a_public_organization_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        mark_repository_public(&pool, repository_id).await;
        let owner_id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "owner-token").await;
        let push_token = issue_test_token(&state, owner_id, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);

        let config_bytes = b"anonymous-pull-config-bytes";
        let config_digest = Digest::of(config_bytes);
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = manifest_body(&config_digest);
        app.clone()
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

        let token_response =
            app.clone().oneshot(Request::builder().uri("/token").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(token_response.status(), StatusCode::OK);
        let token_body = axum::body::to_bytes(token_response.into_body(), usize::MAX).await.unwrap();
        let token_json: serde_json::Value = serde_json::from_slice(&token_body).unwrap();
        let anonymous_token = token_json["token"].as_str().unwrap();

        let pull_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {anonymous_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(pull_response.status(), StatusCode::OK);
        let returned = axum::body::to_bytes(pull_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(returned.to_vec(), body);
    }

    /// The anonymous token must degrade like a missing `Authorization` header: a challenge, never a 403 that would
    /// reveal the repository.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_docker_client_pulling_a_private_repository_gets_a_challenge(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        let app = crate::router(state);

        let token_response =
            app.clone().oneshot(Request::builder().uri("/token").body(Body::empty()).unwrap()).await.unwrap();
        let token_body = axum::body::to_bytes(token_response.into_body(), usize::MAX).await.unwrap();
        let token_json: serde_json::Value = serde_json::from_slice(&token_body).unwrap();
        let anonymous_token = token_json["token"].as_str().unwrap();

        let pull_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {anonymous_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_read_challenge(&pull_response, &format!("repository:{repo_name}/myimage:pull"));
    }

    /// A write still needs a real, authenticated caller: an anonymous token is rejected like a missing header.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_docker_client_cannot_push_even_to_a_public_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        mark_repository_public(&pool, repository_id).await;
        let app = crate::router(state);

        let token_response =
            app.clone().oneshot(Request::builder().uri("/token").body(Body::empty()).unwrap()).await.unwrap();
        let token_body = axum::body::to_bytes(token_response.into_body(), usize::MAX).await.unwrap();
        let token_json: serde_json::Value = serde_json::from_slice(&token_body).unwrap();
        let anonymous_token = token_json["token"].as_str().unwrap();
        let body = manifest_body(&Digest::of(b"anonymous-push-attempt-config-bytes"));

        let push_response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {anonymous_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(push_response.status(), StatusCode::UNAUTHORIZED);
    }

    /// Pushes `myimage:latest` to a public hosted repository, sends one request for `reference`, and returns what was counted.
    async fn counted_pulls(pool: sqlx::PgPool, method: &str, reference: &str) -> Vec<artiferris_domain::download_stats::DownloadCount> {
        let dir = tempfile::tempdir().unwrap();
        let buffer = Arc::new(artiferris_application::download_counter::DownloadCounterBuffer::new());
        let state = crate::state::DockerState { downloads: buffer.clone(), ..test_state(pool.clone(), dir.path()).await };
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        mark_repository_public(&pool, repository_id).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let config_bytes = b"counted-pull-config-bytes";
        let config_digest = Digest::of(config_bytes);
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = manifest_body(&config_digest);
        let pushed = app
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
        assert_eq!(pushed.status(), StatusCode::CREATED);
        assert!(buffer.drain().is_empty(), "pushing is not a download");
        let reference = reference.replace("<digest>", Digest::of(&body).as_str());

        let response = app.oneshot(Request::builder().method(method).uri(format!("/{repo_name}/myimage/manifests/{reference}")).body(Body::empty()).unwrap()).await.unwrap();
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        buffer.drain()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pulling_an_image_by_tag_from_a_hosted_repository_is_counted_once(pool: sqlx::PgPool) {
        let counted = counted_pulls(pool, "GET", "latest").await;

        assert_eq!(counted.len(), 1);
        assert_eq!((counted[0].name.as_str(), counted[0].format, counted[0].downloads), ("myimage", RepositoryFormat::Docker, 1));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_head_probe_of_a_manifest_is_not_a_pull(pool: sqlx::PgPool) {
        assert!(counted_pulls(pool, "HEAD", "latest").await.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn fetching_a_manifest_by_digest_is_not_counted_again(pool: sqlx::PgPool) {
        assert!(counted_pulls(pool, "GET", "<digest>").await.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_tag_that_does_not_exist_is_not_counted(pool: sqlx::PgPool) {
        assert!(counted_pulls(pool, "GET", "nope").await.is_empty());
    }

    /// A real `docker push` is POST, PATCH to the returned `Location`, then PUT: each must stay under `/u/{username}/{repo}`.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_chunked_push_to_a_personal_repository_follows_its_location_headers(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        let token = issue_test_token(&state, alice_id, repo_id, "my-image", "myimage", &["push", "pull"]);
        let app = crate::router(state);
        let content = b"personal-layer-bytes".to_vec();
        let digest = Digest::of(&content);
        let authorized = |method: &str, uri: &str, body: Vec<u8>| {
            Request::builder().method(method).uri(uri.trim_start_matches("/v2")).header(axum::http::header::AUTHORIZATION, format!("Bearer {token}")).body(Body::from(body)).unwrap()
        };

        let started = app.clone().oneshot(authorized("POST", "/v2/u/alice/my-image/myimage/blobs/uploads/", Vec::new())).await.unwrap();
        assert_eq!(started.status(), StatusCode::ACCEPTED);
        let upload_location = started.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string();
        assert!(upload_location.starts_with("/v2/u/alice/my-image/myimage/blobs/uploads/"), "got {upload_location}");

        let patched = app.clone().oneshot(authorized("PATCH", &upload_location, content[..8].to_vec())).await.unwrap();
        assert_eq!(patched.status(), StatusCode::ACCEPTED, "the PATCH must reach the personal route, not the org route");
        let patch_location = patched.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string();
        assert_eq!(patch_location, upload_location);

        let finished = app.clone().oneshot(authorized("PUT", &format!("{patch_location}?digest={}", digest.as_str()), content[8..].to_vec())).await.unwrap();
        assert_eq!(finished.status(), StatusCode::CREATED);
        let blob_location = finished.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string();
        assert_eq!(blob_location, format!("/v2/u/alice/my-image/myimage/blobs/{}", digest.as_str()));

        let pulled = app.clone().oneshot(authorized("GET", &blob_location, Vec::new())).await.unwrap();
        assert_eq!(pulled.status(), StatusCode::OK);
        assert_eq!(axum::body::to_bytes(pulled.into_body(), usize::MAX).await.unwrap().to_vec(), content);

        let manifest = manifest_body(&digest);
        let pushed = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/u/alice/my-image/myimage/manifests/latest")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(manifest.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pushed.status(), StatusCode::CREATED);
        assert_eq!(
            pushed.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap(),
            format!("/v2/u/alice/my-image/myimage/manifests/{}", Digest::of(&manifest).as_str())
        );
    }

    /// Pulls `myimage:latest` once per `(peer, x-forwarded-for)` and returns how many pulls were counted.
    async fn counted_pulls_from(pool: sqlx::PgPool, trusted: &[&str], clients: &[([u8; 4], Option<&str>)]) -> i64 {
        let dir = tempfile::tempdir().unwrap();
        let buffer = Arc::new(artiferris_application::download_counter::DownloadCounterBuffer::new());
        let guard = artiferris_application::request_guard::RequestGuard::new(Arc::new(artiferris_application::client_ip::TrustedProxies::parse(trusted.iter().copied()).unwrap()));
        let state = crate::state::DockerState { downloads: buffer.clone(), guard: Arc::new(guard), ..test_state(pool.clone(), dir.path()).await };
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        mark_repository_public(&pool, repository_id).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let app = crate::router(state);
        let config_bytes = b"dedupe-config-bytes";
        let config_digest = Digest::of(config_bytes);
        let uploaded = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(uploaded.status(), StatusCode::CREATED);
        let pushed = app
            .clone()
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
        assert_eq!(pushed.status(), StatusCode::CREATED);

        for (peer, forwarded_for) in clients {
            let mut request = Request::builder().method("GET").uri(format!("/{repo_name}/myimage/manifests/latest")).body(Body::empty()).unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((*peer, 4000))));
            if let Some(forwarded_for) = forwarded_for {
                request.headers_mut().insert("x-forwarded-for", forwarded_for.parse().unwrap());
            }
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        buffer.drain().iter().map(|count| count.downloads).sum()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn pulling_the_same_image_over_and_over_from_one_client_counts_once(pool: sqlx::PgPool) {
        let counted = counted_pulls_from(pool, &[], &[([203, 0, 113, 7], None); 5]).await;

        assert_eq!(counted, 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn different_clients_each_count_once(pool: sqlx::PgPool) {
        let counted = counted_pulls_from(pool, &[], &[([203, 0, 113, 7], None), ([203, 0, 113, 8], None), ([203, 0, 113, 7], None)]).await;

        assert_eq!(counted, 2);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn clients_behind_a_trusted_proxy_are_counted_by_their_forwarded_address(pool: sqlx::PgPool) {
        let counted = counted_pulls_from(
            pool,
            &["10.0.0.0/8"],
            &[([10, 0, 0, 5], Some("198.51.100.1")), ([10, 0, 0, 5], Some("198.51.100.2")), ([10, 0, 0, 5], Some("198.51.100.1"))],
        )
        .await;

        assert_eq!(counted, 2);
    }

    fn anonymous(method: &str, uri: &str, host: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(host) = host {
            builder = builder.header(axum::http::header::HOST, host);
        }
        builder.body(Body::empty()).unwrap()
    }

    /// A private repository and a missing one get the same answer, whatever is read from them, so an anonymous caller
    /// cannot tell which repositories exist.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_read_of_a_missing_repository_is_answered_exactly_like_a_private_one(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let private = format!("repo-{repository_id}");
        let app = crate::router(state);
        let digest = Digest::of(b"anything").as_str().to_string();

        for repository in [private.as_str(), "missing-repo"] {
            for (method, rest) in [
                ("GET", "myimage/manifests/latest".to_string()),
                ("HEAD", "myimage/manifests/latest".to_string()),
                ("GET", format!("myimage/blobs/{digest}")),
                ("HEAD", format!("myimage/blobs/{digest}")),
                ("GET", "myimage/tags/list".to_string()),
            ] {
                let response = app.clone().oneshot(anonymous(method, &format!("/{repository}/{rest}"), None)).await.unwrap();
                assert_read_challenge(&response, &format!("repository:{repository}/myimage:pull"));
            }
        }

        let private_body = axum::body::to_bytes(
            app.clone().oneshot(anonymous("GET", &format!("/{private}/myimage/manifests/latest"), None)).await.unwrap().into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        let missing_body = axum::body::to_bytes(
            app.oneshot(anonymous("GET", "/missing-repo/myimage/manifests/latest", None)).await.unwrap().into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        assert_eq!(private_body, missing_body);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_read_through_an_unknown_organization_or_personal_project_is_challenged(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let app = crate::router(state);

        let unknown_organization =
            app.clone().oneshot(anonymous("GET", "/some-repo/myimage/manifests/latest", Some("ghost.artiferris.localhost"))).await.unwrap();
        assert_read_challenge(&unknown_organization, "repository:some-repo/myimage:pull");

        let unknown_personal = app.oneshot(anonymous("GET", "/u/nobody/nothing/myimage/manifests/latest", None)).await.unwrap();
        assert_read_challenge(&unknown_personal, "repository:u/nobody/nothing/myimage:pull");
    }

    /// containerd, the runtime of a Kubernetes node, asks for the image straight away, and only sends the pull secret
    /// once the registry challenges it. Before, a private image answered 404 and the node gave up.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_client_that_only_sends_its_credentials_after_a_challenge_can_pull_a_private_image(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let reader = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "pull-secret-token").await;
        seed_permission(&pool, reader, repository_id, "read").await;
        let app = crate::router(state);

        let config_bytes = b"challenge-flow-config-bytes";
        let config_digest = Digest::of(config_bytes);
        let blob = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", config_digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blob.status(), StatusCode::CREATED);
        let body = manifest_body(&config_digest);
        let put = app
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
        assert_eq!(put.status(), StatusCode::CREATED);

        // 1. No credentials yet: the challenge says where to get a token, and for what.
        let first = app.clone().oneshot(anonymous("GET", &format!("/{repo_name}/myimage/manifests/latest"), None)).await.unwrap();
        assert_read_challenge(&first, &format!("repository:{repo_name}/myimage:pull"));
        let challenge = first.headers().get(axum::http::header::WWW_AUTHENTICATE).unwrap().to_str().unwrap().to_string();
        let param = |name: &str| challenge.split(&format!(r#"{name}=""#)).nth(1).and_then(|rest| rest.split('"').next()).unwrap().to_string();
        assert!(param("realm").ends_with("/v2/token"), "{challenge}");

        // 2. The pull secret, exchanged at the realm for exactly what the challenge asked for.
        let query: String = form_urlencoded::Serializer::new(String::new()).append_pair("service", &param("service")).append_pair("scope", &param("scope")).finish();
        let mut basic = axum::http::HeaderMap::new();
        basic.typed_insert(Authorization::basic("ignored", "pull-secret-token"));
        let token_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/token?{query}"))
                    .header(axum::http::header::AUTHORIZATION, basic.get(axum::http::header::AUTHORIZATION).unwrap().clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(token_response.status(), StatusCode::OK);
        let token_json: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(token_response.into_body(), usize::MAX).await.unwrap()).unwrap();
        let token = token_json["token"].as_str().unwrap();

        // 3. The retry with that token gets the image.
        let pulled = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/manifests/latest"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pulled.status(), StatusCode::OK);
        assert_eq!(axum::body::to_bytes(pulled.into_body(), usize::MAX).await.unwrap().to_vec(), body);
    }
}
