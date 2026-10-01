//! Every privilege, credential and configuration change leaves an audit entry, and the audit endpoint serves it back per organization, in pages.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use artiferris_domain::audit::{AuditEntry, AuditQueryFilter};
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::state::AppState;
use crate::build_router;

const PUBLIC_ORG: &str = "00000000-0000-0000-0000-000000000001";
const PASSWORD: &str = "sup3r-s3cret!";
/// Planted in every request that carries a secret; it must never come back out of the audit log.
const SECRET: &str = "sentinel-secret-value-9f3a";

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

struct Fixture {
    state: AppState,
    app: Router,
    admin_id: Uuid,
    admin_token: String,
}

async fn fixture(pool: sqlx::PgPool) -> Fixture {
    let state = AppState::build(pool, &test_config());
    let admin_id = state.create_user.execute(public_org(), "root", PASSWORD, true).await.unwrap();
    let admin_token = state.authenticate_user.execute("root", PASSWORD).await.unwrap();
    let app = build_router(state.clone());
    Fixture { state, app, admin_id, admin_token }
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

async fn all_entries(state: &AppState) -> Vec<AuditEntry> {
    state.query_audit_log.execute(AuditQueryFilter::default()).await.unwrap().entries
}

async fn entries_of(state: &AppState, event_type: &str) -> Vec<AuditEntry> {
    all_entries(state).await.into_iter().filter(|e| e.event_type == event_type).collect()
}

async fn the_only(state: &AppState, event_type: &str) -> AuditEntry {
    let mut found = entries_of(state, event_type).await;
    assert_eq!(found.len(), 1, "expected exactly one {event_type}, got {found:?}");
    found.remove(0)
}

fn assert_no_secret(entry: &AuditEntry) {
    assert!(!entry.payload.to_string().contains(SECRET), "secret leaked into {:?}", entry.payload);
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

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn inviting_and_deleting_a_user_are_recorded(pool: sqlx::PgPool) {
    let f = fixture(pool).await;

    let (status, created) = send(&f.app, "POST", "/api/users", &f.admin_token, Some(serde_json::json!({ "email": "invitee@example.com", "is_super_admin": false, "is_organization_admin": true }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = created["id"].as_str().unwrap().to_string();
    let invited = the_only(&f.state, "UserInvited").await;
    assert_eq!(invited.actor_id, Some(f.admin_id));
    assert_eq!(invited.organization_id, Some(public_org()));
    assert_eq!(invited.payload["user_id"], user_id);
    assert_eq!(invited.payload["email"], "invitee@example.com");
    assert_eq!(invited.payload["is_organization_admin"], true);

    let (status, _) = send(&f.app, "DELETE", &format!("/api/users/{user_id}"), &f.admin_token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let deleted = the_only(&f.state, "UserDeleted").await;
    assert_eq!(deleted.actor_id, Some(f.admin_id));
    assert_eq!(deleted.payload["user_id"], user_id);
    assert!(deleted.payload["username"].as_str().unwrap().starts_with("invite-"), "a never-activated account holds its placeholder name");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_organization_admin_inviting_a_member_is_recorded_under_their_organization(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let (org_admin_id, org_admin_token) = new_org_admin(&f.state, acme, "acme-admin").await;

    let (status, _) = send(&f.app, "POST", &format!("/api/organizations/{acme}/users"), &org_admin_token, Some(serde_json::json!({ "email": "member@acme.example" }))).await;

    assert_eq!(status, StatusCode::CREATED);
    let invited = the_only(&f.state, "UserInvited").await;
    assert_eq!(invited.actor_id, Some(org_admin_id));
    assert_eq!(invited.organization_id, Some(acme));
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn granting_and_revoking_super_admin_are_recorded_but_a_refused_change_is_not(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let (member_id, _) = new_user(&f.state, public_org(), "member").await;
    let uri = format!("/api/users/{member_id}/super-admin");

    assert_eq!(send(&f.app, "PUT", &uri, &f.admin_token, Some(serde_json::json!({ "is_super_admin": true }))).await.0, StatusCode::NO_CONTENT);
    let granted = the_only(&f.state, "SuperAdminGranted").await;
    assert_eq!(granted.actor_id, Some(f.admin_id));
    assert_eq!(granted.payload["user_id"], member_id.to_string());

    assert_eq!(send(&f.app, "PUT", &uri, &f.admin_token, Some(serde_json::json!({ "is_super_admin": false }))).await.0, StatusCode::NO_CONTENT);
    the_only(&f.state, "SuperAdminRevoked").await;

    let refused = send(&f.app, "PUT", &format!("/api/users/{}/super-admin", f.admin_id), &f.admin_token, Some(serde_json::json!({ "is_super_admin": false }))).await;
    assert_eq!(refused.0, StatusCode::CONFLICT, "the last super-admin cannot be demoted");
    assert_eq!(entries_of(&f.state, "SuperAdminRevoked").await.len(), 1, "a refused demotion leaves no entry");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn granting_and_revoking_organization_admin_are_recorded_once_per_actual_change(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let (member_id, _) = new_user(&f.state, acme, "member").await;
    let uri = format!("/api/organizations/{acme}/users/{member_id}/organization-admin");

    send(&f.app, "PUT", &uri, &f.admin_token, Some(serde_json::json!({ "is_organization_admin": true }))).await;
    send(&f.app, "PUT", &uri, &f.admin_token, Some(serde_json::json!({ "is_organization_admin": true }))).await;
    send(&f.app, "PUT", &uri, &f.admin_token, Some(serde_json::json!({ "is_organization_admin": false }))).await;

    let granted = the_only(&f.state, "OrganizationAdminGranted").await;
    assert_eq!(granted.organization_id, Some(acme));
    assert_eq!(granted.payload["user_id"], member_id.to_string());
    the_only(&f.state, "OrganizationAdminRevoked").await;
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn creating_an_organization_is_recorded(pool: sqlx::PgPool) {
    let f = fixture(pool).await;

    let (status, created) = send(&f.app, "POST", "/api/organizations", &f.admin_token, Some(serde_json::json!({ "slug": "acme", "display_name": "Acme Corp" }))).await;

    assert_eq!(status, StatusCode::CREATED);
    let entry = the_only(&f.state, "OrganizationCreated").await;
    assert_eq!(entry.organization_id, Some(Uuid::parse_str(created["id"].as_str().unwrap()).unwrap()));
    assert_eq!(entry.actor_id, Some(f.admin_id));
    assert_eq!(entry.payload["slug"], "acme");
    assert_eq!(entry.payload["display_name"], "Acme Corp");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn swapping_and_clearing_an_identity_provider_are_recorded_without_the_secret(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let (org_admin_id, org_admin_token) = new_org_admin(&f.state, acme, "acme-admin").await;
    let uri = format!("/api/organizations/{acme}/identity-provider");

    let first = serde_json::json!({ "type": "oidc", "issuer_url": "https://idp.example", "client_id": "artiferris", "client_secret": SECRET });
    assert_eq!(send(&f.app, "PUT", &uri, &org_admin_token, Some(first)).await.0, StatusCode::NO_CONTENT);
    // A stored secret is not carried over to another issuer, so the swap has to bring its own.
    let swapped = serde_json::json!({ "type": "oidc", "issuer_url": "https://attacker.example", "client_id": "evil", "client_secret": SECRET });
    assert_eq!(send(&f.app, "PUT", &uri, &org_admin_token, Some(swapped)).await.0, StatusCode::NO_CONTENT);
    let kept = serde_json::json!({ "type": "oidc", "issuer_url": "https://attacker.example", "client_id": "evil" });
    assert_eq!(send(&f.app, "PUT", &uri, &org_admin_token, Some(kept)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&f.app, "DELETE", &uri, &org_admin_token, None).await.0, StatusCode::NO_CONTENT);

    let mut sets = entries_of(&f.state, "IdentityProviderSet").await;
    sets.sort_by_key(|entry| entry.occurred_at);
    assert_eq!(sets.len(), 3);
    assert_eq!(sets[0].actor_id, Some(org_admin_id));
    assert_eq!(sets[0].organization_id, Some(acme));
    assert!(sets[0].payload["before"].is_null());
    assert_eq!(sets[0].payload["after"]["issuer_url"], "https://idp.example");
    assert_eq!(sets[0].payload["after"]["client_id"], "artiferris");
    assert_eq!(sets[0].payload["secret_changed"], true);
    assert_eq!(sets[1].payload["before"]["issuer_url"], "https://idp.example");
    assert_eq!(sets[1].payload["after"]["issuer_url"], "https://attacker.example");
    assert_eq!(sets[1].payload["secret_changed"], true);
    assert_eq!(sets[2].payload["after"]["issuer_url"], "https://attacker.example");
    assert_eq!(sets[2].payload["secret_changed"], false, "no new secret was supplied");
    let cleared = the_only(&f.state, "IdentityProviderCleared").await;
    assert_eq!(cleared.payload["before"]["issuer_url"], "https://attacker.example");
    for entry in sets.iter().chain(std::iter::once(&cleared)) {
        assert_no_secret(entry);
    }
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_ldap_provider_change_is_recorded_without_the_bind_password(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let body = serde_json::json!({
        "type": "ldap", "server_url": "ldaps://dc.example:636", "bind_dn": "cn=svc,dc=example", "bind_password": SECRET,
        "user_search_base": "ou=people,dc=example", "user_search_filter": "(uid={username})", "email_attribute": "mail"
    });

    let (status, _) = send(&f.app, "PUT", &format!("/api/organizations/{acme}/identity-provider"), &f.admin_token, Some(body)).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    let entry = the_only(&f.state, "IdentityProviderSet").await;
    assert_eq!(entry.payload["after"]["provider"], "ldap");
    assert_eq!(entry.payload["after"]["bind_dn"], "cn=svc,dc=example");
    assert_no_secret(&entry);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn smtp_changes_are_recorded_without_the_password(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let settings = |host: &str, password: Option<&str>| {
        let mut body = serde_json::json!({ "host": host, "port": 587, "username": "mailer", "from_name": "ArtiFerris", "from_address": "noreply@example.com", "security": "start_tls" });
        if let Some(password) = password {
            body["password"] = password.into();
        }
        body
    };

    assert_eq!(send(&f.app, "PUT", "/api/admin/settings/smtp", &f.admin_token, Some(settings("smtp.example.com", Some(SECRET)))).await.0, StatusCode::NO_CONTENT);
    // A stored password is not carried over to another host, so the move has to bring its own.
    assert_eq!(send(&f.app, "PUT", "/api/admin/settings/smtp", &f.admin_token, Some(settings("smtp.other.example", Some(SECRET)))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&f.app, "PUT", "/api/admin/settings/smtp", &f.admin_token, Some(settings("smtp.other.example", None))).await.0, StatusCode::NO_CONTENT);

    let mut changes = entries_of(&f.state, "SmtpSettingsChanged").await;
    changes.sort_by_key(|entry| entry.occurred_at);
    assert_eq!(changes.len(), 3);
    assert!(changes[0].payload["before"].is_null());
    assert_eq!(changes[0].payload["after"]["host"], "smtp.example.com");
    assert_eq!(changes[0].payload["password_changed"], true);
    assert_eq!(changes[1].payload["before"]["host"], "smtp.example.com");
    assert_eq!(changes[1].payload["after"]["host"], "smtp.other.example");
    assert_eq!(changes[1].payload["password_changed"], true);
    assert_eq!(changes[2].payload["before"]["host"], "smtp.other.example");
    assert_eq!(changes[2].payload["password_changed"], false);
    assert!(changes.iter().all(|entry| entry.actor_id == Some(f.admin_id) && entry.organization_id == Some(public_org())));
    changes.iter().for_each(assert_no_secret);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn system_settings_changes_list_what_changed_and_nothing_when_nothing_did(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let defaults = artiferris_domain::system_settings::SystemSettings::defaults();
    let with = |registration: bool, seo: bool| serde_json::json!({ "max_login_attempts": defaults.max_login_attempts, "login_attempt_window_seconds": defaults.login_attempt_window_seconds, "session_ttl_hours": defaults.session_ttl_hours, "registration_enabled": registration, "seo_indexing_enabled": seo });

    send(&f.app, "PUT", "/api/admin/settings", &f.admin_token, Some(with(true, false))).await;
    assert!(entries_of(&f.state, "SystemSettingsChanged").await.is_empty(), "saving the current values changes nothing");
    assert_eq!(send(&f.app, "PUT", "/api/admin/settings", &f.admin_token, Some(with(false, true))).await.0, StatusCode::NO_CONTENT);

    let entry = the_only(&f.state, "SystemSettingsChanged").await;
    assert_eq!(entry.actor_id, Some(f.admin_id));
    let changes = entry.payload["changes"].as_array().unwrap();
    let names: Vec<_> = changes.iter().map(|c| c["setting"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["registration_enabled", "seo_indexing_enabled"]);
    assert_eq!(changes[0]["before"], true);
    assert_eq!(changes[0]["after"], false);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_organization_admin_cannot_flip_seo_indexing_and_it_is_not_recorded_as_flipped(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let (_, org_admin_token) = new_org_admin(&f.state, acme, "acme-admin").await;
    let defaults = artiferris_domain::system_settings::SystemSettings::defaults();
    let body = serde_json::json!({ "max_login_attempts": 3, "login_attempt_window_seconds": defaults.login_attempt_window_seconds, "session_ttl_hours": defaults.session_ttl_hours, "registration_enabled": true, "seo_indexing_enabled": true });

    assert_eq!(send(&f.app, "PUT", "/api/admin/settings", &org_admin_token, Some(body)).await.0, StatusCode::NO_CONTENT);

    let entry = the_only(&f.state, "SystemSettingsChanged").await;
    assert_eq!(entry.organization_id, Some(acme));
    let names: Vec<_> = entry.payload["changes"].as_array().unwrap().iter().map(|c| c["setting"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, vec!["max_login_attempts"]);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn branding_changes_are_recorded(pool: sqlx::PgPool) {
    let f = fixture(pool).await;

    assert_eq!(send(&f.app, "DELETE", "/api/admin/branding/logo", &f.admin_token, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&f.app, "DELETE", "/api/admin/branding/favicon", &f.admin_token, None).await.0, StatusCode::NO_CONTENT);

    let mut recorded = entries_of(&f.state, "BrandingChanged").await;
    recorded.sort_by_key(|entry| entry.occurred_at);
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].payload["asset"], "logo");
    assert_eq!(recorded[0].payload["cleared"], true);
    assert_eq!(recorded[1].payload["asset"], "favicon");
    assert_eq!(recorded[0].organization_id, Some(public_org()));
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn configuration_export_and_import_are_recorded_as_instance_level_counts(pool: sqlx::PgPool) {
    let f = fixture(pool).await;

    let (status, _) = send(&f.app, "GET", "/api/admin/export/configuration", &f.admin_token, None).await;
    assert_eq!(status, StatusCode::OK);
    let exported = the_only(&f.state, "ConfigurationExported").await;
    assert_eq!(exported.actor_id, Some(f.admin_id));
    assert_eq!(exported.organization_id, None);
    assert_eq!(exported.payload["users"], 1);
    assert_eq!(exported.payload["repositories"], 0);

    let import = serde_json::json!({
        "users": [{ "id": Uuid::new_v4(), "username": "restored", "is_super_admin": false, "created_at": chrono::Utc::now(), "email": null }],
        "repositories": [], "permissions": [],
        "system_settings": { "max_login_attempts": 10, "login_attempt_window_seconds": 300, "session_ttl_hours": 12 }
    });
    let (status, _) = send(&f.app, "POST", "/api/admin/import/configuration", &f.admin_token, Some(import)).await;
    assert_eq!(status, StatusCode::OK);
    let imported = the_only(&f.state, "ConfigurationImported").await;
    assert_eq!(imported.organization_id, None);
    assert_eq!(imported.payload["users_created"], 1);
    assert_eq!(imported.payload["failures"], 0);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn api_tokens_created_and_revoked_are_recorded_without_the_token_value(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let (member_id, member_token) = new_user(&f.state, public_org(), "member").await;

    let (status, created) = send(&f.app, "POST", "/api/tokens", &member_token, Some(serde_json::json!({ "label": "ci" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (token_id, raw_token) = (created["id"].as_str().unwrap().to_string(), created["token"].as_str().unwrap().to_string());
    let entry = the_only(&f.state, "ApiTokenCreated").await;
    assert_eq!(entry.actor_id, Some(member_id));
    assert_eq!(entry.payload["token_id"], token_id);
    assert_eq!(entry.payload["label"], "ci");
    assert!(!entry.payload.to_string().contains(&raw_token));

    assert_eq!(send(&f.app, "DELETE", &format!("/api/tokens/{token_id}"), &member_token, None).await.0, StatusCode::NO_CONTENT);
    let revoked = the_only(&f.state, "ApiTokenRevoked").await;
    assert_eq!(revoked.actor_id, Some(member_id));
    assert_eq!(revoked.payload["user_id"], member_id.to_string());

    let (_, second) = send(&f.app, "POST", "/api/tokens", &member_token, Some(serde_json::json!({ "label": "deploy" }))).await;
    let second_id = second["id"].as_str().unwrap();
    assert_eq!(send(&f.app, "DELETE", &format!("/api/admin/tokens/{second_id}"), &f.admin_token, None).await.0, StatusCode::NO_CONTENT);
    let by_admin = entries_of(&f.state, "ApiTokenRevoked").await.into_iter().find(|e| e.actor_id == Some(f.admin_id)).expect("the admin's revocation is recorded");
    assert_eq!(by_admin.payload["user_id"], member_id.to_string(), "the owner, not the revoker");
    assert_eq!(by_admin.organization_id, Some(public_org()));
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn totp_disable_and_passkey_removal_are_recorded(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let (member_id, member_token) = new_user(&f.state, public_org(), "member").await;
    let enrollment = f.state.enroll_totp.execute(member_id, "member", Some(PASSWORD)).await.unwrap();
    let code = artiferris_application::use_cases::mfa::generate_current_totp_code(&enrollment.secret_base32);
    f.state.confirm_totp.execute(member_id, "member", &code).await.unwrap();
    let passkey_id = Uuid::new_v4();
    f.state
        .webauthn_credentials
        .insert(&artiferris_domain::webauthn::WebauthnCredential { id: passkey_id, user_id: member_id, name: "key".to_string(), passkey_data: vec![0u8; 8], created_at: chrono::Utc::now() }, None)
        .await
        .unwrap();

    let body = serde_json::json!({ "current_password": PASSWORD });
    assert_eq!(send(&f.app, "DELETE", &format!("/api/me/mfa/passkey/{passkey_id}"), &member_token, Some(body.clone())).await.0, StatusCode::NO_CONTENT);
    // Removing a factor ends the sessions issued before it, so the second removal needs a fresh one.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let fresh_token = f.state.authenticate_user.execute("member", PASSWORD).await.unwrap();
    assert_eq!(send(&f.app, "DELETE", "/api/me/mfa/totp", &fresh_token, Some(body)).await.0, StatusCode::NO_CONTENT);

    let removed = the_only(&f.state, "PasskeyDeleted").await;
    assert_eq!(removed.payload["passkey_id"], passkey_id.to_string());
    assert_eq!(removed.actor_id, Some(member_id));
    let disabled = the_only(&f.state, "MfaDisabled").await;
    assert_eq!(disabled.payload["method"], "totp");
    assert_eq!(disabled.organization_id, Some(public_org()));
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn confirming_totp_from_the_account_page_is_recorded(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let (member_id, member_token) = new_user(&f.state, public_org(), "member").await;
    let (_, enrolled) = send(&f.app, "POST", "/api/me/mfa/totp/enroll", &member_token, Some(serde_json::json!({ "current_password": PASSWORD }))).await;
    let code = artiferris_application::use_cases::mfa::generate_current_totp_code(enrolled["secret"].as_str().unwrap());

    let (status, _) = send(&f.app, "POST", "/api/me/mfa/totp/confirm", &member_token, Some(serde_json::json!({ "code": code }))).await;

    assert_eq!(status, StatusCode::OK);
    let enabled = the_only(&f.state, "MfaEnabled").await;
    assert_eq!(enabled.actor_id, Some(member_id));
    assert_eq!(enabled.payload["method"], "totp");
    assert_no_secret(&enabled);
    assert!(!enabled.payload.to_string().contains(enrolled["secret"].as_str().unwrap()));
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_failing_audit_write_does_not_fail_an_operation_that_is_not_atomic_with_it(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    sqlx::query("ALTER TABLE domain_events RENAME TO domain_events_gone").execute(&pool).await.unwrap();

    let (status, created) = send(&f.app, "POST", "/api/organizations", &f.admin_token, Some(serde_json::json!({ "slug": "acme", "display_name": "Acme" }))).await;

    assert_eq!(status, StatusCode::CREATED);
    let id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    assert!(f.state.organizations.find_by_id(id).await.unwrap().is_some(), "the change itself went through");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_organization_admin_sees_their_admin_and_security_events_but_not_another_organizations_or_instance_level_ones(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let globex = f.state.create_organization.execute("globex", "Globex").await.unwrap();
    let (_, acme_token) = new_org_admin(&f.state, acme, "acme-admin").await;
    let (globex_admin_id, _) = new_org_admin(&f.state, globex, "globex-admin").await;
    send(&f.app, "POST", &format!("/api/organizations/{acme}/users"), &acme_token, Some(serde_json::json!({ "email": "m@acme.example" }))).await;
    send(&f.app, "POST", &format!("/api/organizations/{globex}/users"), &f.admin_token, Some(serde_json::json!({ "email": "m@globex.example" }))).await;
    f.state.record_admin_event.execute(artiferris_domain::audit::AdminAuditEvent::ConfigurationExported { users: 1, repositories: 0, permissions: 0 }, Some(f.admin_id)).await.unwrap();
    f.state.record_security_event.execute(artiferris_domain::audit::SecurityEvent::LoginFailed { username: "nobody".to_string(), ip: "10.0.0.1".to_string() }, None).await.unwrap();
    f.state.record_security_event.execute(artiferris_domain::audit::SecurityEvent::PasswordChanged { user_id: globex_admin_id, organization_id: globex }, Some(globex_admin_id)).await.unwrap();

    let (status, page) = send(&f.app, "GET", "/api/audit/events", &acme_token, None).await;

    assert_eq!(status, StatusCode::OK);
    let types: Vec<&str> = page["entries"].as_array().unwrap().iter().map(|e| e["event_type"].as_str().unwrap()).collect();
    assert!(types.contains(&"UserInvited"));
    assert!(!types.contains(&"ConfigurationExported"), "instance-level events are not any organization's");
    assert!(!types.contains(&"LoginFailed"));
    assert!(!types.contains(&"PasswordChanged"), "another organization's security events stay out");
    let invited: Vec<_> = page["entries"].as_array().unwrap().iter().filter(|e| e["event_type"] == "UserInvited").collect();
    assert_eq!(invited.len(), 1);
    assert_eq!(invited[0]["payload"]["email"], "m@acme.example");

    let (_, everything) = send(&f.app, "GET", "/api/audit/events", &f.admin_token, None).await;
    let all_types: Vec<&str> = everything["entries"].as_array().unwrap().iter().map(|e| e["event_type"].as_str().unwrap()).collect();
    for expected in ["ConfigurationExported", "LoginFailed", "PasswordChanged"] {
        assert!(all_types.contains(&expected), "the super-admin's unscoped view shows {expected}");
    }
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_quiet_organization_still_sees_its_events_behind_a_flood_from_others(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let (_, acme_token) = new_org_admin(&f.state, acme, "acme-admin").await;
    send(&f.app, "POST", &format!("/api/organizations/{acme}/users"), &acme_token, Some(serde_json::json!({ "email": "m@acme.example" }))).await;
    sqlx::query(
        "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, organization_id) \
         SELECT 'Admin', gen_random_uuid()::text, 'UserInvited', '{}'::jsonb, 1, gen_random_uuid() FROM generate_series(1, 300)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (_, page) = send(&f.app, "GET", "/api/audit/events", &acme_token, None).await;

    assert!(page["entries"].as_array().unwrap().iter().any(|e| e["event_type"] == "UserInvited" && e["payload"]["email"] == "m@acme.example"));
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn the_audit_endpoint_pages_through_the_log_with_an_opaque_cursor(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    sqlx::query(
        "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, occurred_at) \
         SELECT 'Security', gen_random_uuid()::text, 'LoginFailed', '{}'::jsonb, 1, now() - make_interval(secs => g) FROM generate_series(1, 7) g",
    )
    .execute(&pool)
    .await
    .unwrap();

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let uri = match &cursor {
            Some(cursor) => format!("/api/audit/events?aggregate_type=Security&limit=3&cursor={cursor}"),
            None => "/api/audit/events?aggregate_type=Security&limit=3".to_string(),
        };
        let (status, page) = send(&f.app, "GET", &uri, &f.admin_token, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(page["entries"].as_array().unwrap().len() <= 3);
        seen.extend(page["entries"].as_array().unwrap().iter().map(|e| e["occurred_at"].as_str().unwrap().to_string()));
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }

    assert_eq!(seen.len(), 7);
    let mut newest_first = seen.clone();
    newest_first.sort_by(|a, b| b.cmp(a));
    assert_eq!(seen, newest_first);
    newest_first.dedup();
    assert_eq!(newest_first.len(), 7, "no entry repeats across pages");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_malformed_cursor_is_a_bad_request_and_the_limit_is_capped(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    sqlx::query(
        "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version) \
         SELECT 'Security', gen_random_uuid()::text, 'LoginFailed', '{}'::jsonb, 1 FROM generate_series(1, 210)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, _) = send(&f.app, "GET", "/api/audit/events?cursor=not-a-cursor", &f.admin_token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, page) = send(&f.app, "GET", "/api/audit/events?limit=100000", &f.admin_token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["entries"].as_array().unwrap().len(), 200);
    assert!(page["next_cursor"].is_string());
}

async fn every_page(app: &Router, token: &str, query: &str, limit: usize) -> Vec<serde_json::Value> {
    let mut entries = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut uri = format!("/api/audit/events?limit={limit}{query}");
        if let Some(cursor) = &cursor {
            uri.push_str(&format!("&cursor={cursor}"));
        }
        let (status, page) = send(app, "GET", &uri, token, None).await;
        assert_eq!(status, StatusCode::OK);
        entries.extend(page["entries"].as_array().unwrap().iter().cloned());
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => return entries,
        }
    }
}

fn types_of(entries: &[serde_json::Value]) -> Vec<&str> {
    entries.iter().map(|e| e["event_type"].as_str().unwrap()).collect()
}

/// One story through the HTTP surface only: an organization admin whose account is misused, everything they and the instance admin do, what each of them can read back,
/// and then the log ageing out without touching the repository history that lives in the same table.
#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_organization_admins_actions_leave_a_scoped_secret_free_trail_that_ages_out_without_touching_repository_history(pool: sqlx::PgPool) {
    let state = AppState::build(pool.clone(), &Config { audit_retention_days: Some(365), ..test_config() });
    state.create_user.execute(public_org(), "root", PASSWORD, true).await.unwrap();
    let root = state.authenticate_user.execute("root", PASSWORD).await.unwrap();
    let app = build_router(state.clone());

    let (status, acme) = send(&app, "POST", "/api/organizations", &root, Some(serde_json::json!({ "slug": "acme", "display_name": "Acme" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let acme = Uuid::parse_str(acme["id"].as_str().unwrap()).unwrap();
    let (_, acme_admin) = new_org_admin(&state, acme, "acme-admin").await;
    let (_, member) = new_user(&state, public_org(), "member").await;

    let idp = serde_json::json!({ "type": "oidc", "issuer_url": "https://attacker.example", "client_id": "evil", "client_secret": SECRET });
    assert_eq!(send(&app, "PUT", &format!("/api/organizations/{acme}/identity-provider"), &acme_admin, Some(idp)).await.0, StatusCode::NO_CONTENT);
    let invite = serde_json::json!({ "email": "member@acme.example" });
    assert_eq!(send(&app, "POST", &format!("/api/organizations/{acme}/users"), &acme_admin, Some(invite)).await.0, StatusCode::CREATED);
    let repository = serde_json::json!({ "name": "libs", "format": "npm", "repo_type": "hosted", "remote_url": null });
    assert_eq!(send(&app, "POST", "/api/repositories", &root, Some(repository)).await.0, StatusCode::CREATED);
    let settings = serde_json::json!({ "max_login_attempts": 7, "login_attempt_window_seconds": 60, "session_ttl_hours": 4 });
    assert_eq!(send(&app, "PUT", "/api/admin/settings", &root, Some(settings)).await.0, StatusCode::NO_CONTENT);
    let (status, token) = send(&app, "POST", "/api/tokens", &member, Some(serde_json::json!({ "label": "ci" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let raw_token = token["token"].as_str().unwrap().to_string();
    assert_eq!(send(&app, "POST", "/api/auth/logout-all", &member, None).await.0, StatusCode::NO_CONTENT);

    // The organization admin reads back their own organization's trail: the swapped provider and the invitation, nothing from the public organization.
    let theirs = every_page(&app, &acme_admin, "", 200).await;
    let their_types = types_of(&theirs);
    assert!(their_types.contains(&"IdentityProviderSet") && their_types.contains(&"UserInvited"), "got {their_types:?}");
    for foreign in ["SystemSettingsChanged", "ApiTokenCreated", "SessionsRevoked", "Created"] {
        assert!(!their_types.contains(&foreign), "{foreign} belongs to another organization");
    }
    let raw_log = serde_json::to_string(&theirs).unwrap();
    assert!(!raw_log.contains(SECRET), "the identity provider's secret came back out of the log");

    // Paging one entry at a time walks exactly the same log, newest first, with no repeats.
    let one_by_one = every_page(&app, &acme_admin, "", 1).await;
    assert_eq!(one_by_one.iter().map(|e| e["id"].clone()).collect::<Vec<_>>(), theirs.iter().map(|e| e["id"].clone()).collect::<Vec<_>>());
    assert!(theirs.windows(2).all(|pair| pair[0]["occurred_at"].as_str() >= pair[1]["occurred_at"].as_str()));

    // The instance admin sees the whole story.
    let everything = every_page(&app, &root, "", 3).await;
    let all_types = types_of(&everything);
    for expected in ["OrganizationCreated", "IdentityProviderSet", "UserInvited", "SystemSettingsChanged", "ApiTokenCreated", "SessionsRevoked"] {
        assert!(all_types.contains(&expected), "missing {expected} in {all_types:?}");
    }
    let raw_everything = serde_json::to_string(&everything).unwrap();
    assert!(!raw_everything.contains(SECRET) && !raw_everything.contains(&raw_token), "a secret leaked into the instance-wide log");
    let repository_events_before = every_page(&app, &root, "&aggregate_type=PackageRepository", 50).await.len();
    assert!(repository_events_before >= 1, "creating a repository is part of the repository's own history");

    // A year later: the security and administrative events from the first two steps are old, the rest are recent.
    sqlx::query("UPDATE domain_events SET occurred_at = now() - interval '400 days' WHERE event_type IN ('OrganizationCreated', 'IdentityProviderSet')").execute(&pool).await.unwrap();
    sqlx::query("UPDATE domain_events SET occurred_at = now() - interval '400 days' WHERE aggregate_type = 'PackageRepository'").execute(&pool).await.unwrap();

    let removed = state.prune_audit_events.execute().await.unwrap();

    assert_eq!(removed, 2, "only the old Admin events go, not the old repository history");
    let after = every_page(&app, &root, "", 200).await;
    let after_types = types_of(&after);
    assert!(!after_types.contains(&"OrganizationCreated") && !after_types.contains(&"IdentityProviderSet"));
    for kept in ["UserInvited", "SystemSettingsChanged", "ApiTokenCreated", "SessionsRevoked"] {
        assert!(after_types.contains(&kept), "{kept} is recent and stays");
    }
    assert_eq!(every_page(&app, &root, "&aggregate_type=PackageRepository", 50).await.len(), repository_events_before);
    let (status, repositories) = send(&app, "GET", "/api/repositories", &root, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(repositories.as_array().unwrap().iter().any(|r| r["name"] == "libs"), "the repository still loads from its events after the sweep");
}

async fn break_audit_writes(pool: &sqlx::PgPool) {
    sqlx::query("ALTER TABLE domain_events RENAME TO domain_events_gone").execute(pool).await.unwrap();
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_identity_provider_change_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let uri = format!("/api/organizations/{acme}/identity-provider");
    let first = serde_json::json!({ "type": "oidc", "issuer_url": "https://idp.example", "client_id": "artiferris", "client_secret": SECRET });
    assert_eq!(send(&f.app, "PUT", &uri, &f.admin_token, Some(first)).await.0, StatusCode::NO_CONTENT);
    break_audit_writes(&pool).await;

    let swapped = serde_json::json!({ "type": "oidc", "issuer_url": "https://other.example", "client_id": "evil", "client_secret": SECRET });
    let (set_status, _) = send(&f.app, "PUT", &uri, &f.admin_token, Some(swapped)).await;
    let (clear_status, _) = send(&f.app, "DELETE", &uri, &f.admin_token, None).await;

    assert!(set_status.is_server_error() && clear_status.is_server_error(), "{set_status} {clear_status}");
    match f.state.identity_providers.get(acme).await.unwrap() {
        Some(artiferris_domain::sso::IdentityProviderConfig::Oidc(oidc)) => assert_eq!(oidc.issuer_url, "https://idp.example", "the original provider is untouched"),
        other => panic!("expected the original provider, got {other:?}"),
    }
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_smtp_change_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    break_audit_writes(&pool).await;
    let body = serde_json::json!({ "host": "smtp.example.com", "port": 587, "username": "mailer", "password": SECRET, "from_name": "ArtiFerris", "from_address": "noreply@example.com", "security": "start_tls" });

    let (status, _) = send(&f.app, "PUT", "/api/admin/settings/smtp", &f.admin_token, Some(body)).await;

    assert!(status.is_server_error(), "{status}");
    assert!(f.state.get_smtp_settings.execute(public_org()).await.unwrap().is_none(), "nothing was stored");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_organization_admin_grant_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let acme = f.state.create_organization.execute("acme", "Acme").await.unwrap();
    let (member_id, _) = new_user(&f.state, acme, "member").await;
    break_audit_writes(&pool).await;

    let (status, _) = send(&f.app, "PUT", &format!("/api/organizations/{acme}/users/{member_id}/organization-admin"), &f.admin_token, Some(serde_json::json!({ "is_organization_admin": true }))).await;

    assert!(status.is_server_error(), "{status}");
    assert!(!f.state.users.find_by_id(member_id).await.unwrap().unwrap().is_organization_admin);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_admin_token_revocation_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let (_, member_token) = new_user(&f.state, public_org(), "member").await;
    let (_, created) = send(&f.app, "POST", "/api/tokens", &member_token, Some(serde_json::json!({ "label": "ci" }))).await;
    let token_id = created["id"].as_str().unwrap().to_string();
    break_audit_writes(&pool).await;

    let (status, _) = send(&f.app, "DELETE", &format!("/api/admin/tokens/{token_id}"), &f.admin_token, None).await;

    assert!(status.is_server_error(), "{status}");
    let tokens = f.state.admin_list_api_tokens.execute(None, 500, 0).await.unwrap();
    assert!(tokens.iter().find(|t| t.id.to_string() == token_id).unwrap().revoked_at.is_none(), "the token still works");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_user_deletion_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let (member_id, _) = new_user(&f.state, public_org(), "member").await;
    break_audit_writes(&pool).await;

    let (status, _) = send(&f.app, "DELETE", &format!("/api/users/{member_id}"), &f.admin_token, None).await;

    assert!(status.is_server_error(), "{status}");
    assert!(f.state.users.find_by_id(member_id).await.unwrap().is_some(), "the user is still there");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_system_settings_change_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    break_audit_writes(&pool).await;
    let defaults = artiferris_domain::system_settings::SystemSettings::defaults();
    let body = serde_json::json!({ "max_login_attempts": 5, "login_attempt_window_seconds": defaults.login_attempt_window_seconds, "session_ttl_hours": defaults.session_ttl_hours, "registration_enabled": true, "seo_indexing_enabled": false });

    let (status, _) = send(&f.app, "PUT", "/api/admin/settings", &f.admin_token, Some(body)).await;

    assert!(status.is_server_error(), "{status}");
    assert_eq!(f.state.get_system_settings.execute(public_org()).await.unwrap().max_login_attempts, defaults.max_login_attempts);
}

async fn put_bytes(app: &Router, uri: &str, token: &str, bytes: Vec<u8>) -> StatusCode {
    let request = Request::builder().method("PUT").uri(uri).header("authorization", format!("Bearer {token}")).body(Body::from(bytes)).unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_branding_change_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let png = [&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A][..], &[0u8; 16][..]].concat();
    assert_eq!(put_bytes(&f.app, "/api/admin/branding/logo", &f.admin_token, png.clone()).await, StatusCode::NO_CONTENT);
    break_audit_writes(&pool).await;

    let replaced = put_bytes(&f.app, "/api/admin/branding/logo", &f.admin_token, [png.clone(), vec![1, 2, 3]].concat()).await;
    let (cleared, _) = send(&f.app, "DELETE", "/api/admin/branding/logo", &f.admin_token, None).await;
    let favicon = put_bytes(&f.app, "/api/admin/branding/favicon", &f.admin_token, png.clone()).await;

    assert!(replaced.is_server_error() && cleared.is_server_error() && favicon.is_server_error(), "{replaced} {cleared} {favicon}");
    let stored = f.state.get_branding.execute(public_org()).await.unwrap();
    assert_eq!(stored.logo.bytes, png, "the first logo is untouched");
    assert_ne!(stored.favicon.bytes, png, "no favicon was stored");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_invitation_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    break_audit_writes(&pool).await;

    let (status, _) = send(&f.app, "POST", "/api/users", &f.admin_token, Some(serde_json::json!({ "email": "invitee@example.com", "is_super_admin": false, "is_organization_admin": false }))).await;

    assert!(status.is_server_error(), "{status}");
    let username = artiferris_domain::user::Username::parse("invitee").unwrap();
    assert!(f.state.users.find_by_username(&username).await.unwrap().is_none(), "no account without its invitation and its audit entry");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_resent_invitation_keeps_the_old_link_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let (_, created) = send(&f.app, "POST", "/api/users", &f.admin_token, Some(serde_json::json!({ "email": "invitee@example.com", "is_super_admin": false, "is_organization_admin": false }))).await;
    let user_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    let before = f.state.user_invitations.find_by_user_id(user_id).await.unwrap().unwrap();
    break_audit_writes(&pool).await;

    let (status, _) = send(&f.app, "POST", &format!("/api/users/{user_id}/resend-invitation"), &f.admin_token, None).await;

    assert!(status.is_server_error(), "{status}");
    assert_eq!(f.state.user_invitations.find_by_user_id(user_id).await.unwrap().unwrap().token_hash, before.token_hash);
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn an_activation_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let (_, created) = send(&f.app, "POST", "/api/users", &f.admin_token, Some(serde_json::json!({ "email": "invitee@example.com", "is_super_admin": false, "is_organization_admin": false }))).await;
    let user_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    let token = "known-activation-token";
    f.state
        .user_invitations
        .upsert(&artiferris_domain::invitation::UserInvitation { user_id, token_hash: artiferris_application::use_cases::invitation::hash_invitation_token(token), expires_at: chrono::Utc::now() + chrono::Duration::hours(1) }, None)
        .await
        .unwrap();
    let hash_before = f.state.users.find_by_id(user_id).await.unwrap().unwrap().password_hash;
    break_audit_writes(&pool).await;

    let (status, _) = send(&f.app, "POST", "/api/auth/activate", "", Some(serde_json::json!({ "token": token, "username": "chosen-name", "new_password": "an0ther-s3cret!" }))).await;

    assert!(status.is_server_error(), "{status}");
    assert_eq!(f.state.users.find_by_id(user_id).await.unwrap().unwrap().password_hash, hash_before, "the password did not change");
    assert!(f.state.user_invitations.find_by_user_id(user_id).await.unwrap().is_some(), "the mailed link still works");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_configuration_export_is_not_served_when_it_cannot_be_recorded(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    break_audit_writes(&pool).await;

    let request = Request::builder().method("GET").uri("/api/admin/export/configuration").header("authorization", format!("Bearer {}", f.admin_token)).body(Body::empty()).unwrap();
    let response = f.app.clone().oneshot(request).await.unwrap();

    assert!(response.status().is_server_error(), "{}", response.status());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("root"), "no user list in the failure");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_password_change_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let (member_id, member_token) = new_user(&f.state, public_org(), "member").await;
    let hash_before = f.state.users.find_by_id(member_id).await.unwrap().unwrap().password_hash;
    break_audit_writes(&pool).await;

    let (status, _) = send(&f.app, "PUT", "/api/me/password", &member_token, Some(serde_json::json!({ "current_password": PASSWORD, "new_password": "an0ther-s3cret!" }))).await;

    assert!(status.is_server_error(), "{status}");
    assert_eq!(f.state.users.find_by_id(member_id).await.unwrap().unwrap().password_hash, hash_before);
    assert!(f.state.authenticate_user.execute("member", PASSWORD).await.is_ok(), "the old password still works");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn signing_out_everywhere_is_rolled_back_when_its_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let (member_id, member_token) = new_user(&f.state, public_org(), "member").await;
    let valid_after_before = f.state.users.find_by_id(member_id).await.unwrap().unwrap().tokens_valid_after;
    break_audit_writes(&pool).await;

    let (status, _) = send(&f.app, "POST", "/api/auth/logout-all", &member_token, None).await;

    assert!(status.is_server_error(), "{status}");
    assert_eq!(f.state.users.find_by_id(member_id).await.unwrap().unwrap().tokens_valid_after, valid_after_before, "the sessions were not ended");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn new_backup_codes_and_a_passkey_removal_are_rolled_back_when_their_audit_row_cannot_be_written(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let (member_id, member_token) = new_user(&f.state, public_org(), "member").await;
    let enrollment = f.state.enroll_totp.execute(member_id, "member", Some(PASSWORD)).await.unwrap();
    let code = artiferris_application::use_cases::mfa::generate_current_totp_code(&enrollment.secret_base32);
    f.state.confirm_totp.execute(member_id, "member", &code).await.unwrap();
    let passkey_id = Uuid::new_v4();
    f.state
        .webauthn_credentials
        .insert(&artiferris_domain::webauthn::WebauthnCredential { id: passkey_id, user_id: member_id, name: "key".to_string(), passkey_data: vec![0u8; 8], created_at: chrono::Utc::now() }, None)
        .await
        .unwrap();
    let hashes_before: Vec<String> = sqlx::query_scalar("SELECT code_hash FROM mfa_backup_codes WHERE user_id = $1 ORDER BY code_hash").bind(member_id).fetch_all(&pool).await.unwrap();
    break_audit_writes(&pool).await;
    let body = serde_json::json!({ "current_password": PASSWORD });

    let (regenerated, _) = send(&f.app, "POST", "/api/me/mfa/backup-codes/regenerate", &member_token, Some(body.clone())).await;
    let (removed, _) = send(&f.app, "DELETE", &format!("/api/me/mfa/passkey/{passkey_id}"), &member_token, Some(body)).await;

    assert!(regenerated.is_server_error() && removed.is_server_error(), "{regenerated} {removed}");
    let hashes_after: Vec<String> = sqlx::query_scalar("SELECT code_hash FROM mfa_backup_codes WHERE user_id = $1 ORDER BY code_hash").bind(member_id).fetch_all(&pool).await.unwrap();
    assert_eq!(hashes_after, hashes_before, "the old codes are still the valid ones");
    assert_eq!(f.state.webauthn_credentials.count_for_user(member_id).await.unwrap(), 1, "the passkey is still registered");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_repositorys_proxy_password_never_leaves_through_either_audit_view(pool: sqlx::PgPool) {
    let f = fixture(pool.clone()).await;
    let (_, org_admin_token) = new_org_admin(&f.state, public_org(), "org-admin").await;
    let body = serde_json::json!({ "name": "npmjs", "format": "npm", "repo_type": "proxy", "remote_url": "https://registry.npmjs.org", "remote_username": "upstream-robot", "remote_password": SECRET });
    assert_eq!(send(&f.app, "POST", "/api/repositories", &f.admin_token, Some(body)).await.0, StatusCode::CREATED);
    let stored: String = sqlx::query_scalar("SELECT payload ->> 'remote_password' FROM domain_events WHERE aggregate_type = 'PackageRepository' AND event_type = 'Created'").fetch_one(&pool).await.unwrap();
    assert!(stored.starts_with("af1."), "the journal keeps the sealed password, replaying the repository needs it");

    for token in [&f.admin_token, &org_admin_token] {
        let (status, page) = send(&f.app, "GET", "/api/audit/events?aggregate_type=PackageRepository", token, None).await;
        assert_eq!(status, StatusCode::OK);
        let created = page["entries"].as_array().unwrap().iter().find(|e| e["event_type"] == "Created").expect("the Created event is listed");
        assert_eq!(created["payload"]["remote_password"], "[redacted]");
        assert_eq!(created["payload"]["remote_username"], "[redacted]");
        assert_eq!(created["payload"]["remote_url"], "https://registry.npmjs.org");
        let raw = page.to_string();
        assert!(!raw.contains(&stored) && !raw.contains(SECRET) && !raw.contains("af1."), "{raw}");
    }
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn a_failed_login_records_a_short_clean_username(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let typed = format!("{}\u{7}\n{}", "a".repeat(40), "b".repeat(200));

    let request = Request::builder().method("POST").uri("/api/auth/login").header("content-type", "application/json").body(Body::from(serde_json::json!({ "username": typed, "password": "whatever" }).to_string())).unwrap();
    let status = f.app.clone().oneshot(request).await.unwrap().status();

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let recorded = the_only(&f.state, "LoginFailed").await;
    let username = recorded.payload["username"].as_str().unwrap();
    assert_eq!(username.chars().count(), 64);
    assert!(username.starts_with(&"a".repeat(40)) && !username.chars().any(char::is_control), "{username:?}");
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn exporting_an_instance_with_personal_repositories_is_refused_and_not_recorded(pool: sqlx::PgPool) {
    let f = fixture(pool).await;
    let (alice, _) = new_user(&f.state, public_org(), "alice").await;
    f.state.reserve_personal_organization.execute(alice).await.unwrap();
    f.state.create_user_project.execute(alice, "my-lib", artiferris_domain::package_repository::RepositoryFormat::Npm, artiferris_domain::package_repository::RepositoryType::Hosted).await.unwrap();

    let (status, body) = send(&f.app, "GET", "/api/admin/export/configuration", &f.admin_token, None).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("personal"), "{body}");
    assert!(entries_of(&f.state, "ConfigurationExported").await.is_empty());
}

struct MailSink;

#[async_trait::async_trait]
impl artiferris_domain::email::EmailPort for MailSink {
    async fn send(&self, _organization_id: Uuid, _to: &str, _subject: &str, _text_body: &str, _html_body: &str) -> Result<(), artiferris_domain::error::DomainError> {
        Ok(())
    }
}

#[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
async fn regenerating_backup_codes_and_resending_an_invitation_are_recorded(pool: sqlx::PgPool) {
    let mut f = fixture(pool).await;
    f.state.resend_invitation = std::sync::Arc::new(artiferris_application::use_cases::invitation::ResendInvitationUseCase::new(
        f.state.users.clone(),
        f.state.user_invitations.clone(),
        std::sync::Arc::new(MailSink),
        f.state.organizations.clone(),
        f.state.artiferris_base_domain.clone(),
    ));
    f.app = build_router(f.state.clone());
    let (member_id, member_token) = new_user(&f.state, public_org(), "member").await;
    let enrollment = f.state.enroll_totp.execute(member_id, "member", Some(PASSWORD)).await.unwrap();
    let code = artiferris_application::use_cases::mfa::generate_current_totp_code(&enrollment.secret_base32);
    f.state.confirm_totp.execute(member_id, "member", &code).await.unwrap();

    let (status, regenerated) = send(&f.app, "POST", "/api/me/mfa/backup-codes/regenerate", &member_token, Some(serde_json::json!({ "current_password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::OK);
    let entry = the_only(&f.state, "BackupCodesRegenerated").await;
    assert_eq!((entry.actor_id, entry.organization_id), (Some(member_id), Some(public_org())));
    for backup_code in regenerated["backup_codes"].as_array().unwrap() {
        assert!(!entry.payload.to_string().contains(backup_code.as_str().unwrap()));
    }

    let (status, invited) = send(&f.app, "POST", "/api/users", &f.admin_token, Some(serde_json::json!({ "email": "invitee@example.com", "is_super_admin": false, "is_organization_admin": false }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let invitee = invited["id"].as_str().unwrap();
    assert_eq!(send(&f.app, "POST", &format!("/api/users/{invitee}/resend-invitation"), &f.admin_token, None).await.0, StatusCode::NO_CONTENT);
    let resent = the_only(&f.state, "InvitationResent").await;
    assert_eq!((resent.actor_id, resent.organization_id), (Some(f.admin_id), Some(public_org())));
    assert_eq!(resent.payload["user_id"], invitee);
}
