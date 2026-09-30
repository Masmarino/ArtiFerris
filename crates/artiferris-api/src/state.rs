use std::sync::Arc;

use artiferris_application::use_cases::admin::{
    AdminListApiTokensUseCase, AdminRevokeApiTokenUseCase, ExportConfigurationUseCase, GetAdminStatsUseCase, GetHealthStatusUseCase, GetMetricsHistoryUseCase, GetSystemSettingsUseCase,
    GetUsageMetricsUseCase, ImportConfigurationUseCase, QueryAuditLogUseCase, RecordAdminEventUseCase, RecordMetricsSnapshotUseCase, RecordSecurityEventUseCase, UpdateSystemSettingsUseCase,
};
use artiferris_application::use_cases::api_token::{CreateApiTokenUseCase, ListApiTokensUseCase, RevokeApiTokenUseCase};
use artiferris_application::use_cases::branding::{BrandingDefaults, ClearBrandingFaviconUseCase, ClearBrandingLogoUseCase, GetBrandingUseCase, SetBrandingFaviconUseCase, SetBrandingLogoUseCase};
use artiferris_application::use_cases::docker_manifest_delete::{DeleteDockerImageUseCase, DeleteManifestUseCase};
use artiferris_application::use_cases::docker_scan::{GetDockerImageScanResultUseCase, ScanDockerImageUseCase};
use artiferris_application::use_cases::list_repository_packages::ListRepositoryPackagesUseCase;
use artiferris_application::use_cases::npm_audit::AuditNpmPackageUseCase;
use artiferris_application::use_cases::npm_dependency_scan::{GetDependencyAuditResultUseCase, ScanDependencyTreeUseCase};
use artiferris_application::use_cases::npm_unpublish::UnpublishNpmPackageUseCase;
use artiferris_application::use_cases::organization::CreateOrganizationUseCase;
use artiferris_application::use_cases::package_details::{GetDockerImageDetailsUseCase, GetNpmPackageDetailsUseCase};
use artiferris_application::use_cases::package_repository::{
    AddGroupMemberUseCase, CreatePackageRepositoryUseCase, DeletePackageRepositoryUseCase, RemoveGroupMemberUseCase,
    RenamePackageRepositoryUseCase, SetRepositoryQuotaUseCase, SetRepositoryVisibilityUseCase, SetRetentionPolicyUseCase,
};
use artiferris_application::use_cases::permission::{GrantPermissionUseCase, RevokePermissionUseCase};
use artiferris_application::use_cases::personal_repository::{CreateUserProjectUseCase, FindMyPersonalOrganizationUseCase, ReservePersonalOrganizationUseCase};
use artiferris_application::cached_public_catalog::CachedPublicCatalog;
use artiferris_application::audit_retention::PruneAuditEventsUseCase;
use artiferris_application::download_counter::{DownloadCounterBuffer, FlushDownloadCountersUseCase, PruneDownloadStatsUseCase};
use artiferris_application::use_cases::public_catalog::{GetOwnerSummaryUseCase, ListCatalogsUseCase, SearchPublicCatalogUseCase, SuggestPublicCatalogUseCase};
use artiferris_application::use_cases::resolve_organization_repository::ResolveOrganizationRepositoryUseCase;
use artiferris_application::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase;
use artiferris_application::use_cases::invitation::{ActivateAccountUseCase, InviteUserUseCase, ResendInvitationUseCase};
use artiferris_application::use_cases::mfa::{
    ConfirmTotpUseCase, DisableTotpUseCase, EnrollTotpUseCase, GetMfaStatusUseCase, RegenerateBackupCodesUseCase, VerifyBackupCodeUseCase, VerifyTotpUseCase,
};
use artiferris_application::use_cases::smtp::{GetSmtpSettingsUseCase, SendTestEmailUseCase, UpdateSmtpSettingsUseCase};
use artiferris_application::use_cases::webauthn::{
    build_webauthn_client, DeletePasskeyUseCase, FinishPasskeyAuthenticationUseCase, FinishPasskeyRegistrationUseCase, ListPasskeysUseCase, PasskeyCeremonyStore,
    StartPasskeyAuthenticationUseCase, StartPasskeyRegistrationUseCase,
};
use artiferris_application::use_cases::user::{AuthenticateUserUseCase, ChangePasswordUseCase, CreateUserUseCase, DeleteUserUseCase, ConfirmPasswordUseCase, RevokeUserSessionsUseCase, SetOrganizationAdminUseCase, SetSuperAdminUseCase};
use artiferris_application::use_cases::registration::RegisterPublicUserUseCase;
use artiferris_application::use_cases::sso::ProvisionSsoUserUseCase;
use artiferris_domain::api_token::ApiTokenRepositoryPort;
use artiferris_domain::audit::EventPublisherPort;
use artiferris_domain::npm_audit::NpmAuditPort;
use artiferris_domain::npm_package::NpmPackageRepositoryPort;
use artiferris_domain::organization::OrganizationRepositoryPort;
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, RepositoryQuotaLockPort};
use artiferris_domain::permission::PermissionQueryPort;
use artiferris_domain::storage::StorageBackendPort;
use artiferris_domain::user::{TokenIssuerPort, UserRepositoryPort, UserSecurityPort};
use artiferris_domain::sso::{IdentityProviderRepositoryPort, LdapAuthPort, OidcAuthPort};
use artiferris_domain::branding::BrandingAsset;
use artiferris_infrastructure::argon2_hasher::Argon2PasswordHasher;
use artiferris_infrastructure::branding_defaults;
use artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend;
use artiferris_infrastructure::jwt_mfa_pending_token_issuer::JwtMfaPendingTokenIssuer;
use artiferris_infrastructure::jwt_token_issuer::JwtTokenIssuer;
use artiferris_infrastructure::postgres::api_token_repository::PostgresApiTokenRepository;
use artiferris_infrastructure::postgres::backup_code_repository::PostgresBackupCodeRepository;
use artiferris_infrastructure::postgres::configuration_import::PostgresConfigurationImport;
use artiferris_infrastructure::postgres::event_publisher::PostgresEventPublisher;
use artiferris_infrastructure::postgres::health::{PostgresHealthCheck, PostgresReadiness};
use artiferris_infrastructure::postgres::docker_image_scan_repository::PostgresDockerImageScanRepository;
use artiferris_infrastructure::postgres::metrics_snapshot_repository::PostgresMetricsSnapshotRepository;
use artiferris_infrastructure::postgres::npm_dependency_audit_repository::PostgresDependencyAuditRepository;
use artiferris_infrastructure::postgres::npm_package_repository::PostgresNpmPackageRepository;
use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
use artiferris_infrastructure::postgres::permission_store::PostgresPermissionStore;
use artiferris_infrastructure::postgres::download_stats_repository::PostgresDownloadStats;
use artiferris_infrastructure::postgres::public_catalog_repository::PostgresPublicCatalog;
use artiferris_infrastructure::postgres::smtp_settings_repository::PostgresSmtpSettingsRepository;
use artiferris_infrastructure::postgres::system_settings_repository::PostgresSystemSettingsRepository;
use artiferris_infrastructure::postgres::totp_credential_repository::PostgresTotpCredentialRepository;
use artiferris_infrastructure::postgres::user_invitation_repository::PostgresUserInvitationRepository;
use artiferris_infrastructure::postgres::user_repository::PostgresUserRepository;
use artiferris_infrastructure::postgres::webauthn_credential_repository::PostgresWebauthnCredentialRepository;
use artiferris_infrastructure::ldap3_auth_adapter::Ldap3AuthAdapter;
use artiferris_infrastructure::openidconnect_auth_adapter::OpenidConnectAuthAdapter;
use artiferris_infrastructure::postgres::identity_provider_repository::PostgresIdentityProviderRepository;
use artiferris_infrastructure::smtp_email_sender::SmtpEmailSender;
use sqlx::PgPool;
use std::time::Instant;

use artiferris_application::login_throttle::LoginThrottle;
use artiferris_application::single_use_tokens::SingleUseTokens;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub users: Arc<dyn UserRepositoryPort>,
    pub user_preferences: Arc<dyn artiferris_domain::user_preferences::UserPreferencesPort>,
    pub permissions: Arc<dyn PermissionQueryPort>,
    pub repositories: Arc<dyn PackageRepositoryQueryPort>,
    /// Backed by the same `PostgresPackageRepositoryStore` instance as `repositories` — a
    /// separate trait-object view for npm's quota-check TOCTOU fix (see `npm_publish.rs`).
    pub repository_quota_lock: Arc<dyn RepositoryQuotaLockPort>,
    pub organizations: Arc<dyn OrganizationRepositoryPort>,
    pub identity_providers: Arc<dyn IdentityProviderRepositoryPort>,
    pub ldap_auth: Arc<dyn LdapAuthPort>,
    pub oidc_auth: Arc<dyn OidcAuthPort>,
    pub provision_sso_user: Arc<ProvisionSsoUserUseCase>,
    /// Base domain `ResolvedOrganization` strips off the `Host` header to find the subdomain label.
    pub artiferris_base_domain: String,
    /// The SPA's public URL — used to build the final browser redirect after an OIDC callback.
    pub public_url: String,
    pub api_tokens: Arc<dyn ApiTokenRepositoryPort>,
    /// Exposed so `main.rs` can reuse the same adapters for `artiferris_npm`/`artiferris_docker` state.
    pub storage: Arc<dyn StorageBackendPort>,
    pub events: Arc<dyn EventPublisherPort>,
    pub npm_packages: Arc<dyn NpmPackageRepositoryPort>,
    pub npm_audit: Arc<dyn NpmAuditPort>,
    pub docker_blobs: Arc<dyn artiferris_domain::docker_registry::DockerBlobStorePort>,
    pub docker_manifests: Arc<dyn artiferris_domain::docker_registry::DockerManifestRepositoryPort>,
    pub docker_uploads: Arc<dyn artiferris_domain::docker_registry::DockerUploadSessionPort>,
    pub create_user: Arc<CreateUserUseCase>,
    pub register_public_user: Arc<RegisterPublicUserUseCase>,
    pub authenticate_user: Arc<AuthenticateUserUseCase>,
    pub delete_user: Arc<DeleteUserUseCase>,
    pub set_super_admin: Arc<SetSuperAdminUseCase>,
    pub set_organization_admin: Arc<SetOrganizationAdminUseCase>,
    pub change_password: Arc<ChangePasswordUseCase>,
    pub create_organization: Arc<CreateOrganizationUseCase>,
    pub create_repository: Arc<CreatePackageRepositoryUseCase>,
    pub reserve_personal_organization: Arc<ReservePersonalOrganizationUseCase>,
    pub find_my_personal_organization: Arc<FindMyPersonalOrganizationUseCase>,
    pub resolve_personal_repository: Arc<ResolvePersonalRepositoryUseCase>,
    pub resolve_organization_repository: Arc<ResolveOrganizationRepositoryUseCase>,
    pub list_readable_repositories: Arc<artiferris_application::use_cases::list_readable_repositories::ListReadableRepositoriesUseCase>,
    pub search_public_catalog: Arc<SearchPublicCatalogUseCase>,
    pub search_readable_catalog: Arc<artiferris_application::use_cases::public_catalog::SearchReadableCatalogUseCase>,
    pub list_catalogs: Arc<ListCatalogsUseCase>,
    pub get_owner_summary: Arc<GetOwnerSummaryUseCase>,
    pub suggest_public_catalog: Arc<SuggestPublicCatalogUseCase>,
    /// Read-only, for the sitemap and the `<head>` of public pages.
    pub public_catalog: Arc<dyn artiferris_domain::public_catalog::PublicCatalogPort>,
    pub seo_pages: Arc<artiferris_application::use_cases::seo::SeoPageUseCase>,
    /// Where the registries note each download; `flush_downloads` moves them to the database in batches.
    pub download_counter_buffer: Arc<DownloadCounterBuffer>,
    pub flush_downloads: Arc<FlushDownloadCountersUseCase>,
    pub prune_download_stats: Arc<PruneDownloadStatsUseCase>,
    pub prune_audit_events: Arc<PruneAuditEventsUseCase>,
    pub download_stats: Arc<dyn artiferris_domain::download_stats::DownloadStatsPort>,
    pub create_user_project: Arc<CreateUserProjectUseCase>,
    pub rename_repository: Arc<RenamePackageRepositoryUseCase>,
    pub delete_repository: Arc<DeletePackageRepositoryUseCase>,
    pub add_group_member: Arc<AddGroupMemberUseCase>,
    pub remove_group_member: Arc<RemoveGroupMemberUseCase>,
    pub set_repository_quota: Arc<SetRepositoryQuotaUseCase>,
    pub set_retention_policy: Arc<SetRetentionPolicyUseCase>,
    pub set_repository_visibility: Arc<SetRepositoryVisibilityUseCase>,
    pub sweep_retention: Arc<artiferris_application::use_cases::retention::SweepRetentionUseCase>,
    pub sweep_expired_uploads: Arc<artiferris_application::use_cases::docker_upload_sweep::SweepExpiredDockerUploadsUseCase>,
    pub sweep_repository_deletions: Arc<artiferris_application::use_cases::repository_deletion_sweep::RepositoryDeletionSweepUseCase>,
    pub grant_permission: Arc<GrantPermissionUseCase>,
    pub revoke_permission: Arc<RevokePermissionUseCase>,
    pub query_audit_log: Arc<QueryAuditLogUseCase>,
    pub record_security_event: Arc<RecordSecurityEventUseCase>,
    pub record_admin_event: Arc<RecordAdminEventUseCase>,
    pub get_usage_metrics: Arc<GetUsageMetricsUseCase>,
    pub get_metrics_history: Arc<GetMetricsHistoryUseCase>,
    pub record_metrics_snapshot: Arc<RecordMetricsSnapshotUseCase>,
    pub get_health_status: Arc<GetHealthStatusUseCase>,
    pub readiness: Arc<PostgresReadiness>,
    pub get_admin_stats: Arc<GetAdminStatsUseCase>,
    pub export_configuration: Arc<ExportConfigurationUseCase>,
    pub import_configuration: Arc<ImportConfigurationUseCase>,
    pub create_api_token: Arc<CreateApiTokenUseCase>,
    pub list_api_tokens: Arc<ListApiTokensUseCase>,
    pub revoke_api_token: Arc<RevokeApiTokenUseCase>,
    pub admin_list_api_tokens: Arc<AdminListApiTokensUseCase>,
    pub admin_revoke_api_token: Arc<AdminRevokeApiTokenUseCase>,
    pub list_repository_packages: Arc<ListRepositoryPackagesUseCase>,
    pub get_npm_package_details: Arc<GetNpmPackageDetailsUseCase>,
    pub get_docker_image_details: Arc<GetDockerImageDetailsUseCase>,
    pub unpublish_npm_package: Arc<UnpublishNpmPackageUseCase>,
    pub audit_npm_package: Arc<AuditNpmPackageUseCase>,
    pub scan_dependency_tree: Arc<ScanDependencyTreeUseCase>,
    pub get_dependency_audit: Arc<GetDependencyAuditResultUseCase>,
    pub delete_docker_manifest: Arc<DeleteManifestUseCase>,
    pub delete_docker_image: Arc<DeleteDockerImageUseCase>,
    pub scan_docker_image: Arc<ScanDockerImageUseCase>,
    pub get_docker_image_scan: Arc<GetDockerImageScanResultUseCase>,
    pub token_issuer: Arc<JwtTokenIssuer>,
    pub login_throttle: LoginThrottle,
    /// MFA-pending tokens that already completed a login or mandatory setup.
    pub used_mfa_tokens: SingleUseTokens,
    pub revoke_user_sessions: Arc<RevokeUserSessionsUseCase>,
    pub confirm_password: Arc<ConfirmPasswordUseCase>,
    /// Direct TCP peers allowed to set `X-Forwarded-For` when resolving a client IP for throttling.
    pub trusted_proxy_ips: std::collections::HashSet<String>,
    /// `trusted_proxy_ips` parsed once: single addresses or CIDR ranges.
    pub trusted_proxies: Arc<artiferris_application::client_ip::TrustedProxies>,
    /// Request budgets of the public pages and endpoints, apart from the login counters so that visitors can neither crowd them out nor show up as blocked users.
    pub public_throttle: LoginThrottle,
    /// Budgets for audit events an actor can repeat at will, kept apart from `login_throttle` so they never show up as blocked logins.
    pub audit_throttle: LoginThrottle,
    pub get_system_settings: Arc<GetSystemSettingsUseCase>,
    pub update_system_settings: Arc<UpdateSystemSettingsUseCase>,
    pub get_smtp_settings: Arc<GetSmtpSettingsUseCase>,
    pub update_smtp_settings: Arc<UpdateSmtpSettingsUseCase>,
    pub send_test_email: Arc<SendTestEmailUseCase>,
    pub get_branding: Arc<GetBrandingUseCase>,
    pub set_branding_logo: Arc<SetBrandingLogoUseCase>,
    pub clear_branding_logo: Arc<ClearBrandingLogoUseCase>,
    pub set_branding_favicon: Arc<SetBrandingFaviconUseCase>,
    pub clear_branding_favicon: Arc<ClearBrandingFaviconUseCase>,
    pub invite_user: Arc<InviteUserUseCase>,
    pub resend_invitation: Arc<ResendInvitationUseCase>,
    pub activate_account: Arc<ActivateAccountUseCase>,
    pub user_invitations: Arc<dyn artiferris_domain::invitation::UserInvitationPort>,
    pub totp_credentials: Arc<dyn artiferris_domain::mfa::TotpCredentialPort>,
    /// Signs a "password verified" proof — never interchangeable with `token_issuer`.
    pub mfa_pending_token_issuer: Arc<dyn TokenIssuerPort>,
    pub get_mfa_status: Arc<GetMfaStatusUseCase>,
    pub enroll_totp: Arc<EnrollTotpUseCase>,
    pub confirm_totp: Arc<ConfirmTotpUseCase>,
    pub verify_totp: Arc<VerifyTotpUseCase>,
    pub verify_backup_code: Arc<VerifyBackupCodeUseCase>,
    pub disable_totp: Arc<DisableTotpUseCase>,
    pub regenerate_backup_codes: Arc<RegenerateBackupCodesUseCase>,
    pub webauthn_credentials: Arc<dyn artiferris_domain::webauthn::WebauthnCredentialPort>,
    pub start_passkey_registration: Arc<StartPasskeyRegistrationUseCase>,
    pub finish_passkey_registration: Arc<FinishPasskeyRegistrationUseCase>,
    pub start_passkey_authentication: Arc<StartPasskeyAuthenticationUseCase>,
    pub finish_passkey_authentication: Arc<FinishPasskeyAuthenticationUseCase>,
    pub list_passkeys: Arc<ListPasskeysUseCase>,
    pub delete_passkey: Arc<DeletePasskeyUseCase>,
}

const ACTOR_EVENT_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);
/// A flood is logged once per this, not once per dropped event.
const ACTOR_EVENT_WARNING_WINDOW: std::time::Duration = std::time::Duration::from_secs(300);

/// The budget group and per-minute limit of an event an actor can repeat at will, `None` for one that can't be looped.
fn actor_event_budget(event: &artiferris_domain::audit::SecurityEvent) -> Option<(&'static str, usize)> {
    use artiferris_domain::audit::SecurityEvent;
    match event {
        SecurityEvent::AccessDenied { .. } => Some(("access-denied", 60)),
        SecurityEvent::ApiTokenCreated { .. } | SecurityEvent::ApiTokenRevoked { .. } => Some(("api-token", 30)),
        _ => None,
    }
}

/// A write failure is logged, never surfaced. Events an actor can repeat at will (refused requests, token create and revoke) have a budget per actor:
/// past it the operation goes ahead but leaves no row, so one member can't bury their organization's real events.
pub async fn record_security_event(state: &AppState, event: artiferris_domain::audit::SecurityEvent, actor_id: Option<uuid::Uuid>) {
    if let (Some(actor_id), Some((group, limit))) = (actor_id, actor_event_budget(&event)) {
        let key = format!("{group}:{actor_id}");
        if !state.audit_throttle.reserve(&key, limit, ACTOR_EVENT_WINDOW) {
            if state.audit_throttle.reserve(&format!("{key}:warned"), 1, ACTOR_EVENT_WARNING_WINDOW) {
                tracing::warn!("{actor_id} is over the audit budget for {group} events ({limit} per minute), further ones are not recorded");
            }
            return;
        }
    }
    if let Err(e) = state.record_security_event.execute(event, actor_id).await {
        tracing::warn!("failed to record security event: {e}");
    }
}

/// Same contract as `record_security_event`: a write failure is logged, never surfaced.
pub async fn record_admin_event(state: &AppState, event: artiferris_domain::audit::AdminAuditEvent, actor_id: Option<uuid::Uuid>) {
    if let Err(e) = state.record_admin_event.execute(event, actor_id).await {
        tracing::warn!("failed to record admin audit event: {e}");
    }
}

impl AppState {
    pub fn build(pool: PgPool, config: &Config) -> Self {
        let started_at = Instant::now();
        let users_repo = Arc::new(PostgresUserRepository::new(pool.clone()));
        let organizations: Arc<dyn OrganizationRepositoryPort> = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let identity_providers: Arc<dyn IdentityProviderRepositoryPort> = Arc::new(PostgresIdentityProviderRepository::new(pool.clone(), config.secrets_encryption_key.clone()));
        let ldap_auth: Arc<dyn LdapAuthPort> = Arc::new(Ldap3AuthAdapter);
        let oidc_auth: Arc<dyn OidcAuthPort> = Arc::new(OpenidConnectAuthAdapter::new(config.jwt_secret.clone()));
        let create_organization = Arc::new(CreateOrganizationUseCase::new(organizations.clone()));
        let permission_store = Arc::new(PostgresPermissionStore::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), config.secrets_encryption_key.clone()));
        let repository_quota_lock: Arc<dyn RepositoryQuotaLockPort> = repository_store.clone();
        let event_publisher = Arc::new(PostgresEventPublisher::new(pool.clone()));
        let health_check = Arc::new(PostgresHealthCheck::new(pool.clone(), config.db_max_connections));
        let readiness = Arc::new(PostgresReadiness::new(pool.clone()));
        let hasher = Arc::new(Argon2PasswordHasher::new());
        let token_issuer = Arc::new(JwtTokenIssuer::new(config.jwt_secret.clone()));
        let storage = Arc::new(FilesystemStorageBackend::new(config.storage_root.clone()));
        let api_tokens: Arc<dyn ApiTokenRepositoryPort> = Arc::new(PostgresApiTokenRepository::new(pool.clone()));
        let npm_packages: Arc<dyn NpmPackageRepositoryPort> = Arc::new(PostgresNpmPackageRepository::new(pool.clone()));
        let npm_audit: Arc<dyn NpmAuditPort> = Arc::new(artiferris_infrastructure::http_npm_audit_client::HttpNpmAuditClient::new());
        let dependency_audits: Arc<dyn artiferris_domain::npm_audit::DependencyAuditRepositoryPort> =
            Arc::new(PostgresDependencyAuditRepository::new(pool.clone()));
        let public_registry: Arc<dyn artiferris_domain::npm_remote::RemoteNpmRegistryPort> =
            Arc::new(artiferris_infrastructure::http_remote_npm_registry::HttpRemoteNpmRegistry::new());
        let docker_blobs: Arc<dyn artiferris_domain::docker_registry::DockerBlobStorePort> = Arc::new(
            artiferris_infrastructure::filesystem_docker_blob_store::FilesystemDockerBlobStore::new(
                pool.clone(),
                std::path::Path::new(&config.storage_root).join("docker-blobs"),
            ),
        );
        let docker_manifests: Arc<dyn artiferris_domain::docker_registry::DockerManifestRepositoryPort> =
            Arc::new(artiferris_infrastructure::postgres::docker_manifest_repository::PostgresDockerManifestRepository::new(pool.clone()));
        let docker_uploads: Arc<dyn artiferris_domain::docker_registry::DockerUploadSessionPort> = Arc::new(
            artiferris_infrastructure::postgres::docker_upload_session_repository::PostgresDockerUploadSessionRepository::new(
                pool.clone(),
                std::path::Path::new(&config.storage_root).join("docker-uploads"),
            ),
        );
        let docker_image_scans: Arc<dyn artiferris_domain::docker_scan::DockerImageScanRepositoryPort> =
            Arc::new(PostgresDockerImageScanRepository::new(pool.clone()));
        // Trivy runs as a subprocess on this same host, reached over loopback.
        let registry_port = config.bind_addr.rsplit(':').next().unwrap_or("8080");
        let docker_scanner: Arc<dyn artiferris_domain::docker_scan::DockerImageScannerPort> =
            Arc::new(artiferris_infrastructure::trivy_docker_image_scanner::TrivyDockerImageScanner::new(format!("127.0.0.1:{registry_port}")));
        let docker_token_issuer: Arc<dyn artiferris_domain::docker_registry::DockerTokenIssuerPort> =
            Arc::new(artiferris_infrastructure::jwt_docker_token_issuer::JwtDockerTokenIssuer::new(config.jwt_secret.clone()));
        let metrics_snapshots: Arc<dyn artiferris_domain::metrics_snapshot::MetricsSnapshotRepositoryPort> =
            Arc::new(PostgresMetricsSnapshotRepository::new(pool.clone()));
        let get_usage_metrics = Arc::new(GetUsageMetricsUseCase::new(repository_store.clone(), storage.clone(), docker_blobs.clone()));
        let system_settings: Arc<dyn artiferris_domain::system_settings::SystemSettingsPort> = Arc::new(PostgresSystemSettingsRepository::new(pool.clone()));
        let smtp_settings: Arc<dyn artiferris_domain::email::SmtpSettingsPort> = Arc::new(PostgresSmtpSettingsRepository::new(pool.clone(), config.secrets_encryption_key.clone()));
        let branding: Arc<dyn artiferris_domain::branding::BrandingPort> = Arc::new(artiferris_infrastructure::postgres::branding_repository::PostgresBrandingRepository::new(pool.clone()));
        let user_preferences: Arc<dyn artiferris_domain::user_preferences::UserPreferencesPort> =
            Arc::new(artiferris_infrastructure::postgres::user_preferences_repository::PostgresUserPreferencesRepository::new(pool.clone()));
        let email_sender: Arc<dyn artiferris_domain::email::EmailPort> = Arc::new(SmtpEmailSender::new(smtp_settings.clone(), branding.clone()).with_preferences(user_preferences.clone()));
        let user_invitations: Arc<dyn artiferris_domain::invitation::UserInvitationPort> = Arc::new(PostgresUserInvitationRepository::new(pool.clone()));
        let totp_credentials: Arc<dyn artiferris_domain::mfa::TotpCredentialPort> = Arc::new(PostgresTotpCredentialRepository::new(pool.clone(), config.secrets_encryption_key.clone()));
        let backup_codes: Arc<dyn artiferris_domain::mfa::BackupCodePort> = Arc::new(PostgresBackupCodeRepository::new(pool.clone()));
        let mfa_pending_token_issuer: Arc<dyn TokenIssuerPort> = Arc::new(JwtMfaPendingTokenIssuer::new(config.jwt_secret.clone()));
        let webauthn_credentials: Arc<dyn artiferris_domain::webauthn::WebauthnCredentialPort> = Arc::new(PostgresWebauthnCredentialRepository::new(pool.clone()));
        // Degrades to "passkeys disabled" rather than refusing to start the server.
        let webauthn_client = Arc::new(match build_webauthn_client(&config.artiferris_base_domain, "ArtiFerris", &config.public_url) {
            Ok(client) => Some(client),
            Err(e) => {
                tracing::warn!("passkeys disabled: {e}. Set ARTIFERRIS_BASE_DOMAIN to this deployment's real base domain to enable them.");
                None
            }
        });
        let passkey_ceremonies = Arc::new(PasskeyCeremonyStore::new());
        let login_throttle = LoginThrottle::new();
        // A pending token lives 5 minutes; the extra minutes cover the JWT library's clock-skew leeway.
        let used_mfa_tokens = SingleUseTokens::new(std::time::Duration::from_secs(10 * 60));
        let user_security: Arc<dyn UserSecurityPort> = users_repo.clone();
        // Bound here so `sweep_retention` can reuse the same instances.
        let unpublish_npm_package = Arc::new(UnpublishNpmPackageUseCase::new(npm_packages.clone(), storage.clone(), event_publisher.clone()));
        let delete_docker_manifest = Arc::new(DeleteManifestUseCase::new(docker_manifests.clone(), docker_blobs.clone(), event_publisher.clone()));
        let delete_docker_image = Arc::new(DeleteDockerImageUseCase::new(docker_manifests.clone(), delete_docker_manifest.clone()));
        let sweep_retention = Arc::new(artiferris_application::use_cases::retention::SweepRetentionUseCase::new(
            repository_store.clone(),
            npm_packages.clone(),
            unpublish_npm_package.clone(),
            docker_manifests.clone(),
            delete_docker_manifest.clone(),
        ));
        let sweep_expired_uploads = Arc::new(artiferris_application::use_cases::docker_upload_sweep::SweepExpiredDockerUploadsUseCase::new(docker_uploads.clone(), docker_blobs.clone()));
        let sweep_repository_deletions = Arc::new(artiferris_application::use_cases::repository_deletion_sweep::RepositoryDeletionSweepUseCase::new(
            repository_store.clone(),
            docker_blobs.clone(),
            storage.clone(),
        ));

        // Bound here so `import_configuration` can reuse the same instances.
        let create_repository = Arc::new(CreatePackageRepositoryUseCase::new(repository_store.clone(), repository_store.clone()));
        let reserve_personal_organization = Arc::new(ReservePersonalOrganizationUseCase::new(organizations.clone(), users_repo.clone()));
        let find_my_personal_organization = Arc::new(FindMyPersonalOrganizationUseCase::new(organizations.clone()));
        let resolve_personal_repository = Arc::new(ResolvePersonalRepositoryUseCase::new(users_repo.clone(), organizations.clone(), repository_store.clone()));
        let list_readable_repositories = Arc::new(artiferris_application::use_cases::list_readable_repositories::ListReadableRepositoriesUseCase::new(organizations.clone(), repository_store.clone(), permission_store.clone()));
        let resolve_organization_repository = Arc::new(ResolveOrganizationRepositoryUseCase::new(organizations.clone(), repository_store.clone()));
        let public_catalog: Arc<dyn artiferris_domain::public_catalog::PublicCatalogPort> = Arc::new(CachedPublicCatalog::new(Arc::new(PostgresPublicCatalog::new(pool.clone()))));
        let download_stats: Arc<dyn artiferris_domain::download_stats::DownloadStatsPort> = Arc::new(PostgresDownloadStats::new(pool.clone()));
        let download_counter_buffer = Arc::new(DownloadCounterBuffer::new());
        let flush_downloads = Arc::new(FlushDownloadCountersUseCase::new(download_counter_buffer.clone(), download_stats.clone()));
        let prune_download_stats = Arc::new(PruneDownloadStatsUseCase::new(download_stats.clone()));
        let prune_audit_events = Arc::new(PruneAuditEventsUseCase::new(event_publisher.clone(), config.audit_retention_days));
        let seo_pages = Arc::new(artiferris_application::use_cases::seo::SeoPageUseCase::new(public_catalog.clone(), config.public_url.clone()));
        let search_readable_catalog = Arc::new(artiferris_application::use_cases::public_catalog::SearchReadableCatalogUseCase::new(
            public_catalog.clone(),
            permission_store.clone(),
            repository_store.clone(),
            organizations.clone(),
        ));
        let search_public_catalog = Arc::new(SearchPublicCatalogUseCase::new(public_catalog.clone()));
        let list_catalogs = Arc::new(ListCatalogsUseCase::new(public_catalog.clone()));
        let get_owner_summary = Arc::new(GetOwnerSummaryUseCase::new(public_catalog.clone()));
        let suggest_public_catalog = Arc::new(SuggestPublicCatalogUseCase::new(public_catalog.clone()));
        let create_user_project = Arc::new(CreateUserProjectUseCase::new(organizations.clone(), repository_store.clone(), repository_store.clone()));
        let set_repository_quota = Arc::new(SetRepositoryQuotaUseCase::new(repository_store.clone()));
        let set_retention_policy = Arc::new(SetRetentionPolicyUseCase::new(repository_store.clone()));
        let set_repository_visibility = Arc::new(SetRepositoryVisibilityUseCase::new(repository_store.clone()));
        let add_group_member = Arc::new(AddGroupMemberUseCase::new(repository_store.clone(), repository_store.clone()));
        let grant_permission = Arc::new(GrantPermissionUseCase::new(permission_store.clone(), repository_store.clone(), users_repo.clone()));
        let update_system_settings = Arc::new(UpdateSystemSettingsUseCase::new(system_settings.clone()));
        let import_configuration = Arc::new(ImportConfigurationUseCase::new(
            users_repo.clone(),
            hasher.clone(),
            Arc::new(PostgresConfigurationImport::new(pool.clone(), config.secrets_encryption_key.clone())),
            repository_store.clone(),
            email_sender.clone(),
            organizations.clone(),
            repository_quota_lock.clone(),
            config.artiferris_base_domain.clone(),
        ));

        Self {
            users: users_repo.clone(),
            user_preferences: user_preferences.clone(),
            permissions: permission_store.clone(),
            repositories: repository_store.clone(),
            repository_quota_lock: repository_quota_lock.clone(),
            organizations: organizations.clone(),
            identity_providers: identity_providers.clone(),
            ldap_auth: ldap_auth.clone(),
            oidc_auth: oidc_auth.clone(),
            provision_sso_user: Arc::new(ProvisionSsoUserUseCase::new(users_repo.clone(), user_security.clone(), hasher.clone(), token_issuer.clone(), system_settings.clone())),
            artiferris_base_domain: config.artiferris_base_domain.clone(),
            public_url: config.public_url.clone(),
            api_tokens: api_tokens.clone(),
            storage: storage.clone(),
            events: event_publisher.clone(),
            npm_packages: npm_packages.clone(),
            npm_audit: npm_audit.clone(),
            docker_blobs: docker_blobs.clone(),
            docker_manifests: docker_manifests.clone(),
            docker_uploads: docker_uploads.clone(),
            create_user: Arc::new(CreateUserUseCase::new(users_repo.clone(), hasher.clone())),
            register_public_user: Arc::new(RegisterPublicUserUseCase::new(users_repo.clone(), hasher.clone())),
            authenticate_user: Arc::new(AuthenticateUserUseCase::new(users_repo.clone(), hasher.clone(), token_issuer.clone(), system_settings.clone())),
            delete_user: Arc::new(DeleteUserUseCase::new(users_repo.clone())),
            set_super_admin: Arc::new(SetSuperAdminUseCase::new(users_repo.clone())),
            set_organization_admin: Arc::new(SetOrganizationAdminUseCase::new(users_repo.clone())),
            change_password: Arc::new(ChangePasswordUseCase::new(users_repo.clone(), user_security.clone(), hasher.clone(), email_sender.clone())),
            create_organization,
            create_repository,
            reserve_personal_organization,
            find_my_personal_organization,
            resolve_personal_repository,
            resolve_organization_repository,
            list_readable_repositories,
            search_public_catalog,
            search_readable_catalog,
            list_catalogs,
            get_owner_summary,
            suggest_public_catalog,
            public_catalog: public_catalog.clone(),
            seo_pages,
            download_counter_buffer,
            flush_downloads,
            prune_download_stats,
            prune_audit_events,
            download_stats: download_stats.clone(),
            create_user_project,
            rename_repository: Arc::new(RenamePackageRepositoryUseCase::new(repository_store.clone(), repository_store.clone())),
            delete_repository: Arc::new(DeletePackageRepositoryUseCase::new(repository_store.clone())),
            add_group_member,
            remove_group_member: Arc::new(RemoveGroupMemberUseCase::new(repository_store.clone())),
            set_repository_quota,
            set_retention_policy,
            set_repository_visibility,
            sweep_retention,
            sweep_expired_uploads,
            sweep_repository_deletions,
            grant_permission,
            revoke_permission: Arc::new(RevokePermissionUseCase::new(permission_store.clone())),
            query_audit_log: Arc::new(QueryAuditLogUseCase::new(event_publisher.clone())),
            record_security_event: Arc::new(RecordSecurityEventUseCase::new(event_publisher.clone())),
            record_admin_event: Arc::new(RecordAdminEventUseCase::new(event_publisher.clone())),
            get_usage_metrics: get_usage_metrics.clone(),
            get_metrics_history: Arc::new(GetMetricsHistoryUseCase::new(metrics_snapshots.clone())),
            record_metrics_snapshot: Arc::new(RecordMetricsSnapshotUseCase::new(users_repo.clone(), get_usage_metrics, metrics_snapshots)),
            get_health_status: Arc::new(GetHealthStatusUseCase::new(health_check.clone(), storage.clone(), started_at)),
            readiness,
            get_admin_stats: Arc::new(GetAdminStatsUseCase::new(users_repo.clone(), repository_store.clone(), permission_store.clone())),
            export_configuration: Arc::new(ExportConfigurationUseCase::new(users_repo.clone(), repository_store.clone(), permission_store.clone(), system_settings.clone(), organizations.clone())),
            import_configuration,
            create_api_token: Arc::new(CreateApiTokenUseCase::new(api_tokens.clone())),
            list_api_tokens: Arc::new(ListApiTokensUseCase::new(api_tokens.clone())),
            revoke_api_token: Arc::new(RevokeApiTokenUseCase::new(api_tokens.clone())),
            admin_list_api_tokens: Arc::new(AdminListApiTokensUseCase::new(api_tokens.clone())),
            admin_revoke_api_token: Arc::new(AdminRevokeApiTokenUseCase::new(api_tokens.clone())),
            list_repository_packages: Arc::new(ListRepositoryPackagesUseCase::new(
                npm_packages.clone(),
                dependency_audits.clone(),
                docker_manifests.clone(),
                docker_image_scans.clone(),
            )),
            get_npm_package_details: Arc::new(GetNpmPackageDetailsUseCase::new(npm_packages.clone(), Arc::new(artiferris_infrastructure::readme_renderer::PulldownAmmoniaReadmeRenderer::new()), download_stats.clone())),
            get_docker_image_details: Arc::new(GetDockerImageDetailsUseCase::new(docker_manifests.clone(), download_stats.clone())),
            unpublish_npm_package: unpublish_npm_package.clone(),
            audit_npm_package: Arc::new(AuditNpmPackageUseCase::new(npm_packages.clone(), npm_audit.clone())),
            scan_dependency_tree: Arc::new(ScanDependencyTreeUseCase::new(
                npm_packages.clone(),
                public_registry,
                npm_audit.clone(),
                dependency_audits.clone(),
            )),
            get_dependency_audit: Arc::new(GetDependencyAuditResultUseCase::new(npm_packages.clone(), dependency_audits)),
            scan_docker_image: Arc::new(ScanDockerImageUseCase::new(
                docker_manifests.clone(),
                repository_store.clone(),
                docker_token_issuer,
                docker_scanner,
                docker_image_scans.clone(),
            )),
            get_docker_image_scan: Arc::new(GetDockerImageScanResultUseCase::new(docker_manifests.clone(), docker_image_scans)),
            delete_docker_manifest: delete_docker_manifest.clone(),
            delete_docker_image,
            token_issuer,
            login_throttle,
            used_mfa_tokens,
            revoke_user_sessions: Arc::new(RevokeUserSessionsUseCase::new(user_security.clone())),
            confirm_password: Arc::new(ConfirmPasswordUseCase::new(users_repo.clone(), hasher.clone())),
            trusted_proxy_ips: config.trusted_proxy_ips.clone(),
            trusted_proxies: Arc::new(
                artiferris_application::client_ip::TrustedProxies::parse(config.trusted_proxy_ips.iter().map(String::as_str)).unwrap_or_else(|e| {
                    tracing::warn!("ignoring TRUSTED_PROXY_IPS: {e}");
                    Default::default()
                }),
            ),
            public_throttle: LoginThrottle::new(),
            audit_throttle: LoginThrottle::new(),
            get_system_settings: Arc::new(GetSystemSettingsUseCase::new(system_settings.clone())),
            update_system_settings,
            get_smtp_settings: Arc::new(GetSmtpSettingsUseCase::new(smtp_settings.clone())),
            update_smtp_settings: Arc::new(UpdateSmtpSettingsUseCase::new(smtp_settings)),
            send_test_email: Arc::new(SendTestEmailUseCase::new(email_sender.clone())),
            get_branding: Arc::new(GetBrandingUseCase::new(
                branding.clone(),
                BrandingDefaults {
                    logo: BrandingAsset { bytes: branding_defaults::DEFAULT_LOGO_BYTES.to_vec(), content_type: branding_defaults::DEFAULT_LOGO_CONTENT_TYPE.to_string() },
                    favicon: BrandingAsset { bytes: branding_defaults::DEFAULT_FAVICON_BYTES.to_vec(), content_type: branding_defaults::DEFAULT_FAVICON_CONTENT_TYPE.to_string() },
                },
            )),
            set_branding_logo: Arc::new(SetBrandingLogoUseCase::new(branding.clone())),
            clear_branding_logo: Arc::new(ClearBrandingLogoUseCase::new(branding.clone())),
            set_branding_favicon: Arc::new(SetBrandingFaviconUseCase::new(branding.clone())),
            clear_branding_favicon: Arc::new(ClearBrandingFaviconUseCase::new(branding)),
            // Scoped to artiferris_base_domain, not public_url — the invitation must link to
            // the invitee's own organization's subdomain.
            invite_user: Arc::new(InviteUserUseCase::new(
                users_repo.clone(),
                user_security.clone(),
                user_invitations.clone(),
                hasher.clone(),
                email_sender.clone(),
                organizations.clone(),
                config.artiferris_base_domain.clone(),
            )),
            resend_invitation: Arc::new(ResendInvitationUseCase::new(users_repo.clone(), user_invitations.clone(), email_sender.clone(), organizations.clone(), config.artiferris_base_domain.clone())),
            activate_account: Arc::new(ActivateAccountUseCase::new(users_repo.clone(), user_security.clone(), user_invitations.clone(), hasher.clone())),
            user_invitations,
            totp_credentials: totp_credentials.clone(),
            mfa_pending_token_issuer,
            get_mfa_status: Arc::new(GetMfaStatusUseCase::new(totp_credentials.clone(), backup_codes.clone(), webauthn_credentials.clone())),
            enroll_totp: Arc::new(EnrollTotpUseCase::new(totp_credentials.clone(), users_repo.clone(), hasher.clone())),
            confirm_totp: Arc::new(ConfirmTotpUseCase::new(totp_credentials.clone(), backup_codes.clone(), users_repo.clone(), user_security.clone(), email_sender.clone())),
            verify_totp: Arc::new(VerifyTotpUseCase::new(totp_credentials.clone())),
            verify_backup_code: Arc::new(VerifyBackupCodeUseCase::new(backup_codes.clone())),
            disable_totp: Arc::new(DisableTotpUseCase::new(users_repo.clone(), hasher.clone(), totp_credentials.clone(), backup_codes.clone())),
            regenerate_backup_codes: Arc::new(RegenerateBackupCodesUseCase::new(users_repo.clone(), hasher.clone(), totp_credentials, backup_codes)),
            webauthn_credentials: webauthn_credentials.clone(),
            start_passkey_registration: Arc::new(StartPasskeyRegistrationUseCase::new(webauthn_client.clone(), webauthn_credentials.clone(), passkey_ceremonies.clone(), users_repo.clone(), hasher.clone())),
            finish_passkey_registration: Arc::new(FinishPasskeyRegistrationUseCase::new(webauthn_client.clone(), webauthn_credentials.clone(), passkey_ceremonies.clone(), users_repo.clone(), user_security.clone(), email_sender.clone())),
            start_passkey_authentication: Arc::new(StartPasskeyAuthenticationUseCase::new(webauthn_client.clone(), webauthn_credentials.clone(), passkey_ceremonies.clone())),
            finish_passkey_authentication: Arc::new(FinishPasskeyAuthenticationUseCase::new(webauthn_client, webauthn_credentials.clone(), passkey_ceremonies)),
            list_passkeys: Arc::new(ListPasskeysUseCase::new(webauthn_credentials.clone())),
            delete_passkey: Arc::new(DeletePasskeyUseCase::new(users_repo, hasher, webauthn_credentials)),
        }
    }
}
