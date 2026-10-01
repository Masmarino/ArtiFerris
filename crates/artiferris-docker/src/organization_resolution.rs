use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use artiferris_domain::organization::{Organization, OrganizationSlug};

use crate::state::DockerState;

/// Copy of `artiferris_api::organization_middleware::ResolvedOrganization` for this crate's `DockerState`: it cannot be
/// shared across the crate boundary.
#[derive(Clone)]
pub struct ResolvedOrganization(pub Organization);

impl FromRequestParts<DockerState> for ResolvedOrganization {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &DockerState) -> Result<Self, Self::Rejection> {
        let host = parts
            .headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // Strip a port if present, and lowercase — an uppercase Host header must resolve
        // the same as its lowercase form.
        let host_without_port = host.split(':').next().unwrap_or(host).to_ascii_lowercase();

        let label = host_without_port
            .strip_suffix(&format!(".{}", state.artiferris_base_domain))
            .unwrap_or("");

        // The image scanner reaches this registry over loopback, which names no organization: it takes the one its
        // scoped token was issued for.
        if is_loopback_host(host) {
            if let Some(organization_id) = scanner_token_organization(parts, state) {
                return state
                    .organizations
                    .find_by_id(organization_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    .map(ResolvedOrganization)
                    .ok_or(StatusCode::NOT_FOUND);
            }
        }

        let org = if label.is_empty() || label == "www" || label == "app" {
            state
                .organizations
                .find_public()
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        } else {
            let slug = OrganizationSlug::parse(label).map_err(|_| StatusCode::NOT_FOUND)?;
            state
                .organizations
                .find_by_slug(&slug)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .ok_or(StatusCode::NOT_FOUND)?
        };

        Ok(ResolvedOrganization(org))
    }
}

/// A `Host` that is a loopback IP literal, with or without a port (`127.0.0.1:8080`, `[::1]:8080`).
fn is_loopback_host(host: &str) -> bool {
    let ip = host
        .parse::<std::net::SocketAddr>()
        .map(|address| address.ip())
        .or_else(|_| host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>());
    ip.is_ok_and(|ip| ip.is_loopback())
}

/// The organization of a valid access token granted against one repository (the scanner's token is one). No token, a
/// token that does not verify, or one without such a grant leaves the `Host` to decide.
fn scanner_token_organization(parts: &Parts, state: &DockerState) -> Option<uuid::Uuid> {
    use axum_extra::headers::{Authorization, HeaderMapExt, authorization::Bearer};
    let Authorization(bearer) = parts.headers.typed_get::<Authorization<Bearer>>()?;
    let claims = state.token_issuer.verify(bearer.token()).ok()?;
    claims.granted_scope.as_ref()?.granted_repository_id?;
    Some(claims.organization_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route_test_support::test_state;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn router(state: DockerState) -> Router {
        async fn handler(ResolvedOrganization(org): ResolvedOrganization) -> String {
            org.slug.as_str().to_string()
        }
        Router::new().route("/", get(handler)).with_state(state)
    }

    async fn create_org(state: &DockerState, slug: &str) {
        state
            .organizations
            .create(&Organization {
                id: uuid::Uuid::new_v4(),
                slug: OrganizationSlug::parse(slug).unwrap(),
                display_name: slug.to_string(),
                is_public: false,
                is_personal: false,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
    }

    async fn resolve(state: DockerState, host: &str) -> axum::response::Response {
        router(state)
            .oneshot(Request::builder().uri("/").header("host", host).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn create_org_with_id(state: &DockerState, slug: &str) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        state
            .organizations
            .create(&Organization { id, slug: OrganizationSlug::parse(slug).unwrap(), display_name: slug.to_string(), is_public: false, is_personal: false, created_at: chrono::Utc::now() })
            .await
            .unwrap();
        id
    }

    fn scanner_token(state: &DockerState, organization_id: uuid::Uuid, repository_id: Option<uuid::Uuid>) -> String {
        let scope = artiferris_domain::docker_registry::DockerGrantedScope {
            resource_type: "repository".to_string(),
            name: "acme-docker/app".to_string(),
            actions: vec!["pull".to_string()],
            granted_repository_id: repository_id,
        };
        state.token_issuer.issue(uuid::Uuid::new_v4(), organization_id, false, Some(scope)).unwrap()
    }

    async fn resolve_with_token(state: DockerState, host: &str, token: &str) -> axum::response::Response {
        router(state)
            .oneshot(Request::builder().uri("/").header("host", host).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn body_of(response: axum::response::Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_loopback_host_with_a_repository_token_resolves_to_the_tokens_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let acme = create_org_with_id(&state, "acme").await;
        let token = scanner_token(&state, acme, Some(uuid::Uuid::new_v4()));

        for host in ["127.0.0.1:8080", "127.0.0.1", "[::1]:8080", "127.0.0.2:8080"] {
            let response = resolve_with_token(state.clone(), host, &token).await;
            assert_eq!(response.status(), StatusCode::OK, "{host}");
            assert_eq!(body_of(response).await, b"acme", "{host}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_loopback_host_without_a_usable_token_still_resolves_to_the_public_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let acme = create_org_with_id(&state, "acme").await;

        let anonymous = resolve(state.clone(), "127.0.0.1:8080").await;
        assert_eq!(body_of(anonymous).await, b"public");
        let forged = resolve_with_token(state.clone(), "127.0.0.1:8080", "not-a-token").await;
        assert_eq!(body_of(forged).await, b"public");
        let ungranted = resolve_with_token(state.clone(), "127.0.0.1:8080", &scanner_token(&state, acme, None)).await;
        assert_eq!(body_of(ungranted).await, b"public", "a token with no repository grant does not choose the organization");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_token_does_not_override_a_host_that_names_an_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let acme = create_org_with_id(&state, "acme").await;
        let token = scanner_token(&state, acme, Some(uuid::Uuid::new_v4()));

        let public_host = resolve_with_token(state.clone(), "artiferris.localhost", &token).await;
        assert_eq!(body_of(public_host).await, b"public");
        let lookalike = resolve_with_token(state, "127.0.0.1.evil.com", &token).await;
        assert_eq!(body_of(lookalike).await, b"public", "only an IP literal counts as loopback");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_loopback_host_with_a_token_for_a_deleted_organization_is_not_found(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let token = scanner_token(&state, uuid::Uuid::new_v4(), Some(uuid::Uuid::new_v4()));

        let response = resolve_with_token(state, "127.0.0.1:8080", &token).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_plain_base_domain_host_resolves_to_the_public_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let response = resolve(state, "artiferris.localhost").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "public".as_bytes());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_www_host_label_resolves_to_the_public_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let response = resolve(state, "www.artiferris.localhost").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "public".as_bytes());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_app_host_label_resolves_to_the_public_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let response = resolve(state, "app.artiferris.localhost").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "public".as_bytes());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_slug_dot_base_domain_host_resolves_to_that_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        create_org(&state, "acme").await;
        let response = resolve(state, "acme.artiferris.localhost").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "acme".as_bytes());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_port_suffix_on_the_host_header_is_stripped(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let response = resolve(state, "artiferris.localhost:8080").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "public".as_bytes());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_port_suffix_on_an_organization_subdomain_is_stripped(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        create_org(&state, "acme").await;
        let response = resolve(state, "acme.artiferris.localhost:8080").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "acme".as_bytes());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_uppercase_host_header_still_resolves_the_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        create_org(&state, "acme").await;
        let response = resolve(state, "ACME.ARTIFERRIS.LOCALHOST").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "acme".as_bytes());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_unknown_organization_slug_is_not_found(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let response = resolve(state, "nope.artiferris.localhost").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// `evilacme` contains `acme` but is another slug: match whole dot-separated labels.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_host_that_merely_contains_another_organizations_slug_is_not_confused_with_it(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        create_org(&state, "acme").await;
        let response = resolve(state, "evilacme.artiferris.localhost").await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a host whose label merely contains another organization's slug must not resolve to that organization"
        );
    }

    /// Anchor the base domain to the end of the host: `acme.artiferris.localhost.evil.com` falls back to public, not to
    /// acme.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_host_where_the_base_domain_appears_as_a_substring_but_not_as_the_final_label_does_not_match(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        create_org(&state, "acme").await;
        let response = resolve(state, "acme.artiferris.localhost.evil.com").await;
        assert_eq!(response.status(), StatusCode::OK, "must not error, but also must not be treated as acme");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            body, "public".as_bytes(),
            "the base domain appearing mid-host (not as the final label) must fall back to public, never match \"acme\""
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_host_that_does_not_end_with_the_base_domain_falls_back_to_the_public_organization(pool: PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(pool, dir.path()).await;
        let response = resolve(state, "totally-unrelated-host.example.com").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "public".as_bytes());
    }
}
