use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use artiferris_domain::permission::Role;
use serde::Deserialize;
use serde_json::json;

use crate::authz::{require_npm_format_repository, require_readable_repository_by_name, require_repository_role};
use crate::auth::NpmAuthUser;
use crate::errors::npm_error_response;
use crate::organization_resolution::ResolvedOrganization;
use crate::state::NpmState;

pub fn router() -> Router<NpmState> {
    Router::new().route("/{repository}/-/v1/search", get(search))
}

#[derive(Deserialize)]
struct SearchQuery {
    text: String,
    #[serde(default = "default_size")]
    size: i64,
}

fn default_size() -> i64 {
    20
}

/// Matches the real npm registry's own default/max — negative or unbounded otherwise.
fn clamp_size(size: i64) -> i64 {
    size.clamp(1, 250)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_size_is_clamped_to_the_minimum() {
        assert_eq!(clamp_size(-1), 1);
        assert_eq!(clamp_size(i64::MIN), 1);
    }

    #[test]
    fn an_excessive_size_is_clamped_to_the_maximum() {
        assert_eq!(clamp_size(10_000), 250);
        assert_eq!(clamp_size(i64::MAX), 250);
    }

    #[test]
    fn an_in_range_size_is_left_untouched() {
        assert_eq!(clamp_size(20), 20);
    }

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
    use artiferris_domain::organization::{Organization, OrganizationSlug};
    use artiferris_infrastructure::filesystem_storage::FilesystemStorageBackend;
    use artiferris_infrastructure::http_npm_audit_client::HttpNpmAuditClient;
    use artiferris_infrastructure::http_remote_npm_registry::HttpRemoteNpmRegistry;
    use artiferris_infrastructure::postgres::api_token_repository::PostgresApiTokenRepository;
    use artiferris_infrastructure::postgres::npm_dependency_audit_repository::PostgresDependencyAuditRepository;
    use artiferris_infrastructure::postgres::npm_package_repository::PostgresNpmPackageRepository;
    use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
    use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
    use artiferris_infrastructure::postgres::permission_store::PostgresPermissionStore;
    use artiferris_infrastructure::postgres::user_repository::PostgresUserRepository;
    use artiferris_infrastructure::postgres::event_publisher::PostgresEventPublisher;
    use axum::body::Body;
    use axum::http::Request;
    use sqlx::PgPool;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn test_state(pool: PgPool, root: &std::path::Path) -> NpmState {
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repositories = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let permissions = Arc::new(PostgresPermissionStore::new(pool.clone()));
        let api_tokens: Arc<dyn artiferris_domain::api_token::ApiTokenRepositoryPort> = Arc::new(PostgresApiTokenRepository::new(pool.clone()));
        let organizations: Arc<dyn artiferris_domain::organization::OrganizationRepositoryPort> = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let npm_packages: Arc<dyn artiferris_domain::npm_package::NpmPackageRepositoryPort> = Arc::new(PostgresNpmPackageRepository::new(pool.clone()));
        let npm_audit: Arc<dyn artiferris_domain::npm_audit::NpmAuditPort> = Arc::new(HttpNpmAuditClient::new());
        let dependency_audits: Arc<dyn artiferris_domain::npm_audit::DependencyAuditRepositoryPort> = Arc::new(PostgresDependencyAuditRepository::new(pool.clone()));
        let storage: Arc<dyn artiferris_domain::storage::StorageBackendPort> = Arc::new(FilesystemStorageBackend::new(root.to_path_buf()));
        let events: Arc<dyn artiferris_domain::audit::EventPublisherPort> = Arc::new(PostgresEventPublisher::new(pool.clone()));
        let remote_registry: Arc<dyn artiferris_domain::npm_remote::RemoteNpmRegistryPort> = Arc::new(HttpRemoteNpmRegistry::new());

        NpmState {
            users: users.clone(),
            repositories: repositories.clone(),
            permissions: permissions.clone(),
            api_tokens: api_tokens.clone(),
            organizations: organizations.clone(),
            artiferris_base_domain: "artiferris.localhost".to_string(),
            public_scheme: "http".to_string(),
            guard: std::sync::Arc::new(artiferris_application::request_guard::RequestGuard::default()),
            publish: Arc::new(PublishNpmPackageUseCase::new(npm_packages.clone(), storage.clone(), repositories.clone(), repositories.clone(), events.clone())),
            metadata: Arc::new(GetNpmPackageMetadataUseCase::new(npm_packages.clone(), repositories.clone(), remote_registry.clone())),
            download: Arc::new(DownloadNpmTarballUseCase::new(npm_packages.clone(), storage.clone(), remote_registry.clone(), repositories.clone())),
            downloads: Arc::new(artiferris_domain::download_stats::NoopDownloadRecorder),
            unpublish: Arc::new(UnpublishNpmPackageUseCase::new(npm_packages.clone(), storage.clone(), events.clone())),
            deprecate: Arc::new(DeprecateNpmVersionUseCase::new(npm_packages.clone(), events.clone())),
            set_dist_tag: Arc::new(SetDistTagUseCase::new(npm_packages.clone(), events.clone())),
            delete_dist_tag: Arc::new(DeleteDistTagUseCase::new(npm_packages.clone())),
            list_dist_tags: Arc::new(ListDistTagsUseCase::new(npm_packages.clone())),
            search: Arc::new(SearchNpmPackagesUseCase::new(npm_packages.clone())),
            bulk_audit: Arc::new(BulkAuditNpmPackagesUseCase::new(npm_audit.clone())),
            scan_dependency_tree: Arc::new(ScanDependencyTreeUseCase::new(npm_packages.clone(), remote_registry.clone(), npm_audit.clone(), dependency_audits.clone())),
            create_api_token: Arc::new(CreateApiTokenUseCase::new(api_tokens.clone())),
            list_api_tokens: Arc::new(ListApiTokensUseCase::new(api_tokens.clone())),
            revoke_api_token: Arc::new(RevokeApiTokenUseCase::new(api_tokens.clone())),
            resolve_personal_repository: Arc::new(ResolvePersonalRepositoryUseCase::new(users.clone(), organizations.clone(), repositories.clone())),
        }
    }

    async fn create_org(state: &NpmState, id: Uuid, slug: &str) {
        state
            .organizations
            .create(&Organization {
                id,
                slug: OrganizationSlug::parse(slug).unwrap(),
                display_name: slug.to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
    }

    async fn seed_repository(pool: &PgPool, organization_id: Uuid, id: Uuid, format: &str, repo_type: &str) {
        sqlx::query!(
            "INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, remote_url, version, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, NULL, 1, now(), now())",
            id,
            organization_id,
            format!("repo-{id}"),
            format,
            repo_type,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Marks a repository public through the event store, like `routes::metadata`'s tests.
    async fn mark_repository_public(pool: &PgPool, repository_id: Uuid) {
        use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort};
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let (version, _) = store.load(repository_id).await.unwrap();
        store
            .append(repository_id, version, vec![PackageRepositoryEvent::VisibilityChanged { repository_id, is_public: true }], Uuid::new_v4())
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_is_reachable_anonymously_against_a_public_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        create_org(&state, acme_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        mark_repository_public(&pool, repo_id).await;
        let repo_name = format!("repo-{repo_id}");

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/-/v1/search?text=foo"))
                    .header("host", "acme.artiferris.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "search must not require authentication against a public repository, matching get_metadata/get_tarball"
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_is_not_found_anonymously_against_a_private_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        create_org(&state, acme_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        // Deliberately not marked public: stays private.
        let repo_name = format!("repo-{repo_id}");

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/-/v1/search?text=foo"))
                    .header("host", "acme.artiferris.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::NOT_FOUND,
            "search against a private repository without credentials must 404, same 404-not-401 discipline as get_metadata/get_tarball"
        );
    }
}

async fn search(
    State(state): State<NpmState>,
    axum::extract::Path(repository): axum::extract::Path<String>,
    Query(params): Query<SearchQuery>,
    resolved_org: ResolvedOrganization,
    user: Option<NpmAuthUser>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let (repo, caller) = require_readable_repository_by_name(&state, user.as_ref(), resolved_org.0.id, &repository)
        .await
        .map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_npm_format_repository(&repo).map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    if let Some(caller) = caller {
        require_repository_role(&state, caller, repo.id, repo.organization_id, Role::Read).await.map_err(|s| (s, Json(json!({ "error": "forbidden" }))))?;
    }

    let size = clamp_size(params.size);
    let packages = state.search.execute(repo.id, &params.text, size).await.map_err(npm_error_response)?;
    let objects: Vec<serde_json::Value> = packages
        .into_iter()
        // Real `npm search` unconditionally calls `.map()` on `maintainers` —
        // an empty array (not a missing key) is required or the CLI crashes.
        .map(|p| json!({ "package": { "name": p.name.as_str(), "maintainers": [] } }))
        .collect();
    Ok(Json(json!({ "objects": objects, "total": objects.len() })))
}
