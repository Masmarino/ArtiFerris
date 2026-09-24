use std::sync::Arc;

use artiferris_application::request_guard::RequestGuard;
use artiferris_application::use_cases::api_token::{CreateApiTokenUseCase, ListApiTokensUseCase, RevokeApiTokenUseCase};
use artiferris_application::use_cases::npm_audit::BulkAuditNpmPackagesUseCase;
use artiferris_application::use_cases::npm_dependency_scan::ScanDependencyTreeUseCase;
use artiferris_application::use_cases::npm_deprecate::DeprecateNpmVersionUseCase;
use artiferris_application::use_cases::npm_dist_tags::{DeleteDistTagUseCase, ListDistTagsUseCase, SetDistTagUseCase};
use artiferris_application::use_cases::npm_download::DownloadNpmTarballUseCase;
use artiferris_application::use_cases::npm_metadata::GetNpmPackageMetadataUseCase;
use artiferris_application::use_cases::npm_publish::PublishNpmPackageUseCase;
use artiferris_application::use_cases::npm_search::SearchNpmPackagesUseCase;
use artiferris_application::use_cases::npm_unpublish::UnpublishNpmPackageUseCase;
use artiferris_application::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase;
use artiferris_domain::api_token::ApiTokenRepositoryPort;
use artiferris_domain::organization::OrganizationRepositoryPort;
use artiferris_domain::package_repository::PackageRepositoryQueryPort;
use artiferris_domain::permission::PermissionQueryPort;
use artiferris_domain::user::UserRepositoryPort;

#[derive(Clone)]
pub struct NpmState {
    pub users: Arc<dyn UserRepositoryPort>,
    pub repositories: Arc<dyn PackageRepositoryQueryPort>,
    pub permissions: Arc<dyn PermissionQueryPort>,
    pub api_tokens: Arc<dyn ApiTokenRepositoryPort>,
    pub organizations: Arc<dyn OrganizationRepositoryPort>,
    /// Base domain `ResolvedOrganization` strips off the `Host` header to find the subdomain label.
    pub artiferris_base_domain: String,
    /// `"http"` or `"https"`, from `PUBLIC_URL` — the scheme of the tarball URLs handed to clients.
    pub public_scheme: String,
    /// Client address resolution, the body-memory budget and the download dedupe.
    pub guard: Arc<RequestGuard>,
    pub publish: Arc<PublishNpmPackageUseCase>,
    pub metadata: Arc<GetNpmPackageMetadataUseCase>,
    pub download: Arc<DownloadNpmTarballUseCase>,
    /// Counts tarball downloads served straight from a hosted repository (not HEAD requests).
    pub downloads: Arc<dyn artiferris_domain::download_stats::DownloadRecorderPort>,
    pub unpublish: Arc<UnpublishNpmPackageUseCase>,
    pub deprecate: Arc<DeprecateNpmVersionUseCase>,
    pub set_dist_tag: Arc<SetDistTagUseCase>,
    pub delete_dist_tag: Arc<DeleteDistTagUseCase>,
    pub list_dist_tags: Arc<ListDistTagsUseCase>,
    pub search: Arc<SearchNpmPackagesUseCase>,
    pub bulk_audit: Arc<BulkAuditNpmPackagesUseCase>,
    pub scan_dependency_tree: Arc<ScanDependencyTreeUseCase>,
    pub create_api_token: Arc<CreateApiTokenUseCase>,
    pub list_api_tokens: Arc<ListApiTokensUseCase>,
    pub revoke_api_token: Arc<RevokeApiTokenUseCase>,
    /// Resolves `/u/{username}/{repo}` to a personal project — shared with `artiferris-docker`.
    pub resolve_personal_repository: Arc<ResolvePersonalRepositoryUseCase>,
}
