use artiferris_domain::error::{DomainError, EventStoreError};
use artiferris_domain::storage::StorageError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    EventStore(#[from] EventStoreError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("username already taken")]
    UsernameTaken,
    #[error("invalid credentials")]
    InvalidCredentials,
    /// A real API token that is revoked, expired or predates a password change. Kept apart from `InvalidCredentials` so its holder isn't throttled like a guesser.
    #[error("invalid credentials")]
    InactiveApiToken,
    #[error("repository name already taken")]
    RepositoryNameTaken,
    #[error("you already have a personal repository namespace")]
    PersonalOrganizationAlreadyExists,
    #[error("you don't have a personal repository namespace yet")]
    NoPersonalOrganization,
    #[error("cannot delete the last super-administrator")]
    LastSuperAdmin,
    #[error("this package version already exists and cannot be republished")]
    PackageVersionExists,
    #[error("package not found")]
    NpmPackageNotFound,
    #[error("version not found")]
    NpmVersionNotFound,
    #[error("invalid tarball or manifest: {0}")]
    InvalidNpmPayload(String),
    #[error("blob with this digest already exists")]
    DockerBlobAlreadyExists,
    #[error("blob not found")]
    DockerBlobNotFound,
    #[error("manifest not found")]
    DockerManifestNotFound,
    #[error("digest mismatch: expected {expected}, computed {computed}")]
    DockerDigestMismatch { expected: String, computed: String },
    #[error("invalid manifest or upload: {0}")]
    InvalidDockerPayload(String),
    #[error("upload session not found or expired")]
    DockerUploadSessionNotFound,
    #[error("chunk offset mismatch: expected to start at {expected}, got {got}")]
    DockerChunkOffsetMismatch { expected: i64, got: i64 },
    #[error("this upload already has a chunk being written, or is being completed")]
    DockerUploadInProgress,
    #[error("too many uploads are open for this repository, finish or cancel some first")]
    DockerTooManyUploads,
    #[error("this repository holds as many tags as it may, delete some first")]
    DockerTooManyTags,
    #[error("upload is larger than the registry accepts")]
    DockerUploadTooLarge,
    #[error("the tarball fetched from the upstream registry does not match the checksum it advertises")]
    UpstreamIntegrityMismatch,
    #[error("invalid system settings: {0}")]
    InvalidSystemSettings(String),
    #[error("this write would exceed the repository's storage quota")]
    StorageQuotaExceeded,
    #[error("dependency scans are busy right now, try again in a moment")]
    DependencyScanBusy,
    #[error("a dependency scan was started for this repository a moment ago, wait before starting another")]
    DependencyScanRateLimited,
    #[error("invalid SMTP settings: {0}")]
    InvalidSmtpSettings(String),
    #[error("invalid catalog query: {0}")]
    InvalidCatalogQuery(String),
    #[error("invalid email address: {0}")]
    InvalidEmail(String),
    #[error("invitation not found or already used")]
    InvitationNotFound,
    #[error("this invitation has expired")]
    InvitationExpired,
    #[error("two-factor authentication is already enabled")]
    MfaAlreadyEnabled,
    #[error("two-factor authentication is not enabled")]
    MfaNotEnrolled,
    #[error("this authenticator setup has expired, start it again")]
    MfaEnrollmentExpired,
    #[error("invalid or already-used authentication code")]
    InvalidMfaCode,
    #[error("passkeys are not available: the server's PUBLIC_URL is not configured with a valid domain")]
    PasskeysUnavailable,
    #[error("this instance is not empty — import only works on a freshly-provisioned instance with no repositories and no users other than the one running the import")]
    InstanceNotEmpty,
    #[error("configuration export and import only support single-tenant instances: this instance has organizations besides the public one and users' personal namespaces, and the file has no way to keep them apart")]
    MultiTenantInstance,
    #[error("configuration export cannot represent users' personal repositories ({0} found): the file has no owner for a repository, so they would be restored into the public organization. Delete or move them, then export again")]
    PersonalRepositoriesNotExportable(usize),
    #[error("{0}")]
    ImportTooLarge(String),
    #[error("invalid branding asset: {0}")]
    InvalidBrandingAsset(String),
    #[error("organization slug already taken")]
    OrganizationSlugTaken,
    #[error("this organization slug is reserved")]
    ReservedOrganizationSlug,
    #[error("the acting admin's own user record could not be found — its token was valid enough to identify a user, but that user no longer exists")]
    ActingAdminNotFound,
    /// Either the id doesn't exist at all, or it belongs to a different user —
    /// deliberately indistinguishable to the caller, same as a cross-organization
    /// lookup elsewhere in this codebase.
    #[error("api token not found")]
    ApiTokenNotFound,
    #[error("too many active api tokens, revoke one first")]
    ApiTokenLimitReached,
    #[error("api token label is too long")]
    ApiTokenLabelTooLong,
}
