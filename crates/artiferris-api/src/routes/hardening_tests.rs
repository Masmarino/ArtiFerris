//! Login throttling across tenants, the admin unlock, admin token management, the session TOTP confirm, and the audit budget of repeatable events.

use std::net::SocketAddr;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use artiferris_application::login_throttle::{shared_username_key, LOGIN_ATTEMPT_WINDOW, MAX_LOGIN_ATTEMPTS};
use artiferris_domain::audit::{AuditEntry, AuditQueryFilter};
use artiferris_domain::system_settings::SystemSettings;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::state::AppState;
use crate::build_router;

const PUBLIC_ORG: &str = "00000000-0000-0000-0000-000000000001";
const PASSWORD: &str = "sup3r-s3cret!";

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

fn public_org() -> Uuid {
    Uuid::parse_str(PUBLIC_ORG).unwrap()
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

/// From its own address each time, so only the per-username budgets are under test.
async fn login_from(app: &Router, host: &str, username: &str, password: &str, address: u16) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("host", host)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({ "username": username, "password": password }).to_string()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([10, 1, (address >> 8) as u8, address as u8], 51234))));
    app.clone().oneshot(request).await.unwrap().status()
}

async fn new_user(state: &AppState, organization_id: Uuid, username: &str) -> (Uuid, String) {
    let id = state.create_user.execute(organization_id, username, PASSWORD, false).await.unwrap();
    (id, state.authenticate_user.execute(username, PASSWORD).await.unwrap())
}

async fn new_org_admin(state: &AppState, organization_id: Uuid, username: &str) -> (Uuid, String) {
    let id = state.create_user.execute(organization_id, username, PASSWORD, false).await.unwrap();
    state.set_organization_admin.execute(id, true, None).await.unwrap();
    (id, state.authenticate_user.execute(username, PASSWORD).await.unwrap())
}

async fn new_super_admin(state: &AppState, organization_id: Uuid, username: &str) -> (Uuid, String) {
    let id = state.create_user.execute(organization_id, username, PASSWORD, true).await.unwrap();
    (id, state.authenticate_user.execute(username, PASSWORD).await.unwrap())
}

async fn organization_with_policy(state: &AppState, slug: &str, max_login_attempts: i32, login_attempt_window_seconds: i32) -> Uuid {
    let organization_id = state.create_organization.execute(slug, slug).await.unwrap();
    state
        .update_system_settings
        .execute(organization_id, SystemSettings { max_login_attempts, login_attempt_window_seconds, session_ttl_hours: 12, registration_enabled: true, seo_indexing_enabled: false, seo_indexing_blocked: false, public_page_enabled: true }, None)
        .await
        .unwrap();
    organization_id
}

async fn audit_entries(state: &AppState, event_type: &str) -> Vec<AuditEntry> {
    state.query_audit_log.execute(AuditQueryFilter::default()).await.unwrap().entries.into_iter().filter(|e| e.event_type == event_type).collect()
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_tenants_stricter_limit_cannot_lock_an_account_of_another_tenant(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    organization_with_policy(&state, "hostile", 1, 300).await;
    new_user(&state, public_org(), "victim").await;
    let app = build_router(state);

    assert_eq!(login_from(&app, "hostile.artiferris.localhost", "victim", "wrong", 1).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        login_from(&app, "hostile.artiferris.localhost", "victim", "wrong", 2).await,
        StatusCode::TOO_MANY_REQUESTS,
        "the hostile tenant's own policy still holds on its own host"
    );

    assert_eq!(login_from(&app, "artiferris.localhost", "victim", PASSWORD, 3).await, StatusCode::OK, "but the victim, on their own host, is not locked out by it");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_tenants_long_window_cannot_stretch_a_lock_on_the_shared_username_key(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    organization_with_policy(&state, "hostile", 10, 86_400).await;
    new_user(&state, public_org(), "victim").await;
    let app = build_router(state.clone());

    for address in 1..=MAX_LOGIN_ATTEMPTS as u16 {
        login_from(&app, "hostile.artiferris.localhost", "victim", "wrong", address).await;
    }

    let blocked = state.login_throttle.blocked_usernames(MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW);
    let shared = blocked.iter().find(|b| b.username == shared_username_key("victim")).expect("the shared key is at its limit");
    assert!(shared.remaining_seconds <= LOGIN_ATTEMPT_WINDOW.as_secs(), "the lock lasts the default window, not the hostile tenant's 24 hours: {}", shared.remaining_seconds);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_tenants_stricter_policy_counts_only_the_logins_made_on_its_own_host(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let strict = organization_with_policy(&state, "strict", 2, 300).await;
    new_user(&state, strict, "member").await;
    let app = build_router(state);

    assert_eq!(login_from(&app, "strict.artiferris.localhost", "member", "wrong", 1).await, StatusCode::UNAUTHORIZED);
    assert_eq!(login_from(&app, "strict.artiferris.localhost", "member", "wrong", 2).await, StatusCode::UNAUTHORIZED);
    assert_eq!(login_from(&app, "strict.artiferris.localhost", "member", PASSWORD, 3).await, StatusCode::TOO_MANY_REQUESTS);

    assert_eq!(login_from(&app, "artiferris.localhost", "member", PASSWORD, 4).await, StatusCode::OK, "the default host only sees the shared key, at two failures of ten");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_successful_password_login_forgets_the_failures_on_both_keys(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let strict = organization_with_policy(&state, "strict", 3, 300).await;
    new_user(&state, strict, "member").await;
    let app = build_router(state);

    for address in 1..=2 {
        assert_eq!(login_from(&app, "strict.artiferris.localhost", "member", "wrong", address).await, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(login_from(&app, "strict.artiferris.localhost", "member", PASSWORD, 3).await, StatusCode::OK);

    for address in 4..=5 {
        assert_eq!(login_from(&app, "strict.artiferris.localhost", "member", "wrong", address).await, StatusCode::UNAUTHORIZED, "the tally started over");
    }
}

struct DirectoryAnswering(&'static str);

#[async_trait::async_trait]
impl artiferris_domain::sso::LdapAuthPort for DirectoryAnswering {
    async fn authenticate(
        &self,
        _config: &artiferris_domain::sso::LdapConfig,
        _username: &str,
        _password: &str,
    ) -> Result<artiferris_domain::sso::ExternalIdentity, artiferris_domain::error::DomainError> {
        Ok(artiferris_domain::sso::ExternalIdentity { email: self.0.to_string(), display_name: None })
    }
}

async fn with_directory_answering(pool: sqlx::PgPool, email: &'static str) -> AppState {
    let mut state = AppState::build(pool, &test_config());
    state.ldap_auth = std::sync::Arc::new(DirectoryAnswering(email));
    state
        .identity_providers
        .set(
            public_org(),
            &artiferris_domain::sso::IdentityProviderConfig::Ldap(artiferris_domain::sso::LdapConfig {
                server_url: "ldap://dc.corp.example:389".to_string(),
                bind_dn: "cn=service,dc=corp,dc=example".to_string(),
                bind_password: "s3cret!".to_string(),
                user_search_base: "ou=people,dc=corp,dc=example".to_string(),
                user_search_filter: "(uid={username})".to_string(),
                email_attribute: "mail".to_string(),
            }),
            None,
        )
        .await
        .unwrap();
    state
}

async fn ldap_login_from(app: &Router, username: &str, address: u16) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/auth/sso/ldap")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({ "username": username, "password": "whatever" }).to_string()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([10, 2, (address >> 8) as u8, address as u8], 51234))));
    app.clone().oneshot(request).await.unwrap().status()
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_directory_login_cannot_reset_the_failures_counted_against_another_account(pool: sqlx::PgPool) {
    let state = with_directory_answering(pool, "attacker@corp.example").await;
    new_user(&state, public_org(), "victim").await;
    let app = build_router(state);
    let limit = MAX_LOGIN_ATTEMPTS as u16;
    for address in 1..limit {
        assert_eq!(login_from(&app, "artiferris.localhost", "victim", "guess", address).await, StatusCode::UNAUTHORIZED);
    }

    // A directory that accepts anything, asked to sign in "victim": it lands in the attacker's own account.
    assert_eq!(ldap_login_from(&app, "victim", 100).await, StatusCode::OK);

    assert_eq!(login_from(&app, "artiferris.localhost", "victim", "guess", 200).await, StatusCode::UNAUTHORIZED, "the tenth failure is still allowed");
    assert_eq!(login_from(&app, "artiferris.localhost", "victim", "guess", 201).await, StatusCode::TOO_MANY_REQUESTS, "and the counter was not wiped: the eleventh is refused");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_directory_login_forgets_the_failures_of_the_account_it_really_signed_in(pool: sqlx::PgPool) {
    let state = with_directory_answering(pool, "florian@corp.example").await;
    let app = build_router(state);
    let limit = MAX_LOGIN_ATTEMPTS as u16;
    for address in 1..limit {
        assert_eq!(login_from(&app, "artiferris.localhost", "florian", "guess", address).await, StatusCode::UNAUTHORIZED);
    }

    assert_eq!(ldap_login_from(&app, "florian", 100).await, StatusCode::OK, "the directory's account for florian@corp.example is provisioned as florian");

    for address in 200..200 + limit - 1 {
        assert_eq!(login_from(&app, "artiferris.localhost", "florian", "guess", address).await, StatusCode::UNAUTHORIZED, "a fresh window");
    }
}

async fn lock_out(app: &Router, host: &str, username: &str) {
    for address in 1..=MAX_LOGIN_ATTEMPTS as u16 {
        login_from(app, host, username, "guess", 1000 + address).await;
    }
    assert_eq!(login_from(app, host, username, PASSWORD, 2000).await, StatusCode::TOO_MANY_REQUESTS, "{username} is locked");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_super_admin_unlocks_an_account_and_it_is_audited(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (admin_id, admin_token) = new_super_admin(&state, public_org(), "root").await;
    new_user(&state, public_org(), "member").await;
    let app = build_router(state.clone());
    lock_out(&app, "artiferris.localhost", "member").await;

    let (status, _) = send(&app, "DELETE", "/api/admin/login-throttle/Member", &admin_token, None).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(login_from(&app, "artiferris.localhost", "member", PASSWORD, 3000).await, StatusCode::OK);
    let cleared = audit_entries(&state, "LoginThrottleCleared").await;
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].actor_id, Some(admin_id));
    assert_eq!(cleared[0].organization_id, Some(public_org()));
    assert_eq!(cleared[0].payload["username"], "member");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn unlocking_also_lifts_the_organizations_own_key_and_the_second_factor_budget(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (_, admin_token) = new_super_admin(&state, public_org(), "root").await;
    let strict = organization_with_policy(&state, "strict", 2, 300).await;
    let (member_id, _) = new_user(&state, strict, "member").await;
    let app = build_router(state.clone());
    login_from(&app, "strict.artiferris.localhost", "member", "guess", 1).await;
    login_from(&app, "strict.artiferris.localhost", "member", "guess", 2).await;
    assert_eq!(login_from(&app, "strict.artiferris.localhost", "member", PASSWORD, 3).await, StatusCode::TOO_MANY_REQUESTS);
    let mfa_key = crate::routes::auth::mfa_verify_throttle_key(member_id);
    for _ in 0..MAX_LOGIN_ATTEMPTS {
        state.login_throttle.record_failure(&mfa_key, MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW);
    }
    assert!(state.login_throttle.is_throttled(&mfa_key, MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW));

    let (status, _) = send(&app, "DELETE", "/api/admin/login-throttle/member", &admin_token, None).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(login_from(&app, "strict.artiferris.localhost", "member", PASSWORD, 4).await, StatusCode::OK);
    assert!(!state.login_throttle.is_throttled(&mfa_key, MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW));
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_super_admin_can_lift_a_lock_on_a_name_nobody_holds(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (_, admin_token) = new_super_admin(&state, public_org(), "root").await;
    let app = build_router(state.clone());
    lock_out(&app, "artiferris.localhost", "ghost").await;

    let (status, _) = send(&app, "DELETE", "/api/admin/login-throttle/ghost", &admin_token, None).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_ne!(login_from(&app, "artiferris.localhost", "ghost", "guess", 3000).await, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(audit_entries(&state, "LoginThrottleCleared").await[0].organization_id, None);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_organization_admin_unlocks_only_the_non_admin_members_of_their_own_organization(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let acme = state.create_organization.execute("acme", "Acme").await.unwrap();
    let (_, org_admin_token) = new_org_admin(&state, acme, "acme-admin").await;
    new_user(&state, acme, "acme-member").await;
    new_user(&state, public_org(), "outsider").await;
    new_super_admin(&state, acme, "acme-root").await;
    let (_, member_token) = new_user(&state, acme, "acme-other").await;
    let app = build_router(state.clone());
    lock_out(&app, "acme.artiferris.localhost", "acme-member").await;
    lock_out(&app, "artiferris.localhost", "outsider").await;
    lock_out(&app, "acme.artiferris.localhost", "acme-root").await;

    let (status, _) = send(&app, "DELETE", "/api/admin/login-throttle/acme-member", &org_admin_token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(login_from(&app, "acme.artiferris.localhost", "acme-member", PASSWORD, 3000).await, StatusCode::OK);

    let (status, _) = send(&app, "DELETE", "/api/admin/login-throttle/outsider", &org_admin_token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "another organization's member");
    assert_eq!(login_from(&app, "artiferris.localhost", "outsider", PASSWORD, 3001).await, StatusCode::TOO_MANY_REQUESTS, "still locked");

    let (status, _) = send(&app, "DELETE", "/api/admin/login-throttle/acme-root", &org_admin_token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a super-admin of their own organization");
    assert_eq!(login_from(&app, "acme.artiferris.localhost", "acme-root", PASSWORD, 3002).await, StatusCode::TOO_MANY_REQUESTS);

    let (status, _) = send(&app, "DELETE", "/api/admin/login-throttle/nobody-at-all", &org_admin_token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(&app, "DELETE", "/api/admin/login-throttle/acme-member", &member_token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a plain member");
    assert_eq!(audit_entries(&state, "LoginThrottleCleared").await.len(), 1, "only the allowed unlock was recorded");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn unlocking_needs_a_session(pool: sqlx::PgPool) {
    let app = build_router(AppState::build(pool, &test_config()));

    let response = app.oneshot(Request::builder().method("DELETE").uri("/api/admin/login-throttle/anyone").body(Body::empty()).unwrap()).await.unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_organization_admin_cannot_revoke_the_tokens_of_a_super_admin_in_their_organization(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (_, org_admin_token) = new_org_admin(&state, public_org(), "org-admin").await;
    let (super_admin_id, _) = new_super_admin(&state, public_org(), "root").await;
    let (member_id, _) = new_user(&state, public_org(), "member").await;
    let (operators_token_id, _) = state.create_api_token.execute(super_admin_id, "publishing").await.unwrap();
    let (members_token_id, _) = state.create_api_token.execute(member_id, "ci").await.unwrap();
    let app = build_router(state.clone());

    let (status, _) = send(&app, "DELETE", &format!("/api/admin/tokens/{operators_token_id}"), &org_admin_token, None).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(state.admin_list_api_tokens.find(operators_token_id).await.unwrap().unwrap().revoked_at.is_none(), "the operator's token still works");
    let (status, _) = send(&app, "DELETE", &format!("/api/admin/tokens/{members_token_id}"), &org_admin_token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "an ordinary member's token is still theirs to revoke");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_super_admin_can_still_revoke_any_token(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (_, root_token) = new_super_admin(&state, public_org(), "root").await;
    let (other_super_admin, _) = new_super_admin(&state, public_org(), "root-two").await;
    let (token_id, _) = state.create_api_token.execute(other_super_admin, "ci").await.unwrap();
    let app = build_router(state.clone());

    let (status, _) = send(&app, "DELETE", &format!("/api/admin/tokens/{token_id}"), &root_token, None).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(state.admin_list_api_tokens.find(token_id).await.unwrap().unwrap().revoked_at.is_some());
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn the_admin_token_list_is_paged_and_capped(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (root_id, root_token) = new_super_admin(&state, public_org(), "root").await;
    for label in ["a", "b", "c"] {
        state.create_api_token.execute(root_id, label).await.unwrap();
    }
    let app = build_router(state);

    let (_, everything) = send(&app, "GET", "/api/admin/tokens", &root_token, None).await;
    let (_, first_page) = send(&app, "GET", "/api/admin/tokens?limit=2", &root_token, None).await;
    let (_, second_page) = send(&app, "GET", "/api/admin/tokens?limit=2&offset=2", &root_token, None).await;
    let (status, silly) = send(&app, "GET", "/api/admin/tokens?limit=99999999&offset=-4", &root_token, None).await;

    assert_eq!(everything.as_array().unwrap().len(), 3);
    assert_eq!(first_page.as_array().unwrap().len(), 2);
    assert_eq!(second_page.as_array().unwrap().len(), 1);
    assert_eq!(second_page[0]["id"], everything[2]["id"], "pages continue where the last one stopped");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(silly.as_array().unwrap().len(), 3);
    assert!(everything[0]["username"] == "root" && everything[0]["label"].is_string() && everything[0]["revoked_at"].is_null(), "the response shape is unchanged");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_organization_admins_token_list_never_reaches_another_organization(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let acme = state.create_organization.execute("acme", "Acme").await.unwrap();
    let (_, acme_admin_token) = new_org_admin(&state, acme, "acme-admin").await;
    let (acme_member, _) = new_user(&state, acme, "acme-member").await;
    let (public_member, _) = new_user(&state, public_org(), "public-member").await;
    state.create_api_token.execute(acme_member, "acme ci").await.unwrap();
    state.create_api_token.execute(public_member, "public ci").await.unwrap();
    let app = build_router(state);

    let (_, listed) = send(&app, "GET", &format!("/api/admin/tokens?organization_id={public_org}", public_org = public_org()), &acme_admin_token, None).await;

    let usernames: Vec<&str> = listed.as_array().unwrap().iter().map(|t| t["username"].as_str().unwrap()).collect();
    assert_eq!(usernames, vec!["acme-member"], "the organization_id parameter is a super-admin's alone");
}

async fn started_totp_enrolment(app: &Router, token: &str) -> String {
    let (status, enrolled) = send(app, "POST", "/api/me/mfa/totp/enroll", token, Some(serde_json::json!({ "current_password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::OK);
    enrolled["secret"].as_str().unwrap().to_string()
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn confirming_a_totp_enrolment_with_wrong_codes_runs_out_of_attempts_even_for_the_right_code(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (_, token) = new_user(&state, public_org(), "florian").await;
    let app = build_router(state);
    let secret = started_totp_enrolment(&app, &token).await;

    for _ in 0..MAX_LOGIN_ATTEMPTS {
        let (status, _) = send(&app, "POST", "/api/me/mfa/totp/confirm", &token, Some(serde_json::json!({ "code": "000000" }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    let right = artiferris_application::use_cases::mfa::generate_current_totp_code(&secret);
    let (status, _) = send(&app, "POST", "/api/me/mfa/totp/confirm", &token, Some(serde_json::json!({ "code": right }))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_correct_confirmation_still_works_after_a_few_typos(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (_, token) = new_user(&state, public_org(), "florian").await;
    let app = build_router(state);
    let secret = started_totp_enrolment(&app, &token).await;
    for _ in 0..3 {
        send(&app, "POST", "/api/me/mfa/totp/confirm", &token, Some(serde_json::json!({ "code": "000000" }))).await;
    }

    let right = artiferris_application::use_cases::mfa::generate_current_totp_code(&secret);
    let (status, confirmed) = send(&app, "POST", "/api/me/mfa/totp/confirm", &token, Some(serde_json::json!({ "code": right }))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(confirmed["backup_codes"].as_array().unwrap().len(), 10);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_enrolment_left_unconfirmed_for_too_long_is_expired(pool: sqlx::PgPool) {
    let state = AppState::build(pool.clone(), &test_config());
    let (_, token) = new_user(&state, public_org(), "florian").await;
    let app = build_router(state);
    let secret = started_totp_enrolment(&app, &token).await;
    sqlx::query("UPDATE totp_credentials SET created_at = now() - interval '16 minutes'").execute(&pool).await.unwrap();

    let right = artiferris_application::use_cases::mfa::generate_current_totp_code(&secret);
    let (status, body) = send(&app, "POST", "/api/me/mfa/totp/confirm", &token, Some(serde_json::json!({ "code": right }))).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("expired"), "{body}");
    let again = started_totp_enrolment(&app, &token).await;
    let (status, _) = send(&app, "POST", "/api/me/mfa/totp/confirm", &token, Some(serde_json::json!({ "code": artiferris_application::use_cases::mfa::generate_current_totp_code(&again) }))).await;
    assert_eq!(status, StatusCode::OK, "starting over gets a fresh window");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_table_full_of_spent_mfa_tokens_never_blocks_the_next_login(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let user_id = state.create_user.execute(public_org(), "florian", PASSWORD, false).await.unwrap();
    let enrollment = state.enroll_totp.execute(user_id, "florian", Some(PASSWORD)).await.unwrap();
    state.confirm_totp.execute(user_id, "florian", &artiferris_application::use_cases::mfa::generate_current_totp_code(&enrollment.secret_base32)).await.unwrap();
    for _ in 0..10_050 {
        assert!(state.used_mfa_tokens.consume(Uuid::new_v4(), &Uuid::new_v4().to_string()));
    }
    let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
    let app = build_router(state.clone());
    let code = artiferris_application::use_cases::mfa::generate_totp_code_after_step(&enrollment.secret_base32, chrono::Utc::now().timestamp() / 30);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/mfa/verify")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "mfa_token": mfa_token, "code": code }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

/// A member's requests that the server refuses, one audit row each until the budget runs out.
async fn refused_requests(app: &Router, token: &str, repository_id: Uuid, count: usize) {
    for _ in 0..count {
        let (status, _) = send(app, "GET", &format!("/api/repositories/{repository_id}"), token, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn one_member_cannot_flood_the_audit_trail_with_refused_requests(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (admin_id, _) = new_super_admin(&state, public_org(), "root").await;
    let repository_id = state
        .create_repository
        .execute(public_org(), "private-repo", artiferris_domain::package_repository::RepositoryFormat::Npm, artiferris_domain::package_repository::RepositoryType::Hosted, None, None, None, admin_id)
        .await
        .unwrap();
    let (flooder_id, flooder_token) = new_user(&state, public_org(), "flooder").await;
    let (_, bystander_token) = new_user(&state, public_org(), "bystander").await;
    let app = build_router(state.clone());

    refused_requests(&app, &flooder_token, repository_id, 90).await;

    assert_eq!(audit_entries(&state, "AccessDenied").await.len(), 60, "the operation was refused every time, but only the first sixty were recorded");
    refused_requests(&app, &bystander_token, repository_id, 1).await;
    let recorded = audit_entries(&state, "AccessDenied").await;
    assert_eq!(recorded.len(), 61, "another member's budget is their own");
    assert_eq!(recorded.iter().filter(|e| e.actor_id == Some(flooder_id)).count(), 60);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn creating_and_revoking_tokens_in_a_loop_leaves_a_bounded_audit_trail_but_still_works(pool: sqlx::PgPool) {
    let state = AppState::build(pool, &test_config());
    let (_, token) = new_user(&state, public_org(), "looper").await;
    let (_, other_token) = new_user(&state, public_org(), "other").await;
    let app = build_router(state.clone());

    for _ in 0..25 {
        let (status, created) = send(&app, "POST", "/api/tokens", &token, Some(serde_json::json!({ "label": "loop" }))).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = send(&app, "DELETE", &format!("/api/tokens/{}", created["id"].as_str().unwrap()), &token, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    let created = audit_entries(&state, "ApiTokenCreated").await.len();
    let revoked = audit_entries(&state, "ApiTokenRevoked").await.len();
    assert_eq!(created + revoked, 30, "created {created}, revoked {revoked}");
    let (status, _) = send(&app, "POST", "/api/tokens", &other_token, Some(serde_json::json!({ "label": "ci" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(audit_entries(&state, "ApiTokenCreated").await.len(), created + 1, "another user's budget is untouched");
}

async fn state_with_another_encryption_key(pool: sqlx::PgPool) -> AppState {
    let mut config = test_config();
    config.secrets_encryption_key = "a-different-secrets-encryption-key".to_string();
    AppState::build(pool, &config)
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_totp_seed_that_cannot_be_read_is_a_generic_409_at_login_not_a_400_with_key_fingerprints(pool: sqlx::PgPool) {
    let sealed_by_the_old_key = AppState::build(pool.clone(), &test_config());
    let user_id = sealed_by_the_old_key.create_user.execute(public_org(), "florian", PASSWORD, false).await.unwrap();
    let enrollment = sealed_by_the_old_key.enroll_totp.execute(user_id, "florian", Some(PASSWORD)).await.unwrap();
    sealed_by_the_old_key.confirm_totp.execute(user_id, "florian", &artiferris_application::use_cases::mfa::generate_current_totp_code(&enrollment.secret_base32)).await.unwrap();
    let app = build_router(state_with_another_encryption_key(pool).await);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "username": "florian", "password": PASSWORD }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = String::from_utf8(to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    assert!(!body.contains("key id") && !body.contains("SECRETS_ENCRYPTION_KEY") && !body.contains("sealed"), "{body}");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_identity_provider_whose_secret_cannot_be_read_does_not_break_the_login_page(pool: sqlx::PgPool) {
    let sealed_by_the_old_key = AppState::build(pool.clone(), &test_config());
    sealed_by_the_old_key
        .identity_providers
        .set(
            public_org(),
            &artiferris_domain::sso::IdentityProviderConfig::Ldap(artiferris_domain::sso::LdapConfig {
                server_url: "ldap://dc.corp.example:389".to_string(),
                bind_dn: "cn=service,dc=corp,dc=example".to_string(),
                bind_password: "s3cret!".to_string(),
                user_search_base: "ou=people,dc=corp,dc=example".to_string(),
                user_search_filter: "(uid={username})".to_string(),
                email_attribute: "mail".to_string(),
            }),
            None,
        )
        .await
        .unwrap();
    let app = build_router(state_with_another_encryption_key(pool).await);

    let response = app.oneshot(Request::builder().uri("/api/auth/sso/config").body(Body::empty()).unwrap()).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert!(body["provider_type"].is_null(), "{body}");
}
