use std::sync::Arc;

use artiferris_application::login_throttle::LoginThrottle;
use artiferris_application::request_guard::RequestGuard;
use artiferris_application::use_cases::admin::RecordSecurityEventUseCase;
use artiferris_application::use_cases::docker_access_token::IssueDockerAccessTokenUseCase;
use artiferris_application::use_cases::docker_blob_get::GetBlobUseCase;
use artiferris_application::use_cases::docker_list::{ListCatalogUseCase, ListDockerRegistryCatalogUseCase, ListTagsUseCase};
use artiferris_application::use_cases::docker_manifest_cache::CacheProxiedManifestUseCase;
use artiferris_application::use_cases::docker_manifest_delete::DeleteManifestUseCase;
use artiferris_application::use_cases::docker_scan::ScanDockerImageUseCase;
use artiferris_application::use_cases::docker_manifest_get::GetManifestUseCase;
use artiferris_application::use_cases::docker_manifest_put::PutManifestUseCase;
use artiferris_application::use_cases::docker_upload::{CompleteBlobUploadUseCase, MonolithicBlobUploadUseCase, PatchBlobUploadUseCase, StartBlobUploadUseCase};
use artiferris_application::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase;
use artiferris_domain::docker_registry::DockerTokenIssuerPort;
use artiferris_domain::organization::OrganizationRepositoryPort;
use artiferris_domain::package_repository::PackageRepositoryQueryPort;
use artiferris_domain::permission::PermissionQueryPort;
use artiferris_domain::user::UserRepositoryPort;

use crate::tokens_valid_after_cache::TokensValidAfterCache;

/// How long a user's `tokens_valid_after` may be served from cache: the worst-case delay before a revocation stops an
/// issued access token. Well under the token's own 5-minute TTL.
pub const TOKENS_VALID_AFTER_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone)]
pub struct DockerState {
    pub repositories: Arc<dyn PackageRepositoryQueryPort>,
    pub permissions: Arc<dyn PermissionQueryPort>,
    pub organizations: Arc<dyn OrganizationRepositoryPort>,
    /// Only read by `DockerAuthUser`'s revocation check — the data plane otherwise trusts the token's claims.
    pub users: Arc<dyn UserRepositoryPort>,
    pub tokens_valid_after_cache: TokensValidAfterCache,
    /// Base domain `ResolvedOrganization` strips off the `Host` header to find the subdomain label.
    pub artiferris_base_domain: String,
    pub token_issuer: Arc<dyn DockerTokenIssuerPort>,
    /// `ARTIFERRIS_DOCKER_TOKEN_REALM` override. `None` (the default) derives the realm per request instead — see `token_realm`.
    pub token_realm_override: Option<String>,
    /// `"http"` or `"https"`, from `PUBLIC_URL` — a proxied request can't tell this from its own `Host` header.
    pub public_scheme: String,
    pub token_service: String,
    pub issue_access_token: Arc<IssueDockerAccessTokenUseCase>,
    /// Throttles `/v2/token` credential guessing by client address — Basic auth here carries no reliable username (M-11).
    pub login_throttle: LoginThrottle,
    /// Client address resolution, the body-memory budget and the download dedupe.
    pub guard: Arc<RequestGuard>,
    pub record_security_event: Arc<RecordSecurityEventUseCase>,
    pub start_upload: Arc<StartBlobUploadUseCase>,
    pub patch_upload: Arc<PatchBlobUploadUseCase>,
    pub complete_upload: Arc<CompleteBlobUploadUseCase>,
    pub monolithic_upload: Arc<MonolithicBlobUploadUseCase>,
    pub put_manifest: Arc<PutManifestUseCase>,
    pub cache_proxied_manifest: Arc<CacheProxiedManifestUseCase>,
    pub get_manifest: Arc<GetManifestUseCase>,
    /// Counts manifest pulls served straight from a hosted repository (not HEAD requests).
    pub downloads: Arc<dyn artiferris_domain::download_stats::DownloadRecorderPort>,
    pub get_blob: Arc<GetBlobUseCase>,
    pub delete_manifest: Arc<DeleteManifestUseCase>,
    pub list_tags: Arc<ListTagsUseCase>,
    pub list_catalog: Arc<ListCatalogUseCase>,
    pub list_registry_catalog: Arc<ListDockerRegistryCatalogUseCase>,
    pub scan_docker_image: Arc<ScanDockerImageUseCase>,
    /// Resolves `/u/{username}/{repo}` to a personal project — shared with `artiferris-npm`.
    pub resolve_personal_repository: Arc<ResolvePersonalRepositoryUseCase>,
}

impl DockerState {
    /// Built from the request's `Host`, like `ResolvedOrganization`. A `Host` outside this instance's base domain and
    /// its subdomains is replaced by the base domain.
    pub fn token_realm(&self, host: &str) -> String {
        match &self.token_realm_override {
            Some(realm) => realm.clone(),
            None => format!("{}://{}/v2/token", self.public_scheme, self.own_host(host)),
        }
    }

    fn own_host<'a>(&'a self, host: &'a str) -> &'a str {
        artiferris_application::base_domain::own_host(host, &self.artiferris_base_domain).unwrap_or(&self.artiferris_base_domain)
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
        state.artiferris_base_domain = "artiferris.pro".to_string();

        assert_eq!(state.token_realm("acme.artiferris.pro"), "https://acme.artiferris.pro/v2/token");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_different_host_derives_a_different_realm(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.token_realm_override = None;
        state.artiferris_base_domain = "artiferris.pro".to_string();

        assert_ne!(state.token_realm("acme.artiferris.pro"), state.token_realm("other.artiferris.pro"));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_override_wins_over_the_request_host_when_set(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.artiferris_base_domain = "artiferris.pro".to_string();
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

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_host_that_is_not_ours_never_reaches_the_realm(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.token_realm_override = None;
        state.public_scheme = "https".to_string();
        state.artiferris_base_domain = "artiferris.pro".to_string();
        let expected = "https://artiferris.pro/v2/token";

        for hostile in ["evil.example", "artiferris.pro.evil.example", "artiferris.pro:x@evil.example", "artiferris.pro:@evil", "notartiferris.pro", ".artiferris.pro", "acme.artiferris.pro:80/../x", "artiferris.pro:", "evil.example/x.artiferris.pro", "evil.example#.artiferris.pro", "a@evil.example/.artiferris.pro", "evil.example?.artiferris.pro"] {
            assert_eq!(state.token_realm(hostile), expected, "{hostile:?}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn our_own_hosts_keep_their_case_and_port(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool, dir.path()).await;
        state.token_realm_override = None;
        state.public_scheme = "http".to_string();
        state.artiferris_base_domain = "artiferris.pro".to_string();

        assert_eq!(state.token_realm("artiferris.pro"), "http://artiferris.pro/v2/token");
        assert_eq!(state.token_realm("Acme.Artiferris.pro:8080"), "http://Acme.Artiferris.pro:8080/v2/token");
    }
}
