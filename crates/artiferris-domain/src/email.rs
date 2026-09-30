use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::AdminAuditRecord;
use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmtpSecurity {
    None,
    StartTls,
    Tls,
}

/// Kept separate from `SystemSettings` so `password` never rides along in a settings row that gets echoed back over HTTP.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpSettings {
    pub host: String,
    pub port: i32,
    pub username: String,
    /// Plaintext at this layer — encryption at rest is the Postgres adapter's job.
    pub password: String,
    pub from_name: String,
    pub from_address: String,
    pub security: SmtpSecurity,
}

impl std::fmt::Debug for SmtpSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .field("from_name", &self.from_name)
            .field("from_address", &self.from_address)
            .field("security", &self.security)
            .finish()
    }
}

#[async_trait]
pub trait SmtpSettingsPort: Send + Sync {
    /// `None` means never configured — treat as disabled, not an error.
    async fn get(&self, organization_id: Uuid) -> Result<Option<SmtpSettings>, DomainError>;
    /// `audit` goes in the same transaction.
    async fn update(&self, organization_id: Uuid, settings: &SmtpSettings, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError>;
}

#[async_trait]
pub trait EmailPort: Send + Sync {
    async fn send(&self, organization_id: Uuid, to: &str, subject: &str, text_body: &str, html_body: &str) -> Result<(), DomainError>;

    /// The language to write to this user in: the one they chose for the interface, English when they have not chosen (or when it cannot be read: a notification must go out anyway).
    async fn language_for(&self, _user_id: Uuid) -> crate::user_preferences::Language {
        crate::user_preferences::Language::FALLBACK
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_smtp_password() {
        let settings = SmtpSettings {
            host: "smtp.example.com".to_string(),
            port: 587,
            username: "mailer".to_string(),
            password: "smtp-password-value".to_string(),
            from_name: "A".to_string(),
            from_address: "a@example.com".to_string(),
            security: SmtpSecurity::StartTls,
        };

        let printed = format!("{settings:?}");

        assert!(!printed.contains("smtp-password-value"), "got: {printed}");
        assert!(printed.contains("mailer"));
    }
}
