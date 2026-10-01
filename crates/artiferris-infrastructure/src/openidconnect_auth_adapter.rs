use async_trait::async_trait;
use chrono::{Duration, Utc};
use artiferris_domain::error::DomainError;
use artiferris_domain::sso::{ExternalIdentity, OidcAuthPort, OidcConfig};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreIdTokenClaims, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error_ext::InfraErr;
use crate::oidc_http_client::OidcHttpClient;
use crate::token_keys::derive_signing_key;

const OIDC_STATE_TOKEN_TYPE: &str = "oidc-state";
/// Generous enough for a slow identity-provider login screen, short enough that a captured but unused redirect URL stops being useful quickly.
const STATE_TOKEN_TTL_MINUTES: i64 = 10;

/// The typestate `CoreClient::from_provider_metadata` produces: set for what discovery always returns, maybe-set for
/// what it usually returns, not-set for the rest. It must be named, since the bare `CoreClient` alias defaults
/// everything to `EndpointNotSet`.
type OidcCoreClient = CoreClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointMaybeSet, EndpointMaybeSet>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateClaims {
    typ: String,
    exp: i64,
    organization_id: Uuid,
    nonce: String,
    /// SHA-256 of the browser's binding cookie — hashed so the state token, which travels through the identity provider's logs, never carries the raw cookie value.
    binding_hash: String,
}

pub struct OpenidConnectAuthAdapter {
    state_key: [u8; 32],
    #[cfg(test)]
    trusted_test_origin: Option<(String, u16)>,
}

/// The error and everything it wraps, since the identity-provider crate's own messages ("Request failed") leave out the cause.
fn describe(error: &dyn std::error::Error) -> DomainError {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    DomainError::Infrastructure(message)
}

impl OpenidConnectAuthAdapter {
    pub fn new(jwt_secret: String) -> Self {
        Self {
            state_key: derive_signing_key(&jwt_secret, OIDC_STATE_TOKEN_TYPE),
            #[cfg(test)]
            trusted_test_origin: None,
        }
    }

    #[cfg(test)]
    fn trusting_test_origin(mut self, host: &str, port: u16) -> Self {
        self.trusted_test_origin = Some((host.to_string(), port));
        self
    }

    fn binding_hash(binding_secret: &str) -> String {
        hex::encode(Sha256::digest(binding_secret.as_bytes()))
    }

    /// Derived from the browser's binding secret and the server key, not carried in `state` (which travels through the
    /// IdP, history and logs): nobody who sees the authorization URL can compute it, and nothing is stored server-side.
    fn pkce_verifier(&self, binding_secret: &str) -> PkceCodeVerifier {
        let mut verifier = [0u8; 32];
        hkdf::Hkdf::<Sha256>::new(Some(&self.state_key), binding_secret.as_bytes())
            .expand(b"artiferris-oidc-pkce-v1", &mut verifier)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        PkceCodeVerifier::new(hex::encode(verifier))
    }

    fn encode_state(&self, organization_id: Uuid, nonce: &Nonce, binding_secret: &str) -> Result<String, DomainError> {
        let claims = StateClaims {
            typ: OIDC_STATE_TOKEN_TYPE.to_string(),
            exp: (Utc::now() + Duration::minutes(STATE_TOKEN_TTL_MINUTES)).timestamp(),
            organization_id,
            nonce: nonce.secret().clone(),
            binding_hash: Self::binding_hash(binding_secret),
        };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(&self.state_key)).infra_err()
    }

    fn decode_state(&self, raw_state: &str, expected_organization_id: Uuid, expected_binding_secret: &str) -> Result<StateClaims, DomainError> {
        let data = decode::<StateClaims>(raw_state, &DecodingKey::from_secret(&self.state_key), &Validation::default()).infra_err()?;
        if data.claims.typ != OIDC_STATE_TOKEN_TYPE {
            return Err(DomainError::Infrastructure("not an oidc-state token".to_string()));
        }
        // A token minted for one organization must never complete a session for another.
        if data.claims.organization_id != expected_organization_id {
            return Err(DomainError::Infrastructure("oidc state token organization mismatch".to_string()));
        }
        // Only the browser that started this login can complete it — otherwise a captured callback URL would log the victim in from any browser.
        if data.claims.binding_hash != Self::binding_hash(expected_binding_secret) {
            return Err(DomainError::Infrastructure("oidc state token browser binding mismatch".to_string()));
        }
        Ok(data.claims)
    }

    fn http_client(&self) -> Result<OidcHttpClient, DomainError> {
        let client = OidcHttpClient::new()?;
        #[cfg(test)]
        let client = match &self.trusted_test_origin {
            Some((host, port)) => client.trusting_test_origin(host, *port),
            None => client,
        };
        Ok(client)
    }

    /// Every URL the discovery document hands back has to pass the same checks as the issuer, before anything is sent to it or the browser is redirected to it.
    async fn build_client(&self, config: &OidcConfig, callback_url: &str) -> Result<(OidcCoreClient, OidcHttpClient), DomainError> {
        let http_client = self.http_client()?;
        http_client.check_url(&config.issuer_url).await?;
        let issuer_url = IssuerUrl::new(config.issuer_url.clone()).infra_err()?;
        let metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client).await.map_err(|e| describe(&e))?;
        let discovered = [
            Some(metadata.authorization_endpoint().url()),
            metadata.token_endpoint().map(|url| url.url()),
            Some(metadata.jwks_uri().url()),
            metadata.userinfo_endpoint().map(|url| url.url()),
        ];
        for url in discovered.into_iter().flatten() {
            http_client.check_url(url.as_str()).await?;
        }
        let redirect_url = RedirectUrl::new(callback_url.to_string()).infra_err()?;
        let client = CoreClient::from_provider_metadata(metadata, ClientId::new(config.client_id.clone()), Some(ClientSecret::new(config.client_secret.clone())))
            .set_redirect_uri(redirect_url);
        Ok((client, http_client))
    }
}

#[async_trait]
impl OidcAuthPort for OpenidConnectAuthAdapter {
    async fn build_redirect(&self, config: &OidcConfig, organization_id: Uuid, callback_url: &str, binding_secret: &str) -> Result<String, DomainError> {
        let (client, _) = self.build_client(config, callback_url).await?;

        let pkce_challenge = PkceCodeChallenge::from_code_verifier_sha256(&self.pkce_verifier(binding_secret));
        let nonce = Nonce::new_random();
        let state_token = self.encode_state(organization_id, &nonce, binding_secret)?;

        let (auth_url, _csrf, _nonce) = client
            .authorize_url(CoreAuthenticationFlow::AuthorizationCode, move || CsrfToken::new(state_token.clone()), move || nonce.clone())
            .add_scope(openidconnect::Scope::new("openid".to_string()))
            .add_scope(openidconnect::Scope::new("email".to_string()))
            .set_pkce_challenge(pkce_challenge)
            .url();

        Ok(auth_url.to_string())
    }

    fn state_is_valid(&self, raw_state: &str, expected_organization_id: Uuid, binding_secret: &str) -> bool {
        self.decode_state(raw_state, expected_organization_id, binding_secret).is_ok()
    }

    async fn handle_callback(
        &self,
        config: &OidcConfig,
        code: &str,
        raw_state: &str,
        callback_url: &str,
        expected_organization_id: Uuid,
        binding_secret: &str,
    ) -> Result<ExternalIdentity, DomainError> {
        let claims = self.decode_state(raw_state, expected_organization_id, binding_secret)?;
        let (client, http_client) = self.build_client(config, callback_url).await?;

        let token_response = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .infra_err()?
            .set_pkce_verifier(self.pkce_verifier(binding_secret))
            .request_async(&http_client)
            .await
            .map_err(|e| describe(&e))?;

        let expected_nonce = Nonce::new(claims.nonce);
        let id_token = token_response.id_token().ok_or_else(|| DomainError::Infrastructure("oidc token response had no id_token".to_string()))?;
        let id_token_verifier = client.id_token_verifier();
        let id_token_claims = id_token.claims(&id_token_verifier, &expected_nonce).infra_err()?;

        external_identity_from_claims(id_token_claims)
    }
}

/// Split out of `handle_callback` so it can be unit-tested without a live identity provider.
fn external_identity_from_claims(id_token_claims: &CoreIdTokenClaims) -> Result<ExternalIdentity, DomainError> {
    let email = id_token_claims
        .email()
        .ok_or_else(|| DomainError::Infrastructure("oidc id token missing the email claim".to_string()))?
        .to_string();

    // Provisioning matches accounts by email, so an unverified claim would let a self-asserted email log someone in as a colleague. Absent counts as unverified.
    if id_token_claims.email_verified() != Some(true) {
        return Err(DomainError::Infrastructure("oidc identity provider did not verify the user's email address".to_string()));
    }

    Ok(ExternalIdentity { email, display_name: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openidconnect::{Audience, EmptyAdditionalClaims, EndUserEmail, StandardClaims, SubjectIdentifier};

    const TEST_BINDING: &str = "test-browser-binding-secret";

    #[test]
    fn a_state_token_round_trips_its_claims() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let organization_id = Uuid::new_v4();
        let nonce = Nonce::new("test-nonce".to_string());

        let token = adapter.encode_state(organization_id, &nonce, TEST_BINDING).unwrap();
        let claims = adapter.decode_state(&token, organization_id, TEST_BINDING).unwrap();

        assert_eq!(claims.organization_id, organization_id);
        assert_eq!(claims.nonce, "test-nonce");
    }

    #[test]
    fn the_state_token_does_not_carry_the_pkce_verifier() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let organization_id = Uuid::new_v4();
        let token = adapter.encode_state(organization_id, &Nonce::new("test-nonce".to_string()), TEST_BINDING).unwrap();

        let payload = decode::<serde_json::Value>(&token, &DecodingKey::from_secret(&adapter.state_key), &Validation::default()).unwrap().claims.to_string();

        assert!(!payload.contains("pkce"), "got: {payload}");
        assert!(!payload.contains(adapter.pkce_verifier(TEST_BINDING).secret()), "got: {payload}");
    }

    #[test]
    fn the_pkce_verifier_is_reproducible_only_with_the_same_server_key_and_browser_binding() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let verifier = adapter.pkce_verifier(TEST_BINDING);

        assert_eq!(verifier.secret(), adapter.pkce_verifier(TEST_BINDING).secret());
        assert_ne!(verifier.secret(), adapter.pkce_verifier("another-browser").secret());
        assert_ne!(verifier.secret(), OpenidConnectAuthAdapter::new("other-secret".to_string()).pkce_verifier(TEST_BINDING).secret());
        assert!((43..=128).contains(&verifier.secret().len()), "RFC 7636 verifier length, got {}", verifier.secret().len());
        assert!(verifier.secret().chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn a_state_token_minted_for_one_organization_is_rejected_for_another() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let nonce = Nonce::new("test-nonce".to_string());
        let token = adapter.encode_state(Uuid::new_v4(), &nonce, TEST_BINDING).unwrap();

        let result = adapter.decode_state(&token, Uuid::new_v4(), TEST_BINDING);

        assert!(result.is_err(), "a state token must not verify against a different organization id than the one it was minted for");
    }

    #[test]
    fn a_state_token_bound_to_one_browser_is_rejected_for_a_different_browser() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let organization_id = Uuid::new_v4();
        let nonce = Nonce::new("test-nonce".to_string());
        let token = adapter.encode_state(organization_id, &nonce, "browser-a-binding").unwrap();

        let result = adapter.decode_state(&token, organization_id, "browser-b-binding");

        assert!(result.is_err(), "a state token must not verify for a browser other than the one that started the login attempt");
    }

    /// Built by hand rather than via `encode_state` so the claims carry an already-expired `exp`, well past the 60s clock-skew leeway `Validation::default()` allows.
    #[test]
    fn a_state_token_past_its_expiry_is_rejected() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let organization_id = Uuid::new_v4();
        let claims = StateClaims {
            typ: OIDC_STATE_TOKEN_TYPE.to_string(),
            exp: (Utc::now() - Duration::minutes(30)).timestamp(),
            organization_id,
            nonce: "test-nonce".to_string(),
            binding_hash: OpenidConnectAuthAdapter::binding_hash(TEST_BINDING),
        };
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(&adapter.state_key)).unwrap();

        let result = adapter.decode_state(&token, organization_id, TEST_BINDING);

        assert!(result.is_err(), "a state token past its exp must be rejected even if otherwise validly signed, org-scoped and browser-bound");
    }

    #[test]
    fn a_state_token_signed_with_a_different_secret_is_rejected() {
        let adapter_a = OpenidConnectAuthAdapter::new("secret-a".to_string());
        let adapter_b = OpenidConnectAuthAdapter::new("secret-b".to_string());
        let organization_id = Uuid::new_v4();
        let nonce = Nonce::new("test-nonce".to_string());
        let token = adapter_a.encode_state(organization_id, &nonce, TEST_BINDING).unwrap();

        assert!(adapter_b.decode_state(&token, organization_id, TEST_BINDING).is_err());
    }

    fn claims_with(email: Option<&str>, email_verified: Option<bool>) -> CoreIdTokenClaims {
        let mut standard = StandardClaims::new(SubjectIdentifier::new("subject-1".to_string()));
        standard = standard.set_email(email.map(|e| EndUserEmail::new(e.to_string()))).set_email_verified(email_verified);
        CoreIdTokenClaims::new(
            IssuerUrl::new("https://accounts.example.com".to_string()).unwrap(),
            vec![Audience::new("artiferris".to_string())],
            Utc::now() + Duration::minutes(5),
            Utc::now(),
            standard,
            EmptyAdditionalClaims {},
        )
    }

    #[test]
    fn a_verified_email_claim_yields_an_external_identity() {
        let identity = external_identity_from_claims(&claims_with(Some("florian@corp.example"), Some(true))).unwrap();

        assert_eq!(identity.email, "florian@corp.example");
    }

    #[test]
    fn an_unverified_email_claim_is_rejected() {
        let err = external_identity_from_claims(&claims_with(Some("florian@corp.example"), Some(false))).unwrap_err();

        assert!(err.to_string().contains("did not verify"), "got: {err}");
    }

    #[test]
    fn an_absent_email_verified_claim_is_rejected_like_an_unverified_one() {
        let err = external_identity_from_claims(&claims_with(Some("florian@corp.example"), None)).unwrap_err();

        assert!(err.to_string().contains("did not verify"), "got: {err}");
    }

    #[test]
    fn a_missing_email_claim_is_rejected() {
        let err = external_identity_from_claims(&claims_with(None, Some(true))).unwrap_err();

        assert!(err.to_string().contains("missing the email claim"), "got: {err}");
    }

    #[tokio::test]
    async fn an_issuer_url_pointing_at_a_private_address_is_rejected_before_discovery() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let config = OidcConfig { issuer_url: "https://127.0.0.1:1".to_string(), client_id: "artiferris".to_string(), client_secret: "s3cret!".to_string() };

        let err = adapter
            .build_redirect(&config, Uuid::new_v4(), "https://acme.artiferris.example/api/auth/sso/oidc/callback", TEST_BINDING)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn an_issuer_url_pointing_at_the_cloud_metadata_endpoint_is_rejected() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let config = OidcConfig { issuer_url: "https://169.254.169.254/".to_string(), client_id: "artiferris".to_string(), client_secret: "s3cret!".to_string() };

        let err = adapter
            .build_redirect(&config, Uuid::new_v4(), "https://acme.artiferris.example/api/auth/sso/oidc/callback", TEST_BINDING)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    const CALLBACK: &str = "https://acme.artiferris.example/api/auth/sso/oidc/callback";

    /// Endpoints of a fake provider's discovery document; `None` points one at the fake provider itself.
    #[derive(Default)]
    struct Endpoints {
        authorization: Option<&'static str>,
        token: Option<&'static str>,
        jwks: Option<&'static str>,
    }

    /// A provider on loopback, the one origin the adapter is told to trust, whose discovery document points its endpoints wherever the test says.
    async fn fake_provider(endpoints: Endpoints) -> (wiremock::MockServer, OpenidConnectAuthAdapter, OidcConfig) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let own = |path: &str| format!("{}{path}", server.uri());
        let document = serde_json::json!({
            "issuer": server.uri(),
            "authorization_endpoint": endpoints.authorization.map(str::to_string).unwrap_or_else(|| own("/authorize")),
            "token_endpoint": endpoints.token.map(str::to_string).unwrap_or_else(|| own("/token")),
            "jwks_uri": endpoints.jwks.map(str::to_string).unwrap_or_else(|| own("/jwks")),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
        });
        Mock::given(method("GET")).and(path("/.well-known/openid-configuration")).respond_with(ResponseTemplate::new(200).set_body_json(document)).mount(&server).await;
        Mock::given(method("GET")).and(path("/jwks")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "keys": [] }))).mount(&server).await;
        let address = server.address();
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string()).trusting_test_origin(&address.ip().to_string(), address.port());
        let config = OidcConfig { issuer_url: server.uri(), client_id: "artiferris".to_string(), client_secret: "s3cret!".to_string() };
        (server, adapter, config)
    }

    #[tokio::test]
    async fn a_provider_whose_endpoints_all_sit_on_a_trusted_origin_starts_a_login() {
        let (_server, adapter, config) = fake_provider(Endpoints::default()).await;

        let url = adapter.build_redirect(&config, Uuid::new_v4(), CALLBACK, TEST_BINDING).await.unwrap();

        assert!(url.contains("/authorize?"), "got: {url}");
    }

    #[tokio::test]
    async fn a_discovery_document_pointing_the_key_set_at_a_private_address_is_refused() {
        for jwks in ["https://127.0.0.1:1/jwks", "https://10.0.0.5/jwks", "https://169.254.169.254/latest/meta-data"] {
            let (_server, adapter, config) = fake_provider(Endpoints { jwks: Some(jwks), ..Default::default() }).await;

            let err = adapter.build_redirect(&config, Uuid::new_v4(), CALLBACK, TEST_BINDING).await.unwrap_err();

            assert!(err.to_string().contains("private or reserved"), "{jwks}: {err}");
        }
    }

    #[tokio::test]
    async fn a_discovery_document_pointing_the_token_endpoint_at_a_private_address_is_refused() {
        let (_server, adapter, config) = fake_provider(Endpoints { token: Some("https://10.0.0.5/token"), ..Default::default() }).await;

        let err = adapter.build_redirect(&config, Uuid::new_v4(), CALLBACK, TEST_BINDING).await.unwrap_err();

        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn a_discovery_document_sending_the_browser_to_a_private_authorization_endpoint_is_refused() {
        let (_server, adapter, config) = fake_provider(Endpoints { authorization: Some("https://192.168.1.1/authorize"), ..Default::default() }).await;

        let err = adapter.build_redirect(&config, Uuid::new_v4(), CALLBACK, TEST_BINDING).await.unwrap_err();

        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn a_discovery_document_with_a_plain_http_endpoint_is_refused() {
        let (_server, adapter, config) = fake_provider(Endpoints { token: Some("http://idp.example.com/token"), ..Default::default() }).await;

        let err = adapter.build_redirect(&config, Uuid::new_v4(), CALLBACK, TEST_BINDING).await.unwrap_err();

        assert!(err.to_string().contains("must use https"), "got: {err}");
    }

    #[tokio::test]
    async fn a_plain_http_issuer_is_refused() {
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string());
        let config = OidcConfig { issuer_url: "http://idp.example.com".to_string(), client_id: "artiferris".to_string(), client_secret: "s3cret!".to_string() };

        let err = adapter.build_redirect(&config, Uuid::new_v4(), CALLBACK, TEST_BINDING).await.unwrap_err();

        assert!(err.to_string().contains("must use https"), "got: {err}");
    }

    #[tokio::test]
    async fn an_oversized_discovery_document_is_refused() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/.well-known/openid-configuration")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b' '; 2 * 1024 * 1024])).mount(&server).await;
        let address = server.address();
        let adapter = OpenidConnectAuthAdapter::new("jwt-secret".to_string()).trusting_test_origin(&address.ip().to_string(), address.port());
        let config = OidcConfig { issuer_url: server.uri(), client_id: "artiferris".to_string(), client_secret: "s3cret!".to_string() };

        let err = adapter.build_redirect(&config, Uuid::new_v4(), CALLBACK, TEST_BINDING).await.unwrap_err();

        assert!(err.to_string().contains("byte limit"), "got: {err}");
    }
}
