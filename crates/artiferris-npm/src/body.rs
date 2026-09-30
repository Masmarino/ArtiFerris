//! Request bodies are read here, after the handler has authorized the caller, never by an extractor beforehand.

use axum::Json;
use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::extract::rejection::ExtensionRejection;
use axum::http::{HeaderMap, StatusCode, header};
use artiferris_application::body_budget::{BodyReservation, Refusal};
use artiferris_application::body_read::{BodyReadError, BodyTimeouts, read_limited};
use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde_json::json;
use uuid::Uuid;

use crate::state::NpmState;

type Rejection = (StatusCode, Json<serde_json::Value>);

/// Who a body comes from, for the limit on bodies one client or user may have in flight.
pub struct BodyCaller {
    pub user_id: Uuid,
    pub client: String,
}

impl BodyCaller {
    pub fn new(state: &NpmState, user_id: Uuid, headers: &HeaderMap, connect_info: &Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> Self {
        let forwarded: Vec<&str> = headers.get_all("x-forwarded-for").iter().filter_map(|value| value.to_str().ok()).collect();
        let client = state.guard.client_bucket(connect_info.as_ref().ok().map(|ConnectInfo(addr)| addr.ip()), &forwarded);
        Self { user_id, client }
    }

    fn keys(&self) -> [String; 2] {
        [format!("client:{}", self.client), format!("user:{}", self.user_id)]
    }
}

/// Reads and parses a JSON body of at most `limit` bytes. The body budget is charged as the body arrives, up to `memory_factor`
/// times what it declared in `Content-Length` (the limit, when none is declared or the body is compressed), since parsing keeps
/// more than one copy alive. A body the budget couldn't cover is turned away before it is read.
pub async fn read_json<T: DeserializeOwned>(
    state: &NpmState,
    headers: &HeaderMap,
    caller: &BodyCaller,
    body: Body,
    limit: usize,
    memory_factor: usize,
) -> Result<(T, BodyReservation), Rejection> {
    let compressed = headers.contains_key(header::CONTENT_ENCODING);
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<usize>().ok()).filter(|_| !compressed);
    let expected = declared.unwrap_or(limit);
    if expected > limit {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, Json(json!({ "error": "request body is too large" }))));
    }
    let mut reservation = state.guard.body_budget.admit(&caller.keys(), expected.saturating_mul(memory_factor)).map_err(|refusal| match refusal {
        Refusal::Busy => busy(),
        Refusal::TooManyFromOneClient => (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "too many uploads in flight from this client, retry shortly" }))),
    })?;
    let bytes = read_body(body, limit, state.guard.body_timeouts, declared, |received| reservation.grow_to(received.saturating_mul(memory_factor))).await?;
    let parsed = serde_json::from_slice(&bytes).map_err(|_| (StatusCode::BAD_REQUEST, Json(json!({ "error": "request body is not valid JSON of the expected shape" }))))?;
    Ok((parsed, reservation))
}

/// Reads a small text body of at most `limit` bytes.
pub async fn read_text(state: &NpmState, body: Body, limit: usize) -> Result<String, Rejection> {
    let bytes = read_body(body, limit, state.guard.body_timeouts, None, |_| true).await?;
    String::from_utf8(bytes.to_vec()).map_err(|_| (StatusCode::BAD_REQUEST, Json(json!({ "error": "request body is not valid UTF-8" }))))
}

fn busy() -> Rejection {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "the registry is busy, retry shortly" })))
}

async fn read_body(body: Body, limit: usize, timeouts: BodyTimeouts, declared: Option<usize>, charge: impl FnMut(usize) -> bool) -> Result<Bytes, Rejection> {
    read_limited(body.into_data_stream(), limit, timeouts, declared, charge).await.map_err(|error| match error {
        BodyReadError::TimedOut => (StatusCode::REQUEST_TIMEOUT, Json(json!({ "error": "the request body took too long to arrive" }))),
        BodyReadError::Busy => busy(),
        BodyReadError::TooLarge | BodyReadError::Unreadable => (StatusCode::PAYLOAD_TOO_LARGE, Json(json!({ "error": "request body is too large or could not be read" }))),
    })
}
