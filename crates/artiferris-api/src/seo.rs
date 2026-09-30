//! What a crawler (or a link preview) sees of the public catalog: the `<head>` of each page, `robots.txt` and the sitemap.
//! The page bodies stay client-rendered; only the head is produced here, from the same catalog data and visibility rules.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use artiferris_application::use_cases::seo::{catalog_path, owner_path, package_path, parse_route, repository_path, PageMeta, SeoRoute, SITE_NAME};
use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
use artiferris_domain::public_catalog::{SitemapEntry, SitemapTarget, CATALOG_FORMATS};
use artiferris_domain::user_preferences::Language;
use chrono::{DateTime, SecondsFormat, Utc};
use percent_encoding::percent_decode_str;

use crate::routes::auth::peer_ip_bucket;
use crate::routes::public_catalog::within_budget;
use crate::state::AppState;

/// Under the 50,000 URLs a sitemap file may hold, with room to spare.
const URLS_PER_SITEMAP: usize = 40_000;
const SITEMAP_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// After a failed build the database is left alone for this long; the previous copy, or a 503, is served meanwhile.
const SITEMAP_RETRY_DELAY: Duration = Duration::from_secs(60);
/// A crawler needs a handful of files a day; a sitemap is expensive to build and big to send.
const SITEMAP_REQUESTS_PER_MINUTE: usize = 30;
/// Building a head can cost a few catalog queries, so a client that only ever asks for pages gets the generic head once it exceeds this.
const PAGE_HEADS_PER_MINUTE: usize = 120;
/// The app's shell must not wait on the database: past this, the page is served with the generic head.
const HEAD_TIMEOUT: Duration = Duration::from_millis(300);
const INDEXING_SETTING_TTL: Duration = Duration::from_secs(30);
const INDEXING_SETTING_TIMEOUT: Duration = Duration::from_secs(1);
/// Well under what the database and the sitemap files can hold; past it the list is cut, with a warning.
const MAX_SITEMAP_ENTRIES: usize = 500_000;
/// Paths of the built app (scripts, styles, images, fonts) that are not app routes when nothing serves them.
const ASSET_PREFIXES: [&str; 4] = ["/assets/", "/media/", "/fonts/", "/i18n/"];
const ASSET_EXTENSIONS: [&str; 20] =
    ["js", "mjs", "css", "map", "ico", "png", "jpg", "jpeg", "gif", "svg", "webp", "avif", "woff", "woff2", "ttf", "otf", "eot", "json", "txt", "webmanifest"];

/// What the head of one response depends on besides the page itself.
pub struct HeadContext<'a> {
    pub public_url: &'a str,
    pub indexing_enabled: bool,
    /// A search-result page (`?q=`): useful to people, not worth an index entry of its own.
    pub has_search_text: bool,
    /// The page or the switch could not be read just now: no robots directive at all, never a `noindex` guess.
    pub indexing_unknown: bool,
}

pub fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// JSON that can sit inside a `<script>` element: nothing in it can close the element or open a comment.
fn json_for_script(value: &serde_json::Value) -> String {
    value.to_string().replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

/// The document with the `lang` of its `<html>` element set, or as it was when it has no such element.
fn with_html_lang(document: &str, language: Language) -> String {
    let Some(start) = document.find("<html") else { return document.to_string() };
    let Some(tag_end) = document[start..].find('>').map(|end| start + end) else { return document.to_string() };
    let tag = &document[start..tag_end];
    let lang = format!(r#"lang="{}""#, language.as_str());
    let rewritten = match tag.find("lang=\"") {
        Some(at) => match tag[at + 6..].find('"') {
            Some(close) => format!("{}{lang}{}", &tag[..at], &tag[at + 6 + close + 1..]),
            None => return document.to_string(),
        },
        None => format!("{tag} {lang}"),
    };
    format!("{}{rewritten}{}", &document[..start], &document[tag_end..])
}

/// Rewrites the app's `index.html` head for one page.
pub fn render_head(template: &str, meta: &PageMeta, context: &HeadContext<'_>) -> String {
    let robots = if context.indexing_unknown {
        None
    } else if !context.indexing_enabled || !meta.indexable {
        Some("noindex, nofollow")
    } else if context.has_search_text {
        Some("noindex, follow")
    } else {
        Some("index, follow")
    };
    let base = context.public_url.trim_end_matches('/');
    let mut tags: Vec<String> = robots.map(|robots| format!(r#"<meta name="robots" content="{robots}">"#)).into_iter().collect();
    if meta.indexable {
        let title = escape_html(&meta.title);
        tags.push(format!(r#"<meta property="og:site_name" content="{}">"#, escape_html(SITE_NAME)));
        tags.push(r#"<meta property="og:type" content="website">"#.to_string());
        if let Some(language) = meta.language {
            tags.push(format!(r#"<meta property="og:locale" content="{}">"#, language.og_locale()));
        }
        tags.push(format!(r#"<meta property="og:title" content="{title}">"#));
        tags.push(format!(r#"<meta property="og:image" content="{}">"#, escape_html(&format!("{base}/api/branding/logo"))));
        tags.push(r#"<meta name="twitter:card" content="summary">"#.to_string());
        tags.push(format!(r#"<meta name="twitter:title" content="{title}">"#));
        if let Some(description) = &meta.description {
            let description = escape_html(description);
            tags.push(format!(r#"<meta name="description" content="{description}">"#));
            tags.push(format!(r#"<meta property="og:description" content="{description}">"#));
            tags.push(format!(r#"<meta name="twitter:description" content="{description}">"#));
        }
        if let Some(path) = &meta.canonical_path {
            let url = escape_html(&format!("{base}{path}"));
            tags.push(format!(r#"<link rel="canonical" href="{url}">"#));
            tags.push(format!(r#"<meta property="og:url" content="{url}">"#));
        }
        if let Some(data) = &meta.structured_data {
            tags.push(format!(r#"<script type="application/ld+json">{}</script>"#, json_for_script(data)));
        }
    }

    let title = format!("<title>{}</title>", escape_html(&meta.title));
    let with_title = match (template.find("<title>"), template.find("</title>")) {
        (Some(start), Some(end)) if start < end => format!("{}{title}{}", &template[..start], &template[end + "</title>".len()..]),
        _ => template.replacen("</head>", &format!("{title}</head>"), 1),
    };
    let with_title = match (meta.indexable, meta.language) {
        (true, Some(language)) => with_html_lang(&with_title, language),
        _ => with_title,
    };
    let injected = format!("    {}\n  ", tags.join("\n    "));
    match with_title.find("</head>") {
        Some(end) => format!("{}{injected}{}", &with_title[..end], &with_title[end..]),
        None => with_title,
    }
}

pub fn robots_txt(public_url: &str, indexing_enabled: bool) -> String {
    if !indexing_enabled {
        return "User-agent: *\nDisallow: /\n".to_string();
    }
    let mut lines = vec!["User-agent: *".to_string()];
    for area in ["/api/", "/npm/", "/v2/", "/login", "/register", "/activate", "/repositories", "/users", "/account", "/admin", "/my-repository"] {
        lines.push(format!("Disallow: {area}"));
    }
    lines.push(format!("Sitemap: {}/sitemap.xml", public_url.trim_end_matches('/')));
    lines.join("\n") + "\n"
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn lastmod(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Every page worth listing, as `(absolute URL, last modification)`. The fixed pages come first.
fn sitemap_urls(public_url: &str, entries: &[SitemapEntry]) -> Vec<(String, Option<DateTime<Utc>>)> {
    let base = public_url.trim_end_matches('/');
    let mut urls: Vec<(String, Option<DateTime<Utc>>)> = vec![(format!("{base}/explorer"), None)];
    urls.extend(CATALOG_FORMATS.iter().filter_map(|spec| catalog_path(spec.format)).map(|path| (format!("{base}{path}"), None)));
    urls.extend(entries.iter().map(|entry| {
        let path = match &entry.target {
            SitemapTarget::Owner => owner_path(&entry.owner),
            SitemapTarget::Repository { repository } => repository_path(&entry.owner, repository),
            SitemapTarget::Package { repository, format, name } => package_path(&entry.owner, repository, *format, name),
        };
        (format!("{base}{path}"), Some(entry.updated_at))
    }));
    urls
}

/// The sitemap index and its pages, ready to serve.
pub struct RenderedSitemap {
    pub index: String,
    pub pages: Vec<String>,
}

pub fn render_sitemap(public_url: &str, entries: &[SitemapEntry]) -> RenderedSitemap {
    let base = public_url.trim_end_matches('/');
    let urls = sitemap_urls(public_url, entries);
    let pages: Vec<String> = urls
        .chunks(URLS_PER_SITEMAP)
        .map(|chunk| {
            let body: String = chunk
                .iter()
                .map(|(loc, modified)| match modified {
                    Some(at) => format!("  <url><loc>{}</loc><lastmod>{}</lastmod></url>\n", xml_escape(loc), lastmod(*at)),
                    None => format!("  <url><loc>{}</loc></url>\n", xml_escape(loc)),
                })
                .collect();
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{body}</urlset>\n")
        })
        .collect();
    let listed: String = (1..=pages.len()).map(|n| format!("  <sitemap><loc>{}</loc></sitemap>\n", xml_escape(&format!("{base}/sitemap-{n}.xml")))).collect();
    RenderedSitemap { index: format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{listed}</sitemapindex>\n"), pages }
}

const EMPTY_URLSET: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n</urlset>\n";

#[derive(Clone)]
pub struct SeoState {
    app: AppState,
    index_template: Arc<str>,
    sitemap: Arc<SitemapCache>,
    indexing: Arc<Mutex<Option<(Instant, bool)>>>,
    indexing_ttl: Duration,
    sitemap_ttl: Duration,
    sitemap_retry_delay: Duration,
    max_sitemap_entries: usize,
}

/// The last sitemap, when the last build failed, and the lock held by the one build allowed at a time.
struct SitemapCache {
    built: Mutex<Option<(Instant, Arc<RenderedSitemap>)>>,
    failed_at: Mutex<Option<Instant>>,
    rebuilding: Arc<tokio::sync::Mutex<()>>,
}

impl SeoState {
    pub fn new(app: AppState, index_template: String) -> Self {
        Self {
            app,
            index_template: index_template.into(),
            sitemap: Arc::new(SitemapCache { built: Mutex::new(None), failed_at: Mutex::new(None), rebuilding: Arc::new(tokio::sync::Mutex::new(())) }),
            indexing: Arc::new(Mutex::new(None)),
            indexing_ttl: INDEXING_SETTING_TTL,
            sitemap_ttl: SITEMAP_CACHE_TTL,
            sitemap_retry_delay: SITEMAP_RETRY_DELAY,
            max_sitemap_entries: MAX_SITEMAP_ENTRIES,
        }
    }

    /// The indexing switch, read at most every 30 seconds. When the database cannot answer, the last known value is used;
    /// `None` only when there never was one, which callers must not mistake for "off".
    async fn indexing_enabled(&self) -> Option<bool> {
        let known = *self.indexing.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((read_at, enabled)) = known {
            if read_at.elapsed() < self.indexing_ttl {
                return Some(enabled);
            }
        }
        let read = tokio::time::timeout(INDEXING_SETTING_TIMEOUT, self.app.get_system_settings.execute(PUBLIC_ORGANIZATION_ID)).await;
        match read {
            Ok(Ok(settings)) => {
                // An instance that closed its public pages has nothing for a search engine to index either.
                let indexing_enabled = settings.seo_indexing_enabled && settings.public_page_enabled;
                *self.indexing.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), indexing_enabled));
                Some(indexing_enabled)
            }
            failed => {
                let reason = match failed {
                    Ok(Err(e)) => e.to_string(),
                    _ => "timed out".to_string(),
                };
                tracing::warn!("could not read the indexing setting, using the last known value ({}): {reason}", known.map_or("none", |(_, enabled)| if enabled { "on" } else { "off" }));
                known.map(|(_, enabled)| enabled)
            }
        }
    }

    fn fresh_sitemap(&self) -> Option<Arc<RenderedSitemap>> {
        self.sitemap.built.lock().unwrap_or_else(|p| p.into_inner()).as_ref().filter(|(built_at, _)| built_at.elapsed() < self.sitemap_ttl).map(|(_, sitemap)| sitemap.clone())
    }

    fn any_sitemap(&self) -> Option<Arc<RenderedSitemap>> {
        self.sitemap.built.lock().unwrap_or_else(|p| p.into_inner()).as_ref().map(|(_, sitemap)| sitemap.clone())
    }

    fn build_failed_recently(&self) -> bool {
        self.sitemap.failed_at.lock().unwrap_or_else(|p| p.into_inner()).is_some_and(|failed_at| failed_at.elapsed() < self.sitemap_retry_delay)
    }

    /// Starts a build unless one is running or the last one failed a moment ago. It runs on its own task, so a client that
    /// gives up does not cancel it.
    fn start_rebuild(&self) {
        if self.build_failed_recently() {
            return;
        }
        let Ok(running) = self.sitemap.rebuilding.clone().try_lock_owned() else { return };
        let seo = self.clone();
        tokio::spawn(async move {
            let _running = running;
            match seo.build_sitemap().await {
                Ok(built) => {
                    *seo.sitemap.built.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), built));
                    *seo.sitemap.failed_at.lock().unwrap_or_else(|p| p.into_inner()) = None;
                }
                Err(()) => *seo.sitemap.failed_at.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now()),
            }
        });
    }

    /// An expired copy is served while a build refreshes it. With no copy at all, requests wait for the build in progress,
    /// or get an error while the last build's failure is recent.
    async fn sitemap(&self) -> Result<Arc<RenderedSitemap>, ()> {
        if let Some(fresh) = self.fresh_sitemap() {
            return Ok(fresh);
        }
        self.start_rebuild();
        if let Some(stale) = self.any_sitemap() {
            return Ok(stale);
        }
        drop(self.sitemap.rebuilding.lock().await);
        self.any_sitemap().ok_or(())
    }

    async fn build_sitemap(&self) -> Result<Arc<RenderedSitemap>, ()> {
        let mut entries = self.app.public_catalog.sitemap_entries(self.max_sitemap_entries + 1).await.map_err(|e| tracing::warn!("could not build the sitemap: {e}"))?;
        if entries.len() > self.max_sitemap_entries {
            tracing::warn!("the public catalog has more than {} pages, the sitemap lists only the first {}", self.max_sitemap_entries, self.max_sitemap_entries);
            entries.truncate(self.max_sitemap_entries);
        }
        let public_url = self.app.public_url.clone();
        let rendered = tokio::task::spawn_blocking(move || render_sitemap(&public_url, &entries)).await.map_err(|e| tracing::warn!("could not render the sitemap: {e}"))?;
        Ok(Arc::new(rendered))
    }

    /// The last value of the switch, however old.
    fn known_indexing(&self) -> Option<bool> {
        self.indexing.lock().unwrap_or_else(|p| p.into_inner()).map(|(_, enabled)| enabled)
    }

    /// What a page gets when its head could not be worked out: only a switch known to be off says `noindex`.
    fn unresolved_head(switch: Option<bool>) -> Head {
        Head { meta: PageMeta::generic(), indexing_enabled: false, unknown: switch != Some(false) }
    }

    /// The metadata of one page and whether the instance lets it be indexed, within the time a page load can afford.
    async fn head_inputs(&self, route: &SeoRoute, language: Language) -> Head {
        let build = async {
            let meta = match self.app.seo_pages.execute(route, language).await {
                Ok(meta) => meta,
                Err(e) => {
                    tracing::warn!("could not build the page head, using the generic one: {e}");
                    return Self::unresolved_head(self.indexing_enabled().await);
                }
            };
            if !meta.indexable {
                return Head { meta, indexing_enabled: false, unknown: false };
            }
            match self.indexing_enabled().await {
                Some(indexing_enabled) => Head { meta, indexing_enabled, unknown: false },
                None => Head { meta, indexing_enabled: true, unknown: true },
            }
        };
        tokio::time::timeout(HEAD_TIMEOUT, build).await.unwrap_or_else(|_| {
            tracing::warn!("building the page head took over {HEAD_TIMEOUT:?}, using the generic one");
            Self::unresolved_head(self.known_indexing())
        })
    }
}

struct Head {
    meta: PageMeta,
    indexing_enabled: bool,
    unknown: bool,
}

pub fn router(state: SeoState) -> Router {
    Router::new().route("/robots.txt", get(robots)).route("/sitemap.xml", get(sitemap_index)).fallback(page).with_state(state)
}

/// The built app: its files first, and every page (`/` and `/index.html` included) through the head injection.
pub fn app_router(static_dir: &str, state: SeoState) -> Router {
    let files = tower_http::services::ServeDir::new(static_dir).append_index_html_on_directories(false).fallback(router(state.clone()));
    Router::new().route("/", get(page)).route("/index.html", get(page)).fallback_service(files).with_state(state)
}

fn text(content_type: &'static str, body: String) -> Response {
    let mut response = body.into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

/// A crawler that is told "closed" caches that for a day, so a switch that cannot be read is a 503, never a guess.
async fn robots(State(seo): State<SeoState>) -> Response {
    match seo.indexing_enabled().await {
        Some(enabled) => text("text/plain; charset=utf-8", robots_txt(&seo.app.public_url, enabled)),
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

fn sitemap_over_budget(seo: &SeoState, headers: &HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> Option<Response> {
    if within_budget(&seo.app, headers, connect_info, "public-sitemap", SITEMAP_REQUESTS_PER_MINUTE) {
        return None;
    }
    let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
    response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(60));
    Some(response)
}

async fn sitemap_index(State(seo): State<SeoState>, headers: HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> Response {
    if let Some(rejected) = sitemap_over_budget(&seo, &headers, connect_info) {
        return rejected;
    }
    match seo.indexing_enabled().await {
        None => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Some(false) => return text("application/xml; charset=utf-8", EMPTY_URLSET.to_string()),
        Some(true) => {}
    }
    match seo.sitemap().await {
        Ok(sitemap) => text("application/xml; charset=utf-8", sitemap.index.clone()),
        Err(()) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// `sitemap-<n>.xml`, 1-based.
fn sitemap_page_number(path: &str) -> Option<usize> {
    path.strip_prefix("/sitemap-")?.strip_suffix(".xml")?.parse::<usize>().ok().filter(|n| *n >= 1)
}

fn looks_like_a_static_file(path: &str) -> bool {
    let extension = path.rsplit('/').next().and_then(|last| last.rsplit_once('.')).map(|(_, extension)| extension);
    ASSET_PREFIXES.iter().any(|prefix| path.starts_with(prefix)) || extension.is_some_and(|extension| ASSET_EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(extension)))
}

fn has_search_text(uri: &Uri) -> bool {
    uri.query().is_some_and(|query| query.split('&').any(|pair| pair.strip_prefix("q=").is_some_and(|value| !value.is_empty())))
}

/// Everything the static files did not answer: a sitemap page, or an SPA route served with its own head.
async fn page(State(seo): State<SeoState>, uri: Uri, headers: HeaderMap, connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") || path.starts_with("/npm/") || path.starts_with("/v2/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(number) = sitemap_page_number(path) {
        if let Some(rejected) = sitemap_over_budget(&seo, &headers, connect_info) {
            return rejected;
        }
        match seo.indexing_enabled().await {
            None => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            Some(false) => return StatusCode::NOT_FOUND.into_response(),
            Some(true) => {}
        }
        return match seo.sitemap().await {
            Ok(sitemap) => sitemap.pages.get(number - 1).map_or_else(|| StatusCode::NOT_FOUND.into_response(), |page| text("application/xml; charset=utf-8", page.clone())),
            Err(()) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
    }

    let segments: Vec<String> = if path == "/index.html" {
        Vec::new()
    } else {
        path.split('/').filter(|s| !s.is_empty()).map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned()).collect()
    };
    let route = parse_route(&segments);
    // A missing script, stylesheet or image, not an app route: answering it with the app would hide the breakage.
    // A package called `chart.js` is a route, so only what no route claims can be a missing file.
    if route == SeoRoute::Other && looks_like_a_static_file(path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let throttle_key = format!("public-page:{}", peer_ip_bucket(&seo.app, &headers, connect_info));
    let over_budget = seo.app.public_throttle.is_throttled(&throttle_key, PAGE_HEADS_PER_MINUTE, Duration::from_secs(60));
    seo.app.public_throttle.record_failure(&throttle_key, PAGE_HEADS_PER_MINUTE, Duration::from_secs(60));
    // The URL is the same in every language: the head follows the reader's `Accept-Language`, English for a crawler that sends none.
    let language = headers.get(header::ACCEPT_LANGUAGE).and_then(|value| value.to_str().ok()).map_or(Language::FALLBACK, Language::from_accept_language);
    let head = if over_budget { SeoState::unresolved_head(seo.known_indexing()) } else { seo.head_inputs(&route, language).await };

    let context = HeadContext { public_url: &seo.app.public_url, indexing_enabled: head.indexing_enabled, has_search_text: has_search_text(&uri), indexing_unknown: head.unknown };
    let mut response = text("text/html; charset=utf-8", render_head(&seo.index_template, &head.meta, &context));
    // The head depends on the indexing switch and on what is public right now; a head that could not be worked out must not be reused.
    response.headers_mut().insert(header::VARY, HeaderValue::from_static("Accept-Language"));
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(if head.unknown { "no-store" } else { "no-cache" }));
    response
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use artiferris_application::use_cases::admin::GetSystemSettingsUseCase;
    use artiferris_application::use_cases::seo::SeoPageUseCase;
    use artiferris_domain::error::DomainError;
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion};
    use artiferris_domain::public_catalog::{CatalogEntry, CatalogEntryCount, CatalogPage, CatalogQuery, CatalogRepository, CatalogSuggestion, OwnerSummary, PublicCatalogPort};
    use artiferris_domain::system_settings::{SystemSettings, SystemSettingsPort};
    use tokio::sync::Semaphore;
    use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
    use artiferris_domain::public_catalog::{OwnerKind, OwnerRef};
    use serde_json::json;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::routes::repositories::test_support::test_config;

    const TEMPLATE: &str = "<!doctype html>\n<html lang=\"fr\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <title>ArtiFerris · Artifact Repository</title>\n    <base href=\"/\" />\n  </head>\n  <body><app-root></app-root></body>\n</html>\n";

    fn meta(title: &str, description: Option<&str>) -> PageMeta {
        PageMeta { title: title.to_string(), description: description.map(str::to_string), canonical_path: Some("/@alice/lib".to_string()), indexable: true, structured_data: None, language: Some(Language::Fr) }
    }

    fn context(indexing_enabled: bool) -> HeadContext<'static> {
        HeadContext { public_url: "https://registry.example.com/", indexing_enabled, has_search_text: false, indexing_unknown: false }
    }

    #[test]
    fn the_head_of_an_indexable_page_carries_its_metadata_and_keeps_the_rest_of_the_document() {
        let html = render_head(TEMPLATE, &meta("left-pad | ArtiFerris", Some("Pads strings")), &context(true));

        for expected in [
            "<title>left-pad | ArtiFerris</title>",
            r#"<meta name="robots" content="index, follow">"#,
            r#"<meta name="description" content="Pads strings">"#,
            r#"<link rel="canonical" href="https://registry.example.com/@alice/lib">"#,
            r#"<meta property="og:title" content="left-pad | ArtiFerris">"#,
            r#"<meta property="og:image" content="https://registry.example.com/api/branding/logo">"#,
            r#"<meta name="twitter:card" content="summary">"#,
            r#"<base href="/" />"#,
            "<app-root></app-root>",
        ] {
            assert!(html.contains(expected), "missing {expected} in {html}");
        }
        assert_eq!(html.matches("<title>").count(), 1, "the original title is replaced, not duplicated");
        assert!(!html.contains("Artifact Repository"));
    }

    #[test]
    fn the_html_lang_is_replaced_added_or_left_alone() {
        assert!(with_html_lang("<html lang=\"fr\"><head>", Language::De).starts_with("<html lang=\"de\"><head>"));
        assert!(with_html_lang("<html class=\"a\" lang=\"fr\" dir=\"ltr\">", Language::Es).starts_with("<html class=\"a\" lang=\"es\" dir=\"ltr\">"));
        assert!(with_html_lang("<html>", Language::It).starts_with("<html lang=\"it\">"));
        assert_eq!(with_html_lang("<div>no html element</div>", Language::En), "<div>no html element</div>");
    }

    #[test]
    fn a_generic_head_leaves_the_language_of_the_document_alone() {
        let html = render_head(TEMPLATE, &PageMeta::generic(), &context(true));

        assert!(html.contains(r#"<html lang="fr">"#) && !html.contains("og:locale"), "{html}");
    }

    #[test]
    fn nothing_is_indexable_while_the_instance_switch_is_off_but_link_previews_still_work() {
        let html = render_head(TEMPLATE, &meta("x | ArtiFerris", Some("d")), &context(false));

        assert!(html.contains(r#"<meta name="robots" content="noindex, nofollow">"#));
        assert!(html.contains(r#"og:title"#), "a chat app can still preview a public page");
    }

    #[test]
    fn a_generic_head_says_noindex_and_reveals_nothing_else() {
        let html = render_head(TEMPLATE, &PageMeta::generic(), &context(true));

        assert!(html.contains(r#"<meta name="robots" content="noindex, nofollow">"#));
        for absent in ["canonical", "og:", "twitter:", "description", "ld+json"] {
            assert!(!html.contains(absent), "{absent} in {html}");
        }
        assert!(html.contains("<title>ArtiFerris · Artifact Repository</title>"));
    }

    #[test]
    fn search_result_pages_are_noindex_but_their_links_may_be_followed() {
        let html = render_head(TEMPLATE, &meta("t", None), &HeadContext { has_search_text: true, ..context(true) });

        assert!(html.contains(r#"<meta name="robots" content="noindex, follow">"#));
    }

    #[test]
    fn user_authored_text_cannot_break_out_of_the_head() {
        let hostile = r#"</title><script>alert(1)</script><meta x=" onload="alert(2)"#;

        let html = render_head(TEMPLATE, &meta(hostile, Some(hostile)), &context(true));

        assert!(!html.contains("<script>alert"), "{html}");
        assert!(!html.contains(r#"" onload=""#), "an attribute must not be closable: {html}");
        assert!(html.contains("&lt;/title&gt;&lt;script&gt;"), "{html}");
        assert_eq!(html.matches("</title>").count(), 1);
    }

    #[test]
    fn structured_data_cannot_close_its_script_element() {
        let mut page = meta("t", None);
        page.structured_data = Some(json!({ "name": "</script><script>alert(1)</script>", "description": "<!-- & -->", "x": "line\u{2028}sep" }));

        let html = render_head(TEMPLATE, &page, &context(true));

        assert_eq!(html.matches("<script").count(), 1, "only the JSON-LD element itself: {html}");
        assert_eq!(html.matches("</script>").count(), 1, "{html}");
        let data = html.split(r#"<script type="application/ld+json">"#).nth(1).unwrap().split("</script>").next().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(parsed["name"], "</script><script>alert(1)</script>", "escaping is lossless for a JSON reader");
        assert!(!data.contains('<') && !data.contains('>') && !data.contains('&') && !data.contains('\u{2028}'));
    }

    #[test]
    fn a_template_without_a_title_still_gets_one() {
        let html = render_head("<html><head></head><body></body></html>", &meta("Hello", None), &context(true));

        assert!(html.contains("<title>Hello</title>") && html.contains("</head>"));
    }

    #[test]
    fn robots_txt_shuts_everything_when_indexing_is_off() {
        assert_eq!(robots_txt("https://r.example", false), "User-agent: *\nDisallow: /\n");
    }

    #[test]
    fn robots_txt_keeps_crawlers_out_of_the_api_and_the_app_and_points_at_the_sitemap() {
        let robots = robots_txt("https://r.example/", true);

        for line in ["Disallow: /api/", "Disallow: /login", "Disallow: /admin", "Disallow: /npm/", "Disallow: /v2/", "Sitemap: https://r.example/sitemap.xml"] {
            assert!(robots.lines().any(|l| l == line), "missing {line} in {robots}");
        }
        assert!(!robots.lines().any(|l| l == "Disallow: /"), "the public pages stay crawlable");
    }

    fn entry(target: SitemapTarget, owner: OwnerRef) -> SitemapEntry {
        SitemapEntry { owner, target, updated_at: "2026-09-20T10:00:00Z".parse().unwrap() }
    }

    #[test]
    fn the_sitemap_lists_the_fixed_pages_then_every_public_page_with_its_last_modification() {
        let alice = OwnerRef { kind: OwnerKind::Personal, slug: "alice".into() };
        let entries = vec![
            entry(SitemapTarget::Owner, alice.clone()),
            entry(SitemapTarget::Repository { repository: "lib".into() }, alice.clone()),
            entry(SitemapTarget::Package { repository: "lib".into(), format: RepositoryFormat::Npm, name: "@scope/a&b".into() }, alice),
        ];

        let sitemap = render_sitemap("https://r.example", &entries);

        assert_eq!(sitemap.pages.len(), 1);
        let page = &sitemap.pages[0];
        for loc in ["https://r.example/explorer", "https://r.example/artiferris-npm", "https://r.example/artiferris-docker", "https://r.example/@alice", "https://r.example/@alice/lib"] {
            assert!(page.contains(&format!("<loc>{loc}</loc>")), "missing {loc} in {page}");
        }
        assert!(page.contains("<loc>https://r.example/@alice/lib/packages/npm/@scope%2Fa&amp;b</loc><lastmod>2026-09-20T10:00:00Z</lastmod>"), "{page}");
        assert!(sitemap.index.contains("<loc>https://r.example/sitemap-1.xml</loc>"));
    }

    #[test]
    fn a_large_sitemap_is_split_under_the_protocol_limit() {
        let owner = OwnerRef { kind: OwnerKind::Organization, slug: "acme".into() };
        let entries: Vec<_> = (0..(URLS_PER_SITEMAP * 2 + 10)).map(|i| entry(SitemapTarget::Package { repository: "r".into(), format: RepositoryFormat::Npm, name: format!("p{i}") }, owner.clone())).collect();

        let sitemap = render_sitemap("https://r.example", &entries);

        assert_eq!(sitemap.pages.len(), 3);
        assert!(sitemap.pages.iter().all(|page| page.matches("<url>").count() <= URLS_PER_SITEMAP));
        assert_eq!(sitemap.index.matches("<sitemap>").count(), 3);
        assert_eq!(sitemap_page_number("/sitemap-3.xml"), Some(3));
        assert_eq!((sitemap_page_number("/sitemap-0.xml"), sitemap_page_number("/sitemap-x.xml"), sitemap_page_number("/sitemap.xml")), (None, None, None));
    }

    #[test]
    fn only_a_non_empty_q_makes_a_page_a_search_result() {
        for (uri, expected) in [("/explorer?q=pad", true), ("/explorer?format=npm&q=pad", true), ("/explorer?q=", false), ("/explorer?sort=popular", false), ("/explorer", false)] {
            assert_eq!(has_search_text(&uri.parse().unwrap()), expected, "{uri}");
        }
    }

    async fn app_with(pool: sqlx::PgPool) -> (AppState, Uuid) {
        let state = AppState::build(pool, &test_config());
        let acme = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let admin = state.create_user.execute(acme, "acme-admin", "sup3r-s3cret!", false).await.unwrap();
        let open = state.create_repository.execute(acme, "open-npm", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        state.set_repository_visibility.execute(open, true, admin).await.unwrap();
        let closed = state.create_repository.execute(acme, "closed-npm", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        for (repo, name, description) in [(open, "left-pad", "Pads strings on the left"), (closed, "secret-pad", "never listed")] {
            let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repo, name: NpmPackageName::parse(name).unwrap(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
            state.npm_packages.create_package(&package).await.unwrap();
            state
                .npm_packages
                .insert_version(&NpmPackageVersion {
                    id: Uuid::new_v4(),
                    npm_package_id: package.id,
                    version: NpmVersion::parse("1.2.3").unwrap(),
                    manifest: json!({ "description": description }),
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
        (state, admin)
    }

    async fn enable_indexing(state: &AppState) {
        let mut settings = state.get_system_settings.execute(PUBLIC_ORGANIZATION_ID).await.unwrap();
        settings.seo_indexing_enabled = true;
        state.update_system_settings.execute(PUBLIC_ORGANIZATION_ID, settings, None).await.unwrap();
    }

    async fn get(state: &AppState, uri: &str) -> (StatusCode, HeaderMap, String) {
        get_accepting(state, uri, None).await
    }

    async fn get_accepting(state: &AppState, uri: &str, accept_language: Option<&str>) -> (StatusCode, HeaderMap, String) {
        let app = router(SeoState::new(state.clone(), TEMPLATE.to_string()));
        let mut request = Request::builder().uri(uri);
        if let Some(accept_language) = accept_language {
            request = request.header(header::ACCEPT_LANGUAGE, accept_language);
        }
        let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        (status, headers, String::from_utf8(axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap())
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_public_package_page_is_served_with_its_own_head_and_a_200(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        enable_indexing(&state).await;

        let (status, headers, html) = get(&state, "/o/acme/open-npm/packages/npm/left-pad").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
        for expected in [
            "<title>left-pad: npm package by Acme Corp | ArtiFerris</title>",
            r#"<meta name="description" content="Pads strings on the left">"#,
            r#"<link rel="canonical" href="http://localhost:4200/o/acme/open-npm/packages/npm/left-pad">"#,
            r#"content="index, follow""#,
            r#""@type":"SoftwareSourceCode""#,
        ] {
            assert!(html.contains(expected), "missing {expected} in {html}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_page_head_follows_the_accept_language_of_the_request_and_english_without_one(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let uri = "/o/acme/open-npm/packages/npm/left-pad";

        let (_, headers, french) = get_accepting(&state, uri, Some("fr-CA,fr;q=0.9,en;q=0.5")).await;
        let (_, _, german) = get_accepting(&state, uri, Some("ja, de;q=0.8")).await;
        let (_, _, unknown) = get_accepting(&state, uri, Some("ja, zh-CN")).await;
        let (_, _, none) = get(&state, uri).await;

        assert_eq!(headers[header::VARY], "Accept-Language");
        assert!(french.contains("<title>left-pad : paquet npm de Acme Corp | ArtiFerris</title>"), "{french}");
        assert!(french.contains(r#"<html lang="fr">"#) && french.contains(r#"<meta property="og:locale" content="fr_FR">"#), "{french}");
        assert!(german.contains("<title>left-pad: npm-Paket von Acme Corp | ArtiFerris</title>") && german.contains(r#"<html lang="de">"#), "{german}");
        for english in [&unknown, &none] {
            assert!(english.contains("<title>left-pad: npm package by Acme Corp | ArtiFerris</title>"), "{english}");
            assert!(english.contains(r#"<html lang="en">"#) && english.contains(r#"<meta property="og:locale" content="en_US">"#), "{english}");
        }
        let canonical = |html: &str| html.lines().find(|l| l.contains("canonical")).map(str::trim).map(str::to_string);
        assert_eq!(canonical(&french), canonical(&german), "one canonical URL, whatever the language");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_private_or_unknown_target_gets_the_same_generic_head_and_reveals_nothing(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        enable_indexing(&state).await;

        for uri in ["/o/acme/closed-npm/packages/npm/secret-pad", "/o/acme/closed-npm", "/o/nobody/x/packages/npm/y", "/@nobody", "/login", "/repositories/123", "/o/acme/open-npm/packages/npm/never-published", "/o/acme/open-npm/packages/npm/left%00pad", "/o/ac%00me", "/@al%00ice/lib", "/o/acme/op%00en"] {
            let (status, _, html) = get(&state, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert!(html.contains("<title>ArtiFerris · Artifact Repository</title>") && html.contains("noindex, nofollow"), "{uri}: {html}");
            assert!(!html.contains("secret-pad") && !html.contains("canonical"), "{uri}: {html}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn with_indexing_off_every_page_is_noindex_and_robots_and_the_sitemap_are_closed(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;

        let (_, _, page) = get(&state, "/o/acme").await;
        let (_, _, robots) = get(&state, "/robots.txt").await;
        let (_, _, sitemap) = get(&state, "/sitemap.xml").await;
        let (page_status, _, _) = get(&state, "/sitemap-1.xml").await;

        assert!(page.contains("noindex, nofollow"));
        assert_eq!(robots, "User-agent: *\nDisallow: /\n");
        assert!(!sitemap.contains("<loc>"), "{sitemap}");
        assert_eq!(page_status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn with_indexing_on_the_sitemap_lists_public_pages_only(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        enable_indexing(&state).await;

        let (_, headers, index) = get(&state, "/sitemap.xml").await;
        let (page_status, _, page) = get(&state, "/sitemap-1.xml").await;
        let (missing_status, _, _) = get(&state, "/sitemap-2.xml").await;
        let (_, _, robots) = get(&state, "/robots.txt").await;

        assert_eq!(headers[header::CONTENT_TYPE], "application/xml; charset=utf-8");
        assert!(index.contains("<loc>http://localhost:4200/sitemap-1.xml</loc>"));
        assert_eq!(page_status, StatusCode::OK);
        assert!(page.contains("/o/acme/open-npm/packages/npm/left-pad") && page.contains("<loc>http://localhost:4200/o/acme</loc>"), "{page}");
        assert!(!page.contains("closed-npm") && !page.contains("secret-pad"), "{page}");
        assert_eq!(missing_status, StatusCode::NOT_FOUND);
        assert!(robots.contains("Sitemap: http://localhost:4200/sitemap.xml"));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_search_result_page_is_noindex_even_when_indexing_is_on(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        enable_indexing(&state).await;

        let (_, _, html) = get(&state, "/artiferris-npm?q=pad").await;
        let (_, _, plain) = get(&state, "/artiferris-npm").await;

        assert!(html.contains("noindex, follow") && html.contains("<title>artiferris-npm: public npm packages | ArtiFerris</title>"), "{html}");
        assert!(plain.contains("index, follow"), "{plain}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn api_registry_and_asset_paths_are_404_not_the_app(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;

        for uri in [
            "/api/nothing-here",
            "/npm/x/y",
            "/v2/x/blobs/y",
            "/main-ABC123.js",
            "/assets/logo.svg",
            "/favicon.ico",
            "/styles-ABC123.css",
            "/chunk-ABC123.js.map",
            "/fonts/Inter.woff2",
            "/i18n/fr.json",
            "/media/no-extension",
            "/some/deep/logo.PNG",
        ] {
            assert_eq!(get(&state, uri).await.0, StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn packages_repositories_and_images_with_dots_in_their_names_are_pages_of_the_app(pool: sqlx::PgPool) {
        let (state, admin) = app_with(pool).await;
        enable_indexing(&state).await;
        let acme = state.organizations.find_by_slug(&artiferris_domain::organization::OrganizationSlug::parse("acme").unwrap()).await.unwrap().unwrap().id;
        let dotted = state.create_repository.execute(acme, "lib.name", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin).await.unwrap();
        state.set_repository_visibility.execute(dotted, true, admin).await.unwrap();
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: dotted, name: NpmPackageName::parse("socket.io").unwrap(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        state.npm_packages.create_package(&package).await.unwrap();
        state
            .npm_packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: NpmVersion::parse("4.0.0").unwrap(),
                manifest: json!({ "description": "Realtime engine" }),
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

        for uri in [
            "/o/acme/lib.name/packages/npm/socket.io",
            "/o/acme/lib.name/packages/npm/chart.js",
            "/o/acme/lib.name",
            "/o/acme/images/packages/docker/team%2Fmy.image",
            "/o/acme/images/packages/docker/my.image",
            "/@alice/lib/packages/npm/chart.js",
            "/@alice/my.lib",
            "/@alice/lib/packages/npm/socket.io?q=x",
        ] {
            let (status, headers, html) = get(&state, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8", "{uri}");
            assert!(html.contains("<app-root>"), "{uri}: {html}");
        }
        let (_, _, html) = get(&state, "/o/acme/lib.name/packages/npm/socket.io").await;
        assert!(html.contains("<title>socket.io: npm package by Acme Corp | ArtiFerris</title>") && html.contains(r#"content="index, follow""#), "{html}");
        let (_, _, sitemap) = get(&state, "/sitemap-1.xml").await;
        assert!(sitemap.contains("/o/acme/lib.name/packages/npm/socket.io"), "the sitemap advertises it, so it must not 404: {sitemap}");
    }


    async fn get_via(app: &Router, uri: &str) -> (StatusCode, String) {
        let response = app.clone().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        (status, String::from_utf8(axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap())
    }

    /// Answers the indexing switch from a script, `None` being a database error; the last step repeats.
    struct ScriptedSettings {
        script: Mutex<Vec<Option<bool>>>,
        reads: AtomicUsize,
    }

    impl ScriptedSettings {
        fn new(script: Vec<Option<bool>>) -> Arc<Self> {
            Arc::new(Self { script: Mutex::new(script), reads: AtomicUsize::new(0) })
        }
    }

    #[async_trait]
    impl SystemSettingsPort for ScriptedSettings {
        async fn get(&self, _organization_id: Uuid) -> Result<SystemSettings, DomainError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let mut script = self.script.lock().unwrap();
            let step = if script.len() > 1 { script.remove(0) } else { script[0] };
            step.map(|enabled| SystemSettings { seo_indexing_enabled: enabled, ..SystemSettings::defaults() }).ok_or_else(|| DomainError::Infrastructure("database is down".to_string()))
        }
        async fn update(&self, _organization_id: Uuid, _settings: &SystemSettings, _audit: Option<&artiferris_domain::audit::AdminAuditRecord>) -> Result<(), DomainError> {
            unreachable!()
        }
    }

    fn with_settings(state: &mut AppState, settings: &Arc<ScriptedSettings>) {
        state.get_system_settings = Arc::new(GetSystemSettingsUseCase::new(settings.clone()));
    }

    /// A catalog whose owner lookups take `delay` and whose sitemap waits on a gate, so tests decide what overlaps.
    struct SlowCatalog {
        delay: Duration,
        gate: Semaphore,
        sitemap_calls: AtomicUsize,
        last_limit: AtomicUsize,
        rows: usize,
        failing: AtomicBool,
        lookups_fail: AtomicBool,
    }

    impl SlowCatalog {
        fn new(delay: Duration, open_gate_permits: usize, rows: usize) -> Arc<Self> {
            Arc::new(Self { delay, gate: Semaphore::new(open_gate_permits), sitemap_calls: AtomicUsize::new(0), last_limit: AtomicUsize::new(0), rows, failing: AtomicBool::new(false), lookups_fail: AtomicBool::new(false) })
        }
    }

    #[async_trait]
    impl PublicCatalogPort for SlowCatalog {
        async fn search(&self, _query: &CatalogQuery) -> Result<CatalogPage, DomainError> {
            Ok(CatalogPage { items: vec![], total: 0 })
        }
        async fn entry_counts(&self) -> Result<Vec<CatalogEntryCount>, DomainError> {
            Ok(vec![])
        }
        async fn suggest(&self, _query: &artiferris_domain::public_catalog::SuggestQuery) -> Result<Vec<CatalogSuggestion>, DomainError> {
            Ok(vec![])
        }
        async fn repository(&self, _owner: &OwnerRef, _repository: &str) -> Result<Option<CatalogRepository>, DomainError> {
            Ok(None)
        }
        async fn find_entry(&self, _owner: &OwnerRef, _repository: &str, _format: RepositoryFormat, _name: &str) -> Result<Option<CatalogEntry>, DomainError> {
            Ok(None)
        }
        async fn sitemap_entries(&self, limit: usize) -> Result<Vec<SitemapEntry>, DomainError> {
            self.sitemap_calls.fetch_add(1, Ordering::SeqCst);
            self.last_limit.store(limit, Ordering::SeqCst);
            self.gate.acquire().await.unwrap().forget();
            if self.failing.load(Ordering::SeqCst) {
                return Err(DomainError::Infrastructure("database is down".to_string()));
            }
            let owner = OwnerRef { kind: OwnerKind::Personal, slug: "alice".into() };
            Ok((0..self.rows.min(limit)).map(|i| entry(SitemapTarget::Package { repository: "lib".into(), format: RepositoryFormat::Npm, name: format!("p{i}") }, owner.clone())).collect())
        }
        async fn owner_summary(&self, _owner: &OwnerRef) -> Result<Option<OwnerSummary>, DomainError> {
            tokio::time::sleep(self.delay).await;
            if self.lookups_fail.load(Ordering::SeqCst) {
                return Err(DomainError::Infrastructure("database is down".to_string()));
            }
            Ok(None)
        }
    }

    async fn wait_until(what: &str, condition: impl Fn() -> bool) {
        for _ in 0..500 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_head_that_takes_too_long_is_replaced_by_the_generic_one_so_the_page_still_loads(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        state.seo_pages = Arc::new(SeoPageUseCase::new(SlowCatalog::new(Duration::from_secs(30), 0, 0), state.public_url.clone()));

        let started = Instant::now();
        let (status, _, html) = get(&state, "/@alice").await;

        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("<app-root>") && html.contains("<title>ArtiFerris · Artifact Repository</title>"), "{html}");
        assert!(!html.contains(r#"name="robots""#), "a slow moment must not de-index a page: {html}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_indexing_switch_is_read_once_and_then_remembered(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        let settings = ScriptedSettings::new(vec![Some(true)]);
        with_settings(&mut state, &settings);
        let app = router(SeoState::new(state.clone(), TEMPLATE.to_string()));

        for _ in 0..3 {
            let (status, robots) = get_via(&app, "/robots.txt").await;
            assert_eq!(status, StatusCode::OK);
            assert!(robots.contains("Sitemap:"), "{robots}");
        }
        get_via(&app, "/o/acme/open-npm/packages/npm/left-pad").await;

        assert_eq!(settings.reads.load(Ordering::SeqCst), 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_switch_that_cannot_be_read_keeps_its_last_known_value(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        let settings = ScriptedSettings::new(vec![Some(true), None]);
        with_settings(&mut state, &settings);
        let app = router(SeoState { indexing_ttl: Duration::ZERO, ..SeoState::new(state.clone(), TEMPLATE.to_string()) });

        let (_, first) = get_via(&app, "/robots.txt").await;
        let (status, second) = get_via(&app, "/robots.txt").await;
        let (_, page) = get_via(&app, "/o/acme/open-npm/packages/npm/left-pad").await;

        assert!(settings.reads.load(Ordering::SeqCst) >= 3, "each request tried to refresh");
        assert_eq!((status, &second), (StatusCode::OK, &first), "a blip does not close the catalog to crawlers");
        assert!(page.contains(r#"content="index, follow""#), "{page}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn robots_and_the_sitemap_answer_503_when_the_switch_was_never_readable_and_pages_still_load(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        with_settings(&mut state, &ScriptedSettings::new(vec![None]));
        let app = router(SeoState::new(state.clone(), TEMPLATE.to_string()));

        for uri in ["/robots.txt", "/sitemap.xml", "/sitemap-1.xml"] {
            assert_eq!(get_via(&app, uri).await.0, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        }
        let (status, page) = get_via(&app, "/o/acme/open-npm/packages/npm/left-pad").await;
        assert_eq!(status, StatusCode::OK);
        assert!(page.contains("<app-root>") && !page.contains(r#"name="robots""#), "an unreadable switch is no reason to de-index a page: {page}");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn concurrent_requests_share_one_sitemap_rebuild(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 0, 3);
        state.public_catalog = catalog.clone();
        let app = router(SeoState::new(state.clone(), TEMPLATE.to_string()));

        let mut requests = tokio::task::JoinSet::new();
        for _ in 0..6 {
            let app = app.clone();
            requests.spawn(async move { get_via(&app, "/sitemap.xml").await });
        }
        wait_until("the first rebuild to start", || catalog.sitemap_calls.load(Ordering::SeqCst) == 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(catalog.sitemap_calls.load(Ordering::SeqCst), 1, "the others wait for it instead of starting their own");
        catalog.gate.add_permits(1);

        while let Some(response) = requests.join_next().await {
            let (status, body) = response.unwrap();
            assert_eq!(status, StatusCode::OK);
            assert!(body.contains("sitemap-1.xml"), "{body}");
        }
        assert_eq!(catalog.sitemap_calls.load(Ordering::SeqCst), 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn while_the_sitemap_is_rebuilt_the_previous_copy_is_served_and_a_failed_rebuild_keeps_it(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 1, 3);
        state.public_catalog = catalog.clone();
        let app = router(SeoState { sitemap_ttl: Duration::ZERO, ..SeoState::new(state.clone(), TEMPLATE.to_string()) });
        let (status, first) = get_via(&app, "/sitemap.xml").await;
        assert_eq!(status, StatusCode::OK);

        let rebuild = tokio::spawn({
            let app = app.clone();
            async move { get_via(&app, "/sitemap.xml").await }
        });
        wait_until("the second rebuild to start", || catalog.sitemap_calls.load(Ordering::SeqCst) == 2).await;
        let during = tokio::time::timeout(Duration::from_secs(5), get_via(&app, "/sitemap.xml")).await.expect("served without waiting for the rebuild");

        assert_eq!(during, (StatusCode::OK, first.clone()));
        catalog.failing.store(true, Ordering::SeqCst);
        catalog.gate.add_permits(2);
        assert_eq!(rebuild.await.unwrap().0, StatusCode::OK, "a rebuild that fails still answers with the previous copy");
        assert_eq!(get_via(&app, "/sitemap.xml").await, (StatusCode::OK, first));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_sitemap_with_no_previous_copy_and_no_database_is_a_503(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 1, 3);
        catalog.failing.store(true, Ordering::SeqCst);
        state.public_catalog = catalog;
        let app = router(SeoState::new(state.clone(), TEMPLATE.to_string()));

        assert_eq!(get_via(&app, "/sitemap.xml").await.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_failed_sitemap_build_is_not_retried_before_the_delay_and_the_waiters_get_a_503(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 100, 3);
        catalog.failing.store(true, Ordering::SeqCst);
        state.public_catalog = catalog.clone();
        let app = router(SeoState { sitemap_retry_delay: Duration::from_millis(300), ..SeoState::new(state.clone(), TEMPLATE.to_string()) });

        for _ in 0..5 {
            assert_eq!(get_via(&app, "/sitemap.xml").await.0, StatusCode::SERVICE_UNAVAILABLE);
        }
        assert_eq!(catalog.sitemap_calls.load(Ordering::SeqCst), 1, "one attempt, not one per waiting request");

        catalog.failing.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(get_via(&app, "/sitemap.xml").await.0, StatusCode::OK);
        assert_eq!(catalog.sitemap_calls.load(Ordering::SeqCst), 2);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_expired_sitemap_is_served_while_a_failing_rebuild_is_not_hammered(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 100, 3);
        state.public_catalog = catalog.clone();
        let app = router(SeoState { sitemap_ttl: Duration::ZERO, ..SeoState::new(state.clone(), TEMPLATE.to_string()) });
        let (_, first) = get_via(&app, "/sitemap.xml").await;
        catalog.failing.store(true, Ordering::SeqCst);

        for _ in 0..5 {
            assert_eq!(get_via(&app, "/sitemap.xml").await, (StatusCode::OK, first.clone()));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(catalog.sitemap_calls.load(Ordering::SeqCst), 2, "the first build and a single failed refresh");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_client_that_gives_up_does_not_cancel_the_sitemap_build(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 0, 3);
        state.public_catalog = catalog.clone();
        let seo = SeoState::new(state.clone(), TEMPLATE.to_string());
        let request = tokio::spawn({
            let seo = seo.clone();
            async move { seo.sitemap().await.is_ok() }
        });
        wait_until("the build to start", || catalog.sitemap_calls.load(Ordering::SeqCst) == 1).await;

        request.abort();
        let _ = request.await;
        catalog.gate.add_permits(1);

        wait_until("the abandoned build to finish", || seo.any_sitemap().is_some()).await;
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_sitemap_files_have_a_per_client_budget(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let app = router(SeoState::new(state.clone(), TEMPLATE.to_string()));
        for i in 0..SITEMAP_REQUESTS_PER_MINUTE {
            let uri = if i % 2 == 0 { "/sitemap.xml" } else { "/sitemap-1.xml" };
            assert_eq!(get_via(&app, uri).await.0, StatusCode::OK, "{uri} {i}");
        }

        for uri in ["/sitemap.xml", "/sitemap-1.xml"] {
            let response = app.clone().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "{uri}");
            assert_eq!(response.headers()[header::RETRY_AFTER], "60");
        }
        assert_eq!(get_via(&app, "/robots.txt").await.0, StatusCode::OK, "robots.txt is not part of the sitemap budget");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_sitemap_lists_at_most_the_cap_and_only_asks_the_database_for_one_more(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 1, 10);
        state.public_catalog = catalog.clone();
        let app = router(SeoState { max_sitemap_entries: 2, ..SeoState::new(state.clone(), TEMPLATE.to_string()) });

        let (status, page) = get_via(&app, "/sitemap-1.xml").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(page.matches("<url>").count(), 3 + 2, "the three fixed pages and two entries: {page}");
        assert_eq!(catalog.last_limit.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn only_known_asset_extensions_or_asset_folders_look_like_files() {
        for path in ["/main-ABC.js", "/a/b/c.css", "/favicon.ico", "/x.SVG", "/assets/anything", "/fonts/x", "/i18n/fr.json", "/media/y"] {
            assert!(looks_like_a_static_file(path), "{path}");
        }
        for path in ["/o/acme/lib/packages/npm/socket.io", "/@alice/my.lib", "/explorer", "/", "/o/acme/lib/packages/npm/chart.jsx", "/o/acme/lib.name"] {
            assert!(!looks_like_a_static_file(path), "{path}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_client_over_the_page_budget_still_gets_the_app_with_the_generic_head(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let app = router(SeoState::new(state.clone(), TEMPLATE.to_string()));
        let request = || Request::builder().uri("/o/acme").body(Body::empty()).unwrap();
        for _ in 0..PAGE_HEADS_PER_MINUTE {
            app.clone().oneshot(request()).await.unwrap();
        }

        let response = app.oneshot(request()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let response_cache_control = response.headers()[header::CACHE_CONTROL].clone();
        let html = String::from_utf8(axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
        assert!(html.contains("<app-root>") && !html.contains(r#"name="robots""#) && !html.contains("canonical"), "{html}");
        assert_eq!(response_cache_control, "no-store");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_signed_in_pages_of_the_app_are_served_whatever_the_last_segment_looks_like(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        let id = Uuid::new_v4();

        for uri in [
            format!("/repositories/{id}/packages/npm/chart.js"),
            format!("/repositories/{id}/packages/npm/three.js"),
            format!("/repositories/{id}/packages/npm/x.json"),
            format!("/repositories/{id}/packages/npm/y.css"),
            format!("/repositories/{id}/packages/npm/%40scope%2Fhighlight.js"),
            format!("/repositories/{id}/packages/docker/team%2Fmy.image"),
            format!("/repositories/{id}"),
            "/users/a.json".to_string(),
            "/admin/organizations/acme.js".to_string(),
        ] {
            let (status, _, html) = get(&state, &uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert!(html.contains("<app-root>") && html.contains("noindex, nofollow"), "{uri}: {html}");
        }
        for uri in ["/repositories/x/main.js", "/admin/whatever.css", "/repositories/x/y/z/logo.png"] {
            assert_eq!(get(&state, uri).await.0, StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_lookup_that_fails_never_turns_into_noindex_while_the_instance_is_open(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 0, 0);
        catalog.lookups_fail.store(true, Ordering::SeqCst);
        state.seo_pages = Arc::new(SeoPageUseCase::new(catalog, state.public_url.clone()));

        let (status, headers, html) = get(&state, "/@alice").await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("<app-root>") && !html.contains(r#"name="robots""#) && !html.contains("canonical"), "{html}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_lookup_that_fails_on_a_closed_instance_still_says_noindex(pool: sqlx::PgPool) {
        let (mut state, _) = app_with(pool).await;
        let catalog = SlowCatalog::new(Duration::ZERO, 0, 0);
        catalog.lookups_fail.store(true, Ordering::SeqCst);
        state.seo_pages = Arc::new(SeoPageUseCase::new(catalog, state.public_url.clone()));

        let (_, headers, html) = get(&state, "/@alice").await;

        assert!(html.contains(r#"<meta name="robots" content="noindex, nofollow">"#), "{html}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache", "the answer is known, so it is not a degraded head");
    }

    fn static_dir_with_index() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("artiferris-static-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), TEMPLATE).unwrap();
        std::fs::write(dir.join("main-ABC.js"), "console.log(1)").unwrap();
        dir
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_root_and_index_html_get_the_generic_head_like_any_other_page(pool: sqlx::PgPool) {
        let (state, _) = app_with(pool).await;
        enable_indexing(&state).await;
        let dir = static_dir_with_index();
        let app = app_router(dir.to_str().unwrap(), SeoState::new(state.clone(), TEMPLATE.to_string()));

        for uri in ["/", "/index.html", "/?q=x"] {
            let (status, html) = get_via(&app, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert!(html.contains("<app-root>") && html.contains(r#"<meta name="robots" content="noindex, nofollow">"#), "{uri}: {html}");
        }
        let (status, script) = get_via(&app, "/main-ABC.js").await;
        assert_eq!((status, script.as_str()), (StatusCode::OK, "console.log(1)"), "real files are still served as they are");
        assert_eq!(get_via(&app, "/missing-DEF.js").await.0, StatusCode::NOT_FOUND);
        assert!(get_via(&app, "/explorer").await.1.contains(r#"content="index, follow""#));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
