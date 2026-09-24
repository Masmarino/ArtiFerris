use std::sync::Arc;

use artiferris_domain::audit::AdminAuditRecord;
use artiferris_domain::system_settings::{SystemSettings, SystemSettingsPort};
use uuid::Uuid;

use crate::error::ApplicationError;

pub struct GetSystemSettingsUseCase {
    settings: Arc<dyn SystemSettingsPort>,
}

impl GetSystemSettingsUseCase {
    pub fn new(settings: Arc<dyn SystemSettingsPort>) -> Self {
        Self { settings }
    }

    pub async fn execute(&self, organization_id: Uuid) -> Result<SystemSettings, ApplicationError> {
        Ok(self.settings.get(organization_id).await?)
    }
}

pub struct UpdateSystemSettingsUseCase {
    settings: Arc<dyn SystemSettingsPort>,
}

impl UpdateSystemSettingsUseCase {
    pub fn new(settings: Arc<dyn SystemSettingsPort>) -> Self {
        Self { settings }
    }

    /// `audit` is written in the same transaction as the change.
    pub async fn execute(&self, organization_id: Uuid, settings: SystemSettings, audit: Option<&AdminAuditRecord>) -> Result<(), ApplicationError> {
        validate_settings(&settings)?;
        self.settings.update(organization_id, &settings, audit).await?;
        Ok(())
    }
}

/// The only place these ranges are validated — downstream consumers trust their inputs.
pub(super) fn validate_settings(settings: &SystemSettings) -> Result<(), ApplicationError> {
    if !(1..=1000).contains(&settings.max_login_attempts) {
        return Err(ApplicationError::InvalidSystemSettings("max_login_attempts must be between 1 and 1000".to_string()));
    }
    if !(1..=86_400).contains(&settings.login_attempt_window_seconds) {
        return Err(ApplicationError::InvalidSystemSettings("login_attempt_window_seconds must be between 1 and 86400 (24h)".to_string()));
    }
    if !(1..=720).contains(&settings.session_ttl_hours) {
        return Err(ApplicationError::InvalidSystemSettings("session_ttl_hours must be between 1 and 720 (30 days)".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use crate::use_cases::admin_test_support::FakeSystemSettings;

    #[tokio::test]
    async fn get_system_settings_returns_the_current_values() {
        let org_id = Uuid::new_v4();
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = GetSystemSettingsUseCase::new(settings);
        assert_eq!(use_case.execute(org_id).await.unwrap(), SystemSettings::defaults());
    }

    #[tokio::test]
    async fn update_system_settings_persists_valid_values() {
        let org_id = Uuid::new_v4();
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = UpdateSystemSettingsUseCase::new(settings.clone());
        let updated = SystemSettings { max_login_attempts: 5, login_attempt_window_seconds: 60, session_ttl_hours: 1, registration_enabled: false, seo_indexing_enabled: false };

        use_case.execute(org_id, updated, None).await.unwrap();

        assert_eq!(settings.get(org_id).await.unwrap(), updated);
    }

    #[tokio::test]
    async fn update_system_settings_rejects_a_zero_login_attempt_limit() {
        let org_id = Uuid::new_v4();
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = UpdateSystemSettingsUseCase::new(settings.clone());

        let err = use_case.execute(org_id, SystemSettings { max_login_attempts: 0, ..SystemSettings::defaults() }, None).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InvalidSystemSettings(_)));
        assert_eq!(settings.get(org_id).await.unwrap(), SystemSettings::defaults(), "an invalid update must not be persisted");
    }

    #[tokio::test]
    async fn update_system_settings_rejects_an_excessive_session_ttl() {
        let org_id = Uuid::new_v4();
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = UpdateSystemSettingsUseCase::new(settings);

        let err = use_case.execute(org_id, SystemSettings { session_ttl_hours: 10_000, ..SystemSettings::defaults() }, None).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InvalidSystemSettings(_)));
    }
}
