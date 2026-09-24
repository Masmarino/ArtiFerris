use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::email::{SmtpSecurity, SmtpSettings};
use crate::error::EventStoreError;
use crate::sso::IdentityProviderConfig;
use crate::system_settings::SystemSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginMethod {
    Password,
    Ldap,
    Oidc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MfaMethod {
    Totp,
    BackupCode,
    Passkey,
}

pub const MAX_RECORDED_USERNAME_CHARS: usize = 64;

/// Unicode format characters (category Cf: bidi overrides and isolates, zero-width marks, soft hyphen). They render as nothing or reorder the text around them.
fn is_format_character(c: char) -> bool {
    matches!(u32::from(c),
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2 | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

/// What a failed login stores of the typed name: format characters dropped, control characters shown as `?`. A password pasted into the username field still ends up in the log.
pub fn recorded_username(raw: &str) -> String {
    raw.trim().chars().filter(|c| !is_format_character(*c)).take(MAX_RECORDED_USERNAME_CHARS).map(|c| if c.is_control() { '?' } else { c }).collect()
}

/// Authentication and credential events. Events without an `organization_id` of their own are attributed to the actor's organization when stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum SecurityEvent {
    /// `username` is what was typed, through `recorded_username`.
    LoginFailed { username: String, ip: String },
    AccessDenied { user_id: Uuid, repository_id: Uuid, action: String },
    PasswordChangeFailed { username: String, ip: String },
    MfaVerificationFailed { user_id: Uuid, method: String },
    PasskeyVerificationFailed { user_id: Uuid },
    OidcLoginFailed { organization_id: Uuid },
    DockerTokenFailed { ip: String },
    /// `second_factor` is what completed a local login (or the factor just enrolled during mandatory setup); `None` for SSO.
    LoginSucceeded { user_id: Uuid, organization_id: Uuid, method: LoginMethod, second_factor: Option<MfaMethod> },
    PasswordChanged { user_id: Uuid, organization_id: Uuid },
    MfaEnabled { user_id: Uuid, organization_id: Uuid, method: MfaMethod },
    MfaDisabled { user_id: Uuid, organization_id: Uuid, method: MfaMethod },
    PasskeyAdded { user_id: Uuid, organization_id: Uuid, passkey_id: Uuid },
    PasskeyDeleted { user_id: Uuid, organization_id: Uuid, passkey_id: Uuid },
    /// `user_id` owns the token; whoever revoked it is the event's actor.
    ApiTokenCreated { user_id: Uuid, organization_id: Uuid, token_id: Uuid, label: String },
    ApiTokenRevoked { user_id: Uuid, organization_id: Uuid, token_id: Uuid },
    SessionsRevoked { user_id: Uuid, organization_id: Uuid },
    BackupCodesRegenerated { user_id: Uuid, organization_id: Uuid },
}

impl SecurityEvent {
    /// The form that gets stored.
    pub fn normalized(self) -> Self {
        match self {
            SecurityEvent::LoginFailed { username, ip } => SecurityEvent::LoginFailed { username: recorded_username(&username), ip },
            other => other,
        }
    }

    pub fn event_type(&self) -> &'static str {
        match self {
            SecurityEvent::LoginFailed { .. } => "LoginFailed",
            SecurityEvent::AccessDenied { .. } => "AccessDenied",
            SecurityEvent::PasswordChangeFailed { .. } => "PasswordChangeFailed",
            SecurityEvent::MfaVerificationFailed { .. } => "MfaVerificationFailed",
            SecurityEvent::PasskeyVerificationFailed { .. } => "PasskeyVerificationFailed",
            SecurityEvent::OidcLoginFailed { .. } => "OidcLoginFailed",
            SecurityEvent::DockerTokenFailed { .. } => "DockerTokenFailed",
            SecurityEvent::LoginSucceeded { .. } => "LoginSucceeded",
            SecurityEvent::PasswordChanged { .. } => "PasswordChanged",
            SecurityEvent::MfaEnabled { .. } => "MfaEnabled",
            SecurityEvent::MfaDisabled { .. } => "MfaDisabled",
            SecurityEvent::PasskeyAdded { .. } => "PasskeyAdded",
            SecurityEvent::PasskeyDeleted { .. } => "PasskeyDeleted",
            SecurityEvent::ApiTokenCreated { .. } => "ApiTokenCreated",
            SecurityEvent::ApiTokenRevoked { .. } => "ApiTokenRevoked",
            SecurityEvent::SessionsRevoked { .. } => "SessionsRevoked",
            SecurityEvent::BackupCodesRegenerated { .. } => "BackupCodesRegenerated",
        }
    }

    /// `None` for events recorded before anyone is identified.
    pub fn organization_id(&self) -> Option<Uuid> {
        match self {
            SecurityEvent::OidcLoginFailed { organization_id }
            | SecurityEvent::LoginSucceeded { organization_id, .. }
            | SecurityEvent::PasswordChanged { organization_id, .. }
            | SecurityEvent::MfaEnabled { organization_id, .. }
            | SecurityEvent::MfaDisabled { organization_id, .. }
            | SecurityEvent::PasskeyAdded { organization_id, .. }
            | SecurityEvent::PasskeyDeleted { organization_id, .. }
            | SecurityEvent::ApiTokenCreated { organization_id, .. }
            | SecurityEvent::ApiTokenRevoked { organization_id, .. }
            | SecurityEvent::SessionsRevoked { organization_id, .. }
            | SecurityEvent::BackupCodesRegenerated { organization_id, .. } => Some(*organization_id),
            SecurityEvent::LoginFailed { .. }
            | SecurityEvent::AccessDenied { .. }
            | SecurityEvent::PasswordChangeFailed { .. }
            | SecurityEvent::MfaVerificationFailed { .. }
            | SecurityEvent::PasskeyVerificationFailed { .. }
            | SecurityEvent::DockerTokenFailed { .. } => None,
        }
    }
}

/// The non-secret shape of an identity provider: enough to see what changed, never the client secret or bind password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum IdentityProviderSummary {
    Oidc { issuer_url: String, client_id: String },
    Ldap { server_url: String, bind_dn: String, user_search_base: String, user_search_filter: String, email_attribute: String },
}

/// Drops userinfo, query and fragment: the trail keeps where a URL points, not what it carries.
fn without_credentials(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut parsed) if !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() || parsed.fragment().is_some() => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        }
        _ => raw.to_string(),
    }
}

impl From<&IdentityProviderConfig> for IdentityProviderSummary {
    fn from(config: &IdentityProviderConfig) -> Self {
        match config {
            IdentityProviderConfig::Oidc(oidc) => IdentityProviderSummary::Oidc { issuer_url: without_credentials(&oidc.issuer_url), client_id: oidc.client_id.clone() },
            IdentityProviderConfig::Ldap(ldap) => IdentityProviderSummary::Ldap {
                server_url: without_credentials(&ldap.server_url),
                bind_dn: ldap.bind_dn.clone(),
                user_search_base: ldap.user_search_base.clone(),
                user_search_filter: ldap.user_search_filter.clone(),
                email_attribute: ldap.email_attribute.clone(),
            },
        }
    }
}

/// Everything about an organization's SMTP settings except the password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpSettingsSummary {
    pub host: String,
    pub port: i32,
    pub username: String,
    pub from_name: String,
    pub from_address: String,
    pub security: SmtpSecurity,
}

impl From<&SmtpSettings> for SmtpSettingsSummary {
    fn from(settings: &SmtpSettings) -> Self {
        Self {
            host: settings.host.clone(),
            port: settings.port,
            username: settings.username.clone(),
            from_name: settings.from_name.clone(),
            from_address: settings.from_address.clone(),
            security: settings.security,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingChange {
    pub setting: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
}

/// One entry per setting whose value differs, sorted by name.
pub fn system_settings_changes(before: &SystemSettings, after: &SystemSettings) -> Vec<SettingChange> {
    let as_map = |settings: &SystemSettings| -> BTreeMap<String, serde_json::Value> {
        match serde_json::to_value(settings) {
            Ok(serde_json::Value::Object(fields)) => fields.into_iter().collect(),
            _ => BTreeMap::new(),
        }
    };
    let (before, after) = (as_map(before), as_map(after));
    after
        .into_iter()
        .filter_map(|(setting, after)| {
            let before = before.get(&setting).cloned().unwrap_or(serde_json::Value::Null);
            (before != after).then_some(SettingChange { setting, before, after })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrandingAsset {
    Logo,
    Favicon,
}

/// Privilege and configuration changes made by administrators (and account activation).
/// Payloads carry ids, names and non-secret settings only, never passwords, tokens or client secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum AdminAuditEvent {
    UserInvited { user_id: Uuid, organization_id: Uuid, username: String, is_organization_admin: bool, is_super_admin: bool },
    UserActivated { user_id: Uuid, organization_id: Uuid },
    InvitationResent { user_id: Uuid, organization_id: Uuid },
    UserDeleted { user_id: Uuid, organization_id: Uuid, username: String },
    SuperAdminGranted { user_id: Uuid, organization_id: Uuid },
    SuperAdminRevoked { user_id: Uuid, organization_id: Uuid },
    OrganizationAdminGranted { user_id: Uuid, organization_id: Uuid },
    OrganizationAdminRevoked { user_id: Uuid, organization_id: Uuid },
    OrganizationCreated { organization_id: Uuid, slug: String, display_name: String },
    /// `organization_id` is the account's own, `None` when nobody by that name exists (a super-admin can still lift a lock on a name).
    LoginThrottleCleared { organization_id: Option<Uuid>, username: String },
    /// `secret_changed` tells whether a new client secret / bind password was supplied; its value is never recorded.
    IdentityProviderSet { organization_id: Uuid, before: Option<IdentityProviderSummary>, after: IdentityProviderSummary, secret_changed: bool },
    IdentityProviderCleared { organization_id: Uuid, before: Option<IdentityProviderSummary> },
    SmtpSettingsChanged { organization_id: Uuid, before: Option<SmtpSettingsSummary>, after: SmtpSettingsSummary, password_changed: bool },
    SystemSettingsChanged { organization_id: Uuid, changes: Vec<SettingChange> },
    BrandingChanged { organization_id: Uuid, asset: BrandingAsset, cleared: bool },
    /// Instance-wide: the export covers every user, so it belongs to no organization.
    ConfigurationExported { users: usize, repositories: usize, permissions: usize },
    ConfigurationImported { users_created: usize, repositories_created: usize, permissions_granted: usize, failures: usize },
}

impl AdminAuditEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            AdminAuditEvent::UserInvited { .. } => "UserInvited",
            AdminAuditEvent::UserActivated { .. } => "UserActivated",
            AdminAuditEvent::InvitationResent { .. } => "InvitationResent",
            AdminAuditEvent::UserDeleted { .. } => "UserDeleted",
            AdminAuditEvent::SuperAdminGranted { .. } => "SuperAdminGranted",
            AdminAuditEvent::SuperAdminRevoked { .. } => "SuperAdminRevoked",
            AdminAuditEvent::OrganizationAdminGranted { .. } => "OrganizationAdminGranted",
            AdminAuditEvent::OrganizationAdminRevoked { .. } => "OrganizationAdminRevoked",
            AdminAuditEvent::OrganizationCreated { .. } => "OrganizationCreated",
            AdminAuditEvent::LoginThrottleCleared { .. } => "LoginThrottleCleared",
            AdminAuditEvent::IdentityProviderSet { .. } => "IdentityProviderSet",
            AdminAuditEvent::IdentityProviderCleared { .. } => "IdentityProviderCleared",
            AdminAuditEvent::SmtpSettingsChanged { .. } => "SmtpSettingsChanged",
            AdminAuditEvent::SystemSettingsChanged { .. } => "SystemSettingsChanged",
            AdminAuditEvent::BrandingChanged { .. } => "BrandingChanged",
            AdminAuditEvent::ConfigurationExported { .. } => "ConfigurationExported",
            AdminAuditEvent::ConfigurationImported { .. } => "ConfigurationImported",
        }
    }

    /// `None` for instance-wide events, which only the super-admin's unscoped view shows.
    pub fn organization_id(&self) -> Option<Uuid> {
        match self {
            AdminAuditEvent::UserInvited { organization_id, .. }
            | AdminAuditEvent::UserActivated { organization_id, .. }
            | AdminAuditEvent::InvitationResent { organization_id, .. }
            | AdminAuditEvent::UserDeleted { organization_id, .. }
            | AdminAuditEvent::SuperAdminGranted { organization_id, .. }
            | AdminAuditEvent::SuperAdminRevoked { organization_id, .. }
            | AdminAuditEvent::OrganizationAdminGranted { organization_id, .. }
            | AdminAuditEvent::OrganizationAdminRevoked { organization_id, .. }
            | AdminAuditEvent::OrganizationCreated { organization_id, .. }
            | AdminAuditEvent::IdentityProviderSet { organization_id, .. }
            | AdminAuditEvent::IdentityProviderCleared { organization_id, .. }
            | AdminAuditEvent::SmtpSettingsChanged { organization_id, .. }
            | AdminAuditEvent::SystemSettingsChanged { organization_id, .. }
            | AdminAuditEvent::BrandingChanged { organization_id, .. } => Some(*organization_id),
            AdminAuditEvent::LoginThrottleCleared { organization_id, .. } => *organization_id,
            AdminAuditEvent::ConfigurationExported { .. } | AdminAuditEvent::ConfigurationImported { .. } => None,
        }
    }
}

/// An admin event bundled with its actor, for a repository that writes it in the same transaction as the change it describes.
#[derive(Debug, Clone)]
pub struct AdminAuditRecord {
    pub event: AdminAuditEvent,
    pub actor_id: Option<Uuid>,
}

/// The same for a security event.
#[derive(Debug, Clone)]
pub struct SecurityAuditRecord {
    pub event: SecurityEvent,
    pub actor_id: Option<Uuid>,
}

/// Either kind, for a write that one caller records as an admin event and another as a security event.
#[derive(Debug, Clone)]
pub enum AuditRecord {
    Admin(AdminAuditRecord),
    Security(SecurityAuditRecord),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum NpmPackageEvent {
    PackagePushed { package_name: String, version: String },
    PackageVersionUnpublished { package_name: String, version: String },
    PackageDeleted { package_name: String },
    PackageVersionDeprecated { package_name: String, version: String, message: Option<String> },
    DistTagChanged { package_name: String, tag: String, version: String },
}

impl NpmPackageEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            NpmPackageEvent::PackagePushed { .. } => "PackagePushed",
            NpmPackageEvent::PackageVersionUnpublished { .. } => "PackageVersionUnpublished",
            NpmPackageEvent::PackageDeleted { .. } => "PackageDeleted",
            NpmPackageEvent::PackageVersionDeprecated { .. } => "PackageVersionDeprecated",
            NpmPackageEvent::DistTagChanged { .. } => "DistTagChanged",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum DockerRegistryEvent {
    ImagePushed { image_name: String, digest: String },
    ManifestDeleted { image_name: String, digest: String },
}

impl DockerRegistryEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            DockerRegistryEvent::ImagePushed { .. } => "ImagePushed",
            DockerRegistryEvent::ManifestDeleted { .. } => "ManifestDeleted",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub id: Uuid,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
    pub actor_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
}

pub const DEFAULT_AUDIT_PAGE_SIZE: i64 = 100;
pub const MAX_AUDIT_PAGE_SIZE: i64 = 200;

/// Position of an entry in the log's `(occurred_at, id)` descending order; a page starts strictly after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditCursor {
    pub occurred_at: DateTime<Utc>,
    pub id: Uuid,
}

#[derive(Debug, Clone, Default)]
pub struct AuditQueryFilter {
    pub aggregate_type: Option<String>,
    /// Applied at the SQL level, before `LIMIT`.
    pub exclude_aggregate_type: Option<String>,
    pub aggregate_id: Option<String>,
    pub actor_id: Option<Uuid>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// Only entries stored under this organization. Entries without one match no organization.
    pub organization_id: Option<Uuid>,
    pub cursor: Option<AuditCursor>,
    /// Defaults to `DEFAULT_AUDIT_PAGE_SIZE`, capped at `MAX_AUDIT_PAGE_SIZE`.
    pub limit: Option<i64>,
}

impl AuditQueryFilter {
    pub fn page_size(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_AUDIT_PAGE_SIZE).clamp(1, MAX_AUDIT_PAGE_SIZE)
    }
}

#[derive(Debug, Clone)]
pub struct AuditPage {
    pub entries: Vec<AuditEntry>,
    /// `None` on the last page.
    pub next_cursor: Option<AuditCursor>,
}

const REDACTED: &str = "[redacted]";

/// Leads a sealed secret stored as text; `secret_box` writes the same prefix (its tests check they agree).
pub const SEALED_SECRET_PREFIX: &str = "af1.";

/// Blanks payload values whose key names a password, secret or token (the sealed proxy password in a repository's `Created` event, say),
/// and any string that is a sealed secret whatever its key is called. Booleans like `secret_changed` and ids like `token_id` stay.
pub fn redact_secrets(payload: &mut serde_json::Value) {
    match payload {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields.iter_mut() {
                let key = key.to_ascii_lowercase();
                let names_a_secret = ["password", "secret", "token"].iter().any(|word| key.contains(word)) || key == "remote_username";
                let describes_one = key.ends_with("_id") || value.is_boolean() || value.is_null();
                if names_a_secret && !describes_one {
                    *value = serde_json::Value::String(REDACTED.to_string());
                } else {
                    redact_secrets(value);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_secrets),
        serde_json::Value::String(text) if text.starts_with(SEALED_SECRET_PREFIX) => *payload = serde_json::Value::String(REDACTED.to_string()),
        _ => {}
    }
}

/// Only security and administrative events are ever aged out. Repository, permission and package events stay: the first two are the source of truth for their aggregates, the last are business history.
#[async_trait]
pub trait AuditRetentionPort: Send + Sync {
    /// Deletes at most `limit` security and administrative events that happened before `cutoff`, oldest first, and returns how many went.
    async fn delete_audit_events_before(&self, cutoff: DateTime<Utc>, limit: i64) -> Result<u64, EventStoreError>;
}

#[async_trait]
pub trait EventPublisherPort: Send + Sync {
    async fn publish_security_event(&self, event: SecurityEvent, actor_id: Option<Uuid>) -> Result<(), EventStoreError>;
    async fn publish_admin_event(&self, event: AdminAuditEvent, actor_id: Option<Uuid>) -> Result<(), EventStoreError>;
    async fn query_audit_log(&self, filter: AuditQueryFilter) -> Result<AuditPage, EventStoreError>;
    async fn publish_npm_event(
        &self,
        event: NpmPackageEvent,
        npm_package_id: uuid::Uuid,
        package_repository_id: uuid::Uuid,
        actor_id: Option<uuid::Uuid>,
    ) -> Result<(), EventStoreError>;
    async fn publish_docker_event(
        &self,
        event: DockerRegistryEvent,
        package_repository_id: uuid::Uuid,
        actor_id: Option<uuid::Uuid>,
    ) -> Result<(), EventStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_event_round_trips_through_json() {
        let event = SecurityEvent::LoginFailed { username: "florian".to_string(), ip: "127.0.0.1".to_string() };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: SecurityEvent = serde_json::from_str(&json).unwrap();
        match decoded {
            SecurityEvent::LoginFailed { username, ip } => {
                assert_eq!(username, "florian");
                assert_eq!(ip, "127.0.0.1");
            }
            _ => panic!("expected LoginFailed"),
        }
    }

    #[test]
    fn mfa_verification_failed_round_trips_through_json() {
        let event = SecurityEvent::MfaVerificationFailed { user_id: Uuid::new_v4(), method: "totp".to_string() };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: SecurityEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.event_type(), "MfaVerificationFailed");
    }

    #[test]
    fn default_audit_filter_has_no_constraints() {
        let filter = AuditQueryFilter::default();
        assert!(filter.aggregate_type.is_none());
        assert!(filter.exclude_aggregate_type.is_none());
        assert!(filter.aggregate_id.is_none());
        assert!(filter.actor_id.is_none());
        assert!(filter.from.is_none());
        assert!(filter.to.is_none());
    }

    const SECRET: &str = "hunter2-sentinel-secret";

    fn ldap_config() -> IdentityProviderConfig {
        IdentityProviderConfig::Ldap(crate::sso::LdapConfig {
            server_url: "ldaps://dc.example:636".to_string(),
            bind_dn: "cn=svc,dc=example".to_string(),
            bind_password: SECRET.to_string(),
            user_search_base: "ou=people,dc=example".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        })
    }

    fn oidc_config() -> IdentityProviderConfig {
        IdentityProviderConfig::Oidc(crate::sso::OidcConfig { issuer_url: "https://idp.example".to_string(), client_id: "artiferris".to_string(), client_secret: SECRET.to_string() })
    }

    fn smtp_settings() -> SmtpSettings {
        SmtpSettings {
            host: "smtp.example".to_string(),
            port: 587,
            username: "mailer".to_string(),
            password: SECRET.to_string(),
            from_name: "ArtiFerris".to_string(),
            from_address: "noreply@example.com".to_string(),
            security: SmtpSecurity::StartTls,
        }
    }

    fn every_new_event() -> (Vec<SecurityEvent>, Vec<AdminAuditEvent>) {
        let (user_id, organization_id) = (Uuid::new_v4(), Uuid::new_v4());
        let security = vec![
            SecurityEvent::LoginSucceeded { user_id, organization_id, method: LoginMethod::Password, second_factor: Some(MfaMethod::Totp) },
            SecurityEvent::PasswordChanged { user_id, organization_id },
            SecurityEvent::MfaEnabled { user_id, organization_id, method: MfaMethod::Totp },
            SecurityEvent::MfaDisabled { user_id, organization_id, method: MfaMethod::Passkey },
            SecurityEvent::PasskeyAdded { user_id, organization_id, passkey_id: Uuid::new_v4() },
            SecurityEvent::PasskeyDeleted { user_id, organization_id, passkey_id: Uuid::new_v4() },
            SecurityEvent::ApiTokenCreated { user_id, organization_id, token_id: Uuid::new_v4(), label: "ci".to_string() },
            SecurityEvent::ApiTokenRevoked { user_id, organization_id, token_id: Uuid::new_v4() },
            SecurityEvent::SessionsRevoked { user_id, organization_id },
            SecurityEvent::BackupCodesRegenerated { user_id, organization_id },
        ];
        let admin = vec![
            AdminAuditEvent::UserInvited { user_id, organization_id, username: "alice".to_string(), is_organization_admin: true, is_super_admin: false },
            AdminAuditEvent::UserActivated { user_id, organization_id },
            AdminAuditEvent::InvitationResent { user_id, organization_id },
            AdminAuditEvent::UserDeleted { user_id, organization_id, username: "alice".to_string() },
            AdminAuditEvent::SuperAdminGranted { user_id, organization_id },
            AdminAuditEvent::SuperAdminRevoked { user_id, organization_id },
            AdminAuditEvent::OrganizationAdminGranted { user_id, organization_id },
            AdminAuditEvent::OrganizationAdminRevoked { user_id, organization_id },
            AdminAuditEvent::OrganizationCreated { organization_id, slug: "acme".to_string(), display_name: "Acme".to_string() },
            AdminAuditEvent::IdentityProviderSet {
                organization_id,
                before: Some(IdentityProviderSummary::from(&oidc_config())),
                after: IdentityProviderSummary::from(&ldap_config()),
                secret_changed: true,
            },
            AdminAuditEvent::IdentityProviderSet { organization_id, before: None, after: IdentityProviderSummary::from(&oidc_config()), secret_changed: true },
            AdminAuditEvent::IdentityProviderCleared { organization_id, before: Some(IdentityProviderSummary::from(&ldap_config())) },
            AdminAuditEvent::SmtpSettingsChanged {
                organization_id,
                before: Some(SmtpSettingsSummary::from(&smtp_settings())),
                after: SmtpSettingsSummary::from(&smtp_settings()),
                password_changed: true,
            },
            AdminAuditEvent::SystemSettingsChanged {
                organization_id,
                changes: system_settings_changes(&SystemSettings::defaults(), &SystemSettings { registration_enabled: false, ..SystemSettings::defaults() }),
            },
            AdminAuditEvent::BrandingChanged { organization_id, asset: BrandingAsset::Logo, cleared: false },
            AdminAuditEvent::LoginThrottleCleared { organization_id: Some(organization_id), username: "alice".to_string() },
            AdminAuditEvent::ConfigurationExported { users: 3, repositories: 2, permissions: 4 },
            AdminAuditEvent::ConfigurationImported { users_created: 3, repositories_created: 2, permissions_granted: 4, failures: 1 },
        ];
        (security, admin)
    }

    fn field_names(value: &serde_json::Value, names: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(fields) => {
                for (name, inner) in fields {
                    names.push(name.clone());
                    field_names(inner, names);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| field_names(item, names)),
            _ => {}
        }
    }

    #[test]
    fn no_new_event_serialises_a_secret() {
        let (security, admin) = every_new_event();
        let payloads: Vec<serde_json::Value> =
            security.iter().map(|e| serde_json::to_value(e).unwrap()).chain(admin.iter().map(|e| serde_json::to_value(e).unwrap())).collect();
        for payload in payloads {
            assert!(!payload.to_string().contains(SECRET), "secret value leaked into {payload}");
            let mut names = Vec::new();
            field_names(&payload, &mut names);
            for name in names {
                let secret_shaped = ["password", "secret", "token", "key"].iter().any(|word| name.contains(word));
                assert!(!secret_shaped || ["password_changed", "secret_changed", "token_id", "passkey_id"].contains(&name.as_str()), "secret-looking field {name} in {payload}");
            }
        }
    }

    #[test]
    fn identity_provider_summaries_keep_the_public_fields_only() {
        let oidc = serde_json::to_value(IdentityProviderSummary::from(&oidc_config())).unwrap();
        assert_eq!(oidc, serde_json::json!({ "provider": "oidc", "issuer_url": "https://idp.example", "client_id": "artiferris" }));
        let ldap = serde_json::to_value(IdentityProviderSummary::from(&ldap_config())).unwrap();
        assert_eq!(ldap["bind_dn"], "cn=svc,dc=example");
        assert!(ldap.get("bind_password").is_none());
    }

    #[test]
    fn every_new_event_round_trips_and_keeps_its_type_name() {
        let (security, admin) = every_new_event();
        for event in security {
            let decoded: SecurityEvent = serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
            assert_eq!(decoded.event_type(), event.event_type());
            assert_eq!(serde_json::to_value(&event).unwrap()["event_type"], event.event_type());
            assert!(event.organization_id().is_some());
        }
        for event in admin {
            let decoded: AdminAuditEvent = serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
            assert_eq!(decoded.event_type(), event.event_type());
            assert_eq!(serde_json::to_value(&event).unwrap()["event_type"], event.event_type());
        }
    }

    #[test]
    fn configuration_events_belong_to_no_organization() {
        assert!(AdminAuditEvent::ConfigurationExported { users: 1, repositories: 0, permissions: 0 }.organization_id().is_none());
        assert!(AdminAuditEvent::ConfigurationImported { users_created: 1, repositories_created: 0, permissions_granted: 0, failures: 0 }.organization_id().is_none());
        assert!(SecurityEvent::LoginFailed { username: "x".to_string(), ip: "1.1.1.1".to_string() }.organization_id().is_none());
    }

    #[test]
    fn system_settings_changes_lists_only_the_differing_settings() {
        let before = SystemSettings::defaults();
        let after = SystemSettings { registration_enabled: false, seo_indexing_enabled: true, ..before };
        let changes = system_settings_changes(&before, &after);
        let names: Vec<_> = changes.iter().map(|c| c.setting.as_str()).collect();
        assert_eq!(names, vec!["registration_enabled", "seo_indexing_enabled"]);
        assert_eq!(changes[0].before, serde_json::json!(true));
        assert_eq!(changes[0].after, serde_json::json!(false));
        assert!(system_settings_changes(&before, &before).is_empty());
    }

    #[test]
    fn redaction_blanks_secrets_at_any_depth_but_keeps_flags_and_ids() {
        let mut payload = serde_json::json!({
            "event_type": "Created",
            "remote_username": "robot",
            "remote_password": "af1.sealed",
            "remote_url": "https://registry.example",
            "token_id": "0b4a",
            "secret_changed": true,
            "password_changed": false,
            "after": { "client_secret": "s3", "client_id": "app" },
            "items": [{ "Authorization_Token": "abc" }, { "note": "plain" }],
            "remote_password_unset": null,
        });

        redact_secrets(&mut payload);

        assert_eq!(payload["remote_password"], "[redacted]");
        assert_eq!(payload["remote_username"], "[redacted]");
        assert_eq!(payload["after"]["client_secret"], "[redacted]");
        assert_eq!(payload["items"][0]["Authorization_Token"], "[redacted]");
        assert_eq!(payload["remote_url"], "https://registry.example");
        assert_eq!(payload["token_id"], "0b4a");
        assert_eq!(payload["secret_changed"], true);
        assert_eq!(payload["password_changed"], false);
        assert_eq!(payload["after"]["client_id"], "app");
        assert_eq!(payload["items"][1]["note"], "plain");
        assert!(payload["remote_password_unset"].is_null(), "nothing to hide when nothing is set");
    }

    #[test]
    fn a_sealed_secret_is_redacted_under_any_key_name() {
        let mut payload = serde_json::json!({
            "api_key": "af1.0a1b2c3d.00ff00ff",
            "nested": { "credential": "af1.deadbeef.abcdef", "notes": ["af1.deadbeef.abcdef", "af1 is just text"] },
            "digest": "sha256:af1.not-a-prefix",
        });

        redact_secrets(&mut payload);

        assert_eq!(payload["api_key"], "[redacted]");
        assert_eq!(payload["nested"]["credential"], "[redacted]");
        assert_eq!(payload["nested"]["notes"][0], "[redacted]");
        assert_eq!(payload["nested"]["notes"][1], "af1 is just text");
        assert_eq!(payload["digest"], "sha256:af1.not-a-prefix");
    }

    #[test]
    fn bidirectional_and_other_format_characters_are_stripped_from_a_recorded_username() {
        assert_eq!(recorded_username("admin\u{202E}gnp.exe"), "admingnp.exe");
        assert_eq!(recorded_username("\u{2066}root\u{2069}\u{200B}\u{FEFF}"), "root");
        assert_eq!(recorded_username("a\u{202A}b\u{202B}c\u{202C}d\u{202D}e\u{2067}f\u{2068}g"), "abcdefg");
        assert_eq!(recorded_username("café"), "café", "ordinary letters and accents stay");
    }

    #[test]
    fn a_recorded_username_is_short_and_free_of_control_characters() {
        assert_eq!(recorded_username("  florian "), "florian");
        assert_eq!(recorded_username("a\u{0}b\nc\u{1b}[31m"), "a?b?c?[31m");
        let long = "é".repeat(500);
        assert_eq!(recorded_username(&long).chars().count(), MAX_RECORDED_USERNAME_CHARS);
        match (SecurityEvent::LoginFailed { username: "x".repeat(300), ip: "1.1.1.1".to_string() }).normalized() {
            SecurityEvent::LoginFailed { username, .. } => assert_eq!(username.len(), 64),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_provider_summary_drops_credentials_and_query_strings_from_urls() {
        let oidc = IdentityProviderConfig::Oidc(crate::sso::OidcConfig { issuer_url: "https://user:hunter2@idp.example/realm?access_token=abc#frag".to_string(), client_id: "app".to_string(), client_secret: "s".to_string() });
        let ldap = IdentityProviderConfig::Ldap(crate::sso::LdapConfig {
            server_url: "ldaps://svc:pw@dc.example:636".to_string(),
            bind_dn: "cn=svc".to_string(),
            bind_password: "s".to_string(),
            user_search_base: "ou=people".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        });

        let shown = serde_json::to_string(&[IdentityProviderSummary::from(&oidc), IdentityProviderSummary::from(&ldap)]).unwrap();

        assert!(!shown.contains("hunter2") && !shown.contains("access_token") && !shown.contains("svc:pw"), "{shown}");
        assert!(shown.contains("https://idp.example/realm") && shown.contains("ldaps://dc.example:636"), "{shown}");
        assert_eq!(IdentityProviderSummary::from(&IdentityProviderConfig::Oidc(crate::sso::OidcConfig { issuer_url: "https://idp.example".to_string(), client_id: "a".to_string(), client_secret: "s".to_string() })),
            IdentityProviderSummary::Oidc { issuer_url: "https://idp.example".to_string(), client_id: "a".to_string() }, "a clean URL is stored as typed");
    }

    #[test]
    fn the_page_size_defaults_and_is_capped() {
        assert_eq!(AuditQueryFilter::default().page_size(), DEFAULT_AUDIT_PAGE_SIZE);
        assert_eq!(AuditQueryFilter { limit: Some(10_000), ..Default::default() }.page_size(), MAX_AUDIT_PAGE_SIZE);
        assert_eq!(AuditQueryFilter { limit: Some(0), ..Default::default() }.page_size(), 1);
    }
}
