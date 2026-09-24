//! Time limits on request bodies for the JSON API and the app fallback. The npm and Docker routes stream large bodies and
//! apply their own limits.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use tower_http::timeout::{DeadlineBody, TimeoutBody};

pub use artiferris_application::body_read::BodyTimeouts;

/// Fails the body read once no chunk arrives within `timeouts.idle` or the whole body takes longer than `timeouts.total`,
/// and answers 408 when that is why the request failed.
pub async fn limit_body_time(State(timeouts): State<BodyTimeouts>, request: Request, next: Next) -> Response {
    let timed_out = Arc::new(AtomicBool::new(false));
    let (parts, body) = request.into_parts();
    let flag = timed_out.clone();
    let body = DeadlineBody::new(timeouts.total, TimeoutBody::new(timeouts.idle, body)).map_err(move |e| {
        flag.store(true, Ordering::Relaxed);
        e
    });
    let response = next.run(Request::from_parts(parts, Body::new(body))).await;
    if timed_out.load(Ordering::Relaxed) {
        return StatusCode::REQUEST_TIMEOUT.into_response();
    }
    response
}
