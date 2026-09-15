use std::sync::Arc;

use bunker_application::use_cases::api_token::{CreateApiTokenUseCase, ListApiTokensUseCase, RevokeApiTokenUseCase};
use bunker_application::use_cases::npm_audit::BulkAuditNpmPackagesUseCase;
use bunker_application::use_cases::npm_dependency_scan::ScanDependencyTreeUseCase;
use bunker_application::use_cases::npm_deprecate::DeprecateNpmVersionUseCase;
use bunker_application::use_cases::npm_dist_tags::{DeleteDistTagUseCase, ListDistTagsUseCase, SetDistTagUseCase};
use bunker_application::use_cases::npm_download::DownloadNpmTarballUseCase;
use bunker_application::use_cases::npm_metadata::GetNpmPackageMetadataUseCase;
use bunker_application::use_cases::npm_publish::PublishNpmPackageUseCase;
use bunker_application::use_cases::npm_search::SearchNpmPackagesUseCase;
use bunker_application::use_cases::npm_unpublish::UnpublishNpmPackageUseCase;
use bunker_domain::api_token::ApiTokenRepositoryPort;
use bunker_domain::organization::OrganizationRepositoryPort;
use bunker_domain::package_repository::PackageRepositoryQueryPort;
use bunker_domain::permission::PermissionQueryPort;
use bunker_domain::user::UserRepositoryPort;

#[derive(Clone)]
pub struct NpmState {
    pub users: Arc<dyn UserRepositoryPort>,
    pub repositories: Arc<dyn PackageRepositoryQueryPort>,
    pub permissions: Arc<dyn PermissionQueryPort>,
    pub api_tokens: Arc<dyn ApiTokenRepositoryPort>,
    pub organizations: Arc<dyn OrganizationRepositoryPort>,
    /// Base domain `ResolvedOrganization` strips off the `Host` header to find the subdomain label.
    pub hangar_base_domain: String,
    pub publish: Arc<PublishNpmPackageUseCase>,
    pub metadata: Arc<GetNpmPackageMetadataUseCase>,
    pub download: Arc<DownloadNpmTarballUseCase>,
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
}
