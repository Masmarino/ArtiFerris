use axum::Json;
use axum::RequestPartsExt;
use axum::extract::{FromRequestParts, OptionalFromRequestParts, Path};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::{Authorization, HeaderMapExt, authorization::Bearer};
use artiferris_domain::docker_registry::DockerGrantedScope;
use serde_json::json;
use uuid::Uuid;

use crate::errors::www_authenticate_challenge;
use crate::state::DockerState;

#[derive(Clone)]
pub struct DockerAuthUser {
    pub user_id: Uuid,
    /// Snapshot as of token issuance, not re-checked live per request.
    pub organization_id: Uuid,
    pub is_super_admin: bool,
    pub granted_scope: Option<DockerGrantedScope>,
}

/// `None` for a route with no `:repository`/`*rest` params (e.g. `/_catalog`) — falls back to the unscoped challenge, same as `GET /v2/`.
/// Also tries the personal route's 3-segment shape, keeping the hint unstripped — the client's own request form.
async fn scope_hint(parts: &mut Parts, state: &DockerState) -> Option<String> {
    let org_route: Option<Path<(String, String)>> = parts.extract_with_state(state).await.ok();
    if let Some(Path((repository, rest))) = org_route {
        let image_name = crate::routes::path::parse_operation(&rest)?.image_name().to_string();
        return Some(format!("repository:{repository}/{image_name}:pull,push"));
    }
    let Path((username, repo, rest)): Path<(String, String, String)> = parts.extract_with_state(state).await.ok()?;
    let image_name = crate::routes::path::parse_operation(&rest)?.image_name().to_string();
    Some(format!("repository:u/{username}/{repo}/{image_name}:pull,push"))
}

/// Always challenges with the combined `pull,push` scope — `docker push` relies on this for its first, unauthenticated request. Safe: it's an upper bound, narrowed later by `IssueDockerAccessTokenUseCase`.
fn unauthorized(state: &DockerState, host: &str, scope: Option<&str>) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, www_authenticate_challenge(state, host, scope))],
        Json(json!({ "errors": [{ "code": "UNAUTHORIZED", "message": "authentication required" }] })),
    )
        .into_response()
}

impl FromRequestParts<DockerState> for DockerAuthUser {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &DockerState) -> Result<Self, Self::Rejection> {
        // Must run before the Bearer extraction below consumes `parts`.
        let scope = scope_hint(parts, state).await;
        let host = state.host_header(&parts.headers).to_string();

        let TypedHeader(Authorization(bearer)) =
            parts.extract::<TypedHeader<Authorization<Bearer>>>().await.map_err(|_| unauthorized(state, &host, scope.as_deref()))?;
        let claims = state.token_issuer.verify(bearer.token()).map_err(|_| unauthorized(state, &host, scope.as_deref()))?;

        // A valid signature and a live `exp` only prove the token was minted; they say nothing about
        // whether the user has since been deactivated or had their tokens rotated. Cached, so this
        // costs a query at most once per user per cache TTL rather than once per request (M-17).
        let still_valid = state
            .tokens_valid_after_cache
            .is_valid(&state.users, claims.user_id, claims.issued_at)
            .await
            .map_err(|_| unauthorized(state, &host, scope.as_deref()))?;
        if !still_valid {
            return Err(unauthorized(state, &host, scope.as_deref()));
        }

        // Revoking the API token this one was exchanged for has to end it too, same cache lifetime.
        if let Some(api_token_id) = claims.api_token_id {
            let source_active = state
                .tokens_valid_after_cache
                .is_api_token_active(api_token_id, state.issue_access_token.api_token_is_active(claims.user_id, api_token_id))
                .await
                .map_err(|_| unauthorized(state, &host, scope.as_deref()))?;
            if !source_active {
                return Err(unauthorized(state, &host, scope.as_deref()));
            }
        }

        Ok(DockerAuthUser {
            user_id: claims.user_id,
            organization_id: claims.organization_id,
            is_super_admin: claims.is_super_admin,
            granted_scope: claims.granted_scope,
        })
    }
}

/// No bearer token, or the placeholder handed to anonymous callers, is anonymous (`None`). A bearer token that was
/// presented but does not check out (expired, forged, revoked) is a 401 challenge instead, so the client fetches a new one.
impl OptionalFromRequestParts<DockerState> for DockerAuthUser {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &DockerState) -> Result<Option<Self>, Self::Rejection> {
        let presented = parts
            .headers
            .typed_get::<Authorization<Bearer>>()
            .is_some_and(|Authorization(bearer)| !bearer.token().is_empty() && bearer.token() != crate::routes::handshake::ANONYMOUS_DOCKER_TOKEN);
        match <Self as FromRequestParts<DockerState>>::from_request_parts(parts, state).await {
            Ok(user) => Ok(Some(user)),
            Err(rejection) if presented => Err(rejection),
            Err(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route_test_support::{seed_bare_user, seed_user_with_active_token, test_state, test_state_with_cache_ttl};
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use artiferris_domain::docker_registry::DockerTokenIssuerPort;
    use artiferris_infrastructure::jwt_docker_token_issuer::JwtDockerTokenIssuer;
    use artiferris_infrastructure::jwt_token_issuer::JwtTokenIssuer;
    use sqlx::PgPool;
    use tower::ServiceExt;

    /// No `:repository`/`*rest` params, so `DockerAuthUser` gets exercised in isolation without `dispatch.rs`'s routing machinery.
    fn router(state: DockerState) -> Router {
        async fn handler(user: DockerAuthUser) -> Json<serde_json::Value> {
            Json(json!({
                "user_id": user.user_id,
                "organization_id": user.organization_id,
                "is_super_admin": user.is_super_admin,
                "granted_scope_name": user.granted_scope.map(|s| s.name),
            }))
        }
        Router::new().route("/", get(handler)).with_state(state)
    }

    async fn request(state: DockerState, auth_header: Option<&str>) -> Response {
        let mut builder = Request::builder().uri("/");
        if let Some(value) = auth_header {
            builder = builder.header(axum::http::header::AUTHORIZATION, value);
        }
        router(state).oneshot(builder.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_valid_docker_access_token_authenticates_and_maps_claims_correctly(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let user_id = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let organization_id = Uuid::new_v4();
        let token = state.token_issuer.issue(user_id, organization_id, true, None).unwrap();

        let response = request(state, Some(&format!("Bearer {token}"))).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["user_id"], user_id.to_string());
        assert_eq!(json["organization_id"], organization_id.to_string());
        assert_eq!(json["is_super_admin"], true);
        assert!(json["granted_scope_name"].is_null());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_valid_token_carries_its_granted_scope_through_the_extractor(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let repository_id = Uuid::new_v4();
        let scope = DockerGrantedScope {
            resource_type: "repository".to_string(),
            name: "myrepo/myimage".to_string(),
            actions: vec!["pull".to_string()],
            granted_repository_id: Some(repository_id),
        };
        let holder = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let token = state.token_issuer.issue(holder, Uuid::new_v4(), false, Some(scope)).unwrap();

        let response = request(state, Some(&format!("Bearer {token}"))).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["granted_scope_name"], "myrepo/myimage");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_missing_authorization_header_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;

        let response = request(state, None).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_malformed_authorization_header_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;

        // Wrong auth scheme entirely — `TypedHeader<Authorization<Bearer>>` extraction fails.
        let response = request(state, Some("Basic dXNlcjpwYXNz")).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_garbage_bearer_token_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;

        // Not a JWT at all — three-part structural decoding fails immediately.
        let response = request(state, Some("Bearer not-a-real-jwt")).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// A structurally well-formed, correctly-signed JWT — just signed with a secret this server doesn't recognize. Must be rejected exactly like garbage.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_token_signed_with_a_different_secret_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await; // route_test_support wires "test-secret"
        // A real, seeded holder — so rejection is attributable to the signature check alone, not
        // conflated with the unknown-holder rejection `DockerAuthUser::from_request_parts` also does (M-17).
        let holder = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let other_issuer = JwtDockerTokenIssuer::new("a-different-secret".to_string());
        let token = other_issuer.issue(holder, Uuid::new_v4(), false, None).unwrap();

        let response = request(state, Some(&format!("Bearer {token}"))).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// A syntactically valid, correctly-shaped token whose signature has been tampered with must not verify.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_token_with_a_tampered_signature_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        // A real, seeded holder — so rejection is attributable to the signature check alone, not
        // conflated with the unknown-holder rejection `DockerAuthUser::from_request_parts` also does (M-17).
        let holder = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let token = state.token_issuer.issue(holder, Uuid::new_v4(), false, None).unwrap();
        let mut tampered = token.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'A' { 'B' } else { 'A' });
        assert_ne!(tampered, token, "the mutation must actually change the token");

        let response = request(state, Some(&format!("Bearer {tampered}"))).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// Both issuers share the same secret, so the `typ` claim is the only thing keeping a session token from being accepted as a Docker access token.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_session_login_token_is_rejected_as_a_docker_access_token(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await; // route_test_support wires "test-secret"
        let session_issuer = JwtTokenIssuer::new("test-secret".to_string());
        let token = artiferris_domain::user::TokenIssuerPort::issue(&session_issuer, Uuid::new_v4(), chrono::Duration::hours(12)).unwrap();

        let response = request(state, Some(&format!("Bearer {token}"))).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// Pushes `tokens_valid_after` to `offset_seconds` away from now. `iat` only has
    /// second granularity, so a bump has to land in a strictly later second than the token's
    /// issuance to be observable at all — hence the explicit offsets rather than a bare `now()`.
    async fn set_tokens_valid_after(pool: &PgPool, user_id: Uuid, offset_seconds: i32) {
        sqlx::query!(
            "UPDATE users SET tokens_valid_after = now() + make_interval(secs => $2) WHERE id = $1",
            user_id,
            f64::from(offset_seconds),
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// The revocation window this whole mechanism exists to close (M-17): the user's tokens were
    /// invalidated after this token was minted, so the token must stop working even though its
    /// signature and `exp` are both still perfectly good.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_token_issued_before_the_users_tokens_valid_after_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let user_id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "irrelevant-token").await;
        set_tokens_valid_after(&pool, user_id, 3600).await;

        let token = state.token_issuer.issue(user_id, Uuid::new_v4(), false, None).unwrap();
        let response = request(state, Some(&format!("Bearer {token}"))).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// The other half of the same check — the common case must not regress into a blanket 401.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_token_issued_after_the_users_tokens_valid_after_is_accepted(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let user_id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "irrelevant-token").await;
        set_tokens_valid_after(&pool, user_id, -3600).await;

        let token = state.token_issuer.issue(user_id, Uuid::new_v4(), false, None).unwrap();
        let response = request(state, Some(&format!("Bearer {token}"))).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// The revocation has to land while the user's entry is already cached — otherwise the test
    /// only proves the cold-miss path, and a warm entry could happily serve a revoked token
    /// forever. A zero TTL makes every check re-read, which is what the 30s TTL does on expiry.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_revocation_takes_effect_once_the_cached_entry_goes_stale(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with_cache_ttl(pool.clone(), dir.path(), std::time::Duration::ZERO).await;
        let user_id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "irrelevant-token").await;
        set_tokens_valid_after(&pool, user_id, -3600).await;
        let token = state.token_issuer.issue(user_id, Uuid::new_v4(), false, None).unwrap();

        // Warms the cache entry for this user.
        let response = request(state.clone(), Some(&format!("Bearer {token}"))).await;
        assert_eq!(response.status(), StatusCode::OK, "the token must work before anything revokes it");

        set_tokens_valid_after(&pool, user_id, 3600).await;

        let response = request(state, Some(&format!("Bearer {token}"))).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "a refreshed entry must reject the now-revoked token");
    }

    async fn exchanged_token(state: &DockerState, plaintext: &str) -> String {
        state.issue_access_token.execute(PUBLIC_ORGANIZATION_ID, plaintext, None).await.unwrap()
    }

    /// The token exchange remembers which API token it came from; revoking that token must end the access token.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_access_token_stops_working_when_its_api_token_is_revoked(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with_cache_ttl(pool.clone(), dir.path(), std::time::Duration::ZERO).await;
        seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "pat-to-revoke").await;
        let token = exchanged_token(&state, "pat-to-revoke").await;
        let response = request(state.clone(), Some(&format!("Bearer {token}"))).await;
        assert_eq!(response.status(), StatusCode::OK, "the token must work before anything revokes it");

        sqlx::query!("UPDATE api_tokens SET revoked_at = now()").execute(&pool).await.unwrap();

        let response = request(state, Some(&format!("Bearer {token}"))).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_access_token_stops_working_when_its_api_token_expires(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with_cache_ttl(pool.clone(), dir.path(), std::time::Duration::ZERO).await;
        seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "pat-to-expire").await;
        let token = exchanged_token(&state, "pat-to-expire").await;

        sqlx::query!("UPDATE api_tokens SET expires_at = now() - interval '1 minute'").execute(&pool).await.unwrap();

        let response = request(state, Some(&format!("Bearer {token}"))).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn revoking_one_api_token_leaves_an_access_token_from_another_alone(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with_cache_ttl(pool.clone(), dir.path(), std::time::Duration::ZERO).await;
        let user_id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "first-pat").await;
        sqlx::query!(
            "INSERT INTO api_tokens (id, user_id, token_hash, label, created_at) VALUES ($1, $2, $3, 'second', now())",
            Uuid::new_v4(),
            user_id,
            artiferris_application::use_cases::api_token::hash_api_token("second-pat"),
        )
        .execute(&pool)
        .await
        .unwrap();
        let kept = exchanged_token(&state, "second-pat").await;

        sqlx::query!("UPDATE api_tokens SET revoked_at = now() WHERE label = 'test'").execute(&pool).await.unwrap();

        let response = request(state, Some(&format!("Bearer {kept}"))).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Fails closed on a deleted account, matching `artiferris-api`'s `AuthUser`: nothing ever bumps
    /// a `tokens_valid_after` for a row that no longer exists, so "unknown" must not read as "fine".
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_token_whose_user_no_longer_exists_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let user_id = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let token = state.token_issuer.issue(user_id, Uuid::new_v4(), false, None).unwrap();
        sqlx::query!("DELETE FROM users WHERE id = $1", user_id).execute(&pool).await.unwrap();

        let response = request(state, Some(&format!("Bearer {token}"))).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// Mirrors `router` above, but mounted at the personal-repository path shape — needed since
    /// `scope_hint`'s original `Path<(String, String)>` attempt can't match a 3-segment route.
    fn personal_router(state: DockerState) -> Router {
        async fn handler(_user: DockerAuthUser) -> StatusCode {
            StatusCode::OK
        }
        Router::new().route("/u/{username}/{repo}/{*rest}", get(handler)).with_state(state)
    }

    /// A client's first, unauthenticated request under `/u/{username}/{repo}/...` must still get
    /// a real scope hint — without it, `docker push` has nothing to ask `/v2/token` for.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_unauthenticated_request_under_a_personal_repository_path_still_gets_a_scope_hint(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;

        let response = personal_router(state)
            .oneshot(Request::builder().uri("/u/alice/my-image/myimage/manifests/latest").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let challenge = response.headers().get(axum::http::header::WWW_AUTHENTICATE).unwrap().to_str().unwrap();
        assert!(challenge.contains(r#"scope="repository:u/alice/my-image/myimage:pull,push""#), "got: {challenge}");
    }

    fn optional_router(state: DockerState) -> Router {
        async fn handler(user: Option<DockerAuthUser>) -> Json<serde_json::Value> {
            Json(json!({ "authenticated": user.is_some() }))
        }
        Router::new().route("/", get(handler)).with_state(state)
    }

    async fn optional_request(state: DockerState, auth_header: Option<&str>) -> Response {
        let mut builder = Request::builder().uri("/");
        if let Some(value) = auth_header {
            builder = builder.header(axum::http::header::AUTHORIZATION, value);
        }
        optional_router(state).oneshot(builder.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_read_without_a_token_or_with_the_anonymous_placeholder_is_anonymous(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;

        for header in [None, Some("Bearer anonymous"), Some("Basic dXNlcjpwYXNz")] {
            let response = optional_request(state.clone(), header).await;
            assert_eq!(response.status(), StatusCode::OK, "{header:?}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_read_with_a_token_that_does_not_check_out_gets_a_challenge_not_an_anonymous_pass(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let unknown_holder = state.token_issuer.issue(Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();
        let expired = JwtDockerTokenIssuer::with_ttl("test-secret".to_string(), chrono::Duration::seconds(-30)).issue(seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, Uuid::new_v4(), false, None).unwrap();

        for header in ["Bearer not-a-real-jwt".to_string(), format!("Bearer {unknown_holder}"), format!("Bearer {expired}")] {
            let response = optional_request(state.clone(), Some(&header)).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{header}");
            let challenge = response.headers().get(axum::http::header::WWW_AUTHENTICATE).unwrap().to_str().unwrap();
            assert!(challenge.starts_with("Bearer realm="), "{challenge}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_read_with_a_valid_token_is_authenticated(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let token = state.token_issuer.issue(seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, Uuid::new_v4(), false, None).unwrap();

        let response = optional_request(state, Some(&format!("Bearer {token}"))).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["authenticated"], true);
    }
}
