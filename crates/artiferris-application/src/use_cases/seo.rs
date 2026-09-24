use std::sync::Arc;

use artiferris_domain::package_repository::RepositoryFormat;
use artiferris_domain::public_catalog::{catalog_format_spec, CatalogEntry, OwnerKind, OwnerRef, PublicCatalogPort, CATALOG_FORMATS};
use serde_json::{json, Value};

use crate::error::ApplicationError;
use crate::use_cases::public_catalog::{format_key, has_control_character};

pub const SITE_NAME: &str = "ArtiFerris";
/// What the app's `index.html` already carries, kept for every page that is not a public catalog page.
pub const DEFAULT_TITLE: &str = "ArtiFerris · Artifact Repository";
const MAX_DESCRIPTION_CHARS: usize = 160;
/// The longest a URL segment can be and still name something that exists. Longer or garbled ones get the generic head
/// without a lookup, since the URL is the only thing bounding them.
const MAX_OWNER_BYTES: usize = 128;
const MAX_REPOSITORY_BYTES: usize = 64;
const MAX_NPM_NAME_BYTES: usize = 214;
const MAX_DOCKER_NAME_BYTES: usize = 128;

/// The public catalog pages, recognised from the URL the SPA serves them on. `App` is a signed-in page of the app
/// itself (its last segment can be free text, like a package called `chart.js`). Anything else is `Other`, including
/// `/<user>/<repo>` without the `@` (which the app treats as a personal repository but which is ambiguous with app routes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeoRoute {
    Home,
    Explorer,
    Catalog { format: RepositoryFormat },
    Owner(OwnerRef),
    Repository { owner: OwnerRef, repository: String },
    Package { owner: OwnerRef, repository: String, format: RepositoryFormat, name: String },
    App,
    Other,
}

/// `segments` are the URL path segments, already percent-decoded.
pub fn parse_route(segments: &[String]) -> SeoRoute {
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    match segments.as_slice() {
        [] => SeoRoute::Home,
        ["explorer"] => SeoRoute::Explorer,
        ["repositories", _] | ["repositories", _, "packages", _, _] | ["users", _] | ["admin", "organizations", _] => SeoRoute::App,
        [catalog] => {
            if let Some(spec) = CATALOG_FORMATS.iter().find(|spec| spec.catalog_name == *catalog) {
                return SeoRoute::Catalog { format: spec.format };
            }
            personal_owner(catalog).map_or(SeoRoute::Other, SeoRoute::Owner)
        }
        ["o", slug] => SeoRoute::Owner(organization(slug)),
        ["o", slug, repository] => SeoRoute::Repository { owner: organization(slug), repository: (*repository).to_string() },
        ["o", slug, repository, "packages", format, name] => package(organization(slug), repository, format, name),
        [user, repository] => personal_owner(user).map_or(SeoRoute::Other, |owner| SeoRoute::Repository { owner, repository: (*repository).to_string() }),
        [user, repository, "packages", format, name] => personal_owner(user).map_or(SeoRoute::Other, |owner| package(owner, repository, format, name)),
        _ => SeoRoute::Other,
    }
}

fn personal_owner(segment: &str) -> Option<OwnerRef> {
    segment.strip_prefix('@').filter(|username| !username.is_empty()).map(|username| OwnerRef { kind: OwnerKind::Personal, slug: username.to_string() })
}

fn organization(slug: &str) -> OwnerRef {
    OwnerRef { kind: OwnerKind::Organization, slug: slug.to_string() }
}

fn package(owner: OwnerRef, repository: &str, format: &str, name: &str) -> SeoRoute {
    match parse_format(format) {
        Some(format) => SeoRoute::Package { owner, repository: repository.to_string(), format, name: name.to_string() },
        None => SeoRoute::Other,
    }
}

fn parse_format(format: &str) -> Option<RepositoryFormat> {
    CATALOG_FORMATS.iter().find(|spec| format_key(spec.format) == format).map(|spec| spec.format)
}

/// Encodes one path segment the way the Angular router does, so a canonical URL is the very URL the app links to.
pub fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'@' | b':' | b'$' | b',' | b'&' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

pub fn owner_path(owner: &OwnerRef) -> String {
    match owner.kind {
        OwnerKind::Personal => format!("/@{}", encode_segment(&owner.slug)),
        OwnerKind::Organization => format!("/o/{}", encode_segment(&owner.slug)),
    }
}

pub fn repository_path(owner: &OwnerRef, repository: &str) -> String {
    format!("{}/{}", owner_path(owner), encode_segment(repository))
}

pub fn package_path(owner: &OwnerRef, repository: &str, format: RepositoryFormat, name: &str) -> String {
    format!("{}/packages/{}/{}", repository_path(owner, repository), format_key(format), encode_segment(name))
}

pub fn catalog_path(format: RepositoryFormat) -> Option<String> {
    catalog_format_spec(format).map(|spec| format!("/{}", spec.catalog_name))
}

/// What a page needs in its `<head>`.
#[derive(Debug, Clone, PartialEq)]
pub struct PageMeta {
    pub title: String,
    pub description: Option<String>,
    pub canonical_path: Option<String>,
    /// False for anything that isn't a resolvable public page. Says nothing about the instance's indexing switch.
    pub indexable: bool,
    /// Schema.org data, without any markup escaping.
    pub structured_data: Option<Value>,
}

impl PageMeta {
    /// The head every non-catalog page (and every unknown or private target) gets: nothing that reveals whether it exists.
    pub fn generic() -> Self {
        Self { title: DEFAULT_TITLE.to_string(), description: None, canonical_path: None, indexable: false, structured_data: None }
    }
}

pub struct SeoPageUseCase {
    catalog: Arc<dyn PublicCatalogPort>,
    /// The instance's public URL, for absolute URLs in structured data.
    base_url: String,
}

impl SeoPageUseCase {
    pub fn new(catalog: Arc<dyn PublicCatalogPort>, base_url: String) -> Self {
        Self { catalog, base_url: base_url.trim_end_matches('/').to_string() }
    }

    pub async fn execute(&self, route: &SeoRoute) -> Result<PageMeta, ApplicationError> {
        if !could_name_something(route) {
            return Ok(PageMeta::generic());
        }
        Ok(match route {
            SeoRoute::Home | SeoRoute::App | SeoRoute::Other => PageMeta::generic(),
            SeoRoute::Explorer => PageMeta {
                title: title("Explorer les paquets publics"),
                description: Some("Recherchez parmi les paquets npm et les images Docker publics hébergés sur ArtiFerris.".to_string()),
                canonical_path: Some("/explorer".to_string()),
                indexable: true,
                structured_data: None,
            },
            SeoRoute::Catalog { format } => self.catalog_meta(*format).await?,
            SeoRoute::Owner(owner) => self.owner_meta(owner).await?,
            SeoRoute::Repository { owner, repository } => self.repository_meta(owner, repository).await?,
            SeoRoute::Package { owner, repository, format, name } => self.package_meta(owner, repository, *format, name).await?,
        })
    }

    async fn catalog_meta(&self, format: RepositoryFormat) -> Result<PageMeta, ApplicationError> {
        let Some(spec) = catalog_format_spec(format) else { return Ok(PageMeta::generic()) };
        let count = self.catalog.entry_counts().await?.into_iter().find(|c| c.format == format).map_or(0, |c| c.entry_count);
        let noun = match format {
            RepositoryFormat::Npm => "paquets npm",
            RepositoryFormat::Docker => "images Docker",
        };
        Ok(PageMeta {
            title: title(&format!("{} : {noun} publics", spec.catalog_name)),
            description: Some(truncate(&format!("Recherchez parmi les {count} {noun} publics hébergés sur ArtiFerris. On installe depuis l'URL de chaque propriétaire."))),
            canonical_path: catalog_path(format),
            indexable: true,
            structured_data: None,
        })
    }

    async fn owner_meta(&self, owner: &OwnerRef) -> Result<PageMeta, ApplicationError> {
        let Some(summary) = self.catalog.owner_summary(owner).await? else { return Ok(PageMeta::generic()) };
        Ok(PageMeta {
            title: title(&format!("{} : paquets et images publics", summary.display_name)),
            description: Some(truncate(&format!(
                "{} publie {} et {} sur ArtiFerris.",
                summary.display_name,
                plural(summary.package_count, "paquet", "paquets"),
                plural(summary.image_count, "image", "images")
            ))),
            canonical_path: Some(owner_path(owner)),
            indexable: true,
            structured_data: None,
        })
    }

    async fn repository_meta(&self, owner: &OwnerRef, repository: &str) -> Result<PageMeta, ApplicationError> {
        let (Some(found), Some(summary)) = (self.catalog.repository(owner, repository).await?, self.catalog.owner_summary(owner).await?) else {
            return Ok(PageMeta::generic());
        };
        let what = match found.format {
            RepositoryFormat::Npm => "npm",
            RepositoryFormat::Docker => "Docker",
        };
        Ok(PageMeta {
            title: title(&format!("{} / {} : dépôt {what} public", summary.display_name, found.name)),
            description: Some(truncate(&format!("Dépôt {what} public « {} » de {} sur ArtiFerris.", found.name, summary.display_name))),
            canonical_path: Some(repository_path(owner, repository)),
            indexable: true,
            structured_data: None,
        })
    }

    async fn package_meta(&self, owner: &OwnerRef, repository: &str, format: RepositoryFormat, name: &str) -> Result<PageMeta, ApplicationError> {
        let Some(entry) = self.catalog.find_entry(owner, repository, format, name).await? else {
            return Ok(PageMeta::generic());
        };
        let canonical_path = package_path(owner, repository, format, name);
        let description = package_description(&entry);
        let structured_data = Some(self.structured_data(&entry, &canonical_path, &description));
        let what = match format {
            RepositoryFormat::Npm => "paquet npm",
            RepositoryFormat::Docker => "image Docker",
        };
        Ok(PageMeta { title: title(&format!("{name} : {what} de {}", entry.owner.display_name)), description: Some(description), canonical_path: Some(canonical_path), indexable: true, structured_data })
    }

    fn structured_data(&self, entry: &CatalogEntry, canonical_path: &str, description: &str) -> Value {
        let author_type = match entry.owner.kind {
            OwnerKind::Personal => "Person",
            OwnerKind::Organization => "Organization",
        };
        let mut data = json!({
            "@context": "https://schema.org",
            "@type": match entry.format {
                RepositoryFormat::Npm => "SoftwareSourceCode",
                RepositoryFormat::Docker => "SoftwareApplication",
            },
            "name": entry.name,
            "description": description,
            "url": format!("{}{canonical_path}", self.base_url),
            "dateModified": entry.updated_at.to_rfc3339(),
            "author": { "@type": author_type, "name": entry.owner.display_name },
        });
        if entry.format == RepositoryFormat::Docker {
            data["applicationCategory"] = json!("DeveloperApplication");
        }
        if let Some(version) = &entry.latest {
            data["version"] = json!(version);
        }
        data
    }
}

fn plausible(segment: &str, max_bytes: usize) -> bool {
    !segment.is_empty() && segment.len() <= max_bytes && !has_control_character(segment)
}

/// False for a route whose segments could not be a real owner, repository or package name.
fn could_name_something(route: &SeoRoute) -> bool {
    match route {
        SeoRoute::Owner(owner) => plausible(&owner.slug, MAX_OWNER_BYTES),
        SeoRoute::Repository { owner, repository } => plausible(&owner.slug, MAX_OWNER_BYTES) && plausible(repository, MAX_REPOSITORY_BYTES),
        SeoRoute::Package { owner, repository, format, name } => {
            let max_name = match format {
                RepositoryFormat::Npm => MAX_NPM_NAME_BYTES,
                RepositoryFormat::Docker => MAX_DOCKER_NAME_BYTES,
            };
            plausible(&owner.slug, MAX_OWNER_BYTES) && plausible(repository, MAX_REPOSITORY_BYTES) && plausible(name, max_name)
        }
        SeoRoute::Home | SeoRoute::Explorer | SeoRoute::Catalog { .. } | SeoRoute::App | SeoRoute::Other => true,
    }
}

fn package_description(entry: &CatalogEntry) -> String {
    let own = entry.description.as_deref().map(str::trim).filter(|d| !d.is_empty());
    match (own, entry.format) {
        (Some(description), _) => truncate(description),
        (None, RepositoryFormat::Npm) => truncate(&format!("Paquet npm {}{} publié par {} sur ArtiFerris.", entry.name, version_suffix(entry), entry.owner.display_name)),
        (None, RepositoryFormat::Docker) => truncate(&format!("Image Docker {}{} publiée par {} sur ArtiFerris.", entry.name, version_suffix(entry), entry.owner.display_name)),
    }
}

fn version_suffix(entry: &CatalogEntry) -> String {
    entry.latest.as_ref().map(|v| format!(" ({v})")).unwrap_or_default()
}

fn title(page: &str) -> String {
    format!("{page} | {SITE_NAME}")
}

fn plural(count: i64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn truncate(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_DESCRIPTION_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(MAX_DESCRIPTION_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use artiferris_domain::error::DomainError;
    use artiferris_domain::public_catalog::{CatalogEntryCount, CatalogMatch, CatalogOwner, CatalogPage, CatalogQuery, CatalogRepository, CatalogSuggestion, OwnerSummary, SitemapEntry};
    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;

    fn segments(path: &str) -> Vec<String> {
        path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect()
    }

    fn route(path: &str) -> SeoRoute {
        parse_route(&segments(path))
    }

    fn personal(name: &str) -> OwnerRef {
        OwnerRef { kind: OwnerKind::Personal, slug: name.to_string() }
    }

    fn org(slug: &str) -> OwnerRef {
        OwnerRef { kind: OwnerKind::Organization, slug: slug.to_string() }
    }

    #[test]
    fn recognises_every_public_catalog_page() {
        assert_eq!(route("/"), SeoRoute::Home);
        assert_eq!(route("/explorer"), SeoRoute::Explorer);
        assert_eq!(route("/artiferris-npm"), SeoRoute::Catalog { format: RepositoryFormat::Npm });
        assert_eq!(route("/artiferris-docker"), SeoRoute::Catalog { format: RepositoryFormat::Docker });
        assert_eq!(route("/@alice"), SeoRoute::Owner(personal("alice")));
        assert_eq!(route("/o/acme"), SeoRoute::Owner(org("acme")));
        assert_eq!(route("/@alice/lib"), SeoRoute::Repository { owner: personal("alice"), repository: "lib".into() });
        assert_eq!(route("/o/acme/libs"), SeoRoute::Repository { owner: org("acme"), repository: "libs".into() });
        assert_eq!(route("/@alice/lib/packages/npm/@scope/name"), SeoRoute::Other, "a scoped name arrives as ONE encoded segment, never as two");
        assert_eq!(
            route("/@alice/lib/packages/npm/left-pad"),
            SeoRoute::Package { owner: personal("alice"), repository: "lib".into(), format: RepositoryFormat::Npm, name: "left-pad".into() }
        );
        assert_eq!(
            parse_route(&["o".into(), "acme".into(), "images".into(), "packages".into(), "docker".into(), "team/api".into()]),
            SeoRoute::Package { owner: org("acme"), repository: "images".into(), format: RepositoryFormat::Docker, name: "team/api".into() }
        );
    }

    #[test]
    fn app_routes_and_lookalikes_are_not_public_pages() {
        for path in ["/login", "/repositories", "/admin/health", "/alice/lib", "/@", "/o", "/o/acme/libs/extra", "/@alice/lib/packages/helm/x", "/@alice/lib/packages/npm", "/repositories/1/packages/npm"] {
            assert_eq!(route(path), SeoRoute::Other, "{path}");
        }
    }

    #[test]
    fn the_signed_in_pages_with_a_free_text_segment_are_app_routes() {
        for path in ["/repositories/123", "/repositories/123/packages/npm/chart.js", "/repositories/123/packages/docker/team%2Fapi", "/users/1", "/admin/organizations/7"] {
            assert_eq!(route(path), SeoRoute::App, "{path}");
        }
    }

    #[test]
    fn canonical_paths_are_encoded_like_the_app_encodes_its_links() {
        assert_eq!(owner_path(&personal("alice")), "/@alice");
        assert_eq!(owner_path(&org("acme")), "/o/acme");
        assert_eq!(package_path(&org("acme"), "images", RepositoryFormat::Docker, "team/api"), "/o/acme/images/packages/docker/team%2Fapi");
        assert_eq!(package_path(&personal("alice"), "lib", RepositoryFormat::Npm, "@scope/name"), "/@alice/lib/packages/npm/@scope%2Fname");
        assert_eq!(encode_segment("a b(c)é"), "a%20b%28c%29%C3%A9");
        assert_eq!(encode_segment("x&y:z$1,2"), "x&y:z$1,2");
    }

    #[derive(Default)]
    struct FakeCatalog {
        entries: Vec<CatalogEntry>,
        owner: Option<OwnerSummary>,
        repository: Option<CatalogRepository>,
        counts: Vec<CatalogEntryCount>,
        looked_up: Mutex<Vec<(OwnerRef, String, RepositoryFormat, String)>>,
    }

    #[async_trait]
    impl PublicCatalogPort for FakeCatalog {
        async fn search(&self, _query: &CatalogQuery) -> Result<CatalogPage, DomainError> {
            unreachable!("a page head never searches")
        }
        async fn entry_counts(&self) -> Result<Vec<CatalogEntryCount>, DomainError> {
            Ok(self.counts.clone())
        }
        async fn suggest(&self, _query: &artiferris_domain::public_catalog::SuggestQuery) -> Result<Vec<CatalogSuggestion>, DomainError> {
            Ok(vec![])
        }
        async fn repository(&self, _owner: &OwnerRef, _repository: &str) -> Result<Option<CatalogRepository>, DomainError> {
            Ok(self.repository.clone())
        }
        async fn find_entry(&self, owner: &OwnerRef, repository: &str, format: RepositoryFormat, name: &str) -> Result<Option<CatalogEntry>, DomainError> {
            self.looked_up.lock().unwrap().push((owner.clone(), repository.to_string(), format, name.to_string()));
            Ok(self.entries.iter().find(|e| e.name == name && e.repository_name == repository && e.format == format).cloned())
        }
        async fn sitemap_entries(&self, _limit: usize) -> Result<Vec<SitemapEntry>, DomainError> {
            Ok(vec![])
        }
        async fn owner_summary(&self, _owner: &OwnerRef) -> Result<Option<OwnerSummary>, DomainError> {
            Ok(self.owner.clone())
        }
    }

    fn entry(name: &str, description: Option<&str>) -> CatalogEntry {
        CatalogEntry {
            format: RepositoryFormat::Npm,
            name: name.to_string(),
            description: description.map(str::to_string),
            keywords: vec![],
            latest: Some("1.2.3".to_string()),
            updated_at: Utc::now(),
            downloads_7d: 0,
            match_kind: Some(CatalogMatch::Exact),
            repository_id: uuid::Uuid::nil(),
            repository_type: artiferris_domain::package_repository::RepositoryType::Hosted,
            repository_name: "lib".to_string(),
            owner: CatalogOwner { kind: OwnerKind::Personal, slug: "alice".to_string(), display_name: "alice".to_string(), is_public_organization: false },
        }
    }

    fn use_case(catalog: FakeCatalog) -> SeoPageUseCase {
        SeoPageUseCase::new(Arc::new(catalog), "https://registry.example.com/".to_string())
    }

    fn summary(display_name: &str, packages: i64, images: i64) -> OwnerSummary {
        OwnerSummary { kind: OwnerKind::Personal, slug: display_name.to_string(), display_name: display_name.to_string(), repository_count: 1, package_count: packages, image_count: images }
    }

    #[tokio::test]
    async fn other_pages_and_home_get_the_generic_head() {
        let seo = use_case(FakeCatalog::default());

        for route in [SeoRoute::Home, SeoRoute::Other] {
            assert_eq!(seo.execute(&route).await.unwrap(), PageMeta::generic());
        }
        assert!(!PageMeta::generic().indexable);
    }

    #[tokio::test]
    async fn the_explorer_and_the_catalogs_describe_themselves() {
        let seo = use_case(FakeCatalog { counts: vec![CatalogEntryCount { format: RepositoryFormat::Npm, entry_count: 12 }], ..Default::default() });

        let explorer = seo.execute(&SeoRoute::Explorer).await.unwrap();
        let catalog = seo.execute(&SeoRoute::Catalog { format: RepositoryFormat::Npm }).await.unwrap();

        assert_eq!((explorer.canonical_path.as_deref(), explorer.indexable), (Some("/explorer"), true));
        assert_eq!(catalog.title, "artiferris-npm : paquets npm publics | ArtiFerris");
        assert!(catalog.description.unwrap().contains("12 paquets npm"));
        assert_eq!(catalog.canonical_path.as_deref(), Some("/artiferris-npm"));
    }

    #[tokio::test]
    async fn an_owner_page_uses_the_display_name_and_the_counts() {
        let seo = use_case(FakeCatalog { owner: Some(summary("alice", 1, 3)), ..Default::default() });

        let meta = seo.execute(&SeoRoute::Owner(personal("alice"))).await.unwrap();

        assert_eq!(meta.title, "alice : paquets et images publics | ArtiFerris");
        assert_eq!(meta.description.as_deref(), Some("alice publie 1 paquet et 3 images sur ArtiFerris."));
        assert_eq!((meta.canonical_path.as_deref(), meta.indexable), (Some("/@alice"), true));
    }

    #[tokio::test]
    async fn an_unknown_or_private_owner_or_repository_or_package_looks_exactly_like_any_other_app_page() {
        let seo = use_case(FakeCatalog::default());

        for route in [
            SeoRoute::Owner(personal("nobody")),
            SeoRoute::Repository { owner: personal("nobody"), repository: "lib".into() },
            SeoRoute::Package { owner: personal("nobody"), repository: "lib".into(), format: RepositoryFormat::Npm, name: "x".into() },
        ] {
            assert_eq!(seo.execute(&route).await.unwrap(), PageMeta::generic(), "{route:?}");
        }
    }

    #[tokio::test]
    async fn a_repository_page_names_its_format_and_owner() {
        let seo = use_case(FakeCatalog { owner: Some(summary("alice", 1, 0)), repository: Some(CatalogRepository { name: "lib".into(), format: RepositoryFormat::Docker }), ..Default::default() });

        let meta = seo.execute(&SeoRoute::Repository { owner: personal("alice"), repository: "lib".into() }).await.unwrap();

        assert_eq!(meta.title, "alice / lib : dépôt Docker public | ArtiFerris");
        assert_eq!(meta.canonical_path.as_deref(), Some("/@alice/lib"));
    }

    #[tokio::test]
    async fn a_package_page_carries_its_description_and_structured_data() {
        let catalog = FakeCatalog { entries: vec![entry("left-pad", Some("Pads strings on the left"))], ..Default::default() };
        let seo = use_case(catalog);

        let meta = seo.execute(&SeoRoute::Package { owner: personal("alice"), repository: "lib".into(), format: RepositoryFormat::Npm, name: "left-pad".into() }).await.unwrap();

        assert_eq!(meta.title, "left-pad : paquet npm de alice | ArtiFerris");
        assert_eq!(meta.description.as_deref(), Some("Pads strings on the left"));
        let data = meta.structured_data.unwrap();
        assert_eq!(data["@type"], "SoftwareSourceCode");
        assert_eq!(data["url"], "https://registry.example.com/@alice/lib/packages/npm/left-pad");
        assert_eq!((data["version"].as_str(), data["author"]["@type"].as_str(), data["author"]["name"].as_str()), (Some("1.2.3"), Some("Person"), Some("alice")));
    }

    #[tokio::test]
    async fn a_package_without_a_description_gets_a_sentence_built_from_what_is_known() {
        let seo = use_case(FakeCatalog { entries: vec![entry("widget", Some("   "))], ..Default::default() });

        let meta = seo.execute(&SeoRoute::Package { owner: personal("alice"), repository: "lib".into(), format: RepositoryFormat::Npm, name: "widget".into() }).await.unwrap();

        assert_eq!(meta.description.as_deref(), Some("Paquet npm widget (1.2.3) publié par alice sur ArtiFerris."));
    }

    #[tokio::test]
    async fn a_package_from_another_repository_or_another_name_is_not_taken_for_the_one_asked() {
        let mut other_repository = entry("widget", Some("in another repo"));
        other_repository.repository_name = "elsewhere".to_string();
        let seo = use_case(FakeCatalog { entries: vec![other_repository, entry("widgets", None)], ..Default::default() });

        let meta = seo.execute(&SeoRoute::Package { owner: personal("alice"), repository: "lib".into(), format: RepositoryFormat::Npm, name: "widget".into() }).await.unwrap();

        assert_eq!(meta, PageMeta::generic());
    }

    #[tokio::test]
    async fn a_package_is_looked_up_by_its_exact_owner_repository_format_and_name() {
        let catalog = Arc::new(FakeCatalog { entries: vec![entry("x", None)], ..Default::default() });
        let seo = SeoPageUseCase::new(catalog.clone(), "https://r.example".to_string());

        seo.execute(&SeoRoute::Package { owner: org("acme"), repository: "lib".into(), format: RepositoryFormat::Docker, name: "x".into() }).await.unwrap();

        assert_eq!(catalog.looked_up.lock().unwrap().clone(), vec![(org("acme"), "lib".to_string(), RepositoryFormat::Docker, "x".to_string())]);
    }

    #[tokio::test]
    async fn names_that_could_not_exist_get_the_generic_head_without_any_lookup() {
        let catalog = Arc::new(FakeCatalog { entries: vec![entry("x", None)], owner: Some(summary("alice", 1, 1)), ..Default::default() });
        let seo = SeoPageUseCase::new(catalog.clone(), "https://r.example".to_string());
        let package = |format, name: String| SeoRoute::Package { owner: personal("alice"), repository: "lib".into(), format, name };

        for route in [
            package(RepositoryFormat::Npm, "a".repeat(215)),
            package(RepositoryFormat::Docker, "a".repeat(129)),
            package(RepositoryFormat::Npm, "x\u{0}".into()),
            package(RepositoryFormat::Docker, "line\nbreak".into()),
            package(RepositoryFormat::Npm, String::new()),
            SeoRoute::Package { owner: personal("alice"), repository: "l\u{0}b".into(), format: RepositoryFormat::Npm, name: "x".into() },
            SeoRoute::Package { owner: personal("alice"), repository: "r".repeat(65), format: RepositoryFormat::Npm, name: "x".into() },
            SeoRoute::Repository { owner: personal("alice"), repository: "l\u{0}b".into() },
            SeoRoute::Owner(personal("al\u{0}ce")),
            SeoRoute::Owner(personal(&"a".repeat(129))),
        ] {
            assert_eq!(seo.execute(&route).await.unwrap(), PageMeta::generic(), "{route:?}");
        }
        assert!(catalog.looked_up.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_longest_names_that_can_exist_are_still_looked_up() {
        let catalog = Arc::new(FakeCatalog { entries: vec![entry(&"a".repeat(214), None)], ..Default::default() });
        let seo = SeoPageUseCase::new(catalog.clone(), "https://r.example".to_string());

        let meta = seo.execute(&SeoRoute::Package { owner: personal("alice"), repository: "lib".into(), format: RepositoryFormat::Npm, name: "a".repeat(214) }).await.unwrap();

        assert!(meta.indexable);
    }

    #[test]
    fn long_descriptions_are_cut_and_whitespace_is_flattened() {
        let long = "word ".repeat(80);

        let cut = truncate(&long);

        assert!(cut.chars().count() <= 160 && cut.ends_with('…'), "{cut}");
        assert_eq!(truncate("a\n  b \t c"), "a b c");
    }
}
