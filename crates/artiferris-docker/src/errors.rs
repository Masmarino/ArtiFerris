use axum::Json;
use axum::http::StatusCode;
use artiferris_application::error::ApplicationError;
use serde_json::json;

use crate::state::DockerState;

pub fn docker_error_response(error: ApplicationError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, code) = match &error {
        ApplicationError::DockerBlobNotFound => (StatusCode::NOT_FOUND, "BLOB_UNKNOWN"),
        ApplicationError::DockerManifestNotFound => (StatusCode::NOT_FOUND, "MANIFEST_UNKNOWN"),
        ApplicationError::DockerDigestMismatch { .. } => (StatusCode::BAD_REQUEST, "DIGEST_INVALID"),
        ApplicationError::InvalidDockerPayload(_) => (StatusCode::BAD_REQUEST, "MANIFEST_INVALID"),
        ApplicationError::DockerUploadSessionNotFound => (StatusCode::NOT_FOUND, "BLOB_UPLOAD_UNKNOWN"),
        ApplicationError::DockerChunkOffsetMismatch { .. } => (StatusCode::RANGE_NOT_SATISFIABLE, "BLOB_UPLOAD_INVALID"),
        ApplicationError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
        ApplicationError::StorageQuotaExceeded => (StatusCode::INSUFFICIENT_STORAGE, "DENIED"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "UNKNOWN"),
    };
    if status == StatusCode::INTERNAL_SERVER_ERROR {
        tracing::error!(error = %error, "docker route internal error");
        // Never echo a raw infrastructure error string back to the client.
        return (status, Json(json!({ "errors": [{ "code": code, "message": "internal server error" }] })));
    }
    (status, Json(json!({ "errors": [{ "code": code, "message": error.to_string() }] })))
}

pub fn docker_error(status: StatusCode, code: &str, message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(json!({ "errors": [{ "code": code, "message": message }] })))
}

/// `scope` is `None` for the unscoped `GET /v2/` ping. `host` is the request's `Host` header — untrusted, like `scope`, hence the escaping below.
pub fn www_authenticate_challenge(state: &DockerState, host: &str, scope: Option<&str>) -> String {
    let realm = escape_header_quoted_string(&state.token_realm(host));
    match scope {
        Some(scope) => {
            let escaped_scope = escape_header_quoted_string(scope);
            format!(r#"Bearer realm="{realm}",service="{}",scope="{escaped_scope}""#, state.token_service)
        }
        None => format!(r#"Bearer realm="{realm}",service="{}""#, state.token_service),
    }
}

/// Escapes untrusted content before it's interpolated into a quoted header value.
fn escape_header_quoted_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route_test_support::test_state;
    use sqlx::PgPool;

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_challenge_realm_reflects_the_given_host(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.token_realm_override = None;
        state.public_scheme = "https".to_string();

        let challenge = www_authenticate_challenge(&state, "acme.artiferris.pro", None);

        assert_eq!(challenge, r#"Bearer realm="https://acme.artiferris.pro/v2/token",service="artiferris""#);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_quote_or_backslash_in_the_host_is_escaped_instead_of_breaking_the_quoted_string(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.token_realm_override = None;
        state.public_scheme = "https".to_string();

        let challenge = www_authenticate_challenge(&state, r#"evil"\host"#, None);

        assert_eq!(challenge, r#"Bearer realm="https://evil\"\\host/v2/token",service="artiferris""#);
    }
}
