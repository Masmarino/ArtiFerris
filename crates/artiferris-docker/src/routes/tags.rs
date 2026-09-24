use artiferris_application::use_cases::docker_list::MAX_TAGS_PER_PAGE;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::Json;
use artiferris_domain::docker_registry::DockerImageName;
use artiferris_domain::package_repository::PackageRepositorySummary;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::DockerAuthUser;
use crate::authz::{member_is_readable, require_docker_repository, require_granted_action_for_route, require_readable_repository_by_name};
use crate::errors::{docker_authz_error, docker_error, docker_error_response};
use crate::state::DockerState;

/// The spec's `n` and `last` query parameters. `n` stays text so a bad one can be answered in the registry's own error format.
#[derive(Deserialize, Default)]
pub struct TagsQueryParams {
    n: Option<String>,
    last: Option<String>,
}

pub async fn list_tags(
    state: DockerState,
    organization_id: Uuid,
    repository_name: String,
    image_name_str: String,
    user: Option<DockerAuthUser>,
    query: TagsQueryParams,
    location_base: String,
) -> Response {
    let (repo, caller) = match require_readable_repository_by_name(&state, user.as_ref(), organization_id, &repository_name).await {
        Ok(found) => found,
        Err(status) => return docker_authz_error(status),
    };
    if let Err(status) = require_docker_repository(&repo) {
        return docker_authz_error(status);
    }
    if let Some((caller, is_personal)) = caller {
        if let Err(status) = require_granted_action_for_route(caller, repo.id, &repository_name, "pull", is_personal) {
            return docker_authz_error(status);
        }
    }
    let Ok(image_name) = DockerImageName::parse(&image_name_str) else {
        return docker_error(StatusCode::BAD_REQUEST, "NAME_INVALID", "invalid image name").into_response();
    };
    let limit = match query.n.as_deref().map(str::parse::<usize>) {
        None => MAX_TAGS_PER_PAGE,
        Some(Ok(n)) => n.min(MAX_TAGS_PER_PAGE),
        Some(Err(_)) => return docker_error(StatusCode::BAD_REQUEST, "PAGINATION_NUMBER_INVALID", "n must be a non-negative integer").into_response(),
    };

    // Same closure shape as `manifests.rs::get_manifest` — deliberately `user`, not `caller`: `caller`
    // is `None` for a public top-level repository, and a caller who did authenticate must not lose
    // their own organization's members because the group wrapping them happens to be public.
    let caller_user = user.as_ref();
    let top_level_organization_id = repo.organization_id;
    let top_level_was_authorized = caller.is_some();
    let state_ref = &state;
    match state
        .list_tags
        .execute_page(repo.id, &image_name, limit, query.last.as_deref(), move |member: &PackageRepositorySummary| {
            member_is_readable(state_ref, caller_user, top_level_organization_id, top_level_was_authorized, member.id, member.organization_id, member.is_public)
        })
        .await
    {
        Ok(page) => {
            let mut response = Json(json!({ "name": image_name_str, "tags": page.tags })).into_response();
            if let (true, Some(last)) = (page.has_more, page.tags.last()) {
                let next = format!("<{location_base}/{image_name_str}/tags/list?n={limit}&last={last}>; rel=\"next\"");
                if let Ok(value) = HeaderValue::from_str(&next) {
                    response.headers_mut().insert(header::LINK, value);
                }
            }
            response
        }
        Err(e) => docker_error_response(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use artiferris_domain::docker_registry::Digest;
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::route_test_support::{issue_test_token, seed_bare_user, seed_repository, test_state};

    async fn push_config_blob(app: &axum::Router, repo_name: &str, push_token: &str, config_bytes: &[u8]) -> Digest {
        let digest = Digest::of(config_bytes);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{repo_name}/myimage/blobs/uploads/?digest={}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .body(Body::from(config_bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        digest
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn lists_tags_for_a_pushed_image(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let config_digest = push_config_blob(&app, &repo_name, &push_token, b"tags-list-config-bytes").await;
        let manifest_body = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "config": { "digest": config_digest.as_str() },
            "layers": []
        }))
        .unwrap();

        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/v1"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(manifest_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/tags/list"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["tags"], serde_json::json!(["v1"]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn tags_are_listed_a_page_at_a_time_with_a_link_to_the_next(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        let state = test_state(pool.clone(), dir.path()).await;
        let push_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["push"]);
        let pull_token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, repository_id, &repo_name, "myimage", &["pull"]);
        let app = crate::router(state);
        let config_digest = push_config_blob(&app, &repo_name, &push_token, b"tags-page-config-bytes").await;
        let manifest_body = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "config": { "digest": config_digest.as_str() },
            "layers": []
        }))
        .unwrap();
        let put_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/{repo_name}/myimage/manifests/v1"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {push_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/vnd.docker.distribution.manifest.v2+json")
                    .body(Body::from(manifest_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_response.status(), StatusCode::CREATED);
        for tag in ["v2", "v3", "v4"] {
            sqlx::query!("INSERT INTO docker_tags (package_repository_id, image_name, tag, manifest_id) SELECT $1, 'myimage', $2, id FROM docker_manifests WHERE package_repository_id = $1", repository_id, tag)
                .execute(&pool)
                .await
                .unwrap();
        }
        let list = |query: &'static str| {
            let app = app.clone();
            let request = Request::builder()
                .method("GET")
                .uri(format!("/{repo_name}/myimage/tags/list{query}"))
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {pull_token}"))
                .body(Body::empty())
                .unwrap();
            async move {
                let response = app.oneshot(request).await.unwrap();
                let status = response.status();
                let link = response.headers().get(axum::http::header::LINK).map(|value| value.to_str().unwrap().to_string());
                let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
                (status, link, body)
            }
        };

        let (status, link, body) = list("?n=2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["tags"], serde_json::json!(["v1", "v2"]));
        assert_eq!(link, Some(format!("</v2/{repo_name}/myimage/tags/list?n=2&last=v2>; rel=\"next\"")));

        let (_, link, body) = list("?n=2&last=v2").await;
        assert_eq!(body["tags"], serde_json::json!(["v3", "v4"]));
        assert_eq!(link, None, "that was the last page");

        let (_, link, body) = list("").await;
        assert_eq!(body["tags"], serde_json::json!(["v1", "v2", "v3", "v4"]));
        assert_eq!(link, None);

        let (status, _, _) = list("?n=lots").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn listing_tags_with_a_token_scoped_to_a_different_repository_is_forbidden(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let repository_id = Uuid::new_v4();
        seed_repository(&pool, PUBLIC_ORGANIZATION_ID, repository_id, "docker", "hosted").await;
        let repo_name = format!("repo-{repository_id}");
        let state = test_state(pool.clone(), dir.path()).await;
        let token = issue_test_token(&state, seed_bare_user(&pool, PUBLIC_ORGANIZATION_ID).await, Uuid::new_v4(), "some-other-repo", "myimage", &["pull"]);
        let app = crate::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/{repo_name}/myimage/tags/list"))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
