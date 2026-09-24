use async_trait::async_trait;
use uuid::Uuid;

use crate::error::EventStoreError;
use crate::package_repository::PackageRepositoryEvent;
use crate::permission::PermissionEvent;

/// Creates a brand-new personal-namespace repository and grants its owner Admin in one
/// transaction, so a failure partway through can never leave a repository with no owner grant —
/// which used to be unrecoverable, since the name stays taken and the orphan is invisible to
/// `list_my_projects`.
#[async_trait]
pub trait PersonalProjectProvisioningPort: Send + Sync {
    /// `repository_event` and `permission_event` must each be the first event of a brand-new
    /// aggregate — later appends go through the regular per-aggregate `append` methods instead.
    async fn create_with_owner_grant(
        &self,
        repository_id: Uuid,
        repository_event: PackageRepositoryEvent,
        owner_user_id: Uuid,
        permission_event: PermissionEvent,
        actor_id: Uuid,
    ) -> Result<(), EventStoreError>;
}
