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
    /// Same answer for an unknown, expired, used or malformed link, so nobody can probe them.
    #[error("invalid or expired password reset link")]
    PasswordResetLinkInvalid,
    /// Losing the mail would lock an administrator out of their own account, and a sole one has nobody to issue another.
    #[error("use your account settings to change your own password")]
    OwnPasswordReset,
    #[error("this account has not been activated yet: resend the invitation instead")]
    AccountNotActivated,
    #[error("this organization signs in through its identity provider, which manages its passwords")]
    PasswordManagedByIdentityProvider,
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
    /// The id does not exist, or belongs to another user. The caller cannot tell which.
    #[error("api token not found")]
    ApiTokenNotFound,
    #[error("too many active api tokens, revoke one first")]
    ApiTokenLimitReached,
    #[error("api token label is too long")]
    ApiTokenLabelTooLong,
}

impl ApplicationError {
    /// A stable, language-independent name for this error, sent to clients next to the message so they can react to it (and translate it) without matching the wording. Changing one is a breaking change of the API.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Domain(inner) => inner.code(),
            Self::EventStore(_) => "event_store_failure",
            Self::Storage(_) => "storage_failure",
            Self::UsernameTaken => "username_taken",
            Self::InvalidCredentials => "invalid_credentials",
            // Same code as InvalidCredentials, like their message: a caller must not learn that a token exists but is revoked.
            Self::InactiveApiToken => "invalid_credentials",
            Self::RepositoryNameTaken => "repository_name_taken",
            Self::PersonalOrganizationAlreadyExists => "personal_organization_already_exists",
            Self::NoPersonalOrganization => "no_personal_organization",
            Self::LastSuperAdmin => "last_super_admin",
            Self::PackageVersionExists => "package_version_exists",
            Self::NpmPackageNotFound => "npm_package_not_found",
            Self::NpmVersionNotFound => "npm_version_not_found",
            Self::InvalidNpmPayload(..) => "invalid_npm_payload",
            Self::DockerBlobAlreadyExists => "docker_blob_already_exists",
            Self::DockerBlobNotFound => "docker_blob_not_found",
            Self::DockerManifestNotFound => "docker_manifest_not_found",
            Self::DockerDigestMismatch { .. } => "docker_digest_mismatch",
            Self::InvalidDockerPayload(..) => "invalid_docker_payload",
            Self::DockerUploadSessionNotFound => "docker_upload_session_not_found",
            Self::DockerChunkOffsetMismatch { .. } => "docker_chunk_offset_mismatch",
            Self::DockerUploadInProgress => "docker_upload_in_progress",
            Self::DockerTooManyUploads => "docker_too_many_uploads",
            Self::DockerTooManyTags => "docker_too_many_tags",
            Self::DockerUploadTooLarge => "docker_upload_too_large",
            Self::UpstreamIntegrityMismatch => "upstream_integrity_mismatch",
            Self::InvalidSystemSettings(..) => "invalid_system_settings",
            Self::StorageQuotaExceeded => "storage_quota_exceeded",
            Self::DependencyScanBusy => "dependency_scan_busy",
            Self::DependencyScanRateLimited => "dependency_scan_rate_limited",
            Self::InvalidSmtpSettings(..) => "invalid_smtp_settings",
            Self::InvalidCatalogQuery(..) => "invalid_catalog_query",
            Self::InvalidEmail(..) => "invalid_email",
            Self::InvitationNotFound => "invitation_not_found",
            Self::InvitationExpired => "invitation_expired",
            Self::MfaAlreadyEnabled => "mfa_already_enabled",
            Self::MfaNotEnrolled => "mfa_not_enrolled",
            Self::PasswordResetLinkInvalid => "password_reset_link_invalid",
            Self::OwnPasswordReset => "own_password_reset",
            Self::AccountNotActivated => "account_not_activated",
            Self::PasswordManagedByIdentityProvider => "password_managed_by_identity_provider",
            Self::MfaEnrollmentExpired => "mfa_enrollment_expired",
            Self::InvalidMfaCode => "invalid_mfa_code",
            Self::PasskeysUnavailable => "passkeys_unavailable",
            Self::InstanceNotEmpty => "instance_not_empty",
            Self::MultiTenantInstance => "multi_tenant_instance",
            Self::PersonalRepositoriesNotExportable(..) => "personal_repositories_not_exportable",
            Self::ImportTooLarge(..) => "import_too_large",
            Self::InvalidBrandingAsset(..) => "invalid_branding_asset",
            Self::OrganizationSlugTaken => "organization_slug_taken",
            Self::ReservedOrganizationSlug => "reserved_organization_slug",
            Self::ActingAdminNotFound => "acting_admin_not_found",
            Self::ApiTokenNotFound => "api_token_not_found",
            Self::ApiTokenLimitReached => "api_token_limit_reached",
            Self::ApiTokenLabelTooLong => "api_token_label_too_long",
        }
    }
}
