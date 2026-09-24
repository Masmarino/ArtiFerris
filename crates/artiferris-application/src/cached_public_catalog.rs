use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use artiferris_domain::error::DomainError;
use artiferris_domain::package_repository::RepositoryFormat;
use artiferris_domain::public_catalog::{
    CatalogEntry, CatalogEntryCount, CatalogPage, CatalogQuery, CatalogRepository, CatalogScope, CatalogSuggestion, OwnerRef, OwnerSummary, PublicCatalogPort, SitemapEntry, SuggestQuery,
};

const TTL: Duration = Duration::from_secs(30);
const MAX_CACHED_PAGES: usize = 256;

/// The landing lists (a search without text) and the per-format counts are the same for everybody and the costliest
/// queries of the catalog, so they are kept for a few seconds. A repository made private can therefore still be listed
/// for up to that long; its content stays behind the access checks either way. Everything else goes straight through.
pub struct CachedPublicCatalog {
    inner: Arc<dyn PublicCatalogPort>,
    ttl: Duration,
    pages: Mutex<HashMap<String, (Instant, CatalogPage)>>,
    counts: Mutex<Option<(Instant, Vec<CatalogEntryCount>)>>,
}

impl CachedPublicCatalog {
    pub fn new(inner: Arc<dyn PublicCatalogPort>) -> Self {
        Self::with_ttl(inner, TTL)
    }

    fn with_ttl(inner: Arc<dyn PublicCatalogPort>, ttl: Duration) -> Self {
        Self { inner, ttl, pages: Mutex::new(HashMap::new()), counts: Mutex::new(None) }
    }

    fn cached_page(&self, key: &str) -> Option<CatalogPage> {
        let pages = self.pages.lock().unwrap_or_else(|p| p.into_inner());
        pages.get(key).filter(|(stored_at, _)| stored_at.elapsed() < self.ttl).map(|(_, page)| page.clone())
    }

    fn remember_page(&self, key: String, page: &CatalogPage) {
        let mut pages = self.pages.lock().unwrap_or_else(|p| p.into_inner());
        if pages.len() >= MAX_CACHED_PAGES {
            pages.retain(|_, (stored_at, _)| stored_at.elapsed() < self.ttl);
        }
        if pages.len() >= MAX_CACHED_PAGES {
            if let Some(oldest) = pages.iter().min_by_key(|(_, (stored_at, _))| *stored_at).map(|(key, _)| key.clone()) {
                pages.remove(&oldest);
            }
        }
        pages.insert(key, (Instant::now(), page.clone()));
    }
}

#[async_trait::async_trait]
impl PublicCatalogPort for CachedPublicCatalog {
    async fn search(&self, query: &CatalogQuery) -> Result<CatalogPage, DomainError> {
        if query.text.is_some() || query.scope != CatalogScope::PublicOnly {
            return self.inner.search(query).await;
        }
        let key = format!("{query:?}");
        if let Some(page) = self.cached_page(&key) {
            return Ok(page);
        }
        let page = self.inner.search(query).await?;
        self.remember_page(key, &page);
        Ok(page)
    }

    async fn entry_counts(&self) -> Result<Vec<CatalogEntryCount>, DomainError> {
        if let Some((stored_at, counts)) = self.counts.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            if stored_at.elapsed() < self.ttl {
                return Ok(counts.clone());
            }
        }
        let counts = self.inner.entry_counts().await?;
        *self.counts.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), counts.clone()));
        Ok(counts)
    }

    async fn suggest(&self, query: &SuggestQuery) -> Result<Vec<CatalogSuggestion>, DomainError> {
        self.inner.suggest(query).await
    }

    async fn repository(&self, owner: &OwnerRef, repository: &str) -> Result<Option<CatalogRepository>, DomainError> {
        self.inner.repository(owner, repository).await
    }

    async fn find_entry(&self, owner: &OwnerRef, repository: &str, format: RepositoryFormat, name: &str) -> Result<Option<CatalogEntry>, DomainError> {
        self.inner.find_entry(owner, repository, format, name).await
    }

    async fn sitemap_entries(&self, limit: usize) -> Result<Vec<SitemapEntry>, DomainError> {
        self.inner.sitemap_entries(limit).await
    }

    async fn owner_summary(&self, owner: &OwnerRef) -> Result<Option<OwnerSummary>, DomainError> {
        self.inner.owner_summary(owner).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use artiferris_domain::public_catalog::CatalogSort;

    use super::*;

    #[derive(Default)]
    struct CountingCatalog {
        searches: AtomicUsize,
        counts: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl PublicCatalogPort for CountingCatalog {
        async fn search(&self, query: &CatalogQuery) -> Result<CatalogPage, DomainError> {
            let n = self.searches.fetch_add(1, Ordering::SeqCst);
            Ok(CatalogPage { items: vec![], total: (n + 1) as i64 * 100 + i64::from(query.page) })
        }
        async fn entry_counts(&self) -> Result<Vec<CatalogEntryCount>, DomainError> {
            let n = self.counts.fetch_add(1, Ordering::SeqCst);
            Ok(vec![CatalogEntryCount { format: RepositoryFormat::Npm, entry_count: n as i64 + 1 }])
        }
        async fn suggest(&self, _query: &SuggestQuery) -> Result<Vec<CatalogSuggestion>, DomainError> {
            Ok(vec![])
        }
        async fn repository(&self, _owner: &OwnerRef, _repository: &str) -> Result<Option<CatalogRepository>, DomainError> {
            Ok(None)
        }
        async fn find_entry(&self, _owner: &OwnerRef, _repository: &str, _format: RepositoryFormat, _name: &str) -> Result<Option<CatalogEntry>, DomainError> {
            Ok(None)
        }
        async fn sitemap_entries(&self, _limit: usize) -> Result<Vec<SitemapEntry>, DomainError> {
            Ok(vec![])
        }
        async fn owner_summary(&self, _owner: &OwnerRef) -> Result<Option<OwnerSummary>, DomainError> {
            Ok(None)
        }
    }

    fn landing(page: u32) -> CatalogQuery {
        CatalogQuery { scope: CatalogScope::PublicOnly, text: None, format: None, owner: None, sort: CatalogSort::Updated, page, per_page: 20 }
    }

    fn cached(ttl: Duration) -> (CachedPublicCatalog, Arc<CountingCatalog>) {
        let inner = Arc::new(CountingCatalog::default());
        (CachedPublicCatalog::with_ttl(inner.clone(), ttl), inner)
    }

    #[tokio::test]
    async fn the_same_landing_page_is_served_from_memory_until_it_expires() {
        let (catalog, inner) = cached(Duration::from_secs(60));

        let first = catalog.search(&landing(1)).await.unwrap();
        let second = catalog.search(&landing(1)).await.unwrap();

        assert_eq!(first, second);
        assert_eq!(inner.searches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_expired_page_is_fetched_again() {
        let (catalog, inner) = cached(Duration::ZERO);

        catalog.search(&landing(1)).await.unwrap();
        catalog.search(&landing(1)).await.unwrap();

        assert_eq!(inner.searches.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn pages_that_differ_in_any_way_are_cached_apart() {
        let (catalog, inner) = cached(Duration::from_secs(60));
        let popular = CatalogQuery { sort: CatalogSort::Popular, ..landing(1) };
        let npm_only = CatalogQuery { format: Some(RepositoryFormat::Npm), ..landing(1) };

        for query in [landing(1), landing(2), popular, npm_only] {
            catalog.search(&query).await.unwrap();
        }

        assert_eq!(inner.searches.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn searches_with_text_or_a_wider_scope_are_never_cached() {
        let (catalog, inner) = cached(Duration::from_secs(60));
        let with_text = CatalogQuery { text: Some("pad".into()), sort: CatalogSort::Relevance, ..landing(1) };
        let scoped = CatalogQuery { scope: CatalogScope::AllRepositories, ..landing(1) };

        for query in [&with_text, &with_text, &scoped, &scoped] {
            catalog.search(query).await.unwrap();
        }

        assert_eq!(inner.searches.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn entry_counts_are_cached_too() {
        let (catalog, inner) = cached(Duration::from_secs(60));

        let first = catalog.entry_counts().await.unwrap();
        let second = catalog.entry_counts().await.unwrap();

        assert_eq!(first, second);
        assert_eq!(inner.counts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_cache_never_holds_more_than_its_bound() {
        let (catalog, _) = cached(Duration::from_secs(60));

        for page in 1..=(MAX_CACHED_PAGES as u32 + 40) {
            catalog.search(&landing(page)).await.unwrap();
        }

        assert_eq!(catalog.pages.lock().unwrap().len(), MAX_CACHED_PAGES);
    }
}
