//! Anonymous reads of the registry share a per-client budget, so one address cannot drain the server's bandwidth.
//! A request that carries a token is never counted: the token names who to hold to account.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::state::NpmState;

pub async fn limit_anonymous_reads(State(state): State<NpmState>, request: Request, next: Next) -> Response {
    if matches!(*request.method(), Method::GET | Method::HEAD) && !request.headers().contains_key(header::AUTHORIZATION) {
        let forwarded: Vec<&str> = request.headers().get_all("x-forwarded-for").iter().filter_map(|value| value.to_str().ok()).collect();
        let direct = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|ConnectInfo(address)| address.ip());
        let client = state.guard.client_bucket(direct, &forwarded);
        if !state.guard.allow_anonymous_read("npm", &client) {
            let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "too many requests, try again shortly" }))).into_response();
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(artiferris_application::rate_limiter::RETRY_AFTER_SECONDS));
            return response;
        }
    }
    next.run(request).await
}
