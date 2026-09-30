use std::net::SocketAddr;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, RawQuery, State};
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Basic;
use artiferris_application::client_ip::throttle_bucket;
use artiferris_application::error::ApplicationError;
use artiferris_domain::docker_registry::DOCKER_ACCESS_TOKEN_TTL_SECONDS;
use serde_json::json;

use crate::auth::DockerAuthUser;
use crate::errors::{docker_error, docker_error_response, www_authenticate_challenge};
use crate::organization_resolution::ResolvedOrganization;
use crate::state::DockerState;

/// Returned for `/token` requests without Basic credentials, instead of `401`: a standard Docker client caches the
/// `WWW-Authenticate` challenge from `GET /v2/` and always exchanges it for a token first, even for a repository it can
/// read anonymously. It is deliberately not a verifiable token (not signed by `state.token_issuer`), so
/// `DockerTokenIssuerPort::verify` rejects it like any non-JWT bearer, the same path as a missing header:
/// `Option<DockerAuthUser>` degrades to `None` (public served, private 404), and non-optional `DockerAuthUser` (writes,
/// `/_catalog`) still 401s.
pub(crate) const ANONYMOUS_DOCKER_TOKEN: &str = "anonymous";

pub fn router() -> Router<DockerState> {
    Router::new().route("/", get(check_version)).route("/token", get(issue_token))
}

// Routed through `Option<DockerAuthUser>` rather than a raw `verify()`: a bare signature check would accept a valid
// token whose holder was deactivated or deleted, or issued before `tokens_valid_after`. This probe carries no data, but
// it should not be the one place a revoked token reads as authenticated.
async fn check_version(State(state): State<DockerState>, headers: axum::http::HeaderMap, user: Option<DockerAuthUser>) -> Response {
    if user.is_some() {
        (StatusCode::OK, [(HeaderName::from_static("docker-distribution-api-version"), "registry/2.0")], Json(json!({}))).into_response()
    } else {
        let host = state.host_header(&headers);
        (
            StatusCode::UNAUTHORIZED,
            [(axum::http::header::WWW_AUTHENTICATE, www_authenticate_challenge(&state, host, None))],
            Json(json!({ "errors": [{ "code": "UNAUTHORIZED", "message": "authentication required" }] })),
        )
            .into_response()
    }
}

// Real clients can send more than one `?scope=` param, but `axum::extract::Query` 400s on repeated keys — parse the raw query string instead.
fn scope_values(raw_query: Option<&str>) -> Vec<String> {
    let Some(raw_query) = raw_query else { return Vec::new() };
    form_urlencoded::parse(raw_query.as_bytes()).filter(|(key, _)| key == "scope").map(|(_, value)| value.into_owned()).collect()
}

/// Unions actions for scopes naming the same resource; keeps only the first distinct resource if more than one is requested.
fn merge_scopes(raw_scopes: &[String]) -> Option<String> {
    let mut merged: Option<artiferris_domain::docker_registry::DockerScopeRequest> = None;
    for raw in raw_scopes {
        let Some(parsed) = artiferris_domain::docker_registry::DockerScopeRequest::parse(raw) else { continue };
        match &mut merged {
            None => merged = Some(parsed),
            Some(existing) if existing.resource_type == parsed.resource_type && existing.name == parsed.name => {
                for action in parsed.actions {
                    if !existing.actions.contains(&action) {
                        existing.actions.push(action);
                    }
                }
            }
            Some(_) => {}
        }
    }
    merged.map(|s| format!("{}:{}:{}", s.resource_type, s.name, s.actions.join(",")))
}

fn token_body(token: &str) -> serde_json::Value {
    json!({ "token": token, "access_token": token, "expires_in": DOCKER_ACCESS_TOKEN_TTL_SECONDS, "issued_at": chrono::Utc::now().to_rfc3339() })
}

async fn issue_token(
    State(state): State<DockerState>,
    headers: axum::http::HeaderMap,
    resolved_org: ResolvedOrganization,
    RawQuery(raw_query): RawQuery,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    basic_auth: Option<TypedHeader<Authorization<Basic>>>,
) -> Response {
    let merged_scope = merge_scopes(&scope_values(raw_query.as_deref()));
    let Some(TypedHeader(Authorization(basic))) = basic_auth else {
        return Json(token_body(ANONYMOUS_DOCKER_TOKEN)).into_response();
    };

    // Basic auth carries no reliable username here (the use case checks only the password, an API token), so the client
    // address is the only throttle key.
    let forwarded: Vec<&str> = headers.get_all("x-forwarded-for").iter().filter_map(|value| value.to_str().ok()).collect();
    let ip = state.guard.client_address(connect_info.ok().map(|ConnectInfo(addr)| addr.ip()), &forwarded);
    let throttle_key = format!("docker-token:{}", throttle_bucket(&ip));
    if !state.login_throttle.reserve(&throttle_key, artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS, artiferris_application::login_throttle::LOGIN_ATTEMPT_WINDOW) {
        return docker_error(StatusCode::TOO_MANY_REQUESTS, "TOOMANYREQUESTS", "too many failed token requests, try again later").into_response();
    }

    match state.issue_access_token.execute(resolved_org.0.id, basic.password(), merged_scope.as_deref()).await {
        Ok(token) => {
            state.login_throttle.release(&throttle_key);
            Json(token_body(&token)).into_response()
        }
        Err(ApplicationError::InactiveApiToken) => {
            // A real but stale token: retrying it isn't guessing, so it costs the token's own budget (which also caps its audit rows), not the address's.
            state.login_throttle.release(&throttle_key);
            let stale_token_key = format!("docker-token-stale:{}", artiferris_application::use_cases::api_token::hash_api_token(basic.password()));
            if state.login_throttle.reserve(&stale_token_key, artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS, artiferris_application::login_throttle::LOGIN_ATTEMPT_WINDOW) {
                if let Err(e) = state.record_security_event.execute(artiferris_domain::audit::SecurityEvent::DockerTokenFailed { ip }, None).await {
                    tracing::warn!("failed to record security event: {e}");
                }
            }
            docker_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "invalid credentials").into_response()
        }
        Err(ApplicationError::InvalidCredentials) => {
            // Not `let _ = ...`: the silent-discard pattern was fixed in artiferris-api and must not come back in this
            // crate.
            if let Err(e) = state.record_security_event.execute(artiferris_domain::audit::SecurityEvent::DockerTokenFailed { ip }, None).await {
                tracing::warn!("failed to record security event: {e}");
            }
            docker_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "invalid credentials").into_response()
        }
        Err(e) => {
            state.login_throttle.release(&throttle_key);
            docker_error_response(e).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_application::client_ip::TrustedProxies;
    use artiferris_application::request_guard::RequestGuard;
    use axum::body::Body;
    use axum::http::Request;
    use axum_extra::headers::HeaderMapExt;
    use std::sync::Arc;
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::route_test_support::{seed_bare_user, seed_organization_admin_with_active_token, seed_permission, seed_repository, seed_user_with_active_token, test_state};

    fn basic_auth_header(password: &str) -> axum::http::HeaderValue {
        let mut headers = axum::http::HeaderMap::new();
        headers.typed_insert(Authorization::basic("ignored", password));
        headers.get(axum::http::header::AUTHORIZATION).unwrap().clone()
    }

    fn token_request(auth_header: axum::http::HeaderValue) -> Request<Body> {
        Request::builder().uri("/token").header(axum::http::header::AUTHORIZATION, auth_header).body(Body::empty()).unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn version_check_without_a_token_challenges_with_bearer(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::router(test_state(pool, dir.path()).await);
        let response = app.oneshot(Request::builder().uri("/").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let challenge = response.headers().get(axum::http::header::WWW_AUTHENTICATE).unwrap().to_str().unwrap();
        assert!(challenge.starts_with("Bearer "));
        assert!(challenge.contains("realm="));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_challenge_realm_matches_the_organization_subdomain_the_client_actually_pushed_to(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::router(test_state(pool, dir.path()).await);

        let acme_response = app
            .clone()
            .oneshot(Request::builder().uri("/").header(axum::http::header::HOST, "acme.artiferris.localhost").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let acme_challenge = acme_response.headers().get(axum::http::header::WWW_AUTHENTICATE).unwrap().to_str().unwrap().to_string();

        let other_response = app
            .oneshot(Request::builder().uri("/").header(axum::http::header::HOST, "other.artiferris.localhost").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let other_challenge = other_response.headers().get(axum::http::header::WWW_AUTHENTICATE).unwrap().to_str().unwrap().to_string();

        assert!(acme_challenge.contains(r#"realm="http://acme.artiferris.localhost/v2/token""#), "got: {acme_challenge}");
        assert!(other_challenge.contains(r#"realm="http://other.artiferris.localhost/v2/token""#), "got: {other_challenge}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn version_check_with_a_valid_token_succeeds(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let holder = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let token = state.token_issuer.issue(holder, Uuid::new_v4(), false, None).unwrap();
        let app = crate::router(state);

        let response =
            app.oneshot(Request::builder().uri("/").header(axum::http::header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap())
                .await
                .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn version_check_with_a_token_for_an_unknown_holder_is_rejected(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let token = state.token_issuer.issue(Uuid::new_v4(), Uuid::new_v4(), false, None).unwrap();
        let app = crate::router(state);

        let response =
            app.oneshot(Request::builder().uri("/").header(axum::http::header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap())
                .await
                .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn token_endpoint_without_credentials_issues_an_anonymous_token(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let app = crate::router(state.clone());

        let response = app.oneshot(Request::builder().uri("/token").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = json["token"].as_str().unwrap();
        assert_eq!(token, json["access_token"].as_str().unwrap());
        assert!(state.token_issuer.verify(token).is_err());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn token_endpoint_with_a_wrong_password_is_unauthorized(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::router(test_state(pool, dir.path()).await);
        let response = app
            .oneshot(Request::builder().uri("/token").header(axum::http::header::AUTHORIZATION, basic_auth_header("wrong")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// `/v2/token` had no throttling, and its credential check is a cheap unsalted SHA-256 comparison: an efficient
    /// guessing oracle without it.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn repeated_wrong_password_token_requests_are_eventually_throttled(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let app = crate::router(state);

        for _ in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let response = app.clone().oneshot(token_request(basic_auth_header("wrong-password"))).await.unwrap();
            assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        }

        let response = app.oneshot(token_request(basic_auth_header("wrong-password"))).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn retrying_a_revoked_token_never_uses_up_the_addresss_budget_and_writes_a_bounded_number_of_audit_rows(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "stale-api-token").await;
        sqlx::query("UPDATE api_tokens SET revoked_at = now()").execute(&pool).await.unwrap();
        let app = crate::router(state);
        let attempts = artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS;

        for _ in 0..attempts * 3 {
            let response = app.clone().oneshot(token_request(basic_auth_header("stale-api-token"))).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let guesses_still_have_their_full_budget = app.clone().oneshot(token_request(basic_auth_header("someone-elses-guess"))).await.unwrap();
        assert_eq!(guesses_still_have_their_full_budget.status(), StatusCode::UNAUTHORIZED);
        let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE event_type = 'DockerTokenFailed'").fetch_one(&pool).await.unwrap();
        assert_eq!(audited as usize, attempts + 1, "the stale token's first {attempts} retries plus the one guess");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn wrong_passwords_are_still_throttled_after_stale_token_retries(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "stale-api-token").await;
        sqlx::query("UPDATE api_tokens SET revoked_at = now()").execute(&pool).await.unwrap();
        let app = crate::router(state);
        let attempts = artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS;
        for _ in 0..attempts {
            app.clone().oneshot(token_request(basic_auth_header("stale-api-token"))).await.unwrap();
        }

        for _ in 0..attempts {
            assert_eq!(app.clone().oneshot(token_request(basic_auth_header("wrong-password"))).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        }

        assert_eq!(app.oneshot(token_request(basic_auth_header("wrong-password"))).await.unwrap().status(), StatusCode::TOO_MANY_REQUESTS);
    }

    fn token_request_from(peer: [u8; 4], forwarded_for: Option<&str>) -> Request<Body> {
        let mut request = token_request(basic_auth_header("wrong-password"));
        request.extensions_mut().insert(ConnectInfo(SocketAddr::from((peer, 40000))));
        if let Some(forwarded_for) = forwarded_for {
            request.headers_mut().insert("x-forwarded-for", forwarded_for.parse().unwrap());
        }
        request
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn clients_behind_a_trusted_proxy_are_throttled_one_by_one(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.guard = Arc::new(RequestGuard::new(Arc::new(TrustedProxies::parse(["10.0.0.0/8"]).unwrap())));
        let app = crate::router(state);

        for _ in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let response = app.clone().oneshot(token_request_from([10, 0, 0, 5], Some("198.51.100.1"))).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let bot = app.clone().oneshot(token_request_from([10, 0, 0, 5], Some("198.51.100.1"))).await.unwrap();
        assert_eq!(bot.status(), StatusCode::TOO_MANY_REQUESTS);
        let bystander = app.oneshot(token_request_from([10, 0, 0, 5], Some("198.51.100.2"))).await.unwrap();
        assert_eq!(bystander.status(), StatusCode::UNAUTHORIZED, "another client behind the same proxy is not locked out");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_client_cannot_pick_its_own_throttle_bucket_with_a_forged_forwarded_for(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.guard = Arc::new(RequestGuard::new(Arc::new(TrustedProxies::parse(["10.0.0.0/8"]).unwrap())));
        let app = crate::router(state);

        for spoofed in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let claimed = format!("198.51.100.{spoofed}");
            app.clone().oneshot(token_request_from([203, 0, 113, 9], Some(&claimed))).await.unwrap();
        }

        let response = app.oneshot(token_request_from([203, 0, 113, 9], Some("198.51.100.99"))).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "X-Forwarded-For from a peer that is not a trusted proxy is ignored");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failed_token_request_is_audited_with_the_resolved_client_address(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool.clone(), dir.path()).await;
        state.guard = Arc::new(RequestGuard::new(Arc::new(TrustedProxies::parse(["10.0.0.0/8"]).unwrap())));
        let app = crate::router(state);

        app.oneshot(token_request_from([10, 0, 0, 5], Some("198.51.100.1"))).await.unwrap();

        let ips: Vec<String> = sqlx::query_scalar!("SELECT payload->>'ip' FROM domain_events WHERE event_type = 'DockerTokenFailed'")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(ips, vec!["198.51.100.1".to_string()]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn issued_tokens_say_how_long_they_live(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let (_user, password) = {
            let id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "an-api-token").await;
            (id, "an-api-token")
        };
        let app = crate::router(state);

        let response = app.oneshot(token_request(basic_auth_header(password))).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["expires_in"], artiferris_domain::docker_registry::DOCKER_ACCESS_TOKEN_TTL_SECONDS);
        assert!(json["issued_at"].as_str().is_some());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn token_endpoint_with_a_valid_token_and_no_scope_issues_an_unscoped_jwt(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "plaintext-token").await;
        let app = crate::router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/token")
                    .header(axum::http::header::AUTHORIZATION, basic_auth_header("plaintext-token"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let claims = state.token_issuer.verify(json["token"].as_str().unwrap()).unwrap();
        assert!(claims.granted_scope.is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn token_endpoint_narrows_scope_to_the_users_granted_role(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let user_id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "plaintext-token").await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        seed_permission(&pool, user_id, repository_id, "read").await;
        let app = crate::router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/token?scope=repository:repo-{repository_id}/myimage:pull,push"))
                    .header(axum::http::header::AUTHORIZATION, basic_auth_header("plaintext-token"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let claims = state.token_issuer.verify(json["token"].as_str().unwrap()).unwrap();
        assert_eq!(claims.granted_scope.unwrap().actions, vec!["pull".to_string()]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn token_endpoint_accepts_repeated_scope_query_parameters(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let user_id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "plaintext-token").await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        seed_permission(&pool, user_id, repository_id, "write").await;
        let app = crate::router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/token?scope=repository%3Arepo-{repository_id}%2Fmyimage%3Apull&scope=repository%3Arepo-{repository_id}%2Fmyimage%3Apull%2Cpush"
                    ))
                    .header(axum::http::header::AUTHORIZATION, basic_auth_header("plaintext-token"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let claims = state.token_issuer.verify(json["token"].as_str().unwrap()).unwrap();
        let mut actions = claims.granted_scope.unwrap().actions;
        actions.sort();
        assert_eq!(actions, vec!["pull".to_string(), "push".to_string()]);
    }

    /// Mirrors artiferris-api's org-admin bypass (`effective_repository_role`): implicit Admin on any repository of
    /// their own organization, no explicit grant.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn token_endpoint_grants_full_scope_to_an_organization_admin_of_the_repositorys_own_organization(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        seed_organization_admin_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "org-admin-token").await;
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        // Deliberately no `seed_permission` call — the org-admin bypass must not need one.
        let app = crate::router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/token?scope=repository:repo-{repository_id}/myimage:pull,push"))
                    .header(axum::http::header::AUTHORIZATION, basic_auth_header("org-admin-token"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let claims = state.token_issuer.verify(json["token"].as_str().unwrap()).unwrap();
        let mut actions = claims.granted_scope.unwrap().actions;
        actions.sort();
        assert_eq!(actions, vec!["pull".to_string(), "push".to_string()]);
    }
}
