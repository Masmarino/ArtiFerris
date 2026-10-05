use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid username: {0}")]
    InvalidUsername(String),
    #[error("email already in use")]
    EmailTaken,
    #[error("username already in use")]
    UsernameTaken,
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),
    #[error("password must be at least 8 characters")]
    PasswordTooShort,
    #[error("invalid repository name: {0}")]
    InvalidRepositoryName(String),
    #[error("invalid organization slug: {0}")]
    InvalidOrganizationSlug(String),
    #[error("names starting with \"artiferris-\" are reserved: {0}")]
    ReservedName(String),
    #[error("invalid remote url: {0}")]
    InvalidRemoteUrl(String),
    #[error("operation not valid for repository type {0:?}")]
    InvalidForRepositoryType(String),
    #[error("a group repository cannot contain itself")]
    SelfGroupMembership,
    #[error("member repository does not exist: {0}")]
    UnknownGroupMember(Uuid),
    #[error("member repository {0} has a different format than the group")]
    GroupMemberFormatMismatch(Uuid),
    #[error("member repository {0} belongs to a different organization than the group")]
    GroupMemberOrganizationMismatch(Uuid),
    #[error("user {0} belongs to a different organization than the repository")]
    GranteeOrganizationMismatch(Uuid),
    #[error("permission already absent, nothing to revoke")]
    NothingToRevoke,
    #[error("package repository has already been deleted")]
    AlreadyDeleted,
    #[error("infrastructure failure: {0}")]
    Infrastructure(String),
    /// The organization has no mail server: nothing was attempted. Its own case so an administrator can be told what to fix.
    #[error("SMTP is not configured")]
    EmailNotConfigured,
    /// A stored secret that this server's keys cannot open (rotated or wrong `SECRETS_ENCRYPTION_KEY`, corrupted value).
    #[error("stored secret cannot be read: {0}")]
    SecretUnreadable(String),
    /// Nothing is broken: the work was refused or cut short to protect the service, and asking again shortly may work.
    #[error("temporarily busy: {0}")]
    Busy(String),
    #[error("chunk offset mismatch: expected {expected}, got {got}")]
    ChunkOffsetMismatch { expected: i64, got: i64 },
    #[error("validation error: {0}")]
    Validation(String),
    #[error("upload session not found")]
    UploadSessionNotFound,
    /// The session was sealed, or changed while a chunk was being written.
    #[error("upload session cannot take a chunk right now")]
    UploadInProgress,
    #[error("upload exceeds the allowed size")]
    UploadTooLarge,
    #[error("digest mismatch: expected {expected}, computed {computed}")]
    DigestMismatch { expected: String, computed: String },
    /// The client stopped sending, or sent too slowly.
    #[error("the request body took too long to arrive")]
    RequestTimeout,
    #[error("too many uploads are open for this repository")]
    TooManyUploads,
    #[error("this repository holds as many tags as it may")]
    TooManyTags,
    /// A manifest-referenced blob is not reachable from the pushing repository, re-verified in the manifest insert
    /// transaction.
    #[error("blob not reachable: {0}")]
    DockerBlobNotReachable(String),
    /// The repository's storage quota would be exceeded, re-verified in the manifest insert transaction so two
    /// concurrent pushes cannot both pass.
    #[error("storage quota exceeded")]
    StorageQuotaExceeded,
    /// `insert_version`'s unique `(npm_package_id, version)` constraint was violated: the losing side of two concurrent
    /// publishes of the same new version. The storage key is unique per attempt, so this is the only place the race
    /// resolves; the application layer maps it to `PackageVersionExists`.
    #[error("this package version already exists")]
    NpmVersionAlreadyExists,
    #[error("this package version was not found")]
    NpmVersionNotFound,
    /// The package holds as many versions, or as many manifest bytes, as one package may.
    #[error("{0}")]
    NpmPackageLimit(String),
    /// The database reported an error on COMMIT, which may or may not have been applied.
    #[error("commit failed: {0}")]
    CommitFailed(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EventStoreError {
    #[error("concurrency conflict: expected version {expected}, found {actual}")]
    ConcurrencyConflict { expected: u64, actual: u64 },
    #[error("storage failure: {0}")]
    Storage(String),
}

impl DomainError {
    /// A stable, language-independent name for this error, sent to clients next to the message so they can react to it (and translate it) without matching the wording. Changing one is a breaking change of the API.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUsername(..) => "invalid_username",
            Self::EmailTaken => "email_taken",
            Self::UsernameTaken => "username_taken",
            Self::UnsupportedLanguage(..) => "unsupported_language",
            Self::PasswordTooShort => "password_too_short",
            Self::InvalidRepositoryName(..) => "invalid_repository_name",
            Self::InvalidOrganizationSlug(..) => "invalid_organization_slug",
            Self::ReservedName(..) => "reserved_name",
            Self::InvalidRemoteUrl(..) => "invalid_remote_url",
            Self::InvalidForRepositoryType(..) => "invalid_for_repository_type",
            Self::SelfGroupMembership => "self_group_membership",
            Self::UnknownGroupMember(..) => "unknown_group_member",
            Self::GroupMemberFormatMismatch(..) => "group_member_format_mismatch",
            Self::GroupMemberOrganizationMismatch(..) => "group_member_organization_mismatch",
            Self::GranteeOrganizationMismatch(..) => "grantee_organization_mismatch",
            Self::NothingToRevoke => "nothing_to_revoke",
            Self::AlreadyDeleted => "already_deleted",
            Self::Infrastructure(..) => "infrastructure_failure",
            Self::EmailNotConfigured => "email_not_configured",
            Self::SecretUnreadable(..) => "secret_unreadable",
            Self::Busy(..) => "busy",
            Self::ChunkOffsetMismatch { .. } => "chunk_offset_mismatch",
            Self::Validation(..) => "validation",
            Self::UploadSessionNotFound => "upload_session_not_found",
            Self::UploadInProgress => "upload_in_progress",
            Self::UploadTooLarge => "upload_too_large",
            Self::DigestMismatch { .. } => "digest_mismatch",
            Self::RequestTimeout => "request_timeout",
            Self::TooManyUploads => "too_many_uploads",
            Self::TooManyTags => "too_many_tags",
            Self::DockerBlobNotReachable(..) => "docker_blob_not_reachable",
            Self::StorageQuotaExceeded => "storage_quota_exceeded",
            Self::NpmVersionAlreadyExists => "npm_version_already_exists",
            Self::NpmVersionNotFound => "npm_version_not_found",
            Self::NpmPackageLimit(..) => "npm_package_limit",
            Self::CommitFailed(..) => "commit_failed",
        }
    }
}
