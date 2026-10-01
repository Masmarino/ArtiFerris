use async_trait::async_trait;
use uuid::Uuid;

use crate::audit::AdminAuditRecord;
use crate::error::DomainError;

/// Deliberately minimal — never a role/group/admin claim, so a compromised IdP can't escalate a JIT-provisioned account beyond a plain member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIdentity {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityProviderConfig {
    Ldap(LdapConfig),
    Oidc(OidcConfig),
}

#[derive(Clone, PartialEq, Eq)]
pub struct LdapConfig {
    /// e.g. `ldap://dc.corp.example:389` or `ldaps://dc.corp.example:636`.
    pub server_url: String,
    pub bind_dn: String,
    /// Plaintext in memory; encrypted at rest via `secret_box` in `artiferris-infrastructure`.
    pub bind_password: String,
    pub user_search_base: String,
    /// e.g. `(uid={username})` — `{username}` is substituted verbatim, so the `LdapAuthPort` adapter must escape it against LDAP filter injection.
    pub user_search_filter: String,
    pub email_attribute: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OidcConfig {
    /// e.g. `https://accounts.example.com` — discovery reads `{issuer_url}/.well-known/openid-configuration`.
    pub issuer_url: String,
    pub client_id: String,
    /// Plaintext in memory; encrypted at rest the same way as `LdapConfig::bind_password`.
    pub client_secret: String,
}

impl std::fmt::Debug for LdapConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LdapConfig")
            .field("server_url", &self.server_url)
            .field("bind_dn", &self.bind_dn)
            .field("bind_password", &"[redacted]")
            .field("user_search_base", &self.user_search_base)
            .field("user_search_filter", &self.user_search_filter)
            .field("email_attribute", &self.email_attribute)
            .finish()
    }
}

impl std::fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcConfig").field("issuer_url", &self.issuer_url).field("client_id", &self.client_id).field("client_secret", &"[redacted]").finish()
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_secrets() {
        let ldap = LdapConfig {
            server_url: "ldaps://dc".to_string(),
            bind_dn: "cn=svc".to_string(),
            bind_password: "ldap-secret-value".to_string(),
            user_search_base: "ou=people".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        };
        let oidc = OidcConfig { issuer_url: "https://idp".to_string(), client_id: "client".to_string(), client_secret: "oidc-secret-value".to_string() };

        let printed = format!("{ldap:?} {oidc:?} {:?}", IdentityProviderConfig::Ldap(ldap.clone()));

        assert!(!printed.contains("secret-value"), "got: {printed}");
        assert!(printed.contains("ldaps://dc") && printed.contains("client"), "the non-secret fields stay visible: {printed}");
    }
}

#[async_trait]
pub trait LdapAuthPort: Send + Sync {
    /// Binds as the service account, searches for one matching entry, then re-binds with its DN and the submitted password. Every failure mode collapses to the same `Err`.
    async fn authenticate(&self, config: &LdapConfig, username: &str, password: &str) -> Result<ExternalIdentity, DomainError>;
}

#[async_trait]
pub trait OidcAuthPort: Send + Sync {
    /// Returns the redirect URL. The nonce and PKCE verifier live in a signed, self-contained `state` token (no server
    /// session), which `handle_callback` verifies. `binding_secret` is a fresh value also given to the browser as a
    /// cookie (login-CSRF defense, RFC 6749 §10.12): without it a captured callback URL would work in any browser.
    async fn build_redirect(&self, config: &OidcConfig, organization_id: Uuid, callback_url: &str, binding_secret: &str) -> Result<String, DomainError>;

    /// Whether `raw_state` is a state this server minted for `expected_organization_id` and this browser, and still unexpired. Needs no network, so a callback that fails it costs nothing.
    fn state_is_valid(&self, raw_state: &str, expected_organization_id: Uuid, binding_secret: &str) -> bool;

    /// `expected_organization_id` must match the id embedded in `raw_state`, and `binding_secret` must match the one the token was minted with — either mismatch fails closed.
    async fn handle_callback(
        &self,
        config: &OidcConfig,
        code: &str,
        raw_state: &str,
        callback_url: &str,
        expected_organization_id: Uuid,
        binding_secret: &str,
    ) -> Result<ExternalIdentity, DomainError>;
}

#[async_trait]
pub trait IdentityProviderRepositoryPort: Send + Sync {
    /// `None` means the organization uses local accounts only.
    async fn get(&self, organization_id: Uuid) -> Result<Option<IdentityProviderConfig>, DomainError>;
    /// Replaces any existing configuration for this organization (one active provider at a time). `audit` goes in the same transaction.
    async fn set(&self, organization_id: Uuid, config: &IdentityProviderConfig, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError>;
    /// Idempotent: clearing an organization with no configuration is a no-op, not an error. `audit` goes in the same transaction.
    async fn clear(&self, organization_id: Uuid, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> LdapConfig {
        LdapConfig {
            server_url: "ldap://dc.corp.example:389".to_string(),
            bind_dn: "cn=service,dc=corp,dc=example".to_string(),
            bind_password: "s3cret!".to_string(),
            user_search_base: "ou=people,dc=corp,dc=example".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        }
    }

    #[test]
    fn identity_provider_config_wraps_an_ldap_config_by_value() {
        let config = IdentityProviderConfig::Ldap(sample_config());
        let IdentityProviderConfig::Ldap(inner) = config else { panic!("expected Ldap variant") };
        assert_eq!(inner.server_url, "ldap://dc.corp.example:389");
    }

    #[test]
    fn external_identity_carries_email_and_optional_display_name() {
        let identity = ExternalIdentity { email: "florian@corp.example".to_string(), display_name: Some("Florian".to_string()) };
        assert_eq!(identity.email, "florian@corp.example");
        assert_eq!(identity.display_name.as_deref(), Some("Florian"));
    }

    #[test]
    fn identity_provider_config_wraps_an_oidc_config_by_value() {
        let config = IdentityProviderConfig::Oidc(OidcConfig {
            issuer_url: "https://accounts.example.com".to_string(),
            client_id: "artiferris".to_string(),
            client_secret: "s3cret!".to_string(),
        });
        let IdentityProviderConfig::Oidc(inner) = config else { panic!("expected Oidc variant") };
        assert_eq!(inner.issuer_url, "https://accounts.example.com");
    }
}
