//! Fakes shared by several of the admin use-case test modules.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use artiferris_domain::error::{DomainError, EventStoreError};
use artiferris_domain::organization::{Organization, OrganizationRepositoryPort, OrganizationSlug};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary};
use artiferris_domain::permission::{PermissionQueryPort, Role};
use artiferris_domain::storage::StorageBackendPort;
use artiferris_domain::storage::StorageError;
use artiferris_domain::system_settings::{SystemSettings, SystemSettingsPort};
use artiferris_domain::user::{User, UserRepositoryPort, Username};
use uuid::Uuid;

pub struct FakeOrganizations {
    pub organizations: Vec<Organization>,
}

impl FakeOrganizations {
    pub fn with(organizations: Vec<Organization>) -> Self {
        Self { organizations }
    }

    pub fn organization(slug: &str, is_public: bool, is_personal: bool) -> Organization {
        Organization { id: Uuid::new_v4(), slug: OrganizationSlug::parse(slug).unwrap(), display_name: slug.to_string(), is_public, is_personal, created_at: chrono::Utc::now() }
    }
}

#[async_trait]
impl OrganizationRepositoryPort for FakeOrganizations {
    async fn create(&self, _org: &Organization) -> Result<(), DomainError> {
        unreachable!("not exercised by these tests")
    }
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Organization>, DomainError> {
        Ok(self.organizations.iter().find(|o| o.id == id).cloned())
    }
    async fn find_by_slug(&self, slug: &OrganizationSlug) -> Result<Option<Organization>, DomainError> {
        Ok(self.organizations.iter().find(|o| &o.slug == slug).cloned())
    }
    async fn find_public(&self) -> Result<Organization, DomainError> {
        self.organizations.iter().find(|o| o.is_public).cloned().ok_or_else(|| DomainError::Infrastructure("no public organization".to_string()))
    }
    async fn list_all(&self) -> Result<Vec<Organization>, DomainError> {
        Ok(self.organizations.clone())
    }
}

pub struct FakeUsers {
    pub users: Mutex<HashMap<Uuid, User>>,
}

impl FakeUsers {
    pub fn seeded(users: Vec<User>) -> Self {
        Self { users: Mutex::new(users.into_iter().map(|u| (u.id, u)).collect()) }
    }
}

#[async_trait]
impl UserRepositoryPort for FakeUsers {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, DomainError> {
        Ok(self.users.lock().unwrap().get(&id).cloned())
    }
    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, DomainError> {
        Ok(self.users.lock().unwrap().values().find(|u| &u.username == username).cloned())
    }
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, DomainError> {
        Ok(self.users.lock().unwrap().values().find(|u| u.email.as_deref() == Some(email)).cloned())
    }
    async fn list_all(&self) -> Result<Vec<User>, DomainError> {
        Ok(self.users.lock().unwrap().values().cloned().collect())
    }
    async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<User>, DomainError> {
        Ok(self.users.lock().unwrap().values().filter(|u| ids.contains(&u.id)).cloned().collect())
    }
    async fn count_by_organization(&self, organization_id: Uuid) -> Result<i64, DomainError> {
        Ok(self.users.lock().unwrap().values().filter(|u| u.organization_id == organization_id).count() as i64)
    }
    async fn search_by_organization(&self, organization_id: Uuid, query: &str, limit: i64) -> Result<Vec<User>, DomainError> {
        let query = query.to_lowercase();
        let mut matches: Vec<User> = self
            .users
            .lock()
            .unwrap()
            .values()
            .filter(|u| u.organization_id == organization_id && u.username.as_str().to_lowercase().contains(&query))
            .cloned()
            .collect();
        matches.sort_by(|a, b| a.username.as_str().cmp(b.username.as_str()));
        matches.truncate(limit as usize);
        Ok(matches)
    }
    async fn search_all_organizations(&self, query: &str, limit: i64) -> Result<Vec<User>, DomainError> {
        let query = query.to_lowercase();
        let mut matches: Vec<User> =
            self.users.lock().unwrap().values().filter(|u| u.username.as_str().to_lowercase().contains(&query)).cloned().collect();
        matches.sort_by(|a, b| a.username.as_str().cmp(b.username.as_str()));
        matches.truncate(limit as usize);
        Ok(matches)
    }
    async fn insert(&self, user: &User) -> Result<(), DomainError> {
        self.users.lock().unwrap().insert(user.id, user.clone());
        Ok(())
    }
    async fn delete(&self, id: Uuid) -> Result<(), DomainError> {
        self.users.lock().unwrap().remove(&id);
        Ok(())
    }
    async fn update_password(&self, id: Uuid, new_password_hash: String, _audit: Option<&artiferris_domain::audit::AuditRecord>) -> Result<(), DomainError> {
        if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
            user.password_hash = new_password_hash;
        }
        Ok(())
    }
    async fn set_super_admin(&self, id: Uuid, is_super_admin: bool) -> Result<(), DomainError> {
        if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
            user.is_super_admin = is_super_admin;
        }
        Ok(())
    }
    async fn set_organization_admin(&self, id: Uuid, is_organization_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
        if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
            user.is_organization_admin = is_organization_admin;
        }
        Ok(())
    }
    async fn delete_unless_last_super_admin(&self, id: Uuid, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> {
        self.users.lock().unwrap().remove(&id);
        Ok(true)
    }
    async fn set_super_admin_unless_last(&self, id: Uuid, is_super_admin: bool, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<bool, DomainError> {
        if let Some(user) = self.users.lock().unwrap().get_mut(&id) {
            user.is_super_admin = is_super_admin;
        }
        Ok(true)
    }
}

pub struct FakeRepositoryQuery {
    pub repos: Vec<PackageRepositorySummary>,
}

#[async_trait]
impl PackageRepositoryQueryPort for FakeRepositoryQuery {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repos.iter().find(|r| r.id == id).cloned())
    }
    async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repos.iter().find(|r| r.organization_id == organization_id && r.name == name).cloned())
    }
    async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repos.clone())
    }
    async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
        Ok(self.repos.iter().filter(|r| r.organization_id == organization_id).cloned().collect())
    }
}

pub struct FakeDockerBlobs {
    pub used_bytes_by_repository: HashMap<Uuid, u64>,
}

#[async_trait]
impl artiferris_domain::docker_registry::DockerBlobStorePort for FakeDockerBlobs {
    async fn write(&self, _digest: &artiferris_domain::docker_registry::Digest, _bytes: &[u8]) -> Result<(), DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn write_stream(&self, _digest: &artiferris_domain::docker_registry::Digest, _body: artiferris_domain::docker_registry::ByteStream, _max_bytes: u64) -> Result<u64, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn adopt_staged_file(&self, _digest: &artiferris_domain::docker_registry::Digest, _staging_path: &str, _size_bytes: u64) -> Result<(), DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn read(&self, _digest: &artiferris_domain::docker_registry::Digest) -> Result<Vec<u8>, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn read_stream(&self, _digest: &artiferris_domain::docker_registry::Digest) -> Result<artiferris_domain::docker_registry::ByteStream, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn link_to_repository(&self, _repository_id: Uuid, _digest: &artiferris_domain::docker_registry::Digest) -> Result<(), DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn is_uploaded_to_repository(&self, _repository_id: Uuid, _digest: &artiferris_domain::docker_registry::Digest) -> Result<bool, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn unlink_from_repository_if_unreferenced(&self, _repository_id: Uuid, _digest: &artiferris_domain::docker_registry::Digest) -> Result<(), DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn exists(&self, _digest: &artiferris_domain::docker_registry::Digest) -> Result<bool, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn size_if_exists(&self, _digest: &artiferris_domain::docker_registry::Digest) -> Result<Option<u64>, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn existing_digests(&self, _digests: &[artiferris_domain::docker_registry::Digest]) -> Result<std::collections::HashSet<String>, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn sum_sizes(&self, _digests: &[artiferris_domain::docker_registry::Digest]) -> Result<u64, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn increment_ref(&self, _digest: &artiferris_domain::docker_registry::Digest) -> Result<(), DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn increment_ref_all(&self, _digests: &[artiferris_domain::docker_registry::Digest]) -> Result<(), DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn delete_if_unreferenced(&self, _digest: &artiferris_domain::docker_registry::Digest) -> Result<bool, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn remove_reclaimed_blob_files(&self, _digests: &[artiferris_domain::docker_registry::Digest]) {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
    async fn used_bytes_for_repository(&self, repository_id: Uuid) -> Result<u64, DomainError> {
        Ok(self.used_bytes_by_repository.get(&repository_id).copied().unwrap_or(0))
    }
    async fn used_bytes_for_repositories(&self, repository_ids: &[Uuid]) -> Result<std::collections::HashMap<Uuid, u64>, DomainError> {
        Ok(repository_ids.iter().filter_map(|id| self.used_bytes_by_repository.get(id).map(|bytes| (*id, *bytes))).collect())
    }
    async fn sweep_unreferenced_blobs(&self, _older_than: chrono::DateTime<chrono::Utc>) -> Result<artiferris_domain::docker_registry::BlobSweepReport, DomainError> {
        unreachable!("not exercised by GetUsageMetricsUseCase's tests")
    }
}

pub struct FakeStorage {
    pub healthy: bool,
}

#[async_trait]
impl StorageBackendPort for FakeStorage {
    async fn write(&self, _repository_id: Uuid, _path: &str, _data: &[u8]) -> Result<(), StorageError> {
        Ok(())
    }
    async fn read(&self, _repository_id: Uuid, _path: &str) -> Result<Vec<u8>, StorageError> {
        Ok(Vec::new())
    }
    async fn read_stream(&self, _repository_id: Uuid, _path: &str) -> Result<artiferris_domain::storage::ByteStream, StorageError> {
        Ok(Box::pin(futures::stream::once(async { Ok(bytes::Bytes::new()) })))
    }
    async fn delete(&self, _repository_id: Uuid, _path: &str) -> Result<(), StorageError> {
        Ok(())
    }
    async fn delete_repository(&self, _repository_id: Uuid) -> Result<(), StorageError> {
        Ok(())
    }
    async fn used_bytes(&self, repository_id: Uuid) -> Result<u64, StorageError> {
        Ok(repository_id.as_u128() as u64 % 1000)
    }
    async fn is_healthy(&self) -> bool {
        self.healthy
    }
    async fn volume_space(&self) -> Result<artiferris_domain::storage::VolumeSpace, StorageError> {
        Ok(artiferris_domain::storage::VolumeSpace { total_bytes: 1000, free_bytes: 400 })
    }
}

pub struct FakePermissionQuery {
    /// (repository_id, user_id, role)
    pub entries: Vec<(Uuid, Uuid, Role)>,
}

#[async_trait]
impl PermissionQueryPort for FakePermissionQuery {
    async fn find_role(&self, user_id: Uuid, repository_id: Uuid) -> Result<Option<Role>, EventStoreError> {
        Ok(self.entries.iter().find(|(r, u, _)| *r == repository_id && *u == user_id).map(|(_, _, role)| *role))
    }
    async fn list_for_repository(&self, repository_id: Uuid) -> Result<Vec<(Uuid, Role)>, EventStoreError> {
        Ok(self.entries.iter().filter(|(r, _, _)| *r == repository_id).map(|(_, u, role)| (*u, *role)).collect())
    }
    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<(Uuid, Role)>, EventStoreError> {
        Ok(self.entries.iter().filter(|(_, u, _)| *u == user_id).map(|(r, _, role)| (*r, *role)).collect())
    }
    async fn list_all(&self) -> Result<Vec<(Uuid, Uuid, Role)>, EventStoreError> {
        Ok(self.entries.iter().map(|(r, u, role)| (*u, *r, *role)).collect())
    }
    async fn count_all(&self) -> Result<usize, EventStoreError> {
        Ok(self.entries.len())
    }
    async fn count_for_repositories(&self, repository_ids: &[Uuid]) -> Result<usize, EventStoreError> {
        Ok(self.entries.iter().filter(|(r, _, _)| repository_ids.contains(r)).count())
    }
}

pub struct FakeSystemSettings {
    pub settings: Mutex<HashMap<Uuid, SystemSettings>>,
}

#[async_trait]
impl SystemSettingsPort for FakeSystemSettings {
    async fn get(&self, organization_id: Uuid) -> Result<SystemSettings, DomainError> {
        Ok(self.settings.lock().unwrap().get(&organization_id).copied().unwrap_or(SystemSettings::defaults()))
    }
    async fn update(&self, organization_id: Uuid, settings: &SystemSettings, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
        self.settings.lock().unwrap().insert(organization_id, *settings);
        Ok(())
    }
}
