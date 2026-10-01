use async_trait::async_trait;
use uuid::Uuid;

use crate::audit::AdminAuditRecord;
use crate::error::DomainError;
use crate::invitation::UserInvitation;
use crate::package_repository::PackageRepositoryEvent;
use crate::permission::PermissionEvent;
use crate::system_settings::SystemSettings;
use crate::user::User;

/// Events for one repository stream, appended after `expected_version` events already written in this batch.
#[derive(Debug, Clone)]
pub struct RepositoryStream {
    pub repository_id: Uuid,
    pub expected_version: u64,
    pub events: Vec<PackageRepositoryEvent>,
}

#[derive(Debug, Clone)]
pub struct PermissionStream {
    pub user_id: Uuid,
    pub repository_id: Uuid,
    pub events: Vec<PermissionEvent>,
}

#[derive(Debug, Clone)]
pub struct ImportBatch {
    pub actor_id: Uuid,
    pub organization_id: Uuid,
    pub users: Vec<User>,
    /// In the order to apply them: a group's members must exist before the group's membership events.
    pub repository_streams: Vec<RepositoryStream>,
    pub permission_streams: Vec<PermissionStream>,
    pub invitations: Vec<UserInvitation>,
    pub system_settings: Option<SystemSettings>,
    pub audit: Option<AdminAuditRecord>,
}

#[async_trait]
pub trait ConfigurationImportPort: Send + Sync {
    /// Writes the whole batch or nothing: a failure leaves the instance as it was, so the import can be run again.
    async fn apply(&self, batch: &ImportBatch) -> Result<(), DomainError>;
}
