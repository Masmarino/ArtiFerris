// Builds a real DockerState (real Postgres adapters, real filesystem blob storage under a tempdir) for exercising route wiring end-to-end.
use std::path::Path;
use std::sync::Arc;

use artiferris_application::login_throttle::LoginThrottle;
use artiferris_application::request_guard::RequestGuard;
use artiferris_application::use_cases::admin::RecordSecurityEventUseCase;
use artiferris_application::use_cases::api_token::hash_api_token;
use artiferris_application::use_cases::docker_access_token::IssueDockerAccessTokenUseCase;
use artiferris_application::use_cases::docker_blob_get::GetBlobUseCase;
use artiferris_application::use_cases::docker_list::{ListCatalogUseCase, ListDockerRegistryCatalogUseCase, ListTagsUseCase};
use artiferris_application::use_cases::docker_manifest_cache::CacheProxiedManifestUseCase;
use artiferris_application::use_cases::docker_manifest_delete::DeleteManifestUseCase;
use artiferris_application::use_cases::docker_manifest_get::GetManifestUseCase;
use artiferris_application::use_cases::docker_manifest_put::PutManifestUseCase;
use artiferris_application::use_cases::docker_scan::ScanDockerImageUseCase;
use artiferris_application::use_cases::docker_upload::{CompleteBlobUploadUseCase, MonolithicBlobUploadUseCase, PatchBlobUploadUseCase, StartBlobUploadUseCase};
use artiferris_application::use_cases::personal_repository::{CreateUserProjectUseCase, ReservePersonalOrganizationUseCase};
use artiferris_application::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase;
use artiferris_infrastructure::filesystem_docker_blob_store::FilesystemDockerBlobStore;
use artiferris_infrastructure::http_remote_docker_registry::HttpRemoteDockerRegistry;
use artiferris_infrastructure::jwt_docker_token_issuer::JwtDockerTokenIssuer;
use artiferris_infrastructure::postgres::api_token_repository::PostgresApiTokenRepository;
use artiferris_infrastructure::postgres::docker_image_scan_repository::PostgresDockerImageScanRepository;
use artiferris_infrastructure::postgres::docker_manifest_repository::PostgresDockerManifestRepository;
use artiferris_infrastructure::postgres::docker_upload_session_repository::PostgresDockerUploadSessionRepository;
use artiferris_infrastructure::trivy_docker_image_scanner::TrivyDockerImageScanner;
use artiferris_infrastructure::postgres::event_publisher::PostgresEventPublisher;
use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
use artiferris_infrastructure::postgres::permission_store::PostgresPermissionStore;
use artiferris_infrastructure::postgres::user_repository::PostgresUserRepository;
use artiferris_domain::docker_registry::DockerGrantedScope;
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::{DockerState, TOKENS_VALID_AFTER_CACHE_TTL};
use crate::tokens_valid_after_cache::TokensValidAfterCache;

pub async fn test_state(pool: PgPool, root: &Path) -> DockerState {
    test_state_with_cache_ttl(pool, root, TOKENS_VALID_AFTER_CACHE_TTL).await
}

/// `test_state` with a caller-chosen revocation-cache TTL — a zero TTL turns every auth check into
/// a live lookup, which is how a test exercises the DB-backed path instead of a warm cache entry.
pub async fn test_state_with_cache_ttl(pool: PgPool, root: &Path, cache_ttl: std::time::Duration) -> DockerState {
    let organizations: Arc<dyn artiferris_domain::organization::OrganizationRepositoryPort> = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
    let repositories = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
    let permissions = Arc::new(PostgresPermissionStore::new(pool.clone()));
    let users = Arc::new(PostgresUserRepository::new(pool.clone()));
    let api_tokens = Arc::new(PostgresApiTokenRepository::new(pool.clone()));
    let blobs = Arc::new(FilesystemDockerBlobStore::new(pool.clone(), root.join("blobs")));
    let manifests = Arc::new(PostgresDockerManifestRepository::new(pool.clone()));
    let uploads = Arc::new(PostgresDockerUploadSessionRepository::new(pool.clone(), root.join("uploads")));
    let remote: Arc<dyn artiferris_domain::docker_remote::RemoteDockerRegistryPort> = Arc::new(HttpRemoteDockerRegistry::new());
    let token_issuer: Arc<dyn artiferris_domain::docker_registry::DockerTokenIssuerPort> = Arc::new(JwtDockerTokenIssuer::new("test-secret".to_string()));
    let events: Arc<dyn artiferris_domain::audit::EventPublisherPort> = Arc::new(PostgresEventPublisher::new(pool.clone()));
    let docker_image_scans: Arc<dyn artiferris_domain::docker_scan::DockerImageScanRepositoryPort> = Arc::new(PostgresDockerImageScanRepository::new(pool.clone()));
    let docker_scanner: Arc<dyn artiferris_domain::docker_scan::DockerImageScannerPort> = Arc::new(TrivyDockerImageScanner::new("127.0.0.1:0".to_string()));
    let list_catalog = Arc::new(ListCatalogUseCase::new(manifests.clone()));
    let resolve_personal_repository = Arc::new(ResolvePersonalRepositoryUseCase::new(users.clone(), organizations.clone(), repositories.clone()));

    let start_upload = Arc::new(StartBlobUploadUseCase::new(uploads.clone()));
    let patch_upload = Arc::new(PatchBlobUploadUseCase::new(uploads.clone(), blobs.clone(), repositories.clone(), repositories.clone()));
    let complete_upload = Arc::new(CompleteBlobUploadUseCase::new(uploads.clone(), blobs.clone()));

    DockerState {
        repositories: repositories.clone(),
        permissions: permissions.clone(),
        organizations: organizations.clone(),
        users: users.clone(),
        tokens_valid_after_cache: TokensValidAfterCache::new(cache_ttl),
        artiferris_base_domain: "artiferris.localhost".to_string(),
        token_issuer: token_issuer.clone(),
        token_realm_override: None,
        public_scheme: "http".to_string(),
        token_service: "artiferris".to_string(),
        issue_access_token: Arc::new(IssueDockerAccessTokenUseCase::new(
            api_tokens,
            users.clone(),
            repositories.clone(),
            permissions.clone(),
            token_issuer,
            resolve_personal_repository.clone(),
        )),
        login_throttle: LoginThrottle::new(),
        record_security_event: Arc::new(RecordSecurityEventUseCase::new(events.clone())),
        guard: Arc::new(RequestGuard::default()),
        start_upload: start_upload.clone(),
        patch_upload: patch_upload.clone(),
        complete_upload: complete_upload.clone(),
        monolithic_upload: Arc::new(MonolithicBlobUploadUseCase::new(start_upload, patch_upload, complete_upload, uploads.clone())),
        put_manifest: Arc::new(PutManifestUseCase::new(manifests.clone(), repositories.clone(), events.clone())),
        cache_proxied_manifest: Arc::new(CacheProxiedManifestUseCase::new(manifests.clone())),
        get_manifest: Arc::new(GetManifestUseCase::new(manifests.clone(), repositories.clone(), remote.clone())),
        downloads: Arc::new(artiferris_domain::download_stats::NoopDownloadRecorder),
        get_blob: Arc::new(GetBlobUseCase::new(blobs.clone(), manifests.clone(), repositories.clone(), remote)),
        delete_manifest: Arc::new(DeleteManifestUseCase::new(manifests.clone(), blobs, events)),
        list_tags: Arc::new(ListTagsUseCase::new(manifests.clone(), repositories.clone())),
        list_catalog,
        list_registry_catalog: Arc::new(ListDockerRegistryCatalogUseCase::new(repositories.clone(), permissions.clone(), manifests.clone())),
        scan_docker_image: Arc::new(ScanDockerImageUseCase::new(
            manifests,
            repositories.clone(),
            Arc::new(JwtDockerTokenIssuer::new("test-secret".to_string())),
            docker_scanner,
            docker_image_scans,
        )),
        resolve_personal_repository,
    }
}

pub async fn seed_repository(pool: &PgPool, organization_id: Uuid, id: Uuid, format: &str, repo_type: &str) {
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

/// A user row with no API token attached. `DockerAuthUser` now re-reads the token holder's
/// `tokens_valid_after` on every data-plane request (M-17), so even a test that only cares about a
/// token's granted scope needs its holder to actually exist — a bare `Uuid::new_v4()` holder is
/// rejected as a deleted account, exactly as it would be in production.
pub async fn seed_bare_user(pool: &PgPool, organization_id: Uuid) -> Uuid {
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
    user_id
}

pub async fn seed_user_with_active_token(pool: &PgPool, organization_id: Uuid, plaintext_token: &str) -> Uuid {
    let user_id = Uuid::new_v4();
    // Usernames cap at 32 chars, so truncate the UUID rather than use it whole.
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

pub async fn seed_organization_admin_with_active_token(pool: &PgPool, organization_id: Uuid, plaintext_token: &str) -> Uuid {
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

/// Defers to `issue_test_token_for_org` with the public org and a non-super-admin holder — use that one directly when the holder's org needs to differ from the request's.
pub fn issue_test_token(state: &DockerState, user_id: Uuid, repository_id: Uuid, repository_name: &str, image_name: &str, actions: &[&str]) -> String {
    issue_test_token_for_org(state, user_id, artiferris_domain::organization::PUBLIC_ORGANIZATION_ID, false, repository_id, repository_name, image_name, actions)
}

#[allow(clippy::too_many_arguments)]
pub fn issue_test_token_for_org(
    state: &DockerState,
    user_id: Uuid,
    organization_id: Uuid,
    is_super_admin: bool,
    repository_id: Uuid,
    repository_name: &str,
    image_name: &str,
    actions: &[&str],
) -> String {
    let scope = DockerGrantedScope {
        resource_type: "repository".to_string(),
        name: format!("{repository_name}/{image_name}"),
        actions: actions.iter().map(|a| a.to_string()).collect(),
        granted_repository_id: Some(repository_id),
    };
    state.token_issuer.issue(user_id, organization_id, is_super_admin, Some(scope)).unwrap()
}

pub async fn seed_permission(pool: &PgPool, user_id: Uuid, repository_id: Uuid, role: &str) {
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
/// `/u/{username}/...` routes, which address a user by their real, stable username rather than
/// the throwaway `user-{uuid}` names `seed_user_with_active_token` generates.
pub async fn seed_named_user_with_active_token(pool: &PgPool, organization_id: Uuid, username: &str, plaintext_token: &str) -> Uuid {
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

/// Reserves `owner_user_id`'s personal organization and creates a project inside it, returning the
/// new repository's id — mirrors the setup `ResolvePersonalRepositoryUseCase`'s own tests use.
pub async fn create_personal_project(
    pool: &PgPool,
    owner_user_id: Uuid,
    project_name: &str,
    format: artiferris_domain::package_repository::RepositoryFormat,
) -> Uuid {
    let organizations: Arc<dyn artiferris_domain::organization::OrganizationRepositoryPort> = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
    let users: Arc<dyn artiferris_domain::user::UserRepositoryPort> = Arc::new(PostgresUserRepository::new(pool.clone()));
    let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));

    ReservePersonalOrganizationUseCase::new(organizations.clone(), users).execute(owner_user_id).await.unwrap();
    let create_project = CreateUserProjectUseCase::new(organizations, repository_store.clone(), repository_store);
    create_project.execute(owner_user_id, project_name, format, artiferris_domain::package_repository::RepositoryType::Hosted).await.unwrap()
}

/// Reserves `owner_user_id`'s personal namespace once, then creates a GROUP project and a HOSTED
/// project inside it and attaches the latter to the former — the exact shape the public API
/// produces. Returns `(group id, member id)`. `CreateUserProjectUseCase` grants the owner `Admin`
/// on each project it creates.
pub async fn create_personal_group_over_a_personal_member(
    pool: &PgPool,
    owner_user_id: Uuid,
    group_name: &str,
    member_name: &str,
    format: artiferris_domain::package_repository::RepositoryFormat,
) -> (Uuid, Uuid) {
    use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort, RepositoryType};

    let organizations: Arc<dyn artiferris_domain::organization::OrganizationRepositoryPort> = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
    let users: Arc<dyn artiferris_domain::user::UserRepositoryPort> = Arc::new(PostgresUserRepository::new(pool.clone()));
    let repository_store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));

    ReservePersonalOrganizationUseCase::new(organizations.clone(), users).execute(owner_user_id).await.unwrap();
    let create_project = CreateUserProjectUseCase::new(organizations, repository_store.clone(), repository_store.clone());
    let group_id = create_project.execute(owner_user_id, group_name, format, RepositoryType::Group).await.unwrap();
    let member_id = create_project.execute(owner_user_id, member_name, format, RepositoryType::Hosted).await.unwrap();

    // Attached through the event store — the same event the API's add-group-member route emits.
    let (version, _) = repository_store.load(group_id).await.unwrap();
    repository_store
        .append(
            group_id,
            version,
            vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: group_id, member_repository_id: member_id, position: 0 }],
            owner_user_id,
        )
        .await
        .unwrap();

    (group_id, member_id)
}

pub async fn mark_public(pool: &PgPool, repository_id: Uuid) {
    sqlx::query!("UPDATE package_repository_projections SET is_public = true WHERE id = $1", repository_id).execute(pool).await.unwrap();
}

pub async fn set_quota(pool: &PgPool, repository_id: Uuid, quota_bytes: i64) {
    sqlx::query!("UPDATE package_repository_projections SET quota_bytes = $1 WHERE id = $2", quota_bytes, repository_id).execute(pool).await.unwrap();
}

/// A 64 MiB request body that records whether anyone read from it.
pub fn probe_body() -> (axum::body::Body, Arc<std::sync::atomic::AtomicBool>) {
    let polled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = polled.clone();
    let mut remaining = 64;
    let stream = futures_util::stream::poll_fn(move |_| {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        if remaining == 0 {
            return std::task::Poll::Ready(None);
        }
        remaining -= 1;
        std::task::Poll::Ready(Some(Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![0u8; 1024 * 1024]))))
    });
    (axum::body::Body::from_stream(stream), polled)
}
