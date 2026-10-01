use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use artiferris_domain::npm_package::{NpmPackageName, NpmVersion};
use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryFormat, RepositoryType};
use artiferris_domain::permission::Role;
use serde_json::json;

use crate::authz::{member_is_readable, require_npm_format_repository, require_readable_personal_repository_by_name, require_readable_repository_by_name, require_repository_role};
use crate::auth::NpmAuthUser;
use crate::errors::{bad_request, npm_error_response};
use crate::organization_resolution::ResolvedOrganization;
use crate::state::NpmState;

pub fn router() -> Router<NpmState> {
    Router::new()
        .route("/{repository}/{package}", get(get_metadata))
        .route("/{repository}/{package}/-/{filename}", get(get_tarball))
        .route("/u/{username}/{repo}/{package}", get(get_metadata_personal))
        .route("/u/{username}/{repo}/{package}/-/{filename}", get(get_tarball_personal))
}

async fn get_metadata(
    State(state): State<NpmState>,
    Path((repository, package)): Path<(String, String)>,
    headers: HeaderMap,
    resolved_org: ResolvedOrganization,
    user: Option<NpmAuthUser>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let (repo, caller) = require_readable_repository_by_name(&state, user.as_ref(), resolved_org.0.id, &repository).await.map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_npm_format_repository(&repo).map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    if let Some(caller) = caller {
        require_repository_role(&state, caller, repo.id, repo.organization_id, Role::Read).await.map_err(|s| (s, Json(json!({ "error": "forbidden" }))))?;
    }

    let name = NpmPackageName::parse(&package).map_err(|_| bad_request("invalid package name"))?;
    // `user`, not `caller`: `caller` is `None` for a public top-level repository, and an authenticated caller must not
    // lose the members they hold Read on because the wrapping group is public. Docker's handlers do the same.
    let member_caller = user.as_ref();
    let state_ref = &state;
    let document = state
        .metadata
        .execute(repo.id, &name, move |member: &PackageRepositorySummary| {
            member_is_readable(state_ref, member_caller, member.id, member.organization_id, member.is_public)
        })
        .await
        .map_err(npm_error_response)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "package not found" }))))?;

    let base = request_base_url(&state, &headers, &repository)?;
    Ok(Json(rewrite_tarball_urls(document, &base, &package)))
}

async fn get_tarball(
    State(state): State<NpmState>,
    method: Method,
    Path((repository, package, filename)): Path<(String, String, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    resolved_org: ResolvedOrganization,
    user: Option<NpmAuthUser>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let (repo, caller) = require_readable_repository_by_name(&state, user.as_ref(), resolved_org.0.id, &repository).await.map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_npm_format_repository(&repo).map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    if let Some(caller) = caller {
        require_repository_role(&state, caller, repo.id, repo.organization_id, Role::Read).await.map_err(|s| (s, Json(json!({ "error": "forbidden" }))))?;
    }

    let name = NpmPackageName::parse(&package).map_err(|_| bad_request("invalid package name"))?;
    let version = version_from_tarball_filename(&filename, &package).ok_or_else(|| bad_request("invalid tarball filename"))?;

    // `user`, not `caller`: see `get_metadata`.
    let member_caller = user.as_ref();
    let state_ref = &state;
    let stream = state
        .download
        .execute_stream(repo.id, &name, &version, move |member: &PackageRepositorySummary| {
            member_is_readable(state_ref, member_caller, member.id, member.organization_id, member.is_public)
        })
        .await
        .map_err(npm_error_response)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "tarball not found" }))))?;
    count_download(&state, &repo, &method, &name, &headers, &connect_info);

    Ok(([("content-type", "application/octet-stream")], tarball_cache_headers(repo.is_public), Body::from_stream(stream)))
}

async fn get_metadata_personal(
    State(state): State<NpmState>,
    Path((username, repo, package)): Path<(String, String, String)>,
    headers: HeaderMap,
    user: Option<NpmAuthUser>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // The npm-format check runs inside this call, before any role check reaches the database.
    let (repo, _caller) =
        require_readable_personal_repository_by_name(&state, user.as_ref(), &username, &repo).await.map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;

    let name = NpmPackageName::parse(&package).map_err(|_| bad_request("invalid package name"))?;
    // `user`, not `caller`: see `get_metadata`.
    let member_caller = user.as_ref();
    let state_ref = &state;
    let document = state
        .metadata
        .execute(repo.id, &name, move |member: &PackageRepositorySummary| {
            member_is_readable(state_ref, member_caller, member.id, member.organization_id, member.is_public)
        })
        .await
        .map_err(npm_error_response)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "package not found" }))))?;

    let base = request_base_url(&state, &headers, &format!("u/{username}/{repo_name}", repo_name = repo.name))?;
    Ok(Json(rewrite_tarball_urls(document, &base, &package)))
}

async fn get_tarball_personal(
    State(state): State<NpmState>,
    method: Method,
    Path((username, repo, package, filename)): Path<(String, String, String, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    user: Option<NpmAuthUser>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let (repo, _caller) =
        require_readable_personal_repository_by_name(&state, user.as_ref(), &username, &repo).await.map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;

    let name = NpmPackageName::parse(&package).map_err(|_| bad_request("invalid package name"))?;
    let version = version_from_tarball_filename(&filename, &package).ok_or_else(|| bad_request("invalid tarball filename"))?;

    // `user`, not `caller`: see `get_metadata`.
    let member_caller = user.as_ref();
    let state_ref = &state;
    let stream = state
        .download
        .execute_stream(repo.id, &name, &version, move |member: &PackageRepositorySummary| {
            member_is_readable(state_ref, member_caller, member.id, member.organization_id, member.is_public)
        })
        .await
        .map_err(npm_error_response)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "tarball not found" }))))?;
    count_download(&state, &repo, &method, &name, &headers, &connect_info);

    Ok(([("content-type", "application/octet-stream")], tarball_cache_headers(repo.is_public), Body::from_stream(stream)))
}

/// Only a GET served straight from a hosted repository counts, and one client counts once per package per hour.
fn count_download(state: &NpmState, repo: &PackageRepositorySummary, method: &Method, name: &NpmPackageName, headers: &HeaderMap, connect_info: &Result<ConnectInfo<SocketAddr>, ExtensionRejection>) {
    if method == Method::HEAD || repo.repo_type != RepositoryType::Hosted {
        return;
    }
    let forwarded: Vec<&str> = headers.get_all("x-forwarded-for").iter().filter_map(|value| value.to_str().ok()).collect();
    let client = state.guard.client_bucket(connect_info.as_ref().ok().map(|ConnectInfo(addr)| addr.ip()), &forwarded);
    if state.guard.download_dedupe.first_in_hour(&client, repo.id, name.as_str()) {
        state.downloads.record(repo.id, RepositoryFormat::Npm, name.as_str());
    }
}

/// A published tarball never changes, but a shared cache may keep it only for a public repository and only for a day:
/// the repository can turn private.
fn tarball_cache_headers(repository_is_public: bool) -> [(&'static str, &'static str); 2] {
    let cache_control = if repository_is_public { "public, max-age=86400" } else { "private, no-store" };
    [("cache-control", cache_control), ("vary", "Authorization")]
}

/// Filenames are `{unscoped-name}-{version}.tgz`.
pub(crate) fn version_from_tarball_filename(filename: &str, package: &str) -> Option<NpmVersion> {
    let unscoped = unscoped_name(package);
    let stripped = filename.strip_prefix(unscoped)?.strip_prefix('-')?.strip_suffix(".tgz")?;
    NpmVersion::parse(stripped).ok()
}

fn unscoped_name(package: &str) -> &str {
    package.rsplit('/').next().unwrap_or(package)
}

/// The base URL clients fetch tarballs from: this instance's own host (anything else is refused), a numeric port at most,
/// and the scheme from `PUBLIC_URL`.
fn request_base_url(state: &NpmState, headers: &HeaderMap, repository: &str) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let host = headers.get("host").and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    // Anything else is refused, not reflected into a URL the client follows.
    if artiferris_application::base_domain::own_host(host, &state.artiferris_base_domain).is_none() {
        return Err(bad_request("invalid host"));
    }
    Ok(format!("{}://{host}/npm/{repository}", state.public_scheme))
}

fn rewrite_tarball_urls(mut document: serde_json::Value, base: &str, package: &str) -> serde_json::Value {
    let unscoped = unscoped_name(package);
    // `package` is already percent-decoded, so a scoped name has a literal `/` — re-encode it as `%2f` or the advertised tarball URL 404s.
    let encoded_package = package.replace('/', "%2f");
    if let Some(versions) = document.get_mut("versions").and_then(|v| v.as_object_mut()) {
        for (version, manifest) in versions.iter_mut() {
            if let Some(dist) = manifest.get_mut("dist").and_then(|d| d.as_object_mut()) {
                dist.insert(
                    "tarball".to_string(),
                    serde_json::Value::String(format!("{base}/{encoded_package}/-/{unscoped}-{version}.tgz")),
                );
            }
        }
    }
    document
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use bytes::Bytes;
    use artiferris_application::use_cases::api_token::{CreateApiTokenUseCase, ListApiTokensUseCase, RevokeApiTokenUseCase, hash_api_token};
    use artiferris_application::use_cases::npm_audit::BulkAuditNpmPackagesUseCase;
    use artiferris_application::use_cases::npm_dependency_scan::ScanDependencyTreeUseCase;
    use artiferris_application::use_cases::npm_deprecate::DeprecateNpmVersionUseCase;
    use artiferris_application::use_cases::npm_dist_tags::{DeleteDistTagUseCase, ListDistTagsUseCase, SetDistTagUseCase};
    use artiferris_application::use_cases::npm_download::DownloadNpmTarballUseCase;
    use artiferris_application::use_cases::npm_publish::PublishNpmPackageUseCase;
    use artiferris_application::use_cases::npm_search::SearchNpmPackagesUseCase;
    use artiferris_application::use_cases::npm_unpublish::UnpublishNpmPackageUseCase;
    use artiferris_application::use_cases::personal_repository::{CreateUserProjectUseCase, ReservePersonalOrganizationUseCase};
    use artiferris_application::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase;
    use artiferris_domain::organization::{Organization, OrganizationSlug, PUBLIC_ORGANIZATION_ID};
    use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
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
            metadata: Arc::new(artiferris_application::use_cases::npm_metadata::GetNpmPackageMetadataUseCase::new(
                npm_packages.clone(),
                repositories.clone(),
                remote_registry.clone(),
            )),
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

    async fn seed_user_with_active_token(pool: &PgPool, organization_id: Uuid, plaintext_token: &str) -> Uuid {
        let user_id = Uuid::new_v4();
        // Usernames cap at 32 characters: truncate the UUID.
        sqlx::query!(
            "INSERT INTO users (id, username, password_hash, is_super_admin, organization_id, created_at) VALUES ($1, $2, 'irrelevant', false, $3, now())",
            user_id,
            format!("user-{}", &user_id.simple().to_string()[..8]),
            organization_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO api_tokens (id, user_id, token_hash, label, created_at) VALUES ($1, $2, $3, 'test', now())",
            Uuid::new_v4(),
            user_id,
            hash_api_token(plaintext_token),
        )
        .execute(pool)
        .await
        .unwrap();
        user_id
    }

    async fn seed_named_user_with_active_token(pool: &PgPool, organization_id: Uuid, username: &str, plaintext_token: &str) -> Uuid {
        let user_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, password_hash, is_super_admin, organization_id, created_at) VALUES ($1, $2, 'irrelevant', false, $3, now())",
            user_id,
            username,
            organization_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO api_tokens (id, user_id, token_hash, label, created_at) VALUES ($1, $2, $3, 'test', now())",
            Uuid::new_v4(),
            user_id,
            hash_api_token(plaintext_token),
        )
        .execute(pool)
        .await
        .unwrap();
        user_id
    }

    /// Marks a repository public through the event store, as the store's own tests do: `SetRepositoryVisibilityUseCase`
    /// is not wired into `NpmState`.
    async fn mark_repository_public(pool: &PgPool, repository_id: Uuid) {
        use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort};
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let (version, _) = store.load(repository_id).await.unwrap();
        store
            .append(repository_id, version, vec![PackageRepositoryEvent::VisibilityChanged { repository_id, is_public: true }], Uuid::new_v4())
            .await
            .unwrap();
    }

    async fn attach_group_member(pool: &PgPool, group_id: Uuid, member_id: Uuid) {
        use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort};
        let store = PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string());
        let (version, _) = store.load(group_id).await.unwrap();
        store
            .append(
                group_id,
                version,
                vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: group_id, member_repository_id: member_id, position: 0 }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
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

    async fn seed_permission(pool: &PgPool, user_id: Uuid, repository_id: Uuid, role: &str) {
        sqlx::query!(
            "INSERT INTO permission_projections (user_id, repository_id, role, version, updated_at) VALUES ($1, $2, $3, 1, now())",
            user_id,
            repository_id,
            role,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_in_the_requested_organization_is_reachable_via_its_own_subdomain(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: acme_id,
                slug: OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, acme_id, "acme-plaintext-token").await;
        seed_permission(&pool, user_id, repo_id, "read").await;

        let package_name = NpmPackageName::parse("acme-widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "acme-widget", "version": "1.0.0" }), Bytes::from_static(b"acme-tarball-bytes"), &[], user_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-plaintext-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["name"], "acme-widget");
        assert!(document["versions"]["1.0.0"].is_object(), "the published version must appear in the returned metadata");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_package_in_one_organizations_repository_is_not_reachable_from_another_organization(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: acme_id,
                slug: OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let other_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: other_id,
                slug: OrganizationSlug::parse("other").unwrap(),
                display_name: "Other".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        seed_user_with_active_token(&pool, other_id, "plaintext-token").await;
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/repo-{repo_id}/some-package"))
                    .header("host", "other.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer plaintext-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Seeds an explicit grant and targets the repository's own organization, so only the caller-organization check can
    /// reject it.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_valid_permission_grant_does_not_cross_an_organization_boundary(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: acme_id,
                slug: OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let other_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: other_id,
                slug: OrganizationSlug::parse("other").unwrap(),
                display_name: "Other".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");

        let acme_user_id = seed_user_with_active_token(&pool, acme_id, "acme-owns-this-package").await;
        seed_permission(&pool, acme_user_id, repo_id, "read").await;
        let package_name = NpmPackageName::parse("acme-widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "acme-widget", "version": "1.0.0" }), Bytes::from_static(b"cross-org-replay-tarball-bytes"), &[], acme_user_id)
            .await
            .unwrap();

        let other_user_id = seed_user_with_active_token(&pool, other_id, "plaintext-token").await;
        seed_permission(&pool, other_user_id, repo_id, "read").await;
        let app = crate::router(state);

        let acme_get = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-owns-this-package")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(acme_get.status(), StatusCode::OK, "sanity check: the published package must be fetchable by its own organization");

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer plaintext-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a valid role grant on a package that genuinely exists must not be enough to cross an organization boundary"
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn downloading_a_tarball_from_a_public_repository_carries_a_long_lived_cache_control_header(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: acme_id,
                slug: OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        mark_repository_public(&pool, repo_id).await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, acme_id, "acme-plaintext-token").await;
        seed_permission(&pool, user_id, repo_id, "read").await;

        let package_name = NpmPackageName::parse("acme-widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "acme-widget", "version": "1.0.0" }), Bytes::from_static(b"acme-tarball-bytes"), &[], user_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget/-/acme-widget-1.0.0.tgz"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-plaintext-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(axum::http::header::CACHE_CONTROL).unwrap(),
            "public, max-age=86400",
            "a public repository's tarball may be cached for a day, not a year, since the repository can turn private"
        );
    }

    async fn create_personal_project(pool: &PgPool, owner_user_id: Uuid, project_name: &str) -> Uuid {
        create_personal_project_with_format(pool, owner_user_id, project_name, RepositoryFormat::Npm).await
    }

    async fn create_personal_project_with_format(pool: &PgPool, owner_user_id: Uuid, project_name: &str, format: RepositoryFormat) -> Uuid {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));

        ReservePersonalOrganizationUseCase::new(organizations.clone(), users.clone()).execute(owner_user_id).await.unwrap();
        let create_project = CreateUserProjectUseCase::new(organizations, repository_store.clone(), repository_store);
        create_project.execute(owner_user_id, project_name, format, RepositoryType::Hosted).await.unwrap()
    }

    /// A `PermissionQueryPort` that errors on every call, to prove a path never reaches the permissions query.
    struct AlwaysErrorsPermissionQuery;

    #[async_trait::async_trait]
    impl artiferris_domain::permission::PermissionQueryPort for AlwaysErrorsPermissionQuery {
        async fn find_role(&self, _user_id: Uuid, _repository_id: Uuid) -> Result<Option<Role>, artiferris_domain::error::EventStoreError> {
            Err(artiferris_domain::error::EventStoreError::Storage("simulated database outage".to_string()))
        }
        async fn list_for_repository(&self, _repository_id: Uuid) -> Result<Vec<(Uuid, Role)>, artiferris_domain::error::EventStoreError> {
            Err(artiferris_domain::error::EventStoreError::Storage("simulated database outage".to_string()))
        }
        async fn list_for_user(&self, _user_id: Uuid) -> Result<Vec<(Uuid, Role)>, artiferris_domain::error::EventStoreError> {
            Err(artiferris_domain::error::EventStoreError::Storage("simulated database outage".to_string()))
        }
        async fn list_all(&self) -> Result<Vec<(Uuid, Uuid, Role)>, artiferris_domain::error::EventStoreError> {
            Err(artiferris_domain::error::EventStoreError::Storage("simulated database outage".to_string()))
        }
        async fn count_all(&self) -> Result<usize, artiferris_domain::error::EventStoreError> {
            Err(artiferris_domain::error::EventStoreError::Storage("simulated database outage".to_string()))
        }
        async fn count_for_repositories(&self, _repository_ids: &[Uuid]) -> Result<usize, artiferris_domain::error::EventStoreError> {
            Err(artiferris_domain::error::EventStoreError::Storage("simulated database outage".to_string()))
        }
    }

    /// Regression: the npm-format check on the personal route ran after the role check. With a permissions store that
    /// errors on every query, a Docker-format personal repository must still 404, not 500.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_docker_format_personal_repo_404s_via_the_npm_route_even_when_the_permission_store_is_down(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool.clone(), dir.path()).await;

        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        create_personal_project_with_format(&pool, alice_id, "my-image", RepositoryFormat::Docker).await;
        let mallory_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "mallory", "mallory-token").await;
        let _ = mallory_id;

        state.permissions = Arc::new(AlwaysErrorsPermissionQuery);

        let app = crate::router(state);

        let metadata_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/u/alice/my-image/my-image")
                    .header(axum::http::header::AUTHORIZATION, "Bearer mallory-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            metadata_response.status(),
            StatusCode::NOT_FOUND,
            "the format check must reject a Docker-format repository before the role-check ever queries permissions"
        );

        let tarball_response = app
            .oneshot(
                Request::builder()
                    .uri("/u/alice/my-image/my-image/-/my-image-1.0.0.tgz")
                    .header(axum::http::header::AUTHORIZATION, "Bearer mallory-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(tarball_response.status(), StatusCode::NOT_FOUND, "same ordering requirement applies to the tarball route");
    }

    /// Reserves a personal namespace, creates a group and a hosted project in it and attaches the project to the group:
    /// the shape the public API produces and the per-member policy once rejected.
    async fn create_personal_group_over_a_personal_member(pool: &PgPool, owner_user_id: Uuid, group_name: &str, member_name: &str) -> (Uuid, Uuid) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));

        ReservePersonalOrganizationUseCase::new(organizations.clone(), users.clone()).execute(owner_user_id).await.unwrap();
        let create_project = CreateUserProjectUseCase::new(organizations, repository_store.clone(), repository_store);
        let group_id = create_project.execute(owner_user_id, group_name, RepositoryFormat::Npm, RepositoryType::Group).await.unwrap();
        let member_id = create_project.execute(owner_user_id, member_name, RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap();
        attach_group_member(pool, group_id, member_id).await;
        (group_id, member_id)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn fetches_metadata_for_a_personal_project(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;

        let package_name = NpmPackageName::parse("my-lib").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"personal-tarball-bytes"), &[], alice_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/u/alice/my-lib/my-lib")
                    .header("host", "artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer alice-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["name"], "my-lib");
        assert!(document["versions"]["1.0.0"].is_object(), "the published version must appear in the returned metadata");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_user_cannot_fetch_metadata_from_someone_elses_private_project(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;

        let package_name = NpmPackageName::parse("my-lib").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"personal-tarball-bytes"), &[], alice_id)
            .await
            .unwrap();

        let other_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: other_id,
                slug: OrganizationSlug::parse("other-corp").unwrap(),
                display_name: "Other Corp".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        seed_named_user_with_active_token(&pool, other_id, "mallory", "mallory-token").await;

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/u/alice/my-lib/my-lib")
                    .header(axum::http::header::AUTHORIZATION, "Bearer mallory-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn downloads_a_tarball_for_a_personal_project(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;

        let package_name = NpmPackageName::parse("my-lib").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"personal-tarball-bytes"), &[], alice_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/u/alice/my-lib/my-lib/-/my-lib-1.0.0.tgz")
                    .header(axum::http::header::AUTHORIZATION, "Bearer alice-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, Bytes::from_static(b"personal-tarball-bytes"));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_fetch_metadata_from_a_public_organization_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: acme_id,
                slug: OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        mark_repository_public(&pool, repo_id).await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, acme_id, "acme-plaintext-token").await;
        seed_permission(&pool, user_id, repo_id, "read").await;

        let package_name = NpmPackageName::parse("acme-widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "acme-widget", "version": "1.0.0" }), Bytes::from_static(b"acme-tarball-bytes"), &[], user_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget"))
                    .header("host", "acme.artiferris.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["name"], "acme-widget");
        assert!(document["versions"]["1.0.0"].is_object(), "the published version must appear in the returned metadata");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_fetch_metadata_from_a_private_organization_repository(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        state
            .organizations
            .create(&Organization {
                id: acme_id,
                slug: OrganizationSlug::parse("acme").unwrap(),
                display_name: "Acme".to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, acme_id, "acme-plaintext-token").await;
        seed_permission(&pool, user_id, repo_id, "read").await;

        let package_name = NpmPackageName::parse("acme-widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "acme-widget", "version": "1.0.0" }), Bytes::from_static(b"acme-tarball-bytes"), &[], user_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget"))
                    .header("host", "acme.artiferris.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_fetch_metadata_and_a_tarball_from_a_public_personal_project(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;
        mark_repository_public(&pool, repo_id).await;

        let package_name = NpmPackageName::parse("my-lib").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"personal-tarball-bytes"), &[], alice_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let metadata_response = app
            .clone()
            .oneshot(Request::builder().uri("/u/alice/my-lib/my-lib").header("host", "artiferris.localhost").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(metadata_response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(metadata_response.into_body(), usize::MAX).await.unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["name"], "my-lib");

        let tarball_response = app
            .oneshot(Request::builder().uri("/u/alice/my-lib/my-lib/-/my-lib-1.0.0.tgz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(tarball_response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(tarball_response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, Bytes::from_static(b"personal-tarball-bytes"));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_fetch_from_a_private_personal_project(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;

        let package_name = NpmPackageName::parse("my-lib").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"personal-tarball-bytes"), &[], alice_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(Request::builder().uri("/u/alice/my-lib/my-lib").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }


    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_spoofed_host_header_is_rejected_rather_than_reflected_into_the_tarball_url(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        // `ResolvedOrganization` falls back to the public organization for a Host outside this instance's base domain,
        // so the repository must live there for `request_base_url`'s own rejection to be exercised.
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repo_id, "npm", "hosted").await;
        mark_repository_public(&pool, repo_id).await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "acme-plaintext-token").await;
        seed_permission(&pool, user_id, repo_id, "read").await;

        let package_name = NpmPackageName::parse("acme-widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "acme-widget", "version": "1.0.0" }), Bytes::from_static(b"acme-tarball-bytes"), &[], user_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget"))
                    .header("host", "evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "a Host that isn't this instance's own base domain (or a subdomain of it) must be rejected, not reflected into a client-visible URL"
        );
    }

    /// A real personal repository is resolved from the path alone, independent of `Host`, so a spoofed Host must not
    /// redirect the generated tarball URL to another domain.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_spoofed_host_header_on_the_personal_route_is_rejected_rather_than_reflected_into_the_tarball_url(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;
        mark_repository_public(&pool, repo_id).await;

        let package_name = NpmPackageName::parse("my-lib").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"personal-tarball-bytes"), &[], alice_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let spoofed = app
            .clone()
            .oneshot(Request::builder().uri("/u/alice/my-lib/my-lib").header("host", "evil.example").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            spoofed.status(),
            StatusCode::BAD_REQUEST,
            "a spoofed Host on the personal-namespace route must be rejected rather than reflected into the tarball URL, even though this route resolves its repository purely from the path"
        );

        let legitimate = app
            .oneshot(Request::builder().uri("/u/alice/my-lib/my-lib").header("host", "artiferris.localhost").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(legitimate.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(legitimate.into_body(), usize::MAX).await.unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let tarball = document["versions"]["1.0.0"]["dist"]["tarball"].as_str().unwrap();
        assert!(tarball.starts_with("http://artiferris.localhost/npm/u/alice/my-lib/"), "got: {tarball}");
    }

    /// Regression: `ResolvedOrganization` lowercases the Host, but `request_base_url` compared the raw one and rejected
    /// with 400 a request the organization lookup had accepted.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_uppercase_host_header_still_succeeds_against_the_organization_route(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let acme_id = Uuid::new_v4();
        create_org(&state, acme_id, "acme").await;

        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, acme_id, "acme-plaintext-token").await;
        seed_permission(&pool, user_id, repo_id, "read").await;

        let package_name = NpmPackageName::parse("acme-widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "acme-widget", "version": "1.0.0" }), Bytes::from_static(b"acme-tarball-bytes"), &[], user_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget"))
                    .header("host", "ACME.ARTIFERRIS.LOCALHOST")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-plaintext-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "an uppercase Host resolves the organization fine via ResolvedOrganization (which lowercases), so request_base_url's own domain check must lowercase too rather than wrongly reject it"
        );
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let tarball = document["versions"]["1.0.0"]["dist"]["tarball"].as_str().unwrap();
        assert!(tarball.contains("ACME.ARTIFERRIS.LOCALHOST"), "the tarball URL must still be built from the (uppercase) Host that was accepted: got {tarball}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_tarball_url_uses_the_scheme_the_instance_is_served_under(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool.clone(), dir.path()).await;
        state.public_scheme = "https".to_string();

        let acme_id = Uuid::new_v4();
        create_org(&state, acme_id, "acme").await;

        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        mark_repository_public(&pool, repo_id).await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, acme_id, "acme-plaintext-token").await;
        seed_permission(&pool, user_id, repo_id, "read").await;

        let package_name = NpmPackageName::parse("acme-widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(repo_id, &package_name, &version, json!({ "name": "acme-widget", "version": "1.0.0" }), Bytes::from_static(b"acme-tarball-bytes"), &[], user_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/acme-widget"))
                    .header("host", "acme.artiferris.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tarball = json["versions"]["1.0.0"]["dist"]["tarball"].as_str().unwrap();
        assert!(tarball.starts_with("https://"), "got: {tarball}");
    }

    /// Suffix matching in isolation: each of these hosts resembles `artiferris.localhost` but is neither it nor a
    /// subdomain. Only the `.ends_with(".{base}")` check, with its leading dot, stands between a spoofed Host and a
    /// client-visible tarball URL.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn request_base_url_rejects_hosts_that_only_superficially_resemble_the_base_domain(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;

        let reject = |host: &str| {
            let mut headers = HeaderMap::new();
            headers.insert("host", host.parse().unwrap());
            headers
        };

        assert!(request_base_url(&state, &reject("notartiferris.localhost"), "some-repo").is_err(), "a host that merely ends with the base domain's characters, without a leading dot, must not match");
        assert!(request_base_url(&state, &reject("evil-artiferris.localhost"), "some-repo").is_err(), "a hyphenated sibling label must not match either");
        assert!(request_base_url(&state, &reject("artiferris.localhost.evil.example"), "some-repo").is_err(), "the base domain appearing as a prefix must not satisfy the suffix check");
        assert!(request_base_url(&state, &reject("evil.example"), "some-repo").is_err());
        for hostile in ["evil.example/x.artiferris.localhost", "evil.example#.artiferris.localhost", "evil.example?.artiferris.localhost", "a@evil.example/.artiferris.localhost", "evil\\.artiferris.localhost", "evil .artiferris.localhost"] {
            assert!(request_base_url(&state, &reject(hostile), "some-repo").is_err(), "{hostile:?}");
        }

        assert!(request_base_url(&state, &reject("artiferris.localhost"), "some-repo").is_ok(), "the base domain itself must be accepted");
        assert!(request_base_url(&state, &reject("acme.artiferris.localhost"), "some-repo").is_ok(), "a genuine subdomain must be accepted");
    }


    fn acme_get(uri: String, token: Option<&str>) -> Request<Body> {
        let builder = Request::builder().method("GET").uri(uri).header("host", "acme.artiferris.localhost");
        match token {
            Some(token) => builder.header(axum::http::header::AUTHORIZATION, format!("Bearer {token}")),
            None => builder,
        }
        .body(Body::empty())
        .unwrap()
    }

    /// A private npm group whose only member is a private hosted repository holding one published package. Returns
    /// `(group id, group name, member id, member name)`; `owner-token` holds Read on the member and nothing on the
    /// group.
    async fn seed_group_over_a_private_member(state: &NpmState, pool: &PgPool, organization_id: Uuid) -> (Uuid, String, Uuid, String) {
        let group_id = Uuid::new_v4();
        seed_repository(pool, organization_id, group_id, "npm", "group").await;
        let member_id = Uuid::new_v4();
        seed_repository(pool, organization_id, member_id, "npm", "hosted").await;
        attach_group_member(pool, group_id, member_id).await;

        let owner_id = seed_user_with_active_token(pool, organization_id, "owner-token").await;
        seed_permission(pool, owner_id, member_id, "read").await;
        state
            .publish
            .execute(
                member_id,
                &NpmPackageName::parse("secret-widget").unwrap(),
                &NpmVersion::parse("1.0.0").unwrap(),
                json!({ "name": "secret-widget", "version": "1.0.0" }),
                Bytes::from_static(b"secret-widget-tarball-bytes"),
                &[],
                owner_id,
            )
            .await
            .unwrap();

        (group_id, format!("repo-{group_id}"), member_id, format!("repo-{member_id}"))
    }

    /// Reading through a group is never broader than reading the member directly: a caller with Read on the group and
    /// nothing on the private member gets neither its metadata nor its tarball.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_group_member_is_unreachable_through_a_group_the_caller_can_read(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let acme_id = Uuid::new_v4();
        create_org(&state, acme_id, "acme").await;
        let (group_id, group_name, _member_id, member_name) = seed_group_over_a_private_member(&state, &pool, acme_id).await;

        let reader_id = seed_user_with_active_token(&pool, acme_id, "reader-token").await;
        seed_permission(&pool, reader_id, group_id, "read").await;

        let app = crate::router(state);

        let direct = app.clone().oneshot(acme_get(format!("/{member_name}/secret-widget"), Some("owner-token"))).await.unwrap();
        assert_eq!(direct.status(), StatusCode::OK, "sanity check: the member's package must be readable directly by a caller holding Read on the member");

        let metadata = app.clone().oneshot(acme_get(format!("/{group_name}/secret-widget"), Some("reader-token"))).await.unwrap();
        assert_eq!(metadata.status(), StatusCode::NOT_FOUND, "Read on the group must not leak into a member the caller holds nothing on");

        let tarball = app
            .oneshot(acme_get(format!("/{group_name}/secret-widget/-/secret-widget-1.0.0.tgz"), Some("reader-token")))
            .await
            .unwrap();
        assert_eq!(tarball.status(), StatusCode::NOT_FOUND, "the tarball route must apply the same per-member policy as the metadata route");
    }

    /// The other half: a caller with Read on the private member still reaches it through the group, so "fail closed" is
    /// not "traversal is broken".
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_group_member_the_caller_holds_read_on_is_still_reachable_through_the_group(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let acme_id = Uuid::new_v4();
        create_org(&state, acme_id, "acme").await;
        let (group_id, group_name, member_id, _member_name) = seed_group_over_a_private_member(&state, &pool, acme_id).await;

        let reader_id = seed_user_with_active_token(&pool, acme_id, "reader-token").await;
        seed_permission(&pool, reader_id, group_id, "read").await;
        seed_permission(&pool, reader_id, member_id, "read").await;

        let app = crate::router(state);

        let metadata = app.clone().oneshot(acme_get(format!("/{group_name}/secret-widget"), Some("reader-token"))).await.unwrap();
        assert_eq!(metadata.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(metadata.into_body(), usize::MAX).await.unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(document["versions"]["1.0.0"].is_object(), "the member's published version must resolve through the group");

        let tarball = app
            .oneshot(acme_get(format!("/{group_name}/secret-widget/-/secret-widget-1.0.0.tgz"), Some("reader-token")))
            .await
            .unwrap();
        assert_eq!(tarball.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(tarball.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, Bytes::from_static(b"secret-widget-tarball-bytes"));
    }

    /// Regression: for a public top-level repository `require_readable_repository_by_name` returns `None` as caller,
    /// authenticated or not, so building the member policy from it denied an authenticated caller their private
    /// members. The policy runs as the real requester. The fixture also pins the organization guard: a public top level
    /// skips the top-level check and the member lookup ignores organizations, so a stale cross-organization grant must
    /// not work.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_authenticated_callers_own_member_grant_survives_a_public_top_level_group(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let acme_id = Uuid::new_v4();
        create_org(&state, acme_id, "acme").await;
        let other_id = Uuid::new_v4();
        create_org(&state, other_id, "other").await;
        let (group_id, group_name, member_id, _member_name) = seed_group_over_a_private_member(&state, &pool, acme_id).await;
        mark_repository_public(&pool, group_id).await;

        let reader_id = seed_user_with_active_token(&pool, acme_id, "reader-token").await;
        seed_permission(&pool, reader_id, member_id, "read").await;
        let outsider_id = seed_user_with_active_token(&pool, other_id, "outsider-token").await;
        seed_permission(&pool, outsider_id, member_id, "read").await;

        let app = crate::router(state);

        let legitimate = app.clone().oneshot(acme_get(format!("/{group_name}/secret-widget"), Some("reader-token"))).await.unwrap();
        assert_eq!(
            legitimate.status(),
            StatusCode::OK,
            "the per-member check must run as the authenticated requester, not as the `None` a public top-level repository yields"
        );

        let anonymous = app.clone().oneshot(acme_get(format!("/{group_name}/secret-widget"), None)).await.unwrap();
        assert_eq!(anonymous.status(), StatusCode::NOT_FOUND, "a public group must not hand a private member to an anonymous caller");

        let cross_org = app.oneshot(acme_get(format!("/{group_name}/secret-widget"), Some("outsider-token"))).await.unwrap();
        assert_eq!(cross_org.status(), StatusCode::NOT_FOUND, "a stale cross-organization grant on the member must not become usable through a public group");
    }

    /// Regression: a personal organization's id never equals a real user's `organization_id`, so requiring a match
    /// locked alice out of her own group's members. The policy falls to the explicit-grant lookup for personal
    /// organizations.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_personal_group_members_own_owner_can_read_it_through_the_group(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;

        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let (_group_id, member_id) = create_personal_group_over_a_personal_member(&pool, alice_id, "my-group", "my-lib").await;

        let package_name = NpmPackageName::parse("my-lib").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .publish
            .execute(member_id, &package_name, &version, json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"personal-group-tarball-bytes"), &[], alice_id)
            .await
            .unwrap();

        let app = crate::router(state);

        let metadata = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/u/alice/my-group/my-lib")
                    .header("host", "artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer alice-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(metadata.status(), StatusCode::OK, "a user must be able to read their own personal hosted project through their own personal group project");
        let bytes = axum::body::to_bytes(metadata.into_body(), usize::MAX).await.unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(document["versions"]["1.0.0"].is_object(), "the member's published version must resolve through the personal group");

        let tarball = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/u/alice/my-group/my-lib/-/my-lib-1.0.0.tgz")
                    .header(axum::http::header::AUTHORIZATION, "Bearer alice-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(tarball.status(), StatusCode::OK, "the tarball route must apply the same per-member policy as the metadata route");
        let bytes = axum::body::to_bytes(tarball.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, Bytes::from_static(b"personal-group-tarball-bytes"));

        seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "mallory", "mallory-token").await;
        let stranger = app
            .clone()
            .oneshot(
                Request::builder().uri("/u/alice/my-group/my-lib").header(axum::http::header::AUTHORIZATION, "Bearer mallory-token").body(Body::empty()).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stranger.status(), StatusCode::NOT_FOUND, "a personal organization's members are still gated on a real grant, not merely on being authenticated");

        let anonymous = app.oneshot(Request::builder().uri("/u/alice/my-group/my-lib").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(anonymous.status(), StatusCode::NOT_FOUND, "an anonymous caller must not reach a private personal member through the group either");
    }

    async fn counted_downloads(pool: PgPool, method: &str, filename: &str) -> Vec<artiferris_domain::download_stats::DownloadCount> {
        let dir = tempfile::tempdir().unwrap();
        let buffer = Arc::new(artiferris_application::download_counter::DownloadCounterBuffer::new());
        let state = NpmState { downloads: buffer.clone(), ..test_state(pool.clone(), dir.path()).await };
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;
        let package_name = NpmPackageName::parse("my-lib").unwrap();
        state.publish.execute(repo_id, &package_name, &NpmVersion::parse("1.0.0").unwrap(), json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"bytes"), &[], alice_id).await.unwrap();
        let app = crate::router(state);

        let response = app
            .oneshot(Request::builder().method(method).uri(format!("/u/alice/my-lib/my-lib/-/{filename}")).header(axum::http::header::AUTHORIZATION, "Bearer alice-token").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        buffer.drain()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_tarball_download_from_a_hosted_repository_is_counted_once(pool: PgPool) {
        let counted = counted_downloads(pool, "GET", "my-lib-1.0.0.tgz").await;

        assert_eq!(counted.len(), 1);
        assert_eq!((counted[0].name.as_str(), counted[0].format, counted[0].downloads), ("my-lib", artiferris_domain::package_repository::RepositoryFormat::Npm, 1));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_head_probe_of_a_tarball_is_not_a_download(pool: PgPool) {
        assert!(counted_downloads(pool, "HEAD", "my-lib-1.0.0.tgz").await.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_tarball_that_does_not_exist_is_not_counted(pool: PgPool) {
        assert!(counted_downloads(pool, "GET", "my-lib-9.9.9.tgz").await.is_empty());
    }

    /// Downloads `my-lib-1.0.0.tgz` once per `(peer, x-forwarded-for)` and returns how many were counted.
    async fn counted_downloads_from(pool: PgPool, trusted: &[&str], clients: &[([u8; 4], Option<&str>)]) -> i64 {
        let dir = tempfile::tempdir().unwrap();
        let buffer = Arc::new(artiferris_application::download_counter::DownloadCounterBuffer::new());
        let guard = artiferris_application::request_guard::RequestGuard::new(Arc::new(artiferris_application::client_ip::TrustedProxies::parse(trusted.iter().copied()).unwrap()));
        let state = NpmState { downloads: buffer.clone(), guard: Arc::new(guard), ..test_state(pool.clone(), dir.path()).await };
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;
        let package_name = NpmPackageName::parse("my-lib").unwrap();
        state.publish.execute(repo_id, &package_name, &NpmVersion::parse("1.0.0").unwrap(), json!({ "name": "my-lib", "version": "1.0.0" }), Bytes::from_static(b"bytes"), &[], alice_id).await.unwrap();
        let app = crate::router(state);

        for (peer, forwarded_for) in clients {
            let mut request = Request::builder().method("GET").uri("/u/alice/my-lib/my-lib/-/my-lib-1.0.0.tgz").header(axum::http::header::AUTHORIZATION, "Bearer alice-token").body(Body::empty()).unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((*peer, 4000))));
            if let Some(forwarded_for) = forwarded_for {
                request.headers_mut().insert("x-forwarded-for", forwarded_for.parse().unwrap());
            }
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        }
        buffer.drain().iter().map(|count| count.downloads).sum()
    }

    /// Hammering one tarball from one address must not move the popularity figures.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn downloading_the_same_tarball_over_and_over_from_one_client_counts_once(pool: PgPool) {
        assert_eq!(counted_downloads_from(pool, &[], &[([203, 0, 113, 7], None); 5]).await, 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn different_clients_each_count_once(pool: PgPool) {
        assert_eq!(counted_downloads_from(pool, &[], &[([203, 0, 113, 7], None), ([203, 0, 113, 8], None), ([203, 0, 113, 7], None)]).await, 2);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn clients_behind_a_trusted_proxy_are_counted_by_their_forwarded_address(pool: PgPool) {
        let counted = counted_downloads_from(
            pool,
            &["10.0.0.0/8"],
            &[([10, 0, 0, 5], Some("198.51.100.1")), ([10, 0, 0, 5], Some("198.51.100.2")), ([10, 0, 0, 5], Some("198.51.100.1"))],
        )
        .await;

        assert_eq!(counted, 2);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_repositorys_tarball_is_not_cacheable_by_a_shared_cache(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        let repo_id = create_personal_project(&pool, alice_id, "my-lib").await;
        let name = NpmPackageName::parse("my-lib").unwrap();
        state.publish.execute(repo_id, &name, &NpmVersion::parse("1.0.0").unwrap(), json!({}), Bytes::from_static(b"bytes"), &[], alice_id).await.unwrap();
        let app = crate::router(state);

        let response = app
            .oneshot(Request::builder().uri("/u/alice/my-lib/my-lib/-/my-lib-1.0.0.tgz").header(axum::http::header::AUTHORIZATION, "Bearer alice-token").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(axum::http::header::CACHE_CONTROL).unwrap(), "private, no-store");
        assert_eq!(response.headers().get(axum::http::header::VARY).unwrap(), "Authorization");
    }

    async fn base_url_for(state: NpmState, host: &str, forwarded_proto: Option<&str>) -> Result<String, StatusCode> {
        let mut headers = HeaderMap::new();
        headers.insert("host", host.parse().unwrap());
        if let Some(proto) = forwarded_proto {
            headers.insert("x-forwarded-proto", proto.parse().unwrap());
        }
        request_base_url(&state, &headers, "repo").map_err(|(status, _)| status)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_tarball_base_url_ignores_x_forwarded_proto_and_only_takes_a_numeric_port(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;

        assert_eq!(base_url_for(state.clone(), "acme.artiferris.localhost", Some("https")).await.unwrap(), "http://acme.artiferris.localhost/npm/repo");
        assert_eq!(base_url_for(state.clone(), "acme.artiferris.localhost:8080", None).await.unwrap(), "http://acme.artiferris.localhost:8080/npm/repo");
        for hostile in ["artiferris.localhost:x@evil.example", "artiferris.localhost:80@evil.example", "acme.artiferris.localhost:", "acme.artiferris.localhost:80/x", "evil.example"] {
            assert_eq!(base_url_for(state.clone(), hostile, None).await, Err(StatusCode::BAD_REQUEST), "{hostile}");
        }
    }
}
