use std::net::SocketAddr;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use artiferris_application::error::ApplicationError;
use artiferris_application::login_throttle::{bounded_identifier, organization_username_key, shared_username_key, LoginThrottle, LOGIN_ATTEMPT_WINDOW, MAX_LOGIN_ATTEMPTS};
use artiferris_domain::audit::{AuditRecord, LoginMethod, MfaMethod, SecurityAuditRecord, SecurityEvent};
use artiferris_domain::error::DomainError;
use artiferris_domain::user::User;
use serde::{Deserialize, Serialize};

use artiferris_domain::user::TokenIssuerPort;

use crate::auth_middleware::AuthUser;
use crate::dto::{
    application_error_response, ActivateAccountRequest, ChangePasswordRequest, ErrorResponse, LdapLoginRequest, LoginRequest, LoginResponse, MeResponse, MfaVerifyRequest, RegisterRequest,
    SsoConfigResponse, SsoProviderType,
};
use crate::organization_middleware::ResolvedOrganization;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/register", post(register))
        .route("/api/auth/sso/ldap", post(sso_ldap_login))
        .route("/api/auth/sso/config", get(sso_config))
        .route("/api/auth/sso/oidc/login", get(sso_oidc_login))
        .route("/api/auth/sso/oidc/callback", get(sso_oidc_callback))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/activate", post(activate_account))
        .route("/api/auth/mfa/verify", post(verify_mfa))
        .route("/api/auth/mfa/passkey/start", post(start_mfa_passkey))
        .route("/api/auth/mfa/passkey/finish", post(finish_mfa_passkey))
        .route("/api/auth/mfa/setup/totp/enroll", post(setup_mfa_totp_enroll))
        .route("/api/auth/mfa/setup/totp/confirm", post(setup_mfa_totp_confirm))
        .route("/api/auth/mfa/setup/passkey/start", post(setup_mfa_passkey_start))
        .route("/api/auth/mfa/setup/passkey/finish", post(setup_mfa_passkey_finish))
        .route("/api/auth/logout-all", post(logout_all))
        .route("/api/me", get(me))
        .route("/api/me/password", axum::routing::put(change_password))
        .layer(axum::extract::DefaultBodyLimit::max(AUTH_BODY_LIMIT_BYTES))
}

/// Login, registration and MFA bodies are a few short fields; the 10 MiB default for the rest of `/api` would let an unauthenticated caller push megabytes through the throttle key and the audit log.
const AUTH_BODY_LIMIT_BYTES: usize = 64 * 1024;
/// More than the failed-login budget: a start is a normal click, and an office behind one address may sign in together.
const OIDC_START_MAX_ATTEMPTS: usize = 60;
/// A password past this is refused as wrong without hashing it.
const MAX_LOGIN_PASSWORD_BYTES: usize = 1024;

/// Resolves the client IP for throttling. `X-Forwarded-For` is only read when the direct peer is a
/// configured trusted proxy — otherwise any client could self-report an IP and pick its own
/// throttle key. Falls back to the direct peer for everything else.
///
/// `Result`, not `Option` — axum 0.8 only extracts `Option<T>` for extractors that opt into
/// `OptionalFromRequestParts`, which `ConnectInfo` doesn't; `Result<T, T::Rejection>` still has the blanket impl.
pub(crate) fn peer_ip(state: &AppState, headers: &HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> String {
    let direct = connect_info.ok().map(|ConnectInfo(addr)| addr.ip());
    let forwarded: Vec<&str> = headers.get_all("x-forwarded-for").iter().filter_map(|v| v.to_str().ok()).collect();
    artiferris_application::client_ip::client_ip(direct, &forwarded, &state.trusted_proxies)
}

/// The key a request's client is counted under by a throttle: `peer_ip`, with an IPv6 client widened to its /64.
pub(crate) fn peer_ip_bucket(state: &AppState, headers: &HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> String {
    artiferris_application::client_ip::throttle_bucket(&peer_ip(state, headers, connect_info))
}

/// Exposed separately, not just OR'd together, so the login page's verify step can show only the factor(s) that actually exist.
struct MfaFactors {
    has_totp: bool,
    has_passkey: bool,
}

impl MfaFactors {
    fn any(&self) -> bool {
        self.has_totp || self.has_passkey
    }
}

/// Fails CLOSED: a repository error here must not be silently treated as "no MFA factors exist" —
/// that would misroute a user with real MFA configured into the mandatory-setup flow, or (worse)
/// let a stuck error mask an existing factor from the "already enrolled" refusal check that guards
/// against a session-hijacker planting a second one (B-4).
async fn mfa_factors(state: &AppState, user_id: uuid::Uuid) -> Result<MfaFactors, ApplicationError> {
    let has_totp = state.totp_credentials.get(user_id).await?.is_some_and(|c| c.confirmed);
    let has_passkey = state.webauthn_credentials.count_for_user(user_id).await? > 0;
    Ok(MfaFactors { has_totp, has_passkey })
}

/// `Username::parse` lowercases, so alice/Alice/ALICE are all one account — throttle keys and audit
/// entries have to collapse the same way or every case variant gets its own budget. An unparseable
/// username is lowercased too rather than passed through, so it can't dodge the same way, and
/// cut to a sane length so it can't bloat the throttle map or the audit log.
pub(crate) fn normalized_username(raw: &str) -> String {
    let bounded = bounded_identifier(raw);
    artiferris_domain::user::Username::parse(&bounded).map_or_else(|_| bounded.to_ascii_lowercase(), |username| username.as_str().to_string())
}

/// Anything past these can't be a real credential, so it skips the directory and the hasher.
fn credentials_within_bounds(username: &str, password: &str) -> bool {
    username.chars().count() <= artiferris_application::login_throttle::MAX_LOGIN_IDENTIFIER_LEN && password.len() <= MAX_LOGIN_PASSWORD_BYTES
}

/// The shared username key always uses the fixed limits, so no tenant can loosen it to guess passwords or stretch it to lock somebody else out.
/// The stricter policy of the organization the request came in on lives on a key of its own.
struct UsernameThrottle {
    shared_key: String,
    organization: Option<(String, usize, std::time::Duration)>,
}

impl UsernameThrottle {
    async fn for_login(state: &AppState, organization_id: uuid::Uuid, username: &str) -> Self {
        // The request's organization, not the account's: the limit can't depend on whether the username exists.
        let (max_attempts, window) = throttle_limits_for_organization(state, organization_id).await;
        let stricter = max_attempts < MAX_LOGIN_ATTEMPTS || window > LOGIN_ATTEMPT_WINDOW;
        Self { shared_key: shared_username_key(username), organization: stricter.then(|| (organization_username_key(organization_id, username), max_attempts, window)) }
    }

    /// Charges the address budget in the same step.
    fn reserve_with(&self, throttle: &LoginThrottle, ip_key: &str) -> bool {
        let mut budgets = vec![(self.shared_key.as_str(), MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW), (ip_key, MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW)];
        if let Some((key, max_attempts, window)) = &self.organization {
            budgets.push((key.as_str(), *max_attempts, *window));
        }
        throttle.reserve_all(&budgets)
    }

    /// The account that just proved it owns the name starts over.
    fn clear(&self, throttle: &LoginThrottle) {
        throttle.clear(&self.shared_key);
        if let Some((key, ..)) = &self.organization {
            throttle.clear(key);
        }
    }

    /// The attempt was fine but the name isn't proven to be this account's: it stops counting, nothing else is touched.
    fn release(&self, throttle: &LoginThrottle) {
        throttle.release(&self.shared_key);
        if let Some((key, ..)) = &self.organization {
            throttle.release(key);
        }
    }
}

pub(crate) async fn throttle_limits_for_organization(state: &AppState, organization_id: uuid::Uuid) -> (usize, std::time::Duration) {
    match state.get_system_settings.execute(organization_id).await {
        Ok(settings) => (settings.max_login_attempts as usize, std::time::Duration::from_secs(settings.login_attempt_window_seconds as u64)),
        Err(e) => {
            tracing::warn!("failed to load system settings for login throttle limits, falling back to global defaults: {e}");
            (artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS, artiferris_application::login_throttle::LOGIN_ATTEMPT_WINDOW)
        }
    }
}

async fn login(
    State(state): State<AppState>,
    resolved_org: ResolvedOrganization,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    let ip = peer_ip(&state, &headers, connect_info);
    let username = normalized_username(&body.username);
    let ip_key = format!("login-ip:{}", artiferris_application::client_ip::throttle_bucket(&ip));
    let username_throttle = UsernameThrottle::for_login(&state, resolved_org.0.id, &username).await;

    if !username_throttle.reserve_with(&state.login_throttle, &ip_key) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse { error: "too many failed login attempts, try again later".to_string() }),
        ));
    }

    let outcome = if credentials_within_bounds(&body.username, &body.password) {
        state.authenticate_user.execute(&body.username, &body.password).await
    } else {
        Err(ApplicationError::InvalidCredentials)
    };
    match outcome {
        Ok(token) => {
            username_throttle.clear(&state.login_throttle);
            // Only handed back, not wiped — a shared IP (NAT/office) must not have its
            // failures reset by one unrelated account's successful login.
            state.login_throttle.release(&ip_key);
            let user_id = state.token_issuer.verify(&token).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() })))?.user_id;
            // mfa_setup_required tells the client whether to go to /mfa/setup/* or /mfa/verify.
            let factors = mfa_factors(&state, user_id).await.map_err(|e| application_error_response("failed to check MFA status", e))?;
            // `JwtMfaPendingTokenIssuer` ignores this ttl and always uses its own fixed, short lifetime.
            let mfa_token = state
                .mfa_pending_token_issuer
                .issue(user_id, chrono::Duration::minutes(5))
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() })))?;
            Ok(Json(LoginResponse {
                token: None,
                mfa_token: Some(mfa_token),
                mfa_setup_required: !factors.any(),
                mfa_has_totp: factors.has_totp,
                mfa_has_passkey: factors.has_passkey,
            }))
        }
        // Turned away before any password was checked: not the client's failed attempt.
        Err(e @ ApplicationError::Domain(DomainError::Busy(_))) => {
            username_throttle.release(&state.login_throttle);
            state.login_throttle.release(&ip_key);
            Err(application_error_response("failed to check credentials", e))
        }
        Err(_) => {
            crate::state::record_security_event(&state, SecurityEvent::LoginFailed { username, ip }, None).await;
            Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid credentials".to_string() })))
        }
    }
}

/// Unauthenticated — this is how an account is first created. Only for the `public` organization; others provision via admin invitation instead.
async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    resolved_org: ResolvedOrganization,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Json(body): Json<RegisterRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Every attempt counts, whatever its outcome — a registration flood costs server work regardless.
    let throttle_key = format!("register:{}", peer_ip_bucket(&state, &headers, connect_info));
    if !state.login_throttle.reserve(&throttle_key, MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse { error: "too many registration attempts, try again later".to_string() }),
        ));
    }

    if !resolved_org.0.is_public {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "public self-registration is not available on this organization".to_string() })));
    }

    let settings = state
        .get_system_settings
        .execute(resolved_org.0.id)
        .await
        .map_err(|e| application_error_response("failed to check system settings", e))?;
    if !settings.registration_enabled {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "public self-registration is currently disabled".to_string() })));
    }

    let user_id = state
        .register_public_user
        .execute(resolved_org.0.id, &body.username, &body.email, &body.password)
        .await
        .map_err(|e| application_error_response("failed to register user", e))?;

    let mfa_token = state
        .mfa_pending_token_issuer
        .issue(user_id, chrono::Duration::minutes(5))
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() })))?;
    Ok(Json(LoginResponse { token: None, mfa_token: Some(mfa_token), mfa_setup_required: true, mfa_has_totp: false, mfa_has_passkey: false }))
}

/// Unauthenticated — the login page checks this before submitting, to know whether to post to `/api/auth/sso` or `/api/auth/login`. Reveals only the provider type.
async fn sso_config(State(state): State<AppState>, resolved_org: ResolvedOrganization) -> Result<Json<SsoConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    // An identity provider whose secret can't be read can't sign anybody in either, so the login page just doesn't offer it.
    let config = match state.identity_providers.get(resolved_org.0.id).await {
        Ok(config) => config,
        Err(DomainError::SecretUnreadable(detail)) => {
            tracing::error!("the identity provider of organization {} is hidden from the login page: {detail}", resolved_org.0.id);
            None
        }
        Err(_) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() }))),
    };
    let provider_type = config.map(|c| match c {
        artiferris_domain::sso::IdentityProviderConfig::Ldap(_) => SsoProviderType::Ldap,
        artiferris_domain::sso::IdentityProviderConfig::Oidc(_) => SsoProviderType::Oidc,
    });
    let settings = state
        .get_system_settings
        .execute(resolved_org.0.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() })))?;
    Ok(Json(SsoConfigResponse { provider_type, registration_enabled: resolved_org.0.is_public && settings.registration_enabled }))
}

/// This organization's own origin — never derived from a caller-supplied header, so a non-public org never gets sent back to the wrong one.
fn organization_origin(state: &AppState, resolved_org: &ResolvedOrganization) -> String {
    artiferris_application::use_cases::invitation::organization_origin(&state.artiferris_base_domain, &resolved_org.0)
}

/// Must match the `redirect_uri`/`callback_url` registered with the identity provider exactly.
fn oidc_callback_url(state: &AppState, resolved_org: &ResolvedOrganization) -> String {
    format!("{}/api/auth/sso/oidc/callback", organization_origin(state, resolved_org))
}

/// Unprefixed name, for plain-HTTP local dev only — `__Host-` cookies require `Secure`, which browsers drop over `http://`.
const OIDC_BINDING_COOKIE: &str = "artiferris_oidc_binding";
/// `__Host-` prefixed everywhere else — a browser-enforced guarantee this cookie can't be shadowed by a sibling subdomain.
const OIDC_BINDING_COOKIE_HOST_PREFIXED: &str = "__Host-artiferris_oidc_binding";
/// Matches `STATE_TOKEN_TTL_MINUTES` in `OpenidConnectAuthAdapter` — no point outliving the token it's bound to.
const OIDC_BINDING_COOKIE_MAX_AGE_SECONDS: i64 = 10 * 60;

fn oidc_binding_cookie_name(state: &AppState) -> &'static str {
    if artiferris_application::base_domain::is_local_dev_domain(&state.artiferris_base_domain) { OIDC_BINDING_COOKIE } else { OIDC_BINDING_COOKIE_HOST_PREFIXED }
}

// Parses a raw Set-Cookie string rather than using Cookie::build, which needs a time::Duration
// for Max-Age that axum-extra doesn't re-export. SameSite=Lax, not Strict: the browser still
// needs to send this on the cross-site redirect the identity provider sends it through.
fn oidc_binding_cookie(state: &AppState, value: &str) -> Option<axum_extra::extract::cookie::Cookie<'static>> {
    let name = oidc_binding_cookie_name(state);
    let secure = if artiferris_application::base_domain::is_local_dev_domain(&state.artiferris_base_domain) { "" } else { "; Secure" };
    axum_extra::extract::cookie::Cookie::parse(format!("{name}={value}; Path=/; Max-Age={OIDC_BINDING_COOKIE_MAX_AGE_SECONDS}; HttpOnly; SameSite=Lax{secure}")).ok()
}

fn oidc_start_key(state: &AppState, headers: &HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> String {
    format!("oidc-start:{}", peer_ip_bucket(state, headers, connect_info))
}

/// Unauthenticated — redirects the browser to the identity provider to start the OIDC flow, 400 if none is configured. Also sets `artiferris_oidc_binding`, a login-CSRF binding secret (RFC 6749 §10.12) tying the callback to this same browser.
async fn sso_oidc_login(
    State(state): State<AppState>,
    resolved_org: ResolvedOrganization,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    jar: axum_extra::extract::cookie::CookieJar,
) -> Result<(axum_extra::extract::cookie::CookieJar, axum::response::Redirect), (StatusCode, Json<ErrorResponse>)> {
    // Every start is a discovery round-trip to the identity provider. A callback that signs the person in hands it back.
    if !state.login_throttle.reserve(&oidc_start_key(&state, &headers, connect_info), OIDC_START_MAX_ATTEMPTS, LOGIN_ATTEMPT_WINDOW) {
        return Err((StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse { error: "too many login attempts, try again later".to_string() })));
    }
    let config = state
        .identity_providers
        .get(resolved_org.0.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() })))?;
    let Some(artiferris_domain::sso::IdentityProviderConfig::Oidc(oidc_config)) = config else {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "this organization has no OIDC identity provider configured".to_string() })));
    };

    let binding_secret = uuid::Uuid::new_v4().to_string();
    let cookie = oidc_binding_cookie(&state, &binding_secret)
        .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "failed to start oidc login".to_string() })))?;

    let callback_url = oidc_callback_url(&state, &resolved_org);
    let redirect_url = state
        .oidc_auth
        .build_redirect(&oidc_config, resolved_org.0.id, &callback_url, &binding_secret)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "failed to start oidc login".to_string() })))?;
    Ok((jar.add(cookie), axum::response::Redirect::to(&redirect_url)))
}

#[derive(Deserialize)]
struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

/// Unauthenticated — the identity provider redirects here after the user authenticates. On success, sends the session token back in the URL fragment (`#token=...`), never a query param, since fragments never reach the server or its access logs.
async fn sso_oidc_callback(
    State(state): State<AppState>,
    resolved_org: ResolvedOrganization,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    jar: axum_extra::extract::cookie::CookieJar,
    axum::extract::Query(query): axum::extract::Query<OidcCallbackQuery>,
) -> Result<(axum_extra::extract::cookie::CookieJar, axum::response::Redirect), (StatusCode, Json<ErrorResponse>)> {
    let (Some(code), Some(raw_state)) = (query.code, query.state) else {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "missing code or state".to_string() })));
    };

    // Reserved before anything is looked up or recorded, handed back on success.
    let bucket = peer_ip_bucket(&state, &headers, connect_info);
    let budget_key = format!("oidc-callback:{bucket}");
    if !state.login_throttle.reserve(&budget_key, MAX_LOGIN_ATTEMPTS, LOGIN_ATTEMPT_WINDOW) {
        return Err((StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse { error: "too many failed login attempts, try again later".to_string() })));
    }

    let config = state
        .identity_providers
        .get(resolved_org.0.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() })))?;
    let Some(artiferris_domain::sso::IdentityProviderConfig::Oidc(oidc_config)) = config else {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "this organization has no OIDC identity provider configured".to_string() })));
    };

    // Nothing is audited until the state proves this browser started a login here: the Host header alone picks the tenant, so anyone could fill its log otherwise.
    let binding_secret = jar.get(oidc_binding_cookie_name(&state)).map(|c| c.value().to_string());
    let Some(binding_secret) = binding_secret.filter(|secret| state.oidc_auth.state_is_valid(&raw_state, resolved_org.0.id, secret)) else {
        return Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid credentials".to_string() })));
    };

    let callback_url = oidc_callback_url(&state, &resolved_org);
    let identity = match state
        .oidc_auth
        .handle_callback(&oidc_config, &code, &raw_state, &callback_url, resolved_org.0.id, &binding_secret)
        .await
    {
        Ok(identity) => identity,
        Err(_) => {
            crate::state::record_security_event(&state, SecurityEvent::OidcLoginFailed { organization_id: resolved_org.0.id }, None).await;
            return Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid credentials".to_string() })));
        }
    };

    let token = match state.provision_sso_user.execute(resolved_org.0.id, &identity).await {
        Ok(token) => token,
        // Same 401 whether the exchange failed or the account was blocked afterward.
        Err(ApplicationError::InvalidCredentials) => {
            crate::state::record_security_event(&state, SecurityEvent::OidcLoginFailed { organization_id: resolved_org.0.id }, None).await;
            return Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid credentials".to_string() })));
        }
        Err(e) => return Err(application_error_response("failed to provision sso user", e)),
    };

    state.login_throttle.release(&budget_key);
    state.login_throttle.release(&format!("oidc-start:{bucket}"));
    record_sso_login(&state, &token, resolved_org.0.id, LoginMethod::Oidc).await;

    // One binding secret, one use — clear it so a replayed callback has nothing to match.
    let cleared = jar.remove(
        axum_extra::extract::cookie::Cookie::build((oidc_binding_cookie_name(&state), ""))
            .path("/")
            .secure(!artiferris_application::base_domain::is_local_dev_domain(&state.artiferris_base_domain))
            .build(),
    );

    Ok((cleared, axum::response::Redirect::to(&format!("{}/login#token={}", organization_origin(&state, &resolved_org), token))))
}

/// Best-effort audit entry for a failed LDAP login. Its throttle budget was already charged up front.
async fn record_ldap_login_failure(state: &AppState, username: String, ip: String) {
    crate::state::record_security_event(state, SecurityEvent::LoginFailed { username, ip }, None).await;
}

/// All LDAP failure modes collapse to 401 — same as local login never distinguishing "unknown user" from "wrong password".
async fn sso_ldap_login(
    State(state): State<AppState>,
    resolved_org: ResolvedOrganization,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Json(body): Json<LdapLoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    let config = state
        .identity_providers
        .get(resolved_org.0.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() })))?;
    let Some(artiferris_domain::sso::IdentityProviderConfig::Ldap(ldap_config)) = config else {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "this organization has no LDAP identity provider configured".to_string() })));
    };

    let ip = peer_ip(&state, &headers, connect_info);
    let username = normalized_username(&body.username);
    let ip_key = format!("login-ip:{}", artiferris_application::client_ip::throttle_bucket(&ip));
    let username_throttle = UsernameThrottle::for_login(&state, resolved_org.0.id, &username).await;

    if !username_throttle.reserve_with(&state.login_throttle, &ip_key) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse { error: "too many failed login attempts, try again later".to_string() }),
        ));
    }

    let authenticated = if credentials_within_bounds(&body.username, &body.password) {
        state.ldap_auth.authenticate(&ldap_config, &body.username, &body.password).await.ok()
    } else {
        None
    };
    let Some(identity) = authenticated else {
        record_ldap_login_failure(&state, username, ip).await;
        return Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid credentials".to_string() })));
    };

    match state.provision_sso_user.execute(resolved_org.0.id, &identity).await {
        Ok(token) => {
            // Typed into a directory the organization controls: only an account that really has this name gets its failures forgotten.
            if account_username(&state, &token).await.as_deref() == Some(username.as_str()) {
                username_throttle.clear(&state.login_throttle);
            } else {
                username_throttle.release(&state.login_throttle);
            }
            state.login_throttle.release(&ip_key);
            record_sso_login(&state, &token, resolved_org.0.id, LoginMethod::Ldap).await;
            Ok(Json(LoginResponse { token: Some(token), mfa_token: None, mfa_setup_required: false, mfa_has_totp: false, mfa_has_passkey: false }))
        }
        Err(e @ ApplicationError::Domain(DomainError::Busy(_))) => {
            username_throttle.release(&state.login_throttle);
            state.login_throttle.release(&ip_key);
            Err(application_error_response("failed to provision sso user", e))
        }
        Err(e) => {
            record_ldap_login_failure(&state, username, ip).await;
            match e {
                // Same 401 whether the directory bind failed or the account was blocked afterward.
                ApplicationError::InvalidCredentials => Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid credentials".to_string() }))),
                e => Err(application_error_response("failed to provision sso user", e)),
            }
        }
    }
}

type ApiError = (StatusCode, Json<ErrorResponse>);

fn internal_error() -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() }))
}

fn invalid_mfa_token() -> ApiError {
    (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid or expired mfa token".to_string() }))
}

/// A start costs server memory and needs nothing but a pending `mfa_token`, so each client gets a budget of them. A finished ceremony hands its start back.
const PASSKEY_START_ATTEMPTS: usize = 30;

fn passkey_start_key(state: &AppState, headers: &HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> String {
    format!("passkey-start:{}", peer_ip_bucket(state, headers, connect_info))
}

fn reserve_passkey_start(state: &AppState, headers: &HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> Result<(), ApiError> {
    if state.login_throttle.reserve(&passkey_start_key(state, headers, connect_info), PASSKEY_START_ATTEMPTS, LOGIN_ATTEMPT_WINDOW) { Ok(()) } else { Err(too_many_attempts()) }
}

fn too_many_attempts() -> ApiError {
    (StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse { error: "too many failed attempts, try again later".to_string() }))
}

/// The account behind a valid `mfa_token`. Refused once the account has revoked its tokens since the token was minted (password change, sign out everywhere, demotion).
async fn user_for_mfa_token(state: &AppState, mfa_token: &str) -> Result<User, ApiError> {
    let verified = state.mfa_pending_token_issuer.verify(mfa_token).map_err(|_| invalid_mfa_token())?;
    let user = state.users.find_by_id(verified.user_id).await.map_err(|_| internal_error())?.ok_or_else(invalid_mfa_token)?;
    if user.has_revoked_tokens_issued_at(verified.issued_at) {
        return Err(invalid_mfa_token());
    }
    Ok(user)
}

/// Checked again where the session is minted: a factor can be enrolled after setup started.
async fn require_no_factor(state: &AppState, user_id: uuid::Uuid) -> Result<(), ApiError> {
    if mfa_factors(state, user_id).await.map_err(|e| application_error_response("failed to check MFA status", e))?.any() {
        return Err(application_error_response("mandatory MFA setup rejected", ApplicationError::MfaAlreadyEnabled));
    }
    Ok(())
}

/// The final step of a login or of mandatory MFA setup: a token may complete one of them only once.
fn consume_mfa_token(state: &AppState, user_id: uuid::Uuid, mfa_token: &str) -> Result<(), ApiError> {
    if state.used_mfa_tokens.consume(user_id, mfa_token) { Ok(()) } else { Err(invalid_mfa_token()) }
}

async fn issue_session_token(state: &AppState, user: &User) -> Result<String, ApiError> {
    let settings = state.get_system_settings.execute(user.organization_id).await.map_err(|e| application_error_response("failed to get system settings", e))?;
    state.token_issuer.issue(user.id, chrono::Duration::hours(settings.session_ttl_hours as i64)).map_err(|_| internal_error())
}

async fn record_login(state: &AppState, user: &User, second_factor: MfaMethod) {
    let event = SecurityEvent::LoginSucceeded { user_id: user.id, organization_id: user.organization_id, method: LoginMethod::Password, second_factor: Some(second_factor) };
    crate::state::record_security_event(state, event, Some(user.id)).await;
}

async fn account_username(state: &AppState, session_token: &str) -> Option<String> {
    let verified = state.token_issuer.verify(session_token).ok()?;
    Some(state.users.find_by_id(verified.user_id).await.ok()??.username.as_str().to_string())
}

/// A directory or OIDC login lands in the organization it came in on.
async fn record_sso_login(state: &AppState, session_token: &str, organization_id: uuid::Uuid, method: LoginMethod) {
    let Ok(verified) = state.token_issuer.verify(session_token) else {
        return;
    };
    let event = SecurityEvent::LoginSucceeded { user_id: verified.user_id, organization_id, method, second_factor: None };
    crate::state::record_security_event(state, event, Some(verified.user_id)).await;
}

fn session_login_response(token: String) -> LoginResponse {
    LoginResponse { token: Some(token), mfa_token: None, mfa_setup_required: false, mfa_has_totp: false, mfa_has_passkey: false }
}

/// Exchanges the short-lived `mfa_token` plus a TOTP/backup code for a real session token. Unauthenticated by design — `mfa_token` itself is the credential.
async fn verify_mfa(State(state): State<AppState>, Json(body): Json<MfaVerifyRequest>) -> Result<Json<LoginResponse>, ApiError> {
    let user = user_for_mfa_token(&state, &body.mfa_token).await?;
    let user_id = user.id;

    let (max_attempts, window) = throttle_limits_for_organization(&state, user.organization_id).await;
    let throttle_key = mfa_verify_throttle_key(user_id);
    // Reserved before the code is checked, so a burst of parallel guesses can't all pass the limit first.
    if !state.login_throttle.reserve(&throttle_key, max_attempts, window) {
        return Err(too_many_attempts());
    }

    let verified = match (&body.code, &body.backup_code) {
        (Some(code), _) => state.verify_totp.execute(user_id, code).await,
        (None, Some(backup_code)) => state.verify_backup_code.execute(user_id, backup_code).await,
        (None, None) => Err(ApplicationError::InvalidMfaCode),
    };

    match verified {
        Ok(()) => {
            consume_mfa_token(&state, user_id, &body.mfa_token)?;
            state.login_throttle.clear(&throttle_key);
            let token = issue_session_token(&state, &user).await?;
            record_login(&state, &user, if body.code.is_some() { MfaMethod::Totp } else { MfaMethod::BackupCode }).await;
            Ok(Json(session_login_response(token)))
        }
        Err(_) => {
            let method = if body.code.is_some() { "totp" } else { "backup_code" };
            crate::state::record_security_event(&state, SecurityEvent::MfaVerificationFailed { user_id, method: method.to_string() }, Some(user_id)).await;
            Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid code".to_string() })))
        }
    }
}

#[derive(Deserialize)]
struct MfaPasskeyStartRequest {
    mfa_token: String,
}

#[derive(Serialize)]
struct MfaPasskeyStartResponse {
    challenge_id: uuid::Uuid,
    public_key: webauthn_rs_proto::PublicKeyCredentialRequestOptions,
}

/// Unwraps `RequestChallengeResponse` down to its `public_key` field — it already serializes to `{"publicKey": {...}}`, which would otherwise double-nest here.
async fn start_mfa_passkey(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Json(body): Json<MfaPasskeyStartRequest>,
) -> Result<Json<MfaPasskeyStartResponse>, ApiError> {
    reserve_passkey_start(&state, &headers, connect_info)?;
    let user = user_for_mfa_token(&state, &body.mfa_token).await?;

    let (challenge_id, public_key) = state.start_passkey_authentication.execute(user.id).await.map_err(|e| application_error_response("failed to start passkey authentication", e))?;
    Ok(Json(MfaPasskeyStartResponse { challenge_id, public_key: public_key.public_key }))
}

#[derive(Deserialize)]
struct MfaPasskeyFinishRequest {
    mfa_token: String,
    challenge_id: uuid::Uuid,
    credential: webauthn_rs::prelude::PublicKeyCredential,
}

async fn finish_mfa_passkey(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Json(body): Json<MfaPasskeyFinishRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let user = user_for_mfa_token(&state, &body.mfa_token).await?;
    let user_id = user.id;

    let (max_attempts, window) = throttle_limits_for_organization(&state, user.organization_id).await;
    let throttle_key = mfa_verify_throttle_key(user_id);
    if !state.login_throttle.reserve(&throttle_key, max_attempts, window) {
        return Err(too_many_attempts());
    }

    match state.finish_passkey_authentication.execute(user_id, body.challenge_id, &body.credential).await {
        Ok(()) => {
            consume_mfa_token(&state, user_id, &body.mfa_token)?;
            state.login_throttle.clear(&throttle_key);
            state.login_throttle.release(&passkey_start_key(&state, &headers, connect_info));
            let token = issue_session_token(&state, &user).await?;
            record_login(&state, &user, MfaMethod::Passkey).await;
            Ok(Json(session_login_response(token)))
        }
        Err(_) => {
            crate::state::record_security_event(&state, SecurityEvent::PasskeyVerificationFailed { user_id }, Some(user_id)).await;
            Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "passkey verification failed".to_string() })))
        }
    }
}

#[derive(Deserialize)]
struct MfaSetupTokenOnlyRequest {
    mfa_token: String,
}

#[derive(Serialize)]
struct TotpEnrollmentResponse {
    secret: String,
    otpauth_url: String,
}

pub(crate) fn mfa_verify_throttle_key(user_id: uuid::Uuid) -> String {
    format!("mfa:{user_id}")
}

/// Distinct from `manage_throttle_key` (session-authenticated `/api/me/mfa/*`) and the bare
/// username key `login`/`change_password` use — this covers every failed attempt during the
/// mandatory-setup flow (wrong password at enroll/start, wrong code at confirm/finish) for one
/// account, all sharing the mfa_token's short lifetime.
pub(crate) fn mfa_setup_throttle_key(user_id: uuid::Uuid) -> String {
    format!("mfa-setup:{user_id}")
}

/// Mandatory-enrollment counterpart to `/api/me/mfa/totp/enroll`, authenticated by `mfa_token` rather than a full session.
///
/// Deliberately does NOT require `current_password` (unlike `/api/me/mfa/totp/enroll`) — this route
/// runs right after login/register, gated by the short-lived `mfa_token` rather than a full session,
/// and that token itself already proves the password was verified moments earlier. Requiring it
/// again here would just break this flow's wire contract (the frontend never sends one) for no
/// security benefit; see Task 5 fix round 1.
async fn setup_mfa_totp_enroll(State(state): State<AppState>, Json(body): Json<MfaSetupTokenOnlyRequest>) -> Result<Json<TotpEnrollmentResponse>, ApiError> {
    let user = user_for_mfa_token(&state, &body.mfa_token).await?;
    require_no_factor(&state, user.id).await?;
    let enrollment = state.enroll_totp.execute(user.id, user.username.as_str(), None).await.map_err(|e| application_error_response("failed to enroll TOTP during mandatory setup", e))?;
    Ok(Json(TotpEnrollmentResponse { secret: enrollment.secret_base32, otpauth_url: enrollment.otpauth_url }))
}

#[derive(Deserialize)]
struct MfaSetupTotpConfirmRequest {
    mfa_token: String,
    code: String,
}

#[derive(Serialize)]
struct MfaSetupCompleteResponse {
    token: String,
    backup_codes: Vec<String>,
}

/// Success issues a real session token immediately.
async fn setup_mfa_totp_confirm(State(state): State<AppState>, Json(body): Json<MfaSetupTotpConfirmRequest>) -> Result<Json<MfaSetupCompleteResponse>, ApiError> {
    let user = user_for_mfa_token(&state, &body.mfa_token).await?;

    let (max_attempts, window) = throttle_limits_for_organization(&state, user.organization_id).await;
    let throttle_key = mfa_setup_throttle_key(user.id);
    if !state.login_throttle.reserve(&throttle_key, max_attempts, window) {
        return Err(too_many_attempts());
    }

    require_no_factor(&state, user.id).await?;
    match state.confirm_totp.execute(user.id, user.username.as_str(), &body.code).await {
        Ok(backup_codes) => {
            consume_mfa_token(&state, user.id, &body.mfa_token)?;
            state.login_throttle.clear(&throttle_key);
            let token = issue_session_token(&state, &user).await?;
            crate::state::record_security_event(&state, SecurityEvent::MfaEnabled { user_id: user.id, organization_id: user.organization_id, method: MfaMethod::Totp }, Some(user.id)).await;
            record_login(&state, &user, MfaMethod::Totp).await;
            Ok(Json(MfaSetupCompleteResponse { token, backup_codes }))
        }
        Err(e) => Err(application_error_response("failed to confirm TOTP during mandatory setup", e)),
    }
}

#[derive(Serialize)]
struct MfaSetupPasskeyStartResponse {
    challenge_id: uuid::Uuid,
    public_key: webauthn_rs_proto::PublicKeyCredentialCreationOptions,
}

/// See `start_mfa_passkey`'s doc comment: same unwrap, same reason.
async fn setup_mfa_passkey_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Json(body): Json<MfaSetupTokenOnlyRequest>,
) -> Result<Json<MfaSetupPasskeyStartResponse>, ApiError> {
    reserve_passkey_start(&state, &headers, connect_info)?;
    let user = user_for_mfa_token(&state, &body.mfa_token).await?;
    require_no_factor(&state, user.id).await?;
    let (challenge_id, public_key) =
        state.start_passkey_registration.execute(user.id, user.username.as_str(), None).await.map_err(|e| application_error_response("failed to start passkey registration during mandatory setup", e))?;
    Ok(Json(MfaSetupPasskeyStartResponse { challenge_id, public_key: public_key.public_key }))
}

#[derive(Deserialize)]
struct MfaSetupPasskeyFinishRequest {
    mfa_token: String,
    challenge_id: uuid::Uuid,
    credential: webauthn_rs::prelude::RegisterPublicKeyCredential,
    name: String,
}

async fn setup_mfa_passkey_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Json(body): Json<MfaSetupPasskeyFinishRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let user = user_for_mfa_token(&state, &body.mfa_token).await?;

    let (max_attempts, window) = throttle_limits_for_organization(&state, user.organization_id).await;
    let throttle_key = mfa_setup_throttle_key(user.id);
    if !state.login_throttle.reserve(&throttle_key, max_attempts, window) {
        return Err(too_many_attempts());
    }

    require_no_factor(&state, user.id).await?;
    match state.finish_passkey_registration.execute(user.id, user.organization_id, body.challenge_id, &body.credential, &body.name).await {
        Ok(_) => {
            consume_mfa_token(&state, user.id, &body.mfa_token)?;
            state.login_throttle.clear(&throttle_key);
            state.login_throttle.release(&passkey_start_key(&state, &headers, connect_info));
            let token = issue_session_token(&state, &user).await?;
            record_login(&state, &user, MfaMethod::Passkey).await;
            Ok(Json(session_login_response(token)))
        }
        Err(e) => Err(application_error_response("failed to finish passkey registration during mandatory setup", e)),
    }
}

/// Sessions are stateless JWTs, so this alone can't end one; the client just discards its token. `logout-all` is the server-side kill switch.
async fn logout() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Ends every session, Docker token and API token this account holds, including the one making the request. Also the way out for an SSO account, which has no password to change.
async fn logout_all(State(state): State<AppState>, user: AuthUser) -> Result<StatusCode, ApiError> {
    let audit = SecurityAuditRecord { event: SecurityEvent::SessionsRevoked { user_id: user.id, organization_id: user.organization_id }, actor_id: Some(user.id) };
    state.revoke_user_sessions.execute(user.id, Some(&audit)).await.map_err(|e| application_error_response("failed to revoke sessions", e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unauthenticated by design — the activation token itself is the credential.
async fn activate_account(State(state): State<AppState>, Json(body): Json<ActivateAccountRequest>) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state.activate_account.execute(&body.token, &body.new_password).await.map_err(|e| application_error_response("failed to activate account", e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(user: AuthUser) -> Json<MeResponse> {
    Json(MeResponse {
        id: user.id,
        username: user.username,
        is_super_admin: user.is_super_admin,
        is_organization_admin: user.is_organization_admin,
        organization_id: user.organization_id,
        created_at: user.created_at,
    })
}

async fn change_password(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let (max_attempts, window) = throttle_limits_for_organization(&state, user.organization_id).await;
    if !state.login_throttle.reserve(&user.username, max_attempts, window) {
        return Err(too_many_attempts());
    }

    let audit = AuditRecord::Security(SecurityAuditRecord { event: SecurityEvent::PasswordChanged { user_id: user.id, organization_id: user.organization_id }, actor_id: Some(user.id) });
    match state.change_password.execute(user.id, &body.current_password, &body.new_password, Some(&audit)).await {
        Ok(()) => {
            state.login_throttle.clear(&user.username);
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            if matches!(e, ApplicationError::InvalidCredentials) {
                crate::state::record_security_event(
                    &state,
                    SecurityEvent::PasswordChangeFailed { username: user.username.clone(), ip: peer_ip(&state, &headers, connect_info) },
                    Some(user.id),
                )
                .await;
            } else {
                // A rejected weak new password is not a credential-guessing signal.
                state.login_throttle.release(&user.username);
            }
            Err(application_error_response("failed to change password", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::{build_router, state::AppState};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;
    use uuid::Uuid;

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

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn login_succeeds_with_correct_credentials_but_requires_mfa_setup_when_none_is_enrolled(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"sup3r-s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["token"].is_null());
        assert!(json["mfa_token"].is_string());
        assert_eq!(json["mfa_setup_required"], true);
    }

    async fn enroll_and_confirm_totp(state: &AppState, user_id: uuid::Uuid, password: &str) {
        let enrollment = state.enroll_totp.execute(user_id, "florian", Some(password)).await.unwrap();
        let code = artiferris_application::use_cases::mfa::generate_current_totp_code(&enrollment.secret_base32);
        state.confirm_totp.execute(user_id, "florian", &code).await.unwrap();
    }

    /// Bypasses the real WebAuthn ceremony (unfakeable over HTTP) by inserting the credential directly through the same port `FinishPasskeyRegistrationUseCase` writes to.
    async fn enroll_a_passkey(state: &AppState, user_id: uuid::Uuid) {
        state
            .webauthn_credentials
            .insert(&artiferris_domain::webauthn::WebauthnCredential {
                id: Uuid::new_v4(),
                user_id,
                name: "Test key".to_string(),
                passkey_data: vec![0u8; 8],
                created_at: chrono::Utc::now(),
            }, None)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn logging_in_with_confirmed_totp_returns_an_mfa_token_instead_of_a_session_token(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"sup3r-s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["token"].is_null(), "no full session token before the second factor is verified");
        assert!(json["mfa_token"].is_string());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_mfa_token_cannot_be_used_as_a_bearer_session_token(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let app = build_router(state);

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"sup3r-s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(login_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let response = app.oneshot(Request::builder().uri("/api/me").header("authorization", format!("Bearer {mfa_token}")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED, "an mfa-pending token must not authenticate a normal API request");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn completing_mfa_verify_with_a_correct_totp_code_issues_a_working_session_token(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let app = build_router(state.clone());

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"sup3r-s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(login_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mfa_token = json["mfa_token"].as_str().unwrap();

        // The confirm step already consumed the current step's code; verify with the next one.
        let credential = state.totp_credentials.get(user_id).await.unwrap().unwrap();
        let code = artiferris_application::use_cases::mfa::generate_totp_code_after_step(&credential.secret, credential.last_used_step.unwrap());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}","code":"{code}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_token = json["token"].as_str().unwrap();

        let me_response = app.oneshot(Request::builder().uri("/api/me").header("authorization", format!("Bearer {session_token}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(me_response.status(), axum::http::StatusCode::OK, "the token issued by mfa/verify must work as a normal session token");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn mfa_verify_with_a_wrong_code_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let app = build_router(state);

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"sup3r-s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(login_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}","code":"000000"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn mfa_verify_with_a_session_token_instead_of_an_mfa_token_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let session_token = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{session_token}","code":"000000"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn starting_mfa_passkey_with_an_invalid_mfa_token_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/passkey/start")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"mfa_token":"not-a-real-token"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn starting_mfa_passkey_with_a_session_token_instead_of_an_mfa_token_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let session_token = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/passkey/start")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{session_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn starting_mfa_passkey_fails_when_the_account_has_no_registered_passkey(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let enrollment = state.enroll_totp.execute(user_id, "florian", Some("sup3r-s3cret!")).await.unwrap();
        let code = artiferris_application::use_cases::mfa::generate_current_totp_code(&enrollment.secret_base32);
        state.confirm_totp.execute(user_id, "florian", &code).await.unwrap();
        let app = build_router(state.clone());

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"sup3r-s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(login_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/passkey/start")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn finishing_mfa_passkey_with_an_unknown_challenge_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let enrollment = state.enroll_totp.execute(user_id, "florian", Some("sup3r-s3cret!")).await.unwrap();
        let code = artiferris_application::use_cases::mfa::generate_current_totp_code(&enrollment.secret_base32);
        state.confirm_totp.execute(user_id, "florian", &code).await.unwrap();
        let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/passkey/finish")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"mfa_token":"{mfa_token}","challenge_id":"{}","credential":{{"id":"AAAA","rawId":"AAAA","response":{{"authenticatorData":"","clientDataJSON":"","signature":""}},"type":"public-key"}}}}"#,
                        uuid::Uuid::new_v4()
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    async fn login_response(app: axum::Router, username: &str, password: &str) -> serde_json::Value {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"username":"{username}","password":"{password}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    /// Route-level counterpart to `enrolling_totp_requires_the_current_password` in
    /// `use_cases::mfa`'s own tests — a hijacked session token alone must not be enough to plant a
    /// new TOTP factor on `/api/me/mfa/totp/enroll` (M-7).
    ///
    /// Uses `authenticate_user` directly, not the `/api/auth/login` HTTP route via `login_response`
    /// — this app enforces mandatory MFA setup at login, so a freshly created, not-yet-enrolled user
    /// never gets a full session `token` back from that route (only an `mfa_token`). Every other
    /// session-authenticated route test in `routes::mfa`'s test module obtains its bearer token the
    /// same way, for the same reason.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn enrolling_totp_via_the_route_requires_the_current_password(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/mfa/totp/enroll")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&serde_json::json!({ "current_password": "wrong" })).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn enrolling_totp_during_mandatory_setup_issues_a_working_session_token(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        assert_eq!(json["mfa_setup_required"], true);
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let enroll_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/enroll")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(enroll_response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(enroll_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let secret = json["secret"].as_str().unwrap();
        let code = artiferris_application::use_cases::mfa::generate_current_totp_code(secret);

        let confirm_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/confirm")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}","code":"{code}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(confirm_response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(confirm_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["backup_codes"].as_array().unwrap().len(), 10);
        let session_token = json["token"].as_str().unwrap();

        let me_response = app.oneshot(Request::builder().uri("/api/me").header("authorization", format!("Bearer {session_token}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(me_response.status(), axum::http::StatusCode::OK);
    }

    /// mfa_token is issued on password alone, so setup must refuse an account that already has a factor.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn mandatory_totp_setup_is_rejected_once_the_account_already_has_a_passkey(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        state
            .webauthn_credentials
            .insert(&artiferris_domain::webauthn::WebauthnCredential { id: uuid::Uuid::new_v4(), user_id, name: "YubiKey".to_string(), passkey_data: b"opaque".to_vec(), created_at: chrono::Utc::now() }, None)
            .await
            .unwrap();
        let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/enroll")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn mandatory_passkey_setup_is_rejected_once_the_account_already_has_confirmed_totp(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/passkey/start")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn logging_in_again_after_totp_setup_no_longer_requires_setup(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let app = build_router(state);

        let json = login_response(app, "florian", "sup3r-s3cret!").await;

        assert_eq!(json["mfa_setup_required"], false);
        assert!(json["mfa_token"].is_string(), "the account now has a confirmed factor, so it goes through verify, not setup");
        assert_eq!(json["mfa_has_totp"], true);
        assert_eq!(json["mfa_has_passkey"], false);
    }

    /// Guards a real bug class: the response must say *which* factor exists, not just *whether* one does — an account with only a passkey must not show a TOTP field.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn logging_in_with_only_a_passkey_reports_has_passkey_but_not_has_totp(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_a_passkey(&state, user_id).await;
        let app = build_router(state);

        let json = login_response(app, "florian", "sup3r-s3cret!").await;

        assert_eq!(json["mfa_setup_required"], false);
        assert_eq!(json["mfa_has_totp"], false);
        assert_eq!(json["mfa_has_passkey"], true);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn logging_in_with_both_factors_reports_both_as_available(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        enroll_a_passkey(&state, user_id).await;
        let app = build_router(state);

        let json = login_response(app, "florian", "sup3r-s3cret!").await;

        assert_eq!(json["mfa_has_totp"], true);
        assert_eq!(json["mfa_has_passkey"], true);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn confirming_totp_setup_with_a_wrong_code_is_throttled_and_never_issues_a_token(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        let mfa_token = json["mfa_token"].as_str().unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/enroll")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/confirm")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}","code":"000000"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn totp_setup_with_an_invalid_mfa_token_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/enroll")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"mfa_token":"not-a-real-token"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn passkey_setup_start_during_mandatory_enrollment_returns_a_challenge(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/passkey/start")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["challenge_id"].is_string());
        assert_eq!(json["public_key"]["user"]["name"], "florian");
    }

    fn passkey_start_request(uri: &str, mfa_token: &str, ip: [u8; 4]) -> Request<Body> {
        let mut request = json_post(uri, serde_json::json!({ "mfa_token": mfa_token }));
        request.extensions_mut().insert(ConnectInfo(SocketAddr::from((ip, 51234))));
        request
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn passkey_setup_starts_are_throttled_per_client(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);
        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        let mfa_token = json["mfa_token"].as_str().unwrap();

        for i in 0..PASSKEY_START_ATTEMPTS {
            let response = app.clone().oneshot(passkey_start_request("/api/auth/mfa/setup/passkey/start", mfa_token, [203, 0, 113, 9])).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK, "start {i} is within the budget");
        }

        let refused = app.clone().oneshot(passkey_start_request("/api/auth/mfa/setup/passkey/start", mfa_token, [203, 0, 113, 9])).await.unwrap();
        assert_eq!(refused.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        let other_client = app.oneshot(passkey_start_request("/api/auth/mfa/setup/passkey/start", mfa_token, [198, 51, 100, 1])).await.unwrap();
        assert_eq!(other_client.status(), axum::http::StatusCode::OK, "another client keeps its own budget");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn passkey_login_starts_are_throttled_per_client(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_a_passkey(&state, user_id).await;
        let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
        let app = build_router(state);

        for _ in 0..PASSKEY_START_ATTEMPTS {
            let response = app.clone().oneshot(passkey_start_request("/api/auth/mfa/passkey/start", &mfa_token, [203, 0, 113, 9])).await.unwrap();
            assert_ne!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        }

        let refused = app.oneshot(passkey_start_request("/api/auth/mfa/passkey/start", &mfa_token, [203, 0, 113, 9])).await.unwrap();
        assert_eq!(refused.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn passkey_setup_finish_with_an_unknown_challenge_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/passkey/finish")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"mfa_token":"{mfa_token}","challenge_id":"{}","name":"My key","credential":{{"id":"AAAA","rawId":"AAAA","response":{{"attestationObject":"","clientDataJSON":""}},"type":"public-key"}}}}"#,
                        uuid::Uuid::new_v4()
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    async fn error_of(response: axum::response::Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"].as_str().unwrap().to_string()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn passkey_setup_finish_is_refused_once_a_factor_was_enrolled_after_it_started(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state.clone());
        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        let mfa_token = json["mfa_token"].as_str().unwrap();
        let started = app.clone().oneshot(json_post("/api/auth/mfa/setup/passkey/start", serde_json::json!({ "mfa_token": mfa_token }))).await.unwrap();
        let challenge_id = serde_json::from_slice::<serde_json::Value>(&to_bytes(started.into_body(), usize::MAX).await.unwrap()).unwrap()["challenge_id"].as_str().unwrap().to_string();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;

        let response = app
            .oneshot(json_post(
                "/api/auth/mfa/setup/passkey/finish",
                serde_json::json!({
                    "mfa_token": mfa_token,
                    "challenge_id": challenge_id,
                    "name": "My key",
                    "credential": { "id": "AAAA", "rawId": "AAAA", "response": { "attestationObject": "", "clientDataJSON": "" }, "type": "public-key" },
                }),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(error_of(response).await, "two-factor authentication is already enabled", "refused for the existing factor, not for the ceremony");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn totp_setup_confirm_is_refused_once_a_passkey_was_enrolled_after_it_started(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state.clone());
        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        let mfa_token = json["mfa_token"].as_str().unwrap();
        let enrolled = app.clone().oneshot(json_post("/api/auth/mfa/setup/totp/enroll", serde_json::json!({ "mfa_token": mfa_token }))).await.unwrap();
        let secret = serde_json::from_slice::<serde_json::Value>(&to_bytes(enrolled.into_body(), usize::MAX).await.unwrap()).unwrap()["secret"].as_str().unwrap().to_string();
        enroll_a_passkey(&state, user_id).await;
        let code = artiferris_application::use_cases::mfa::generate_current_totp_code(&secret);

        let response = app.oneshot(json_post("/api/auth/mfa/setup/totp/confirm", serde_json::json!({ "mfa_token": mfa_token, "code": code }))).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(error_of(response).await, "two-factor authentication is already enabled");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn login_fails_with_wrong_password(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"wrong"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    fn activate_request(token: &str, new_password: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/auth/activate")
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"token":"{token}","new_password":"{new_password}"}}"#)))
            .unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn activating_with_a_valid_token_sets_a_working_password(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "invitee", "placeholder-not-usable", false).await.unwrap();
        state
            .user_invitations
            .upsert(&artiferris_domain::invitation::UserInvitation {
                user_id,
                token_hash: artiferris_application::use_cases::invitation::hash_invitation_token("raw-test-token"),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
            }, None)
            .await
            .unwrap();
        let app = build_router(state);

        let response = app.clone().oneshot(activate_request("raw-test-token", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);

        let login = app.oneshot(login_request("invitee", "new-s3cret!")).await.unwrap();
        assert_eq!(login.status(), axum::http::StatusCode::OK);
    }

    async fn invite_with_known_token(state: &AppState, organization_id: Uuid, username: &str, email: &str, token: &str) -> Uuid {
        let user_id = state.invite_user.execute(organization_id, false, username, email, false, Uuid::new_v4()).await.unwrap();
        state
            .user_invitations
            .upsert(&artiferris_domain::invitation::UserInvitation {
                user_id,
                token_hash: artiferris_application::use_cases::invitation::hash_invitation_token(token),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
            }, None)
            .await
            .unwrap();
        user_id
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_invited_email_becomes_verified_only_when_the_invitation_is_activated(pool: sqlx::PgPool) {
        use artiferris_domain::user::UserSecurityPort;
        let security = artiferris_infrastructure::postgres::user_repository::PostgresUserRepository::new(pool.clone());
        let state = AppState::build(pool, &test_config());
        let public = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let user_id = invite_with_known_token(&state, public, "invitee", "invitee@corp.example", "raw-test-token").await;
        let app = build_router(state);

        assert!(security.find_by_verified_email(public, "invitee@corp.example").await.unwrap().is_none(), "an unredeemed invitation proves nothing about the address");

        let response = app.oneshot(activate_request("raw-test-token", "new-s3cret!")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        assert_eq!(security.find_by_verified_email(public, "invitee@corp.example").await.unwrap().map(|u| u.id), Some(user_id));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_organizations_can_invite_the_same_email_and_each_verifies_its_own(pool: sqlx::PgPool) {
        use artiferris_domain::user::UserSecurityPort;
        let security = artiferris_infrastructure::postgres::user_repository::PostgresUserRepository::new(pool.clone());
        let state = AppState::build(pool, &test_config());
        let public = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let acme = state.create_organization.execute("acme", "Acme").await.unwrap();
        let in_public = invite_with_known_token(&state, public, "victim", "victim@corp.example", "public-token").await;
        let in_acme = invite_with_known_token(&state, acme, "victim-acme", "victim@corp.example", "acme-token").await;
        let app = build_router(state);

        assert_eq!(app.clone().oneshot(activate_request("public-token", "new-s3cret!")).await.unwrap().status(), axum::http::StatusCode::NO_CONTENT);
        assert_eq!(app.oneshot(activate_request("acme-token", "new-s3cret!")).await.unwrap().status(), axum::http::StatusCode::NO_CONTENT);

        assert_eq!(security.find_by_verified_email(public, "victim@corp.example").await.unwrap().map(|u| u.id), Some(in_public));
        assert_eq!(security.find_by_verified_email(acme, "victim@corp.example").await.unwrap().map(|u| u.id), Some(in_acme));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_burst_of_parallel_activations_of_one_link_lets_exactly_one_through(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let public = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        invite_with_known_token(&state, public, "invitee", "invitee@corp.example", "raw-test-token").await;
        let app = build_router(state);

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let app = app.clone();
                tokio::spawn(async move { status_of(&app, activate_request("raw-test-token", &format!("new-s3cret-{i}!"))).await })
            })
            .collect();
        let statuses: Vec<_> = futures::future::join_all(handles).await.into_iter().map(|r| r.unwrap()).collect();

        assert_eq!(statuses.iter().filter(|s| **s == axum::http::StatusCode::NO_CONTENT).count(), 1, "{statuses:?}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn activating_with_an_unknown_token_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(activate_request("not-a-real-token", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn activating_with_an_expired_token_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "invitee", "placeholder-not-usable", false).await.unwrap();
        state
            .user_invitations
            .upsert(&artiferris_domain::invitation::UserInvitation {
                user_id,
                token_hash: artiferris_application::use_cases::invitation::hash_invitation_token("raw-test-token"),
                expires_at: chrono::Utc::now() - chrono::Duration::hours(1),
            }, None)
            .await
            .unwrap();
        let app = build_router(state);

        let response = app.oneshot(activate_request("raw-test-token", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    fn login_request(username: &str, password: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"username":"{username}","password":"{password}"}}"#)))
            .unwrap()
    }

    fn register_request(username: &str, email: &str, password: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/auth/register")
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"username":"{username}","email":"{email}","password":"{password}"}}"#)))
            .unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn registering_on_the_public_organization_returns_a_mfa_setup_response(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(register_request("florian", "florian@example.com", "sup3r-s3cret!")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["token"].is_null());
        assert!(json["mfa_token"].is_string());
        assert_eq!(json["mfa_setup_required"], true);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn registering_while_registration_is_disabled_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let mut settings = state.get_system_settings.execute(artiferris_domain::organization::PUBLIC_ORGANIZATION_ID).await.unwrap();
        settings.registration_enabled = false;
        state.update_system_settings.execute(artiferris_domain::organization::PUBLIC_ORGANIZATION_ID, settings, None).await.unwrap();
        let app = build_router(state);

        let response = app.oneshot(register_request("florian", "florian@example.com", "sup3r-s3cret!")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "public self-registration is currently disabled");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn sso_config_reports_registration_enabled_by_default_and_reflects_the_setting(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let mut settings = state.get_system_settings.execute(artiferris_domain::organization::PUBLIC_ORGANIZATION_ID).await.unwrap();
        let app = build_router(state.clone());

        let response = app.clone().oneshot(Request::builder().uri("/api/auth/sso/config").body(Body::empty()).unwrap()).await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["registration_enabled"], true, "must default to enabled");

        settings.registration_enabled = false;
        state.update_system_settings.execute(artiferris_domain::organization::PUBLIC_ORGANIZATION_ID, settings, None).await.unwrap();

        let response = app.oneshot(Request::builder().uri("/api/auth/sso/config").body(Body::empty()).unwrap()).await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["registration_enabled"], false);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn sso_config_reports_registration_disabled_on_a_non_public_organization_even_when_the_setting_is_on(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state
            .organizations
            .create(&artiferris_domain::organization::Organization {
                id: Uuid::new_v4(),
                slug: artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let app = build_router(state);

        let mut request = Request::builder().uri("/api/auth/sso/config").body(Body::empty()).unwrap();
        request.headers_mut().insert("host", "acme.artiferris.localhost".parse().unwrap());
        let response = app.oneshot(request).await.unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["registration_enabled"], false);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_registered_account_can_complete_enrollment_and_log_in_again(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.clone().oneshot(register_request("florian", "florian@example.com", "sup3r-s3cret!")).await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let enroll_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/enroll")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(enroll_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let secret = json["secret"].as_str().unwrap();
        let code = artiferris_application::use_cases::mfa::generate_current_totp_code(secret);

        let confirm_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/confirm")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}","code":"{code}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(confirm_response.status(), axum::http::StatusCode::OK);

        // The real password set at registration must work immediately — no activation step.
        let login = app.oneshot(login_request("florian", "sup3r-s3cret!")).await.unwrap();
        assert_eq!(login.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn registering_on_a_non_public_organization_subdomain_is_rejected(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state
            .organizations
            .create(&artiferris_domain::organization::Organization {
                id: Uuid::new_v4(),
                slug: artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let app = build_router(state);

        let mut request = register_request("florian", "florian@example.com", "sup3r-s3cret!");
        request.headers_mut().insert("host", "acme.artiferris.localhost".parse().unwrap());
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn registering_a_duplicate_username_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);
        app.clone().oneshot(register_request("florian", "a@example.com", "sup3r-s3cret!")).await.unwrap();

        let response = app.oneshot(register_request("florian", "b@example.com", "sup3r-s3cret!")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn repeated_registration_attempts_from_the_same_ip_are_eventually_throttled(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        for i in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let mut request = register_request(&format!("florian{i}"), &format!("florian{i}@example.com"), "sup3r-s3cret!");
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 9], 51234))));
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK, "attempt {i} should still succeed, distinct username/email each time");
        }

        let mut request = register_request("one-too-many", "one-too-many@example.com", "sup3r-s3cret!");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 9], 51234))));
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_fresh_ip_is_unaffected_by_another_ips_registration_attempts(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        for i in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let mut request = register_request(&format!("attacker{i}"), &format!("attacker{i}@example.com"), "sup3r-s3cret!");
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 9], 51234))));
            app.clone().oneshot(request).await.unwrap();
        }

        let mut request = register_request("florian", "florian@example.com", "sup3r-s3cret!");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([198, 51, 100, 1], 51234))));
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn registering_an_email_already_in_use_looks_exactly_like_registering_a_fresh_one(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);
        let first = app.clone().oneshot(register_request("florian", "shared@example.com", "sup3r-s3cret!")).await.unwrap();

        let second = app.oneshot(register_request("someoneelse", "shared@example.com", "sup3r-s3cret!")).await.unwrap();

        assert_eq!(first.status(), axum::http::StatusCode::OK);
        assert_eq!(second.status(), axum::http::StatusCode::OK, "whether an email is registered must not show in the response");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn registering_with_a_weak_password_fails(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(register_request("florian", "florian@example.com", "short")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_few_failed_attempts_do_not_block_a_later_correct_login(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        for _ in 0..(artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS - 1) {
            let response = app.clone().oneshot(login_request("florian", "wrong")).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        }

        let response = app.oneshot(login_request("florian", "sup3r-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn hitting_the_failure_threshold_rejects_even_correct_credentials(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        for _ in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let response = app.clone().oneshot(login_request("florian", "wrong")).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        }

        let response = app.oneshot(login_request("florian", "sup3r-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn failing_login_for_many_different_usernames_from_one_ip_eventually_throttles_that_ip(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);
        let ip = std::net::SocketAddr::from(([203, 0, 113, 50], 51234));

        // Send MAX_LOGIN_ATTEMPTS failed logins for MAX_LOGIN_ATTEMPTS distinct nonexistent usernames, all from `ip`.
        for i in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let mut req = login_request(&format!("nonexistent-{i}"), "wrong-password");
            req.extensions_mut().insert(axum::extract::ConnectInfo(ip));
            let response = app.clone().oneshot(req).await.unwrap();
            assert_ne!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS, "attempt {i} must not itself be throttled yet");
        }

        let mut req = login_request("yet-another-nonexistent-user", "wrong-password");
        req.extensions_mut().insert(axum::extract::ConnectInfo(ip));
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS, "the IP itself must now be throttled even against a brand-new username");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_throttled_ip_does_not_block_a_different_ip(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "real-user", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);
        let attacker_ip = std::net::SocketAddr::from(([203, 0, 113, 51], 1));
        for i in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let mut req = login_request(&format!("nonexistent-{i}"), "wrong-password");
            req.extensions_mut().insert(axum::extract::ConnectInfo(attacker_ip));
            app.clone().oneshot(req).await.unwrap();
        }

        let victim_ip = std::net::SocketAddr::from(([203, 0, 113, 52], 1));
        let mut req = login_request("real-user", "sup3r-s3cret!");
        req.extensions_mut().insert(axum::extract::ConnectInfo(victim_ip));
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK, "a different IP's legitimate login must not be affected by another IP's lockout");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_successful_login_resets_the_failure_counter(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);
        // Two distinct IPs so the new per-IP throttle (B-2) — which is deliberately not cleared
        // on success — never itself reaches its own cap across this test's two bursts of failures.
        // This test is about the per-USERNAME counter resetting on success, not about IP throttling.
        let ip_a = SocketAddr::from(([203, 0, 113, 21], 1));
        let ip_b = SocketAddr::from(([203, 0, 113, 22], 1));

        for _ in 0..(artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS - 1) {
            let mut request = login_request("florian", "wrong");
            request.extensions_mut().insert(ConnectInfo(ip_a));
            app.clone().oneshot(request).await.unwrap();
        }
        let mut request = login_request("florian", "sup3r-s3cret!");
        request.extensions_mut().insert(ConnectInfo(ip_a));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let mut request = login_request("florian", "wrong");
        request.extensions_mut().insert(ConnectInfo(ip_b));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        let mut request = login_request("florian", "sup3r-s3cret!");
        request.extensions_mut().insert(ConnectInfo(ip_b));
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    async fn login_failure_ips(state: &AppState) -> Vec<String> {
        let entries = state
            .query_audit_log
            .execute(artiferris_domain::audit::AuditQueryFilter {
                aggregate_type: Some("Security".to_string()),
                ..Default::default()
            })
            .await
            .unwrap()
            .entries;
        entries
            .into_iter()
            .filter(|e| e.event_type == "LoginFailed")
            .map(|e| e.payload["ip"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failed_login_records_the_connecting_peers_ip(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state.clone());

        let mut request = login_request("florian", "wrong");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 7], 51234))));
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);

        assert_eq!(login_failure_ips(&state).await, vec!["203.0.113.7".to_string()]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failed_login_without_connect_info_still_succeeds_at_auditing(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state.clone());

        let response = app.oneshot(login_request("florian", "wrong")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);

        assert_eq!(login_failure_ips(&state).await, vec!["unknown".to_string()]);
    }

    async fn login_failure_usernames(state: &AppState) -> Vec<String> {
        security_events_of_type(state, "LoginFailed").await.into_iter().map(|payload| payload["username"].as_str().unwrap_or_default().to_string()).collect()
    }

    /// A fresh IP per attempt, so only the per-username budget is under test — the per-IP one caps
    /// at the same number and would otherwise fire first and prove nothing.
    fn login_request_from(username: &str, password: &str, last_octet: u8) -> Request<Body> {
        let mut request = login_request(username, password);
        request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, last_octet], 51234))));
        request
    }

    /// These are all one account, so an attacker must not get a fresh budget per case variant.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn failed_logins_across_username_case_variants_share_one_throttle_budget(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);
        let variants = ["alice", "Alice", "ALICE", "aLiCe"];

        for i in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let response = app.clone().oneshot(login_request_from(variants[i % variants.len()], "wrong", 100 + i as u8)).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED, "attempt {i} must count as a normal failure, not be throttled yet");
        }

        let response = app.oneshot(login_request_from("ALICE", "sup3r-s3cret!", 200)).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS, "case variants of one username must share one throttle budget, not get one each");
    }

    /// Otherwise one attack scatters across several apparent identities in the audit trail.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failed_login_records_the_normalized_username(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "alice", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state.clone());

        app.clone().oneshot(login_request_from("ALICE", "wrong", 120)).await.unwrap();
        app.oneshot(login_request_from("aLiCe", "wrong", 121)).await.unwrap();

        assert_eq!(login_failure_usernames(&state).await, vec!["alice".to_string(), "alice".to_string()]);
    }

    /// Otherwise the dodge just moves to values `Username::parse` rejects.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn failed_logins_for_an_invalid_username_are_normalized_too(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state.clone());

        app.clone().oneshot(login_request_from("NOT A VALID NAME", "wrong", 130)).await.unwrap();
        app.oneshot(login_request_from("not a valid name", "wrong", 131)).await.unwrap();

        assert_eq!(login_failure_usernames(&state).await, vec!["not a valid name".to_string(), "not a valid name".to_string()]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn failed_ldap_logins_across_username_case_variants_share_one_throttle_budget(pool: sqlx::PgPool) {
        // Every attempt fails at the directory bind (no real LDAP server here) — fine, the throttle
        // sits in front of it either way.
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_ldap_config(&state, public_org.id).await;
        let app = build_router(state);
        let variants = ["someuser", "SomeUser", "SOMEUSER", "sOmEuSeR"];

        for i in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let mut request = ldap_login_request(variants[i % variants.len()], "wrong-password");
            request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 140 + i as u8], 51234))));
            let response = app.clone().oneshot(request).await.unwrap();
            assert_ne!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS, "attempt {i} must not itself be throttled yet");
        }

        let mut request = ldap_login_request("SOMEUSER", "wrong-password");
        request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 201], 51234))));
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS, "case variants of one username must share one throttle budget on the LDAP path too");
    }

    #[test]
    fn every_case_variant_of_a_username_normalizes_to_the_same_key() {
        for variant in ["alice", "Alice", "ALICE", "aLiCe"] {
            assert_eq!(normalized_username(variant), "alice", "variant {variant} must not get its own throttle budget");
        }
        assert_eq!(normalized_username("NOT A VALID NAME"), "not a valid name", "an unparseable username still normalizes");
    }

    fn test_config_with_trusted_proxies(ips: &[&str]) -> Config {
        Config { trusted_proxy_ips: ips.iter().map(|ip| (*ip).to_string()).collect(), ..test_config() }
    }

    /// The audited IP is `peer_ip`'s output, so it's what these assert on.
    async fn recorded_login_failure_ip(pool: sqlx::PgPool, trusted_proxies: &[&str], direct_peer: [u8; 4], forwarded_for: Option<&str>) -> String {
        let state = AppState::build(pool, &test_config_with_trusted_proxies(trusted_proxies));
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state.clone());

        let mut request = login_request("florian", "wrong");
        request.extensions_mut().insert(ConnectInfo(SocketAddr::from((direct_peer, 51234))));
        if let Some(forwarded_for) = forwarded_for {
            request.headers_mut().insert("x-forwarded-for", forwarded_for.parse().unwrap());
        }
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);

        login_failure_ips(&state).await.into_iter().next().expect("the failed login must have been audited")
    }

    /// "1.2.3.4" is the attacker's made-up value; taking it would let them mint a key per request.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_append_style_proxy_gets_the_real_client_not_the_clients_own_claim(pool: sqlx::PgPool) {
        let ip = recorded_login_failure_ip(pool, &["203.0.113.1"], [203, 0, 113, 1], Some("1.2.3.4, 198.51.100.7")).await;
        assert_eq!(ip, "198.51.100.7");
    }

    /// One entry either way, so a replace-style proxy sees no behavior change.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_replace_style_proxy_still_gets_its_single_forwarded_entry(pool: sqlx::PgPool) {
        let ip = recorded_login_failure_ip(pool, &["203.0.113.1"], [203, 0, 113, 1], Some("198.51.100.7")).await;
        assert_eq!(ip, "198.51.100.7");
    }

    /// Each hop appends the peer it saw, so the tail is a run of trusted proxies to walk back over.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_chain_of_trusted_proxies_skips_past_every_hop_to_the_real_client(pool: sqlx::PgPool) {
        let ip = recorded_login_failure_ip(pool, &["203.0.113.1", "203.0.113.2"], [203, 0, 113, 1], Some("9.9.9.9, 198.51.100.7, 203.0.113.2")).await;
        assert_eq!(ip, "198.51.100.7");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_header_containing_only_trusted_proxies_falls_back_to_the_direct_peer(pool: sqlx::PgPool) {
        let ip = recorded_login_failure_ip(pool, &["203.0.113.1", "203.0.113.2"], [203, 0, 113, 1], Some("203.0.113.2, 203.0.113.1")).await;
        assert_eq!(ip, "203.0.113.1");
    }

    /// Any client can set this header, so it's only read once the peer is a configured proxy.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_untrusted_peers_forwarded_header_is_ignored_entirely(pool: sqlx::PgPool) {
        let ip = recorded_login_failure_ip(pool, &["203.0.113.1"], [198, 51, 100, 99], Some("1.2.3.4")).await;
        assert_eq!(ip, "198.51.100.99");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_trusted_proxy_with_no_forwarded_header_falls_back_to_the_direct_peer(pool: sqlx::PgPool) {
        let ip = recorded_login_failure_ip(pool, &["203.0.113.1"], [203, 0, 113, 1], None).await;
        assert_eq!(ip, "203.0.113.1");
    }

    async fn security_events_of_type(state: &AppState, event_type: &str) -> Vec<serde_json::Value> {
        let entries = state
            .query_audit_log
            .execute(artiferris_domain::audit::AuditQueryFilter {
                aggregate_type: Some("Security".to_string()),
                ..Default::default()
            })
            .await
            .unwrap()
            .entries;
        entries.into_iter().filter(|e| e.event_type == event_type).map(|e| e.payload).collect()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failed_mfa_verification_is_recorded_as_a_security_event(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
        let app = build_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&serde_json::json!({ "mfa_token": mfa_token, "code": "000000" })).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);

        let events = security_events_of_type(&state, "MfaVerificationFailed").await;
        assert!(events.iter().any(|payload| payload["user_id"] == user_id.to_string() && payload["method"] == "totp"));
    }

    #[sqlx::test]
    async fn me_requires_a_bearer_token(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/api/me").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn me_returns_the_authenticated_user(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", true).await.unwrap();
        let token = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/me")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], user_id.to_string());
        assert_eq!(json["username"], "florian");
        assert_eq!(json["is_super_admin"], true);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn me_returns_the_users_organization_fields(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let public_org_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let user_id = state.create_user.execute(public_org_id, "florian", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(Request::builder().uri("/api/me").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["organization_id"], public_org_id.to_string());
        assert_eq!(json["is_organization_admin"], false);
        let _ = user_id;
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn me_returns_the_users_created_at(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(Request::builder().uri("/api/me").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["created_at"].is_string(), "expected created_at in /api/me response, got {json}");
    }

    fn change_password_request(token: &str, current_password: &str, new_password: &str) -> Request<Body> {
        Request::builder()
            .method("PUT")
            .uri("/api/me/password")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(format!(r#"{{"current_password":"{current_password}","new_password":"{new_password}"}}"#)))
            .unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn changes_the_password_with_the_correct_current_password(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app.clone().oneshot(change_password_request(&token, "old-s3cret!", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);

        let login_response = app.oneshot(login_request("florian", "new-s3cret!")).await.unwrap();
        assert_eq!(login_response.status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn rejects_a_password_change_with_the_wrong_current_password(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app.oneshot(change_password_request(&token, "wrong", "new-s3cret!")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn rejects_a_new_password_shorter_than_the_minimum_via_the_route(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app.oneshot(change_password_request(&token, "old-s3cret!", "short")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test]
    async fn change_password_requires_a_bearer_token(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/me/password")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"current_password":"a","new_password":"new-s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    async fn password_change_failure_ips(state: &AppState) -> Vec<String> {
        let entries = state
            .query_audit_log
            .execute(artiferris_domain::audit::AuditQueryFilter {
                aggregate_type: Some("Security".to_string()),
                ..Default::default()
            })
            .await
            .unwrap()
            .entries;
        entries
            .into_iter()
            .filter(|e| e.event_type == "PasswordChangeFailed")
            .map(|e| e.payload["ip"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_few_failed_password_change_attempts_do_not_block_a_later_correct_one(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state);

        for _ in 0..(artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS - 1) {
            let response = app.clone().oneshot(change_password_request(&token, "wrong", "new-s3cret!")).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        }

        let response = app.oneshot(change_password_request(&token, "old-s3cret!", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn hitting_the_failure_threshold_rejects_even_a_correct_password_change(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state);

        for _ in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let response = app.clone().oneshot(change_password_request(&token, "wrong", "new-s3cret!")).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        }

        let response = app.oneshot(change_password_request(&token, "old-s3cret!", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failed_password_change_records_a_security_event_with_the_connecting_peers_ip(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        let mut request = change_password_request(&token, "wrong", "new-s3cret!");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 7], 51234))));
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

        assert_eq!(password_change_failure_ips(&state).await, vec!["203.0.113.7".to_string()]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_weak_new_password_does_not_count_against_the_throttle_or_get_audited(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        for _ in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let response = app.clone().oneshot(change_password_request(&token, "old-s3cret!", "short")).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        }

        assert!(password_change_failure_ips(&state).await.is_empty());

        let response = app.oneshot(change_password_request(&token, "old-s3cret!", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_successful_password_change_after_some_failures_clears_the_throttle(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "old-s3cret!").await;
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        for _ in 0..(artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS - 1) {
            app.clone().oneshot(change_password_request(&token, "wrong", "new-s3cret!")).await.unwrap();
        }
        let response = app.clone().oneshot(change_password_request(&token, "old-s3cret!", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);

        let token = login_and_get_token(&app, &state, user_id, "new-s3cret!").await;
        let response = app.clone().oneshot(login_request("florian", "wrong")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        let response = app.oneshot(change_password_request(&token, "old-s3cret!", "new-s3cret!")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    async fn login_and_get_token(app: &Router, state: &AppState, user_id: uuid::Uuid, password: &str) -> String {
        let response = app.clone().oneshot(login_request("florian", password)).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mfa_token = json["mfa_token"].as_str().unwrap().to_string();

        let credential = state.totp_credentials.get(user_id).await.unwrap().unwrap();
        let code = artiferris_application::use_cases::mfa::generate_totp_code_after_step(&credential.secret, credential.last_used_step.unwrap());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}","code":"{code}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        json["token"].as_str().unwrap().to_string()
    }

    async fn seed_ldap_config(state: &AppState, organization_id: Uuid) {
        state
            .identity_providers
            .set(
                organization_id,
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
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn sso_config_reports_no_provider_for_an_unconfigured_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/api/auth/sso/config").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["type"].is_null());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn sso_config_reports_ldap_for_a_configured_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_ldap_config(&state, public_org.id).await;
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/api/auth/sso/config").body(Body::empty()).unwrap()).await.unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["type"], "ldap");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn ldap_login_is_rejected_when_the_organization_has_no_identity_provider_configured(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/sso/ldap")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failed_ldap_bind_is_rejected_as_unauthorized(pool: sqlx::PgPool) {
        // No real directory here, so the connection itself fails — proves that surfaces as 401.
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_ldap_config(&state, public_org.id).await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/sso/ldap")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"florian","password":"s3cret!"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    fn ldap_login_request(username: &str, password: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/auth/sso/ldap")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "username": username, "password": password }).to_string()))
            .unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn repeated_failed_ldap_logins_are_eventually_throttled(pool: sqlx::PgPool) {
        // Every attempt fails at the directory bind (no real LDAP server here), so this proves
        // the throttle sits in front of `sso_ldap_login` itself, not just the local-password path.
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_ldap_config(&state, public_org.id).await;
        let app = build_router(state);

        for _ in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let response = app.clone().oneshot(ldap_login_request("someuser", "wrong-password")).await.unwrap();
            assert_ne!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS, "should not be throttled before the limit is reached");
        }

        let response = app.oneshot(ldap_login_request("someuser", "wrong-password")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn failing_ldap_login_for_many_different_usernames_from_one_ip_eventually_throttles_that_ip(pool: sqlx::PgPool) {
        // Mirrors `failing_login_for_many_different_usernames_from_one_ip_eventually_throttles_that_ip`
        // for the LDAP path: an attacker cycling through many usernames from one IP must eventually
        // get throttled on the IP alone, even though no single username ever hits its own limit.
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_ldap_config(&state, public_org.id).await;
        let app = build_router(state);
        let ip = std::net::SocketAddr::from(([203, 0, 113, 60], 51234));

        for i in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let mut req = ldap_login_request(&format!("nonexistent-{i}"), "wrong-password");
            req.extensions_mut().insert(ConnectInfo(ip));
            let response = app.clone().oneshot(req).await.unwrap();
            assert_ne!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS, "attempt {i} must not itself be throttled yet");
        }

        let mut req = ldap_login_request("yet-another-nonexistent-user", "wrong-password");
        req.extensions_mut().insert(ConnectInfo(ip));
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS, "the IP itself must now be throttled even against a brand-new username");
    }

    async fn seed_oidc_config(state: &AppState, organization_id: uuid::Uuid) {
        state
            .identity_providers
            .set(
                organization_id,
                &artiferris_domain::sso::IdentityProviderConfig::Oidc(artiferris_domain::sso::OidcConfig {
                    issuer_url: "https://accounts.example.com".to_string(),
                    client_id: "artiferris".to_string(),
                    client_secret: "s3cret!".to_string(),
                }),
                None,
            )
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn sso_config_reports_oidc_for_a_configured_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/api/auth/sso/config").body(Body::empty()).unwrap()).await.unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["type"], "oidc");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn oidc_login_is_rejected_when_the_organization_has_no_identity_provider_configured(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/api/auth/sso/oidc/login").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn oidc_login_is_rejected_when_the_organization_has_an_ldap_provider_instead(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_ldap_config(&state, public_org.id).await;
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/api/auth/sso/oidc/login").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_callback_with_no_state_parameter_is_rejected_as_unauthorized(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/api/auth/sso/oidc/callback?code=abc").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_callback_with_an_invalid_state_token_is_rejected_as_unauthorized(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        // Cookie is present, so this tests the state-token check, not the missing-cookie one.
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/sso/oidc/callback?code=abc&state=not-a-real-token")
                    .header("cookie", "artiferris_oidc_binding=some-binding-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    /// Stand-in for `OpenidConnectAuthAdapter`, keeping just the org-scoping and browser-binding checks these route tests care about.
    struct FakeOidcAuth;

    /// The authorization code the fake identity provider refuses to exchange.
    const REJECTED_CODE: &str = "rejected-code";

    fn fake_state_token(organization_id: Uuid, binding_secret: &str) -> String {
        format!("fake-state.{organization_id}.{binding_secret}")
    }

    #[async_trait::async_trait]
    impl artiferris_domain::sso::OidcAuthPort for FakeOidcAuth {
        async fn build_redirect(
            &self,
            _config: &artiferris_domain::sso::OidcConfig,
            organization_id: Uuid,
            callback_url: &str,
            binding_secret: &str,
        ) -> Result<String, artiferris_domain::error::DomainError> {
            Ok(format!("https://idp.example/authorize?redirect_uri={callback_url}&state={}", fake_state_token(organization_id, binding_secret)))
        }

        fn state_is_valid(&self, raw_state: &str, expected_organization_id: Uuid, binding_secret: &str) -> bool {
            raw_state == fake_state_token(expected_organization_id, binding_secret)
        }

        async fn handle_callback(
            &self,
            _config: &artiferris_domain::sso::OidcConfig,
            code: &str,
            raw_state: &str,
            _callback_url: &str,
            expected_organization_id: Uuid,
            binding_secret: &str,
        ) -> Result<artiferris_domain::sso::ExternalIdentity, artiferris_domain::error::DomainError> {
            if raw_state != fake_state_token(expected_organization_id, binding_secret) || code == REJECTED_CODE {
                return Err(artiferris_domain::error::DomainError::Infrastructure("oidc state token mismatch".to_string()));
            }
            Ok(artiferris_domain::sso::ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None })
        }
    }

    fn state_with_fake_oidc(pool: sqlx::PgPool) -> AppState {
        let mut state = AppState::build(pool, &test_config());
        state.oidc_auth = std::sync::Arc::new(FakeOidcAuth);
        state
    }

    async fn create_non_public_organization(state: &AppState, slug: &str) -> Uuid {
        let id = Uuid::new_v4();
        state
            .organizations
            .create(&artiferris_domain::organization::Organization {
                id,
                slug: artiferris_domain::organization::OrganizationSlug::parse(slug).unwrap(),
                display_name: slug.to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        id
    }

    fn header_value(response: &axum::response::Response, name: &str) -> String {
        response.headers().get(name).unwrap().to_str().unwrap().to_string()
    }

    /// The value of the `artiferris_oidc_binding` cookie the login response set, as a browser would echo it back.
    fn binding_cookie_value(response: &axum::response::Response) -> String {
        let set_cookie = header_value(response, "set-cookie");
        assert!(set_cookie.starts_with("artiferris_oidc_binding="), "got: {set_cookie}");
        set_cookie.split(';').next().unwrap().trim_start_matches("artiferris_oidc_binding=").to_string()
    }

    /// The `state` query parameter of the identity-provider URL the login response redirected to.
    fn state_token_from_login(response: &axum::response::Response) -> String {
        let location = header_value(response, "location");
        location.split("state=").nth(1).unwrap_or_else(|| panic!("no state parameter in {location}")).to_string()
    }

    fn oidc_login_request(host: &str) -> Request<Body> {
        let mut request = Request::builder().uri("/api/auth/sso/oidc/login").body(Body::empty()).unwrap();
        request.headers_mut().insert("host", host.parse().unwrap());
        request
    }

    fn oidc_callback_request(host: &str, state_token: &str, cookie: Option<&str>) -> Request<Body> {
        oidc_callback_request_with_code(host, "abc", state_token, cookie)
    }

    fn oidc_callback_request_with_code(host: &str, code: &str, state_token: &str, cookie: Option<&str>) -> Request<Body> {
        let mut request = Request::builder()
            .uri(format!("/api/auth/sso/oidc/callback?code={code}&state={state_token}"))
            .body(Body::empty())
            .unwrap();
        request.headers_mut().insert("host", host.parse().unwrap());
        if let Some(cookie) = cookie {
            request.headers_mut().insert("cookie", format!("artiferris_oidc_binding={cookie}").parse().unwrap());
        }
        request
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_oidc_login_redirect_sets_a_browser_binding_cookie(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        let response = app.oneshot(oidc_login_request("artiferris.localhost")).await.unwrap();

        let set_cookie = header_value(&response, "set-cookie");
        assert!(set_cookie.contains("HttpOnly"), "got: {set_cookie}");
        assert!(set_cookie.contains("SameSite=Lax"), "got: {set_cookie}");
        assert!(set_cookie.contains("Path=/"), "got: {set_cookie}");
        assert!(set_cookie.contains("Max-Age=600"), "got: {set_cookie}");
        assert!(!binding_cookie_value(&response).is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_oidc_callback_for_a_non_public_organization_redirects_to_that_organizations_own_origin(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let acme_id = create_non_public_organization(&state, "acme").await;
        seed_oidc_config(&state, acme_id).await;
        let app = build_router(state);

        let login = app.clone().oneshot(oidc_login_request("acme.artiferris.localhost")).await.unwrap();
        let cookie = binding_cookie_value(&login);
        let state_token = state_token_from_login(&login);

        let response = app.oneshot(oidc_callback_request("acme.artiferris.localhost", &state_token, Some(&cookie))).await.unwrap();

        let location = header_value(&response, "location");
        assert!(location.starts_with("http://acme.artiferris.localhost/login#token="), "got: {location}");
        assert!(!location.starts_with(&test_config().public_url), "the callback must not land on the global public_url, got: {location}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_callback_only_completes_for_the_browser_that_started_the_login(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        // The attacker's login attempt — its callback URL is what gets handed to the victim.
        let login_a = app.clone().oneshot(oidc_login_request("artiferris.localhost")).await.unwrap();
        let cookie_a = binding_cookie_value(&login_a);
        let state_a = state_token_from_login(&login_a);

        // The victim's own browser, with its own binding cookie.
        let login_b = app.clone().oneshot(oidc_login_request("artiferris.localhost")).await.unwrap();
        let cookie_b = binding_cookie_value(&login_b);
        assert_ne!(cookie_a, cookie_b, "each login attempt must mint a fresh binding secret");

        let with_the_wrong_cookie = app.clone().oneshot(oidc_callback_request("artiferris.localhost", &state_a, Some(&cookie_b))).await.unwrap();
        assert_eq!(with_the_wrong_cookie.status(), axum::http::StatusCode::UNAUTHORIZED);

        let with_no_cookie = app.clone().oneshot(oidc_callback_request("artiferris.localhost", &state_a, None)).await.unwrap();
        assert_eq!(with_no_cookie.status(), axum::http::StatusCode::UNAUTHORIZED);

        // ...and the browser that actually started attempt A still completes it.
        let with_the_right_cookie = app.oneshot(oidc_callback_request("artiferris.localhost", &state_a, Some(&cookie_a))).await.unwrap();
        let location = header_value(&with_the_right_cookie, "location");
        assert!(location.starts_with("http://app.artiferris.localhost/login#token="), "got: {location}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_successful_callback_clears_the_binding_cookie(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        let login = app.clone().oneshot(oidc_login_request("artiferris.localhost")).await.unwrap();
        let cookie = binding_cookie_value(&login);

        let response = app
            .oneshot(oidc_callback_request("artiferris.localhost", &state_token_from_login(&login), Some(&cookie)))
            .await
            .unwrap();

        let set_cookie = header_value(&response, "set-cookie");
        assert!(set_cookie.starts_with("artiferris_oidc_binding="), "got: {set_cookie}");
        assert!(set_cookie.contains("Max-Age=0"), "the binding cookie must be cleared after a single use, got: {set_cookie}");
        assert!(set_cookie.contains("Path=/"), "the removal must match the path the cookie was set with, got: {set_cookie}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_state_token_minted_on_one_subdomain_does_not_complete_on_another(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let acme_id = create_non_public_organization(&state, "acme").await;
        let other_id = create_non_public_organization(&state, "globex").await;
        seed_oidc_config(&state, acme_id).await;
        seed_oidc_config(&state, other_id).await;
        let app = build_router(state);

        let login = app.clone().oneshot(oidc_login_request("acme.artiferris.localhost")).await.unwrap();
        let cookie = binding_cookie_value(&login);
        let state_token = state_token_from_login(&login);

        let response = app.oneshot(oidc_callback_request("globex.artiferris.localhost", &state_token, Some(&cookie))).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn anonymous_callbacks_write_nothing_into_a_tenants_audit_log_and_run_out_of_budget(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let acme_id = create_non_public_organization(&state, "acme").await;
        seed_oidc_config(&state, acme_id).await;
        let app = build_router(state.clone());

        let mut statuses = Vec::new();
        for attempt in 0..MAX_LOGIN_ATTEMPTS + 2 {
            // No cookie on the even attempts, a cookie with a state nobody minted on the odd ones.
            let cookie = (attempt % 2 == 1).then_some("made-up-binding");
            statuses.push(status_of(&app, oidc_callback_request("acme.artiferris.localhost", "made-up-state", cookie)).await);
        }

        assert!(statuses[..MAX_LOGIN_ATTEMPTS].iter().all(|s| *s == axum::http::StatusCode::UNAUTHORIZED), "got: {statuses:?}");
        assert!(statuses[MAX_LOGIN_ATTEMPTS..].iter().all(|s| *s == axum::http::StatusCode::TOO_MANY_REQUESTS), "got: {statuses:?}");
        assert!(security_events_of_type(&state, "OidcLoginFailed").await.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_callback_for_an_organization_without_oidc_is_refused_and_not_recorded(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        create_non_public_organization(&state, "acme").await;
        let app = build_router(state.clone());

        let response = app.oneshot(oidc_callback_request("acme.artiferris.localhost", "made-up-state", Some("made-up-binding"))).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(security_events_of_type(&state, "OidcLoginFailed").await.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_rejected_code_on_a_real_login_is_recorded_once_per_attempt_and_stays_within_the_budget(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let acme_id = create_non_public_organization(&state, "acme").await;
        seed_oidc_config(&state, acme_id).await;
        let app = build_router(state.clone());
        let login = app.clone().oneshot(oidc_login_request("acme.artiferris.localhost")).await.unwrap();
        let cookie = binding_cookie_value(&login);
        let state_token = state_token_from_login(&login);

        let mut statuses = Vec::new();
        for _ in 0..MAX_LOGIN_ATTEMPTS + 3 {
            statuses.push(status_of(&app, oidc_callback_request_with_code("acme.artiferris.localhost", REJECTED_CODE, &state_token, Some(&cookie))).await);
        }

        assert_eq!(statuses.iter().filter(|s| **s == axum::http::StatusCode::UNAUTHORIZED).count(), MAX_LOGIN_ATTEMPTS, "got: {statuses:?}");
        let recorded = security_events_of_type(&state, "OidcLoginFailed").await;
        assert_eq!(recorded.len(), MAX_LOGIN_ATTEMPTS);
        assert_eq!(recorded[0]["organization_id"], acme_id.to_string());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn successful_oidc_logins_do_not_use_up_the_callback_budget(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        for _ in 0..MAX_LOGIN_ATTEMPTS + 5 {
            let login = app.clone().oneshot(oidc_login_request("artiferris.localhost")).await.unwrap();
            let cookie = binding_cookie_value(&login);
            let response = app.clone().oneshot(oidc_callback_request("artiferris.localhost", &state_token_from_login(&login), Some(&cookie))).await.unwrap();
            assert!(header_value(&response, "location").contains("/login#token="));
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn starting_an_oidc_login_is_limited_per_client(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        for _ in 0..OIDC_START_MAX_ATTEMPTS {
            assert_eq!(status_of(&app, oidc_login_request("artiferris.localhost")).await, axum::http::StatusCode::SEE_OTHER);
        }

        assert_eq!(status_of(&app, oidc_login_request("artiferris.localhost")).await, axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn oidc_logins_that_complete_never_use_up_the_start_budget(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state);

        for _ in 0..OIDC_START_MAX_ATTEMPTS + 10 {
            let login = app.clone().oneshot(oidc_login_request("artiferris.localhost")).await.unwrap();
            assert_eq!(login.status(), axum::http::StatusCode::SEE_OTHER, "an office signing in together is not a flood");
            let cookie = binding_cookie_value(&login);
            let callback = app.clone().oneshot(oidc_callback_request("artiferris.localhost", &state_token_from_login(&login), Some(&cookie))).await.unwrap();
            assert!(header_value(&callback, "location").contains("/login#token="));
        }
    }

    struct SaturatedHasher;

    #[async_trait::async_trait]
    impl artiferris_domain::user::PasswordHasherPort for SaturatedHasher {
        async fn hash(&self, _plain_password: &str) -> Result<String, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("too many passwords are being checked at once".to_string()))
        }
        async fn verify(&self, _plain_password: &str, _hash: &str) -> Result<bool, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("too many passwords are being checked at once".to_string()))
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_saturated_hasher_answers_503_with_retry_after_and_the_attempt_is_neither_counted_nor_audited(pool: sqlx::PgPool) {
        let mut state = AppState::build(pool.clone(), &test_config());
        state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        state.authenticate_user = std::sync::Arc::new(artiferris_application::use_cases::user::AuthenticateUserUseCase::new(
            state.users.clone(),
            std::sync::Arc::new(SaturatedHasher),
            state.token_issuer.clone(),
            std::sync::Arc::new(artiferris_infrastructure::postgres::system_settings_repository::PostgresSystemSettingsRepository::new(pool)),
        ));
        let app = build_router(state.clone());

        for _ in 0..MAX_LOGIN_ATTEMPTS * 3 {
            let response = app.clone().oneshot(login_request("florian", "sup3r-s3cret!")).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE, "never a 401 for a password nobody checked, never a 429 for load the client didn't cause");
            assert_eq!(header_value(&response, "retry-after"), "5");
        }

        assert!(security_events_of_type(&state, "LoginFailed").await.is_empty());
    }

    fn login_request_on(host: &str, username: &str, password: &str) -> Request<Body> {
        let mut request = login_request(username, password);
        request.headers_mut().insert("host", host.parse().unwrap());
        request
    }

    async fn organization_with_login_limit(state: &AppState, slug: &str, max_login_attempts: i32) -> Uuid {
        let organization_id = state.create_organization.execute(slug, slug).await.unwrap();
        state
            .update_system_settings
            .execute(
                organization_id,
                artiferris_domain::system_settings::SystemSettings { max_login_attempts, login_attempt_window_seconds: 300, session_ttl_hours: 12, registration_enabled: true, seo_indexing_enabled: false },
            None,
        )
            .await
            .unwrap();
        organization_id
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_lower_throttle_limit_in_one_organization_does_not_affect_another(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = organization_with_login_limit(&state, "acme", 1).await;
        organization_with_login_limit(&state, "other", 10).await;
        state.create_user.execute(acme_id, "acme-user", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        // One bad attempt on acme's own host is already at acme's 1-attempt limit.
        let first = app.clone().oneshot(login_request_on("acme.artiferris.localhost", "acme-user", "wrong")).await.unwrap();
        assert_eq!(first.status(), axum::http::StatusCode::UNAUTHORIZED);
        let acme_second_attempt = app.clone().oneshot(login_request_on("acme.artiferris.localhost", "acme-user", "wrong")).await.unwrap();
        assert_eq!(acme_second_attempt.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);

        // Another organization's host, on the default 10-attempt limit, is unaffected by acme's lower limit.
        let other_second_attempt = app.clone().oneshot(login_request_on("other.artiferris.localhost", "other-user", "wrong")).await.unwrap();
        let other_third_attempt = app.oneshot(login_request_on("other.artiferris.localhost", "other-user", "wrong")).await.unwrap();
        assert_eq!(other_second_attempt.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(other_third_attempt.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_known_and_an_unknown_username_are_throttled_at_the_same_attempt(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = organization_with_login_limit(&state, "acme", 2).await;
        state.create_user.execute(acme_id, "acme-user", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let statuses = |username: &'static str| {
            let app = app.clone();
            async move {
                let mut seen = Vec::new();
                for i in 0..4u8 {
                    let mut request = login_request_on("acme.artiferris.localhost", username, "wrong");
                    // A fresh IP each time, so only the per-username budget is under test.
                    request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, i + 1], 51234))));
                    seen.push(app.clone().oneshot(request).await.unwrap().status());
                }
                seen
            }
        };

        assert_eq!(statuses("acme-user").await, statuses("no-such-user").await, "the limit must not reveal whether the username exists");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_lenient_organization_cannot_loosen_or_erase_the_shared_username_lock(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let lenient_id = state.create_organization.execute("lenient", "lenient").await.unwrap();
        state
            .update_system_settings
            .execute(
                lenient_id,
                artiferris_domain::system_settings::SystemSettings { max_login_attempts: 1000, login_attempt_window_seconds: 1, session_ttl_hours: 12, registration_enabled: true, seo_indexing_enabled: false },
            None,
        )
            .await
            .unwrap();
        state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "victim", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);
        let attempt = |host: &'static str, i: u8| {
            let app = app.clone();
            async move {
                let mut request = login_request_on(host, "victim", "wrong");
                // A fresh IP each time, so only the per-username budget is under test.
                request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, i], 51234))));
                app.oneshot(request).await.unwrap().status()
            }
        };

        for i in 1..=artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS as u8 {
            assert_eq!(attempt("artiferris.localhost", i).await, axum::http::StatusCode::UNAUTHORIZED);
        }
        let_the_clock_tick().await;

        assert_eq!(attempt("lenient.artiferris.localhost", 100).await, axum::http::StatusCode::TOO_MANY_REQUESTS, "a 1 second window from another tenant must not erase the lock");
        assert_eq!(attempt("artiferris.localhost", 101).await, axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_lenient_organization_gets_no_more_guesses_than_the_default_limit(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        organization_with_login_limit(&state, "lenient", 1000).await;
        state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "victim", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let mut unauthorized = 0;
        for i in 1..=30u8 {
            let mut request = login_request_on("lenient.artiferris.localhost", "victim", "wrong");
            request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, i], 51234))));
            if app.clone().oneshot(request).await.unwrap().status() == axum::http::StatusCode::UNAUTHORIZED {
                unauthorized += 1;
            }
        }

        assert_eq!(unauthorized, artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_unrecognized_username_is_throttled_using_the_default_limit_of_the_public_organization(pool: sqlx::PgPool) {
        // Also seed an org with a much lower limit — a wrong fallback resolving to it would throttle far earlier and fail this test.
        let state = AppState::build(pool, &test_config());
        organization_with_login_limit(&state, "acme", 1).await;
        let app = build_router(state);

        for _ in 0..artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS {
            let response = app.clone().oneshot(login_request("no-such-user", "wrong")).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED, "should not be throttled before the limit is reached");
        }

        let response = app.oneshot(login_request("no-such-user", "wrong")).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    // Can't hit /api/auth/login and read `token` off the response — MFA is mandatory, so that
    // always returns an `mfa_token`. Drives the same mandatory-TOTP-setup flow as the test
    // above to reach a real, HTTP-issued session token via `setup_mfa_totp_confirm`.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_login_token_carries_the_issuing_users_organizations_session_ttl(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        state.create_user.execute(acme_id, "acme-user", "sup3r-s3cret!", false).await.unwrap();
        state.update_system_settings.execute(acme_id, artiferris_domain::system_settings::SystemSettings { max_login_attempts: 10, login_attempt_window_seconds: 300, session_ttl_hours: 1, registration_enabled: true, seo_indexing_enabled: false }, None).await.unwrap();
        let app = build_router(state.clone());

        let json = login_response(app.clone(), "acme-user", "sup3r-s3cret!").await;
        assert_eq!(json["mfa_setup_required"], true);
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let enroll_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/enroll")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(enroll_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let secret = json["secret"].as_str().unwrap();
        let code = artiferris_application::use_cases::mfa::generate_current_totp_code(secret);

        let confirm_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/setup/totp/confirm")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}","code":"{code}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(confirm_response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(confirm_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = json["token"].as_str().unwrap();

        // Decode the JWT payload without verifying — just checking the claimed expiry is close to 1 hour, not the SystemSettings::defaults() 12 hours.
        let payload_b64 = token.split('.').nth(1).unwrap();
        let payload_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD_NO_PAD, payload_b64).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();
        let exp = payload["exp"].as_i64().unwrap();
        let now = chrono::Utc::now().timestamp();
        let ttl_seconds = exp - now;
        assert!(ttl_seconds > 0 && ttl_seconds <= 3600, "expected a ~1 hour TTL from acme's own session_ttl_hours=1, got {ttl_seconds} seconds");
    }

    const PUBLIC_ORG: &str = "00000000-0000-0000-0000-000000000001";

    async fn status_of(app: &Router, request: Request<Body>) -> axum::http::StatusCode {
        app.clone().oneshot(request).await.unwrap().status()
    }

    fn get_me(token: &str) -> Request<Body> {
        Request::builder().uri("/api/me").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap()
    }

    fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder().method("POST").uri(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()
    }

    /// Past `iat`'s whole-second precision, or the ordering against a revocation is ambiguous.
    async fn let_the_clock_tick() {
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_burst_of_parallel_wrong_passwords_lets_only_the_attempt_limit_through(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state);

        let handles: Vec<_> = (0..40)
            .map(|_| {
                let app = app.clone();
                tokio::spawn(async move { status_of(&app, login_request("florian", "wrong")).await })
            })
            .collect();
        let statuses: Vec<_> = futures::future::join_all(handles).await.into_iter().map(|r| r.unwrap()).collect();

        let reached_the_check = statuses
            .iter()
            .filter(|s| **s == axum::http::StatusCode::UNAUTHORIZED || **s == axum::http::StatusCode::SERVICE_UNAVAILABLE)
            .count();
        let max_attempts = artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS;
        assert_eq!(reached_the_check, max_attempts, "only the limit's worth of guesses may reach the password check: {statuses:?}");
        assert_eq!(statuses.iter().filter(|s| **s == axum::http::StatusCode::TOO_MANY_REQUESTS).count(), 40 - max_attempts);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_burst_of_parallel_wrong_mfa_codes_lets_only_the_attempt_limit_through(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
        let app = build_router(state);

        let handles: Vec<_> = (0..40)
            .map(|_| {
                let app = app.clone();
                let request = json_post("/api/auth/mfa/verify", serde_json::json!({ "mfa_token": mfa_token, "code": "000000" }));
                tokio::spawn(async move { status_of(&app, request).await })
            })
            .collect();
        let statuses: Vec<_> = futures::future::join_all(handles).await.into_iter().map(|r| r.unwrap()).collect();

        assert_eq!(statuses.iter().filter(|s| **s == axum::http::StatusCode::UNAUTHORIZED).count(), artiferris_application::login_throttle::MAX_LOGIN_ATTEMPTS, "{statuses:?}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_very_long_username_is_cut_down_in_the_audit_log(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state.clone());

        let response = app.oneshot(login_request(&"a".repeat(50_000), "wrong")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        let usernames = login_failure_usernames(&state).await;
        assert_eq!(usernames.len(), 1);
        assert_eq!(usernames[0].chars().count(), artiferris_domain::audit::MAX_RECORDED_USERNAME_CHARS);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_oversized_login_body_is_refused(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(login_request(&"a".repeat(100 * 1024), "wrong")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_password_longer_than_any_real_one_is_refused_as_wrong(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let long_password = "p".repeat(4000);
        state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", &long_password, false).await.unwrap();
        let app = build_router(state);

        let response = app.oneshot(login_request("florian", &long_password)).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_mfa_token_minted_before_a_password_change_stops_working(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
        let_the_clock_tick().await;
        state.users.update_password(user_id, "new-hash".to_string(), None).await.unwrap();
        let credential = state.totp_credentials.get(user_id).await.unwrap().unwrap();
        let code = artiferris_application::use_cases::mfa::generate_totp_code_after_step(&credential.secret, credential.last_used_step.unwrap());
        let app = build_router(state);

        for uri in ["/api/auth/mfa/verify", "/api/auth/mfa/setup/totp/enroll", "/api/auth/mfa/passkey/start"] {
            let response = app.clone().oneshot(json_post(uri, serde_json::json!({ "mfa_token": mfa_token, "code": code }))).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED, "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_mfa_token_completes_a_login_only_once(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let mfa_token = state.mfa_pending_token_issuer.issue(user_id, chrono::Duration::minutes(5)).unwrap();
        let app = build_router(state.clone());
        let next_code = || async {
            let credential = state.totp_credentials.get(user_id).await.unwrap().unwrap();
            artiferris_application::use_cases::mfa::generate_totp_code_after_step(&credential.secret, credential.last_used_step.unwrap())
        };

        let code = next_code().await;
        let first = app.clone().oneshot(json_post("/api/auth/mfa/verify", serde_json::json!({ "mfa_token": mfa_token, "code": code }))).await.unwrap();
        assert_eq!(first.status(), axum::http::StatusCode::OK);

        let code = next_code().await;
        let replay = app.oneshot(json_post("/api/auth/mfa/verify", serde_json::json!({ "mfa_token": mfa_token, "code": code }))).await.unwrap();
        assert_eq!(replay.status(), axum::http::StatusCode::UNAUTHORIZED, "a completed login's mfa token must not be usable again");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn signing_out_everywhere_ends_every_earlier_session_but_not_a_later_login(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let laptop = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let phone = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());
        let_the_clock_tick().await;

        let response = app.clone().oneshot(Request::builder().method("POST").uri("/api/auth/logout-all").header("authorization", format!("Bearer {laptop}")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        assert_eq!(status_of(&app, get_me(&laptop)).await, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(status_of(&app, get_me(&phone)).await, axum::http::StatusCode::UNAUTHORIZED);
        let_the_clock_tick().await;
        let fresh = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        assert_eq!(status_of(&app, get_me(&fresh)).await, axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn signing_out_everywhere_requires_a_session(pool: sqlx::PgPool) {
        let app = build_router(AppState::build(pool, &test_config()));

        let response = app.oneshot(Request::builder().method("POST").uri("/api/auth/logout-all").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn demoting_a_super_admin_ends_their_sessions_but_promoting_does_not(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let public_org = Uuid::parse_str(PUBLIC_ORG).unwrap();
        state.create_user.execute(public_org, "root", "sup3r-s3cret!", true).await.unwrap();
        let demoted_id = state.create_user.execute(public_org, "demoted", "sup3r-s3cret!", true).await.unwrap();
        let promoted_id = state.create_user.execute(public_org, "promoted", "sup3r-s3cret!", false).await.unwrap();
        let demoted = state.authenticate_user.execute("demoted", "sup3r-s3cret!").await.unwrap();
        let promoted = state.authenticate_user.execute("promoted", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());
        let_the_clock_tick().await;

        state.set_super_admin.execute(demoted_id, false, demoted_id).await.unwrap();
        state.set_super_admin.execute(promoted_id, true, demoted_id).await.unwrap();

        assert_eq!(status_of(&app, get_me(&demoted)).await, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(status_of(&app, get_me(&promoted)).await, axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn demoting_an_organization_admin_ends_their_sessions(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        state.set_organization_admin.execute(admin_id, true, None).await.unwrap();
        let token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());
        let_the_clock_tick().await;

        state.set_organization_admin.execute(admin_id, false, None).await.unwrap();

        assert_eq!(status_of(&app, get_me(&token)).await, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_unverified_self_registered_account_is_not_hijacked_by_a_later_sso_login(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state.clone());
        let registered = app.oneshot(register_request("squatter", "victim@corp.example", "sup3r-s3cret!")).await.unwrap();
        assert_eq!(registered.status(), axum::http::StatusCode::OK);
        let squatter = state.users.find_by_username(&artiferris_domain::user::Username::parse("squatter").unwrap()).await.unwrap().unwrap();
        let identity = artiferris_domain::sso::ExternalIdentity { email: "victim@corp.example".to_string(), display_name: None };

        let session = state.provision_sso_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), &identity).await.unwrap();

        let session_owner = state.token_issuer.verify(&session).unwrap().user_id;
        assert_ne!(session_owner, squatter.id, "the victim's SSO login must not land in the account the attacker registered");
        let again = state.provision_sso_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), &identity).await.unwrap();
        assert_eq!(state.token_issuer.verify(&again).unwrap().user_id, session_owner, "later SSO logins reach the victim's own account");
    }

    struct FakeLdapAuth;

    #[async_trait::async_trait]
    impl artiferris_domain::sso::LdapAuthPort for FakeLdapAuth {
        async fn authenticate(
            &self,
            _config: &artiferris_domain::sso::LdapConfig,
            _username: &str,
            _password: &str,
        ) -> Result<artiferris_domain::sso::ExternalIdentity, artiferris_domain::error::DomainError> {
            Ok(artiferris_domain::sso::ExternalIdentity { email: "florian@corp.example".to_string(), display_name: None })
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_password_login_is_recorded_once_the_second_factor_completes_it(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let app = build_router(state.clone());

        let first_step = app.clone().oneshot(login_request("florian", "sup3r-s3cret!")).await.unwrap();
        assert_eq!(first_step.status(), axum::http::StatusCode::OK);
        assert!(security_events_of_type(&state, "LoginSucceeded").await.is_empty(), "the password alone is not a login");

        login_and_get_token(&app, &state, user_id, "sup3r-s3cret!").await;

        let recorded = security_events_of_type(&state, "LoginSucceeded").await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["user_id"], user_id.to_string());
        assert_eq!(recorded[0]["organization_id"], PUBLIC_ORG);
        assert_eq!(recorded[0]["method"], "password");
        assert_eq!(recorded[0]["second_factor"], "totp");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_wrong_second_factor_is_not_recorded_as_a_login(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        enroll_and_confirm_totp(&state, user_id, "sup3r-s3cret!").await;
        let app = build_router(state.clone());
        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        let mfa_token = json["mfa_token"].as_str().unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"mfa_token":"{mfa_token}","code":"000000"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert!(security_events_of_type(&state, "LoginSucceeded").await.is_empty());
        assert_eq!(security_events_of_type(&state, "MfaVerificationFailed").await.len(), 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn finishing_mandatory_totp_setup_records_the_factor_and_the_login(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let app = build_router(state.clone());
        let json = login_response(app.clone(), "florian", "sup3r-s3cret!").await;
        let mfa_token = json["mfa_token"].as_str().unwrap();
        let post = |uri: &str, body: String| Request::builder().method("POST").uri(uri.to_string()).header("content-type", "application/json").body(Body::from(body)).unwrap();
        let enrolled = app.clone().oneshot(post("/api/auth/mfa/setup/totp/enroll", format!(r#"{{"mfa_token":"{mfa_token}"}}"#))).await.unwrap();
        let body = to_bytes(enrolled.into_body(), usize::MAX).await.unwrap();
        let secret = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["secret"].as_str().unwrap().to_string();
        let code = artiferris_application::use_cases::mfa::generate_current_totp_code(&secret);

        let confirmed = app.oneshot(post("/api/auth/mfa/setup/totp/confirm", format!(r#"{{"mfa_token":"{mfa_token}","code":"{code}"}}"#))).await.unwrap();

        assert_eq!(confirmed.status(), axum::http::StatusCode::OK);
        let enabled = security_events_of_type(&state, "MfaEnabled").await;
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0]["method"], "totp");
        assert_eq!(enabled[0]["user_id"], user_id.to_string());
        let logins = security_events_of_type(&state, "LoginSucceeded").await;
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0]["second_factor"], "totp");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_directory_login_is_recorded_as_ldap_in_the_organization_it_came_in_on(pool: sqlx::PgPool) {
        let mut state = AppState::build(pool, &test_config());
        state.ldap_auth = std::sync::Arc::new(FakeLdapAuth);
        let public_org = state.organizations.find_public().await.unwrap();
        seed_ldap_config(&state, public_org.id).await;
        let app = build_router(state.clone());

        let response = app.oneshot(ldap_login_request("florian", "s3cret!")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let recorded = security_events_of_type(&state, "LoginSucceeded").await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["method"], "ldap");
        assert_eq!(recorded[0]["organization_id"], public_org.id.to_string());
        assert!(recorded[0]["second_factor"].is_null());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_oidc_login_is_recorded_and_a_rejected_callback_is_not(pool: sqlx::PgPool) {
        let state = state_with_fake_oidc(pool);
        let public_org = state.organizations.find_public().await.unwrap();
        seed_oidc_config(&state, public_org.id).await;
        let app = build_router(state.clone());
        let login = app.clone().oneshot(oidc_login_request("artiferris.localhost")).await.unwrap();
        let cookie = binding_cookie_value(&login);
        let state_token = state_token_from_login(&login);

        app.clone().oneshot(oidc_callback_request("artiferris.localhost", &state_token, None)).await.unwrap();
        assert!(security_events_of_type(&state, "LoginSucceeded").await.is_empty());

        app.oneshot(oidc_callback_request("artiferris.localhost", &state_token, Some(&cookie))).await.unwrap();

        let recorded = security_events_of_type(&state, "LoginSucceeded").await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["method"], "oidc");
        assert_eq!(recorded[0]["organization_id"], public_org.id.to_string());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_password_change_is_recorded_only_when_it_succeeds(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "old-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "old-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        let rejected = app.clone().oneshot(change_password_request(&token, "wrong-password", "new-s3cret!")).await.unwrap();
        assert_eq!(rejected.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(security_events_of_type(&state, "PasswordChanged").await.is_empty());

        let accepted = app.oneshot(change_password_request(&token, "old-s3cret!", "new-s3cret!")).await.unwrap();
        assert_eq!(accepted.status(), axum::http::StatusCode::NO_CONTENT);

        let recorded = security_events_of_type(&state, "PasswordChanged").await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["user_id"], user_id.to_string());
        let serialised = recorded[0].to_string();
        assert!(!serialised.contains("old-s3cret!") && !serialised.contains("new-s3cret!"));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn signing_out_everywhere_is_recorded(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "florian", "sup3r-s3cret!", false).await.unwrap();
        let token = state.authenticate_user.execute("florian", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        let response = app
            .oneshot(Request::builder().method("POST").uri("/api/auth/logout-all").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        let recorded = security_events_of_type(&state, "SessionsRevoked").await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["user_id"], user_id.to_string());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn activating_an_account_is_recorded_with_the_account_as_actor(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let user_id = state.create_user.execute(Uuid::parse_str(PUBLIC_ORG).unwrap(), "invitee", "placeholder-not-usable", false).await.unwrap();
        state
            .user_invitations
            .upsert(&artiferris_domain::invitation::UserInvitation {
                user_id,
                token_hash: artiferris_application::use_cases::invitation::hash_invitation_token("raw-test-token"),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
            }, None)
            .await
            .unwrap();
        let app = build_router(state.clone());

        let response = app.oneshot(activate_request("raw-test-token", "new-s3cret!")).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        let page = state
            .query_audit_log
            .execute(artiferris_domain::audit::AuditQueryFilter { aggregate_type: Some("Admin".to_string()), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].event_type, "UserActivated");
        assert_eq!(page.entries[0].actor_id, Some(user_id));
        assert_eq!(page.entries[0].organization_id, Some(Uuid::parse_str(PUBLIC_ORG).unwrap()));
        assert!(!page.entries[0].payload.to_string().contains("new-s3cret!"));
    }
}
