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

// dispatch_* holds the per-verb match shape once; each handle_*/handle_*_personal wrapper just
// supplies its own resolve_organization_id closure.
//
// The closure returns Result<Uuid, Response>, not StatusCode, so each path keeps its own
// resolution-failure shape: on the org path, a bad Host never even reaches the closure — that's
// ResolvedOrganization's own `Rejection = StatusCode` extractor rejecting the request first — while
// on the personal path the closure itself returns docker_authz_error's JSON-enveloped Response.
//
// resolve_organization_id().await must be called INSIDE the matched parse_operation arm, not before
// it — the personal path's lookup hits the database, so calling it before the match would let an
// unhandled verb/shape leak a private repo's existence via 405-vs-404 (see
// a_wrongly_shaped_personal_request_is_not_an_existence_oracle).

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

async fn handle_get(
    State(state): State<DockerState>,
    Path((repository, rest)): Path<(String, String)>,
    Query(tags_query): Query<TagsQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    resolved_org: ResolvedOrganization,
    user: Option<DockerAuthUser>,
) -> Response {
    let organization_id = resolved_org.0.id;
    let client = client_of(&state, &headers, &connect_info);
    let location_base = format!("/v2/{repository}");
    dispatch_get(state, repository, rest, user, client, tags_query, location_base, || async move { Ok::<Uuid, Response>(organization_id) }).await
}

async fn handle_head(
    State(state): State<DockerState>,
    Path((repository, rest)): Path<(String, String)>,
    Query(tags_query): Query<TagsQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    resolved_org: ResolvedOrganization,
    user: Option<DockerAuthUser>,
) -> Response {
    let organization_id = resolved_org.0.id;
    let client = client_of(&state, &headers, &connect_info);
    let location_base = format!("/v2/{repository}");
    dispatch_head(state, repository, rest, user, client, tags_query, location_base, || async move { Ok::<Uuid, Response>(organization_id) }).await
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

// Personal-repository counterparts of the handlers above: resolved via `resolve_personal_repository`
// instead of the `Host`-header `ResolvedOrganization`, since personal orgs have no subdomain — see
// the `dispatch_*` doc comment above for why that resolution is threaded through as a closure called
// from inside each dispatcher's matched arm.
async fn resolve_personal_organization_id(state: &DockerState, username: &str, repo: &str) -> Result<Uuid, Response> {
    resolve_personal_repository(state, username, repo).await.map(|r| r.organization_id).map_err(docker_authz_error)
}

async fn handle_get_personal(
    State(state): State<DockerState>,
    Path((username, repo, rest)): Path<(String, String, String)>,
    Query(tags_query): Query<TagsQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    user: Option<DockerAuthUser>,
) -> Response {
    let resolve_state = state.clone();
    let resolve_repo = repo.clone();
    let client = client_of(&state, &headers, &connect_info);
    let location_base = format!("/v2/u/{username}/{repo}");
    dispatch_get(state, repo, rest, user, client, tags_query, location_base, || resolve_personal_organization_id(&resolve_state, &username, &resolve_repo)).await
}

/// Only the blob case gets the lightweight `head_blob` treatment; other HEAD requests fall back to the same GET handlers.
async fn handle_head_personal(
    State(state): State<DockerState>,
    Path((username, repo, rest)): Path<(String, String, String)>,
    Query(tags_query): Query<TagsQueryParams>,
    headers: axum::http::HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    user: Option<DockerAuthUser>,
) -> Response {
    let resolve_state = state.clone();
    let resolve_repo = repo.clone();
    let client = client_of(&state, &headers, &connect_info);
    let location_base = format!("/v2/u/{username}/{repo}");
    dispatch_head(state, repo, rest, user, client, tags_query, location_base, || resolve_personal_organization_id(&resolve_state, &username, &resolve_repo)).await
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

    use crate::route_test_support::{create_personal_project, issue_test_token, seed_bare_user, seed_named_user_with_active_token, seed_repository, seed_user_with_active_token, test_state};

    /// `PostgresPackageRepositoryStore`'s own tests seed a `VisibilityChanged` event directly,
    /// since this task's own implementation is the only route that could otherwise flip it.
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

    /// An unknown, or someone else's nonexistent, personal project must 404 — same as the plain
    /// org-based route — without ever reaching `require_granted_action`.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_request_for_an_unknown_personal_project_is_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        // Any valid Bearer token — resolution fails before the granted scope is ever inspected.
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

    /// Resolving the personal repository must not, by itself, grant push access — and a denied
    /// caller must get the same 404 a nonexistent repository would, not a 403 that would confirm
    /// alice's project exists.
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

    /// A genuine stranger — not alice, holding no grant on her project at all, not merely a
    /// narrower scope — must not be able to tell her private personal project apart from one that
    /// doesn't exist.
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
        // A completely unscoped token — a stranger's, not merely one without the right action.
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

    /// The test that was missing before this task's review: a real docker client requests a token
    /// scoped to the FULL `u/{username}/{repo}/{image}` path it pulls/pushes against, unstripped —
    /// not a pre-stripped `{repo}/{image}` scope no real client ever sends. Drives the actual
    /// `/v2/token` exchange, then uses the resulting token against the personal data routes
    /// end-to-end, proving both the exchange and the data-route match on the same bare scope shape.
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

    /// A personal org's slug is a syntactically valid subdomain label, so a caller can reach a
    /// personal repo through the plain `/{repository}/{*rest}` route too, not just `/u/...` —
    /// `require_repository_by_name` must still recognize it as personal either way, so a denied
    /// caller gets 404, never the 403 a real org's own members would see.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_personal_org_reached_via_its_own_derived_subdomain_still_remaps_forbidden_to_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        let repo_name = "my-image".to_string();
        let personal_host = format!("u{}.artiferris.localhost", &alice_id.simple().to_string()[..24]);
        let app = crate::router(state.clone());

        // The owner herself, holding a matching granted scope, can still reach it this way.
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

        // A stranger, holding a completely unscoped token, must not learn it exists.
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

    /// The `Some(_) => METHOD_NOT_ALLOWED` arm must never depend on whether the named personal repo
    /// actually exists — that lookup only happens inside a correctly-shaped operation's own arm, same
    /// as the org-route handlers. A GET whose path parses to a write-shaped operation must 405
    /// identically whether the repo is real (and the caller has no grant on it) or doesn't exist.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_wrongly_shaped_personal_request_is_not_an_existence_oracle(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        create_personal_project(&pool, alice_id, "secret-lib", RepositoryFormat::Docker).await;

        // A stranger, holding a completely unscoped token — no grant on alice's project at all.
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

        // "myimage/blobs/uploads/" parses to BlobUploadStart — a write-shaped operation GET never handles.
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

        // No Authorization header at all — an anonymous pull against a public repository.
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

        // Repository stays private — same request, still no Authorization header.
        let get_response = app
            .oneshot(Request::builder().method("GET").uri(format!("/{repo_name}/myimage/manifests/latest")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
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

        // No Authorization header, reached via /u/{username}/{repo}/... rather than an org subdomain.
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

        // Project stays private — same request, still no Authorization header.
        let get_response =
            app.oneshot(Request::builder().method("GET").uri("/u/alice/my-image/myimage/manifests/latest").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
    }

    /// The anonymous tests above only pin that `require_readable_repository_by_name`'s public
    /// check fires before `user.ok_or`. This pins that it fires before `resolve_is_personal` too —
    /// a real, authenticated caller in a completely different organization, holding no grant on
    /// this repository at all, must still be able to read a public one. A refactor that reordered
    /// the public check after `resolve_is_personal` would keep every other test green while
    /// silently 404-ing this caller.
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

        // A real, authenticated stranger — a different organization, no grant on this repository at all.
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

    /// `is_public` only unlocks reads. A push must still require a real, authenticated caller even
    /// against a public repository — the type system makes an accidental regression here unlikely
    /// (a write handler would have to be deliberately changed to `Option<DockerAuthUser>`), but
    /// this is the single highest-value guard for the whole feature.
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

        // No Authorization header at all — same discipline as `DockerAuthUser`'s own
        // `a_missing_authorization_header_is_rejected` test.
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

    /// End-to-end for #73: a real Docker client's own handshake (challenge → `/token` with no
    /// Basic credentials → present the returned token) must succeed against a public repository,
    /// not just a request with no `Authorization` header at all.
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

        // The real handshake: no Basic credentials at all.
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

    /// The anonymous token must degrade exactly like a fully missing `Authorization` header — a
    /// `404`, never a `403` (which would leak that the repository exists at all).
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_docker_client_pulling_a_private_repository_gets_not_found(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        // Deliberately not marked public.
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

        assert_eq!(pull_response.status(), StatusCode::NOT_FOUND);
    }

    /// A write still requires a real, authenticated caller — an anonymous token must be rejected
    /// exactly like a missing `Authorization` header, never silently accepted as "some user".
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
}
