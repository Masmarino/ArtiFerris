use std::collections::HashMap;

use axum::body::Body;
use std::net::SocketAddr;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::put;
use axum::{Json, Router};
use base64::Engine;
use artiferris_domain::npm_package::{NpmPackageName, NpmVersion};
use artiferris_domain::permission::Role;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::authz::{require_hosted, require_npm_format_repository, require_personal_repository_role, require_repository_by_name, require_repository_role, resolve_personal_repository};
use crate::auth::NpmAuthUser;
use crate::body::{BodyCaller, read_json};
use crate::errors::{bad_request, npm_error_response};
use crate::organization_resolution::ResolvedOrganization;
use crate::state::NpmState;

/// The tarball travels base64-encoded inside the JSON document.
pub(crate) const PUBLISH_BODY_LIMIT_BYTES: usize = 200 * 1024 * 1024;

/// The body, the parsed document and the decoded tarball are held at once.
const PUBLISH_MEMORY_FACTOR: usize = 3;

/// Most versions one `npm deprecate` request may touch.
const MAX_DEPRECATED_VERSIONS_PER_REQUEST: usize = 1000;

pub fn router() -> Router<NpmState> {
    Router::new()
        .route("/{repository}/{package}", put(publish))
        .route("/u/{username}/{repo}/{package}", put(publish_personal))
}

#[derive(Deserialize)]
struct PublishDocument {
    name: Option<String>,
    #[serde(rename = "_id")]
    id: Option<String>,
    #[serde(default)]
    versions: HashMap<String, serde_json::Value>,
    #[serde(rename = "_attachments", default)]
    attachments: HashMap<String, Attachment>,
    #[serde(rename = "dist-tags", default)]
    dist_tags: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Attachment {
    data: String,
}

async fn publish(
    State(state): State<NpmState>,
    Path((repository, package)): Path<(String, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    resolved_org: ResolvedOrganization,
    user: NpmAuthUser,
    body: Body,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let repo = require_repository_by_name(&state, &user, resolved_org.0.id, &repository).await.map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_npm_format_repository(&repo).map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    // Role before type (B-14): otherwise an unauthorized caller could tell a proxy repository
    // (405 from require_hosted) apart from a hosted one (403 from the role check below) without
    // ever having read access to either.
    require_repository_role(&state, &user, repo.id, repo.organization_id, Role::Write)
        .await
        .map_err(|s| (s, Json(json!({ "error": "insufficient permissions to publish" }))))?;
    require_hosted(&repo).map_err(|s| (s, Json(json!({ "error": "publish is not supported on a proxy or group repository" }))))?;

    let name = NpmPackageName::parse(&package).map_err(|_| bad_request("invalid package name"))?;
    let caller = BodyCaller::new(&state, user.id, &headers, &connect_info);
    let (doc, _budget) = read_json::<PublishDocument>(&state, &headers, &caller, body, PUBLISH_BODY_LIMIT_BYTES, PUBLISH_MEMORY_FACTOR).await?;
    publish_to_repository(&state, repo.id, &name, doc, user.id).await
}

async fn publish_personal(
    State(state): State<NpmState>,
    Path((username, repo, package)): Path<(String, String, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    user: NpmAuthUser,
    body: Body,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let repo = resolve_personal_repository(&state, &username, &repo).await.map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_npm_format_repository(&repo).map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_personal_repository_role(&state, &user, repo.id, repo.organization_id, Role::Write)
        .await
        .map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_hosted(&repo).map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;

    let name = NpmPackageName::parse(&package).map_err(|_| bad_request("invalid package name"))?;
    let caller = BodyCaller::new(&state, user.id, &headers, &connect_info);
    let (doc, _budget) = read_json::<PublishDocument>(&state, &headers, &caller, body, PUBLISH_BODY_LIMIT_BYTES, PUBLISH_MEMORY_FACTOR).await?;
    publish_to_repository(&state, repo.id, &name, doc, user.id).await
}

/// Shared by `publish` and `publish_personal` — everything after the two routes' differing
/// resolution/authorization preludes is identical, so it lives here once rather than twice.
async fn publish_to_repository(
    state: &NpmState,
    repository_id: Uuid,
    name: &NpmPackageName,
    doc: PublishDocument,
    user_id: Uuid,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    for claimed in [doc.name.as_deref(), doc.id.as_deref()].into_iter().flatten() {
        if claimed != name.as_str() {
            return Err(bad_request("the package name in the document does not match the URL"));
        }
    }

    if doc.attachments.is_empty() {
        // No tarball attached — a metadata-only update (`npm deprecate`), not a new publish.
        if doc.versions.len() > MAX_DEPRECATED_VERSIONS_PER_REQUEST {
            return Err(bad_request("too many versions in one deprecation request"));
        }
        for (version_str, manifest) in &doc.versions {
            let Ok(version) = NpmVersion::parse(version_str) else { continue };
            let message = manifest.get("deprecated").and_then(|d| d.as_str());
            if message.is_some() {
                state
                    .deprecate
                    .execute(repository_id, name, &version, message, user_id)
                    .await
                    .map_err(npm_error_response)?;
            }
        }
        return Ok((StatusCode::OK, Json(json!({ "ok": true, "id": name.as_str(), "rev": Uuid::new_v4().to_string() }))));
    }

    // A plain `npm publish` carries one version and its one tarball, and they must agree.
    let mut attachments = doc.attachments.into_iter();
    let (attachment_name, attachment) = match (attachments.next(), attachments.next()) {
        (Some(only), None) => only,
        _ => return Err(bad_request("a publish document must carry exactly one tarball")),
    };
    let mut versions = doc.versions.into_iter();
    let (version_str, manifest) = match (versions.next(), versions.next()) {
        (Some(only), None) => only,
        _ => return Err(bad_request("a publish document must carry exactly one version")),
    };
    let version = NpmVersion::parse(&version_str).map_err(|_| bad_request("invalid version string"))?;
    if !manifest.is_object() {
        return Err(bad_request("the version manifest must be an object"));
    }
    if manifest.get("name").is_some_and(|claimed| claimed.as_str() != Some(name.as_str())) {
        return Err(bad_request("the package name in the manifest does not match the URL"));
    }
    if manifest.get("version").is_some_and(|claimed| claimed.as_str() != Some(version_str.as_str())) {
        return Err(bad_request("the version in the manifest does not match its key"));
    }
    let expected_names = [format!("{}-{version_str}.tgz", name.as_str()), format!("{}-{version_str}.tgz", name.local_name())];
    if !expected_names.contains(&attachment_name) {
        return Err(bad_request("the tarball's file name does not match the package and version"));
    }
    // The tags the publisher asked for (`--tag`) are the ones pointing at this version.
    let mut dist_tags: Vec<String> = doc.dist_tags.iter().filter(|(_, tagged)| tagged.as_str() == version_str).map(|(tag, _)| tag.clone()).collect();
    if !doc.dist_tags.is_empty() && dist_tags.is_empty() {
        return Err(bad_request("none of the dist-tags points at the published version"));
    }
    dist_tags.sort();

    let tarball_bytes = base64::engine::general_purpose::STANDARD
        .decode(&attachment.data)
        .map_err(|_| bad_request("invalid base64 tarball data"))?;

    state.publish.execute(repository_id, name, &version, manifest, bytes::Bytes::from(tarball_bytes), &dist_tags, user_id).await.map_err(npm_error_response)?;

    // Fire-and-forget. Errors are swallowed and a busy scanner skips it; the manual rescan is the way to recover.
    let scan = state.scan_dependency_tree.clone();
    let scan_repo_id = repository_id;
    let scan_name = name.clone();
    let scan_version = version.clone();
    tokio::spawn(async move {
        let _ = scan.execute_unless_busy(scan_repo_id, &scan_name, &scan_version).await;
    });

    Ok((StatusCode::CREATED, Json(json!({ "ok": true, "id": name.as_str(), "rev": Uuid::new_v4().to_string() }))))
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
    use artiferris_application::use_cases::npm_metadata::GetNpmPackageMetadataUseCase;
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

    /// Mirrors `routes/metadata.rs`'s `test_state` — no shared test-support module for the HTTP-router `NpmState` builder.
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
            .create(&Organization { id, slug: OrganizationSlug::parse(slug).unwrap(), display_name: slug.to_string(), is_public: false, is_personal: false, created_at: chrono::Utc::now() })
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

    async fn seed_user_with_active_token(pool: &PgPool, organization_id: Uuid, plaintext_token: &str) -> Uuid {
        let user_id = Uuid::new_v4();
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

    async fn seed_organization_admin_with_active_token(pool: &PgPool, organization_id: Uuid, plaintext_token: &str) -> Uuid {
        let user_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at) VALUES ($1, $2, 'irrelevant', false, true, $3, now())",
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

    /// Same as `seed_user_with_active_token`, but with a caller-chosen username — needed to hit
    /// `/u/{username}/...` routes. Mirrors `routes/metadata.rs`'s identical helper.
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

    /// Creates `owner_user_id`'s personal organization and a hosted npm project inside it,
    /// returning the new repository's id. Mirrors `routes/metadata.rs`'s identical helper.
    async fn create_personal_project(pool: &PgPool, owner_user_id: Uuid, project_name: &str) -> Uuid {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let users = Arc::new(PostgresUserRepository::new(pool.clone()));
        let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));

        ReservePersonalOrganizationUseCase::new(organizations.clone(), users.clone()).execute(owner_user_id).await.unwrap();
        let create_project = CreateUserProjectUseCase::new(organizations, repository_store.clone(), repository_store);
        create_project.execute(owner_user_id, project_name, RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap()
    }

    fn publish_body(version: &str, attachment_name: Option<&str>, tarball_base64: Option<&str>, extra_manifest_fields: serde_json::Value) -> serde_json::Value {
        let mut manifest = json!({ "name": "widget", "version": version });
        if let (serde_json::Value::Object(base), serde_json::Value::Object(extra)) = (&mut manifest, &extra_manifest_fields) {
            for (k, v) in extra {
                base.insert(k.clone(), v.clone());
            }
        }
        let mut doc = json!({ "versions": { version: manifest } });
        if let (Some(name), Some(data)) = (attachment_name, tarball_base64) {
            doc["_attachments"] = json!({ name: { "data": data } });
        }
        doc
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_valid_publish_with_a_tarball_succeeds(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, org_id, "acme-token").await;
        seed_permission(&pool, user_id, repo_id, "write").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("1.0.0", Some("widget-1.0.0.tgz"), Some(&tarball_b64), json!({}));

        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED, "a genuine new-version publish with a tarball must be treated as a real publish (201), not a metadata-only update");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["id"], "widget");
    }

    async fn put_deprecation(pool: &PgPool, state: &NpmState, doc: serde_json::Value) -> StatusCode {
        let org_id = Uuid::new_v4();
        create_org(state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(pool, org_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(pool, org_id, "acme-token").await;
        seed_permission(pool, user_id, repo_id, "write").await;
        let name = NpmPackageName::parse("widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state.publish.execute(repo_id, &name, &version, json!({ "name": "widget", "version": "1.0.0" }), Bytes::from_static(b"real-tarball"), &[], user_id).await.unwrap();

        crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(doc.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_deprecation_with_an_oversized_message_is_refused(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let doc = json!({ "versions": { "1.0.0": { "deprecated": "x".repeat(2 * 1024 * 1024) } } });

        assert_eq!(put_deprecation(&pool, &state, doc).await, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_deprecation_touching_too_many_versions_is_refused(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let versions: serde_json::Map<String, serde_json::Value> = (0..=MAX_DEPRECATED_VERSIONS_PER_REQUEST).map(|i| (format!("1.0.{i}"), json!({ "deprecated": "old" }))).collect();

        assert_eq!(put_deprecation(&pool, &state, json!({ "versions": versions })).await, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_publish_document_with_no_attachments_only_deprecates_and_does_not_create_a_version(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, org_id, "acme-token").await;
        seed_permission(&pool, user_id, repo_id, "write").await;

        let name = NpmPackageName::parse("widget").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state.publish.execute(repo_id, &name, &version, json!({ "name": "widget", "version": "1.0.0" }), Bytes::from_static(b"real-tarball"), &[], user_id).await.unwrap();

        // Real `npm deprecate` PUTs the packument back with a `deprecated` message and NO `_attachments` key.
        let doc = json!({ "versions": { "1.0.0": { "name": "widget", "version": "1.0.0", "deprecated": "use widget2 instead" } } });

        let app = crate::router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(doc.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "a no-attachment publish document is a metadata-only update (deprecate), must be 200 not 201");

        let document = state.metadata.execute_hosted(repo_id, &name).await.unwrap().unwrap();
        assert_eq!(
            document["versions"]["1.0.0"]["deprecated"], "use widget2 instead",
            "the existing version must have been deprecated in place, not replaced"
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn publishing_with_an_invalid_package_name_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, org_id, "acme-token").await;
        seed_permission(&pool, user_id, repo_id, "write").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("1.0.0", Some("Bad-1.0.0.tgz"), Some(&tarball_b64), json!({}));

        let app = crate::router(state);
        // Uppercase letters are not a valid npm package name segment.
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/Bad-Name"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn publishing_with_an_invalid_version_string_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, org_id, "acme-token").await;
        seed_permission(&pool, user_id, repo_id, "write").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("not-a-version", Some("widget-x.tgz"), Some(&tarball_b64), json!({}));

        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn publishing_with_invalid_base64_tarball_data_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, org_id, "acme-token").await;
        seed_permission(&pool, user_id, repo_id, "write").await;

        let body = publish_body("1.0.0", Some("widget-1.0.0.tgz"), Some("not-valid-base64!!!"), json!({}));

        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Proves `require_hosted` is actually wired up at this route, not just unit-tested in `authz.rs`.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn publishing_against_a_proxy_repository_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, repo_id, "npm", "proxy").await;
        let repo_name = format!("repo-{repo_id}");
        let user_id = seed_user_with_active_token(&pool, org_id, "acme-token").await;
        seed_permission(&pool, user_id, repo_id, "write").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("1.0.0", Some("widget-1.0.0.tgz"), Some(&tarball_b64), json!({}));

        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer acme-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED, "publish must be rejected on a proxy repository at the route level, not just in the unit-tested require_hosted() itself");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn publishing_by_a_caller_from_a_different_organization_is_rejected(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let acme_id = Uuid::new_v4();
        create_org(&state, acme_id, "acme").await;
        let other_id = Uuid::new_v4();
        create_org(&state, other_id, "other").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, acme_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        // A caller from a different organization holding an explicit (stale) grant on acme's repository.
        let other_user_id = seed_user_with_active_token(&pool, other_id, "other-token").await;
        seed_permission(&pool, other_user_id, repo_id, "write").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("1.0.0", Some("widget-1.0.0.tgz"), Some(&tarball_b64), json!({}));

        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer other-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a valid write-role grant on a repository in a DIFFERENT organization must not let a publish through the actual route"
        );
    }

    /// Mirrors artiferris-api's org-admin bypass (`artiferris_api::authz::effective_repository_role`) — implicit Write on any repository in their own org, no explicit grant needed.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn publishing_as_an_organization_admin_of_the_repositorys_own_organization_succeeds_without_an_explicit_grant(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, repo_id, "npm", "hosted").await;
        let repo_name = format!("repo-{repo_id}");
        // Deliberately no `seed_permission` call — the org-admin bypass must not need one.
        seed_organization_admin_with_active_token(&pool, org_id, "org-admin-token").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("1.0.0", Some("widget-1.0.0.tgz"), Some(&tarball_b64), json!({}));

        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/widget"))
                    .header("host", "acme.artiferris.localhost")
                    .header(axum::http::header::AUTHORIZATION, "Bearer org-admin-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    /// B-14: `publish`'s TYPE check (`require_hosted`, which 405s a proxy/group repository) must
    /// not run before the ROLE check — otherwise an unauthorized caller can tell a private proxy
    /// repository apart from a private hosted one purely from the status code, without ever having
    /// read access. A caller from a DIFFERENT organization already gets 404 before either check
    /// (`require_repository_by_name`'s own org-membership gate) regardless of ordering, so this
    /// needs a caller in the REPOSITORIES' OWN organization who simply holds no grant at all.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_unauthorized_publish_against_a_proxy_repository_leaks_no_more_than_against_a_hosted_one(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let hosted_repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, hosted_repo_id, "npm", "hosted").await;
        let hosted_repo_name = format!("repo-{hosted_repo_id}");
        let proxy_repo_id = Uuid::new_v4();
        seed_repository(&pool, org_id, proxy_repo_id, "npm", "proxy").await;
        let proxy_repo_name = format!("repo-{proxy_repo_id}");
        // Same organization as both repositories, but deliberately no `seed_permission` call —
        // genuinely unauthorized, not merely cross-org (which is already 404 before any type check).
        let _user_id = seed_user_with_active_token(&pool, org_id, "acme-token").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("1.0.0", Some("widget-1.0.0.tgz"), Some(&tarball_b64), json!({}));

        let app = crate::router(state);
        let request = |repo_name: &str| {
            Request::builder()
                .method("PUT")
                .uri(format!("/{repo_name}/widget"))
                .header("host", "acme.artiferris.localhost")
                .header(axum::http::header::AUTHORIZATION, "Bearer acme-token")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };

        let hosted_response = app.clone().oneshot(request(&hosted_repo_name)).await.unwrap();
        let proxy_response = app.oneshot(request(&proxy_repo_name)).await.unwrap();

        assert_eq!(
            hosted_response.status(),
            proxy_response.status(),
            "an unauthorized caller's response must not distinguish a proxy repository from a hosted one by status code"
        );
        assert_eq!(hosted_response.status(), StatusCode::FORBIDDEN, "role must be checked before repository type, so a plain permission denial (403) comes back either way");
    }

    // ---- B-13: personal-namespace write support ----

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn publishing_to_a_personal_project_by_its_owner_succeeds(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        create_personal_project(&pool, alice_id, "my-lib").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("1.0.0", Some("my-lib-1.0.0.tgz"), Some(&tarball_b64), json!({ "name": "my-lib" }));

        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/u/alice/my-lib/my-lib")
                    .header(axum::http::header::AUTHORIZATION, "Bearer alice-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn publishing_to_someone_elses_personal_project_is_rejected_as_not_found(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool.clone(), dir.path()).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        create_personal_project(&pool, alice_id, "my-lib").await;

        // Mallory belongs to an unrelated organization and holds no grant on alice's personal
        // repository — same 404-not-403 discipline as the existing read-only personal routes.
        let other_id = Uuid::new_v4();
        create_org(&state, other_id, "other-corp").await;
        seed_named_user_with_active_token(&pool, other_id, "mallory", "mallory-token").await;

        let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let body = publish_body("1.0.0", Some("my-lib-1.0.0.tgz"), Some(&tarball_b64), json!({}));

        let app = crate::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/u/alice/my-lib/my-lib")
                    .header(axum::http::header::AUTHORIZATION, "Bearer mallory-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a non-owner must not be able to tell a private personal project apart from a nonexistent one"
        );
    }

    // ---- B-1 / B-12 / B-13 / B-16 ----

    struct Fixture {
        state: NpmState,
        app: axum::Router,
        repo_id: Uuid,
        repo_name: String,
        _dir: tempfile::TempDir,
    }

    /// A hosted npm repository in `acme`, with a writer (`writer-token`), a reader (`reader-token`) and a member with no grant (`nobody-token`).
    async fn fixture(pool: &PgPool) -> Fixture {
        fixture_with(pool, |_| {}).await
    }

    async fn fixture_with(pool: &PgPool, adjust: impl FnOnce(&mut NpmState)) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut state = test_state(pool.clone(), dir.path()).await;
        adjust(&mut state);
        let org_id = Uuid::new_v4();
        create_org(&state, org_id, "acme").await;
        let repo_id = Uuid::new_v4();
        seed_repository(pool, org_id, repo_id, "npm", "hosted").await;
        let writer = seed_user_with_active_token(pool, org_id, "writer-token").await;
        seed_permission(pool, writer, repo_id, "write").await;
        let reader = seed_user_with_active_token(pool, org_id, "reader-token").await;
        seed_permission(pool, reader, repo_id, "read").await;
        seed_user_with_active_token(pool, org_id, "nobody-token").await;
        Fixture { app: crate::router(state.clone()), state, repo_id, repo_name: format!("repo-{repo_id}"), _dir: dir }
    }

    fn request(method: &str, uri: &str, token: Option<&str>, body: Body) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(uri).header("host", "acme.artiferris.localhost").header(axum::http::header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            builder = builder.header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder.body(body).unwrap()
    }

    async fn publish_document(fixture: &Fixture, token: &str, doc: serde_json::Value) -> StatusCode {
        let text = doc.to_string();
        let mut put = request("PUT", &format!("/{}/widget", fixture.repo_name), Some(token), Body::from(text.clone()));
        put.headers_mut().insert(axum::http::header::CONTENT_LENGTH, text.len().to_string().parse().unwrap());
        fixture.app.clone().oneshot(put).await.unwrap().status()
    }

    fn widget_document(version: &str) -> serde_json::Value {
        let tarball = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        json!({
            "name": "widget",
            "versions": { version: { "name": "widget", "version": version } },
            "_attachments": { format!("widget-{version}.tgz"): { "data": tarball } },
            "dist-tags": { "latest": version },
        })
    }

    async fn dist_tags_of_widget(fixture: &Fixture) -> Vec<(String, String)> {
        let name = NpmPackageName::parse("widget").unwrap();
        let mut tags: Vec<(String, String)> = fixture.state.list_dist_tags.execute(fixture.repo_id, &name).await.unwrap().into_iter().map(|t| (t.tag, t.version.as_str())).collect();
        tags.sort();
        tags
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_publish_body_is_never_read_for_a_caller_who_may_not_publish(pool: PgPool) {
        let f = fixture(&pool).await;
        let alice_id = seed_named_user_with_active_token(&pool, PUBLIC_ORGANIZATION_ID, "alice", "alice-token").await;
        create_personal_project(&pool, alice_id, "my-lib").await;

        for (uri, token, expected) in [
            (format!("/{}/widget", f.repo_name), Some("reader-token"), StatusCode::FORBIDDEN),
            (format!("/{}/widget", f.repo_name), Some("nobody-token"), StatusCode::FORBIDDEN),
            (format!("/{}/widget", f.repo_name), None, StatusCode::UNAUTHORIZED),
            ("/no-such-repository/widget".to_string(), Some("writer-token"), StatusCode::NOT_FOUND),
            ("/u/alice/my-lib/my-lib".to_string(), Some("writer-token"), StatusCode::NOT_FOUND),
        ] {
            let (body, polled) = crate::test_support::probe_body();

            let response = f.app.clone().oneshot(request("PUT", &uri, token, body)).await.unwrap();

            assert_eq!(response.status(), expected, "{uri} {token:?}");
            assert!(!polled.load(std::sync::atomic::Ordering::SeqCst), "{uri} {token:?}: the body of a refused caller was read");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_publish_that_declares_itself_over_the_limit_is_refused_unread(pool: PgPool) {
        let f = fixture(&pool).await;
        let (body, polled) = crate::test_support::probe_body();
        let mut declared_huge = request("PUT", &format!("/{}/widget", f.repo_name), Some("writer-token"), body);
        declared_huge.headers_mut().insert(axum::http::header::CONTENT_LENGTH, (PUBLISH_BODY_LIMIT_BYTES + 1).to_string().parse().unwrap());

        let response = f.app.clone().oneshot(declared_huge).await.unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(!polled.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_publish_is_turned_away_while_the_body_budget_is_spent(pool: PgPool) {
        let f = fixture_with(&pool, |state| {
            state.guard = std::sync::Arc::new(artiferris_application::request_guard::RequestGuard { body_budget: artiferris_application::body_budget::BodyBudget::new(4096), ..Default::default() });
        })
        .await;

        let document = widget_document("1.0.0");
        let mut doc = document;
        doc["padding"] = json!("x".repeat(8192));

        assert_eq!(publish_document(&f, "writer-token", doc).await, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(publish_document(&f, "writer-token", widget_document("1.0.0")).await, StatusCode::CREATED, "a small enough publish still fits");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_repository_name_with_a_control_character_is_not_found_rather_than_a_server_error(pool: PgPool) {
        let f = fixture(&pool).await;

        for name in ["repo%00x", "repo%0Ax", "repo%7Fx"] {
            let response = f.app.clone().oneshot(request("PUT", &format!("/{name}/widget"), Some("writer-token"), Body::from("{}"))).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{name}");
        }
    }

    /// A body that declares `declared` bytes, sends one and then goes quiet.
    fn stalled_publish(fixture: &Fixture, declared: usize) -> Request<Body> {
        use futures_util::StreamExt;

        let stalled = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"{"))]).chain(futures_util::stream::pending());
        let mut put = request("PUT", &format!("/{}/widget", fixture.repo_name), Some("writer-token"), Body::from_stream(stalled));
        put.headers_mut().insert(axum::http::header::CONTENT_LENGTH, declared.to_string().parse().unwrap());
        put
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn bodies_that_stall_after_a_byte_do_not_pin_the_budget_others_need(pool: PgPool) {
        // Each of the two stalled bodies declares 2 MiB, which is 6 MiB of the 8 MiB budget at the publish factor.
        let f = fixture_with(&pool, |state| {
            state.guard = std::sync::Arc::new(artiferris_application::request_guard::RequestGuard { body_budget: artiferris_application::body_budget::BodyBudget::new(8 * 1024 * 1024), ..Default::default() });
        })
        .await;
        let stalled: Vec<_> = (0..2).map(|_| tokio::spawn(f.app.clone().oneshot(stalled_publish(&f, 2 * 1024 * 1024)))).collect();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert_eq!(publish_document(&f, "writer-token", widget_document("1.0.0")).await, StatusCode::CREATED, "a third publish must not be starved by the two stalled ones");
        for task in stalled {
            task.abort();
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn one_user_cannot_have_more_than_a_few_publish_bodies_in_flight(pool: PgPool) {
        let f = fixture(&pool).await;
        let stalled: Vec<_> = (0..artiferris_application::body_budget::MAX_BODIES_PER_CLIENT).map(|_| tokio::spawn(f.app.clone().oneshot(stalled_publish(&f, 1024)))).collect();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let response = f.app.clone().oneshot(stalled_publish(&f, 1024)).await.unwrap();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        for task in stalled {
            task.abort();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(publish_document(&f, "writer-token", widget_document("1.0.0")).await, StatusCode::CREATED, "the slots come back when the stalled bodies go");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_publish_body_that_stalls_gets_408_and_gives_its_budget_back(pool: PgPool) {
        use futures_util::StreamExt;

        let budget_bytes = 2 * PUBLISH_BODY_LIMIT_BYTES * PUBLISH_MEMORY_FACTOR;
        let f = fixture_with(&pool, |state| {
            let timeouts = artiferris_application::body_read::BodyTimeouts { idle: std::time::Duration::from_millis(100), total: std::time::Duration::from_secs(5) };
            state.guard = std::sync::Arc::new(artiferris_application::request_guard::RequestGuard {
                body_budget: artiferris_application::body_budget::BodyBudget::new(budget_bytes),
                body_timeouts: timeouts,
                ..Default::default()
            });
        })
        .await;
        let stalled = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"{\"name\":"))]).chain(futures_util::stream::pending());

        let response = f.app.clone().oneshot(request("PUT", &format!("/{}/widget", f.repo_name), Some("writer-token"), Body::from_stream(stalled))).await.unwrap();

        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        assert!(f.state.guard.body_budget.try_reserve(budget_bytes).is_some(), "the stalled request kept its reservation");
        assert_eq!(publish_document(&f, "writer-token", widget_document("1.0.0")).await, StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_publish_document_whose_parts_disagree_is_refused_and_nothing_is_stored(pool: PgPool) {
        let f = fixture(&pool).await;
        let tarball = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let mismatches = [
            ("name in the document", { let mut d = widget_document("1.0.0"); d["name"] = json!("other"); d }),
            ("_id in the document", { let mut d = widget_document("1.0.0"); d["_id"] = json!("other"); d }),
            ("name in the manifest", json!({ "versions": { "1.0.0": { "name": "other", "version": "1.0.0" } }, "_attachments": { "widget-1.0.0.tgz": { "data": tarball } } })),
            ("version in the manifest", json!({ "versions": { "1.0.0": { "name": "widget", "version": "9.9.9" } }, "_attachments": { "widget-1.0.0.tgz": { "data": tarball } } })),
            ("attachment file name", json!({ "versions": { "1.0.0": { "name": "widget", "version": "1.0.0" } }, "_attachments": { "widget-2.0.0.tgz": { "data": tarball } } })),
            ("attachment for another package", json!({ "versions": { "1.0.0": { "name": "widget", "version": "1.0.0" } }, "_attachments": { "other-1.0.0.tgz": { "data": tarball } } })),
            ("two versions", json!({ "versions": { "1.0.0": { "name": "widget" }, "1.0.1": { "name": "widget" } }, "_attachments": { "widget-1.0.0.tgz": { "data": tarball } } })),
            ("two attachments", json!({ "versions": { "1.0.0": { "name": "widget" } }, "_attachments": { "widget-1.0.0.tgz": { "data": tarball }, "widget-1.0.1.tgz": { "data": tarball } } })),
            ("a manifest that is not an object", json!({ "versions": { "1.0.0": "nope" }, "_attachments": { "widget-1.0.0.tgz": { "data": tarball } } })),
            ("dist-tags that do not point at the version", { let mut d = widget_document("1.0.0"); d["dist-tags"] = json!({ "latest": "2.0.0" }); d }),
        ];

        for (what, document) in mismatches {
            assert_eq!(publish_document(&f, "writer-token", document).await, StatusCode::BAD_REQUEST, "{what}");
        }

        let name = NpmPackageName::parse("widget").unwrap();
        assert!(f.state.metadata.execute_hosted(f.repo_id, &name).await.unwrap().is_none(), "no partial publish");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_scoped_package_publishes_with_its_scope_in_the_tarball_file_name(pool: PgPool) {
        let f = fixture(&pool).await;
        let tarball = base64::engine::general_purpose::STANDARD.encode(b"tarball-bytes");
        let document = json!({
            "name": "@acme/widget",
            "versions": { "1.0.0": { "name": "@acme/widget", "version": "1.0.0" } },
            "_attachments": { "@acme/widget-1.0.0.tgz": { "data": tarball } },
            "dist-tags": { "latest": "1.0.0" },
        });

        let response = f.app.clone().oneshot(request("PUT", &format!("/{}/@acme%2fwidget", f.repo_name), Some("writer-token"), Body::from(document.to_string()))).await.unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn npm_publish_tag_moves_only_the_requested_tag(pool: PgPool) {
        let f = fixture(&pool).await;
        assert_eq!(publish_document(&f, "writer-token", widget_document("2.0.0")).await, StatusCode::CREATED);

        // `npm publish --tag maintenance` of a backport.
        let mut backport = widget_document("1.5.1");
        backport["dist-tags"] = json!({ "maintenance": "1.5.1" });
        assert_eq!(publish_document(&f, "writer-token", backport).await, StatusCode::CREATED);

        assert_eq!(dist_tags_of_widget(&f).await, vec![("latest".to_string(), "2.0.0".to_string()), ("maintenance".to_string(), "1.5.1".to_string())]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_invalid_dist_tag_in_a_publish_is_refused(pool: PgPool) {
        let f = fixture(&pool).await;
        let mut document = widget_document("1.0.0");
        document["dist-tags"] = json!({ "1.0.0": "1.0.0" });

        assert_eq!(publish_document(&f, "writer-token", document).await, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_version_that_was_unpublished_cannot_be_published_again_with_other_bytes(pool: PgPool) {
        let f = fixture(&pool).await;
        assert_eq!(publish_document(&f, "writer-token", widget_document("1.0.0")).await, StatusCode::CREATED);
        let unpublished = f
            .app
            .clone()
            .oneshot(request("DELETE", &format!("/{}/widget/-/widget-1.0.0.tgz/-rev/1-x", f.repo_name), Some("writer-token"), Body::empty()))
            .await
            .unwrap();
        assert_eq!(unpublished.status(), StatusCode::OK);

        let mut different_bytes = widget_document("1.0.0");
        different_bytes["_attachments"]["widget-1.0.0.tgz"]["data"] = json!(base64::engine::general_purpose::STANDARD.encode(b"other bytes"));
        assert_eq!(publish_document(&f, "writer-token", different_bytes).await, StatusCode::CONFLICT);
        assert_eq!(publish_document(&f, "writer-token", widget_document("1.0.1")).await, StatusCode::CREATED, "another version number is fine");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_advisory_body_is_read_only_for_a_reader_and_only_within_its_caps(pool: PgPool) {
        let f = fixture(&pool).await;
        let uri = format!("/{}/-/npm/v1/security/advisories/bulk", f.repo_name);

        let (body, polled) = crate::test_support::probe_body();
        let refused = f.app.clone().oneshot(request("POST", &uri, Some("nobody-token"), body)).await.unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert!(!polled.load(std::sync::atomic::Ordering::SeqCst));

        let many: std::collections::HashMap<String, Vec<String>> = (0..=super::super::advisories::MAX_ADVISORY_PACKAGES).map(|i| (format!("pkg-{i}"), vec!["1.0.0".to_string()])).collect();
        let too_many = f.app.clone().oneshot(request("POST", &uri, Some("reader-token"), Body::from(serde_json::to_vec(&many).unwrap()))).await.unwrap();
        assert_eq!(too_many.status(), StatusCode::BAD_REQUEST);
    }
}
