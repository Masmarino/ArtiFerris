use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use futures::StreamExt;
use artiferris_domain::audit::{AdminAuditEvent, AdminAuditRecord};
use artiferris_domain::configuration_import::{ConfigurationImportPort, ImportBatch, PermissionStream, RepositoryStream};
use artiferris_domain::error::DomainError;
use artiferris_domain::organization::OrganizationRepositoryPort;
use artiferris_domain::package_repository::{parse_repository_name, PackageRepository, PackageRepositoryQueryPort, RepositoryFormat, RepositoryQuotaLockPort, RepositoryType};
use artiferris_domain::permission::Permission;
use artiferris_domain::system_settings::SystemSettings;
use artiferris_domain::user::UserRepositoryPort;
use uuid::Uuid;

use crate::error::ApplicationError;
use super::config_export::{ensure_single_tenant, ExportedPermission, ExportedRepository, ExportedUser};
use super::system_settings::validate_settings;

#[derive(Debug, Clone)]
pub struct ConfigurationImport {
    pub users: Vec<ExportedUser>,
    pub repositories: Vec<ExportedRepository>,
    pub permissions: Vec<ExportedPermission>,
    pub system_settings: SystemSettings,
}

#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub users_created: usize,
    pub repositories_created: usize,
    pub permissions_granted: usize,
    pub invited: Vec<String>,
    pub skipped_no_email: Vec<String>,
    /// Entries skipped before anything was written, plus invitation emails that could not be sent.
    pub failed: Vec<String>,
    /// Proxy repos restored without credentials — expected, since export never includes them.
    pub proxy_credentials_needed: Vec<String>,
}

/// Per list (users, repositories, permissions). The route's 32 MiB body limit is the other bound.
pub const MAX_IMPORT_ENTRIES: usize = 100_000;

/// Invitation emails in flight at once.
const MAIL_CONCURRENCY: usize = 8;

/// An invitation to mail once the import has been committed.
struct PendingInvitation {
    username: String,
    email: String,
    activation_url: String,
}

pub struct ImportConfigurationUseCase {
    users: Arc<dyn UserRepositoryPort>,
    hasher: Arc<dyn artiferris_domain::user::PasswordHasherPort>,
    importer: Arc<dyn ConfigurationImportPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    email: Arc<dyn artiferris_domain::email::EmailPort>,
    organizations: Arc<dyn OrganizationRepositoryPort>,
    lock: Arc<dyn RepositoryQuotaLockPort>,
    /// No trailing slash. See `crate::use_cases::invitation::organization_origin`.
    artiferris_base_domain: String,
}

impl ImportConfigurationUseCase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        users: Arc<dyn UserRepositoryPort>,
        hasher: Arc<dyn artiferris_domain::user::PasswordHasherPort>,
        importer: Arc<dyn ConfigurationImportPort>,
        repositories: Arc<dyn PackageRepositoryQueryPort>,
        email: Arc<dyn artiferris_domain::email::EmailPort>,
        organizations: Arc<dyn OrganizationRepositoryPort>,
        lock: Arc<dyn RepositoryQuotaLockPort>,
        artiferris_base_domain: String,
    ) -> Self {
        Self { users, hasher, importer, repositories, email, organizations, lock, artiferris_base_domain }
    }

    /// Entries that cannot be restored are listed in `failed` and skipped before anything is written; the rest goes in one
    /// transaction, so a failed import leaves the instance empty and can be run again.
    pub async fn execute(&self, import: ConfigurationImport, actor_id: Uuid) -> Result<ImportReport, ApplicationError> {
        for (what, entries) in [("users", import.users.len()), ("repositories", import.repositories.len()), ("permissions", import.permissions.len())] {
            if entries > MAX_IMPORT_ENTRIES {
                return Err(ApplicationError::ImportTooLarge(format!("the file lists {entries} {what}, at most {MAX_IMPORT_ENTRIES} can be imported at once")));
            }
        }
        let import_lock = self.lock.acquire_repository_lock(Uuid::nil()).await?;

        ensure_single_tenant(&self.organizations.list_all().await?)?;
        if !self.repositories.list_all().await?.is_empty() {
            return Err(ApplicationError::InstanceNotEmpty);
        }
        let existing_users = self.users.list_all().await?;
        if existing_users.iter().any(|u| u.id != actor_id) {
            return Err(ApplicationError::InstanceNotEmpty);
        }
        let actor = existing_users.iter().find(|u| u.id == actor_id).ok_or(ApplicationError::ActingAdminNotFound)?;
        let restore_organization_id = actor.organization_id;
        let restore_organization = crate::use_cases::invitation::require_organization(self.organizations.as_ref(), restore_organization_id).await?;
        let context = PlanContext {
            actor_id,
            actor_username: actor.username.as_str().to_string(),
            organization_id: restore_organization_id,
            activation_origin: crate::use_cases::invitation::organization_origin(&self.artiferris_base_domain, &restore_organization),
            unusable_password_hash: crate::use_cases::invitation::unusable_password_hash(self.hasher.as_ref()).await?,
        };

        let Plan { batch, mut report, pending_invitations } =
            tokio::task::spawn_blocking(move || plan_import(import, context)).await.map_err(|e| DomainError::Infrastructure(format!("planning the import failed: {e}")))?;

        self.importer.apply(&batch).await?;
        drop(import_lock);

        let email = self.email.clone();
        let mailed = tokio::spawn(send_invitations(email, restore_organization_id, actor_id, pending_invitations));
        let (invited, failed) = mailed.await.map_err(|e| DomainError::Infrastructure(format!("sending the invitations failed: {e}")))?;
        report.invited = invited;
        report.failed.extend(failed);
        Ok(report)
    }
}

/// Order of `invited` follows the file. Failures are reported per address and never stop the others.
async fn send_invitations(email: Arc<dyn artiferris_domain::email::EmailPort>, organization_id: Uuid, actor_id: Uuid, invitations: Vec<PendingInvitation>) -> (Vec<String>, Vec<String>) {
    let language = email.language_for(actor_id).await;
    let results = futures::stream::iter(invitations.into_iter().map(|invitation| {
        let email = email.clone();
        async move {
            let content = crate::email_templates::account_created(language, &invitation.username, &invitation.activation_url);
            let sent = email.send(organization_id, &invitation.email, &content.subject, &content.text, &content.html).await;
            (invitation.username, sent)
        }
    }))
    .buffered(MAIL_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    let (mut invited, mut failed) = (Vec::new(), Vec::new());
    for (username, sent) in results {
        match sent {
            Ok(()) => invited.push(username),
            Err(e) => failed.push(format!("invitation email for {username}: {e}")),
        }
    }
    (invited, failed)
}

struct PlanContext {
    actor_id: Uuid,
    actor_username: String,
    organization_id: Uuid,
    activation_origin: String,
    /// One unusable hash serves every restored user; none can log in until invited.
    unusable_password_hash: String,
}

struct Plan {
    batch: ImportBatch,
    report: ImportReport,
    pending_invitations: Vec<PendingInvitation>,
}

fn plan_import(import: ConfigurationImport, context: PlanContext) -> Plan {
    let mut report = ImportReport::default();
    let mut batch = ImportBatch {
        actor_id: context.actor_id,
        organization_id: context.organization_id,
        users: Vec::new(),
        repository_streams: Vec::new(),
        permission_streams: Vec::new(),
        invitations: Vec::new(),
        system_settings: None,
        audit: None,
    };
    let mut pending_invitations = Vec::new();

    let restored_user_ids = plan_users(&import.users, &context, &mut batch, &mut report);
    let repo_id_map = plan_repositories(&import.repositories, context.organization_id, &mut batch, &mut report);
    plan_permissions(&import.permissions, &restored_user_ids, &repo_id_map, &mut batch, &mut report);

    match validate_settings(&import.system_settings) {
        Ok(()) => batch.system_settings = Some(import.system_settings),
        Err(e) => report.failed.push(format!("system settings: {e}")),
    }

    for exported in &import.users {
        if !restored_user_ids.contains(&exported.id) {
            continue;
        }
        let Some(email) = exported.email.as_deref() else {
            report.skipped_no_email.push(exported.username.clone());
            continue;
        };
        let token = crate::use_cases::invitation::generate_invitation_token();
        batch.invitations.push(artiferris_domain::invitation::UserInvitation {
            user_id: exported.id,
            token_hash: crate::use_cases::invitation::hash_invitation_token(&token),
            expires_at: Utc::now() + chrono::Duration::hours(crate::use_cases::invitation::INVITATION_TTL_HOURS),
        });
        pending_invitations.push(PendingInvitation { username: exported.username.clone(), email: email.to_string(), activation_url: format!("{}/activate?token={token}", context.activation_origin) });
    }

    batch.audit = Some(AdminAuditRecord {
        event: AdminAuditEvent::ConfigurationImported {
            users_created: report.users_created,
            repositories_created: report.repositories_created,
            permissions_granted: report.permissions_granted,
            failures: report.failed.len(),
        },
        actor_id: Some(context.actor_id),
    });
    Plan { batch, report, pending_invitations }
}

fn plan_users(exported: &[ExportedUser], context: &PlanContext, batch: &mut ImportBatch, report: &mut ImportReport) -> HashSet<Uuid> {
    let mut restored = HashSet::new();
    let mut usernames = HashSet::from([context.actor_username.clone()]);
    for user in exported {
        let username = match artiferris_domain::user::Username::parse_new(&user.username) {
            Ok(u) => u,
            Err(e) => {
                report.failed.push(format!("user {}: {e}", user.username));
                continue;
            }
        };
        if let Some(email) = user.email.as_deref() {
            if let Err(e) = crate::use_cases::invitation::validate_email(email) {
                report.failed.push(format!("user {}: {e}", user.username));
                continue;
            }
        }
        if user.id == context.actor_id || restored.contains(&user.id) {
            report.failed.push(format!("user {}: id {} is already in use", user.username, user.id));
            continue;
        }
        if !usernames.insert(username.as_str().to_string()) {
            report.failed.push(format!("user {}: username already taken by the signed-in account or another user in the file", user.username));
            continue;
        }
        batch.users.push(artiferris_domain::user::User {
            id: user.id,
            username,
            password_hash: context.unusable_password_hash.clone(),
            is_super_admin: user.is_super_admin,
            is_organization_admin: false,
            organization_id: context.organization_id,
            created_at: user.created_at,
            tokens_valid_after: user.created_at,
            email: user.email.clone(),
        });
        restored.insert(user.id);
        report.users_created += 1;
    }
    restored
}

/// Old repository id to new id. Group memberships come after every repository exists.
fn plan_repositories(exported: &[ExportedRepository], organization_id: Uuid, batch: &mut ImportBatch, report: &mut ImportReport) -> HashMap<Uuid, Uuid> {
    struct Planned<'a> {
        source: &'a ExportedRepository,
        new_id: Uuid,
        name: String,
        events: Vec<artiferris_domain::package_repository::PackageRepositoryEvent>,
    }
    let mut planned: Vec<Planned> = Vec::new();
    let mut names = HashSet::new();
    let mut repo_id_map: HashMap<Uuid, Uuid> = HashMap::new();
    for repository in exported {
        let name = match parse_repository_name(&repository.name) {
            Ok(name) => name,
            Err(e) => {
                report.failed.push(format!("repository {}: {e}", repository.name));
                continue;
            }
        };
        if !names.insert(name.clone()) || repo_id_map.contains_key(&repository.id) {
            report.failed.push(format!("repository {name}: {}", ApplicationError::RepositoryNameTaken));
            continue;
        }
        let new_id = Uuid::new_v4();
        let created = match PackageRepository::create(
            new_id,
            organization_id,
            name.clone(),
            repository.format,
            repository.repo_type,
            repository.remote_url.clone(),
            repository.remote_username.clone(),
            repository.remote_password.clone(),
        ) {
            Ok(event) => event,
            Err(e) => {
                report.failed.push(format!("repository {name}: {e}"));
                continue;
            }
        };
        let mut events = vec![created];
        if repository.quota_bytes.is_some() {
            match PackageRepository::from_events(&events).set_quota(repository.quota_bytes) {
                Ok(event) => events.push(event),
                Err(e) => report.failed.push(format!("repository {name} quota: {e}")),
            }
        }
        if repository.retention_keep_last_n.is_some() {
            match PackageRepository::from_events(&events).set_retention_policy(repository.retention_keep_last_n) {
                Ok(event) => events.push(event),
                Err(e) => report.failed.push(format!("repository {name} retention: {e}")),
            }
        }
        if repository.repo_type == RepositoryType::Proxy && repository.remote_username.is_none() && repository.remote_password.is_none() {
            report.proxy_credentials_needed.push(name.clone());
        }
        report.repositories_created += 1;
        repo_id_map.insert(repository.id, new_id);
        planned.push(Planned { source: repository, new_id, name, events });
    }

    let formats: HashMap<Uuid, RepositoryFormat> = planned.iter().map(|p| (p.new_id, p.source.format)).collect();
    let mut membership_streams = Vec::new();
    for plan in &planned {
        let mut events = plan.events.clone();
        let created_events = events.len() as u64;
        for (position, old_member_id) in plan.source.group_members.iter().enumerate() {
            let Some(&member_id) = repo_id_map.get(old_member_id) else {
                report.failed.push(format!("repository {}: group member {old_member_id} was never created", plan.name));
                continue;
            };
            let outcome = if member_id == plan.new_id {
                Err(DomainError::SelfGroupMembership)
            } else if formats.get(&member_id) != Some(&plan.source.format) {
                Err(DomainError::GroupMemberFormatMismatch(member_id))
            } else {
                PackageRepository::from_events(&events).add_group_member(member_id, position as i32)
            };
            match outcome {
                Ok(event) => events.push(event),
                Err(e) => report.failed.push(format!("repository {} group member: {e}", plan.name)),
            }
        }
        if events.len() as u64 > created_events {
            membership_streams.push(RepositoryStream { repository_id: plan.new_id, expected_version: created_events, events: events.split_off(created_events as usize) });
        }
    }
    batch.repository_streams = planned.into_iter().map(|p| RepositoryStream { repository_id: p.new_id, expected_version: 0, events: p.events }).collect();
    batch.repository_streams.extend(membership_streams);
    repo_id_map
}

fn plan_permissions(exported: &[ExportedPermission], restored_user_ids: &HashSet<Uuid>, repo_id_map: &HashMap<Uuid, Uuid>, batch: &mut ImportBatch, report: &mut ImportReport) {
    let mut stream_positions: HashMap<(Uuid, Uuid), usize> = HashMap::new();
    for permission in exported {
        if !restored_user_ids.contains(&permission.user_id) {
            report.failed.push(format!("permission for {}: user was never restored", permission.user_id));
            continue;
        }
        let Some(&repository_id) = repo_id_map.get(&permission.repository_id) else {
            report.failed.push(format!("permission for {}: repository {} was never created", permission.user_id, permission.repository_id));
            continue;
        };
        let position = *stream_positions.entry((permission.user_id, repository_id)).or_insert_with(|| {
            batch.permission_streams.push(PermissionStream { user_id: permission.user_id, repository_id, events: Vec::new() });
            batch.permission_streams.len() - 1
        });
        let stream = &mut batch.permission_streams[position];
        let event = Permission::from_events(&stream.events).grant(permission.user_id, repository_id, permission.role);
        stream.events.push(event);
        report.permissions_granted += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use artiferris_domain::error::{DomainError, EventStoreError};
    use artiferris_domain::organization::{Organization, OrganizationSlug};
    use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryFormat};
    use artiferris_domain::permission::Role;
    use artiferris_domain::system_settings::SystemSettingsPort;
    use artiferris_domain::user::{User, UserRepositoryPort, Username};
    use crate::use_cases::package_repository::CreatePackageRepositoryUseCase;
    use artiferris_domain::invitation::UserInvitationPort;
    use artiferris_domain::permission::PermissionEventStorePort;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use crate::use_cases::admin_test_support::{FakeSystemSettings, FakeUsers};
    use crate::use_cases::npm_test_support::FakeRepositoryQuotaLock;
    use artiferris_domain::configuration_import::{ConfigurationImportPort, ImportBatch};
    use artiferris_domain::package_repository::PackageRepositoryEventStorePort;

    struct FakePackageRepositoryStore {
        streams: Mutex<HashMap<Uuid, Vec<artiferris_domain::package_repository::PackageRepositoryEvent>>>,
    }

    impl FakePackageRepositoryStore {
        fn new() -> Self {
            Self { streams: Mutex::new(HashMap::new()) }
        }

        fn summarize(id: Uuid, events: &[artiferris_domain::package_repository::PackageRepositoryEvent]) -> Option<PackageRepositorySummary> {
            let repo = artiferris_domain::package_repository::PackageRepository::from_events(events);
            if repo.deleted || repo.name.is_none() {
                return None;
            }
            let organization_id = events
                .iter()
                .find_map(|event| match event {
                    artiferris_domain::package_repository::PackageRepositoryEvent::Created { organization_id, .. } => Some(*organization_id),
                    _ => None,
                })
                .expect("a summarizable repository always has a Created event");
            Some(PackageRepositorySummary {
                id,
                organization_id,
                name: repo.name.unwrap(),
                format: repo.format.unwrap(),
                repo_type: repo.repo_type.unwrap(),
                remote_url: repo.remote_url,
                remote_username: repo.remote_username,
                remote_password: repo.remote_password,
                group_members: repo.group_members.into_iter().map(|(id, _)| id).collect(),
                quota_bytes: repo.quota_bytes,
                retention_keep_last_n: repo.retention_keep_last_n,
                is_public: repo.is_public,
            })
        }
    }

    #[async_trait]
    impl artiferris_domain::package_repository::PackageRepositoryEventStorePort for FakePackageRepositoryStore {
        async fn load(&self, repository_id: Uuid) -> Result<(u64, Vec<artiferris_domain::package_repository::PackageRepositoryEvent>), EventStoreError> {
            let streams = self.streams.lock().unwrap();
            let events = streams.get(&repository_id).cloned().unwrap_or_default();
            Ok((events.len() as u64, events))
        }

        async fn append(
            &self,
            repository_id: Uuid,
            expected_version: u64,
            events: Vec<artiferris_domain::package_repository::PackageRepositoryEvent>,
            _actor_id: Uuid,
        ) -> Result<(), EventStoreError> {
            let mut streams = self.streams.lock().unwrap();
            let stream = streams.entry(repository_id).or_default();
            if stream.len() as u64 != expected_version {
                return Err(EventStoreError::ConcurrencyConflict { expected: expected_version, actual: stream.len() as u64 });
            }
            stream.extend(events);
            Ok(())
        }
    }

    #[async_trait]
    impl PackageRepositoryQueryPort for FakePackageRepositoryStore {
        async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
            let streams = self.streams.lock().unwrap();
            Ok(streams.get(&id).and_then(|events| Self::summarize(id, events)))
        }
        async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
            let streams = self.streams.lock().unwrap();
            Ok(streams
                .iter()
                .find_map(|(id, events)| Self::summarize(*id, events).filter(|s| s.organization_id == organization_id && s.name == name)))
        }
        async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
            let streams = self.streams.lock().unwrap();
            Ok(streams.iter().filter_map(|(id, events)| Self::summarize(*id, events)).collect())
        }
        async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
            let streams = self.streams.lock().unwrap();
            Ok(streams.iter().filter_map(|(id, events)| Self::summarize(*id, events)).filter(|s| s.organization_id == organization_id).collect())
        }
    }

    struct FakePermissionStore {
        streams: Mutex<HashMap<(Uuid, Uuid), Vec<artiferris_domain::permission::PermissionEvent>>>,
    }

    impl FakePermissionStore {
        fn new() -> Self {
            Self { streams: Mutex::new(HashMap::new()) }
        }
    }

    #[async_trait]
    impl artiferris_domain::permission::PermissionEventStorePort for FakePermissionStore {
        async fn load(&self, user_id: Uuid, repository_id: Uuid) -> Result<(u64, Vec<artiferris_domain::permission::PermissionEvent>), EventStoreError> {
            let streams = self.streams.lock().unwrap();
            let events = streams.get(&(user_id, repository_id)).cloned().unwrap_or_default();
            Ok((events.len() as u64, events))
        }
        async fn append(
            &self,
            user_id: Uuid,
            repository_id: Uuid,
            expected_version: u64,
            events: Vec<artiferris_domain::permission::PermissionEvent>,
            _actor_id: Uuid,
        ) -> Result<(), EventStoreError> {
            let mut streams = self.streams.lock().unwrap();
            let stream = streams.entry((user_id, repository_id)).or_default();
            if stream.len() as u64 != expected_version {
                return Err(EventStoreError::ConcurrencyConflict { expected: expected_version, actual: stream.len() as u64 });
            }
            stream.extend(events);
            Ok(())
        }
    }

    struct FakeInvitations {
        by_user: Mutex<HashMap<Uuid, artiferris_domain::invitation::UserInvitation>>,
    }

    impl FakeInvitations {
        fn new() -> Self {
            Self { by_user: Mutex::new(HashMap::new()) }
        }
    }

    #[async_trait]
    impl artiferris_domain::invitation::UserInvitationPort for FakeInvitations {
        async fn upsert(&self, invitation: &artiferris_domain::invitation::UserInvitation, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            self.by_user.lock().unwrap().insert(invitation.user_id, invitation.clone());
            Ok(())
        }
        async fn find_by_token_hash(&self, token_hash: &str) -> Result<Option<artiferris_domain::invitation::UserInvitation>, DomainError> {
            Ok(self.by_user.lock().unwrap().values().find(|i| i.token_hash == token_hash).cloned())
        }
        async fn find_by_user_id(&self, user_id: Uuid) -> Result<Option<artiferris_domain::invitation::UserInvitation>, DomainError> {
            Ok(self.by_user.lock().unwrap().get(&user_id).cloned())
        }
        async fn list_pending_user_ids(&self, user_ids: &[Uuid]) -> Result<std::collections::HashSet<Uuid>, DomainError> {
            let by_user = self.by_user.lock().unwrap();
            Ok(user_ids.iter().filter(|id| by_user.contains_key(id)).copied().collect())
        }
        async fn delete(&self, user_id: Uuid) -> Result<(), DomainError> {
            self.by_user.lock().unwrap().remove(&user_id);
            Ok(())
        }
        async fn redeem(&self, token_hash: &str) -> Result<Option<artiferris_domain::invitation::UserInvitation>, DomainError> {
            let mut by_user = self.by_user.lock().unwrap();
            let user_id = by_user.values().find(|i| i.token_hash == token_hash).map(|i| i.user_id);
            Ok(user_id.and_then(|id| by_user.remove(&id)))
        }
    }

    struct FakeOrganizations {
        by_id: Mutex<HashMap<Uuid, Organization>>,
    }

    impl FakeOrganizations {
        fn new() -> Self {
            Self { by_id: Mutex::new(HashMap::new()) }
        }
    }

    #[async_trait]
    impl OrganizationRepositoryPort for FakeOrganizations {
        async fn create(&self, org: &Organization) -> Result<(), DomainError> {
            self.by_id.lock().unwrap().insert(org.id, org.clone());
            Ok(())
        }
        async fn find_by_id(&self, id: Uuid) -> Result<Option<Organization>, DomainError> {
            Ok(self.by_id.lock().unwrap().get(&id).cloned())
        }
        async fn find_by_slug(&self, slug: &OrganizationSlug) -> Result<Option<Organization>, DomainError> {
            Ok(self.by_id.lock().unwrap().values().find(|o| &o.slug == slug).cloned())
        }
        async fn find_public(&self) -> Result<Organization, DomainError> {
            self.by_id.lock().unwrap().values().find(|o| o.is_public).cloned().ok_or_else(|| DomainError::Infrastructure("no public organization seeded".to_string()))
        }
        async fn list_all(&self) -> Result<Vec<Organization>, DomainError> {
            Ok(self.by_id.lock().unwrap().values().cloned().collect())
        }
    }

    struct FakeHasher;

    #[async_trait]
    impl artiferris_domain::user::PasswordHasherPort for FakeHasher {
        async fn hash(&self, plain_password: &str) -> Result<String, DomainError> {
            Ok(format!("hashed:{plain_password}"))
        }
        async fn verify(&self, plain_password: &str, hash: &str) -> Result<bool, DomainError> {
            Ok(hash == format!("hashed:{plain_password}"))
        }
    }

    /// Fails to send only for addresses in `fail_for`.
    struct FakeEmail {
        sent: Mutex<Vec<(Uuid, String, String, String, String)>>,
        fail_for: std::collections::HashSet<String>,
    }

    impl FakeEmail {
        fn new() -> Self {
            Self { sent: Mutex::new(Vec::new()), fail_for: std::collections::HashSet::new() }
        }
        fn failing_for(addresses: &[&str]) -> Self {
            Self { sent: Mutex::new(Vec::new()), fail_for: addresses.iter().map(|s| s.to_string()).collect() }
        }
    }

    #[async_trait]
    impl artiferris_domain::email::EmailPort for FakeEmail {
        async fn send(&self, organization_id: Uuid, to: &str, subject: &str, text_body: &str, html_body: &str) -> Result<(), DomainError> {
            if self.fail_for.contains(to) {
                return Err(DomainError::Infrastructure("simulated send failure".to_string()));
            }
            self.sent.lock().unwrap().push((organization_id, to.to_string(), subject.to_string(), text_body.to_string(), html_body.to_string()));
            Ok(())
        }
    }

    const TEST_BASE_DOMAIN: &str = "artiferris.example.com";

    /// One public org per distinct organization_id already on `users` — tests needing a non-public restore org build their own `FakeOrganizations` instead.
    fn public_orgs_for(users: &FakeUsers) -> Arc<FakeOrganizations> {
        let organizations = Arc::new(FakeOrganizations::new());
        for organization_id in users.users.lock().unwrap().values().map(|u| u.organization_id).collect::<std::collections::HashSet<_>>() {
            organizations.by_id.lock().unwrap().insert(organization_id, Organization { id: organization_id, slug: OrganizationSlug::parse("public").unwrap(), display_name: "Public".to_string(), is_public: true, is_personal: false, created_at: Utc::now() });
        }
        organizations
    }

    /// Applies a batch to the fakes, or fails like a broken write.
    struct FakeImportStore {
        users: Arc<FakeUsers>,
        repos: Arc<FakePackageRepositoryStore>,
        permissions: Arc<FakePermissionStore>,
        invitations: Arc<FakeInvitations>,
        settings: Arc<FakeSystemSettings>,
        audits: Mutex<Vec<artiferris_domain::audit::AdminAuditRecord>>,
        fail: bool,
    }

    #[async_trait]
    impl ConfigurationImportPort for FakeImportStore {
        async fn apply(&self, batch: &ImportBatch) -> Result<(), DomainError> {
            if self.fail {
                return Err(DomainError::Infrastructure("simulated write failure".to_string()));
            }
            for user in &batch.users {
                self.users.insert(user).await?;
            }
            for stream in &batch.repository_streams {
                self.repos.append(stream.repository_id, stream.expected_version, stream.events.clone(), batch.actor_id).await.map_err(|e| DomainError::Infrastructure(e.to_string()))?;
            }
            for stream in &batch.permission_streams {
                self.permissions.append(stream.user_id, stream.repository_id, 0, stream.events.clone(), batch.actor_id).await.map_err(|e| DomainError::Infrastructure(e.to_string()))?;
            }
            for invitation in &batch.invitations {
                self.invitations.upsert(invitation, None).await?;
            }
            if let Some(settings) = &batch.system_settings {
                self.settings.update(batch.organization_id, settings, None).await?;
            }
            self.audits.lock().unwrap().extend(batch.audit.clone());
            Ok(())
        }
    }

    struct Wiring {
        users: Arc<FakeUsers>,
        repos: Arc<FakePackageRepositoryStore>,
        permissions: Arc<FakePermissionStore>,
        settings: Arc<FakeSystemSettings>,
        invitations: Arc<FakeInvitations>,
        email: Arc<FakeEmail>,
        organizations: Arc<FakeOrganizations>,
        queried_repositories: Arc<dyn PackageRepositoryQueryPort>,
        lock: Arc<dyn RepositoryQuotaLockPort>,
        hasher: Arc<dyn artiferris_domain::user::PasswordHasherPort>,
        fail: bool,
    }

    impl Wiring {
        fn new(users: Arc<FakeUsers>, repos: Arc<FakePackageRepositoryStore>, permissions: Arc<FakePermissionStore>, settings: Arc<FakeSystemSettings>, invitations: Arc<FakeInvitations>, email: Arc<FakeEmail>) -> Self {
            Self {
                organizations: public_orgs_for(&users),
                queried_repositories: repos.clone(),
                lock: Arc::new(FakeRepositoryQuotaLock::new()),
                hasher: Arc::new(FakeHasher),
                fail: false,
                users,
                repos,
                permissions,
                settings,
                invitations,
                email,
            }
        }

        fn build(self) -> (ImportConfigurationUseCase, Arc<FakeImportStore>) {
            let store = Arc::new(FakeImportStore {
                users: self.users.clone(),
                repos: self.repos,
                permissions: self.permissions,
                invitations: self.invitations,
                settings: self.settings,
                audits: Mutex::new(Vec::new()),
                fail: self.fail,
            });
            let use_case = ImportConfigurationUseCase::new(self.users, self.hasher, store.clone(), self.queried_repositories, self.email, self.organizations, self.lock, TEST_BASE_DOMAIN.to_string());
            (use_case, store)
        }
    }

    fn import_use_case(
        users: Arc<FakeUsers>,
        repos: Arc<FakePackageRepositoryStore>,
        permissions: Arc<FakePermissionStore>,
        settings: Arc<FakeSystemSettings>,
        invitations: Arc<FakeInvitations>,
        email: Arc<FakeEmail>,
    ) -> ImportConfigurationUseCase {
        Wiring::new(users, repos, permissions, settings, invitations, email).build().0
    }

    fn sample_import() -> ConfigurationImport {
        let admin_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let hosted_id = Uuid::new_v4();
        ConfigurationImport {
            users: vec![
                ExportedUser { id: admin_id, username: "admin".to_string(), is_super_admin: true, created_at: Utc::now(), email: Some("admin@example.com".to_string()) },
                ExportedUser { id: member_id, username: "member".to_string(), is_super_admin: false, created_at: Utc::now(), email: None },
            ],
            repositories: vec![
                ExportedRepository { id: group_id, name: "my-group".to_string(), format: RepositoryFormat::Npm, repo_type: RepositoryType::Group, remote_url: None, remote_username: None, remote_password: None, group_members: vec![hosted_id], quota_bytes: None, retention_keep_last_n: None },
                ExportedRepository { id: hosted_id, name: "my-hosted".to_string(), format: RepositoryFormat::Npm, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, group_members: vec![], quota_bytes: Some(1024), retention_keep_last_n: Some(5) },
            ],
            permissions: vec![ExportedPermission { user_id: member_id, repository_id: hosted_id, role: Role::Write }],
            system_settings: SystemSettings { max_login_attempts: 7, ..SystemSettings::defaults() },
        }
    }

    #[tokio::test]
    async fn rejects_import_when_a_repository_already_exists() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        Arc::new(CreatePackageRepositoryUseCase::new(repos.clone(), repos.clone())).execute(Uuid::new_v4(), "existing", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, caller_id).await.unwrap();
        let permissions = Arc::new(FakePermissionStore::new());
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = import_use_case(users, repos, permissions, settings, Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));

        let err = use_case.execute(sample_import(), caller_id).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InstanceNotEmpty));
    }

    #[tokio::test]
    async fn rejects_import_when_another_user_already_exists() {
        let caller_id = Uuid::new_v4();
        let other_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![
            User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None },
            User { id: other_id, username: Username::parse("other").unwrap(), password_hash: "h".to_string(), is_super_admin: false, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None },
        ]));
        let use_case = import_use_case(users, Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));

        let err = use_case.execute(sample_import(), caller_id).await.unwrap_err();

        assert!(matches!(err, ApplicationError::InstanceNotEmpty));
    }

    #[tokio::test]
    async fn rejects_import_with_a_clear_error_when_the_acting_admins_own_user_row_is_gone() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![]));
        let use_case = import_use_case(users, Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));

        let err = use_case.execute(sample_import(), caller_id).await.unwrap_err();

        assert!(matches!(err, ApplicationError::ActingAdminNotFound));
    }

    #[tokio::test]
    async fn restores_users_repositories_permissions_and_settings_on_an_empty_instance() {
        let caller_id = Uuid::new_v4();
        let caller_org_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: caller_org_id, created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let permissions = Arc::new(FakePermissionStore::new());
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let invitations = Arc::new(FakeInvitations::new());
        let email = Arc::new(FakeEmail::new());
        let use_case = import_use_case(users.clone(), repos.clone(), permissions.clone(), settings.clone(), invitations.clone(), email.clone());
        let import = sample_import();

        let report = use_case.execute(import.clone(), caller_id).await.unwrap();

        assert_eq!(report.users_created, 2);
        assert_eq!(report.repositories_created, 2);
        assert_eq!(report.permissions_granted, 1);
        assert_eq!(report.invited, vec!["admin".to_string()]);
        assert_eq!(report.skipped_no_email, vec!["member".to_string()]);
        assert!(report.failed.is_empty());

        let restored = repos.list_all().await.unwrap();
        assert_eq!(restored.len(), 2);
        assert!(restored.iter().all(|r| r.id != import.repositories[0].id && r.id != import.repositories[1].id));

        let restored_hosted = restored.iter().find(|r| r.name == "my-hosted").unwrap();
        assert_eq!(restored_hosted.quota_bytes, Some(1024));
        assert_eq!(restored_hosted.retention_keep_last_n, Some(5));
        let restored_group = restored.iter().find(|r| r.name == "my-group").unwrap();
        assert_eq!(restored_group.group_members, vec![restored_hosted.id]);

        let restored_member = users.find_by_id(import.users[1].id).await.unwrap().unwrap();
        assert_eq!(restored_member.username.as_str(), "member");

        let (version, _) = permissions.load(import.users[1].id, restored_hosted.id).await.unwrap();
        assert_eq!(version, 1);
        assert_eq!(settings.settings.lock().unwrap().get(&caller_org_id).unwrap().max_login_attempts, 7);

        let sent = email.sent.lock().unwrap();
        assert!(sent[0].3.contains(&format!("https://app.{TEST_BASE_DOMAIN}/activate?token=")), "expected the public organization's own origin, got: {}", sent[0].3);
    }

    #[tokio::test]
    async fn an_instance_with_a_real_organization_besides_the_public_one_is_refused() {
        let caller_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id, created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let permissions = Arc::new(FakePermissionStore::new());
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let email = Arc::new(FakeEmail::new());
        let organizations = Arc::new(FakeOrganizations::new());
        organizations.create(&Organization { id: organization_id, slug: OrganizationSlug::parse("acme").unwrap(), display_name: "Acme".to_string(), is_public: false, is_personal: false, created_at: Utc::now() }).await.unwrap();
        let mut wiring = Wiring::new(users.clone(), repos.clone(), permissions, settings, Arc::new(FakeInvitations::new()), email.clone());
        wiring.organizations = organizations;
        let (use_case, _) = wiring.build();

        let err = use_case.execute(sample_import(), caller_id).await.unwrap_err();

        assert!(matches!(err, ApplicationError::MultiTenantInstance), "got: {err:?}");
        assert_eq!(users.users.lock().unwrap().len(), 1, "nothing was restored");
        assert!(repos.list_all().await.unwrap().is_empty());
        assert!(email.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_personal_namespace_alongside_the_public_organization_does_not_count_as_multi_tenant() {
        let caller_id = Uuid::new_v4();
        let public_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: public_id, created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let organizations = Arc::new(FakeOrganizations::new());
        organizations.create(&Organization { id: public_id, slug: OrganizationSlug::parse("public").unwrap(), display_name: "Public".to_string(), is_public: true, is_personal: false, created_at: Utc::now() }).await.unwrap();
        organizations.create(&Organization { id: Uuid::new_v4(), slug: OrganizationSlug::parse("alice").unwrap(), display_name: "alice".to_string(), is_public: false, is_personal: true, created_at: Utc::now() }).await.unwrap();
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let permissions = Arc::new(FakePermissionStore::new());
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let mut wiring = Wiring::new(users, repos, permissions, settings, Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        wiring.organizations = organizations;
        let (use_case, _) = wiring.build();

        let report = use_case.execute(sample_import(), caller_id).await.unwrap();

        assert_eq!(report.users_created, 2, "got failures: {:?}", report.failed);
    }

    #[tokio::test]
    async fn restored_users_join_the_acting_admins_own_organization() {
        let caller_id = Uuid::new_v4();
        let caller_org_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User {
            id: caller_id,
            username: Username::parse("caller").unwrap(),
            password_hash: "h".to_string(),
            is_super_admin: true,
            is_organization_admin: false,
            organization_id: caller_org_id,
            created_at: Utc::now(),
            tokens_valid_after: Utc::now(),
            email: None,
        }]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let permissions = Arc::new(FakePermissionStore::new());
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let invitations = Arc::new(FakeInvitations::new());
        let email = Arc::new(FakeEmail::new());
        let use_case = import_use_case(users.clone(), repos, permissions, settings, invitations, email);
        let import = sample_import();

        let report = use_case.execute(import.clone(), caller_id).await.unwrap();
        assert_eq!(report.users_created, 2, "got failures: {:?}", report.failed);

        let restored_admin = users.find_by_id(import.users[0].id).await.unwrap().unwrap();
        let restored_member = users.find_by_id(import.users[1].id).await.unwrap().unwrap();
        assert_eq!(restored_admin.organization_id, caller_org_id, "restored users must join the acting admin's own organization");
        assert_eq!(restored_member.organization_id, caller_org_id, "restored users must join the acting admin's own organization");
    }

    #[tokio::test]
    async fn an_imported_user_with_the_reserved_prefix_is_reported_not_created() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = import_use_case(users, Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), settings, Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let import = ConfigurationImport {
            users: vec![ExportedUser { id: Uuid::new_v4(), username: "artiferris-npm".to_string(), is_super_admin: false, created_at: Utc::now(), email: None }],
            repositories: vec![],
            permissions: vec![],
            system_settings: SystemSettings::defaults(),
        };

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.users_created, 0);
        assert!(report.failed.iter().any(|f| f.contains("artiferris-npm") && f.contains("reserved")), "got: {:?}", report.failed);
    }

    #[tokio::test]
    async fn restoring_a_proxy_repository_also_restores_its_remote_credentials() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let permissions = Arc::new(FakePermissionStore::new());
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = import_use_case(users, repos.clone(), permissions, settings, Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let import = ConfigurationImport {
            users: vec![],
            repositories: vec![ExportedRepository {
                id: Uuid::new_v4(),
                name: "proxy-repo".to_string(),
                format: RepositoryFormat::Npm,
                repo_type: RepositoryType::Proxy,
                remote_url: Some("https://registry.npmjs.org".to_string()),
                remote_username: Some("svc-account".to_string()),
                remote_password: Some("s3cret-upstream-token".to_string()),
                group_members: vec![],
                quota_bytes: None,
                retention_keep_last_n: None,
            }],
            permissions: vec![],
            system_settings: SystemSettings::defaults(),
        };

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.repositories_created, 1, "got failures: {:?}", report.failed);
        let restored = repos.list_all().await.unwrap();
        assert_eq!(restored[0].remote_username.as_deref(), Some("svc-account"));
        assert_eq!(restored[0].remote_password.as_deref(), Some("s3cret-upstream-token"));
        assert!(report.proxy_credentials_needed.is_empty(), "credentials were supplied, nothing to flag");
    }

    #[tokio::test]
    async fn restoring_a_proxy_repository_with_no_credentials_flags_it_in_the_report() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let permissions = Arc::new(FakePermissionStore::new());
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = import_use_case(users, repos.clone(), permissions, settings, Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let import = ConfigurationImport {
            users: vec![],
            repositories: vec![ExportedRepository {
                id: Uuid::new_v4(),
                name: "proxy-repo".to_string(),
                format: RepositoryFormat::Npm,
                repo_type: RepositoryType::Proxy,
                remote_url: Some("https://registry.npmjs.org".to_string()),
                remote_username: None,
                remote_password: None,
                group_members: vec![],
                quota_bytes: None,
                retention_keep_last_n: None,
            }],
            permissions: vec![],
            system_settings: SystemSettings::defaults(),
        };

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.repositories_created, 1, "got failures: {:?}", report.failed);
        assert_eq!(report.proxy_credentials_needed, vec!["proxy-repo".to_string()]);
    }

    #[tokio::test]
    async fn a_restored_user_without_an_email_is_not_invited() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let invitations = Arc::new(FakeInvitations::new());
        let use_case = import_use_case(users, Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), invitations.clone(), Arc::new(FakeEmail::new()));
        let import = sample_import();
        let member_id = import.users[1].id;

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert!(report.invited.iter().all(|u| u != "member"));
        assert!(report.skipped_no_email.contains(&"member".to_string()));
        assert!(invitations.find_by_user_id(member_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_username_collision_with_the_calling_admin_is_reported_not_a_raw_db_error() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("admin").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let use_case = import_use_case(users.clone(), Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let import = sample_import();
        let member_id = import.users[1].id;

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.users_created, 1, "only member should have been created — admin collides with the caller");
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].contains("admin"), "got: {:?}", report.failed[0]);
        assert!(!report.failed[0].to_lowercase().contains("duplicate key"), "must not leak the raw Postgres error: {:?}", report.failed[0]);

        assert!(!report.invited.contains(&"admin".to_string()));
        assert!(!report.skipped_no_email.contains(&"admin".to_string()));
        assert!(users.find_by_id(member_id).await.unwrap().is_some());
        assert_eq!(report.repositories_created, 2);
        assert_eq!(report.permissions_granted, 1);
    }

    #[tokio::test]
    async fn a_user_whose_insert_fails_does_not_get_permissions_or_an_invitation_entry() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("admin").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let mut import = sample_import();
        let admin_export_id = import.users[0].id;
        let hosted_id = import.repositories[1].id;
        import.permissions.push(ExportedPermission { user_id: admin_export_id, repository_id: hosted_id, role: Role::Read });
        let use_case = import_use_case(users, Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.permissions_granted, 1);
        assert!(report.failed.iter().any(|f| f.contains(&admin_export_id.to_string())), "expected a failure entry for the orphaned permission, got: {:?}", report.failed);
    }

    #[tokio::test]
    async fn a_duplicate_repository_name_in_the_export_fails_that_one_repository_but_the_rest_of_the_import_continues() {
        let caller_id = Uuid::new_v4();
        let caller_org_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: caller_org_id, created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let mut import = sample_import();
        let duplicate_name = import.repositories[0].name.clone();
        import.repositories[1].name = duplicate_name.clone();
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = import_use_case(users.clone(), Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), settings.clone(), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let member_id = import.users[1].id;

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.repositories_created, 1);
        assert!(
            report.failed.iter().any(|f| f.contains(&duplicate_name) && f.contains("already taken")),
            "expected a repository-name-taken failure for {duplicate_name:?}, got: {:?}",
            report.failed
        );

        assert_eq!(report.users_created, 2);
        assert!(users.find_by_id(member_id).await.unwrap().is_some());
        assert_eq!(settings.get(caller_org_id).await.unwrap().max_login_attempts, 7);
    }

    #[tokio::test]
    async fn a_failed_invitation_email_is_reported_but_does_not_fail_the_import() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let email = Arc::new(FakeEmail::failing_for(&["admin@example.com"]));
        let use_case = import_use_case(users, Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), email);

        let report = use_case.execute(sample_import(), caller_id).await.unwrap();

        assert!(report.invited.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].contains("admin"));
        assert_eq!(report.users_created, 2);
    }

    #[tokio::test]
    async fn an_imported_user_with_an_invalid_email_is_reported_not_created() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let email = Arc::new(FakeEmail::new());
        let use_case = import_use_case(users.clone(), Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), email.clone());
        let import = ConfigurationImport {
            users: vec![
                ExportedUser { id: Uuid::new_v4(), username: "injected".to_string(), is_super_admin: false, created_at: Utc::now(), email: Some("a@b.com\r\nBcc: victim@evil.com".to_string()) },
                ExportedUser { id: Uuid::new_v4(), username: "no-at-sign".to_string(), is_super_admin: false, created_at: Utc::now(), email: Some("not-an-email".to_string()) },
                ExportedUser { id: Uuid::new_v4(), username: "fine".to_string(), is_super_admin: false, created_at: Utc::now(), email: Some("fine@example.com".to_string()) },
            ],
            repositories: vec![],
            permissions: vec![],
            system_settings: SystemSettings::defaults(),
        };

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.users_created, 1);
        assert_eq!(report.failed.len(), 2, "got: {:?}", report.failed);
        assert!(report.failed.iter().all(|f| f.contains("email")), "got: {:?}", report.failed);
        assert_eq!(email.sent.lock().unwrap().len(), 1, "only the valid address is mailed");
        assert!(users.users.lock().unwrap().values().all(|u| u.username.as_str() != "injected" && u.username.as_str() != "no-at-sign"));
    }

    /// Serializes for real, unlike `FakeRepositoryQuotaLock`.
    struct SerializingLock(Arc<futures::lock::Mutex<()>>);

    struct HeldLock(#[allow(dead_code)] futures::lock::OwnedMutexGuard<()>);
    impl artiferris_domain::package_repository::RepositoryLockGuard for HeldLock {}

    #[async_trait]
    impl RepositoryQuotaLockPort for SerializingLock {
        async fn acquire_repository_lock(&self, _repository_id: Uuid) -> Result<Box<dyn artiferris_domain::package_repository::RepositoryLockGuard>, EventStoreError> {
            Ok(Box::new(HeldLock(self.0.clone().lock_owned().await)))
        }
    }

    /// Yields after answering, so a second import can run its own emptiness check before the first one has written anything.
    struct SlowRepositories(Arc<FakePackageRepositoryStore>);

    #[async_trait]
    impl PackageRepositoryQueryPort for SlowRepositories {
        async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
            self.0.find_by_id(id).await
        }
        async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
            self.0.find_by_org_and_name(organization_id, name).await
        }
        async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
            let listed = self.0.list_all().await;
            tokio::task::yield_now().await;
            listed
        }
        async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
            self.0.list_by_organization(organization_id).await
        }
    }

    #[tokio::test]
    async fn two_imports_started_together_cannot_both_find_the_instance_empty() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![User { id: caller_id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let permissions = Arc::new(FakePermissionStore::new());
        let lock = Arc::new(futures::lock::Mutex::new(()));
        let use_case = |users: Arc<FakeUsers>| {
            let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
            let mut wiring = Wiring::new(users, repos.clone(), permissions.clone(), settings, Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
            wiring.queried_repositories = Arc::new(SlowRepositories(repos.clone()));
            wiring.lock = Arc::new(SerializingLock(lock.clone()));
            wiring.build().0
        };
        let (first, second) = (use_case(users.clone()), use_case(users.clone()));

        let repositories_only = || ConfigurationImport { users: vec![], permissions: vec![], ..sample_import() };

        let (a, b) = tokio::join!(first.execute(repositories_only(), caller_id), second.execute(repositories_only(), caller_id));

        let outcomes = [a, b];
        assert_eq!(outcomes.iter().filter(|o| o.is_ok()).count(), 1, "exactly one import may win: {outcomes:?}");
        assert!(outcomes.iter().any(|o| matches!(o, Err(ApplicationError::InstanceNotEmpty))), "the other finds the instance already populated: {outcomes:?}");
    }

    fn caller(id: Uuid) -> User {
        User { id, username: Username::parse("caller").unwrap(), password_hash: "h".to_string(), is_super_admin: true, is_organization_admin: false, organization_id: Uuid::new_v4(), created_at: Utc::now(), tokens_valid_after: Utc::now(), email: None }
    }

    #[tokio::test]
    async fn a_failed_write_leaves_the_instance_untouched_and_mails_nobody() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![caller(caller_id)]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let email = Arc::new(FakeEmail::new());
        let mut wiring = Wiring::new(users.clone(), repos.clone(), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), email.clone());
        wiring.fail = true;
        let (use_case, store) = wiring.build();

        let err = use_case.execute(sample_import(), caller_id).await.unwrap_err();

        assert!(matches!(err, ApplicationError::Domain(DomainError::Infrastructure(_))), "{err:?}");
        assert_eq!(users.users.lock().unwrap().len(), 1);
        assert!(repos.list_all().await.unwrap().is_empty());
        assert!(email.sent.lock().unwrap().is_empty());
        assert!(store.audits.lock().unwrap().is_empty(), "nothing happened, so nothing is recorded");
    }

    #[tokio::test]
    async fn the_import_is_recorded_with_its_counts_in_the_same_write() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![caller(caller_id)]));
        let mut import = sample_import();
        import.users.push(ExportedUser { id: Uuid::new_v4(), username: "artiferris-npm".to_string(), is_super_admin: false, created_at: Utc::now(), email: None });
        let wiring = Wiring::new(users, Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let (use_case, store) = wiring.build();

        use_case.execute(import, caller_id).await.unwrap();

        let audits = store.audits.lock().unwrap();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].actor_id, Some(caller_id));
        assert!(matches!(audits[0].event, AdminAuditEvent::ConfigurationImported { users_created: 2, repositories_created: 2, permissions_granted: 1, failures: 1 }), "{:?}", audits[0].event);
    }

    struct CountingHasher(std::sync::atomic::AtomicUsize);

    #[async_trait]
    impl artiferris_domain::user::PasswordHasherPort for CountingHasher {
        async fn hash(&self, plain_password: &str) -> Result<String, DomainError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(format!("hashed:{plain_password}"))
        }
        async fn verify(&self, _plain_password: &str, _hash: &str) -> Result<bool, DomainError> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn every_restored_user_shares_one_unusable_password_hash() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![caller(caller_id)]));
        let hasher = Arc::new(CountingHasher(std::sync::atomic::AtomicUsize::new(0)));
        let mut wiring = Wiring::new(users.clone(), Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        wiring.hasher = hasher.clone();
        let (use_case, _) = wiring.build();
        let import = ConfigurationImport {
            users: (0..5).map(|n| ExportedUser { id: Uuid::new_v4(), username: format!("user-{n}"), is_super_admin: false, created_at: Utc::now(), email: None }).collect(),
            repositories: vec![],
            permissions: vec![],
            system_settings: SystemSettings::defaults(),
        };

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.users_created, 5);
        assert_eq!(hasher.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let hashes: HashSet<String> = users.users.lock().unwrap().values().filter(|u| u.id != caller_id).map(|u| u.password_hash.clone()).collect();
        assert_eq!(hashes.len(), 1);
    }

    #[tokio::test]
    async fn entries_that_would_collide_inside_the_file_are_left_out_instead_of_failing_the_whole_write() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![caller(caller_id)]));
        let (use_case, _) = Wiring::new(users.clone(), Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new())).build();
        let (first, twin) = (Uuid::new_v4(), Uuid::new_v4());
        let import = ConfigurationImport {
            users: vec![
                ExportedUser { id: first, username: "alice".to_string(), is_super_admin: false, created_at: Utc::now(), email: None },
                ExportedUser { id: twin, username: "Alice".to_string(), is_super_admin: false, created_at: Utc::now(), email: None },
                ExportedUser { id: first, username: "bob".to_string(), is_super_admin: false, created_at: Utc::now(), email: None },
                ExportedUser { id: caller_id, username: "carol".to_string(), is_super_admin: false, created_at: Utc::now(), email: None },
            ],
            repositories: vec![],
            permissions: vec![],
            system_settings: SystemSettings::defaults(),
        };

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.users_created, 1);
        assert_eq!(report.failed.len(), 3, "{:?}", report.failed);
        assert_eq!(users.users.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_group_member_of_another_format_or_the_group_itself_is_left_out_and_reported() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![caller(caller_id)]));
        let repos = Arc::new(FakePackageRepositoryStore::new());
        let (use_case, _) = Wiring::new(users, repos.clone(), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new())).build();
        let (group_id, docker_id, npm_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let repository = |id, name: &str, format, repo_type, members: Vec<Uuid>| ExportedRepository { id, name: name.to_string(), format, repo_type, remote_url: None, remote_username: None, remote_password: None, group_members: members, quota_bytes: None, retention_keep_last_n: None };
        let import = ConfigurationImport {
            users: vec![],
            repositories: vec![
                repository(group_id, "the-group", RepositoryFormat::Npm, RepositoryType::Group, vec![docker_id, group_id, npm_id]),
                repository(docker_id, "images", RepositoryFormat::Docker, RepositoryType::Hosted, vec![]),
                repository(npm_id, "libs", RepositoryFormat::Npm, RepositoryType::Hosted, vec![]),
            ],
            permissions: vec![],
            system_settings: SystemSettings::defaults(),
        };

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.repositories_created, 3);
        assert_eq!(report.failed.len(), 2, "{:?}", report.failed);
        let restored = repos.list_all().await.unwrap();
        let libs = restored.iter().find(|r| r.name == "libs").unwrap();
        assert_eq!(restored.iter().find(|r| r.name == "the-group").unwrap().group_members, vec![libs.id]);
    }

    #[tokio::test]
    async fn out_of_range_settings_are_reported_and_the_rest_is_still_imported() {
        let caller_id = Uuid::new_v4();
        let users = Arc::new(FakeUsers::seeded(vec![caller(caller_id)]));
        let settings = Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) });
        let use_case = import_use_case(users.clone(), Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), settings.clone(), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let mut import = sample_import();
        import.system_settings.max_login_attempts = 0;

        let report = use_case.execute(import, caller_id).await.unwrap();

        assert_eq!(report.users_created, 2);
        assert!(report.failed.iter().any(|f| f.starts_with("system settings")), "{:?}", report.failed);
        assert!(settings.settings.lock().unwrap().is_empty());
    }

    fn plan_context(actor_id: Uuid) -> PlanContext {
        PlanContext { actor_id, actor_username: "caller".to_string(), organization_id: Uuid::new_v4(), activation_origin: "https://artiferris.example.com".to_string(), unusable_password_hash: "unusable".to_string() }
    }

    #[test]
    fn planning_twenty_thousand_entries_of_each_kind_is_fast() {
        let users: Vec<ExportedUser> = (0..20_000).map(|i| ExportedUser { id: Uuid::new_v4(), username: format!("user{i}"), is_super_admin: false, created_at: Utc::now(), email: Some(format!("user{i}@example.com")) }).collect();
        let repositories: Vec<ExportedRepository> = (0..20_000)
            .map(|i| ExportedRepository { id: Uuid::new_v4(), name: format!("repo{i}"), format: RepositoryFormat::Npm, repo_type: RepositoryType::Hosted, remote_url: None, remote_username: None, remote_password: None, group_members: vec![], quota_bytes: None, retention_keep_last_n: None })
            .collect();
        let permissions: Vec<ExportedPermission> = (0..20_000).map(|i| ExportedPermission { user_id: users[i].id, repository_id: repositories[i].id, role: Role::Write }).collect();
        let import = ConfigurationImport { users, repositories, permissions, system_settings: SystemSettings::defaults() };

        let started = std::time::Instant::now();
        let plan = plan_import(import, plan_context(Uuid::new_v4()));

        assert!(started.elapsed() < std::time::Duration::from_secs(5), "planning took {:?}", started.elapsed());
        assert!(plan.report.failed.is_empty(), "{:?}", &plan.report.failed[..plan.report.failed.len().min(3)]);
        assert_eq!((plan.batch.users.len(), plan.batch.repository_streams.len(), plan.batch.permission_streams.len(), plan.pending_invitations.len()), (20_000, 20_000, 20_000, 20_000));
    }

    #[tokio::test]
    async fn a_file_with_more_entries_than_the_cap_is_refused_before_anything_is_read() {
        let caller_id = Uuid::new_v4();
        let use_case = import_use_case(Arc::new(FakeUsers::seeded(vec![caller(caller_id)])), Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let mut import = sample_import();
        import.permissions = (0..=MAX_IMPORT_ENTRIES).map(|_| ExportedPermission { user_id: Uuid::nil(), repository_id: Uuid::nil(), role: Role::Read }).collect();

        let error = use_case.execute(import, caller_id).await.unwrap_err();

        assert!(matches!(&error, ApplicationError::ImportTooLarge(message) if message.contains("permissions")), "{error}");
    }

    /// Counts how many sends overlap.
    struct SlowEmail {
        in_flight: std::sync::atomic::AtomicUsize,
        most_in_flight: std::sync::atomic::AtomicUsize,
        sent: Mutex<Vec<String>>,
        fail_for: String,
    }

    impl SlowEmail {
        fn new(fail_for: &str) -> Self {
            Self { in_flight: 0.into(), most_in_flight: 0.into(), sent: Mutex::new(Vec::new()), fail_for: fail_for.to_string() }
        }
    }

    #[async_trait]
    impl artiferris_domain::email::EmailPort for SlowEmail {
        async fn send(&self, _organization_id: Uuid, to: &str, _subject: &str, _text_body: &str, _html_body: &str) -> Result<(), DomainError> {
            use std::sync::atomic::Ordering;
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_in_flight.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if to == self.fail_for {
                return Err(DomainError::Infrastructure("simulated send failure".to_string()));
            }
            self.sent.lock().unwrap().push(to.to_string());
            Ok(())
        }
    }

    fn import_of_users_with_email(count: usize) -> ConfigurationImport {
        ConfigurationImport {
            users: (0..count).map(|i| ExportedUser { id: Uuid::new_v4(), username: format!("user{i}"), is_super_admin: false, created_at: Utc::now(), email: Some(format!("user{i}@example.com")) }).collect(),
            repositories: vec![],
            permissions: vec![],
            system_settings: SystemSettings::defaults(),
        }
    }

    fn use_case_with_email(caller_id: Uuid, email: Arc<dyn artiferris_domain::email::EmailPort>) -> ImportConfigurationUseCase {
        let users = Arc::new(FakeUsers::seeded(vec![caller(caller_id)]));
        let wiring = Wiring::new(users, Arc::new(FakePackageRepositoryStore::new()), Arc::new(FakePermissionStore::new()), Arc::new(FakeSystemSettings { settings: Mutex::new(HashMap::new()) }), Arc::new(FakeInvitations::new()), Arc::new(FakeEmail::new()));
        let (mut use_case, _) = wiring.build();
        use_case.email = email;
        use_case
    }

    #[tokio::test]
    async fn invitation_emails_go_out_a_few_at_a_time_and_each_failure_is_collected() {
        let caller_id = Uuid::new_v4();
        let email = Arc::new(SlowEmail::new("user3@example.com"));
        let use_case = use_case_with_email(caller_id, email.clone());

        let report = use_case.execute(import_of_users_with_email(24), caller_id).await.unwrap();

        let most = email.most_in_flight.load(std::sync::atomic::Ordering::SeqCst);
        assert!((2..=MAIL_CONCURRENCY).contains(&most), "{most} sends overlapped");
        assert_eq!(report.invited.len(), 23);
        assert_eq!(report.invited[..3], ["user0", "user1", "user2"], "the report keeps the file's order");
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].contains("user3"), "{:?}", report.failed);
    }

    #[tokio::test]
    async fn a_client_that_gives_up_mid_import_does_not_leave_invitations_unsent() {
        let caller_id = Uuid::new_v4();
        let email = Arc::new(SlowEmail::new("nobody"));
        let use_case = use_case_with_email(caller_id, email.clone());
        let request = tokio::spawn(async move { use_case.execute(import_of_users_with_email(20), caller_id).await });
        while email.in_flight.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        request.abort();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while email.sent.lock().unwrap().len() < 20 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the invitations still went out");
    }
}
