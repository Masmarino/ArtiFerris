//! Anonymous reads of the registry share a per-client budget, so one address cannot drain the server's bandwidth.
//! A request that carries a token is never counted: the token names who to hold to account.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::errors::docker_error;
use crate::routes::handshake::ANONYMOUS_DOCKER_TOKEN;
use crate::state::DockerState;

/// No `Authorization` header, or the placeholder token a client is handed when it asks for one anonymously.
fn is_anonymous(headers: &HeaderMap) -> bool {
    match headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()) {
        None => true,
        Some(value) => value.strip_prefix("Bearer ").is_some_and(|token| token == ANONYMOUS_DOCKER_TOKEN),
    }
}

pub async fn limit_anonymous_reads(State(state): State<DockerState>, request: Request, next: Next) -> Response {
    if matches!(*request.method(), Method::GET | Method::HEAD) && is_anonymous(request.headers()) {
        let forwarded: Vec<&str> = request.headers().get_all("x-forwarded-for").iter().filter_map(|value| value.to_str().ok()).collect();
        let direct = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|ConnectInfo(address)| address.ip());
        let client = state.guard.client_bucket(direct, &forwarded);
        if !state.guard.allow_anonymous_read("docker", &client) {
            let mut response = docker_error(StatusCode::TOO_MANY_REQUESTS, "TOOMANYREQUESTS", "too many requests, try again shortly").into_response();
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(artiferris_application::rate_limiter::RETRY_AFTER_SECONDS));
            return response;
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route_test_support::{issue_test_token, seed_bare_user, test_state};
    use artiferris_application::rate_limiter::RateLimiter;
    use artiferris_application::request_guard::RequestGuard;
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn with_authorization(value: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn only_a_request_without_a_real_token_counts_as_anonymous() {
        assert!(is_anonymous(&with_authorization(None)));
        assert!(is_anonymous(&with_authorization(Some("Bearer anonymous"))));
        assert!(!is_anonymous(&with_authorization(Some("Bearer eyJhbGciOi"))));
        assert!(!is_anonymous(&with_authorization(Some("Basic dXNlcjpwYXNz"))));
    }

    async fn app_with_limit(pool: sqlx::PgPool, per_minute: usize) -> (axum::Router, crate::DockerState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.guard = Arc::new(RequestGuard::default().with_anonymous_limit(Arc::new(RateLimiter::default()), per_minute));
        (crate::router(state.clone()), state, dir)
    }

    async fn get(app: &axum::Router, authorization: Option<&str>) -> Response {
        let mut request = Request::builder().method("GET").uri("/");
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }
        app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_client_is_refused_once_its_minute_is_used_up(pool: sqlx::PgPool) {
        let (app, _, _dir) = app_with_limit(pool, 3).await;

        for _ in 0..3 {
            assert_ne!(get(&app, None).await.status(), StatusCode::TOO_MANY_REQUESTS);
        }
        let refused = get(&app, None).await;

        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(refused.headers().contains_key(header::RETRY_AFTER));
        let body = axum::body::to_bytes(refused.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["errors"][0]["code"], "TOOMANYREQUESTS", "a docker client reads this envelope");
        assert_eq!(get(&app, Some("Bearer anonymous")).await.status(), StatusCode::TOO_MANY_REQUESTS, "the anonymous placeholder token is anonymous too");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_client_with_a_token_is_never_counted(pool: sqlx::PgPool) {
        let (app, state, _dir) = app_with_limit(pool.clone(), 1).await;
        let user_id = seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await;
        let token = issue_test_token(&state, user_id, Uuid::new_v4(), "repo", "image", &["pull"]);
        assert_eq!(get(&app, None).await.status(), StatusCode::UNAUTHORIZED, "the first anonymous request is served, here with the registry's challenge");
        assert_eq!(get(&app, None).await.status(), StatusCode::TOO_MANY_REQUESTS);

        for _ in 0..5 {
            assert_ne!(get(&app, Some(&format!("Bearer {token}"))).await.status(), StatusCode::TOO_MANY_REQUESTS);
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_limit_of_zero_turns_the_limit_off(pool: sqlx::PgPool) {
        let (app, _, _dir) = app_with_limit(pool, 0).await;
        for _ in 0..50 {
            assert_ne!(get(&app, None).await.status(), StatusCode::TOO_MANY_REQUESTS);
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn writes_are_not_counted_as_reads(pool: sqlx::PgPool) {
        let (app, _, _dir) = app_with_limit(pool, 1).await;
        for _ in 0..3 {
            let response = app.clone().oneshot(Request::builder().method("POST").uri("/some-repo/image/blobs/uploads/").body(Body::empty()).unwrap()).await.unwrap();
            assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        }
    }
}
