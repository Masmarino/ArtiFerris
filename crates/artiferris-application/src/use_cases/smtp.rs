use std::sync::Arc;

use artiferris_domain::audit::AdminAuditRecord;
use artiferris_domain::email::{EmailPort, SmtpSecurity, SmtpSettings, SmtpSettingsPort};
use artiferris_domain::error::DomainError;

use crate::error::ApplicationError;

/// What `GetSmtpSettingsUseCase` returns — the password itself never leaves this layer. `password_set` is all a caller needs to render a "leave blank to keep it" field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpSettingsView {
    pub host: String,
    pub port: i32,
    pub username: String,
    pub from_name: String,
    pub from_address: String,
    pub security: SmtpSecurity,
    pub password_set: bool,
}

impl SmtpSettingsView {
    pub fn summary(&self) -> artiferris_domain::audit::SmtpSettingsSummary {
        artiferris_domain::audit::SmtpSettingsSummary {
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            from_name: self.from_name.clone(),
            from_address: self.from_address.clone(),
            security: self.security,
        }
    }
}

pub struct GetSmtpSettingsUseCase {
    settings: Arc<dyn SmtpSettingsPort>,
}

impl GetSmtpSettingsUseCase {
    pub fn new(settings: Arc<dyn SmtpSettingsPort>) -> Self {
        Self { settings }
    }

    pub async fn execute(&self, organization_id: uuid::Uuid) -> Result<Option<SmtpSettingsView>, ApplicationError> {
        let Some(settings) = self.settings.get(organization_id).await? else {
            return Ok(None);
        };
        Ok(Some(SmtpSettingsView {
            host: settings.host,
            port: settings.port,
            username: settings.username,
            from_name: settings.from_name,
            from_address: settings.from_address,
            security: settings.security,
            password_set: true,
        }))
    }
}

/// `password: None` means "keep the currently stored password" — the leave-blank-to-keep UX the admin UI relies on. Required on the very first configuration, since there's nothing yet to keep.
pub struct UpdateSmtpSettingsInput {
    pub host: String,
    pub port: i32,
    pub username: String,
    pub password: Option<String>,
    pub from_name: String,
    pub from_address: String,
    pub security: SmtpSecurity,
}

pub struct UpdateSmtpSettingsUseCase {
    settings: Arc<dyn SmtpSettingsPort>,
}

impl UpdateSmtpSettingsUseCase {
    pub fn new(settings: Arc<dyn SmtpSettingsPort>) -> Self {
        Self { settings }
    }

    pub async fn execute(&self, organization_id: uuid::Uuid, input: UpdateSmtpSettingsInput, audit: Option<&AdminAuditRecord>) -> Result<(), ApplicationError> {
        if input.host.trim().is_empty() {
            return Err(ApplicationError::InvalidSmtpSettings("host must not be empty".to_string()));
        }
        if !(1..=65_535).contains(&input.port) {
            return Err(ApplicationError::InvalidSmtpSettings("port must be between 1 and 65535".to_string()));
        }
        if input.username.trim().is_empty() {
            return Err(ApplicationError::InvalidSmtpSettings("username must not be empty".to_string()));
        }
        if input.from_address.trim().is_empty() || !input.from_address.contains('@') {
            return Err(ApplicationError::InvalidSmtpSettings("from_address must be a valid email address".to_string()));
        }
        if input.from_name.trim().is_empty() {
            return Err(ApplicationError::InvalidSmtpSettings("from_name must not be empty".to_string()));
        }

        let password = match input.password {
            Some(password) if !password.is_empty() => password,
            _ => {
                let existing = match self.settings.get(organization_id).await {
                    Ok(existing) => existing,
                    Err(DomainError::SecretUnreadable(_)) => return Err(ApplicationError::InvalidSmtpSettings("the stored password cannot be read by this server; enter it again".to_string())),
                    Err(e) => return Err(e.into()),
                };
                match existing {
                    // A kept password is only ever sent to the server it was entered for.
                    Some(existing) if existing.host.trim().eq_ignore_ascii_case(input.host.trim()) && existing.port == input.port && existing.username == input.username => existing.password,
                    Some(_) => return Err(ApplicationError::InvalidSmtpSettings("re-enter the password when changing the host, port or username".to_string())),
                    None => return Err(ApplicationError::InvalidSmtpSettings("password is required when configuring SMTP for the first time".to_string())),
                }
            }
        };

        self.settings
            .update(organization_id, &SmtpSettings { host: input.host, port: input.port, username: input.username, password, from_name: input.from_name, from_address: input.from_address, security: input.security }, audit)
            .await?;
        Ok(())
    }
}

pub struct SendTestEmailUseCase {
    email: Arc<dyn EmailPort>,
}

impl SendTestEmailUseCase {
    pub fn new(email: Arc<dyn EmailPort>) -> Self {
        Self { email }
    }

    pub async fn execute(&self, organization_id: uuid::Uuid, to: &str) -> Result<(), ApplicationError> {
        let message = "This is a test email from ArtiFerris. If you received it, your SMTP settings are working correctly.";
        self.email.send(organization_id, to, "ArtiFerris SMTP test", message, &format!("<p>{message}</p>")).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use artiferris_domain::error::DomainError;

    use super::*;

    struct FakeSmtpSettings {
        settings: Mutex<std::collections::HashMap<uuid::Uuid, SmtpSettings>>,
    }

    #[async_trait]
    impl SmtpSettingsPort for FakeSmtpSettings {
        async fn get(&self, organization_id: uuid::Uuid) -> Result<Option<SmtpSettings>, DomainError> {
            Ok(self.settings.lock().unwrap().get(&organization_id).cloned())
        }
        async fn update(&self, organization_id: uuid::Uuid, settings: &SmtpSettings, _audit: Option<&AdminAuditRecord>) -> Result<(), DomainError> {
            self.settings.lock().unwrap().insert(organization_id, settings.clone());
            Ok(())
        }
    }

    struct FakeEmail {
        sent: Mutex<Vec<(uuid::Uuid, String, String, String, String)>>,
    }

    #[async_trait]
    impl EmailPort for FakeEmail {
        async fn send(&self, organization_id: uuid::Uuid, to: &str, subject: &str, text_body: &str, html_body: &str) -> Result<(), DomainError> {
            self.sent.lock().unwrap().push((organization_id, to.to_string(), subject.to_string(), text_body.to_string(), html_body.to_string()));
            Ok(())
        }
    }

    fn sample_input() -> UpdateSmtpSettingsInput {
        UpdateSmtpSettingsInput {
            host: "smtp.example.com".to_string(),
            port: 587,
            username: "artiferris@example.com".to_string(),
            password: Some("s3cret".to_string()),
            from_name: "ArtiFerris".to_string(),
            from_address: "artiferris@example.com".to_string(),
            security: SmtpSecurity::StartTls,
        }
    }

    #[tokio::test]
    async fn get_returns_none_when_never_configured() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = GetSmtpSettingsUseCase::new(settings);
        assert_eq!(use_case.execute(org_id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn get_never_exposes_the_password() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        UpdateSmtpSettingsUseCase::new(settings.clone()).execute(org_id, sample_input(), None).await.unwrap();

        let view = GetSmtpSettingsUseCase::new(settings).execute(org_id).await.unwrap().unwrap();
        assert_eq!(view.host, "smtp.example.com");
        assert!(view.password_set);
    }

    #[tokio::test]
    async fn first_time_configuration_requires_a_password() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = UpdateSmtpSettingsUseCase::new(settings);
        let err = use_case.execute(org_id, UpdateSmtpSettingsInput { password: None, ..sample_input() }, None).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidSmtpSettings(_)));
    }

    #[tokio::test]
    async fn leaving_the_password_blank_on_update_keeps_the_existing_password() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = UpdateSmtpSettingsUseCase::new(settings.clone());
        use_case.execute(org_id, sample_input(), None).await.unwrap();

        use_case.execute(org_id, UpdateSmtpSettingsInput { from_name: "Renamed".to_string(), host: "SMTP.Example.com".to_string(), password: None, ..sample_input() }, None).await.unwrap();

        let stored = settings.get(org_id).await.unwrap().unwrap();
        assert_eq!(stored.from_name, "Renamed");
        assert_eq!(stored.password, "s3cret");
    }

    #[tokio::test]
    async fn a_kept_password_is_refused_when_the_host_port_or_username_changes() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = UpdateSmtpSettingsUseCase::new(settings.clone());
        use_case.execute(org_id, sample_input(), None).await.unwrap();

        for changed in [
            UpdateSmtpSettingsInput { host: "evil.example.net".to_string(), password: None, ..sample_input() },
            UpdateSmtpSettingsInput { port: 465, password: None, ..sample_input() },
            UpdateSmtpSettingsInput { username: "someone-else@example.com".to_string(), password: None, ..sample_input() },
        ] {
            let err = use_case.execute(org_id, changed, None).await.unwrap_err();
            assert!(matches!(&err, ApplicationError::InvalidSmtpSettings(m) if m.contains("re-enter the password")), "got: {err}");
        }
        let stored = settings.get(org_id).await.unwrap().unwrap();
        assert_eq!(stored.host, "smtp.example.com");
        assert_eq!(stored.port, 587);
    }

    #[tokio::test]
    async fn a_new_password_may_come_with_a_new_destination() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = UpdateSmtpSettingsUseCase::new(settings.clone());
        use_case.execute(org_id, sample_input(), None).await.unwrap();

        use_case.execute(org_id, UpdateSmtpSettingsInput { host: "smtp2.example.com".to_string(), password: Some("new-secret".to_string()), ..sample_input() }, None).await.unwrap();

        let stored = settings.get(org_id).await.unwrap().unwrap();
        assert_eq!((stored.host.as_str(), stored.password.as_str()), ("smtp2.example.com", "new-secret"));
    }

    #[tokio::test]
    async fn rejects_an_empty_host() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = UpdateSmtpSettingsUseCase::new(settings);
        let err = use_case.execute(org_id, UpdateSmtpSettingsInput { host: "  ".to_string(), ..sample_input() }, None).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidSmtpSettings(_)));
    }

    #[tokio::test]
    async fn rejects_an_out_of_range_port() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = UpdateSmtpSettingsUseCase::new(settings);
        let err = use_case.execute(org_id, UpdateSmtpSettingsInput { port: 0, ..sample_input() }, None).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidSmtpSettings(_)));
    }

    #[tokio::test]
    async fn rejects_a_from_address_without_an_at_sign() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = UpdateSmtpSettingsUseCase::new(settings);
        let err = use_case.execute(org_id, UpdateSmtpSettingsInput { from_address: "not-an-email".to_string(), ..sample_input() }, None).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidSmtpSettings(_)));
    }

    #[tokio::test]
    async fn rejects_an_empty_from_name() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        let use_case = UpdateSmtpSettingsUseCase::new(settings);
        let err = use_case.execute(org_id, UpdateSmtpSettingsInput { from_name: "  ".to_string(), ..sample_input() }, None).await.unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidSmtpSettings(_)));
    }

    #[tokio::test]
    async fn from_name_round_trips_through_get() {
        let org_id = uuid::Uuid::new_v4();
        let settings = Arc::new(FakeSmtpSettings { settings: Mutex::new(std::collections::HashMap::new()) });
        UpdateSmtpSettingsUseCase::new(settings.clone()).execute(org_id, UpdateSmtpSettingsInput { from_name: "Acme Corp".to_string(), ..sample_input() }, None).await.unwrap();

        let view = GetSmtpSettingsUseCase::new(settings).execute(org_id).await.unwrap().unwrap();
        assert_eq!(view.from_name, "Acme Corp");
    }

    #[tokio::test]
    async fn send_test_email_delegates_to_the_email_port() {
        let org_id = uuid::Uuid::new_v4();
        let email = Arc::new(FakeEmail { sent: Mutex::new(Vec::new()) });
        let use_case = SendTestEmailUseCase::new(email.clone());
        use_case.execute(org_id, "admin@example.com").await.unwrap();

        let sent = email.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, org_id);
        assert_eq!(sent[0].1, "admin@example.com");
    }
}
