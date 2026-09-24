use std::sync::atomic::{AtomicU64, Ordering};

use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use artiferris_application::error::ApplicationError;
use artiferris_domain::error::DomainError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LdapLoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SsoProviderType {
    Ldap,
    Oidc,
}

#[derive(Debug, Serialize)]
pub struct SsoConfigResponse {
    /// `None` means this organization uses local accounts — the frontend's login page falls back to `/api/auth/login`.
    #[serde(rename = "type")]
    pub provider_type: Option<SsoProviderType>,
    /// Whether the login page should offer "Créer un compte" — true only when this org is public *and* the super-admin hasn't turned registration off.
    pub registration_enabled: bool,
}

/// Exactly one of `token`/`mfa_token` is set. `mfa_token` is exchanged for a real `token` at `/api/auth/mfa/verify` or `/api/auth/mfa/setup/*`.
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: Option<String>,
    pub mfa_token: Option<String>,
    #[serde(default)]
    pub mfa_setup_required: bool,
    /// Which second factor(s) the account actually has enrolled — lets the login page's verify step show only the relevant option, not a meaningless default TOTP field.
    #[serde(default)]
    pub mfa_has_totp: bool,
    #[serde(default)]
    pub mfa_has_passkey: bool,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub id: Uuid,
    pub username: String,
    pub is_super_admin: bool,
    pub is_organization_admin: bool,
    pub organization_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct ActivateAccountRequest {
    pub token: String,
    pub new_password: String,
}

/// Exactly one of `code`/`backup_code` should be set; `code` is tried first if both are present.
#[derive(Debug, Deserialize)]
pub struct MfaVerifyRequest {
    pub mfa_token: String,
    pub code: Option<String>,
    pub backup_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Seconds between two warnings about a service that is busy; a flood of requests would otherwise be a flood of log lines.
const BUSY_LOG_INTERVAL_SECONDS: u64 = 10;
static LAST_BUSY_LOG: AtomicU64 = AtomicU64::new(0);

/// True when at least `BUSY_LOG_INTERVAL_SECONDS` have passed since `last` was stored; claims the slot for `now` if so.
fn busy_log_due(last: &AtomicU64, now: u64) -> bool {
    let before = last.load(Ordering::Relaxed);
    now.saturating_sub(before) >= BUSY_LOG_INTERVAL_SECONDS && last.compare_exchange(before, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
}

/// Infrastructure-shaped errors are logged server-side and answered with a flat `500`, never echoing raw backend text to the client.
pub fn application_error_response(context: &str, error: ApplicationError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        ApplicationError::EventStore(_) | ApplicationError::Storage(_) | ApplicationError::Domain(DomainError::Infrastructure(_)) => {
            tracing::error!("{context}: {error}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() }))
        }
        ApplicationError::Domain(DomainError::Busy(_)) => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
            if busy_log_due(&LAST_BUSY_LOG, now) {
                tracing::warn!("{context}: {error} (further ones within {BUSY_LOG_INTERVAL_SECONDS}s are not logged)");
            }
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse { error: "busy, try again shortly".to_string() }))
        }
        // The detail names key fingerprints: for the log, not for the client.
        ApplicationError::Domain(DomainError::SecretUnreadable(_)) => {
            tracing::error!("{context}: {error}");
            (StatusCode::CONFLICT, Json(ErrorResponse { error: "a secret stored on the server cannot be read, ask an administrator to check the server's encryption keys".to_string() }))
        }
        ApplicationError::LastSuperAdmin => (StatusCode::CONFLICT, Json(ErrorResponse { error: error.to_string() })),
        ApplicationError::PersonalOrganizationAlreadyExists => (StatusCode::CONFLICT, Json(ErrorResponse { error: error.to_string() })),
        ApplicationError::DependencyScanBusy => (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse { error: error.to_string() })),
        ApplicationError::DependencyScanRateLimited => (StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse { error: error.to_string() })),
        // A server misconfiguration, not the caller's fault.
        ApplicationError::PasskeysUnavailable => (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse { error: error.to_string() })),
        // Same "don't confirm existence across a trust boundary" convention as authz::require_same_organization's 404 — here the boundary is per-user, not per-organization.
        ApplicationError::ApiTokenNotFound => (StatusCode::NOT_FOUND, Json(ErrorResponse { error: error.to_string() })),
        error => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: error.to_string() })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_service_is_a_503_and_never_a_500() {
        let (status, body) = application_error_response("test", ApplicationError::Domain(DomainError::Busy("the public catalog is busy".to_string())));

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0.error, "busy, try again shortly");
        let (status, _) = application_error_response("test", ApplicationError::Domain(DomainError::Infrastructure("boom".to_string())));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn an_unreadable_secret_is_a_409_that_never_repeats_the_key_fingerprints() {
        let detail = "sealed with another SECRETS_ENCRYPTION_KEY (key id a1b2c3d4, this server's is 0badf00d)";

        let (status, body) = application_error_response("test", ApplicationError::Domain(DomainError::SecretUnreadable(detail.to_string())));

        assert_eq!(status, StatusCode::CONFLICT);
        assert!(!body.0.error.contains("a1b2c3d4") && !body.0.error.contains("0badf00d") && !body.0.error.contains("key id"), "got: {}", body.0.error);
    }

    #[test]
    fn busy_warnings_are_spaced_out() {
        let last = AtomicU64::new(0);

        assert!(busy_log_due(&last, 1_000));
        assert!(!busy_log_due(&last, 1_000 + BUSY_LOG_INTERVAL_SECONDS - 1));
        assert!(busy_log_due(&last, 1_000 + BUSY_LOG_INTERVAL_SECONDS));
    }
}
