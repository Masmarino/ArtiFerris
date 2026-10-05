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
    /// The language the user chose for the interface; `null` until they have (or the app has recorded one).
    pub language: Option<String>,
    /// The address ArtiFerris writes to: the account's email once verified (invitation, identity provider); `null`
    /// otherwise.
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetLanguageRequest {
    pub language: String,
}

/// The session a password change hands back: the change ended every session, the caller's included, so it would
/// otherwise be signed out.
#[derive(Debug, Serialize)]
pub struct ChangePasswordResponse {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct ActivateAccountRequest {
    pub token: String,
    pub username: String,
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

/// What became of a password-reset mail. When it could not go out, `email_error` says why (`email_not_configured`,
/// `email_send_failed`, `email_no_address`) and `reset_url` carries the link for the administrator to pass on, once.
#[derive(Serialize, Debug)]
pub struct PasswordResetMailResponse {
    pub email_sent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_url: Option<String>,
}

impl From<Option<artiferris_application::use_cases::password_reset::UndeliveredReset>> for PasswordResetMailResponse {
    fn from(undelivered: Option<artiferris_application::use_cases::password_reset::UndeliveredReset>) -> Self {
        match undelivered {
            None => Self { email_sent: true, email_error: None, reset_url: None },
            Some(undelivered) => Self { email_sent: false, email_error: Some(undelivered.reason.code()), reset_url: Some(undelivered.reset_url) },
        }
    }
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
    /// For a person, in English; clients should not match on its wording (it can be reworded) but on `code`.
    pub error: String,
    /// A stable name for the kind of error (see `DomainError::code` / `ApplicationError::code`); absent on the errors that have none yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
}

impl ErrorResponse {
    pub fn message(error: impl Into<String>) -> Self {
        Self { error: error.into(), code: None }
    }

    pub fn coded(code: &'static str, error: impl Into<String>) -> Self {
        Self { error: error.into(), code: Some(code) }
    }
}

/// Seconds between two warnings about a service that is busy; a flood of requests would otherwise be a flood of log lines.
const BUSY_LOG_INTERVAL_SECONDS: u64 = 10;
static LAST_BUSY_LOG: AtomicU64 = AtomicU64::new(0);

/// True when at least `BUSY_LOG_INTERVAL_SECONDS` have passed since `last` was stored; claims the slot for `now` if so.
fn busy_log_due(last: &AtomicU64, now: u64) -> bool {
    let before = last.load(Ordering::Relaxed);
    now.saturating_sub(before) >= BUSY_LOG_INTERVAL_SECONDS && last.compare_exchange(before, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
}

/// What became of an invitation's mail, sent with the invited account or alone after a resend. When the mail could not
/// go out, `email_error` says why (`email_not_configured`, `email_send_failed`) and `activation_url` carries the link for
/// the administrator to pass on: the only copy, handed over this once.
#[derive(Serialize, Debug, Default)]
pub struct InvitationMailResponse {
    pub email_sent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_url: Option<String>,
}

impl From<Option<artiferris_application::use_cases::invitation::UndeliveredInvitation>> for InvitationMailResponse {
    fn from(undelivered: Option<artiferris_application::use_cases::invitation::UndeliveredInvitation>) -> Self {
        match undelivered {
            None => Self { email_sent: true, email_error: None, activation_url: None },
            Some(undelivered) => Self { email_sent: false, email_error: Some(undelivered.reason.code()), activation_url: Some(undelivered.activation_url) },
        }
    }
}

/// Infrastructure-shaped errors are logged server-side and answered with a flat `500`, never echoing raw backend text to the client.
pub fn application_error_response(context: &str, error: ApplicationError) -> (StatusCode, Json<ErrorResponse>) {
    let code = error.code();
    match error {
        ApplicationError::EventStore(_) | ApplicationError::Storage(_) | ApplicationError::Domain(DomainError::Infrastructure(_)) => {
            tracing::error!("{context}: {error}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::coded(code, "internal error".to_string())))
        }
        ApplicationError::Domain(DomainError::Busy(_)) => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
            if busy_log_due(&LAST_BUSY_LOG, now) {
                tracing::warn!("{context}: {error} (further ones within {BUSY_LOG_INTERVAL_SECONDS}s are not logged)");
            }
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::coded(code, "busy, try again shortly".to_string())))
        }
        // The detail names key fingerprints: for the log, not for the client.
        ApplicationError::Domain(DomainError::SecretUnreadable(_)) => {
            tracing::error!("{context}: {error}");
            (StatusCode::CONFLICT, Json(ErrorResponse::coded(code, "a secret stored on the server cannot be read, ask an administrator to check the server's encryption keys".to_string())))
        }
        ApplicationError::LastSuperAdmin => (StatusCode::CONFLICT, Json(ErrorResponse::coded(code, error.to_string()))),
        ApplicationError::PersonalOrganizationAlreadyExists => (StatusCode::CONFLICT, Json(ErrorResponse::coded(code, error.to_string()))),
        ApplicationError::DependencyScanBusy => (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::coded(code, error.to_string()))),
        ApplicationError::DependencyScanRateLimited => (StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse::coded(code, error.to_string()))),
        // A server misconfiguration, not the caller's fault.
        ApplicationError::PasskeysUnavailable => (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::coded(code, error.to_string()))),
        // Same "don't confirm existence across a trust boundary" convention as authz::require_same_organization's 404 — here the boundary is per-user, not per-organization.
        ApplicationError::ApiTokenNotFound => (StatusCode::NOT_FOUND, Json(ErrorResponse::coded(code, error.to_string()))),
        error => (StatusCode::BAD_REQUEST, Json(ErrorResponse::coded(code, error.to_string()))),
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

    #[test]
    fn every_mapped_error_carries_the_code_of_its_kind() {
        let (_, body) = application_error_response("test", ApplicationError::UsernameTaken);
        assert_eq!(body.0.code, Some("username_taken"));

        let (_, body) = application_error_response("test", ApplicationError::Domain(DomainError::PasswordTooShort));
        assert_eq!(body.0.code, Some("password_too_short"));
        assert_eq!(body.0.error, "password must be at least 8 characters");

        let (status, body) = application_error_response("test", ApplicationError::LastSuperAdmin);
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.0.code, Some("last_super_admin"));
    }

    #[test]
    fn an_internal_failure_keeps_its_flat_message_and_leaks_nothing_in_the_code() {
        let (status, body) = application_error_response("test", ApplicationError::Domain(DomainError::Infrastructure("password=hunter2".to_string())));

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.0.error, "internal error");
        assert_eq!(body.0.code, Some("infrastructure_failure"));
    }

    #[test]
    fn a_revoked_api_token_and_wrong_credentials_are_indistinguishable() {
        let (_, wrong) = application_error_response("test", ApplicationError::InvalidCredentials);
        let (_, revoked) = application_error_response("test", ApplicationError::InactiveApiToken);

        assert_eq!(wrong.0.code, revoked.0.code);
        assert_eq!(wrong.0.error, revoked.0.error);
    }

    #[test]
    fn codes_are_snake_case_and_a_response_without_one_omits_the_field() {
        for code in [DomainError::EmailTaken.code(), DomainError::Busy(String::new()).code(), ApplicationError::DockerChunkOffsetMismatch { expected: 0, got: 1 }.code()] {
            assert!(!code.is_empty() && code.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{code}");
        }
        let json = serde_json::to_value(ErrorResponse::message("plain")).unwrap();
        assert_eq!(json, serde_json::json!({ "error": "plain" }));
        let json = serde_json::to_value(ErrorResponse::coded("busy", "plain")).unwrap();
        assert_eq!(json, serde_json::json!({ "error": "plain", "code": "busy" }));
    }
}
