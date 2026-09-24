//! Response headers that depend on the route: what a browser or a shared cache may keep, and who may embed the response.

use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

pub const PERMISSIONS_POLICY: &str = "accelerometer=(), autoplay=(), camera=(), display-capture=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), usb=(), xr-spatial-tracking=()";

/// The logo and favicon are embedded by other sites through link previews; the registries are used by non-browser clients and
/// proxies that never see this header.
fn allows_cross_origin_embedding(path: &str) -> bool {
    path.starts_with("/npm/") || path.starts_with("/v2/") || path.starts_with("/api/branding/")
}

/// Every `/api` response depends on who asks (a repository, its role, its projection), so unless the handler chose its own
/// `Cache-Control` it is `no-store`, and `Vary: Authorization` keeps a cache that stores it anyway from mixing callers.
pub async fn response_policy(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if !allows_cross_origin_embedding(&path) {
        headers.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    }
    if path.starts_with("/api/") {
        if !headers.contains_key(header::CACHE_CONTROL) {
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        }
        let varies_on_authorization = headers.get_all(header::VARY).iter().any(|v| v.to_str().is_ok_and(|v| v.split(',').any(|name| matches!(name.trim().to_ascii_lowercase().as_str(), "authorization" | "*"))));
        if !varies_on_authorization {
            headers.append(header::VARY, HeaderValue::from_static("Authorization"));
        }
    }
    response
}
