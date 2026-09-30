use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use artiferris_domain::organization::{OrganizationRepositoryPort, OrganizationSlug};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, RepositoryFormat};
use artiferris_domain::permission::PermissionQueryPort;
use artiferris_domain::public_catalog::{
    CatalogEntryCount, CatalogFormatSpec, CatalogPage, CatalogQuery, CatalogScope, CatalogSort, CatalogSuggestion, OwnerKind, OwnerRef, OwnerSummary, PublicCatalogPort, SuggestQuery, CATALOG_FORMATS,
};
use artiferris_domain::user::Username;
use uuid::Uuid;

use crate::error::ApplicationError;
use crate::use_cases::list_readable_repositories::ReadableRepositoriesCaller;

pub const MAX_QUERY_LENGTH: usize = 100;
pub const DEFAULT_PER_PAGE: u32 = 20;
pub const MAX_PER_PAGE: u32 = 50;
pub const MAX_PAGE: u32 = 100;
pub const MIN_SUGGEST_LENGTH: usize = 2;
pub const DEFAULT_SUGGESTIONS: u32 = 8;
pub const MAX_SUGGESTIONS: u32 = 20;

/// Raw, unvalidated input as it arrives from the HTTP layer.
#[derive(Debug, Default, Clone)]
pub struct RawCatalogSearch {
    pub text: Option<String>,
    pub format: Option<String>,
    /// `personal:<username>` or `organization:<slug>`.
    pub owner: Option<String>,
    pub sort: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSearchResult {
    pub page: CatalogPage,
    pub current_page: u32,
    pub per_page: u32,
}

pub struct SearchPublicCatalogUseCase {
    catalog: Arc<dyn PublicCatalogPort>,
}

impl SearchPublicCatalogUseCase {
    pub fn new(catalog: Arc<dyn PublicCatalogPort>) -> Self {
        Self { catalog }
    }

    pub async fn execute(&self, raw: RawCatalogSearch) -> Result<CatalogSearchResult, ApplicationError> {
        let query = validate(raw)?;
        let page = self.catalog.search(&query).await?;
        Ok(CatalogSearchResult { page, current_page: query.page, per_page: query.per_page })
    }
}

/// NUL in particular is refused by Postgres, which would turn a bad URL into a 500.
pub(crate) fn has_control_character(text: &str) -> bool {
    text.chars().any(char::is_control)
}

fn validate(raw: RawCatalogSearch) -> Result<CatalogQuery, ApplicationError> {
    let text = raw.text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if text.as_ref().is_some_and(|t| t.chars().count() > MAX_QUERY_LENGTH) {
        return Err(ApplicationError::InvalidCatalogQuery(format!("search text is limited to {MAX_QUERY_LENGTH} characters")));
    }
    if text.as_deref().is_some_and(has_control_character) {
        return Err(ApplicationError::InvalidCatalogQuery("search text must not contain control characters".to_string()));
    }
    let format = parse_format_filter(raw.format.as_deref())?;
    let owner = parse_owner_filter(raw.owner.as_deref())?;
    let sort = match raw.sort.as_deref() {
        None | Some("") | Some("relevance") => CatalogSort::Relevance,
        Some("updated") => CatalogSort::Updated,
        Some("popular") => CatalogSort::Popular,
        Some(other) => return Err(ApplicationError::InvalidCatalogQuery(format!("unknown sort \"{other}\""))),
    };
    let page = raw.page.unwrap_or(1);
    if !(1..=MAX_PAGE).contains(&page) {
        return Err(ApplicationError::InvalidCatalogQuery(format!("page must be between 1 and {MAX_PAGE}")));
    }
    let per_page = raw.per_page.unwrap_or(DEFAULT_PER_PAGE);
    if !(1..=MAX_PER_PAGE).contains(&per_page) {
        return Err(ApplicationError::InvalidCatalogQuery(format!("per_page must be between 1 and {MAX_PER_PAGE}")));
    }
    // Relevance without any text has nothing to rank by.
    let sort = if text.is_none() && sort == CatalogSort::Relevance { CatalogSort::Updated } else { sort };
    Ok(CatalogQuery { scope: CatalogScope::PublicOnly, text, format, owner, sort, page, per_page })
}

fn parse_format_filter(raw: Option<&str>) -> Result<Option<RepositoryFormat>, ApplicationError> {
    match raw {
        None | Some("") => Ok(None),
        Some(name) => CATALOG_FORMATS
            .iter()
            .find(|spec| format_key(spec.format) == name)
            .map(|spec| Some(spec.format))
            .ok_or_else(|| ApplicationError::InvalidCatalogQuery(format!("unknown format \"{name}\""))),
    }
}

fn parse_owner_filter(raw: Option<&str>) -> Result<Option<OwnerRef>, ApplicationError> {
    let invalid = || ApplicationError::InvalidCatalogQuery("owner must look like personal:<username> or organization:<slug>".to_string());
    match raw {
        None | Some("") => Ok(None),
        Some(value) => {
            let (kind, slug) = value.split_once(':').ok_or_else(invalid)?;
            owner_ref(kind, slug).map(Some).ok_or_else(invalid)
        }
    }
}

/// `None` for an unknown kind or a slug that could never name an owner.
fn owner_ref(kind: &str, slug: &str) -> Option<OwnerRef> {
    match kind {
        "personal" => Username::parse(slug).ok().map(|username| OwnerRef { kind: OwnerKind::Personal, slug: username.as_str().to_string() }),
        "organization" => OrganizationSlug::parse(slug).ok().map(|slug| OwnerRef { kind: OwnerKind::Organization, slug: slug.as_str().to_string() }),
        _ => None,
    }
}

pub fn format_key(format: RepositoryFormat) -> &'static str {
    match format {
        RepositoryFormat::Npm => "npm",
        RepositoryFormat::Docker => "docker",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogSummary {
    pub spec: &'static CatalogFormatSpec,
    pub entry_count: i64,
}

pub struct ListCatalogsUseCase {
    catalog: Arc<dyn PublicCatalogPort>,
}

impl ListCatalogsUseCase {
    pub fn new(catalog: Arc<dyn PublicCatalogPort>) -> Self {
        Self { catalog }
    }

    /// One summary per registered catalog, registry order, an empty catalog counting zero.
    pub async fn execute(&self) -> Result<Vec<CatalogSummary>, ApplicationError> {
        let counts: Vec<CatalogEntryCount> = self.catalog.entry_counts().await?;
        Ok(CATALOG_FORMATS
            .iter()
            .map(|spec| CatalogSummary { spec, entry_count: counts.iter().find(|c| c.format == spec.format).map_or(0, |c| c.entry_count) })
            .collect())
    }
}

/// How long a caller's readable repositories are remembered. A grant or revocation shows up in search after at most this long.
const SCOPE_CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_CACHED_SCOPES: usize = 2000;

/// Whose scope this is: an organization admin's set differs from a member's of the same organization.
type ScopeKey = (Uuid, Uuid, bool);

/// Search for a signed-in caller: the public catalog plus everything they can read, proxies' cached packages included.
pub struct SearchReadableCatalogUseCase {
    catalog: Arc<dyn PublicCatalogPort>,
    permissions: Arc<dyn PermissionQueryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    organizations: Arc<dyn OrganizationRepositoryPort>,
    scopes: Mutex<HashMap<ScopeKey, (Instant, Arc<Vec<Uuid>>)>>,
    scope_ttl: Duration,
}

impl SearchReadableCatalogUseCase {
    pub fn new(catalog: Arc<dyn PublicCatalogPort>, permissions: Arc<dyn PermissionQueryPort>, repositories: Arc<dyn PackageRepositoryQueryPort>, organizations: Arc<dyn OrganizationRepositoryPort>) -> Self {
        Self::with_scope_ttl(catalog, permissions, repositories, organizations, SCOPE_CACHE_TTL)
    }

    fn with_scope_ttl(
        catalog: Arc<dyn PublicCatalogPort>,
        permissions: Arc<dyn PermissionQueryPort>,
        repositories: Arc<dyn PackageRepositoryQueryPort>,
        organizations: Arc<dyn OrganizationRepositoryPort>,
        scope_ttl: Duration,
    ) -> Self {
        Self { catalog, permissions, repositories, organizations, scopes: Mutex::new(HashMap::new()), scope_ttl }
    }

    pub async fn execute(&self, caller: &ReadableRepositoriesCaller, raw: RawCatalogSearch) -> Result<CatalogSearchResult, ApplicationError> {
        let mut query = validate(raw)?;
        query.scope = self.scope(caller).await?;
        let page = self.catalog.search(&query).await?;
        Ok(CatalogSearchResult { page, current_page: query.page, per_page: query.per_page })
    }

    async fn scope(&self, caller: &ReadableRepositoriesCaller) -> Result<CatalogScope, ApplicationError> {
        if caller.is_super_admin {
            return Ok(CatalogScope::AllRepositories);
        }
        let key = (caller.user_id, caller.organization_id, caller.is_organization_admin);
        if let Some((cached_at, ids)) = self.scopes.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
            if cached_at.elapsed() < self.scope_ttl {
                return Ok(CatalogScope::Repositories(ids.as_ref().clone()));
            }
        }
        let ids = Arc::new(self.readable_ids(caller).await?);
        let mut scopes = self.scopes.lock().unwrap_or_else(|p| p.into_inner());
        if scopes.len() >= MAX_CACHED_SCOPES && !scopes.contains_key(&key) {
            scopes.retain(|_, (cached_at, _)| cached_at.elapsed() < self.scope_ttl);
            if scopes.len() >= MAX_CACHED_SCOPES {
                scopes.clear();
            }
        }
        scopes.insert(key, (Instant::now(), ids.clone()));
        Ok(CatalogScope::Repositories(ids.as_ref().clone()))
    }

    async fn readable_ids(&self, caller: &ReadableRepositoriesCaller) -> Result<Vec<Uuid>, ApplicationError> {
        // An organization admin reads their whole organization ...
        let mut ids: HashSet<Uuid> = HashSet::new();
        if caller.is_organization_admin {
            ids.extend(self.repositories.list_by_organization(caller.organization_id).await?.into_iter().map(|r| r.id));
        }
        // ... anyone else what they were granted, but only where a grant actually opens a door: a repository of their own
        // organization, or a personal project (the one place a grant crosses organizations).
        let mut personal: HashMap<Uuid, bool> = HashMap::new();
        for (repository_id, _) in self.permissions.list_for_user(caller.user_id).await? {
            if ids.contains(&repository_id) {
                continue;
            }
            let Some(repository) = self.repositories.find_by_id(repository_id).await? else { continue };
            let reachable = repository.organization_id == caller.organization_id
                || match personal.get(&repository.organization_id) {
                    Some(is_personal) => *is_personal,
                    None => {
                        let is_personal = self.organizations.find_by_id(repository.organization_id).await?.is_some_and(|owner| owner.is_personal);
                        personal.insert(repository.organization_id, is_personal);
                        is_personal
                    }
                };
            if reachable {
                ids.insert(repository_id);
            }
        }
        let mut ids: Vec<Uuid> = ids.into_iter().collect();
        ids.sort_unstable();
        Ok(ids)
    }
}

/// Raw, unvalidated input of a suggestion request; `format` and `owner` follow the search syntax.
#[derive(Debug, Default, Clone)]
pub struct RawSuggestion {
    pub text: String,
    pub format: Option<String>,
    pub owner: Option<String>,
    /// Defaults to `DEFAULT_SUGGESTIONS`, capped at `MAX_SUGGESTIONS`.
    pub limit: Option<u32>,
}

pub struct SuggestPublicCatalogUseCase {
    catalog: Arc<dyn PublicCatalogPort>,
}

impl SuggestPublicCatalogUseCase {
    pub fn new(catalog: Arc<dyn PublicCatalogPort>) -> Self {
        Self { catalog }
    }

    pub async fn execute(&self, raw: RawSuggestion) -> Result<Vec<CatalogSuggestion>, ApplicationError> {
        let text = raw.text.trim();
        let length = text.chars().count();
        if !(MIN_SUGGEST_LENGTH..=MAX_QUERY_LENGTH).contains(&length) {
            return Err(ApplicationError::InvalidCatalogQuery(format!("suggestions need between {MIN_SUGGEST_LENGTH} and {MAX_QUERY_LENGTH} characters")));
        }
        if has_control_character(text) {
            return Err(ApplicationError::InvalidCatalogQuery("suggestions must not contain control characters".to_string()));
        }
        let format = parse_format_filter(raw.format.as_deref())?;
        let owner = parse_owner_filter(raw.owner.as_deref())?;
        let limit = match raw.limit {
            None => DEFAULT_SUGGESTIONS,
            Some(0) => return Err(ApplicationError::InvalidCatalogQuery("limit must be at least 1".to_string())),
            Some(limit) => limit.min(MAX_SUGGESTIONS),
        };
        Ok(self.catalog.suggest(&SuggestQuery { text: text.to_string(), limit, format, owner }).await?)
    }
}

pub struct GetOwnerSummaryUseCase {
    catalog: Arc<dyn PublicCatalogPort>,
}

impl GetOwnerSummaryUseCase {
    pub fn new(catalog: Arc<dyn PublicCatalogPort>) -> Self {
        Self { catalog }
    }

    /// `None` for an owner with no public repository, an unknown one, and a kind or slug that could not name one:
    /// all deliberately the same answer, so the page can't confirm that an account exists.
    pub async fn execute(&self, kind: &str, slug: &str) -> Result<Option<OwnerSummary>, ApplicationError> {
        let Some(owner) = owner_ref(kind, slug) else { return Ok(None) };
        Ok(self.catalog.owner_summary(&owner).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use artiferris_domain::error::DomainError;

    use super::*;

    struct RecordingCatalog {
        seen: Mutex<Vec<CatalogQuery>>,
        counts: Vec<CatalogEntryCount>,
        owners_asked: Mutex<Vec<OwnerRef>>,
        summary: Option<OwnerSummary>,
        suggested: Mutex<Vec<SuggestQuery>>,
    }

    impl RecordingCatalog {
        fn new() -> Arc<Self> {
            Arc::new(Self { seen: Mutex::new(vec![]), counts: vec![], owners_asked: Mutex::new(vec![]), summary: None, suggested: Mutex::new(vec![]) })
        }
    }

    #[async_trait]
    impl PublicCatalogPort for RecordingCatalog {
        async fn search(&self, query: &CatalogQuery) -> Result<CatalogPage, DomainError> {
            self.seen.lock().unwrap().push(query.clone());
            Ok(CatalogPage { items: vec![], total: 0 })
        }
        async fn entry_counts(&self) -> Result<Vec<CatalogEntryCount>, DomainError> {
            Ok(self.counts.clone())
        }
        async fn suggest(&self, query: &SuggestQuery) -> Result<Vec<CatalogSuggestion>, DomainError> {
            self.suggested.lock().unwrap().push(query.clone());
            Ok(vec![])
        }
        async fn repository(&self, _owner: &OwnerRef, _repository: &str) -> Result<Option<artiferris_domain::public_catalog::CatalogRepository>, DomainError> {
            Ok(None)
        }
        async fn find_entry(&self, _owner: &OwnerRef, _repository: &str, _format: RepositoryFormat, _name: &str) -> Result<Option<artiferris_domain::public_catalog::CatalogEntry>, DomainError> {
            Ok(None)
        }
        async fn sitemap_entries(&self, _limit: usize) -> Result<Vec<artiferris_domain::public_catalog::SitemapEntry>, DomainError> {
            Ok(vec![])
        }
        async fn owner_summary(&self, owner: &OwnerRef) -> Result<Option<OwnerSummary>, DomainError> {
            self.owners_asked.lock().unwrap().push(owner.clone());
            Ok(self.summary.clone())
        }
    }

    async fn run(raw: RawCatalogSearch) -> Result<CatalogQuery, ApplicationError> {
        let catalog = RecordingCatalog::new();
        SearchPublicCatalogUseCase::new(catalog.clone()).execute(raw).await?;
        let seen = catalog.seen.lock().unwrap().clone();
        Ok(seen[0].clone())
    }

    #[tokio::test]
    async fn defaults_to_the_first_page_of_twenty_most_recent_entries() {
        let query = run(RawCatalogSearch::default()).await.unwrap();
        assert_eq!(query, CatalogQuery { scope: CatalogScope::PublicOnly, text: None, format: None, owner: None, sort: CatalogSort::Updated, page: 1, per_page: 20 });
    }

    #[tokio::test]
    async fn text_switches_the_default_sort_to_relevance() {
        let query = run(RawCatalogSearch { text: Some("  left-pad ".into()), ..Default::default() }).await.unwrap();
        assert_eq!(query.text.as_deref(), Some("left-pad"));
        assert_eq!(query.sort, CatalogSort::Relevance);
    }

    #[tokio::test]
    async fn blank_text_counts_as_no_text() {
        let query = run(RawCatalogSearch { text: Some("   ".into()), sort: Some("relevance".into()), ..Default::default() }).await.unwrap();
        assert_eq!(query.text, None);
        assert_eq!(query.sort, CatalogSort::Updated);
    }

    #[tokio::test]
    async fn text_over_the_limit_is_rejected_without_calling_the_port() {
        let catalog = RecordingCatalog::new();
        let err = SearchPublicCatalogUseCase::new(catalog.clone())
            .execute(RawCatalogSearch { text: Some("a".repeat(101)), ..Default::default() })
            .await
            .unwrap_err();
        assert!(matches!(err, ApplicationError::InvalidCatalogQuery(_)));
        assert!(catalog.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn control_characters_in_the_text_are_rejected_without_calling_the_port() {
        let catalog = RecordingCatalog::new();
        let use_case = SearchPublicCatalogUseCase::new(catalog.clone());

        for text in ["a\u{0}b", "\u{0}", "line\nbreak", "bell\u{7}"] {
            let err = use_case.execute(RawCatalogSearch { text: Some(text.into()), ..Default::default() }).await.unwrap_err();
            assert!(matches!(err, ApplicationError::InvalidCatalogQuery(_)), "{text:?}");
        }
        assert!(catalog.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_limit_counts_characters_not_bytes() {
        assert!(run(RawCatalogSearch { text: Some("é".repeat(100)), ..Default::default() }).await.is_ok());
    }

    #[tokio::test]
    async fn parses_the_format_and_rejects_unknown_ones() {
        let query = run(RawCatalogSearch { format: Some("docker".into()), ..Default::default() }).await.unwrap();
        assert_eq!(query.format, Some(RepositoryFormat::Docker));
        assert!(matches!(run(RawCatalogSearch { format: Some("helm".into()), ..Default::default() }).await, Err(ApplicationError::InvalidCatalogQuery(_))));
    }

    #[tokio::test]
    async fn rejects_an_unknown_sort() {
        assert!(matches!(run(RawCatalogSearch { sort: Some("trending".into()), ..Default::default() }).await, Err(ApplicationError::InvalidCatalogQuery(_))));
    }

    #[tokio::test]
    async fn popular_is_a_valid_sort_with_or_without_text() {
        for text in [None, Some("pad".to_string())] {
            let query = run(RawCatalogSearch { text, sort: Some("popular".into()), ..Default::default() }).await.unwrap();
            assert_eq!(query.sort, CatalogSort::Popular);
        }
    }

    #[tokio::test]
    async fn pagination_is_bounded() {
        for (page, per_page) in [(Some(0), None), (Some(101), None), (None, Some(0)), (None, Some(51))] {
            let result = run(RawCatalogSearch { page, per_page, ..Default::default() }).await;
            assert!(matches!(result, Err(ApplicationError::InvalidCatalogQuery(_))), "page {page:?} per_page {per_page:?} must be rejected");
        }
        let query = run(RawCatalogSearch { page: Some(100), per_page: Some(50), ..Default::default() }).await.unwrap();
        assert_eq!((query.page, query.per_page), (100, 50));
    }

    #[tokio::test]
    async fn listing_catalogs_covers_every_registered_format_even_when_empty() {
        let catalog = Arc::new(RecordingCatalog { seen: Mutex::new(vec![]), counts: vec![CatalogEntryCount { format: RepositoryFormat::Npm, entry_count: 3 }], owners_asked: Mutex::new(vec![]), summary: None, suggested: Mutex::new(vec![]) });
        let summaries = ListCatalogsUseCase::new(catalog).execute().await.unwrap();

        assert_eq!(summaries.len(), CATALOG_FORMATS.len());
        assert_eq!((summaries[0].spec.catalog_name, summaries[0].entry_count), ("artiferris-npm", 3));
        assert_eq!((summaries[1].spec.catalog_name, summaries[1].entry_count), ("artiferris-docker", 0));
    }

    #[tokio::test]
    async fn parses_a_personal_or_organization_owner_filter() {
        let personal = run(RawCatalogSearch { owner: Some("personal:Alice".into()), ..Default::default() }).await.unwrap();
        assert_eq!(personal.owner, Some(OwnerRef { kind: OwnerKind::Personal, slug: "alice".to_string() }));
        let organization = run(RawCatalogSearch { owner: Some("organization:acme".into()), ..Default::default() }).await.unwrap();
        assert_eq!(organization.owner, Some(OwnerRef { kind: OwnerKind::Organization, slug: "acme".to_string() }));
        assert_eq!(run(RawCatalogSearch { owner: Some("".into()), ..Default::default() }).await.unwrap().owner, None);
    }

    #[tokio::test]
    async fn rejects_a_malformed_owner_filter() {
        for owner in ["alice", "user:alice", "personal:", "personal:!bad name!", "organization:x", ":acme"] {
            let result = run(RawCatalogSearch { owner: Some(owner.into()), ..Default::default() }).await;
            assert!(matches!(result, Err(ApplicationError::InvalidCatalogQuery(_))), "{owner} must be rejected");
        }
    }

    #[tokio::test]
    async fn an_owner_summary_is_passed_through_for_a_valid_owner() {
        let summary = OwnerSummary { kind: OwnerKind::Personal, slug: "alice".into(), display_name: "alice".into(), repository_count: 1, package_count: 2, image_count: 3, indexing_blocked: false };
        let catalog = Arc::new(RecordingCatalog { seen: Mutex::new(vec![]), counts: vec![], owners_asked: Mutex::new(vec![]), summary: Some(summary.clone()), suggested: Mutex::new(vec![]) });

        let found = GetOwnerSummaryUseCase::new(catalog.clone()).execute("personal", "Alice").await.unwrap();

        assert_eq!(found, Some(summary));
        assert_eq!(catalog.owners_asked.lock().unwrap().clone(), vec![OwnerRef { kind: OwnerKind::Personal, slug: "alice".to_string() }]);
    }

    #[tokio::test]
    async fn an_owner_that_could_not_exist_is_none_without_asking_the_port() {
        let catalog = RecordingCatalog::new();
        let use_case = GetOwnerSummaryUseCase::new(catalog.clone());

        assert_eq!(use_case.execute("team", "acme").await.unwrap(), None);
        assert_eq!(use_case.execute("personal", "!bad name!").await.unwrap(), None);
        assert!(catalog.owners_asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn suggestions_are_asked_for_the_trimmed_text_with_the_default_limit_and_no_filter() {
        let catalog = RecordingCatalog::new();

        SuggestPublicCatalogUseCase::new(catalog.clone()).execute(RawSuggestion { text: "  lef ".to_string(), ..Default::default() }).await.unwrap();

        assert_eq!(catalog.suggested.lock().unwrap().clone(), vec![SuggestQuery { text: "lef".to_string(), limit: DEFAULT_SUGGESTIONS, format: None, owner: None }]);
    }

    #[tokio::test]
    async fn suggestions_pass_the_format_and_owner_filters_and_clamp_the_limit() {
        let catalog = RecordingCatalog::new();
        let use_case = SuggestPublicCatalogUseCase::new(catalog.clone());

        use_case.execute(RawSuggestion { text: "lef".to_string(), format: Some("docker".to_string()), owner: Some("personal:Alice".to_string()), limit: Some(500) }).await.unwrap();
        use_case.execute(RawSuggestion { text: "lef".to_string(), format: Some("".to_string()), owner: Some("organization:acme".to_string()), limit: Some(3) }).await.unwrap();

        let asked = catalog.suggested.lock().unwrap().clone();
        assert_eq!(asked[0], SuggestQuery { text: "lef".to_string(), limit: MAX_SUGGESTIONS, format: Some(RepositoryFormat::Docker), owner: Some(OwnerRef { kind: OwnerKind::Personal, slug: "alice".to_string() }) });
        assert_eq!(asked[1], SuggestQuery { text: "lef".to_string(), limit: 3, format: None, owner: Some(OwnerRef { kind: OwnerKind::Organization, slug: "acme".to_string() }) });
    }

    #[tokio::test]
    async fn suggestions_reject_a_bad_filter_or_limit_without_calling_the_port() {
        let catalog = RecordingCatalog::new();
        let use_case = SuggestPublicCatalogUseCase::new(catalog.clone());
        let base = RawSuggestion { text: "lef".to_string(), ..Default::default() };

        for raw in [
            RawSuggestion { format: Some("pypi".to_string()), ..base.clone() },
            RawSuggestion { owner: Some("acme".to_string()), ..base.clone() },
            RawSuggestion { owner: Some("team:acme".to_string()), ..base.clone() },
            RawSuggestion { owner: Some("personal:!!".to_string()), ..base.clone() },
            RawSuggestion { limit: Some(0), ..base.clone() },
        ] {
            assert!(matches!(use_case.execute(raw.clone()).await, Err(ApplicationError::InvalidCatalogQuery(_))), "{raw:?}");
        }
        assert!(catalog.suggested.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn suggestions_reject_text_that_is_too_short_or_too_long_without_calling_the_port() {
        let catalog = RecordingCatalog::new();
        let use_case = SuggestPublicCatalogUseCase::new(catalog.clone());

        for text in ["", " a ", &"a".repeat(101), "a\u{0}b"] {
            assert!(matches!(use_case.execute(RawSuggestion { text: text.to_string(), ..Default::default() }).await, Err(ApplicationError::InvalidCatalogQuery(_))), "{text:?}");
        }
        assert!(use_case.execute(RawSuggestion { text: "ab".to_string(), ..Default::default() }).await.is_ok());
        assert_eq!(catalog.suggested.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod scope_tests {
    use artiferris_domain::error::DomainError;
    use artiferris_domain::public_catalog::{CatalogEntryCount, CatalogRepository, SitemapEntry};
    use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
    use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
    use artiferris_infrastructure::postgres::permission_store::PostgresPermissionStore;
    use artiferris_domain::error::EventStoreError;
    use artiferris_domain::permission::Role;
    use async_trait::async_trait;
    use sqlx::PgPool;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;

    const PUBLIC_ORG: &str = "00000000-0000-0000-0000-000000000001";

    #[derive(Default)]
    struct ScopeRecorder {
        seen: Mutex<Vec<CatalogQuery>>,
    }

    #[async_trait]
    impl PublicCatalogPort for ScopeRecorder {
        async fn search(&self, query: &CatalogQuery) -> Result<CatalogPage, DomainError> {
            self.seen.lock().unwrap().push(query.clone());
            Ok(CatalogPage { items: vec![], total: 0 })
        }
        async fn entry_counts(&self) -> Result<Vec<CatalogEntryCount>, DomainError> {
            Ok(vec![])
        }
        async fn suggest(&self, _query: &SuggestQuery) -> Result<Vec<CatalogSuggestion>, DomainError> {
            Ok(vec![])
        }
        async fn repository(&self, _owner: &OwnerRef, _repository: &str) -> Result<Option<CatalogRepository>, DomainError> {
            Ok(None)
        }
        async fn find_entry(&self, _owner: &OwnerRef, _repository: &str, _format: RepositoryFormat, _name: &str) -> Result<Option<artiferris_domain::public_catalog::CatalogEntry>, DomainError> {
            Ok(None)
        }
        async fn sitemap_entries(&self, _limit: usize) -> Result<Vec<SitemapEntry>, DomainError> {
            Ok(vec![])
        }
        async fn owner_summary(&self, _owner: &OwnerRef) -> Result<Option<OwnerSummary>, DomainError> {
            Ok(None)
        }
    }

    async fn organization(pool: &PgPool, slug: &str, is_personal: bool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations (id, slug, display_name, is_personal) VALUES ($1, $2, $2, $3)").bind(id).bind(slug).bind(is_personal).execute(pool).await.unwrap();
        id
    }

    async fn repository(pool: &PgPool, organization: Uuid, name: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, version) VALUES ($1, $2, $3, 'npm', 'hosted', 1)").bind(id).bind(organization).bind(name).execute(pool).await.unwrap();
        id
    }

    async fn grant(pool: &PgPool, user: Uuid, repository: Uuid) {
        sqlx::query("INSERT INTO permission_projections (user_id, repository_id, role, version) VALUES ($1, $2, 'read', 1)").bind(user).bind(repository).execute(pool).await.unwrap();
    }

    /// Counts the grant lookups, the expensive part of building a scope.
    struct CountingPermissions {
        inner: PostgresPermissionStore,
        lookups: AtomicUsize,
    }

    #[async_trait]
    impl PermissionQueryPort for CountingPermissions {
        async fn find_role(&self, user_id: Uuid, repository_id: Uuid) -> Result<Option<Role>, EventStoreError> {
            self.inner.find_role(user_id, repository_id).await
        }
        async fn list_for_repository(&self, repository_id: Uuid) -> Result<Vec<(Uuid, Role)>, EventStoreError> {
            self.inner.list_for_repository(repository_id).await
        }
        async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<(Uuid, Role)>, EventStoreError> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            self.inner.list_for_user(user_id).await
        }
        async fn list_all(&self) -> Result<Vec<(Uuid, Uuid, Role)>, EventStoreError> {
            self.inner.list_all().await
        }
        async fn count_all(&self) -> Result<usize, EventStoreError> {
            self.inner.count_all().await
        }
        async fn count_for_repositories(&self, repository_ids: &[Uuid]) -> Result<usize, EventStoreError> {
            self.inner.count_for_repositories(repository_ids).await
        }
    }

    fn use_case(pool: &PgPool, ttl: Duration) -> (SearchReadableCatalogUseCase, Arc<ScopeRecorder>, Arc<CountingPermissions>) {
        let organizations = Arc::new(PostgresOrganizationRepository::new(pool.clone()));
        let repositories = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        let permissions = Arc::new(CountingPermissions { inner: PostgresPermissionStore::new(pool.clone()), lookups: AtomicUsize::new(0) });
        let catalog = Arc::new(ScopeRecorder::default());
        (SearchReadableCatalogUseCase::with_scope_ttl(catalog.clone(), permissions.clone(), repositories, organizations, ttl), catalog, permissions)
    }

    fn last_scope(catalog: &ScopeRecorder) -> CatalogScope {
        catalog.seen.lock().unwrap().last().unwrap().scope.clone()
    }

    async fn scope_for(pool: &PgPool, caller: ReadableRepositoriesCaller) -> CatalogScope {
        let (use_case, catalog, _) = use_case(pool, SCOPE_CACHE_TTL);
        use_case.execute(&caller, RawCatalogSearch::default()).await.unwrap();
        let scope = last_scope(&catalog);
        match scope {
            CatalogScope::Repositories(mut ids) => {
                ids.sort_unstable();
                CatalogScope::Repositories(ids)
            }
            other => other,
        }
    }

    fn caller(organization_id: Uuid) -> ReadableRepositoriesCaller {
        ReadableRepositoriesCaller { user_id: Uuid::new_v4(), organization_id, is_super_admin: false, is_organization_admin: false }
    }

    fn sorted(mut ids: Vec<Uuid>) -> CatalogScope {
        ids.sort_unstable();
        CatalogScope::Repositories(ids)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_super_admin_searches_everything(pool: PgPool) {
        let caller = ReadableRepositoriesCaller { is_super_admin: true, ..caller(Uuid::new_v4()) };

        assert_eq!(scope_for(&pool, caller).await, CatalogScope::AllRepositories);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_searches_their_whole_organization_and_nobody_elses(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let globex = organization(&pool, "globex", false).await;
        let one = repository(&pool, acme, "one").await;
        let two = repository(&pool, acme, "two").await;
        repository(&pool, globex, "theirs").await;
        let caller = ReadableRepositoriesCaller { is_organization_admin: true, ..caller(acme) };

        assert_eq!(scope_for(&pool, caller).await, sorted(vec![one, two]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_member_searches_only_what_they_were_granted(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let granted = repository(&pool, acme, "granted").await;
        repository(&pool, acme, "not-granted").await;
        let me = caller(acme);
        grant(&pool, me.user_id, granted).await;

        assert_eq!(scope_for(&pool, me).await, sorted(vec![granted]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_grant_on_someone_elses_personal_project_opens_it_but_a_stray_grant_in_another_organization_does_not(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let globex = organization(&pool, "globex", false).await;
        let alice_space = organization(&pool, "u-alice-space", true).await;
        let shared = repository(&pool, alice_space, "shared-project").await;
        let stray = repository(&pool, globex, "stray").await;
        let me = caller(acme);
        grant(&pool, me.user_id, shared).await;
        grant(&pool, me.user_id, stray).await;

        assert_eq!(scope_for(&pool, me).await, sorted(vec![shared]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_member_with_no_grant_searches_public_content_only(pool: PgPool) {
        let public: Uuid = PUBLIC_ORG.parse().unwrap();
        repository(&pool, public, "ungranted").await;

        assert_eq!(scope_for(&pool, caller(public)).await, CatalogScope::Repositories(vec![]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_query_is_validated_like_the_public_search_before_any_scope_is_computed(pool: PgPool) {
        let (use_case, catalog, permissions) = use_case(&pool, SCOPE_CACHE_TTL);

        let error = use_case.execute(&caller(Uuid::new_v4()), RawCatalogSearch { per_page: Some(500), ..Default::default() }).await.unwrap_err();

        assert!(matches!(error, ApplicationError::InvalidCatalogQuery(_)));
        assert!(catalog.seen.lock().unwrap().is_empty());
        assert_eq!(permissions.lookups.load(Ordering::SeqCst), 0);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_callers_scope_is_built_once_and_reused_for_the_next_searches(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let granted = repository(&pool, acme, "granted").await;
        let me = caller(acme);
        grant(&pool, me.user_id, granted).await;
        let (use_case, catalog, permissions) = use_case(&pool, SCOPE_CACHE_TTL);

        for _ in 0..5 {
            use_case.execute(&me, RawCatalogSearch::default()).await.unwrap();
        }

        assert_eq!(permissions.lookups.load(Ordering::SeqCst), 1, "one build for five searches");
        assert_eq!(catalog.seen.lock().unwrap().len(), 5);
        assert_eq!(last_scope(&catalog), sorted(vec![granted]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_new_grant_shows_up_once_the_cached_scope_has_expired(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let first = repository(&pool, acme, "first").await;
        let second = repository(&pool, acme, "second").await;
        let me = caller(acme);
        grant(&pool, me.user_id, first).await;
        let (use_case, catalog, _) = use_case(&pool, Duration::from_millis(200));
        use_case.execute(&me, RawCatalogSearch::default()).await.unwrap();

        grant(&pool, me.user_id, second).await;
        use_case.execute(&me, RawCatalogSearch::default()).await.unwrap();
        assert_eq!(last_scope(&catalog), sorted(vec![first]), "still the remembered scope inside the window");

        tokio::time::sleep(Duration::from_millis(250)).await;
        use_case.execute(&me, RawCatalogSearch::default()).await.unwrap();
        assert_eq!(last_scope(&catalog), sorted(vec![first, second]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn one_callers_scope_is_never_served_to_another(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let mine = repository(&pool, acme, "mine").await;
        let hers = repository(&pool, acme, "hers").await;
        let (me, her) = (caller(acme), caller(acme));
        grant(&pool, me.user_id, mine).await;
        grant(&pool, her.user_id, hers).await;
        let admin = ReadableRepositoriesCaller { user_id: me.user_id, is_organization_admin: true, ..me };
        let (use_case, catalog, _) = use_case(&pool, SCOPE_CACHE_TTL);

        use_case.execute(&me, RawCatalogSearch::default()).await.unwrap();
        use_case.execute(&her, RawCatalogSearch::default()).await.unwrap();
        assert_eq!(last_scope(&catalog), sorted(vec![hers]));
        use_case.execute(&admin, RawCatalogSearch::default()).await.unwrap();
        assert_eq!(last_scope(&catalog), sorted(vec![mine, hers]), "the same user as an organization admin is another scope");
        use_case.execute(&me, RawCatalogSearch::default()).await.unwrap();
        assert_eq!(last_scope(&catalog), sorted(vec![mine]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_member_of_the_public_organization_is_scoped_without_listing_the_catalog(pool: PgPool) {
        let public: Uuid = PUBLIC_ORG.parse().unwrap();
        let alice_space = organization(&pool, "u-alice-space", true).await;
        let alice_public_project = repository(&pool, alice_space, "shared-project").await;
        sqlx::query("UPDATE package_repository_projections SET is_public = true WHERE id = $1").bind(alice_public_project).execute(&pool).await.unwrap();
        for i in 0..50 {
            repository(&pool, public, &format!("repo-{i}")).await;
        }

        assert_eq!(scope_for(&pool, caller(public)).await, CatalogScope::Repositories(vec![]), "public repositories reach the result through the query's own filter, not through this list");
    }
}
