//! The HTTP client behind every identity-provider request (discovery, key set, token exchange).
//! The discovery document is written by the provider, so `jwks_uri` and `token_endpoint` are as untrusted as the
//! issuer: each request is checked right before it is sent, must be https, and is read with a size cap. Names are
//! resolved through a resolver that drops private addresses, so a rebinding host is refused at connect time too.

use std::future::Future;
use std::pin::Pin;

use artiferris_domain::error::DomainError;
use openidconnect::reqwest::dns::{Addrs, Name, Resolve, Resolving};
use openidconnect::{AsyncHttpClient, HttpClientError, HttpRequest, HttpResponse};

use crate::error_ext::InfraErr;

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// A discovery document, a key set or a token response is a few KiB.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// `openidconnect` builds on another `reqwest` than the rest of this crate, so it needs its own resolver type.
struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move { Ok(Box::new(crate::ssrf_guard::resolve_public_addresses(&host).await?.into_iter()) as Addrs) })
    }
}

pub struct OidcHttpClient {
    inner: openidconnect::reqwest::Client,
    /// The origin of a test's fake identity provider, the only place plain http or a loopback address is accepted.
    #[cfg(test)]
    trusted_test_origin: Option<(String, u16)>,
}

impl OidcHttpClient {
    pub fn new() -> Result<Self, DomainError> {
        Self::with_timeouts(CONNECT_TIMEOUT, TOTAL_TIMEOUT)
    }

    /// Redirects are off: following one would send the request somewhere the checks never saw.
    fn with_timeouts(connect: std::time::Duration, total: std::time::Duration) -> Result<Self, DomainError> {
        let inner = openidconnect::reqwest::ClientBuilder::new()
            .redirect(openidconnect::reqwest::redirect::Policy::none())
            .connect_timeout(connect)
            .timeout(total)
            .dns_resolver(std::sync::Arc::new(PublicOnlyResolver))
            .build()
            .infra_err()?;
        Ok(Self {
            inner,
            #[cfg(test)]
            trusted_test_origin: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn trusting_test_origin(mut self, host: &str, port: u16) -> Self {
        self.trusted_test_origin = Some((host.to_string(), port));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_timeouts(connect: std::time::Duration, total: std::time::Duration) -> Result<Self, DomainError> {
        Self::with_timeouts(connect, total)
    }

    /// https, and a host that resolves to public addresses (or ones the operator allow-listed).
    pub async fn check_url(&self, url: &str) -> Result<(), DomainError> {
        let parsed = reqwest::Url::parse(url).map_err(|e| DomainError::Infrastructure(format!("invalid identity provider URL {url}: {e}")))?;
        #[cfg(test)]
        if let (Some((host, port)), Some(url_host)) = (&self.trusted_test_origin, parsed.host_str()) {
            if url_host == host && parsed.port_or_known_default() == Some(*port) {
                return Ok(());
            }
        }
        if parsed.scheme() != "https" {
            return Err(DomainError::Infrastructure(format!("identity provider URL {url} must use https")));
        }
        crate::ssrf_guard::ensure_public_host(url).await
    }

    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpClientError<openidconnect::reqwest::Error>> {
        self.check_url(&request.uri().to_string()).await.map_err(|e| HttpClientError::Other(e.to_string()))?;

        let mut response = self.inner.execute(request.try_into().map_err(Box::new)?).await.map_err(Box::new)?;
        if response.content_length().is_some_and(|length| length > MAX_RESPONSE_BYTES as u64) {
            return Err(too_large());
        }
        let mut builder = openidconnect::http::Response::builder().status(response.status()).version(response.version());
        for (name, value) in response.headers() {
            builder = builder.header(name, value);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(Box::new)? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        builder.body(body).map_err(HttpClientError::Http)
    }
}

fn too_large() -> HttpClientError<openidconnect::reqwest::Error> {
    HttpClientError::Other(format!("identity provider response exceeds the {MAX_RESPONSE_BYTES}-byte limit"))
}

impl<'c> AsyncHttpClient<'c> for OidcHttpClient {
    type Error = HttpClientError<openidconnect::reqwest::Error>;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + Sync + 'c>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(self.send(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_identity_provider_that_accepts_the_connection_but_never_answers_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let client = OidcHttpClient::with_test_timeouts(std::time::Duration::from_secs(5), std::time::Duration::from_millis(300)).unwrap().trusting_test_origin("127.0.0.1", port);
        let request = openidconnect::http::Request::builder().uri(format!("http://127.0.0.1:{port}/")).body(Vec::new()).unwrap();

        let started = std::time::Instant::now();
        let result = client.call(request).await;

        assert!(matches!(result, Err(HttpClientError::Reqwest(e)) if e.is_timeout()));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    /// Goes straight to the inner client, skipping the pre-flight check: what a rebinding host would face at connect time.
    #[tokio::test]
    async fn the_connection_itself_refuses_a_name_that_resolves_to_a_private_address() {
        let client = OidcHttpClient::new().unwrap();

        let error = client.inner.get("https://localhost:1/").send().await.unwrap_err();

        let mut chain = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(cause) = source {
            chain.push_str(&cause.to_string());
            source = cause.source();
        }
        assert!(chain.contains("private or reserved"), "got: {chain}");
    }
}
