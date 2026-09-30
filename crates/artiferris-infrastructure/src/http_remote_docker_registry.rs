use async_trait::async_trait;
use artiferris_domain::docker_registry::{Digest, DockerImageName, validate_manifest_reference};
use artiferris_domain::docker_remote::{RemoteBlob, RemoteDockerRegistryPort};
use futures_util::TryStreamExt;
use artiferris_domain::error::DomainError;

use crate::capped_response::read_capped;
use crate::remote_credentials::{apply_credentials_if_allowed, ensure_credentials_are_safe};
use crate::ssrf_guard::{PublicOnlyResolver, ensure_public_host};

const ACCEPTED_MANIFEST_MEDIA_TYPES: &str = "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json";
/// Matches `artiferris-docker`'s own manifest-push cap (`manifests::MANIFEST_BODY_LIMIT_BYTES`).
const MAX_MANIFEST_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOKEN_RESPONSE_BYTES: usize = 1024 * 1024;
/// Registries answer a blob request with a redirect to a CDN or an object store, one hop in practice.
const MAX_BLOB_REDIRECTS: usize = 5;

pub struct HttpRemoteDockerRegistry {
    client: reqwest::Client,
    blob_client: reqwest::Client,
    /// The origins of a test's fake registries, the only places plain http or a loopback address is accepted.
    #[cfg(test)]
    trusted_test_origins: Vec<(String, u16)>,
}

/// Redirects are followed by hand, only for blob requests, so every hop goes through the same checks as the first request.
fn build_blob_client(refuse_private_addresses: bool) -> reqwest::Client {
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(5))
        .read_timeout(std::time::Duration::from_secs(30));
    let builder = if refuse_private_addresses { builder.dns_resolver(std::sync::Arc::new(PublicOnlyResolver)) } else { builder };
    builder.build().expect("reqwest client config is static and always valid")
}

impl HttpRemoteDockerRegistry {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client config is static and always valid");

        let blob_client = build_blob_client(true);

        Self {
            client,
            blob_client,
            #[cfg(test)]
            trusted_test_origins: Vec::new(),
        }
    }

    /// Accepts the origin of a test's fake registry, plain http and loopback included, and drops the connect-time address check
    /// that would refuse it.
    #[cfg(test)]
    fn trusting_test_origins(mut self, origins: &[std::net::SocketAddr]) -> Self {
        self.trusted_test_origins = origins.iter().map(|addr| (addr.ip().to_string(), addr.port())).collect();
        self.blob_client = build_blob_client(false);
        self
    }

    /// The public-host check every request goes through, with the test origins let through.
    async fn ensure_reachable(&self, url: &str) -> Result<(), DomainError> {
        #[cfg(test)]
        if let Ok(parsed) = reqwest::Url::parse(url) {
            if self.trusted_test_origins.iter().any(|(host, port)| parsed.host_str() == Some(host) && parsed.port_or_known_default() == Some(*port)) {
                return Ok(());
            }
        }
        ensure_public_host(url).await
    }

    /// A redirect target must be https, unless it is a test's fake registry, and pass the public-host check.
    async fn ensure_redirect_target(&self, target: &reqwest::Url) -> Result<(), DomainError> {
        #[cfg(test)]
        if self.trusted_test_origins.iter().any(|(host, port)| target.host_str() == Some(host) && target.port_or_known_default() == Some(*port)) {
            return Ok(());
        }
        if target.scheme() != "https" {
            return Err(DomainError::Infrastructure(format!("refusing to follow a redirect to {target}: it is not https")));
        }
        ensure_public_host(target.as_str()).await
    }

    /// On a 401 with a Bearer challenge, fetches a token and retries once. `Ok(None)` for a genuine 404 (either response) — not an error.
    #[allow(clippy::too_many_arguments)]
    async fn get_with_bearer_challenge(
        &self,
        base_url: &str,
        url: &str,
        accept: &str,
        username: Option<&str>,
        password: Option<&str>,
        client: &reqwest::Client,
        follow_redirects: bool,
    ) -> Result<Option<reqwest::Response>, DomainError> {
        ensure_credentials_are_safe(base_url, username, password)?;
        self.ensure_reachable(url).await?;
        let first =
            client.get(url).header("Accept", accept).send().await.map_err(|e| DomainError::Infrastructure(format!("requesting {url}: {e}")))?;
        if first.status() != reqwest::StatusCode::UNAUTHORIZED {
            return self.settle(first, url, accept, None, client, follow_redirects).await;
        }

        let challenge = extract_challenge_or_error(&first, url)?;
        self.ensure_reachable(&challenge.realm).await?;

        let mut token_request = client.get(&challenge.realm).query(&[("service", challenge.service.as_str())]);
        if let Some(scope) = &challenge.scope {
            token_request = token_request.query(&[("scope", scope.as_str())]);
        }
        token_request = apply_credentials_if_allowed(token_request, base_url, &challenge.realm, username, password);
        let token_response = token_request
            .send()
            .await
            .map_err(|e| DomainError::Infrastructure(format!("fetching token from {}: {e}", challenge.realm)))?;
        let token_body = parse_token_response(Self::require_success(token_response, &challenge.realm).await?, &challenge.realm).await?;

        let retried = client
            .get(url)
            .header("Accept", accept)
            .bearer_auth(&token_body.token)
            .send()
            .await
            .map_err(|e| DomainError::Infrastructure(format!("requesting {url}: {e}")))?;
        self.settle(retried, url, accept, Some(&token_body.token), client, follow_redirects).await
    }

    /// `None` for a 404, the response for a success, a redirect followed if `follow_redirects`, anything else an error.
    async fn settle(
        &self,
        response: reqwest::Response,
        url: &str,
        accept: &str,
        bearer: Option<&str>,
        client: &reqwest::Client,
        follow_redirects: bool,
    ) -> Result<Option<reqwest::Response>, DomainError> {
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if follow_redirects && response.status().is_redirection() {
            return self.follow_redirects(response, url, accept, bearer, client).await;
        }
        Self::require_success(response, url).await.map(Some)
    }

    /// Follows up to `MAX_BLOB_REDIRECTS` hops. Each target must be https and pass the public-host check, and it is asked for
    /// with no credentials, except that the registry's own token goes along to a target on the registry's own origin.
    async fn follow_redirects(&self, mut response: reqwest::Response, url: &str, accept: &str, bearer: Option<&str>, client: &reqwest::Client) -> Result<Option<reqwest::Response>, DomainError> {
        let registry = reqwest::Url::parse(url).map_err(|e| DomainError::Infrastructure(format!("invalid remote URL {url}: {e}")))?;
        let mut current = registry.clone();
        for _ in 0..MAX_BLOB_REDIRECTS {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| DomainError::Infrastructure(format!("{current} redirected without a Location")))?;
            let target = current.join(location).map_err(|e| DomainError::Infrastructure(format!("{current} redirected to an invalid location: {e}")))?;
            self.ensure_redirect_target(&target).await?;
            let mut request = client.get(target.clone()).header("Accept", accept);
            if let (Some(token), true) = (bearer, target.origin() == registry.origin()) {
                request = request.bearer_auth(token);
            }
            response = request.send().await.map_err(|e| DomainError::Infrastructure(format!("requesting {target}: {e}")))?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            current = target;
            if !response.status().is_redirection() {
                return Self::require_success(response, current.as_str()).await.map(Some);
            }
        }
        Err(DomainError::Infrastructure(format!("{url} redirected more than {MAX_BLOB_REDIRECTS} times")))
    }

    async fn require_success(response: reqwest::Response, url: &str) -> Result<reqwest::Response, DomainError> {
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(DomainError::Infrastructure(format!("remote registry returned {} for {url}", response.status())))
        }
    }
}

impl Default for HttpRemoteDockerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RemoteDockerRegistryPort for HttpRemoteDockerRegistry {
    async fn fetch_manifest(
        &self,
        base_url: &str,
        image_name: &DockerImageName,
        reference: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<(Vec<u8>, String)>, DomainError> {
        validate_manifest_reference(reference)?;
        let url = upstream_url(base_url, image_name, "manifests", reference);
        let response = self.get_with_bearer_challenge(base_url, &url, ACCEPTED_MANIFEST_MEDIA_TYPES, username, password, &self.client, false).await?;
        map_manifest_response(response, &url).await
    }

    async fn fetch_blob(
        &self,
        base_url: &str,
        image_name: &DockerImageName,
        digest: &Digest,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<RemoteBlob>, DomainError> {
        let url = upstream_url(base_url, image_name, "blobs", digest.as_str());
        let response = self.get_with_bearer_challenge(base_url, &url, "application/octet-stream", username, password, &self.blob_client, true).await?;
        Ok(response.map(|response| map_blob_response(response, &url)))
    }
}

/// Split out of `get_with_bearer_challenge` so it can be tested against an already-obtained response.
fn extract_challenge_or_error(response: &reqwest::Response, url: &str) -> Result<BearerChallenge, DomainError> {
    response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer_challenge)
        .ok_or_else(|| DomainError::Infrastructure(format!("{url} returned 401 without a Bearer challenge")))
}

/// JSON-parse mapping for the token-endpoint response, split out for the same reason as `extract_challenge_or_error` above.
async fn parse_token_response(response: reqwest::Response, realm: &str) -> Result<TokenResponse, DomainError> {
    let bytes = read_capped(response, realm, "token response", MAX_TOKEN_RESPONSE_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|e| DomainError::Infrastructure(format!("parsing token response from {realm}: {e}")))
}

/// Body-read mapping for `fetch_manifest`, split out for the same reason as `extract_challenge_or_error` above.
async fn map_manifest_response(response: Option<reqwest::Response>, url: &str) -> Result<Option<(Vec<u8>, String)>, DomainError> {
    let Some(response) = response else {
        return Ok(None);
    };
    let content_type = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
    let bytes = read_capped(response, url, "manifest body", MAX_MANIFEST_RESPONSE_BYTES).await?;
    Ok(Some((bytes, content_type)))
}

/// The blob's body as a stream, so it is never held whole in memory.
fn map_blob_response(response: reqwest::Response, url: &str) -> RemoteBlob {
    let url = url.to_string();
    RemoteBlob {
        content_length: response.content_length(),
        stream: Box::pin(response.bytes_stream().map_err(move |e| DomainError::Infrastructure(format!("reading blob body from {url}: {e}")))),
    }
}

/// Every segment is percent-encoded, so nothing can climb out of its path (`..`, `?`, `#`).
fn upstream_url(base_url: &str, image_name: &DockerImageName, kind: &str, tail: &str) -> String {
    let image_path = image_name.as_str().split('/').map(encode_path_segment).collect::<Vec<_>>().join("/");
    format!("{}/v2/{image_path}/{kind}/{}", base_url.trim_end_matches('/'), encode_path_segment(tail))
}

/// Leaves unreserved characters and `:` (the digest separator) as they are.
fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b':' => encoded.push(byte as char),
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

#[derive(serde::Deserialize, Debug)]
struct TokenResponse {
    token: String,
}

#[derive(Debug)]
struct BearerChallenge {
    realm: String,
    service: String,
    scope: Option<String>,
}

/// `scope` is optional — some unscoped 401s omit it.
fn parse_bearer_challenge(header_value: &str) -> Option<BearerChallenge> {
    let rest = header_value.strip_prefix("Bearer ")?;
    let mut realm = None;
    let mut service = None;
    let mut scope = None;
    for part in split_challenge_params(rest) {
        let (key, value) = part.split_once('=')?;
        let value = value.trim_matches('"').to_string();
        match key {
            "realm" => realm = Some(value),
            "service" => service = Some(value),
            "scope" => scope = Some(value),
            _ => {}
        }
    }
    Some(BearerChallenge { realm: realm?, service: service?, scope })
}

/// Splits `key="value",key2="value2"` on commas outside quotes — a plain `.split(',')` would break on a quoted comma.
fn split_challenge_params(rest: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in rest.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            ',' if !in_quotes => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts.into_iter().map(|p| p.trim().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_credentials::apply_credentials;

    #[test]
    fn upstream_urls_percent_encode_every_segment() {
        let name = DockerImageName::parse("team/app").unwrap();
        assert_eq!(upstream_url("https://registry.example.com/", &name, "manifests", "latest"), "https://registry.example.com/v2/team/app/manifests/latest");
        let digest = format!("sha256:{}", "a".repeat(64));
        assert_eq!(upstream_url("https://r.example/base", &name, "blobs", &digest), format!("https://r.example/base/v2/team/app/blobs/{digest}"));
        assert_eq!(upstream_url("https://r.example", &name, "manifests", "../../x?y#z/w"), "https://r.example/v2/team/app/manifests/..%2F..%2Fx%3Fy%23z%2Fw");
    }

    #[tokio::test]
    async fn a_reference_that_is_neither_a_tag_nor_a_digest_never_reaches_the_network() {
        let registry = HttpRemoteDockerRegistry::new();
        let name = DockerImageName::parse("app").unwrap();

        for bad in ["../../other/manifests/latest", "latest?x=1", "a/b"] {
            let err = registry.fetch_manifest("https://registry.example.com", &name, bad, None, None).await.unwrap_err();
            assert!(err.to_string().contains("invalid tag"), "{bad:?}: {err}");
        }
    }

    #[tokio::test]
    async fn credentials_are_never_sent_to_an_http_remote() {
        let registry = HttpRemoteDockerRegistry::new();
        let name = DockerImageName::parse("app").unwrap();

        let err = registry.fetch_manifest("http://registry.example.com", &name, "latest", Some("user"), Some("pass")).await.unwrap_err();

        assert!(err.to_string().contains("plain http"), "{err}");
    }

    #[test]
    fn parses_realm_service_and_scope_from_a_bearer_challenge() {
        let header = r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/alpine:pull""#;
        let challenge = parse_bearer_challenge(header).unwrap();
        assert_eq!(challenge.realm, "https://auth.docker.io/token");
        assert_eq!(challenge.service, "registry.docker.io");
        assert_eq!(challenge.scope.as_deref(), Some("repository:library/alpine:pull"));
    }

    #[test]
    fn parses_a_challenge_with_no_scope() {
        let header = r#"Bearer realm="https://auth.example/token",service="registry.example""#;
        let challenge = parse_bearer_challenge(header).unwrap();
        assert!(challenge.scope.is_none());
    }

    #[test]
    fn returns_none_for_a_non_bearer_challenge() {
        assert!(parse_bearer_challenge(r#"Basic realm="registry""#).is_none());
    }

    #[tokio::test]
    #[ignore = "requires network access to registry-1.docker.io"]
    async fn fetches_a_real_manifest_from_docker_hub_via_the_bearer_challenge_flow() {
        let registry = HttpRemoteDockerRegistry::new();
        let image_name = artiferris_domain::docker_registry::DockerImageName::parse("library/alpine").unwrap();
        let (bytes, content_type) = registry.fetch_manifest("https://registry-1.docker.io", &image_name, "latest", None, None).await.unwrap().unwrap();
        assert!(!bytes.is_empty());
        assert!(content_type.contains("manifest") || content_type.contains("json"));
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

        let client = HttpRemoteDockerRegistry::new().client;
        let response = client.get(format!("http://{addr}/")).send().await.unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::FOUND, "the client must return the redirect itself, not silently follow it");
    }

    #[tokio::test]
    async fn a_hung_upstream_does_not_block_forever() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });

        // fetch_manifest -> get_with_bearer_challenge calls ensure_public_host(url) first, which
        // would reject this loopback address before reqwest ever opens a socket — that would make
        // this test pass even with no timeout configured at all. Bypass it the same way
        // `the_configured_client_does_not_follow_a_redirect` does below: drive the raw `client`
        // (used for manifest/token calls) directly against the local listener.
        let client = HttpRemoteDockerRegistry::new().client;
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

    /// `blob_client` only limits silence (30s), since a blob can take a long time to arrive. Ignored by default so the regular run stays fast.
    #[tokio::test]
    #[ignore = "waits out the real 30s read timeout of blob_client"]
    async fn a_hung_upstream_does_not_block_the_blob_client_forever() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });

        // Same SSRF-guard bypass as above, but against `blob_client` (used by fetch_blob).
        let client = HttpRemoteDockerRegistry::new().blob_client;
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(std::time::Duration::from_secs(60), client.get(format!("http://{addr}/")).send()).await;

        assert!(result.expect("the read timeout must fire well within this test's 60s bound").is_err());
        assert!(started.elapsed() >= std::time::Duration::from_secs(25), "resolved too early: {:?}", started.elapsed());
    }

    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn get(url: &str) -> reqwest::Response {
        reqwest::Client::new().get(url).send().await.expect("request to local mock server must succeed")
    }

    #[tokio::test]
    async fn require_success_maps_a_404_response_to_a_domain_error_naming_the_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v2/lib/manifests/latest")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        let url = format!("{}/v2/lib/manifests/latest", server.uri());

        let err = HttpRemoteDockerRegistry::require_success(get(&url).await, &url).await.unwrap_err();

        assert!(err.to_string().contains("404"), "got: {err}");
        assert!(err.to_string().contains("remote registry returned"), "got: {err}");
    }

    #[tokio::test]
    async fn require_success_maps_a_500_response_to_a_domain_error_naming_the_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v2/lib/manifests/latest")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let url = format!("{}/v2/lib/manifests/latest", server.uri());

        let err = HttpRemoteDockerRegistry::require_success(get(&url).await, &url).await.unwrap_err();

        assert!(err.to_string().contains("500"), "got: {err}");
        assert!(err.to_string().contains("remote registry returned"), "got: {err}");
    }

    #[tokio::test]
    async fn require_success_passes_through_a_200_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v2/lib/manifests/latest")).respond_with(ResponseTemplate::new(200).set_body_string("ok")).mount(&server).await;
        let url = format!("{}/v2/lib/manifests/latest", server.uri());

        let response = HttpRemoteDockerRegistry::require_success(get(&url).await, &url).await.unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn extract_challenge_or_error_rejects_a_401_with_no_www_authenticate_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v2/lib/manifests/latest")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        let url = format!("{}/v2/lib/manifests/latest", server.uri());

        let err = extract_challenge_or_error(&get(&url).await, &url).unwrap_err();

        assert!(err.to_string().contains("without a Bearer challenge"), "got: {err}");
    }

    #[tokio::test]
    async fn extract_challenge_or_error_rejects_a_401_with_a_non_bearer_challenge() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/lib/manifests/latest"))
            .respond_with(ResponseTemplate::new(401).insert_header("www-authenticate", r#"Basic realm="registry""#))
            .mount(&server)
            .await;
        let url = format!("{}/v2/lib/manifests/latest", server.uri());

        let err = extract_challenge_or_error(&get(&url).await, &url).unwrap_err();

        assert!(err.to_string().contains("without a Bearer challenge"), "got: {err}");
    }

    #[tokio::test]
    async fn extract_challenge_or_error_parses_a_valid_bearer_challenge() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/lib/manifests/latest"))
            .respond_with(ResponseTemplate::new(401).insert_header(
                "www-authenticate",
                r#"Bearer realm="https://auth.example/token",service="registry.example""#,
            ))
            .mount(&server)
            .await;
        let url = format!("{}/v2/lib/manifests/latest", server.uri());

        let challenge = extract_challenge_or_error(&get(&url).await, &url).unwrap();

        assert_eq!(challenge.realm, "https://auth.example/token");
        assert_eq!(challenge.service, "registry.example");
    }

    #[tokio::test]
    async fn parse_token_response_rejects_a_non_json_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/token")).respond_with(ResponseTemplate::new(200).set_body_string("not json")).mount(&server).await;
        let url = format!("{}/token", server.uri());

        let err = parse_token_response(get(&url).await, &url).await.unwrap_err();

        assert!(err.to_string().contains("parsing token response"), "got: {err}");
    }

    #[tokio::test]
    async fn parse_token_response_accepts_a_valid_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "abc123"})))
            .mount(&server)
            .await;
        let url = format!("{}/token", server.uri());

        let token = parse_token_response(get(&url).await, &url).await.unwrap();

        assert_eq!(token.token, "abc123");
    }

    #[tokio::test]
    async fn map_manifest_response_passes_none_through_unchanged() {
        assert!(map_manifest_response(None, "http://example/whatever").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn map_manifest_response_returns_bytes_and_content_type_for_a_200_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/lib/manifests/latest"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "application/vnd.docker.distribution.manifest.v2+json").set_body_bytes(b"{}".to_vec()))
            .mount(&server)
            .await;
        let url = format!("{}/v2/lib/manifests/latest", server.uri());

        let (bytes, content_type) = map_manifest_response(Some(get(&url).await), &url).await.unwrap().unwrap();

        assert_eq!(bytes, b"{}".to_vec());
        assert_eq!(content_type, "application/vnd.docker.distribution.manifest.v2+json");
    }

    #[tokio::test]
    async fn map_blob_response_streams_the_body_and_reports_its_declared_length() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v2/lib/blobs/sha256:abc")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3])).mount(&server).await;
        let url = format!("{}/v2/lib/blobs/sha256:abc", server.uri());

        let blob = map_blob_response(get(&url).await, &url);

        assert_eq!(blob.content_length, Some(3));
        let chunks: Vec<bytes::Bytes> = blob.stream.try_collect().await.unwrap();
        assert_eq!(chunks.concat(), vec![1, 2, 3]);
    }

    // apply_credentials is what get_with_bearer_challenge calls to attach credentials to the
    // token request. It's exercised directly (rather than through get_with_bearer_challenge's
    // full WWW-Authenticate -> token-fetch -> retry flow) because that flow starts with
    // ensure_public_host(url), which unconditionally rejects loopback addresses — a local
    // wiremock server would never even get a request. Sending the built request to a mock that
    // only responds to the exact expected auth header proves the credential was actually
    // attached, the same way parse_token_response's tests above prove behavior by sending a real
    // request and inspecting what came back.
    #[tokio::test]
    async fn a_password_with_no_username_is_sent_as_a_bearer_token_to_the_token_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .and(header("authorization", "Bearer secret-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "token": "upstream-access-token" })))
            .mount(&server)
            .await;
        let url = format!("{}/token", server.uri());

        let request = apply_credentials(reqwest::Client::new().get(&url), None, Some("secret-token"));
        let response = request.send().await.expect("request to local mock server must succeed");

        assert_eq!(
            response.status(),
            reqwest::StatusCode::OK,
            "a password-only credential (no username) must be sent as a Bearer token, or the mock (which only matches that exact header) never matches"
        );
    }

    #[tokio::test]
    async fn a_username_and_password_are_still_sent_as_basic_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .and(header("authorization", "Basic dXNlcjpwYXNz"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "token": "upstream-access-token" })))
            .mount(&server)
            .await;
        let url = format!("{}/token", server.uri());

        let request = apply_credentials(reqwest::Client::new().get(&url), Some("user"), Some("pass"));
        let response = request.send().await.expect("request to local mock server must succeed");

        assert_eq!(response.status(), reqwest::StatusCode::OK, "username+password must still be sent as Basic auth, unaffected by the bearer-only fix");
    }

    #[test]
    fn no_credentials_at_all_sends_no_authorization_header() {
        let request = apply_credentials(reqwest::Client::new().get("http://example.invalid/token"), None, None).build().unwrap();

        assert!(request.headers().get(reqwest::header::AUTHORIZATION).is_none(), "no credentials configured must mean no Authorization header at all");
    }

    type Requests = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// Answers each connection on `listener` once, from `respond(request)` (lower-cased), and keeps the requests it saw.
    fn serve(listener: tokio::net::TcpListener, respond: impl Fn(&str) -> String + Send + Sync + 'static) -> Requests {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let seen: Requests = Default::default();
        let recorded = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                let response = respond(&request);
                recorded.lock().unwrap().push(request);
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        seen
    }

    async fn fake_server(respond: impl Fn(&str) -> String + Send + Sync + 'static) -> (std::net::SocketAddr, Requests) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        (listener.local_addr().unwrap(), serve(listener, respond))
    }

    fn redirect_to(location: &str) -> String {
        format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    }

    fn ok(body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    /// A registry that wants a token for `/blobs/`, hands one out at `/token` and, once shown it, answers with `then`.
    async fn registry_with_token(then: impl Fn(&str) -> String + Send + Sync + 'static) -> (std::net::SocketAddr, Requests) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = serve(listener, move |request| {
            if request.starts_with("get /token") {
                ok(r#"{"token":"registry-secret"}"#)
            } else if request.contains("authorization: bearer registry-secret") {
                then(request)
            } else {
                format!("HTTP/1.1 401 Unauthorized\r\nWww-Authenticate: Bearer realm=\"http://{addr}/token\",service=\"fake\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            }
        });
        (addr, requests)
    }

    async fn blob_of(registry: &HttpRemoteDockerRegistry, registry_addr: std::net::SocketAddr, credentials: bool) -> Result<Option<Vec<u8>>, DomainError> {
        let name = DockerImageName::parse("library/app").unwrap();
        let digest = Digest::of(b"the layer");
        let (username, password) = if credentials { (Some("user"), Some("pass")) } else { (None, None) };
        let Some(blob) = registry.fetch_blob(&format!("http://{registry_addr}"), &name, &digest, username, password).await? else { return Ok(None) };
        let chunks: Vec<bytes::Bytes> = blob.stream.try_collect().await?;
        Ok(Some(chunks.concat()))
    }

    #[tokio::test]
    async fn a_blob_redirect_to_another_host_is_followed_without_the_registrys_credentials() {
        let (storage, storage_requests) = fake_server(|_| ok("layer-bytes")).await;
        let (registry_addr, registry_requests) = registry_with_token(move |_| redirect_to(&format!("http://{storage}/objects/layer"))).await;
        let registry = HttpRemoteDockerRegistry::new().trusting_test_origins(&[registry_addr, storage]);

        let bytes = blob_of(&registry, registry_addr, true).await.unwrap();

        assert_eq!(bytes, Some(b"layer-bytes".to_vec()));
        let storage_requests = storage_requests.lock().unwrap();
        assert_eq!(storage_requests.len(), 1);
        assert!(!storage_requests[0].contains("authorization"), "nothing the registry was given may reach the other host: {}", storage_requests[0]);
        let registry_requests = registry_requests.lock().unwrap();
        assert!(registry_requests.iter().any(|request| request.starts_with("get /token") && request.contains("authorization: basic")), "the token request is where the credentials belong");
    }

    #[tokio::test]
    async fn a_blob_redirect_back_to_the_registry_keeps_its_token() {
        let (registry_addr, registry_requests) = registry_with_token(|request| if request.starts_with("get /cdn/") { ok("layer-bytes") } else { redirect_to("/cdn/layer") }).await;
        let registry = HttpRemoteDockerRegistry::new().trusting_test_origins(&[registry_addr]);

        assert_eq!(blob_of(&registry, registry_addr, false).await.unwrap(), Some(b"layer-bytes".to_vec()));

        let registry_requests = registry_requests.lock().unwrap();
        assert!(registry_requests.last().unwrap().contains("authorization: bearer registry-secret"));
    }

    #[tokio::test]
    async fn a_blob_redirect_to_a_private_address_or_plain_http_is_refused() {
        for (target, reason) in [("https://127.0.0.1:1/layer", "private or reserved"), ("https://169.254.169.254/latest", "private or reserved"), ("http://cdn.example.com/layer", "not https")] {
            let (registry_addr, _) = registry_with_token(move |_| redirect_to(target)).await;
            let registry = HttpRemoteDockerRegistry::new().trusting_test_origins(&[registry_addr]);

            let error = blob_of(&registry, registry_addr, false).await.unwrap_err();

            assert!(error.to_string().contains(reason), "{target}: {error}");
        }
    }

    #[tokio::test]
    async fn a_blob_that_keeps_redirecting_is_given_up_on() {
        let (registry_addr, registry_requests) = registry_with_token(|_| redirect_to("/again")).await;
        let registry = HttpRemoteDockerRegistry::new().trusting_test_origins(&[registry_addr]);

        let error = blob_of(&registry, registry_addr, false).await.unwrap_err();

        assert!(error.to_string().contains("redirected more than"), "{error}");
        assert!(registry_requests.lock().unwrap().len() <= MAX_BLOB_REDIRECTS + 3);
    }

    #[tokio::test]
    async fn a_manifest_redirect_is_not_followed() {
        let (registry_addr, _) = registry_with_token(|_| redirect_to("http://127.0.0.1:1/")).await;
        let registry = HttpRemoteDockerRegistry::new().trusting_test_origins(&[registry_addr]);
        let name = DockerImageName::parse("library/app").unwrap();

        let error = registry.fetch_manifest(&format!("http://{registry_addr}"), &name, "latest", None, None).await.unwrap_err();

        assert!(error.to_string().contains("307"), "{error}");
    }

    /// Goes straight to the client, skipping the pre-flight check: what a rebinding host would face at connect time.
    #[tokio::test]
    async fn the_blob_client_refuses_a_name_that_resolves_to_a_private_address_at_connect_time() {
        let client = HttpRemoteDockerRegistry::new().blob_client;

        let error = client.get("https://localhost:1/").send().await.unwrap_err();

        let mut chain = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(cause) = source {
            chain.push_str(&cause.to_string());
            source = cause.source();
        }
        assert!(chain.contains("private or reserved"), "got: {chain}");
    }
}
