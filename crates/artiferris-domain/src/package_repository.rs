use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{DomainError, EventStoreError};
use crate::reserved_names::reject_reserved_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryFormat {
    Npm,
    Docker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryType {
    Hosted,
    Proxy,
    Group,
}

/// Definition-time URL-shape validation (M-2) — parseable and http/https only, no embedded credentials (those belong in the separate username/password fields, which are never echoed back), and https when credentials are stored: the
/// credentials would otherwise cross the network in the clear. This is independent of, and layered on top of, the
/// Proxy-only type guard already enforced by `create`/`change_remote_url`'s callers below. It does NOT resolve the host or
/// check for private/reserved addresses — that's `ssrf_guard::ensure_public_host`'s job at fetch time, where DNS is actually
/// available; this is just rejecting obviously-wrong `remote_url` values as early as possible.
fn validate_remote_url(url: &str, has_credentials: bool) -> Result<(), DomainError> {
    let parsed = url::Url::parse(url).map_err(|e| DomainError::InvalidRemoteUrl(format!("{url} is not a valid URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DomainError::InvalidRemoteUrl(format!("remote_url scheme must be http or https, got: {}", parsed.scheme())));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(DomainError::InvalidRemoteUrl("remote_url must not embed a username or password; use the separate credential fields".to_string()));
    }
    if has_credentials && parsed.scheme() != "https" {
        return Err(DomainError::InvalidRemoteUrl("remote_url must be https when credentials are set".to_string()));
    }
    Ok(())
}

pub fn parse_repository_name(raw: &str) -> Result<String, DomainError> {
    let len_ok = (2..=64).contains(&raw.len());
    // "." and ".." pass every char check below, so reject them explicitly too (matches
    // NpmPackageName::validate_segment's guard for the same reason).
    let not_dots = raw != "." && raw != "..";
    let chars_ok = raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if len_ok && not_dots && chars_ok {
        reject_reserved_name(raw)?;
        Ok(raw.to_string())
    } else {
        Err(DomainError::InvalidRepositoryName(raw.to_string()))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum PackageRepositoryEvent {
    Created {
        repository_id: Uuid,
        organization_id: Uuid,
        name: String,
        format: RepositoryFormat,
        repo_type: RepositoryType,
        remote_url: Option<String>,
        remote_username: Option<String>,
        /// Basic-auth password, or a bearer token when `remote_username` is `None`. Never returned by the API.
        remote_password: Option<String>,
    },
    Renamed { repository_id: Uuid, new_name: String },
    RemoteUrlChanged { repository_id: Uuid, remote_url: String },
    GroupMemberAdded { repository_id: Uuid, member_repository_id: Uuid, position: i32 },
    GroupMemberRemoved { repository_id: Uuid, member_repository_id: Uuid },
    /// `None` means unlimited.
    QuotaSet { repository_id: Uuid, quota_bytes: Option<i64> },
    /// `None` disables the background retention sweep for this repository.
    RetentionPolicySet { repository_id: Uuid, keep_last_n_versions: Option<i32> },
    VisibilityChanged { repository_id: Uuid, is_public: bool },
    Deleted { repository_id: Uuid },
}

impl std::fmt::Debug for PackageRepositoryEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created { repository_id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password } => f
                .debug_struct("Created")
                .field("repository_id", repository_id)
                .field("organization_id", organization_id)
                .field("name", name)
                .field("format", format)
                .field("repo_type", repo_type)
                .field("remote_url", remote_url)
                .field("remote_username", remote_username)
                .field("remote_password", &remote_password.as_ref().map(|_| "[redacted]"))
                .finish(),
            Self::Renamed { repository_id, new_name } => f.debug_struct("Renamed").field("repository_id", repository_id).field("new_name", new_name).finish(),
            Self::RemoteUrlChanged { repository_id, remote_url } => f.debug_struct("RemoteUrlChanged").field("repository_id", repository_id).field("remote_url", remote_url).finish(),
            Self::GroupMemberAdded { repository_id, member_repository_id, position } => f
                .debug_struct("GroupMemberAdded")
                .field("repository_id", repository_id)
                .field("member_repository_id", member_repository_id)
                .field("position", position)
                .finish(),
            Self::GroupMemberRemoved { repository_id, member_repository_id } => {
                f.debug_struct("GroupMemberRemoved").field("repository_id", repository_id).field("member_repository_id", member_repository_id).finish()
            }
            Self::QuotaSet { repository_id, quota_bytes } => f.debug_struct("QuotaSet").field("repository_id", repository_id).field("quota_bytes", quota_bytes).finish(),
            Self::RetentionPolicySet { repository_id, keep_last_n_versions } => {
                f.debug_struct("RetentionPolicySet").field("repository_id", repository_id).field("keep_last_n_versions", keep_last_n_versions).finish()
            }
            Self::VisibilityChanged { repository_id, is_public } => f.debug_struct("VisibilityChanged").field("repository_id", repository_id).field("is_public", is_public).finish(),
            Self::Deleted { repository_id } => f.debug_struct("Deleted").field("repository_id", repository_id).finish(),
        }
    }
}

impl PackageRepositoryEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            PackageRepositoryEvent::Created { .. } => "Created",
            PackageRepositoryEvent::Renamed { .. } => "Renamed",
            PackageRepositoryEvent::RemoteUrlChanged { .. } => "RemoteUrlChanged",
            PackageRepositoryEvent::GroupMemberAdded { .. } => "GroupMemberAdded",
            PackageRepositoryEvent::GroupMemberRemoved { .. } => "GroupMemberRemoved",
            PackageRepositoryEvent::QuotaSet { .. } => "QuotaSet",
            PackageRepositoryEvent::RetentionPolicySet { .. } => "RetentionPolicySet",
            PackageRepositoryEvent::VisibilityChanged { .. } => "VisibilityChanged",
            PackageRepositoryEvent::Deleted { .. } => "Deleted",
        }
    }
}

#[derive(Clone, Default)]
pub struct PackageRepository {
    pub name: Option<String>,
    pub format: Option<RepositoryFormat>,
    pub repo_type: Option<RepositoryType>,
    pub remote_url: Option<String>,
    pub remote_username: Option<String>,
    pub remote_password: Option<String>,
    pub group_members: Vec<(Uuid, i32)>,
    pub quota_bytes: Option<i64>,
    pub retention_keep_last_n: Option<i32>,
    pub is_public: bool,
    pub deleted: bool,
}

impl std::fmt::Debug for PackageRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackageRepository")
            .field("name", &self.name)
            .field("format", &self.format)
            .field("repo_type", &self.repo_type)
            .field("remote_url", &self.remote_url)
            .field("remote_username", &self.remote_username)
            .field("remote_password", &self.remote_password.as_ref().map(|_| "[redacted]"))
            .field("group_members", &self.group_members)
            .field("quota_bytes", &self.quota_bytes)
            .field("retention_keep_last_n", &self.retention_keep_last_n)
            .field("is_public", &self.is_public)
            .field("deleted", &self.deleted)
            .finish()
    }
}

impl PackageRepository {
    pub fn from_events(events: &[PackageRepositoryEvent]) -> Self {
        let mut state = Self::default();
        for event in events {
            state.apply(event);
        }
        state
    }

    pub fn apply(&mut self, event: &PackageRepositoryEvent) {
        match event {
            PackageRepositoryEvent::Created { name, format, repo_type, remote_url, remote_username, remote_password, .. } => {
                self.name = Some(name.clone());
                self.format = Some(*format);
                self.repo_type = Some(*repo_type);
                self.remote_url = remote_url.clone();
                self.remote_username = remote_username.clone();
                self.remote_password = remote_password.clone();
            }
            PackageRepositoryEvent::Renamed { new_name, .. } => self.name = Some(new_name.clone()),
            PackageRepositoryEvent::RemoteUrlChanged { remote_url, .. } => {
                self.remote_url = Some(remote_url.clone())
            }
            PackageRepositoryEvent::GroupMemberAdded { member_repository_id, position, .. } => {
                self.group_members.retain(|(id, _)| id != member_repository_id);
                self.group_members.push((*member_repository_id, *position));
            }
            PackageRepositoryEvent::GroupMemberRemoved { member_repository_id, .. } => {
                self.group_members.retain(|(id, _)| id != member_repository_id);
            }
            PackageRepositoryEvent::QuotaSet { quota_bytes, .. } => self.quota_bytes = *quota_bytes,
            // `set_retention_policy` rejects `Some(n)` with `n < 1` at construction time, but this
            // replays trusted history unconditionally (B-37) — sanitize here too, in case an event
            // written before that guard existed still carries a `Some(0)` (or negative) count.
            PackageRepositoryEvent::RetentionPolicySet { keep_last_n_versions, .. } => {
                self.retention_keep_last_n = keep_last_n_versions.map(|n| n.max(1))
            }
            PackageRepositoryEvent::VisibilityChanged { is_public, .. } => self.is_public = *is_public,
            PackageRepositoryEvent::Deleted { .. } => self.deleted = true,
        }
    }

    fn ensure_mutable(&self) -> Result<(), DomainError> {
        if self.deleted {
            Err(DomainError::AlreadyDeleted)
        } else {
            Ok(())
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        repository_id: Uuid,
        organization_id: Uuid,
        name: String,
        format: RepositoryFormat,
        repo_type: RepositoryType,
        remote_url: Option<String>,
        remote_username: Option<String>,
        remote_password: Option<String>,
    ) -> Result<PackageRepositoryEvent, DomainError> {
        if remote_url.is_some() && repo_type != RepositoryType::Proxy {
            return Err(DomainError::InvalidForRepositoryType(
                "remote_url can only be set on a proxy repository".to_string(),
            ));
        }
        if let Some(url) = &remote_url {
            validate_remote_url(url, remote_username.is_some() || remote_password.is_some())?;
        }
        Ok(PackageRepositoryEvent::Created { repository_id, organization_id, name, format, repo_type, remote_url, remote_username, remote_password })
    }

    /// `repository_id` is `Uuid::nil()` here (and in every mutator below) — this aggregate doesn't track its own id; the application layer fills in the real one before persisting.
    pub fn rename(&self, new_name: String) -> Result<PackageRepositoryEvent, DomainError> {
        self.ensure_mutable()?;
        Ok(PackageRepositoryEvent::Renamed { repository_id: Uuid::nil(), new_name })
    }

    pub fn change_remote_url(&self, remote_url: String) -> Result<PackageRepositoryEvent, DomainError> {
        self.ensure_mutable()?;
        if self.repo_type != Some(RepositoryType::Proxy) {
            return Err(DomainError::InvalidForRepositoryType(
                "remote_url can only be set on a proxy repository".to_string(),
            ));
        }
        validate_remote_url(&remote_url, self.remote_username.is_some() || self.remote_password.is_some())?;
        Ok(PackageRepositoryEvent::RemoteUrlChanged { repository_id: Uuid::nil(), remote_url })
    }

    pub fn add_group_member(&self, member_repository_id: Uuid, position: i32) -> Result<PackageRepositoryEvent, DomainError> {
        self.ensure_mutable()?;
        if self.repo_type != Some(RepositoryType::Group) {
            return Err(DomainError::InvalidForRepositoryType(
                "group members can only be added to a group repository".to_string(),
            ));
        }
        Ok(PackageRepositoryEvent::GroupMemberAdded {
            repository_id: Uuid::nil(),
            member_repository_id,
            position,
        })
    }

    pub fn remove_group_member(&self, member_repository_id: Uuid) -> Result<PackageRepositoryEvent, DomainError> {
        self.ensure_mutable()?;
        Ok(PackageRepositoryEvent::GroupMemberRemoved { repository_id: Uuid::nil(), member_repository_id })
    }

    pub fn delete(&self) -> Result<PackageRepositoryEvent, DomainError> {
        self.ensure_mutable()?;
        Ok(PackageRepositoryEvent::Deleted { repository_id: Uuid::nil() })
    }

    pub fn set_quota(&self, quota_bytes: Option<i64>) -> Result<PackageRepositoryEvent, DomainError> {
        self.ensure_mutable()?;
        if let Some(bytes) = quota_bytes
            && bytes < 0
        {
            return Err(DomainError::Validation("quota_bytes must not be negative".to_string()));
        }
        Ok(PackageRepositoryEvent::QuotaSet { repository_id: Uuid::nil(), quota_bytes })
    }

    /// Keeps the `n` most recent versions/tags; the sweep deletes the rest.
    pub fn set_retention_policy(&self, keep_last_n_versions: Option<i32>) -> Result<PackageRepositoryEvent, DomainError> {
        self.ensure_mutable()?;
        if let Some(n) = keep_last_n_versions
            && n < 1
        {
            return Err(DomainError::Validation("keep_last_n_versions must be at least 1".to_string()));
        }
        Ok(PackageRepositoryEvent::RetentionPolicySet { repository_id: Uuid::nil(), keep_last_n_versions })
    }

    pub fn set_visibility(&self, is_public: bool) -> Result<PackageRepositoryEvent, DomainError> {
        self.ensure_mutable()?;
        // A public proxy is an anonymous relay to its upstream using the repository's own stored credentials,
        // and a public group would be served with no token in front of members that are checked one by one.
        //
        // Making a repository private again is always allowed, whatever its type.
        if is_public && self.repo_type != Some(RepositoryType::Hosted) {
            return Err(DomainError::InvalidForRepositoryType(
                "only a hosted repository can be made public".to_string(),
            ));
        }
        Ok(PackageRepositoryEvent::VisibilityChanged { repository_id: Uuid::nil(), is_public })
    }
}

#[derive(Clone)]
pub struct PackageRepositorySummary {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub format: RepositoryFormat,
    pub repo_type: RepositoryType,
    pub remote_url: Option<String>,
    pub remote_username: Option<String>,
    /// Never returned by the API.
    pub remote_password: Option<String>,
    pub group_members: Vec<Uuid>,
    /// `None` means unlimited.
    pub quota_bytes: Option<i64>,
    /// `None` means automatic cleanup is disabled for this repository.
    pub retention_keep_last_n: Option<i32>,
    pub is_public: bool,
}

impl std::fmt::Debug for PackageRepositorySummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackageRepositorySummary")
            .field("id", &self.id)
            .field("organization_id", &self.organization_id)
            .field("name", &self.name)
            .field("format", &self.format)
            .field("repo_type", &self.repo_type)
            .field("remote_url", &self.remote_url)
            .field("remote_username", &self.remote_username)
            .field("remote_password", &self.remote_password.as_ref().map(|_| "[redacted]"))
            .field("group_members", &self.group_members)
            .field("quota_bytes", &self.quota_bytes)
            .field("retention_keep_last_n", &self.retention_keep_last_n)
            .field("is_public", &self.is_public)
            .finish()
    }
}

#[async_trait]
pub trait PackageRepositoryEventStorePort: Send + Sync {
    async fn load(&self, repository_id: Uuid) -> Result<(u64, Vec<PackageRepositoryEvent>), EventStoreError>;

    async fn append(
        &self,
        repository_id: Uuid,
        expected_version: u64,
        events: Vec<PackageRepositoryEvent>,
        actor_id: Uuid,
    ) -> Result<(), EventStoreError>;
}

#[async_trait]
pub trait PackageRepositoryQueryPort: Send + Sync {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError>;
    async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError>;
    async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError>;
    /// Scoped at the database level — do not implement by calling `list_all()` and filtering in memory.
    async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError>;
}

/// A held per-repository advisory lock (`pg_advisory_xact_lock(hashtext(repository_id))`, same
/// convention as Docker's B-18 fix). Serializes concurrent quota-affecting writes to one
/// repository. Dropping the guard releases it — there's nothing else to commit.
pub trait RepositoryLockGuard: Send {}

#[async_trait]
pub trait RepositoryQuotaLockPort: Send + Sync {
    async fn acquire_repository_lock(&self, repository_id: Uuid) -> Result<Box<dyn RepositoryLockGuard>, EventStoreError>;
}

/// Result of `RepositoryDeletionSweepPort::hard_delete_repositories_past_grace_period`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HardDeleteSweepResult {
    pub repositories_removed: usize,
    /// The id of every repository this sweep actually hard-deleted — the use case removes each
    /// one's on-disk storage directory (npm tarballs; a no-op for a Docker-format repository, which
    /// never has one) once this has committed (Finding 2, B-39 fix round 1).
    pub swept_repository_ids: Vec<Uuid>,
    /// Digests of Docker blobs whose `docker_blobs` row was deleted (reference count hit zero, with
    /// no remaining `docker_repository_blobs` link) as part of the SAME transaction as each
    /// repository's cascade delete (Finding 3, B-39 fix round 1). Covers both manifest-backed digests
    /// and digests this repository held only a link to (no manifest ever referenced them — e.g. a
    /// proxy repository's blob cache) — otherwise a link-only digest's cascade-removed row would
    /// never become reclaimable by any future sweep. The use case best-effort removes the files
    /// themselves via `DockerBlobStorePort::remove_reclaimed_blob_files`, which re-acquires each
    /// digest's advisory lock and re-confirms its row is still absent before touching disk.
    pub reclaimed_docker_blob_digests: Vec<String>,
}

/// A narrow, dedicated port (same granularity as `DockerUploadSessionPort`) rather than a new method
/// on the much-implemented `PackageRepositoryQueryPort` — this keeps the change confined to the one
/// production adapter plus this use case's own test fake, instead of every other fake across the
/// codebase that implements the query port.
#[async_trait]
pub trait RepositoryDeletionSweepPort: Send + Sync {
    /// Hard-deletes every repository soft-deleted (`Deleted` event, `deleted_at` set) more than the
    /// 30-day grace period ago, one repository at a time in its own transaction (not one batched
    /// statement for the whole set — see the production adapter's implementation for why), letting
    /// the schema's own `ON DELETE CASCADE` reclaim dependent rows (`npm_packages`, `docker_manifests`,
    /// `docker_tags`, `docker_repository_blobs`, `docker_blob_uploads`, and the repository's own
    /// `package_repository_group_members` rows as a group) that a soft delete alone never reaches
    /// (B-39). It also removes any `package_repository_group_members` row where this repository is
    /// the MEMBER of some other, still-live group — that FK has no `ON DELETE` action, so without
    /// this the hard delete would fail outright for any repository still listed as a group member
    /// (fix round 1, Finding 1).
    ///
    /// The cascade does NOT reach `docker_blobs`: that table is deliberately not FK'd to
    /// `package_repository_projections` (it's globally content-addressed and deduped across every
    /// repository — see its schema comment), and its `reference_count` is maintained entirely by
    /// application code (`insert_manifest_with_checks` increments, `DeleteManifestUseCase` decrements),
    /// never a DB trigger. So each repository's own transaction also decrements `reference_count` for
    /// every blob its manifests referenced, and deletes any blob row that reaches zero — including a
    /// blob this repository only ever held a link to, never a manifest reference — all before that
    /// transaction commits, so this bookkeeping is never lost even if the process crashes right
    /// after. The result reports which blobs' on-disk files the caller should now remove (best-effort,
    /// via `DockerBlobStorePort::remove_reclaimed_blob_files`, which re-locks and re-checks each
    /// digest before touching its file), and which repositories' own on-disk storage directories it
    /// should remove (fix round 1, Finding 2, via `StorageBackendPort::delete_repository`) — both
    /// only meaningful post-commit.
    async fn hard_delete_repositories_past_grace_period(&self) -> Result<HardDeleteSweepResult, DomainError>;
}

#[cfg(test)]
mod redacted_debug_tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_remote_password() {
        let id = Uuid::new_v4();
        let created = PackageRepositoryEvent::Created {
            repository_id: id,
            organization_id: Uuid::new_v4(),
            name: "proxy".to_string(),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Proxy,
            remote_url: Some("https://registry.example.com".to_string()),
            remote_username: Some("svc".to_string()),
            remote_password: Some("upstream-secret-value".to_string()),
        };
        let aggregate = PackageRepository::from_events(std::slice::from_ref(&created));
        let summary = PackageRepositorySummary {
            id,
            organization_id: Uuid::new_v4(),
            name: "proxy".to_string(),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Proxy,
            remote_url: None,
            remote_username: None,
            remote_password: Some("upstream-secret-value".to_string()),
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
        };

        let printed = format!("{created:?} {aggregate:?} {summary:?} {:?}", PackageRepositoryEvent::Deleted { repository_id: id });

        assert!(!printed.contains("upstream-secret-value"), "got: {printed}");
        assert!(printed.contains("[redacted]") && printed.contains("svc") && printed.contains("registry.example.com"), "got: {printed}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_name_cannot_use_the_reserved_prefix() {
        assert_eq!(parse_repository_name("artiferris-npm"), Err(DomainError::ReservedName("artiferris-npm".to_string())));
        assert_eq!(parse_repository_name("ARTIFERRIS-helm"), Err(DomainError::ReservedName("ARTIFERRIS-helm".to_string())));
        assert!(parse_repository_name("my-artiferris-npm").is_ok());
    }
    use uuid::Uuid;

    #[test]
    fn a_repository_name_of_two_dots_is_rejected() {
        assert!(parse_repository_name("..").is_err());
    }

    #[test]
    fn a_repository_name_of_one_dot_is_rejected() {
        assert!(parse_repository_name(".").is_err());
    }

    fn created_event(repository_id: Uuid, repo_type: RepositoryType) -> PackageRepositoryEvent {
        PackageRepositoryEvent::Created {
            repository_id,
            organization_id: Uuid::new_v4(),
            name: "my-repo".to_string(),
            format: RepositoryFormat::Npm,
            repo_type,
            remote_url: None,
            remote_username: None,
            remote_password: None,
        }
    }

    #[test]
    fn creating_sets_the_initial_state() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Hosted)]);
        assert_eq!(repo.name.as_deref(), Some("my-repo"));
        assert_eq!(repo.repo_type, Some(RepositoryType::Hosted));
        assert!(!repo.deleted);
    }

    #[test]
    fn renaming_updates_the_name() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Hosted)]);
        let event = repo.rename("renamed-repo".to_string()).unwrap();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Hosted), event]);
        assert_eq!(repo.name.as_deref(), Some("renamed-repo"));
    }

    #[test]
    fn creating_a_non_proxy_repository_with_a_remote_url_is_rejected() {
        for repo_type in [RepositoryType::Hosted, RepositoryType::Group] {
            let err = PackageRepository::create(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "my-repo".to_string(),
                RepositoryFormat::Npm,
                repo_type,
                Some("https://registry.npmjs.org".to_string()),
                None,
                None,
            )
            .unwrap_err();
            assert!(matches!(err, DomainError::InvalidForRepositoryType(_)), "got {err:?}");
        }
    }

    #[test]
    fn creating_a_proxy_repository_with_a_remote_url_succeeds() {
        let event = PackageRepository::create(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "my-repo".to_string(),
            RepositoryFormat::Npm,
            RepositoryType::Proxy,
            Some("https://registry.npmjs.org".to_string()),
            None,
            None,
        )
        .unwrap();
        assert!(matches!(event, PackageRepositoryEvent::Created { remote_url: Some(_), .. }));
    }

    #[test]
    fn a_remote_url_with_a_non_http_scheme_is_rejected() {
        let err = PackageRepository::create(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "my-repo".to_string(),
            RepositoryFormat::Npm,
            RepositoryType::Proxy,
            Some("file:///etc/passwd".to_string()),
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::InvalidRemoteUrl(_)), "got {err:?}");
    }

    #[test]
    fn a_remote_url_with_embedded_credentials_is_rejected() {
        for url in ["https://user:pw@registry.example.com/", "https://user@registry.example.com/"] {
            let err = PackageRepository::create(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "my-repo".to_string(),
                RepositoryFormat::Npm,
                RepositoryType::Proxy,
                Some(url.to_string()),
                None,
                None,
            )
            .unwrap_err();
            assert!(matches!(err, DomainError::InvalidRemoteUrl(_)), "{url}: {err:?}");
        }
    }

    #[test]
    fn changing_remote_url_to_one_with_embedded_credentials_is_rejected() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Proxy)]);
        let err = repo.change_remote_url("https://user:pw@registry.example.com/".to_string()).unwrap_err();
        assert!(matches!(err, DomainError::InvalidRemoteUrl(_)), "got {err:?}");
    }

    #[test]
    fn a_remote_url_that_fails_to_parse_at_all_is_rejected() {
        let err = PackageRepository::create(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "my-repo".to_string(),
            RepositoryFormat::Npm,
            RepositoryType::Proxy,
            Some("not a url".to_string()),
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::InvalidRemoteUrl(_)), "got {err:?}");
    }

    #[test]
    fn a_well_formed_https_remote_url_on_a_proxy_repository_is_still_accepted() {
        let event = PackageRepository::create(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "my-repo".to_string(),
            RepositoryFormat::Npm,
            RepositoryType::Proxy,
            Some("https://registry.npmjs.org".to_string()),
            None,
            None,
        )
        .unwrap();
        assert!(matches!(event, PackageRepositoryEvent::Created { remote_url: Some(_), .. }));
    }

    #[test]
    fn changing_remote_url_to_a_non_http_scheme_is_rejected() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Proxy)]);
        let err = repo.change_remote_url("ftp://internal-host/".to_string()).unwrap_err();
        assert!(matches!(err, DomainError::InvalidRemoteUrl(_)), "got {err:?}");
    }

    #[test]
    fn changing_remote_url_to_an_unparseable_url_is_rejected() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Proxy)]);
        let err = repo.change_remote_url("not a url".to_string()).unwrap_err();
        assert!(matches!(err, DomainError::InvalidRemoteUrl(_)), "got {err:?}");
    }

    #[test]
    fn changing_remote_url_requires_proxy_type() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Hosted)]);
        let err = repo.change_remote_url("https://registry.npmjs.org".to_string()).unwrap_err();
        assert!(matches!(err, DomainError::InvalidForRepositoryType(_)));
    }

    #[test]
    fn changing_remote_url_on_a_proxy_succeeds() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Proxy)]);
        let event = repo.change_remote_url("https://registry.npmjs.org".to_string()).unwrap();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Proxy), event]);
        assert_eq!(repo.remote_url.as_deref(), Some("https://registry.npmjs.org"));
    }

    #[test]
    fn making_a_proxy_repository_public_is_rejected() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Proxy)]);
        let err = repo.set_visibility(true).unwrap_err();
        assert!(matches!(err, DomainError::InvalidForRepositoryType(_)));
    }

    #[test]
    fn making_a_group_repository_public_is_rejected() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Group)]);
        let err = repo.set_visibility(true).unwrap_err();
        assert!(matches!(err, DomainError::InvalidForRepositoryType(_)));
    }

    #[test]
    fn making_a_proxy_or_group_repository_private_is_always_allowed() {
        let repository_id = Uuid::new_v4();
        for repo_type in [RepositoryType::Proxy, RepositoryType::Group] {
            let repo = PackageRepository::from_events(&[created_event(repository_id, repo_type)]);
            let event = repo.set_visibility(false).unwrap();
            assert!(matches!(event, PackageRepositoryEvent::VisibilityChanged { is_public: false, .. }));
        }
    }

    #[test]
    fn adding_a_group_member_requires_group_type() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Hosted)]);
        let err = repo.add_group_member(Uuid::new_v4(), 0).unwrap_err();
        assert!(matches!(err, DomainError::InvalidForRepositoryType(_)));
    }

    #[test]
    fn adding_and_removing_a_group_member() {
        let repository_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let created = created_event(repository_id, RepositoryType::Group);
        let repo = PackageRepository::from_events(std::slice::from_ref(&created));
        let added = repo.add_group_member(member_id, 0).unwrap();
        let repo = PackageRepository::from_events(&[created.clone(), added.clone()]);
        assert_eq!(repo.group_members, vec![(member_id, 0)]);

        let removed = repo.remove_group_member(member_id).unwrap();
        let repo = PackageRepository::from_events(&[created, added, removed]);
        assert!(repo.group_members.is_empty());
    }

    #[test]
    fn deleting_prevents_further_mutation() {
        let repository_id = Uuid::new_v4();
        let created = created_event(repository_id, RepositoryType::Hosted);
        let repo = PackageRepository::from_events(std::slice::from_ref(&created));
        let deleted = repo.delete().unwrap();
        let repo = PackageRepository::from_events(&[created, deleted]);
        assert!(repo.deleted);
        let err = repo.rename("x".to_string()).unwrap_err();
        assert!(matches!(err, DomainError::AlreadyDeleted));
    }

    #[test]
    fn setting_and_clearing_a_retention_policy() {
        let repository_id = Uuid::new_v4();
        let created = created_event(repository_id, RepositoryType::Hosted);
        let repo = PackageRepository::from_events(std::slice::from_ref(&created));
        assert_eq!(repo.retention_keep_last_n, None);

        let set = repo.set_retention_policy(Some(5)).unwrap();
        let repo = PackageRepository::from_events(&[created.clone(), set]);
        assert_eq!(repo.retention_keep_last_n, Some(5));

        let cleared = repo.set_retention_policy(None).unwrap();
        let repo = PackageRepository::from_events(&[created, cleared]);
        assert_eq!(repo.retention_keep_last_n, None);
    }

    #[test]
    fn setting_a_retention_policy_below_one_is_rejected() {
        let repository_id = Uuid::new_v4();
        let repo = PackageRepository::from_events(&[created_event(repository_id, RepositoryType::Hosted)]);
        let err = repo.set_retention_policy(Some(0)).unwrap_err();
        assert!(matches!(err, DomainError::Validation(_)));
    }

    /// `set_retention_policy` rejects `Some(0)` at construction time, but `apply` replays trusted
    /// history unconditionally — a `RetentionPolicySet { keep_last_n_versions: Some(0), .. }` event
    /// written before that guard existed (B-37) must not replay into a live "keep nothing" policy.
    /// Sanitized to `Some(1)`, the minimum meaningful value, rather than `None` (which would mean
    /// "no policy, keep everything" — a bigger behavior change than the pre-guard writer intended).
    #[test]
    fn replaying_a_pre_existing_zero_retention_event_does_not_produce_a_destructive_policy() {
        let repository_id = Uuid::new_v4();
        let created = created_event(repository_id, RepositoryType::Hosted);
        let bad_event = PackageRepositoryEvent::RetentionPolicySet { repository_id, keep_last_n_versions: Some(0) };
        let repo = PackageRepository::from_events(&[created, bad_event]);
        assert_eq!(repo.retention_keep_last_n, Some(1));
    }

    #[test]
    fn a_new_repository_defaults_to_private() {
        let repository_id = Uuid::new_v4();
        let created = created_event(repository_id, RepositoryType::Hosted);
        let repo = PackageRepository::from_events(std::slice::from_ref(&created));
        assert!(!repo.is_public);
    }

    #[test]
    fn setting_and_clearing_visibility() {
        let repository_id = Uuid::new_v4();
        let created = created_event(repository_id, RepositoryType::Hosted);
        let repo = PackageRepository::from_events(std::slice::from_ref(&created));
        assert!(!repo.is_public);

        let set = repo.set_visibility(true).unwrap();
        let repo = PackageRepository::from_events(&[created.clone(), set]);
        assert!(repo.is_public);

        let cleared = repo.set_visibility(false).unwrap();
        let repo = PackageRepository::from_events(&[created, cleared]);
        assert!(!repo.is_public);
    }

    #[test]
    fn setting_visibility_after_deletion_is_rejected() {
        let repository_id = Uuid::new_v4();
        let created = created_event(repository_id, RepositoryType::Hosted);
        let repo = PackageRepository::from_events(std::slice::from_ref(&created));
        let deleted = repo.delete().unwrap();
        let repo = PackageRepository::from_events(&[created, deleted]);

        let err = repo.set_visibility(true).unwrap_err();
        assert!(matches!(err, DomainError::AlreadyDeleted));
    }

    #[test]
    fn credentials_require_an_https_remote() {
        let create = |url: &str, username: Option<&str>, password: Option<&str>| {
            PackageRepository::create(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "proxy-repo".to_string(),
                RepositoryFormat::Npm,
                RepositoryType::Proxy,
                Some(url.to_string()),
                username.map(str::to_string),
                password.map(str::to_string),
            )
        };

        assert!(matches!(create("http://registry.example.com", Some("user"), Some("pass")), Err(DomainError::InvalidRemoteUrl(_))));
        assert!(matches!(create("http://registry.example.com", None, Some("token")), Err(DomainError::InvalidRemoteUrl(_))));
        assert!(create("http://registry.example.com", None, None).is_ok(), "an anonymous remote may be plain http");
        assert!(create("https://registry.example.com", Some("user"), Some("pass")).is_ok());
    }

    #[test]
    fn changing_the_remote_to_plain_http_is_refused_while_credentials_are_stored() {
        let mut proxy = PackageRepository::default();
        proxy.apply(
            &PackageRepository::create(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "proxy-repo".to_string(),
                RepositoryFormat::Npm,
                RepositoryType::Proxy,
                Some("https://registry.example.com".to_string()),
                Some("user".to_string()),
                Some("pass".to_string()),
            )
            .unwrap(),
        );

        assert!(matches!(proxy.change_remote_url("http://registry.example.com".to_string()), Err(DomainError::InvalidRemoteUrl(_))));
        assert!(proxy.change_remote_url("https://other.example.com".to_string()).is_ok());
    }
}
