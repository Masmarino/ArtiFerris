use async_trait::async_trait;
use artiferris_domain::error::DomainError;
use artiferris_domain::npm_package::NpmPackageName;
use artiferris_domain::npm_remote::RemoteNpmRegistryPort;

use crate::capped_response::read_capped;
use crate::remote_credentials::{apply_credentials_if_allowed, ensure_credentials_are_safe};
use crate::ssrf_guard::ensure_public_host;

const MAX_METADATA_RESPONSE_BYTES: usize = 50 * 1024 * 1024;
/// Matches the npm publish body limit (`artiferris-npm`'s own router).
const MAX_TARBALL_RESPONSE_BYTES: usize = 200 * 1024 * 1024;

pub struct HttpRemoteNpmRegistry {
    client: reqwest::Client,
}

impl HttpRemoteNpmRegistry {
    pub fn new() -> Self {
        // No redirect-following, or a malicious upstream could 302 past the SSRF guard.
        Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client config is static and always valid"),
        }
    }
}

impl Default for HttpRemoteNpmRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RemoteNpmRegistryPort for HttpRemoteNpmRegistry {
    async fn fetch_metadata(
        &self,
        base_url: &str,
        package_name: &NpmPackageName,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<serde_json::Value>, DomainError> {
        // npm scoped package names (@scope/name) are sent URL-encoded (@scope%2fname), not as a literal nested path segment.
        let encoded_name = urlencoding_replace_slash(package_name.as_str());
        let url = format!("{}/{}", base_url.trim_end_matches('/'), encoded_name);
        ensure_credentials_are_safe(base_url, username, password)?;
        ensure_public_host(&url).await?;
        let response = apply_credentials_if_allowed(self.client.get(&url).header("Accept", "application/json"), base_url, &url, username, password)
            .send()
            .await
            .map_err(|e| DomainError::Infrastructure(format!("fetching npm metadata from {url}: {e}")))?;
        map_metadata_response(response, &url).await
    }

    async fn fetch_tarball(&self, base_url: &str, tarball_url: &str, username: Option<&str>, password: Option<&str>) -> Result<Option<Vec<u8>>, DomainError> {
        let url = if tarball_url.starts_with("http://") || tarball_url.starts_with("https://") {
            tarball_url.to_string()
        } else {
            format!("{}/{}", base_url.trim_end_matches('/'), tarball_url.trim_start_matches('/'))
        };
        ensure_credentials_are_safe(base_url, username, password)?;
        ensure_public_host(&url).await?;
        let response = apply_credentials_if_allowed(self.client.get(&url), base_url, &url, username, password)
            .send()
            .await
            .map_err(|e| DomainError::Infrastructure(format!("fetching npm tarball from {url}: {e}")))?;
        map_tarball_response(response, &url).await
    }
}

/// Split out of `fetch_metadata` so it can be tested against an already-obtained response.
/// `Ok(None)` for a genuine 404 — not an error.
async fn map_metadata_response(response: reqwest::Response, url: &str) -> Result<Option<serde_json::Value>, DomainError> {
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(DomainError::Infrastructure(format!("remote registry returned {} for {url}", response.status())));
    }
    let bytes = read_capped(response, url, "npm metadata", MAX_METADATA_RESPONSE_BYTES).await?;
    let document = serde_json::from_slice(&bytes).map_err(|e| DomainError::Infrastructure(format!("parsing npm metadata from {url}: {e}")))?;
    Ok(Some(document))
}

/// Non-2xx / body-read mapping for `fetch_tarball`, split out for the same reason as `map_metadata_response` above.
/// `Ok(None)` for a genuine 404 — not an error.
async fn map_tarball_response(response: reqwest::Response, url: &str) -> Result<Option<Vec<u8>>, DomainError> {
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(DomainError::Infrastructure(format!("remote registry returned {} for {url}", response.status())));
    }
    let bytes = read_capped(response, url, "npm tarball body", MAX_TARBALL_RESPONSE_BYTES).await?;
    Ok(Some(bytes))
}

fn urlencoding_replace_slash(name: &str) -> String {
    name.replace('/', "%2f")
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::npm_package::NpmPackageName;

    #[tokio::test]
    async fn credentials_are_never_sent_to_an_http_remote() {
        let registry = HttpRemoteNpmRegistry::new();

        let metadata = registry.fetch_metadata("http://registry.example.com", &NpmPackageName::parse("left-pad").unwrap(), Some("user"), Some("pass")).await.unwrap_err();
        let tarball = registry.fetch_tarball("http://registry.example.com", "left-pad/-/left-pad-1.0.0.tgz", None, Some("token")).await.unwrap_err();

        assert!(metadata.to_string().contains("plain http"), "{metadata}");
        assert!(tarball.to_string().contains("plain http"), "{tarball}");
    }

    // Ground truth against a real, stable public package.
    #[tokio::test]
    #[ignore = "requires network access to registry.npmjs.org"]
    async fn fetches_real_metadata_from_npmjs() {
        let registry = HttpRemoteNpmRegistry::new();
        let doc = registry.fetch_metadata("https://registry.npmjs.org", &NpmPackageName::parse("is-odd").unwrap(), None, None).await.unwrap().unwrap();
        assert!(doc.get("dist-tags").is_some());
    }

    #[tokio::test]
    async fn the_configured_client_does_not_follow_a_redirect() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: http://internal.example/\r\nContent-Length: 0\r\n\r\n")
                .await;
        });

        let client = HttpRemoteNpmRegistry::new().client;
        let response = client.get(format!("http://{addr}/")).send().await.unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::FOUND, "the client must return the redirect itself, not silently follow it");
    }

    #[tokio::test]
    async fn a_hung_upstream_does_not_block_forever() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            // Accept the connection but never write a response — simulates a hung upstream.
            std::future::pending::<()>().await;
        });

        // fetch_metadata calls ensure_public_host(&url) first, which would reject this loopback
        // address before reqwest ever opens a socket — that would make this test pass even with
        // no timeout configured at all. Bypass it the same way
        // `the_configured_client_does_not_follow_a_redirect` does below: drive the raw configured
        // client directly against the local listener.
        let client = HttpRemoteNpmRegistry::new().client;
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(std::time::Duration::from_secs(40), client.get(format!("http://{addr}/")).send()).await;

        let send_result = result.expect("the client's own configured timeout must fire well within this test's 40s outer bound");
        assert!(send_result.is_err(), "a request against a hung upstream must fail with the client's configured timeout, not succeed");
        assert!(
            started.elapsed() >= std::time::Duration::from_secs(25),
            "should take close to the configured 30s request timeout, not resolve near-instantly (got {:?})",
            started.elapsed()
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(35), "the client's own configured 30s request timeout must fire before this test's outer bound");
    }

    // These tests bypass ensure_public_host to fetch directly from a local wiremock server (which is loopback).
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn get(url: &str) -> reqwest::Response {
        reqwest::Client::new().get(url).send().await.expect("request to local mock server must succeed")
    }

    // NOTE: the brief's Step 1 sample drives these through `registry.fetch_metadata(...)`
    // directly against a local wiremock server. That no longer works: `fetch_metadata` calls
    // `ensure_public_host` first, which rejects loopback before reqwest ever sends the request
    // (see the "these tests bypass ensure_public_host" note above and `a_hung_upstream_does_not_block_forever`).
    // Adapted to drive `map_metadata_response`/`map_tarball_response` directly against an
    // already-obtained response, same as every other test in this module — behavior asserted
    // (`Ok(None)` for 404, `Err` for 500) is identical to what the brief intended.
    #[tokio::test]
    async fn a_404_metadata_response_is_not_found_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/some-package")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        let url = format!("{}/some-package", server.uri());

        let result = map_metadata_response(get(&url).await, &url).await.unwrap();

        assert!(result.is_none(), "a 404 from upstream must be Ok(None), not an error");
    }

    #[tokio::test]
    async fn a_500_metadata_response_is_still_a_domain_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/some-package")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let url = format!("{}/some-package", server.uri());

        let result = map_metadata_response(get(&url).await, &url).await;

        assert!(result.is_err(), "a genuine upstream failure (not a 404) must still surface as an error");
    }

    #[tokio::test]
    async fn maps_a_500_metadata_response_to_a_domain_error_naming_the_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/is-odd")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let url = format!("{}/is-odd", server.uri());

        let err = map_metadata_response(get(&url).await, &url).await.unwrap_err();

        assert!(err.to_string().contains("500"), "got: {err}");
        assert!(err.to_string().contains("remote registry returned"), "got: {err}");
    }

    #[tokio::test]
    async fn maps_a_200_response_with_a_non_json_body_to_a_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/is-odd")).respond_with(ResponseTemplate::new(200).set_body_string("not json")).mount(&server).await;
        let url = format!("{}/is-odd", server.uri());

        let err = map_metadata_response(get(&url).await, &url).await.unwrap_err();

        assert!(err.to_string().contains("parsing npm metadata"), "got: {err}");
    }

    #[tokio::test]
    async fn maps_a_200_response_with_a_valid_body_to_the_parsed_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/is-odd"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"dist-tags": {"latest": "3.0.1"}})))
            .mount(&server)
            .await;
        let url = format!("{}/is-odd", server.uri());

        let doc = map_metadata_response(get(&url).await, &url).await.unwrap().unwrap();

        assert_eq!(doc["dist-tags"]["latest"], "3.0.1");
    }

    // Same SSRF-guard adaptation as `a_404_metadata_response_is_not_found_not_an_error` above.
    #[tokio::test]
    async fn a_404_tarball_response_is_not_found_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/is-odd.tgz")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        let url = format!("{}/is-odd.tgz", server.uri());

        let result = map_tarball_response(get(&url).await, &url).await.unwrap();

        assert!(result.is_none(), "a 404 from upstream must be Ok(None), not an error");
    }

    #[tokio::test]
    async fn a_500_tarball_response_is_still_a_domain_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/is-odd.tgz")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let url = format!("{}/is-odd.tgz", server.uri());

        let result = map_tarball_response(get(&url).await, &url).await;

        assert!(result.is_err(), "a genuine upstream failure (not a 404) must still surface as an error");
    }

    #[tokio::test]
    async fn maps_a_500_tarball_response_to_a_domain_error_naming_the_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/is-odd.tgz")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let url = format!("{}/is-odd.tgz", server.uri());

        let err = map_tarball_response(get(&url).await, &url).await.unwrap_err();

        assert!(err.to_string().contains("500"), "got: {err}");
        assert!(err.to_string().contains("remote registry returned"), "got: {err}");
    }

    #[tokio::test]
    async fn maps_a_200_tarball_response_to_its_raw_bytes() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/is-odd.tgz")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x1f, 0x8b, 0x03])).mount(&server).await;
        let url = format!("{}/is-odd.tgz", server.uri());

        let bytes = map_tarball_response(get(&url).await, &url).await.unwrap().unwrap();

        assert_eq!(bytes, vec![0x1f, 0x8b, 0x03]);
    }
}
