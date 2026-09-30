use axum::Json;
use axum::http::StatusCode;
use artiferris_application::error::ApplicationError;
use serde_json::json;

pub fn bad_request(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message })))
}

pub fn npm_error_response(error: ApplicationError) -> (StatusCode, Json<serde_json::Value>) {
    let status = match &error {
        ApplicationError::PackageVersionExists => StatusCode::CONFLICT,
        ApplicationError::NpmPackageNotFound | ApplicationError::NpmVersionNotFound => StatusCode::NOT_FOUND,
        ApplicationError::InvalidNpmPayload(_) => StatusCode::BAD_REQUEST,
        ApplicationError::StorageQuotaExceeded => StatusCode::INSUFFICIENT_STORAGE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    if status == StatusCode::INTERNAL_SERVER_ERROR {
        tracing::error!(error = %error, "npm route internal error");
        // Never echo a raw infrastructure error to the client.
        return (status, Json(json!({ "error": "internal server error" })));
    }
    (status, Json(json!({ "error": error.to_string() })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::error::DomainError;

    #[test]
    fn an_internal_error_never_echoes_its_raw_message_to_the_client() {
        let error = ApplicationError::Domain(DomainError::Infrastructure("postgres://user:hunter2@db.internal:5432/prod".to_string()));
        let (status, Json(body)) = npm_error_response(error);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let body_str = body.to_string();
        assert!(!body_str.contains("hunter2"), "must never leak raw infrastructure error text: {body_str}");
        assert_eq!(body["error"], "internal server error");
    }
}
