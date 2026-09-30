use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use artiferris_application::error::ApplicationError;
use artiferris_application::use_cases::list_readable_repositories::ReadableRepositoriesCaller;
use artiferris_application::use_cases::public_catalog::{format_key, RawCatalogSearch, RawSuggestion};
use uuid::Uuid;
use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
use artiferris_domain::public_catalog::{CatalogEntry, CatalogMatch, OwnerKind};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth_middleware::AuthUser;
use crate::dto::{application_error_response, ErrorResponse};
use crate::install_location::RepositoryLocation;
use crate::routes::auth::peer_ip_bucket;
use crate::state::AppState;

/// Anyone may call these, so a per-IP request budget keeps one client from hammering the catalog query.
const REQUESTS_PER_WINDOW: usize = 60;
/// Typing fires a request per pause, so suggestions get their own, larger budget: fast typing must not eat the search budget, nor the other way round.
const SUGGESTIONS_PER_WINDOW: usize = 120;
const WINDOW: Duration = Duration::from_secs(60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/public/catalogs", get(list_catalogs))
        .route("/api/public/search", get(search_catalog))
        .route("/api/search", get(search_readable))
        .route("/api/public/suggest", get(suggest))
        .route("/api/public/owners/{kind}/{slug}", get(get_owner))
}

const BUSY_RETRY_AFTER_SECONDS: u64 = 5;

/// Not `Result`-shaped on purpose: every response of this module, errors included, must carry `no-store`
/// so a repository turned private stops appearing immediately.
fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if response.status() == StatusCode::SERVICE_UNAVAILABLE {
        response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(BUSY_RETRY_AFTER_SECONDS));
    }
    response
}

fn over_budget(state: &AppState, headers: &HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> Option<Response> {
    over_budget_for(state, headers, connect_info, "public-catalog", REQUESTS_PER_WINDOW)
}

/// Spends one request of the client's budget for `scope`; false once it is used up.
pub(crate) fn within_budget(state: &AppState, headers: &HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>, scope: &str, limit: usize) -> bool {
    spend_budget(state, &format!("{scope}:{}", peer_ip_bucket(state, headers, connect_info)), limit)
}

/// Spends one request of the budget under `key`; false once it is used up.
pub(crate) fn spend_budget(state: &AppState, key: &str, limit: usize) -> bool {
    if state.public_throttle.is_throttled(key, limit, WINDOW) {
        return false;
    }
    // Every request spends budget, not only failed ones.
    state.public_throttle.record_failure(key, limit, WINDOW);
    true
}

fn over_budget_for(
    state: &AppState,
    headers: &HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    scope: &str,
    limit: usize,
) -> Option<Response> {
    if within_budget(state, headers, connect_info, scope, limit) {
        return None;
    }
    let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(ErrorResponse::message("too many requests, try again shortly".to_string()))).into_response();
    response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(WINDOW.as_secs()));
    Some(no_store(response))
}

#[derive(Serialize)]
struct CatalogResponse {
    format: &'static str,
    name: &'static str,
    label: &'static str,
    entry_count: i64,
}

async fn list_catalogs(State(state): State<AppState>, headers: HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> Response {
    if let Some(rejected) = over_budget(&state, &headers, connect_info) {
        return rejected;
    }
    let response = match state.list_catalogs.execute().await {
        Ok(summaries) => Json(
            summaries
                .into_iter()
                .map(|s| CatalogResponse { format: format_key(s.spec.format), name: s.spec.catalog_name, label: s.spec.label, entry_count: s.entry_count })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => application_error_response("failed to list catalogs", e).into_response(),
    };
    no_store(response)
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    format: Option<String>,
    owner: Option<String>,
    sort: Option<String>,
    page: Option<String>,
    per_page: Option<String>,
}

fn number(name: &str, raw: Option<String>) -> Result<Option<u32>, ApplicationError> {
    raw.filter(|r| !r.is_empty())
        .map(|r| r.parse::<u32>().map_err(|_| ApplicationError::InvalidCatalogQuery(format!("{name} must be a positive number"))))
        .transpose()
}

#[derive(Serialize)]
struct OwnerResponse {
    kind: &'static str,
    slug: String,
    display_name: String,
}

#[derive(Serialize)]
struct RepositoryRefResponse {
    name: String,
    /// Only for a signed-in caller's search: the repository's id and whether its content is hosted or cached from an upstream.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo_type: Option<&'static str>,
}

#[derive(Serialize)]
struct EntryResponse {
    kind: &'static str,
    name: String,
    description: Option<String>,
    keywords: Vec<String>,
    latest: Option<String>,
    updated_at: DateTime<Utc>,
    downloads_7d: i64,
    match_kind: Option<&'static str>,
    repository: RepositoryRefResponse,
    owner: OwnerResponse,
    /// npm entries: the registry to install from.
    registry_url: Option<String>,
    /// Docker entries: the image reference to pull, without the tag.
    image_reference: Option<String>,
}

#[derive(Serialize)]
struct SearchResponse {
    items: Vec<EntryResponse>,
    total: i64,
    page: u32,
    per_page: u32,
}

/// The signed-in counterpart of `/api/public/search`: same parameters, ranking and shape, over everything the caller can read.
async fn search_readable(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Query(params): Query<SearchParams>,
) -> Response {
    if let Some(rejected) = over_budget_for(&state, &headers, connect_info, "readable-search", SUGGESTIONS_PER_WINDOW) {
        return rejected;
    }
    let raw = match (number("page", params.page), number("per_page", params.per_page)) {
        (Ok(page), Ok(per_page)) => RawCatalogSearch { text: params.q, format: params.format, owner: params.owner, sort: params.sort, page, per_page },
        (Err(e), _) | (_, Err(e)) => return no_store(application_error_response("invalid catalog search", e).into_response()),
    };
    let caller = ReadableRepositoriesCaller { user_id: user.id, organization_id: user.organization_id, is_super_admin: user.is_super_admin, is_organization_admin: user.is_organization_admin };
    let response = match state.search_readable_catalog.execute(&caller, raw).await {
        Ok(result) => Json(SearchResponse {
            items: result
                .page
                .items
                .iter()
                .map(|entry| {
                    let mut response = entry_response(entry, &state.public_url, &state.artiferris_base_domain);
                    response.repository.id = Some(entry.repository_id);
                    response.repository.repo_type = Some(match entry.repository_type {
                        RepositoryType::Proxy => "proxy",
                        _ => "hosted",
                    });
                    response
                })
                .collect(),
            total: result.page.total,
            page: result.current_page,
            per_page: result.per_page,
        })
        .into_response(),
        Err(e) => application_error_response("failed to search", e).into_response(),
    };
    no_store(response)
}

async fn search_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Query(params): Query<SearchParams>,
) -> Response {
    if let Some(rejected) = over_budget(&state, &headers, connect_info) {
        return rejected;
    }
    let raw = match (number("page", params.page), number("per_page", params.per_page)) {
        (Ok(page), Ok(per_page)) => RawCatalogSearch { text: params.q, format: params.format, owner: params.owner, sort: params.sort, page, per_page },
        (Err(e), _) | (_, Err(e)) => return no_store(application_error_response("invalid catalog search", e).into_response()),
    };
    let response = match state.search_public_catalog.execute(raw).await {
        Ok(result) => Json(SearchResponse {
            items: result.page.items.iter().map(|entry| entry_response(entry, &state.public_url, &state.artiferris_base_domain)).collect(),
            total: result.page.total,
            page: result.current_page,
            per_page: result.per_page,
        })
        .into_response(),
        Err(e) => application_error_response("failed to search the public catalog", e).into_response(),
    };
    no_store(response)
}

#[derive(Deserialize)]
struct SuggestParams {
    q: Option<String>,
    format: Option<String>,
    owner: Option<String>,
    limit: Option<String>,
}

#[derive(Serialize)]
struct SuggestionResponse {
    kind: &'static str,
    name: String,
    repository: RepositoryRefResponse,
    owner: OwnerResponse,
}

async fn suggest(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Query(params): Query<SuggestParams>,
) -> Response {
    if let Some(rejected) = over_budget_for(&state, &headers, connect_info, "public-suggest", SUGGESTIONS_PER_WINDOW) {
        return rejected;
    }
    let limit = match number("limit", params.limit) {
        Ok(limit) => limit,
        Err(e) => return no_store(application_error_response("invalid catalog suggestion", e).into_response()),
    };
    let raw = RawSuggestion { text: params.q.unwrap_or_default(), format: params.format, owner: params.owner, limit };
    let response = match state.suggest_public_catalog.execute(raw).await {
        Ok(suggestions) => Json(
            suggestions
                .into_iter()
                .map(|s| SuggestionResponse {
                    kind: format_key(s.format),
                    name: s.name,
                    repository: RepositoryRefResponse { name: s.repository_name, id: None, repo_type: None },
                    owner: OwnerResponse { kind: owner_kind_key(s.owner.kind), slug: s.owner.slug, display_name: s.owner.display_name },
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => application_error_response("failed to suggest catalog entries", e).into_response(),
    };
    no_store(response)
}

#[derive(Serialize)]
struct OwnerSummaryResponse {
    kind: &'static str,
    slug: String,
    display_name: String,
    repository_count: i64,
    package_count: i64,
    image_count: i64,
}

/// 404 for an owner with no public repository and for one that doesn't exist, the same answer for both.
async fn get_owner(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    Path((kind, slug)): Path<(String, String)>,
) -> Response {
    if let Some(rejected) = over_budget(&state, &headers, connect_info) {
        return rejected;
    }
    let response = match state.get_owner_summary.execute(&kind, &slug).await {
        Ok(Some(summary)) => Json(OwnerSummaryResponse {
            kind: owner_kind_key(summary.kind),
            slug: summary.slug,
            display_name: summary.display_name,
            repository_count: summary.repository_count,
            package_count: summary.package_count,
            image_count: summary.image_count,
        })
        .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => application_error_response("failed to load the owner summary", e).into_response(),
    };
    no_store(response)
}

fn owner_kind_key(kind: OwnerKind) -> &'static str {
    match kind {
        OwnerKind::Personal => "personal",
        OwnerKind::Organization => "organization",
    }
}

fn entry_response(entry: &CatalogEntry, public_url: &str, base_domain: &str) -> EntryResponse {
    let (registry_url, image_reference) = install_locations(entry, public_url, base_domain);
    EntryResponse {
        kind: format_key(entry.format),
        name: entry.name.clone(),
        description: entry.description.clone(),
        keywords: entry.keywords.clone(),
        latest: entry.latest.clone(),
        updated_at: entry.updated_at,
        downloads_7d: entry.downloads_7d,
        match_kind: entry.match_kind.map(|m| match m {
            CatalogMatch::Exact => "exact",
            CatalogMatch::Prefix => "prefix",
            CatalogMatch::Contains => "contains",
            CatalogMatch::Text => "text",
            CatalogMatch::Fuzzy => "fuzzy",
        }),
        repository: RepositoryRefResponse { name: entry.repository_name.clone(), id: None, repo_type: None },
        owner: OwnerResponse {
            kind: owner_kind_key(entry.owner.kind),
            slug: entry.owner.slug.clone(),
            display_name: entry.owner.display_name.clone(),
        },
        registry_url,
        image_reference,
    }
}

/// Where a client installs from: the owner's own URL, never the catalog.
fn install_locations(entry: &CatalogEntry, public_url: &str, base_domain: &str) -> (Option<String>, Option<String>) {
    let location = RepositoryLocation {
        owner_kind: entry.owner.kind,
        owner_slug: entry.owner.slug.clone(),
        is_public_organization: entry.owner.is_public_organization,
        repository_name: entry.repository_name.clone(),
    };
    match entry.format {
        RepositoryFormat::Npm => (Some(location.registry_url(public_url, base_domain)), None),
        RepositoryFormat::Docker => (None, Some(location.image_reference(&entry.name, public_url, base_domain))),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use artiferris_domain::public_catalog::OwnerRef;
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use artiferris_domain::package_repository::RepositoryType;
    use artiferris_domain::public_catalog::{CatalogOwner, CatalogEntry};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::build_router;
    use crate::routes::repositories::test_support::test_config;

    fn entry(format: RepositoryFormat, owner: CatalogOwner) -> CatalogEntry {
        CatalogEntry {
            format,
            name: if format == RepositoryFormat::Npm { "left-pad".to_string() } else { "api".to_string() },
            description: None,
            keywords: vec![],
            latest: None,
            updated_at: Utc::now(),
            downloads_7d: 0,
            match_kind: None,
            repository_id: Uuid::nil(),
            repository_type: RepositoryType::Hosted,
            repository_name: "repo".to_string(),
            owner,
        }
    }

    fn owner(kind: OwnerKind, slug: &str, is_public_organization: bool) -> CatalogOwner {
        CatalogOwner { kind, slug: slug.to_string(), display_name: slug.to_string(), is_public_organization }
    }

    #[test]
    fn a_personal_repository_installs_from_the_owner_path_on_the_main_host() {
        let personal = owner(OwnerKind::Personal, "alice", false);
        assert_eq!(install_locations(&entry(RepositoryFormat::Npm, personal.clone()), "https://reg.example.com", "reg.example.com"), (Some("https://reg.example.com/npm/u/alice/repo/".to_string()), None));
        assert_eq!(install_locations(&entry(RepositoryFormat::Docker, personal), "https://reg.example.com", "reg.example.com"), (None, Some("reg.example.com/u/alice/repo/api".to_string())));
    }

    #[test]
    fn the_public_organization_is_served_on_the_main_host() {
        let public = owner(OwnerKind::Organization, "public", true);
        assert_eq!(install_locations(&entry(RepositoryFormat::Npm, public.clone()), "http://localhost:4200", "localhost"), (Some("http://localhost:4200/npm/repo/".to_string()), None));
        assert_eq!(install_locations(&entry(RepositoryFormat::Docker, public), "http://localhost:4200", "localhost"), (None, Some("localhost:4200/repo/api".to_string())));
    }

    #[test]
    fn another_organization_is_served_on_its_own_subdomain_keeping_scheme_and_port() {
        let acme = owner(OwnerKind::Organization, "acme", false);
        assert_eq!(install_locations(&entry(RepositoryFormat::Npm, acme.clone()), "http://localhost:4200", "localhost"), (Some("http://acme.localhost:4200/npm/repo/".to_string()), None));
        assert_eq!(install_locations(&entry(RepositoryFormat::Docker, acme), "https://reg.example.com/", "reg.example.com"), (None, Some("acme.reg.example.com/repo/api".to_string())));
    }

    async fn get(app: axum::Router, uri: &str) -> (StatusCode, HeaderMap, serde_json::Value) {
        let response = app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
    }

    async fn publish_npm(state: &AppState, repository_id: Uuid, name: &str, description: &str) {
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repository_id, name: NpmPackageName::parse(name).unwrap(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        state.npm_packages.create_package(&package).await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("1.2.3").unwrap(),
                manifest: serde_json::json!({ "description": description, "keywords": ["pad"] }),
                shasum: "s".to_string(),
                integrity: "i".to_string(),
                tarball_storage_key: "k".to_string(),
                tarball_size_bytes: 1,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
    }

    async fn seeded_app(pool: sqlx::PgPool) -> axum::Router {
        let state = AppState::build(pool, &test_config());
        let acme = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin = state.create_user.execute(acme, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let open = state.create_repository.execute(acme, "open-npm", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        state.set_repository_visibility.execute(open, true, admin).await.unwrap();
        let closed = state.create_repository.execute(acme, "closed-npm", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        publish_npm(&state, open, "left-pad", "pads strings on the left").await;
        publish_npm(&state, closed, "secret-pad", "never listed").await;
        let alice = state.create_user.execute(PUBLIC_ORGANIZATION_ID, "alice", "sup3r-s3cret!", false).await.unwrap();
        state.reserve_personal_organization.execute(alice).await.unwrap();
        let personal = state.create_user_project.execute(alice, "my-lib", RepositoryFormat::Npm, RepositoryType::Hosted).await.unwrap();
        state.set_repository_visibility.execute(personal, true, alice).await.unwrap();
        publish_npm(&state, personal, "pad-lib", "alice's pad").await;
        build_router(state)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn anonymous_search_returns_only_public_entries_with_owner_urls(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;

        let (status, headers, json) = get(app, "/api/public/search?q=pad").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        let items = json["items"].as_array().unwrap();
        let names: Vec<_> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["pad-lib", "left-pad"], "prefix matches rank before substring matches, and secret-pad lives in a private repository");
        assert_eq!(json["total"], 2);
        assert_eq!((json["page"].as_u64(), json["per_page"].as_u64()), (Some(1), Some(20)));
        let left_pad = &items[1];
        assert_eq!(left_pad["owner"], serde_json::json!({ "kind": "organization", "slug": "acme", "display_name": "Acme Corp" }));
        assert_eq!(left_pad["repository"]["name"], "open-npm");
        assert_eq!(left_pad["registry_url"], "http://acme.artiferris.localhost:4200/npm/open-npm/");
        assert_eq!((left_pad["latest"].as_str(), left_pad["match_kind"].as_str()), (Some("1.2.3"), Some("contains")));
        assert_eq!(items[0]["registry_url"], "http://localhost:4200/npm/u/alice/my-lib/");
        assert_eq!(items[0]["owner"]["kind"], "personal");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn listing_the_catalogs_names_every_format_with_its_public_count(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;

        let (status, headers, json) = get(app, "/api/public/catalogs").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            json,
            serde_json::json!([
                { "format": "npm", "name": "artiferris-npm", "label": "npm", "entry_count": 2 },
                { "format": "docker", "name": "artiferris-docker", "label": "Docker", "entry_count": 0 },
            ])
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn invalid_input_is_a_400_with_no_store(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;
        let too_long = "a".repeat(101);
        for uri in ["/api/public/search?format=helm", "/api/public/search?sort=trending", "/api/public/search?page=0", "/api/public/search?page=abc", "/api/public/search?per_page=51", "/api/public/search?q=%00", "/api/public/search?q=a%00b&sort=popular", "/api/public/search?q=line%0Abreak", &format!("/api/public/search?q={too_long}")] {
            let (status, headers, json) = get(app.clone(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{uri}");
            assert!(json["error"].is_string(), "{uri}");
        }
    }

    /// A catalog that refuses every query, the way a saturated one does.
    struct BusyCatalog;

    #[async_trait::async_trait]
    impl artiferris_domain::public_catalog::PublicCatalogPort for BusyCatalog {
        async fn search(&self, _query: &artiferris_domain::public_catalog::CatalogQuery) -> Result<artiferris_domain::public_catalog::CatalogPage, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("the public catalog is busy".to_string()))
        }
        async fn entry_counts(&self) -> Result<Vec<artiferris_domain::public_catalog::CatalogEntryCount>, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("the public catalog is busy".to_string()))
        }
        async fn suggest(&self, _query: &artiferris_domain::public_catalog::SuggestQuery) -> Result<Vec<artiferris_domain::public_catalog::CatalogSuggestion>, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("the public catalog is busy".to_string()))
        }
        async fn repository(&self, _owner: &OwnerRef, _repository: &str) -> Result<Option<artiferris_domain::public_catalog::CatalogRepository>, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("the public catalog is busy".to_string()))
        }
        async fn find_entry(&self, _owner: &OwnerRef, _repository: &str, _format: RepositoryFormat, _name: &str) -> Result<Option<CatalogEntry>, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("the public catalog is busy".to_string()))
        }
        async fn sitemap_entries(&self, _limit: usize) -> Result<Vec<artiferris_domain::public_catalog::SitemapEntry>, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("the public catalog is busy".to_string()))
        }
        async fn owner_summary(&self, _owner: &OwnerRef) -> Result<Option<artiferris_domain::public_catalog::OwnerSummary>, artiferris_domain::error::DomainError> {
            Err(artiferris_domain::error::DomainError::Busy("the public catalog is busy".to_string()))
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_busy_catalog_answers_503_with_retry_after_and_never_500(pool: sqlx::PgPool) {
        let mut state = AppState::build(pool, &test_config());
        let busy: Arc<dyn artiferris_domain::public_catalog::PublicCatalogPort> = Arc::new(BusyCatalog);
        state.search_public_catalog = Arc::new(artiferris_application::use_cases::public_catalog::SearchPublicCatalogUseCase::new(busy.clone()));
        state.suggest_public_catalog = Arc::new(artiferris_application::use_cases::public_catalog::SuggestPublicCatalogUseCase::new(busy.clone()));
        state.get_owner_summary = Arc::new(artiferris_application::use_cases::public_catalog::GetOwnerSummaryUseCase::new(busy.clone()));
        state.list_catalogs = Arc::new(artiferris_application::use_cases::public_catalog::ListCatalogsUseCase::new(busy));
        let app = build_router(state);

        for uri in ["/api/public/search?q=pad", "/api/public/suggest?q=pad", "/api/public/owners/organization/acme", "/api/public/catalogs"] {
            let (status, headers, json) = get(app.clone(), uri).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
            assert_eq!(headers[header::RETRY_AFTER], "5", "{uri}");
            assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{uri}");
            assert_eq!(json["error"], "busy, try again shortly", "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_client_over_the_request_budget_gets_429_with_retry_after(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;
        for _ in 0..REQUESTS_PER_WINDOW {
            assert_eq!(get(app.clone(), "/api/public/search").await.0, StatusCode::OK);
        }

        let (status, headers, _) = get(app, "/api/public/search").await;

        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(headers[header::RETRY_AFTER], "60");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_owner_page_summarises_what_is_public(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;

        let (status, headers, acme) = get(app.clone(), "/api/public/owners/organization/acme").await;
        let (_, _, alice) = get(app, "/api/public/owners/personal/Alice").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(acme, serde_json::json!({ "kind": "organization", "slug": "acme", "display_name": "Acme Corp", "repository_count": 1, "package_count": 1, "image_count": 0 }));
        assert_eq!(alice, serde_json::json!({ "kind": "personal", "slug": "alice", "display_name": "alice", "repository_count": 1, "package_count": 1, "image_count": 0 }));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_owner_without_public_content_is_indistinguishable_from_an_unknown_one(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        state.create_user.execute(PUBLIC_ORGANIZATION_ID, "quiet", "sup3r-s3cret!", false).await.unwrap();
        let hidden = state.create_organization.execute("hidden-org", "Hidden").await.unwrap();
        let admin = state.create_user.execute(hidden, "hidden-admin", "sup3r-s3cret!", false).await.unwrap();
        state.create_repository.execute(hidden, "internal", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        let app = build_router(state);

        for uri in ["/api/public/owners/personal/quiet", "/api/public/owners/organization/hidden-org", "/api/public/owners/personal/nobody", "/api/public/owners/team/acme", "/api/public/owners/personal/!bad!"] {
            let (status, headers, _) = get(app.clone(), uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_can_be_restricted_to_one_owner(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;

        let (_, _, acme) = get(app.clone(), "/api/public/search?owner=organization:acme").await;
        let (_, _, alice) = get(app.clone(), "/api/public/search?owner=personal:alice&q=pad").await;

        assert_eq!(acme["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["left-pad"]);
        assert_eq!(alice["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["pad-lib"]);
        for uri in ["/api/public/search?owner=alice", "/api/public/search?owner=team:acme", "/api/public/search?owner=personal:!bad!"] {
            assert_eq!(get(app.clone(), uri).await.0, StatusCode::BAD_REQUEST, "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn suggestions_are_names_with_their_owner_and_repository(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;

        let (status, headers, json) = get(app, "/api/public/suggest?q=pad").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            json,
            serde_json::json!([
                { "kind": "npm", "name": "pad-lib", "repository": { "name": "my-lib" }, "owner": { "kind": "personal", "slug": "alice", "display_name": "alice" } },
                { "kind": "npm", "name": "left-pad", "repository": { "name": "open-npm" }, "owner": { "kind": "organization", "slug": "acme", "display_name": "Acme Corp" } },
            ])
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn suggestions_can_be_narrowed_by_owner_and_format_and_capped_by_limit(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;
        let names = |json: &serde_json::Value| json.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();

        let (_, _, alice) = get(app.clone(), "/api/public/suggest?q=pad&owner=personal:alice").await;
        let (_, _, acme) = get(app.clone(), "/api/public/suggest?q=pad&owner=organization:acme&format=npm").await;
        let (_, _, docker) = get(app.clone(), "/api/public/suggest?q=pad&format=docker").await;
        let (_, _, all) = get(app.clone(), "/api/public/suggest?q=pad").await;
        let (status, _, one) = get(app.clone(), "/api/public/suggest?q=pad&limit=1").await;
        let (huge_status, _, huge) = get(app.clone(), "/api/public/suggest?q=pad&limit=5000").await;

        assert_eq!(names(&alice), vec!["pad-lib"]);
        assert_eq!(names(&acme), vec!["left-pad"]);
        assert!(names(&docker).is_empty());
        assert_eq!(names(&all).len(), 2, "without filters the answer is what it was");
        assert_eq!((status, one.as_array().unwrap().len()), (StatusCode::OK, 1));
        assert_eq!((huge_status, names(&huge).len()), (StatusCode::OK, 2), "a big limit is clamped, not refused");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn suggestions_reject_a_malformed_owner_format_or_limit(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;
        for uri in [
            "/api/public/suggest?q=pad&owner=alice",
            "/api/public/suggest?q=pad&owner=team:acme",
            "/api/public/suggest?q=pad&owner=personal:!bad!",
            "/api/public/suggest?q=pad&format=pypi",
            "/api/public/suggest?q=pad&limit=0",
            "/api/public/suggest?q=pad&limit=-1",
            "/api/public/suggest?q=pad&limit=many",
        ] {
            let (status, headers, json) = get(app.clone(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{uri}");
            assert!(json["error"].is_string(), "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_typo_is_still_suggested_and_flagged_fuzzy_in_search(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;

        let (_, _, suggestions) = get(app.clone(), "/api/public/suggest?q=lft-pad").await;
        let (_, _, search) = get(app, "/api/public/search?q=lft-pad").await;

        assert_eq!(suggestions[0]["name"], "left-pad");
        assert_eq!((search["items"][0]["name"].as_str(), search["items"][0]["match_kind"].as_str()), (Some("left-pad"), Some("fuzzy")));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn suggestions_reject_text_that_is_too_short_or_too_long(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;
        let too_long = "a".repeat(101);
        for uri in ["/api/public/suggest", "/api/public/suggest?q=", "/api/public/suggest?q=a", "/api/public/suggest?q=%20a%20", "/api/public/suggest?q=ab%00", &format!("/api/public/suggest?q={too_long}")] {
            let (status, headers, json) = get(app.clone(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{uri}");
            assert!(json["error"].is_string(), "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn suggestions_and_search_have_separate_request_budgets(pool: sqlx::PgPool) {
        let app = seeded_app(pool).await;
        for _ in 0..REQUESTS_PER_WINDOW {
            assert_eq!(get(app.clone(), "/api/public/search").await.0, StatusCode::OK);
        }
        assert_eq!(get(app.clone(), "/api/public/search").await.0, StatusCode::TOO_MANY_REQUESTS);

        assert_eq!(get(app.clone(), "/api/public/suggest?q=pad").await.0, StatusCode::OK, "typing must still work when the search budget is spent");
        for _ in 1..SUGGESTIONS_PER_WINDOW {
            get(app.clone(), "/api/public/suggest?q=pad").await;
        }
        let (status, headers, _) = get(app, "/api/public/suggest?q=pad").await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(headers[header::RETRY_AFTER], "60");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_reports_weekly_downloads_and_can_sort_by_them(pool: sqlx::PgPool) {
        use artiferris_domain::download_stats::DownloadCount;

        let state = AppState::build(pool, &test_config());
        let acme = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin = state.create_user.execute(acme, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo = state.create_repository.execute(acme, "open-npm", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        state.set_repository_visibility.execute(repo, true, admin).await.unwrap();
        for name in ["quiet", "busy"] {
            publish_npm(&state, repo, name, "a package").await;
        }
        state
            .download_stats
            .add_batch(&[DownloadCount { day: Utc::now().date_naive(), repository_id: repo, format: RepositoryFormat::Npm, name: "busy".to_string(), downloads: 12 }])
            .await
            .unwrap();
        let app = build_router(state);

        let (status, _, popular) = get(app.clone(), "/api/public/search?sort=popular").await;
        let (_, _, recent) = get(app, "/api/public/search").await;

        assert_eq!(status, StatusCode::OK);
        let summary = |json: &serde_json::Value| json["items"].as_array().unwrap().iter().map(|i| (i["name"].as_str().unwrap().to_string(), i["downloads_7d"].as_i64().unwrap())).collect::<Vec<_>>();
        assert_eq!(summary(&popular), vec![("busy".to_string(), 12), ("quiet".to_string(), 0)]);
        assert!(summary(&recent).contains(&("busy".to_string(), 12)), "the figure is there whatever the sort");
    }

    async fn get_as(app: axum::Router, uri: &str, token: Option<&str>) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder().uri(uri);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
    }

    fn names(json: &serde_json::Value) -> Vec<String> {
        let mut names: Vec<String> = json["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect();
        names.sort();
        names
    }

    /// acme has a private repository, a proxy with a cached package, and a private one nobody in the test is granted; globex has a private one.
    async fn signed_in_scenario(pool: sqlx::PgPool) -> (AppState, String, String, String) {
        let state = AppState::build(pool, &test_config());
        let acme = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let globex = state.create_organization.execute("globex", "Globex").await.unwrap();
        let admin = state.create_user.execute(acme, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        state.set_organization_admin.execute(admin, true, None).await.unwrap();
        let member = state.create_user.execute(acme, "acme-member", "sup3r-s3cret!", false).await.unwrap();
        state.create_user.execute(PUBLIC_ORGANIZATION_ID, "root", "sup3r-s3cret!", true).await.unwrap();
        let globex_admin = state.create_user.execute(globex, "globex-admin", "sup3r-s3cret!", false).await.unwrap();

        let shared = state.create_repository.execute(acme, "shared", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        let hidden = state.create_repository.execute(acme, "hidden", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        let mirror = state.create_repository.execute(acme, "mirror", RepositoryFormat::Npm, RepositoryType::Proxy, Some("https://registry.npmjs.org".to_string()), None, None, admin).await.unwrap();
        let theirs = state.create_repository.execute(globex, "theirs", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, globex_admin).await.unwrap();
        let open = state.create_repository.execute(globex, "open", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, globex_admin).await.unwrap();
        state.set_repository_visibility.execute(open, true, globex_admin).await.unwrap();
        for (repo, name) in [(shared, "pad-shared"), (hidden, "pad-hidden"), (mirror, "pad-cached"), (theirs, "pad-globex-private"), (open, "pad-open")] {
            publish_npm(&state, repo, name, "a pad").await;
        }
        state.grant_permission.execute(member, shared, artiferris_domain::permission::Role::Read, admin).await.unwrap();

        let admin_token = state.authenticate_user.execute("acme-admin", "sup3r-s3cret!").await.unwrap();
        let member_token = state.authenticate_user.execute("acme-member", "sup3r-s3cret!").await.unwrap();
        let root_token = state.authenticate_user.execute("root", "sup3r-s3cret!").await.unwrap();
        (state, admin_token, member_token, root_token)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn signed_in_search_requires_a_token(pool: sqlx::PgPool) {
        let (state, ..) = signed_in_scenario(pool).await;

        let (status, _) = get_as(build_router(state), "/api/search?q=pad", None).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_member_finds_what_they_were_granted_plus_public_content_and_nothing_else(pool: sqlx::PgPool) {
        let (state, _, member, _) = signed_in_scenario(pool).await;

        let (status, json) = get_as(build_router(state), "/api/search?q=pad", Some(&member)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(names(&json), vec!["pad-open", "pad-shared"], "granted + public; not the other private ones, not the proxy cache");
        let shared = json["items"].as_array().unwrap().iter().find(|i| i["name"] == "pad-shared").unwrap();
        assert_eq!(shared["repository"]["repo_type"], "hosted");
        assert!(shared["repository"]["id"].is_string());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_also_finds_the_cached_packages_of_their_proxies(pool: sqlx::PgPool) {
        let (state, admin, ..) = signed_in_scenario(pool).await;

        let (_, json) = get_as(build_router(state), "/api/search?q=pad", Some(&admin)).await;

        assert_eq!(names(&json), vec!["pad-cached", "pad-hidden", "pad-open", "pad-shared"], "their whole organization plus public content; never another organization's private repository");
        let cached = json["items"].as_array().unwrap().iter().find(|i| i["name"] == "pad-cached").unwrap();
        assert_eq!(cached["repository"]["repo_type"], "proxy");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_super_admin_finds_every_package(pool: sqlx::PgPool) {
        let (state, _, _, root) = signed_in_scenario(pool).await;

        let (_, json) = get_as(build_router(state), "/api/search?q=pad", Some(&root)).await;

        assert_eq!(names(&json), vec!["pad-cached", "pad-globex-private", "pad-hidden", "pad-open", "pad-shared"]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_public_search_stays_public_and_unchanged_for_everyone(pool: sqlx::PgPool) {
        let (state, admin, ..) = signed_in_scenario(pool).await;
        let app = build_router(state);

        let (_, anonymous) = get_as(app.clone(), "/api/public/search?q=pad", None).await;
        let (_, as_admin) = get_as(app, "/api/public/search?q=pad", Some(&admin)).await;

        assert_eq!(names(&anonymous), vec!["pad-open"]);
        assert_eq!(names(&as_admin), vec!["pad-open"], "a token does not widen the public endpoint");
        assert!(anonymous["items"][0]["repository"].get("id").is_none(), "public results carry no repository id");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn signed_in_search_validates_like_the_public_one(pool: sqlx::PgPool) {
        let (state, admin, ..) = signed_in_scenario(pool).await;
        let app = build_router(state);

        for uri in ["/api/search?per_page=51", "/api/search?format=helm", "/api/search?page=abc"] {
            assert_eq!(get_as(app.clone(), uri, Some(&admin)).await.0, StatusCode::BAD_REQUEST, "{uri}");
        }
    }
}
