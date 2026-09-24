use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use artiferris_domain::audit::SecurityEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth_middleware::AuthUser;
use crate::dto::{application_error_response, ErrorResponse};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/tokens", get(list_tokens).post(create_token)).route("/api/tokens/{id}", delete(revoke_token))
}

#[derive(Serialize)]
struct TokenResponse {
    id: Uuid,
    label: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

async fn list_tokens(State(state): State<AppState>, user: AuthUser) -> Result<Json<Vec<TokenResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let tokens = state.list_api_tokens.execute(user.id).await.map_err(|e| application_error_response("failed to list api tokens", e))?;
    Ok(Json(
        tokens
            .into_iter()
            .map(|t| TokenResponse { id: t.id, label: t.label, created_at: t.created_at, last_used_at: t.last_used_at })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    label: String,
    /// Without it the token lasts 7 days instead of 365, and an SSO account has no password to give.
    current_password: Option<String>,
}

#[derive(Serialize)]
struct CreateTokenResponse {
    id: Uuid,
    // The only place this ever appears — list_tokens deliberately omits it.
    token: String,
}

async fn create_token(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateTokenRequest>,
) -> Result<(StatusCode, Json<CreateTokenResponse>), (StatusCode, Json<ErrorResponse>)> {
    if body.label.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "label must not be empty".to_string() })));
    }
    let label = body.label.trim();
    let created = match body.current_password.as_deref() {
        Some(password) => {
            confirm_password(&state, &user, password).await?;
            state.create_api_token.execute_reauthenticated(user.id, label).await
        }
        None => state.create_api_token.execute(user.id, label).await,
    };
    let (id, token) = created.map_err(|e| application_error_response("failed to create api token", e))?;
    crate::state::record_security_event(
        &state,
        SecurityEvent::ApiTokenCreated { user_id: user.id, organization_id: user.organization_id, token_id: id, label: label.to_string() },
        Some(user.id),
    )
    .await;
    Ok((StatusCode::CREATED, Json(CreateTokenResponse { id, token })))
}

/// Shares the password-management throttle, so a stolen session can't use this to guess the password either.
async fn confirm_password(state: &AppState, user: &AuthUser, password: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let (max_attempts, window) = crate::routes::auth::throttle_limits_for_organization(state, user.organization_id).await;
    let throttle_key = crate::routes::mfa::manage_throttle_key(user.id);
    if !state.login_throttle.reserve(&throttle_key, max_attempts, window) {
        return Err((StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse { error: "too many failed attempts, try again later".to_string() })));
    }
    match state.confirm_password.execute(user.id, password).await {
        Ok(()) => {
            state.login_throttle.clear(&throttle_key);
            Ok(())
        }
        Err(e) => Err(application_error_response("failed to confirm password", e)),
    }
}

async fn revoke_token(State(state): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state.revoke_api_token.execute(id, user.id).await.map_err(|e| application_error_response("failed to revoke api token", e))?;
    crate::state::record_security_event(&state, SecurityEvent::ApiTokenRevoked { user_id: user.id, organization_id: user.organization_id, token_id: id }, Some(user.id)).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::{build_router, state::AppState};
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    use uuid::Uuid;

    // Same local pattern as every other route test module (see routes/auth.rs's own `test_config()`).
    fn test_config() -> Config {
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

    const PUBLIC_ORG: &str = "00000000-0000-0000-0000-000000000001";

    async fn create_user_and_login(state: &AppState, username: &str) -> (Uuid, String) {
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), username, "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute(username, "sup3r-s3cret!").await.unwrap();
        (user_id, token)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_token_with_an_empty_label_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/tokens")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"label":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_a_token_with_a_whitespace_only_label_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/tokens")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"label":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn creating_and_listing_a_token_round_trips(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state);

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/tokens")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"label":"my laptop"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let body = to_bytes(create_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["token"].as_str().unwrap().starts_with("hgr_"));

        let list_response = app.oneshot(Request::builder().uri("/api/tokens").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(list_response.status(), StatusCode::OK);
        let body = to_bytes(list_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tokens = json.as_array().unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0]["label"], "my laptop");
        assert!(tokens[0].get("token").is_none(), "the plaintext token must never be returned by list");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn listing_tokens_only_returns_the_callers_own_tokens(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, alice_token) = create_user_and_login(&state, "alice").await;
        let (_, bob_token) = create_user_and_login(&state, "bob").await;
        let app = build_router(state);

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/tokens")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"label":"alices token"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        let bob_list = app.oneshot(Request::builder().uri("/api/tokens").header("authorization", format!("Bearer {bob_token}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(bob_list.status(), StatusCode::OK);
        let body = to_bytes(bob_list.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 0, "bob must not see alice's tokens");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn revoking_own_token_removes_it_from_the_list(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state);

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/tokens")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"label":"my laptop"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = json["id"].as_str().unwrap();

        let revoke_response = app
            .clone()
            .oneshot(Request::builder().method("DELETE").uri(format!("/api/tokens/{id}")).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(revoke_response.status(), StatusCode::NO_CONTENT);

        let list_response = app.oneshot(Request::builder().uri("/api/tokens").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap()).await.unwrap();
        let body = to_bytes(list_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    // Does the HTTP handler actually check that the caller owns the token being revoked?
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn revoking_another_users_token_is_rejected_and_does_not_revoke_it(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, alice_token) = create_user_and_login(&state, "alice").await;
        let (_, bob_token) = create_user_and_login(&state, "bob").await;
        let app = build_router(state);

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/tokens")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"label":"alices token"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let alices_token_id = json["id"].as_str().unwrap();

        // Bob attempts to revoke Alice's token by id.
        let revoke_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/tokens/{alices_token_id}"))
                    .header("authorization", format!("Bearer {bob_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // 404, not 403 — doesn't confirm the id exists at all.
        assert_eq!(revoke_response.status(), StatusCode::NOT_FOUND);

        // The important part: Alice's token must still be usable/listed — Bob's request must not have actually revoked it.
        let alice_list = app.oneshot(Request::builder().uri("/api/tokens").header("authorization", format!("Bearer {alice_token}")).body(Body::empty()).unwrap()).await.unwrap();
        let body = to_bytes(alice_list.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tokens = json.as_array().unwrap();
        assert_eq!(tokens.len(), 1, "a different user's revoke attempt must not remove Alice's token");
        assert_eq!(tokens[0]["id"], alices_token_id);
    }

    // A nonexistent id must return the same status as someone else's token — must not let a caller distinguish "not yours" from "doesn't exist".
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn revoking_an_unknown_token_id_returns_not_found(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state);

        let revoke_response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/tokens/{}", Uuid::new_v4()))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke_response.status(), StatusCode::NOT_FOUND);
    }

    fn create_token_request(token: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/tokens")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn expiry_in_days(state: &AppState, user_id: Uuid) -> i64 {
        let tokens = state.api_tokens.list_for_user(user_id).await.unwrap();
        (tokens[0].expires_at.unwrap() - chrono::Utc::now()).num_days()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_token_created_from_a_session_alone_lasts_7_days(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (user_id, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state.clone());

        let response = app.oneshot(create_token_request(&token, serde_json::json!({ "label": "ci" }))).await.unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(expiry_in_days(&state, user_id).await, 6);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_token_created_with_the_password_lasts_365_days(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (user_id, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state.clone());

        let response = app.oneshot(create_token_request(&token, serde_json::json!({ "label": "ci", "current_password": "sup3r-s3cret!" }))).await.unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(expiry_in_days(&state, user_id).await, 364);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_wrong_password_creates_no_token(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (user_id, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state.clone());

        let response = app.oneshot(create_token_request(&token, serde_json::json!({ "label": "ci", "current_password": "wrong" }))).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(state.api_tokens.list_for_user(user_id).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn guessing_the_password_through_token_creation_is_throttled(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state);

        let mut last = StatusCode::OK;
        for _ in 0..=artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            last = app.clone().oneshot(create_token_request(&token, serde_json::json!({ "label": "ci", "current_password": "wrong" }))).await.unwrap().status();
        }

        assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_overlong_label_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let (_, token) = create_user_and_login(&state, "florian").await;
        let app = build_router(state);

        let response = app.oneshot(create_token_request(&token, serde_json::json!({ "label": "x".repeat(101) }))).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
