use std::net::SocketAddr;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use artiferris_application::use_cases::npm_audit::AuditRequest;
use artiferris_domain::npm_package::{NpmPackageName, NpmVersion};
use artiferris_domain::permission::Role;
use serde::Serialize;
use uuid::Uuid;

use crate::auth_middleware::AuthUser;
use crate::dto::{application_error_response, ErrorResponse};
use crate::install_location::RepositoryLocation;
use crate::routes::public_catalog::spend_budget;
use crate::state::AppState;

use super::{load_repository, repository_access_error, require_anonymous_budget, require_readable_repository_access, require_repository_access};

#[derive(Serialize)]
struct NpmVersionDetailResponse {
    version: String,
    published_at: DateTime<Utc>,
    size_bytes: i64,
    deprecated: bool,
    deprecated_message: Option<String>,
    shasum: String,
}

#[derive(Serialize)]
struct NpmDistTagDetailResponse {
    tag: String,
    version: String,
}

#[derive(Serialize)]
pub(super) struct NpmPackageDetailsResponse {
    name: String,
    /// Newest first; capped, see `truncated`.
    versions: Vec<NpmVersionDetailResponse>,
    truncated: bool,
    dist_tags: Vec<NpmDistTagDetailResponse>,
    /// Already converted and sanitized on the backend, so a client may embed it as is.
    readme_html: Option<String>,
    /// Downloads over the last seven days.
    downloads_7d: i64,
    /// The owner's registry URL, to install from.
    registry_url: String,
}

pub(super) async fn get_npm_package_details(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path((id, name)): Path<(Uuid, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<NpmPackageDetailsResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_anonymous_budget(&state, &user, &headers, connect_info)?;
    let repo = load_repository(&state, id).await?;
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, id, repo.is_public, "view package details").await.map_err(repository_access_error)?;
    let parsed = NpmPackageName::parse(&name).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid package name".to_string() })))?;
    let details = state
        .get_npm_package_details
        .execute(id, &parsed)
        .await
        .map_err(|e| application_error_response("failed to get npm package details", e))?
        .ok_or((StatusCode::NOT_FOUND, Json(ErrorResponse { error: "package not found".to_string() })))?;
    let owner = state.organizations.find_by_id(repo.organization_id).await.ok().flatten().ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "internal error".to_string() })))?;
    let registry_url = RepositoryLocation::of(&repo, &owner).registry_url(&state.public_url, &state.artiferris_base_domain);
    Ok(Json(NpmPackageDetailsResponse {
        name: details.name,
        truncated: details.truncated,
        readme_html: details.readme_html,
        downloads_7d: details.downloads_7d,
        registry_url,
        versions: details
            .versions
            .into_iter()
            .map(|v| NpmVersionDetailResponse {
                version: v.version,
                published_at: v.published_at,
                size_bytes: v.size_bytes,
                deprecated: v.deprecated,
                deprecated_message: v.deprecated_message,
                shasum: v.shasum,
            })
            .collect(),
        dist_tags: details.dist_tags.into_iter().map(|t| NpmDistTagDetailResponse { tag: t.tag, version: t.version }).collect(),
    }))
}

pub(super) async fn delete_npm_package(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Write, "delete npm package").await.map_err(repository_access_error)?;
    let parsed = NpmPackageName::parse(&name).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid package name".to_string() })))?;
    state
        .unpublish_npm_package
        .execute_whole_package(id, &parsed, user.id)
        .await
        .map_err(|e| application_error_response("failed to delete npm package", e))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn delete_npm_package_version(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, name, version)): Path<(Uuid, String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Write, "delete npm package version").await.map_err(repository_access_error)?;
    let parsed_name =
        NpmPackageName::parse(&name).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid package name".to_string() })))?;
    let parsed_version =
        NpmVersion::parse(&version).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid version".to_string() })))?;
    state
        .unpublish_npm_package
        .execute_version(id, &parsed_name, &parsed_version, user.id)
        .await
        .map_err(|e| application_error_response("failed to delete npm package version", e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub(super) struct NpmAdvisoryResponse {
    id: i64,
    url: String,
    title: String,
    severity: String,
    vulnerable_versions: String,
    cwe: Vec<String>,
    cvss_score: Option<f64>,
}

/// Each audit can call npm's advisory service, so a signed-in account has a budget too.
const SIGNED_IN_AUDITS_PER_MINUTE: usize = 30;

pub(super) async fn audit_npm_package(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path((id, name)): Path<(Uuid, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<Vec<NpmAdvisoryResponse>>, (StatusCode, Json<ErrorResponse>)> {
    require_anonymous_budget(&state, &user, &headers, connect_info)?;
    let repo = load_repository(&state, id).await?;
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, id, repo.is_public, "audit npm package").await.map_err(repository_access_error)?;
    let parsed = NpmPackageName::parse(&name).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid package name".to_string() })))?;
    // An audit calls npm's advisory service, so only a signed-in caller can start one; anyone can read a recent result.
    let advisories = if let Some(user) = &user {
        if !spend_budget(&state, &format!("npm-audit:{}", user.id), SIGNED_IN_AUDITS_PER_MINUTE) {
            return Err((StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse { error: "too many audits, try again shortly".to_string() })));
        }
        let request = AuditRequest { user_id: user.id, repository_is_public: repo.is_public };
        state.audit_npm_package.execute(id, &parsed, request).await.map_err(|e| application_error_response("failed to audit npm package", e))?
    } else {
        state.audit_npm_package.cached(id, &parsed).ok_or((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "sign in to run an audit".to_string() })))?
    };
    Ok(Json(
        advisories
            .into_iter()
            .map(|a| NpmAdvisoryResponse {
                id: a.id,
                url: a.url,
                title: a.title,
                severity: a.severity,
                vulnerable_versions: a.vulnerable_versions,
                cwe: a.cwe,
                cvss_score: a.cvss_score,
            })
            .collect(),
    ))
}

#[derive(Serialize)]
struct DependencyAuditFindingResponse {
    dependency_name: String,
    dependency_version: String,
    advisory: NpmAdvisoryResponse,
}

#[derive(Serialize)]
pub(super) struct DependencyAuditResultResponse {
    scanned_at: DateTime<Utc>,
    packages_scanned: i32,
    truncated: bool,
    findings: Vec<DependencyAuditFindingResponse>,
}

impl From<artiferris_domain::npm_audit::DependencyAuditResult> for DependencyAuditResultResponse {
    fn from(result: artiferris_domain::npm_audit::DependencyAuditResult) -> Self {
        Self {
            scanned_at: result.scanned_at,
            packages_scanned: result.packages_scanned,
            truncated: result.truncated,
            findings: result
                .findings
                .into_iter()
                .map(|f| DependencyAuditFindingResponse {
                    dependency_name: f.dependency_name,
                    dependency_version: f.dependency_version,
                    advisory: NpmAdvisoryResponse {
                        id: f.advisory.id,
                        url: f.advisory.url,
                        title: f.advisory.title,
                        severity: f.advisory.severity,
                        vulnerable_versions: f.advisory.vulnerable_versions,
                        cwe: f.advisory.cwe,
                        cvss_score: f.advisory.cvss_score,
                    },
                })
                .collect(),
        }
    }
}

pub(super) async fn get_dependency_audit(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path((id, name, version)): Path<(Uuid, String, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<Option<DependencyAuditResultResponse>>, (StatusCode, Json<ErrorResponse>)> {
    require_anonymous_budget(&state, &user, &headers, connect_info)?;
    let repo = load_repository(&state, id).await?;
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, id, repo.is_public, "read npm dependency audit").await.map_err(repository_access_error)?;
    let parsed_name =
        NpmPackageName::parse(&name).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid package name".to_string() })))?;
    let parsed_version =
        NpmVersion::parse(&version).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid version".to_string() })))?;
    let result = state
        .get_dependency_audit
        .execute(id, &parsed_name, &parsed_version)
        .await
        .map_err(|e| application_error_response("failed to read dependency audit", e))?;
    Ok(Json(result.map(DependencyAuditResultResponse::from)))
}

pub(super) async fn scan_dependency_tree(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, name, version)): Path<(Uuid, String, String)>,
) -> Result<Json<DependencyAuditResultResponse>, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Write, "run npm dependency audit").await.map_err(repository_access_error)?;
    let parsed_name =
        NpmPackageName::parse(&name).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid package name".to_string() })))?;
    let parsed_version =
        NpmVersion::parse(&version).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "invalid version".to_string() })))?;
    let result = state
        .scan_dependency_tree
        .execute(user.id, id, &parsed_name, &parsed_version)
        .await
        .map_err(|e| application_error_response("failed to scan dependency tree", e))?;
    Ok(Json(result.into()))
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_support::test_config;
    use crate::{build_router, state::AppState};
    use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;


    /// #75: viewing a public repository's security-audit findings is treated the same as viewing
    /// its package content — open to anyone, `AuthUser` or not.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_read_the_dependency_audit_for_a_public_npm_package(pool: sqlx::PgPool) {
        use artiferris_domain::npm_audit::{DependencyAuditFinding, DependencyAuditResult, NpmAdvisory};
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};
        use artiferris_domain::npm_audit::DependencyAuditRepositoryPort;
        use artiferris_infrastructure::postgres::npm_dependency_audit_repository::PostgresDependencyAuditRepository;

        let state = AppState::build(pool.clone(), &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        let version = NpmPackageVersion {
            id: Uuid::new_v4(),
            npm_package_id: package.id,
            version: NpmVersion::parse("1.0.0").unwrap(),
            manifest: serde_json::json!({}),
            shasum: "shasum".to_string(),
            integrity: "integrity".to_string(),
            tarball_storage_key: "key".to_string(),
            tarball_size_bytes: 42,
            deprecated: false,
            deprecated_message: None,
            published_by: None,
            published_at: chrono::Utc::now(),
            origin: NpmPackageOrigin::Local,
        };
        state.npm_packages.insert_version(&version).await.unwrap();
        let audits = PostgresDependencyAuditRepository::new(pool);
        audits
            .save(&DependencyAuditResult {
                id: Uuid::new_v4(),
                npm_package_version_id: version.id,
                scanned_at: chrono::Utc::now(),
                packages_scanned: 1,
                truncated: false,
                findings: vec![DependencyAuditFinding {
                    dependency_name: "lodash".to_string(),
                    dependency_version: "4.0.0".to_string(),
                    advisory: NpmAdvisory {
                        id: 1,
                        url: "https://example.com/advisory/1".to_string(),
                        title: "Prototype pollution".to_string(),
                        severity: "high".to_string(),
                        vulnerable_versions: "<4.17.0".to_string(),
                        cwe: vec![],
                        cvss_score: None,
                    },
                }],
            })
            .await
            .unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad/versions/1.0.0/dependency-audit"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["findings"][0]["advisory"]["title"], "Prototype pollution");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_read_the_dependency_audit_for_a_private_npm_package(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        // Deliberately not marked public.
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("1.0.0").unwrap(),
                manifest: serde_json::json!({}),
                shasum: "shasum".to_string(),
                integrity: "integrity".to_string(),
                tarball_storage_key: "key".to_string(),
                tarball_size_bytes: 42,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad/versions/1.0.0/dependency-audit"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    /// Counts the calls that would have gone out to npm's advisory service.
    struct CountingAudit(std::sync::atomic::AtomicUsize);

    #[async_trait::async_trait]
    impl artiferris_domain::npm_audit::NpmAuditPort for CountingAudit {
        async fn check(&self, _name: &NpmPackageName, _versions: &[NpmVersion]) -> Result<Vec<artiferris_domain::npm_audit::NpmAdvisory>, artiferris_domain::error::DomainError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(vec![])
        }
        async fn check_bulk_raw(&self, _packages: &std::collections::HashMap<String, Vec<String>>) -> Result<serde_json::Value, artiferris_domain::error::DomainError> {
            unreachable!()
        }
    }

    async fn publish(state: &AppState, repo_id: Uuid, name: &str) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageOrigin, NpmPackageVersion};

        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repo_id, name: NpmPackageName::parse(name).unwrap(), created_at: chrono::Utc::now(), updated_at: chrono::Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        state.npm_packages.create_package(&package).await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("1.0.0").unwrap(),
                manifest: serde_json::json!({}),
                shasum: "s".to_string(),
                integrity: "i".to_string(),
                tarball_storage_key: "k".to_string(),
                tarball_size_bytes: 1,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
    }

    async fn audit_status(app: &axum::Router, repo_id: Uuid, name: &str, token: Option<&str>) -> axum::http::StatusCode {
        let mut request = Request::builder().uri(format!("/api/repositories/{repo_id}/packages/npm/{name}/audit"));
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap().status()
    }

    /// An audit calls npm's advisory service, so an anonymous caller only ever reads a result someone signed in already asked for.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_never_triggers_a_call_to_the_advisory_service(pool: sqlx::PgPool) {
        let mut state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        publish(&state, repo_id, "left-pad").await;
        publish(&state, repo_id, "right-pad").await;
        let audit = Arc::new(CountingAudit(std::sync::atomic::AtomicUsize::new(0)));
        state.audit_npm_package = Arc::new(artiferris_application::use_cases::npm_audit::AuditNpmPackageUseCase::new(state.npm_packages.clone(), audit.clone()));
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);
        let calls = || audit.0.load(std::sync::atomic::Ordering::SeqCst);

        assert_eq!(audit_status(&app, repo_id, "left-pad", None).await, axum::http::StatusCode::UNAUTHORIZED, "nothing cached yet");
        assert_eq!(calls(), 0);
        assert_eq!(audit_status(&app, repo_id, "left-pad", Some(&token)).await, axum::http::StatusCode::OK);
        assert_eq!(audit_status(&app, repo_id, "left-pad", Some(&token)).await, axum::http::StatusCode::OK);
        assert_eq!(calls(), 1, "the second signed-in audit came from the cache");
        assert_eq!(audit_status(&app, repo_id, "left-pad", None).await, axum::http::StatusCode::OK);
        assert_eq!(audit_status(&app, repo_id, "left-pad", None).await, axum::http::StatusCode::OK);
        assert_eq!(audit_status(&app, repo_id, "right-pad", None).await, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(calls(), 1, "anonymous requests never reached the advisory service");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_anonymous_audit_and_dependency_audit_routes_share_the_per_client_budget(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        for _ in 0..super::super::ANONYMOUS_DETAILS_PER_MINUTE {
            audit_status(&app, repo_id, "left-pad", None).await;
        }

        assert_eq!(audit_status(&app, repo_id, "left-pad", None).await, axum::http::StatusCode::TOO_MANY_REQUESTS);
        let dependency_audit = app.clone().oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad/versions/1.0.0/dependency-audit")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(dependency_audit.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_ne!(audit_status(&app, repo_id, "left-pad", Some(&token)).await, axum::http::StatusCode::TOO_MANY_REQUESTS, "a signed-in caller is not limited here");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_signed_in_account_has_an_audit_budget_of_its_own(pool: sqlx::PgPool) {
        let mut state = AppState::build(pool, &test_config());
        let public_org = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let admin_id = state.create_user.execute(public_org, "admin", "sup3r-s3cret!", true).await.unwrap();
        state.create_user.execute(public_org, "other", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(public_org, "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        publish(&state, repo_id, "left-pad").await;
        let audit = Arc::new(CountingAudit(std::sync::atomic::AtomicUsize::new(0)));
        state.audit_npm_package = Arc::new(artiferris_application::use_cases::npm_audit::AuditNpmPackageUseCase::new(state.npm_packages.clone(), audit));
        let admin_token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let other_token = state.authenticate_user.execute("other", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        for _ in 0..super::SIGNED_IN_AUDITS_PER_MINUTE {
            assert_eq!(audit_status(&app, repo_id, "left-pad", Some(&admin_token)).await, axum::http::StatusCode::OK);
        }

        assert_eq!(audit_status(&app, repo_id, "left-pad", Some(&admin_token)).await, axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(audit_status(&app, repo_id, "left-pad", Some(&other_token)).await, axum::http::StatusCode::OK, "the budget is per account");
        assert_eq!(audit_status(&app, repo_id, "left-pad", None).await, axum::http::StatusCode::OK, "anonymous readers of a cached result are not spending it");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_audit_an_npm_package_in_a_private_repository(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        // Deliberately not marked public.
        let app = build_router(state);

        let response = app
            .oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad/audit")).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn viewing_npm_package_details_lists_its_versions_and_dist_tags(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: version.clone(),
                manifest: serde_json::json!({}),
                shasum: "shasum".to_string(),
                integrity: "integrity".to_string(),
                tarball_storage_key: "key".to_string(),
                tarball_size_bytes: 42,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        state.npm_packages.set_dist_tag(package.id, "latest", &version).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "left-pad");
        assert_eq!(json["versions"][0]["version"], "1.0.0");
        assert_eq!(json["versions"][0]["size_bytes"], 42);
        assert_eq!(json["dist_tags"][0]["tag"], "latest");
        assert_eq!(json["dist_tags"][0]["version"], "1.0.0");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_view_a_public_repositorys_npm_package_details(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("1.0.0").unwrap(),
                manifest: serde_json::json!({}),
                shasum: "shasum".to_string(),
                integrity: "integrity".to_string(),
                tarball_storage_key: "key".to_string(),
                tarball_size_bytes: 42,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        let app = build_router(state);

        // No Authorization header at all.
        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "left-pad");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_view_a_private_repositorys_npm_package_details(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "private-npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    // Ground truth against the real npm advisory database.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    #[ignore = "requires network access to registry.npmjs.org"]
    async fn auditing_an_npm_package_reports_known_advisories_for_its_stored_version(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("minimist").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("0.0.8").unwrap(),
                manifest: serde_json::json!({}),
                shasum: "shasum".to_string(),
                integrity: "integrity".to_string(),
                tarball_storage_key: "key".to_string(),
                tarball_size_bytes: 1,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/minimist/audit"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let advisories = json.as_array().unwrap();
        assert!(!advisories.is_empty());
        assert!(advisories.iter().any(|a| a["title"].as_str().unwrap().to_lowercase().contains("prototype pollution")));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn deleting_an_npm_package_version_removes_only_that_version(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        for raw_version in ["1.0.0", "2.0.0"] {
            let tarball_storage_key = format!("left-pad-{raw_version}.tgz");
            state.storage.write(repo_id, &tarball_storage_key, b"tarball bytes").await.unwrap();
            state
                .npm_packages
                .insert_version(&NpmPackageVersion {
                    id: Uuid::new_v4(),
                    npm_package_id: package.id,
                    version: NpmVersion::parse(raw_version).unwrap(),
                    manifest: serde_json::json!({}),
                    shasum: "shasum".to_string(),
                    integrity: "integrity".to_string(),
                    tarball_storage_key,
                    tarball_size_bytes: 1,
                    deprecated: false,
                    deprecated_message: None,
                    published_by: None,
                    published_at: chrono::Utc::now(),
                    origin: NpmPackageOrigin::Local,
                })
                .await
                .unwrap();
        }
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad/versions/1.0.0"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        let remaining = state.npm_packages.list_versions(package.id).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].version.as_str(), "2.0.0");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn deleting_the_whole_npm_package_removes_it_entirely(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        state.storage.write(repo_id, "left-pad-1.0.0.tgz", b"tarball bytes").await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("1.0.0").unwrap(),
                manifest: serde_json::json!({}),
                shasum: "shasum".to_string(),
                integrity: "integrity".to_string(),
                tarball_storage_key: "left-pad-1.0.0.tgz".to_string(),
                tarball_size_bytes: 1,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let repository_id_for_lookup = repo_id;
        let name_for_lookup = NpmPackageName::parse("left-pad").unwrap();
        let app = build_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        assert!(state.npm_packages.find_package(repository_id_for_lookup, &name_for_lookup).await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_read_only_user_cannot_delete_an_npm_package(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id =
            state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state
            .npm_packages
            .create_package(&NpmPackage {
                id: Uuid::new_v4(),
                package_repository_id: repo_id,
                name: NpmPackageName::parse("left-pad").unwrap(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                metadata_fetched_at: None,
                cached_metadata: None,
            })
            .await
            .unwrap();
        let reader_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "reader", "sup3r-s3cret!", false).await.unwrap();
        state.grant_permission.execute(reader_id, repo_id, Role::Read, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("reader", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    /// Representative of all twelve content routes, which share the same org check.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn npm_package_details_in_one_organization_is_not_reachable_by_a_member_of_another(pool: sqlx::PgPool) {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let other_id = state.create_organization.execute("other", "Other Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(acme_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();

        // A real package must exist first, so the assertion below is explained by the org check, not "not found".
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            name: NpmPackageName::parse("left-pad").unwrap(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        state.npm_packages.create_package(&package).await.unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: version.clone(),
                manifest: serde_json::json!({}),
                shasum: "shasum".to_string(),
                integrity: "integrity".to_string(),
                tarball_storage_key: "key".to_string(),
                tarball_size_bytes: 42,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        state.npm_packages.set_dist_tag(package.id, "latest", &version).await.unwrap();
        // Creating a repository doesn't itself grant the creator a role on it.
        state.grant_permission.execute(admin_id, repo_id, Role::Read, admin_id).await.unwrap();

        // Grant as super-admin (only way to cross orgs), then demote — leaves a stale
        // out-of-org grant. A second super-admin so the demotion below isn't rejected.
        state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "another-super-admin", "sup3r-s3cret!", true).await.unwrap();
        let other_user_id = state.create_user.execute(other_id, "other-user", "sup3r-s3cret!", true).await.unwrap();
        state.grant_permission.execute(other_user_id, repo_id, Role::Read, admin_id).await.unwrap();
        state.set_super_admin.execute(other_user_id, false, admin_id).await.unwrap();
        let other_token = state.authenticate_user.execute("other-user", "sup3r-s3cret!").await.unwrap();
        let admin_token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        // Sanity check: acme's own admin can see the package it just created.
        let acme_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad"))
                    .header("host", "acme.artiferris.localhost")
                    .header("authorization", format!("Bearer {admin_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(acme_response.status(), axum::http::StatusCode::OK, "sanity check: the created package must be fetchable by its own organization");

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/repositories/{repo_id}/packages/npm/left-pad"))
                    .header("host", "other.artiferris.localhost")
                    .header("authorization", format!("Bearer {other_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::NOT_FOUND,
            "a valid role grant on a package that genuinely exists must not be enough to cross an organization boundary"
        );
    }

    async fn public_npm_package(state: &AppState, manifest: serde_json::Value) -> Uuid {
        use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "npm-repo", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repo_id, name: NpmPackageName::parse("widget").unwrap(), created_at: chrono::Utc::now(), updated_at: chrono::Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        state.npm_packages.create_package(&package).await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("1.0.0").unwrap(),
                manifest,
                shasum: "s".to_string(),
                integrity: "i".to_string(),
                tarball_storage_key: "k".to_string(),
                tarball_size_bytes: 1,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: chrono::Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        repo_id
    }

    async fn details_json(app: axum::Router, repo_id: Uuid) -> serde_json::Value {
        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/npm/widget")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn package_details_carry_the_readme_already_rendered_and_sanitized(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let readme = "# Widget\n\nUse **it**. [docs](https://example.com)\n\n<script>steal()</script><img src=\"https://example.com/a.png\" onerror=\"steal()\">";
        let repo_id = public_npm_package(&state, serde_json::json!({ "readme": readme })).await;

        let json = details_json(build_router(state), repo_id).await;

        let html = json["readme_html"].as_str().unwrap();
        assert!(html.contains("<h1>Widget</h1>") && html.contains("<strong>it</strong>") && html.contains(r#"rel="nofollow noopener noreferrer ugc""#), "{html}");
        assert!(!html.contains("script") && !html.contains("onerror") && !html.contains("steal"), "{html}");
        assert!(html.contains(r#"src="https://example.com/a.png""#));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn anonymous_callers_of_package_details_are_limited_per_ip_but_signed_in_ones_are_not(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let repo_id = public_npm_package(&state, serde_json::json!({ "readme": "hi" })).await;
        let token = super::super::test_support::bearer(&state, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "reader", "sup3r-s3cret!", false).await;
        let app = build_router(state);
        let get = |token: Option<&str>| {
            let mut request = Request::builder().uri(format!("/api/repositories/{repo_id}/packages/npm/widget"));
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            app.clone().oneshot(request.body(Body::empty()).unwrap())
        };

        for _ in 0..super::super::ANONYMOUS_DETAILS_PER_MINUTE {
            assert_eq!(get(None).await.unwrap().status(), axum::http::StatusCode::OK);
        }

        assert_eq!(get(None).await.unwrap().status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(get(Some(&token)).await.unwrap().status(), axum::http::StatusCode::OK, "a signed-in caller is not counted against the anonymous budget");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_package_without_a_readme_has_null_and_still_names_its_registry(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let repo_id = public_npm_package(&state, serde_json::json!({})).await;

        let json = details_json(build_router(state), repo_id).await;

        assert!(json["readme_html"].is_null());
        assert_eq!(json["registry_url"], "http://acme.artiferris.localhost:4200/npm/npm-repo/");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn package_details_carry_the_weekly_download_count(pool: sqlx::PgPool) {
        use artiferris_domain::download_stats::DownloadCount;

        let state = AppState::build(pool, &test_config());
        let repo_id = public_npm_package(&state, serde_json::json!({})).await;
        state
            .download_stats
            .add_batch(&[DownloadCount { day: chrono::Utc::now().date_naive(), repository_id: repo_id, format: RepositoryFormat::Npm, name: "widget".to_string(), downloads: 7 }])
            .await
            .unwrap();

        let json = details_json(build_router(state), repo_id).await;

        assert_eq!(json["downloads_7d"], 7);
    }
}
