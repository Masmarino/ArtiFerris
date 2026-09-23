use std::sync::Arc;

use artiferris_application::use_cases::docker_access_token::IssueDockerAccessTokenUseCase;
use artiferris_application::use_cases::docker_blob_get::GetBlobUseCase;
use artiferris_application::use_cases::docker_list::{ListCatalogUseCase, ListDockerRegistryCatalogUseCase, ListTagsUseCase};
use artiferris_application::use_cases::docker_manifest_cache::CacheProxiedManifestUseCase;
use artiferris_application::use_cases::docker_manifest_delete::DeleteManifestUseCase;
use artiferris_application::use_cases::docker_scan::ScanDockerImageUseCase;
use artiferris_application::use_cases::docker_manifest_get::GetManifestUseCase;
use artiferris_application::use_cases::docker_manifest_put::PutManifestUseCase;
use artiferris_application::use_cases::docker_upload::{CompleteBlobUploadUseCase, MonolithicBlobUploadUseCase, PatchBlobUploadUseCase, StartBlobUploadUseCase};
use artiferris_domain::docker_registry::DockerTokenIssuerPort;
use artiferris_domain::organization::OrganizationRepositoryPort;
use artiferris_domain::package_repository::PackageRepositoryQueryPort;
use artiferris_domain::permission::PermissionQueryPort;

#[derive(Clone)]
pub struct DockerState {
    pub repositories: Arc<dyn PackageRepositoryQueryPort>,
    pub permissions: Arc<dyn PermissionQueryPort>,
    pub organizations: Arc<dyn OrganizationRepositoryPort>,
    /// Base domain `ResolvedOrganization` strips off the `Host` header to find the subdomain label.
    pub artiferris_base_domain: String,
    pub token_issuer: Arc<dyn DockerTokenIssuerPort>,
    /// `ARTIFERRIS_DOCKER_TOKEN_REALM` override. `None` (the default) derives the realm per request instead — see `token_realm`.
    pub token_realm_override: Option<String>,
    /// `"http"` or `"https"`, from `PUBLIC_URL` — a proxied request can't tell this from its own `Host` header.
    pub public_scheme: String,
    pub token_service: String,
    pub issue_access_token: Arc<IssueDockerAccessTokenUseCase>,
    pub start_upload: Arc<StartBlobUploadUseCase>,
    pub patch_upload: Arc<PatchBlobUploadUseCase>,
    pub complete_upload: Arc<CompleteBlobUploadUseCase>,
    pub monolithic_upload: Arc<MonolithicBlobUploadUseCase>,
    pub put_manifest: Arc<PutManifestUseCase>,
    pub cache_proxied_manifest: Arc<CacheProxiedManifestUseCase>,
    pub get_manifest: Arc<GetManifestUseCase>,
    pub get_blob: Arc<GetBlobUseCase>,
    pub delete_manifest: Arc<DeleteManifestUseCase>,
    pub list_tags: Arc<ListTagsUseCase>,
    pub list_catalog: Arc<ListCatalogUseCase>,
    pub list_registry_catalog: Arc<ListDockerRegistryCatalogUseCase>,
    pub scan_docker_image: Arc<ScanDockerImageUseCase>,
}

impl DockerState {
    /// Built from the request's own `Host`, matching `ResolvedOrganization` — a realm that
    /// didn't match would send the client to fetch a token for the wrong organization.
    pub fn token_realm(&self, host: &str) -> String {
        match &self.token_realm_override {
            Some(realm) => realm.clone(),
            None => format!("{}://{host}/v2/token", self.public_scheme),
        }
    }

    /// Falls back to `artiferris_base_domain` if there's no `Host` header at all.
    pub fn host_header<'a>(&'a self, headers: &'a axum::http::HeaderMap) -> &'a str {
        headers.get(axum::http::header::HOST).and_then(|v| v.to_str().ok()).unwrap_or(&self.artiferris_base_domain)
    }
}

#[cfg(test)]
mod tests {
    use crate::route_test_support::test_state;
    use sqlx::PgPool;

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn derives_the_realm_from_the_given_host_and_its_own_public_scheme_when_no_override_is_set(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.token_realm_override = None;
        state.public_scheme = "https".to_string();

        assert_eq!(state.token_realm("acme.artiferris.pro"), "https://acme.artiferris.pro/v2/token");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_different_host_derives_a_different_realm(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.token_realm_override = None;

        assert_ne!(state.token_realm("acme.artiferris.pro"), state.token_realm("other.artiferris.pro"));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_override_wins_over_the_request_host_when_set(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.token_realm_override = Some("https://fixed.example.com/v2/token".to_string());

        assert_eq!(state.token_realm("acme.artiferris.pro"), "https://fixed.example.com/v2/token");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn host_header_reads_the_host_header(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "acme.artiferris.pro".parse().unwrap());

        assert_eq!(state.host_header(&headers), "acme.artiferris.pro");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn host_header_falls_back_to_the_base_domain_when_the_request_has_none(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let headers = axum::http::HeaderMap::new();

        assert_eq!(state.host_header(&headers), state.artiferris_base_domain);
    }
}
