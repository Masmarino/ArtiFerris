//! Sends mail through whatever `SmtpSettings` are currently configured, read fresh from the port on every send.

use std::sync::Arc;

use async_trait::async_trait;
use artiferris_application::email_templates::LOGO_CID;
use artiferris_domain::branding::BrandingPort;
use artiferris_domain::email::{EmailPort, SmtpSecurity, SmtpSettingsPort};
use artiferris_domain::error::DomainError;
use artiferris_domain::user_preferences::{Language, UserPreferencesPort};
use lettre::message::{Attachment, Body, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use uuid::Uuid;

use crate::branding_defaults::{DEFAULT_LOGO_BYTES, DEFAULT_LOGO_CONTENT_TYPE};

pub const UNENCRYPTED_SMTP_REFUSED: &str = "SMTP without TLS sends the login in cleartext and is only allowed for hosts in ARTIFERRIS_SSRF_ALLOWED_CIDRS; use STARTTLS or TLS";

pub struct SmtpEmailSender {
    settings: Arc<dyn SmtpSettingsPort>,
    branding: Arc<dyn BrandingPort>,
    preferences: Option<Arc<dyn UserPreferencesPort>>,
}

impl SmtpEmailSender {
    pub fn new(settings: Arc<dyn SmtpSettingsPort>, branding: Arc<dyn BrandingPort>) -> Self {
        Self { settings, branding, preferences: None }
    }

    /// Lets e-mails be written in the language each recipient chose (English for the others).
    pub fn with_preferences(mut self, preferences: Arc<dyn UserPreferencesPort>) -> Self {
        self.preferences = Some(preferences);
        self
    }
}

/// Whether the HTML actually references the logo's CID — gates both the branding fetch and the attachment decision from one check.
fn html_references_logo(html_body: &str) -> bool {
    html_body.contains(&format!("cid:{LOGO_CID}"))
}

/// Builds the outgoing MIME message from already-resolved, synchronous values — the seam extracted for direct unit testing.
/// `logo` is `Some((bytes, content_type))` when `html_references_logo` said the HTML needs it and the caller resolved it.
fn build_message(from_address: &str, from_name: &str, to: &str, subject: &str, text_body: &str, html_body: &str, logo: Option<(Vec<u8>, String)>) -> Result<Message, DomainError> {
    let from_address: Address = from_address.parse().map_err(|_| DomainError::Infrastructure("configured SMTP from-address is not a valid mailbox".to_string()))?;
    let from = Mailbox::new(Some(from_name.to_string()), from_address);
    let to: Mailbox = to.parse().map_err(|_| DomainError::Infrastructure("recipient address is not a valid mailbox".to_string()))?;

    let html_part = if html_references_logo(html_body) {
        let (logo_bytes, logo_content_type) = logo.unwrap_or_else(|| (DEFAULT_LOGO_BYTES.to_vec(), DEFAULT_LOGO_CONTENT_TYPE.to_string()));
        let logo = Attachment::new_inline(LOGO_CID.to_string()).body(Body::new(logo_bytes), logo_content_type.parse().map_err(|_| DomainError::Infrastructure("stored logo content type is invalid".to_string()))?);
        MultiPart::related().singlepart(SinglePart::html(html_body.to_string())).singlepart(logo)
    } else {
        MultiPart::related().singlepart(SinglePart::html(html_body.to_string()))
    };
    let body = MultiPart::alternative().singlepart(SinglePart::plain(text_body.to_string())).multipart(html_part);

    Message::builder().from(from).to(to).subject(subject).multipart(body).map_err(|e| DomainError::Infrastructure(format!("failed to build email message: {e}")))
}

#[async_trait]
impl EmailPort for SmtpEmailSender {
    async fn language_for(&self, user_id: Uuid) -> Language {
        let Some(preferences) = &self.preferences else {
            return Language::FALLBACK;
        };
        match preferences.language(user_id).await {
            Ok(language) => language.unwrap_or(Language::FALLBACK),
            Err(e) => {
                tracing::warn!("failed to read the language of user {user_id} for an email: {e}");
                Language::FALLBACK
            }
        }
    }

    async fn send(&self, organization_id: Uuid, to: &str, subject: &str, text_body: &str, html_body: &str) -> Result<(), DomainError> {
        let Some(settings) = self.settings.get(organization_id).await? else {
            return Err(DomainError::Infrastructure("SMTP is not configured".to_string()));
        };

        // Only attach the logo when the HTML actually references it.
        let logo = if html_references_logo(html_body) {
            let branding = self.branding.get(organization_id).await?;
            let (logo_bytes, logo_content_type) = match branding.logo {
                Some(asset) => (asset.bytes, asset.content_type),
                None => (DEFAULT_LOGO_BYTES.to_vec(), DEFAULT_LOGO_CONTENT_TYPE.to_string()),
            };
            Some((logo_bytes, logo_content_type))
        } else {
            None
        };

        let message = build_message(&settings.from_address, &settings.from_name, to, subject, text_body, html_body, logo)?;

        // Same SSRF guard as the other admin-configured remote hosts — the "send test email" feature would otherwise make this an on-demand probe.
        crate::ssrf_guard::ensure_public_host_and_port(&settings.host, settings.port as u16).await?;
        if settings.security == SmtpSecurity::None && !crate::ssrf_allowlist::host_is_allow_listed(&settings.host, settings.port as u16).await {
            return Err(DomainError::Infrastructure(UNENCRYPTED_SMTP_REFUSED.to_string()));
        }

        let credentials = Credentials::new(settings.username.clone(), settings.password.clone());
        let transport = match settings.security {
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host)
                .map_err(|e| DomainError::Infrastructure(format!("failed to build SMTP transport: {e}")))?
                .port(settings.port as u16)
                .credentials(credentials)
                .build(),
            SmtpSecurity::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host)
                .map_err(|e| DomainError::Infrastructure(format!("failed to build SMTP transport: {e}")))?
                .port(settings.port as u16)
                .credentials(credentials)
                .build(),
            SmtpSecurity::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.host).port(settings.port as u16).credentials(credentials).build(),
        };

        transport.send(message).await.map_err(|e| DomainError::Infrastructure(format!("failed to send email: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use artiferris_domain::branding::{BrandingAsset, BrandingSettings};

    use super::*;

    struct FakeSmtpSettings {
        settings: Mutex<std::collections::HashMap<Uuid, artiferris_domain::email::SmtpSettings>>,
    }

    #[async_trait]
    impl SmtpSettingsPort for FakeSmtpSettings {
        async fn get(&self, organization_id: Uuid) -> Result<Option<artiferris_domain::email::SmtpSettings>, DomainError> {
            Ok(self.settings.lock().unwrap().get(&organization_id).cloned())
        }
        async fn update(&self, organization_id: Uuid, settings: &artiferris_domain::email::SmtpSettings, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            self.settings.lock().unwrap().insert(organization_id, settings.clone());
            Ok(())
        }
    }

    struct FakeBranding;

    #[async_trait]
    impl BrandingPort for FakeBranding {
        async fn get(&self, _organization_id: Uuid) -> Result<BrandingSettings, DomainError> {
            Ok(BrandingSettings::default())
        }
        async fn set_logo(&self, _organization_id: Uuid, _asset: &BrandingAsset, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            unreachable!("not exercised by this test")
        }
        async fn clear_logo(&self, _organization_id: Uuid, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            unreachable!("not exercised by this test")
        }
        async fn set_favicon(&self, _organization_id: Uuid, _asset: &BrandingAsset, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            unreachable!("not exercised by this test")
        }
        async fn clear_favicon(&self, _organization_id: Uuid, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            unreachable!("not exercised by this test")
        }
    }

    fn sample_smtp_settings() -> artiferris_domain::email::SmtpSettings {
        artiferris_domain::email::SmtpSettings {
            host: "smtp.example.com".to_string(),
            port: 587,
            username: "artiferris@example.com".to_string(),
            password: "s3cret".to_string(),
            from_name: "ArtiFerris".to_string(),
            from_address: "artiferris@example.com".to_string(),
            security: SmtpSecurity::StartTls,
        }
    }

    #[tokio::test]
    async fn an_organization_without_smtp_configured_never_falls_back_to_another_organizations_settings() {
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();

        let mut settings_by_org = std::collections::HashMap::new();
        settings_by_org.insert(org_b, sample_smtp_settings());
        let settings = FakeSmtpSettings { settings: Mutex::new(settings_by_org) };

        let sender = SmtpEmailSender::new(Arc::new(settings), Arc::new(FakeBranding));

        let result = sender.send(org_a, "to@example.com", "subject", "text", "<p>html</p>").await;

        let err = result.unwrap_err();
        assert!(format!("{err}").contains("SMTP is not configured"), "got {err:?}");
    }

    #[tokio::test]
    async fn a_host_pointing_at_a_private_address_is_rejected_before_connecting() {
        let org_id = Uuid::new_v4();
        let mut settings_by_org = std::collections::HashMap::new();
        settings_by_org.insert(org_id, artiferris_domain::email::SmtpSettings { host: "127.0.0.1".to_string(), ..sample_smtp_settings() });
        let settings = FakeSmtpSettings { settings: Mutex::new(settings_by_org) };

        let sender = SmtpEmailSender::new(Arc::new(settings), Arc::new(FakeBranding));

        let err = sender.send(org_id, "to@example.com", "subject", "text", "<p>html</p>").await.unwrap_err();

        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn a_host_pointing_at_the_cloud_metadata_endpoint_is_rejected() {
        let org_id = Uuid::new_v4();
        let mut settings_by_org = std::collections::HashMap::new();
        settings_by_org.insert(org_id, artiferris_domain::email::SmtpSettings { host: "169.254.169.254".to_string(), ..sample_smtp_settings() });
        let settings = FakeSmtpSettings { settings: Mutex::new(settings_by_org) };

        let sender = SmtpEmailSender::new(Arc::new(settings), Arc::new(FakeBranding));

        let err = sender.send(org_id, "to@example.com", "subject", "text", "<p>html</p>").await.unwrap_err();

        assert!(err.to_string().contains("private or reserved"), "got: {err}");
    }

    #[tokio::test]
    async fn unencrypted_smtp_to_a_host_outside_the_allowlist_is_refused_before_connecting() {
        let org_id = Uuid::new_v4();
        let mut settings_by_org = std::collections::HashMap::new();
        settings_by_org.insert(org_id, artiferris_domain::email::SmtpSettings { host: "8.8.8.8".to_string(), security: SmtpSecurity::None, ..sample_smtp_settings() });
        let sender = SmtpEmailSender::new(Arc::new(FakeSmtpSettings { settings: Mutex::new(settings_by_org) }), Arc::new(FakeBranding));

        let err = sender.send(org_id, "to@example.com", "subject", "text", "<p>html</p>").await.unwrap_err();

        assert!(err.to_string().contains("cleartext"), "got: {err}");
    }

    #[test]
    fn an_invalid_from_address_is_rejected() {
        let result = build_message("not-an-email", "ArtiFerris", "to@example.com", "subject", "text", "<p>html</p>", None);

        let err = result.unwrap_err();
        assert!(format!("{err}").contains("configured SMTP from-address is not a valid mailbox"), "got {err:?}");
    }

    #[test]
    fn an_invalid_recipient_address_is_rejected() {
        let result = build_message("from@example.com", "ArtiFerris", "not-an-email", "subject", "text", "<p>html</p>", None);

        let err = result.unwrap_err();
        assert!(format!("{err}").contains("recipient address is not a valid mailbox"), "got {err:?}");
    }

    #[test]
    fn an_invalid_stored_logo_content_type_is_rejected() {
        let html_body = format!("<img src=\"cid:{LOGO_CID}\">");
        let result = build_message("from@example.com", "ArtiFerris", "to@example.com", "subject", "text", &html_body, Some((vec![1, 2, 3], "not/a/valid/content-type".to_string())));

        let err = result.unwrap_err();
        assert!(format!("{err}").contains("stored logo content type is invalid"), "got {err:?}");
    }

    #[test]
    fn the_logo_is_attached_when_the_html_references_its_cid() {
        let html_body = format!("<img src=\"cid:{LOGO_CID}\">");
        let message = build_message("from@example.com", "ArtiFerris", "to@example.com", "subject", "text", &html_body, Some((b"logo-bytes".to_vec(), "image/png".to_string()))).unwrap();

        let formatted = String::from_utf8_lossy(&message.formatted()).to_string();
        assert!(formatted.contains("image/png"), "expected the logo's content type in the message, got:\n{formatted}");
        assert!(formatted.contains(LOGO_CID), "expected the logo CID in the message, got:\n{formatted}");
    }

    #[test]
    fn the_logo_is_not_attached_when_the_html_does_not_reference_its_cid() {
        let html_body = "<p>no logo here</p>".to_string();
        let message = build_message("from@example.com", "ArtiFerris", "to@example.com", "subject", "text", &html_body, Some((b"logo-bytes".to_vec(), "image/png".to_string()))).unwrap();

        let formatted = String::from_utf8_lossy(&message.formatted()).to_string();
        assert!(!formatted.contains("image/png"), "expected no logo content type in the message, got:\n{formatted}");
        assert!(!formatted.contains(LOGO_CID), "expected no logo CID in the message, got:\n{formatted}");
    }

    #[test]
    fn html_references_logo_detects_the_cid_reference() {
        assert!(html_references_logo(&format!("<img src=\"cid:{LOGO_CID}\">")));
        assert!(!html_references_logo("<p>no logo here</p>"));
    }

    struct FakePreferences {
        answer: Result<Option<Language>, DomainError>,
    }

    #[async_trait]
    impl UserPreferencesPort for FakePreferences {
        async fn language(&self, _user_id: Uuid) -> Result<Option<Language>, DomainError> {
            self.answer.clone()
        }
        async fn set_language(&self, _user_id: Uuid, _language: Language) -> Result<(), DomainError> {
            unreachable!("not exercised by this test")
        }
    }

    fn sender_with(answer: Option<Result<Option<Language>, DomainError>>) -> SmtpEmailSender {
        let sender = SmtpEmailSender::new(Arc::new(FakeSmtpSettings { settings: Mutex::new(Default::default()) }), Arc::new(FakeBranding));
        match answer {
            Some(answer) => sender.with_preferences(Arc::new(FakePreferences { answer })),
            None => sender,
        }
    }

    #[tokio::test]
    async fn a_recipient_is_written_to_in_the_language_they_chose() {
        let sender = sender_with(Some(Ok(Some(Language::De))));

        assert_eq!(sender.language_for(Uuid::new_v4()).await, Language::De);
    }

    #[tokio::test]
    async fn a_recipient_who_never_chose_is_written_to_in_english() {
        let sender = sender_with(Some(Ok(None)));

        assert_eq!(sender.language_for(Uuid::new_v4()).await, Language::En);
    }

    #[tokio::test]
    async fn an_unreadable_preference_does_not_stop_the_notification() {
        let sender = sender_with(Some(Err(DomainError::Infrastructure("db down".to_string()))));

        assert_eq!(sender.language_for(Uuid::new_v4()).await, Language::FALLBACK);
    }

    #[tokio::test]
    async fn without_preferences_wired_every_recipient_gets_english() {
        assert_eq!(sender_with(None).language_for(Uuid::new_v4()).await, Language::FALLBACK);
    }
}
