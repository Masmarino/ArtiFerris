use std::net::SocketAddr;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use artiferris_domain::docker_registry::DockerImageName;
use artiferris_domain::permission::Role;
use serde::Serialize;
use uuid::Uuid;

use crate::auth_middleware::AuthUser;
use crate::dto::{application_error_response, ErrorResponse};
use crate::install_location::RepositoryLocation;
use crate::state::AppState;

use super::{load_repository, repository_access_error, require_anonymous_budget, require_readable_repository_access, require_repository_access};

#[derive(Serialize)]
struct DockerTagDetailResponse {
    tag: String,
    digest: String,
    media_type: String,
    created_at: DateTime<Utc>,
    /// Config plus layers; `None` for a manifest list or index.
    size_bytes: Option<i64>,
}

#[derive(Serialize)]
pub(super) struct DockerImageDetailsResponse {
    image_name: String,
    /// Downloads over the last seven days.
    downloads_7d: i64,
    /// The owner's image reference to pull, without scheme or tag.
    image_reference: String,
    /// The most recently updated tags, capped; see `truncated`.
    tags: Vec<DockerTagDetailResponse>,
    truncated: bool,
}

pub(super) async fn get_docker_image_details(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path((id, image)): Path<(Uuid, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<DockerImageDetailsResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_anonymous_budget(&state, &user, &headers, connect_info)?;
    let repo = load_repository(&state, id).await?;
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, id, repo.is_public, "view image details").await.map_err(repository_access_error)?;
    let parsed = DockerImageName::parse(&image).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse::message("invalid image name".to_string()))))?;
    let details = state.get_docker_image_details.execute(id, &parsed).await.map_err(|e| application_error_response("failed to get docker image details", e))?;
    let owner = state.organizations.find_by_id(repo.organization_id).await.ok().flatten().ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::message("internal error".to_string()))))?;
    let image_reference = RepositoryLocation::of(&repo, &owner).image_reference(&details.image_name, &state.public_url, &state.artiferris_base_domain);
    Ok(Json(DockerImageDetailsResponse {
        image_name: details.image_name,
        downloads_7d: details.downloads_7d,
        image_reference,
        truncated: details.truncated,
        tags: details
            .tags
            .into_iter()
            .map(|t| DockerTagDetailResponse { tag: t.tag, digest: t.digest, media_type: t.media_type, created_at: t.created_at, size_bytes: t.size_bytes })
            .collect(),
    }))
}

#[derive(Serialize)]
struct DockerVulnerabilityResponse {
    id: String,
    package_name: String,
    installed_version: String,
    fixed_version: Option<String>,
    severity: String,
    title: Option<String>,
    primary_url: Option<String>,
}

#[derive(Serialize)]
pub(super) struct DockerImageScanResultResponse {
    scanned_at: DateTime<Utc>,
    vulnerabilities: Vec<DockerVulnerabilityResponse>,
}

impl From<artiferris_domain::docker_scan::DockerImageScanResult> for DockerImageScanResultResponse {
    fn from(result: artiferris_domain::docker_scan::DockerImageScanResult) -> Self {
        Self {
            scanned_at: result.scanned_at,
            vulnerabilities: result
                .vulnerabilities
                .into_iter()
                .map(|v| DockerVulnerabilityResponse {
                    id: v.id,
                    package_name: v.package_name,
                    installed_version: v.installed_version,
                    fixed_version: v.fixed_version,
                    severity: v.severity,
                    title: v.title,
                    primary_url: v.primary_url,
                })
                .collect(),
        }
    }
}

pub(super) async fn get_docker_image_scan(
    State(state): State<AppState>,
    user: Option<AuthUser>,
    Path((id, image, tag)): Path<(Uuid, String, String)>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
) -> Result<Json<Option<DockerImageScanResultResponse>>, (StatusCode, Json<ErrorResponse>)> {
    require_anonymous_budget(&state, &user, &headers, connect_info)?;
    let repo = load_repository(&state, id).await?;
    require_readable_repository_access(&state, user.as_ref(), repo.organization_id, id, repo.is_public, "read docker image scan").await.map_err(repository_access_error)?;
    let parsed = DockerImageName::parse(&image).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse::message("invalid image name".to_string()))))?;
    let result = state
        .get_docker_image_scan
        .execute(id, &parsed, &tag)
        .await
        .map_err(|e| application_error_response("failed to read docker image scan", e))?;
    Ok(Json(result.map(DockerImageScanResultResponse::from)))
}

pub(super) async fn scan_docker_image(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, image, tag)): Path<(Uuid, String, String)>,
) -> Result<Json<DockerImageScanResultResponse>, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Write, "run docker image scan").await.map_err(repository_access_error)?;
    let parsed = DockerImageName::parse(&image).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse::message("invalid image name".to_string()))))?;
    let result = state
        .scan_docker_image
        .execute(id, &parsed, &tag, user.id)
        .await
        .map_err(|e| application_error_response("failed to scan docker image", e))?;
    Ok(Json(result.into()))
}

pub(super) async fn delete_docker_tag(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, image, tag)): Path<(Uuid, String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Write, "delete docker tag").await.map_err(repository_access_error)?;
    let parsed = DockerImageName::parse(&image).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse::message("invalid image name".to_string()))))?;
    let manifest = state
        .docker_manifests
        .find_manifest_by_tag(id, &parsed, &tag)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::message("internal error".to_string()))))?
        .ok_or((StatusCode::NOT_FOUND, Json(ErrorResponse::message("tag not found".to_string()))))?;
    state
        .delete_docker_manifest
        .execute(id, &parsed, &manifest.digest, user.id)
        .await
        .map_err(|e| application_error_response("failed to delete docker tag", e))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn delete_docker_image(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, image)): Path<(Uuid, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let repo = load_repository(&state, id).await?;
    require_repository_access(&state, &user, repo.organization_id, id, repo.is_public, Role::Write, "delete docker image").await.map_err(repository_access_error)?;
    let parsed = DockerImageName::parse(&image).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse::message("invalid image name".to_string()))))?;
    state.delete_docker_image.execute(id, &parsed, user.id).await.map_err(|e| application_error_response("failed to delete docker image", e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_support::test_config;
    use crate::{build_router, state::AppState};
    use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;


    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_read_the_docker_image_scan_for_a_public_repository(pool: sqlx::PgPool) {
        use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerMediaType};
        use artiferris_domain::docker_scan::{DockerImageScanRepositoryPort, DockerImageScanResult, DockerVulnerability};
        use artiferris_infrastructure::postgres::docker_image_scan_repository::PostgresDockerImageScanRepository;

        let state = AppState::build(pool.clone(), &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let image_name = DockerImageName::parse("my-app").unwrap();
        let manifest = DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            image_name: image_name.clone(),
            digest: Digest::of(b"{}"),
            media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(),
            created_at: chrono::Utc::now(),
        };
        state.docker_manifests.insert_manifest(&manifest, &[]).await.unwrap();
        state.docker_manifests.set_tag(repo_id, &image_name, "latest", manifest.id).await.unwrap();
        let scans = PostgresDockerImageScanRepository::new(pool);
        scans
            .save(&DockerImageScanResult {
                id: Uuid::new_v4(),
                docker_manifest_id: manifest.id,
                scanned_at: chrono::Utc::now(),
                vulnerabilities: vec![DockerVulnerability {
                    id: "CVE-2024-0001".to_string(),
                    package_name: "openssl".to_string(),
                    installed_version: "1.0.0".to_string(),
                    fixed_version: Some("1.1.0".to_string()),
                    severity: "HIGH".to_string(),
                    title: None,
                    primary_url: None,
                }],
            })
            .await
            .unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages/docker/my-app/tags/latest/scan"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["vulnerabilities"][0]["id"], "CVE-2024-0001");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_read_the_docker_image_scan_for_a_private_repository(pool: sqlx::PgPool) {
        use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerMediaType};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        // Deliberately not marked public.
        let image_name = DockerImageName::parse("my-app").unwrap();
        let manifest = DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            image_name: image_name.clone(),
            digest: Digest::of(b"{}"),
            media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(),
            created_at: chrono::Utc::now(),
        };
        state.docker_manifests.insert_manifest(&manifest, &[]).await.unwrap();
        state.docker_manifests.set_tag(repo_id, &image_name, "latest", manifest.id).await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages/docker/my-app/tags/latest/scan"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn anonymous_callers_of_image_details_are_limited_per_ip_but_signed_in_ones_are_not(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "images", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);
        let get = |token: Option<&str>| {
            let mut request = Request::builder().uri(format!("/api/repositories/{repo_id}/packages/docker/my-app"));
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            app.clone().oneshot(request.body(Body::empty()).unwrap())
        };

        for _ in 0..super::super::ANONYMOUS_DETAILS_PER_MINUTE {
            assert_eq!(get(None).await.unwrap().status(), axum::http::StatusCode::OK);
        }

        assert_eq!(get(None).await.unwrap().status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(get(Some(&token)).await.unwrap().status(), axum::http::StatusCode::OK);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn viewing_docker_image_details_resolves_each_tags_digest(pool: sqlx::PgPool) {
        use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerMediaType};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        let image_name = DockerImageName::parse("my-app").unwrap();
        let manifest = DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            image_name: image_name.clone(),
            digest: Digest::of(b"{}"),
            media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(),
            created_at: chrono::Utc::now(),
        };
        state.docker_manifests.insert_manifest(&manifest, &[]).await.unwrap();
        state.docker_manifests.set_tag(repo_id, &image_name, "latest", manifest.id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repositories/{repo_id}/packages/docker/my-app"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["image_name"], "my-app");
        assert_eq!(json["tags"][0]["tag"], "latest");
        assert_eq!(json["tags"][0]["digest"], manifest.digest.as_str());
        assert_eq!(json["tags"][0]["media_type"], "application/vnd.docker.distribution.manifest.v2+json");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_can_view_a_public_repositorys_docker_image_details(pool: sqlx::PgPool) {
        use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerMediaType};

        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(acme_id, "docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let image_name = DockerImageName::parse("my-app").unwrap();
        let manifest = DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            image_name: image_name.clone(),
            digest: Digest::of(b"{}"),
            media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(),
            created_at: chrono::Utc::now(),
        };
        state.docker_manifests.insert_manifest(&manifest, &[]).await.unwrap();
        state.docker_manifests.set_tag(repo_id, &image_name, "latest", manifest.id).await.unwrap();
        let app = build_router(state);

        // No Authorization header at all.
        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/docker/my-app")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["image_name"], "my-app");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_anonymous_caller_cannot_view_a_private_repositorys_docker_image_details(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "private-docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/docker/my-app")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn deleting_a_docker_tag_removes_its_manifest(pool: sqlx::PgPool) {
        use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerMediaType};

        let state = AppState::build(pool, &test_config());
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state
            .create_repository
            .execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id)
            .await
            .unwrap();
        let image_name = DockerImageName::parse("my-app").unwrap();
        let manifest = DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repo_id,
            image_name: image_name.clone(),
            digest: Digest::of(b"{}"),
            media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(),
            created_at: chrono::Utc::now(),
        };
        state.docker_manifests.insert_manifest(&manifest, &[]).await.unwrap();
        state.docker_manifests.set_tag(repo_id, &image_name, "latest", manifest.id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let repository_id_for_lookup = repo_id;
        let image_name_for_lookup = image_name.clone();
        let digest_for_lookup = manifest.digest.clone();
        let app = build_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/repositories/{repo_id}/packages/docker/my-app/tags/latest"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        assert!(state
            .docker_manifests
            .find_manifest_by_digest(repository_id_for_lookup, &image_name_for_lookup, &digest_for_lookup)
            .await
            .unwrap()
            .is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn image_details_carry_tag_sizes_and_the_owners_pull_reference(pool: sqlx::PgPool) {
        use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerMediaType};

        let state = AppState::build(pool, &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let image_name = DockerImageName::parse("my-app").unwrap();
        for (tag, body) in [("1.0", br#"{"config":{"size":10},"layers":[{"size":90},{"size":400}]}"#.to_vec()), ("index", br#"{"manifests":[]}"#.to_vec())] {
            let manifest = DockerManifest { id: Uuid::new_v4(), package_repository_id: repo_id, image_name: image_name.clone(), digest: Digest::of(&body), media_type: DockerMediaType::DockerV2Manifest, body, created_at: chrono::Utc::now() };
            state.docker_manifests.insert_manifest(&manifest, &[]).await.unwrap();
            state.docker_manifests.set_tag(repo_id, &image_name, tag, manifest.id).await.unwrap();
        }
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/docker/my-app")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(json["image_reference"], "acme.artiferris.localhost:4200/docker-repo/my-app");
        let sizes: std::collections::HashMap<&str, Option<i64>> = json["tags"].as_array().unwrap().iter().map(|t| (t["tag"].as_str().unwrap(), t["size_bytes"].as_i64())).collect();
        assert_eq!(sizes, std::collections::HashMap::from([("1.0", Some(500)), ("index", None)]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn image_details_list_the_newest_hundred_tags_of_an_image_with_three_hundred(pool: sqlx::PgPool) {
        use artiferris_domain::docker_registry::{Digest, DockerImageName, DockerManifest, DockerMediaType};

        let state = AppState::build(pool.clone(), &test_config());
        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin_id = state.create_user.execute(acme_id, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(acme_id, "docker-repo", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let image_name = DockerImageName::parse("my-app").unwrap();
        for i in 0..300 {
            let body = format!(r#"{{"config":{{"size":0}},"layers":[{{"size":{i}}}]}}"#).into_bytes();
            let manifest = DockerManifest { id: Uuid::new_v4(), package_repository_id: repo_id, image_name: image_name.clone(), digest: Digest::of(&body), media_type: DockerMediaType::DockerV2Manifest, body, created_at: chrono::Utc::now() };
            state.docker_manifests.insert_manifest(&manifest, &[]).await.unwrap();
            state.docker_manifests.set_tag(repo_id, &image_name, &format!("t{i:03}"), manifest.id).await.unwrap();
        }
        sqlx::query("UPDATE docker_tags SET updated_at = now() - make_interval(secs => 1000 - substr(tag, 2)::int) WHERE package_repository_id = $1").bind(repo_id).execute(&pool).await.unwrap();
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri(format!("/api/repositories/{repo_id}/packages/docker/my-app")).body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
        let tags = json["tags"].as_array().unwrap();
        assert_eq!((tags.len(), json["truncated"].as_bool()), (100, Some(true)));
        assert_eq!((tags[0]["tag"].as_str(), tags[99]["tag"].as_str()), (Some("t200"), Some("t299")));
        assert_eq!(tags[0]["size_bytes"].as_i64(), Some(200));
    }
}
