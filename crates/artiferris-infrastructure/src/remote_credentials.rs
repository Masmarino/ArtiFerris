//! Stored proxy credentials go to the configured remote and nowhere else.

use artiferris_domain::error::DomainError;
use reqwest::Url;

/// A username authenticates as HTTP Basic; a password alone as a Bearer token.
pub fn apply_credentials(request: reqwest::RequestBuilder, username: Option<&str>, password: Option<&str>) -> reqwest::RequestBuilder {
    match (username, password) {
        (Some(username), password) => request.basic_auth(username, password),
        (None, Some(token)) => request.bearer_auth(token),
        (None, None) => request,
    }
}

/// Same scheme, host and port as the remote, and https (plain http only for loopback, which `ssrf_guard` keeps out of production).
pub fn may_receive_credentials(remote_base: &str, target: &str) -> bool {
    let (Ok(remote), Ok(target)) = (Url::parse(remote_base), Url::parse(target)) else { return false };
    let same_origin = remote.scheme() == target.scheme() && remote.host_str() == target.host_str() && remote.port_or_known_default() == target.port_or_known_default();
    same_origin && (target.scheme() == "https" || is_loopback(&target))
}

/// `apply_credentials`, but only when `target` may receive them; otherwise the request goes out anonymously.
pub fn apply_credentials_if_allowed(request: reqwest::RequestBuilder, remote_base: &str, target: &str, username: Option<&str>, password: Option<&str>) -> reqwest::RequestBuilder {
    if may_receive_credentials(remote_base, target) { apply_credentials(request, username, password) } else { request }
}

/// Refuses to fetch with credentials from a remote that would carry them over plain http.
pub fn ensure_credentials_are_safe(remote_base: &str, username: Option<&str>, password: Option<&str>) -> Result<(), DomainError> {
    if username.is_none() && password.is_none() {
        return Ok(());
    }
    let url = Url::parse(remote_base).map_err(|e| DomainError::Infrastructure(format!("invalid remote URL {remote_base}: {e}")))?;
    if url.scheme() == "https" || is_loopback(&url) {
        return Ok(());
    }
    Err(DomainError::Infrastructure(format!("refusing to send credentials to {remote_base} over plain http")))
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_go_to_the_same_https_origin_only() {
        let remote = "https://registry.example.com/base";
        assert!(may_receive_credentials(remote, "https://registry.example.com/base/pkg/-/pkg-1.0.0.tgz"));
        assert!(may_receive_credentials(remote, "https://REGISTRY.example.com:443/other"));
        assert!(!may_receive_credentials(remote, "https://cdn.example.com/pkg.tgz"));
        assert!(!may_receive_credentials(remote, "https://registry.example.com:8443/pkg.tgz"));
        assert!(!may_receive_credentials(remote, "http://registry.example.com/pkg.tgz"));
        assert!(!may_receive_credentials(remote, "https://registry.example.com.evil.test/pkg.tgz"));
        assert!(!may_receive_credentials(remote, "https://registry.example.com@evil.test/pkg.tgz"));
    }

    #[test]
    fn plain_http_is_trusted_for_loopback_only() {
        assert!(may_receive_credentials("http://127.0.0.1:4873", "http://127.0.0.1:4873/pkg.tgz"));
        assert!(may_receive_credentials("http://localhost:4873", "http://localhost:4873/pkg.tgz"));
        assert!(!may_receive_credentials("http://registry.example.com", "http://registry.example.com/pkg.tgz"));
    }

    #[test]
    fn an_http_remote_with_credentials_is_refused_and_without_them_is_fine() {
        assert!(ensure_credentials_are_safe("http://registry.example.com", Some("user"), Some("pass")).is_err());
        assert!(ensure_credentials_are_safe("http://registry.example.com", None, Some("token")).is_err());
        assert!(ensure_credentials_are_safe("http://registry.example.com", None, None).is_ok());
        assert!(ensure_credentials_are_safe("https://registry.example.com", Some("user"), Some("pass")).is_ok());
    }

    #[test]
    fn a_foreign_host_gets_no_authorization_header() {
        let remote = "https://registry.example.com";
        let to_foreign = apply_credentials_if_allowed(reqwest::Client::new().get("https://cdn.evil.test/x.tgz"), remote, "https://cdn.evil.test/x.tgz", Some("user"), Some("pass")).build().unwrap();
        assert!(to_foreign.headers().get(reqwest::header::AUTHORIZATION).is_none());
        let to_remote = apply_credentials_if_allowed(reqwest::Client::new().get("https://registry.example.com/x.tgz"), remote, "https://registry.example.com/x.tgz", Some("user"), Some("pass")).build().unwrap();
        assert!(to_remote.headers().get(reqwest::header::AUTHORIZATION).is_some());
    }
}
