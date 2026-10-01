use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::DomainError;
use uuid::Uuid;

use crate::package_repository::{RepositoryFormat, RepositoryType};

/// One public catalog per format. A new format means an entry here and a query branch in the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogFormatSpec {
    pub format: RepositoryFormat,
    pub catalog_name: &'static str,
    pub label: &'static str,
}

pub const CATALOG_FORMATS: [CatalogFormatSpec; 2] = [
    CatalogFormatSpec { format: RepositoryFormat::Npm, catalog_name: "artiferris-npm", label: "npm" },
    CatalogFormatSpec { format: RepositoryFormat::Docker, catalog_name: "artiferris-docker", label: "Docker" },
];

pub fn catalog_format_spec(format: RepositoryFormat) -> Option<&'static CatalogFormatSpec> {
    CATALOG_FORMATS.iter().find(|spec| spec.format == format)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogSort {
    Relevance,
    Updated,
    /// Most downloaded over the last seven days first.
    Popular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerKind {
    Personal,
    Organization,
}

/// The best tier an entry qualified for, best first. Also the primary ranking key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CatalogMatch {
    Exact,
    Prefix,
    Contains,
    Text,
    /// A typo-tolerant name match, only for search text of three characters or more.
    Fuzzy,
}

/// Who owns a repository: a user's personal namespace (by username) or an organization (by slug).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRef {
    pub kind: OwnerKind,
    pub slug: String,
}

/// Which repositories a search looks into. Public content is always included, whatever the scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogScope {
    /// What anyone may see: public hosted repositories.
    PublicOnly,
    /// Public hosted repositories plus exactly these ones, proxies included (they contribute what they have cached).
    Repositories(Vec<Uuid>),
    /// Every hosted and proxy repository, for a caller who may read them all.
    AllRepositories,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogQuery {
    pub scope: CatalogScope,
    pub text: Option<String>,
    pub format: Option<RepositoryFormat>,
    pub owner: Option<OwnerRef>,
    pub sort: CatalogSort,
    pub page: u32,
    pub per_page: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogOwner {
    pub kind: OwnerKind,
    /// The username for a personal owner, the organization slug otherwise.
    pub slug: String,
    pub display_name: String,
    /// The public organization is served on the main host; every other one on its own subdomain.
    pub is_public_organization: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub format: RepositoryFormat,
    pub name: String,
    pub description: Option<String>,
    pub keywords: Vec<String>,
    /// The latest version (npm) or the most recently updated tag (Docker).
    pub latest: Option<String>,
    pub updated_at: DateTime<Utc>,
    /// Downloads over the last seven days: indicative, and never a count of who downloaded.
    pub downloads_7d: i64,
    /// `None` when the search had no text, so nothing was matched against.
    pub match_kind: Option<CatalogMatch>,
    pub repository_id: Uuid,
    pub repository_type: RepositoryType,
    pub repository_name: String,
    pub owner: CatalogOwner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPage {
    pub items: Vec<CatalogEntry>,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerSummary {
    pub kind: OwnerKind,
    pub slug: String,
    pub display_name: String,
    pub repository_count: i64,
    pub package_count: i64,
    pub image_count: i64,
    /// The owner keeps its pages away from search engines: they carry `noindex` and are not in the sitemap.
    pub indexing_blocked: bool,
}

/// What the search box offers while typing: a name and where it lives, nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSuggestion {
    pub format: RepositoryFormat,
    pub name: String,
    pub repository_name: String,
    pub owner: CatalogOwner,
}

/// What the search box asks for while typing, narrowed the same way a search is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestQuery {
    pub text: String,
    pub limit: u32,
    pub format: Option<RepositoryFormat>,
    pub owner: Option<OwnerRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntryCount {
    pub format: RepositoryFormat,
    pub entry_count: i64,
}

/// A public hosted repository, as the catalog knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRepository {
    pub name: String,
    pub format: RepositoryFormat,
}

/// One public page worth listing in a sitemap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SitemapTarget {
    Owner,
    Repository { repository: String },
    Package { repository: String, format: RepositoryFormat, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SitemapEntry {
    pub owner: OwnerRef,
    pub target: SitemapTarget,
    pub updated_at: DateTime<Utc>,
}

/// Read-only view over what anyone may see without authenticating. Implementations must enforce
/// visibility in the query itself, never by filtering afterwards.
#[async_trait]
pub trait PublicCatalogPort: Send + Sync {
    async fn search(&self, query: &CatalogQuery) -> Result<CatalogPage, DomainError>;
    async fn entry_counts(&self) -> Result<Vec<CatalogEntryCount>, DomainError>;
    /// Best name matches first: exact, prefix, substring, then approximate ones.
    async fn suggest(&self, query: &SuggestQuery) -> Result<Vec<CatalogSuggestion>, DomainError>;
    /// `None` when the owner has no public repository, which is also what an unknown owner gets.
    /// `None` unless that owner has a public hosted repository of that name.
    async fn repository(&self, owner: &OwnerRef, repository: &str) -> Result<Option<CatalogRepository>, DomainError>;
    /// The one public entry with exactly this owner, repository, format and name; no fuzzy matching.
    async fn find_entry(&self, owner: &OwnerRef, repository: &str, format: RepositoryFormat, name: &str) -> Result<Option<CatalogEntry>, DomainError>;
    /// Every owner, repository, package and image that is public, in a stable order, at most `limit` of them.
    async fn sitemap_entries(&self, limit: usize) -> Result<Vec<SitemapEntry>, DomainError>;
    async fn owner_summary(&self, owner: &OwnerRef) -> Result<Option<OwnerSummary>, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_repository_format_has_exactly_one_catalog() {
        for format in [RepositoryFormat::Npm, RepositoryFormat::Docker] {
            assert_eq!(CATALOG_FORMATS.iter().filter(|spec| spec.format == format).count(), 1);
        }
    }

    #[test]
    fn catalog_names_all_carry_the_reserved_prefix() {
        for spec in CATALOG_FORMATS {
            assert!(crate::reserved_names::is_reserved_name(spec.catalog_name), "{} would not be protected", spec.catalog_name);
        }
    }

    #[test]
    fn match_tiers_sort_best_first() {
        let mut tiers = vec![CatalogMatch::Fuzzy, CatalogMatch::Text, CatalogMatch::Exact, CatalogMatch::Contains, CatalogMatch::Prefix];
        tiers.sort();
        assert_eq!(tiers, vec![CatalogMatch::Exact, CatalogMatch::Prefix, CatalogMatch::Contains, CatalogMatch::Text, CatalogMatch::Fuzzy]);
    }
}
