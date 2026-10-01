use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use artiferris_domain::error::DomainError;
use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
use artiferris_domain::public_catalog::{
    CatalogEntry, CatalogEntryCount, CatalogMatch, CatalogOwner, CatalogPage, CatalogQuery, CatalogSort, OwnerKind, CatalogRepository, CatalogScope, CatalogSuggestion, OwnerRef, OwnerSummary, PublicCatalogPort, SitemapEntry, SitemapTarget, SuggestQuery,
};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::error_ext::InfraErr;

/// Anyone can hit these queries, so they get a time limit of their own and a few slots out of the shared pool.
const STATEMENT_TIMEOUT_MS: u32 = 3_000;
const CONCURRENT_QUERIES: usize = 4;
const SLOT_WAIT: Duration = Duration::from_secs(3);
/// The sitemap aggregates the whole catalog, so it gets a longer limit and a slot of its own instead of sharing the user-facing ones.
const SITEMAP_STATEMENT_TIMEOUT_MS: u32 = 15_000;
const SITEMAP_SLOTS: usize = 1;
/// SQLSTATE `query_canceled`, what `statement_timeout` raises.
const QUERY_CANCELED: &str = "57014";

pub struct PostgresPublicCatalog {
    pool: PgPool,
    slots: Arc<Semaphore>,
    sitemap_slots: Arc<Semaphore>,
    slot_wait: Duration,
}

fn query_err(error: sqlx::Error) -> DomainError {
    match &error {
        sqlx::Error::Database(db) if db.code().as_deref() == Some(QUERY_CANCELED) => DomainError::Busy("the public catalog query took too long".to_string()),
        _ => DomainError::Infrastructure(error.to_string()),
    }
}

impl PostgresPublicCatalog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool, slots: Arc::new(Semaphore::new(CONCURRENT_QUERIES)), sitemap_slots: Arc::new(Semaphore::new(SITEMAP_SLOTS)), slot_wait: SLOT_WAIT }
    }

    /// A read-only transaction holding one of the query slots, with the statement timeout in force until it ends.
    async fn begin(&self) -> Result<(Transaction<'static, Postgres>, OwnedSemaphorePermit), DomainError> {
        self.begin_in(&self.slots, STATEMENT_TIMEOUT_MS).await
    }

    async fn begin_in(&self, slots: &Arc<Semaphore>, statement_timeout_ms: u32) -> Result<(Transaction<'static, Postgres>, OwnedSemaphorePermit), DomainError> {
        let permit = tokio::time::timeout(self.slot_wait, slots.clone().acquire_owned())
            .await
            .map_err(|_| DomainError::Busy("the public catalog is busy".to_string()))?
            .infra_err()?;
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query(AssertSqlSafe(format!("SET LOCAL statement_timeout = {statement_timeout_ms}"))).execute(&mut *tx).await.infra_err()?;
        Ok((tx, permit))
    }
}

/// Visibility is decided in SQL, so a private repository cannot reach a result through a later branch. A personal owner
/// is the user whose personal organization holds the repository (slug `'u' || first 24 hex of the user id`, see
/// `personal_organization_slug`). The filter is spliced in by the macros, never at run time.
macro_rules! public_repositories {
    ($filter:expr) => {
        concat!(
            r#"public_repositories AS (
    SELECT p.id, p.name AS repository_name, p.format, p.repo_type, p.updated_at AS repository_updated_at,
           CASE WHEN o.is_personal THEN 'personal' ELSE 'organization' END AS owner_kind,
           CASE WHEN o.is_personal THEN u.username ELSE o.slug END AS owner_slug,
           CASE WHEN o.is_personal THEN u.username ELSE o.display_name END AS owner_display_name,
           o.is_public AS owner_is_public_organization,
           COALESCE(ss.seo_indexing_blocked, false) AS indexing_blocked
    FROM package_repository_projections p
    JOIN organizations o ON o.id = p.organization_id
    LEFT JOIN system_settings ss ON ss.organization_id = o.id
    LEFT JOIN users u ON o.is_personal AND o.slug = 'u' || left(replace(u.id::text, '-', ''), 24)
    WHERE "#,
            $filter,
            r#"
      AND (NOT o.is_personal OR u.id IS NOT NULL)
)"#
        )
    };
}

macro_rules! public_filter {
    () => {
        "p.repo_type = 'hosted' AND p.is_public AND p.deleted_at IS NULL AND COALESCE(ss.public_page_enabled, true) \
         AND COALESCE((SELECT i.public_page_enabled FROM system_settings i JOIN organizations io ON io.id = i.organization_id WHERE io.is_public), true)"
    };
}

/// A wider scope adds to the public repositories, never replaces them, and never includes a group (which has no
/// content of its own) or a deleted repository.
macro_rules! all_filter {
    () => {
        "p.repo_type IN ('hosted', 'proxy') AND p.deleted_at IS NULL"
    };
}

macro_rules! listed_filter {
    () => {
        "p.repo_type IN ('hosted', 'proxy') AND p.deleted_at IS NULL AND (p.id = ANY($10) OR (p.repo_type = 'hosted' AND p.is_public))"
    };
}

const PUBLIC_REPOSITORIES: &str = concat!("WITH ", public_repositories!(public_filter!()), "\n");

/// Every search statement takes the same thirteen parameters so one binding path serves all variants. $1 lowercased
/// text, $2 `%text%`, $3 `text%`, $4 format, $5 limit, $6 offset, $7/$8 owner kind and slug, $9 names only, $10 ids a
/// signed-in scope adds, $11/$12 repository and entry name for an exact lookup, $13 whether downloads decide the order.
macro_rules! search_params {
    () => {
        "WITH params AS (SELECT $9::bool AS names_only, $10::uuid[] AS scope_ids, $11::text AS repository_name, $12::text AS entry_name, $13::bool AS rank_by_downloads),\n"
    };
}

const SEARCH_PUBLIC: &str = concat!(search_params!(), public_repositories!(public_filter!()));
const SEARCH_ALL: &str = concat!(search_params!(), public_repositories!(all_filter!()));
const SEARCH_LISTED: &str = concat!(search_params!(), public_repositories!(listed_filter!()));

fn repositories_cte(scope: &CatalogScope) -> &'static str {
    match scope {
        CatalogScope::PublicOnly => SEARCH_PUBLIC,
        CatalogScope::AllRepositories => SEARCH_ALL,
        CatalogScope::Repositories(_) => SEARCH_LISTED,
    }
}

/// Two stages keep the expensive per-entry work to one page. Stage 1 (`candidates`) narrows by name or text through the
/// indexes, tiers and pages. Stage 2 (`page`) fetches description, keywords, latest version and downloads for those
/// rows. `updated_at` is the newest publication of any version (npm) or tag (Docker). A text hit matches an npm
/// version's description or keywords, or an exact Docker tag. The fuzzy tier is a trigram name match (similarity 0.3+),
/// for text of three characters or more. Without text every public entry is a candidate.
const CANDIDATES_WITHOUT_TEXT: &str = r#"
, npm_scope AS (
    SELECT k.id, k.name, false AS text_hit, r.id AS repository_id, r.repository_name
    FROM public_repositories r
    JOIN npm_packages k ON k.package_repository_id = r.id
    WHERE r.format = 'npm' AND ($4::text IS NULL OR $4 = 'npm')
      AND ($7::text IS NULL OR (r.owner_kind = $7 AND r.owner_slug = $8))
      AND ($11::text IS NULL OR (r.repository_name = $11 AND k.name = $12))
)
, docker_images AS (
    SELECT r.id AS repository_id, r.repository_name, i.image_name AS name, false AS text_hit, i.updated_at
    FROM public_repositories r
    JOIN LATERAL (
        SELECT t.image_name, max(t.updated_at) AS updated_at FROM docker_tags t
        WHERE t.package_repository_id = r.id AND ($12::text IS NULL OR t.image_name = $12)
        GROUP BY t.image_name
    ) i ON true
    WHERE r.format = 'docker' AND ($4::text IS NULL OR $4 = 'docker')
      AND ($7::text IS NULL OR (r.owner_kind = $7 AND r.owner_slug = $8))
      AND ($11::text IS NULL OR r.repository_name = $11)
)
"#;

/// With text, the driver is the small set of entries that match it, found through the trigram, full-text and tag indexes.
const CANDIDATES_WITH_TEXT: &str = r#"
, npm_matches AS (
    SELECT id, bool_or(by_text) AS text_hit FROM (
        SELECT k.id, false AS by_text FROM npm_packages k
        WHERE k.name ILIKE $2 ESCAPE '\' OR (char_length($1) >= 3 AND k.name % $1)
        UNION ALL
        SELECT fv.npm_package_id, true FROM npm_package_versions fv
        WHERE NOT $9::bool
          AND to_tsvector('simple', coalesce(fv.manifest ->> 'description', '') || ' ' || coalesce(fv.manifest ->> 'keywords', '')) @@ plainto_tsquery('simple', $1)
    ) matched GROUP BY id
)
, npm_scope AS (
    SELECT k.id, k.name, m.text_hit, r.id AS repository_id, r.repository_name
    FROM npm_matches m
    JOIN npm_packages k ON k.id = m.id
    JOIN public_repositories r ON r.id = k.package_repository_id AND r.format = 'npm'
    WHERE ($4::text IS NULL OR $4 = 'npm')
      AND ($7::text IS NULL OR (r.owner_kind = $7 AND r.owner_slug = $8))
)
, docker_matches AS (
    SELECT package_repository_id, image_name, bool_or(by_text) AS text_hit FROM (
        SELECT m.package_repository_id, m.image_name, false AS by_text FROM docker_tags m
        WHERE m.image_name ILIKE $2 ESCAPE '\' OR (char_length($1) >= 3 AND m.image_name % $1)
        UNION ALL
        SELECT m.package_repository_id, m.image_name, true FROM docker_tags m
        WHERE NOT $9::bool AND lower(m.tag) = $1
    ) matched GROUP BY package_repository_id, image_name
)
, docker_images AS (
    SELECT r.id AS repository_id, r.repository_name, m.image_name AS name, m.text_hit, u.updated_at
    FROM docker_matches m
    JOIN public_repositories r ON r.id = m.package_repository_id AND r.format = 'docker'
    JOIN LATERAL (SELECT max(t.updated_at) AS updated_at FROM docker_tags t WHERE t.package_repository_id = m.package_repository_id AND t.image_name = m.image_name) u ON true
    WHERE ($4::text IS NULL OR $4 = 'docker')
      AND ($7::text IS NULL OR (r.owner_kind = $7 AND r.owner_slug = $8))
)
"#;

/// Candidates carry only what ordering needs; owner details and extras are joined after the LIMIT. Downloads are summed
/// over seven days, only when they decide the order.
const CANDIDATES_COMMON: &str = r#"
, recent_downloads AS (
    SELECT ds.package_repository_id, ds.kind, ds.name, sum(ds.downloads)::bigint AS downloads
    FROM download_stats ds
    WHERE $13 AND ds.day > (now() AT TIME ZONE 'utc')::date - 7
      AND ds.package_repository_id IN (SELECT repository_id FROM npm_scope UNION SELECT repository_id FROM docker_images)
    GROUP BY ds.package_repository_id, ds.kind, ds.name
)
, npm_candidates AS (
    SELECT 'npm'::text AS kind, s.id AS package_id, NULL::uuid AS docker_repository_id, s.name, u.updated_at, s.text_hit,
           coalesce(d.downloads, 0) AS downloads_7d, s.repository_id, s.repository_name
    FROM npm_scope s
    JOIN (
        SELECT v.npm_package_id, max(v.published_at) AS updated_at FROM npm_package_versions v
        WHERE v.npm_package_id IN (SELECT id FROM npm_scope) GROUP BY v.npm_package_id
    ) u ON u.npm_package_id = s.id
    LEFT JOIN recent_downloads d ON d.package_repository_id = s.repository_id AND d.kind = 'npm' AND d.name = s.name
)
, docker_candidates AS (
    SELECT 'docker'::text AS kind, NULL::uuid AS package_id, i.repository_id AS docker_repository_id, i.name, i.updated_at, i.text_hit,
           coalesce(d.downloads, 0) AS downloads_7d, i.repository_id, i.repository_name
    FROM docker_images i
    LEFT JOIN recent_downloads d ON d.package_repository_id = i.repository_id AND d.kind = 'docker' AND d.name = i.name
),
"#;

const SEARCH_TAIL: &str = r#"
candidates AS (
    SELECT * FROM (
        SELECT c.*,
               CASE WHEN $1::text IS NULL THEN NULL
                    WHEN lower(c.name) = $1 THEN 0
                    WHEN c.name ILIKE $3 ESCAPE '\' THEN 1
                    WHEN c.name ILIKE $2 ESCAPE '\' THEN 2
                    WHEN c.text_hit THEN 3
                    WHEN char_length($1) >= 3 AND c.name % $1 THEN 4
               END AS tier
        FROM (SELECT * FROM npm_candidates UNION ALL SELECT * FROM docker_candidates) c
    ) ranked
    WHERE $1::text IS NULL OR tier IS NOT NULL
),
page AS (
    SELECT *, count(*) OVER () AS total
    FROM candidates
    ORDER BY __ORDER__
    LIMIT $5 OFFSET $6
)
SELECT p.kind, p.name, p.description, p.keywords, p.latest, p.updated_at, coalesce(dl.downloads, 0)::bigint AS downloads_7d, p.tier, p.repository_id,
       r.repo_type AS repository_type, p.repository_name, r.owner_kind, r.owner_slug, r.owner_display_name, r.owner_is_public_organization, p.total
FROM (
    SELECT pg.*, n.description, coalesce(n.keywords, ARRAY[]::text[]) AS keywords, coalesce(n.latest, d.latest_tag) AS latest
    FROM page pg
    LEFT JOIN LATERAL (
        SELECT v.manifest ->> 'description' AS description,
               CASE WHEN jsonb_typeof(v.manifest -> 'keywords') = 'array'
                    THEN ARRAY(SELECT jsonb_array_elements_text(v.manifest -> 'keywords'))
                    ELSE ARRAY[]::text[] END AS keywords,
               v.version AS latest
        FROM npm_package_versions v
        WHERE pg.kind = 'npm' AND v.npm_package_id = pg.package_id
        ORDER BY v.version IS NOT DISTINCT FROM (SELECT d.version FROM npm_dist_tags d WHERE d.npm_package_id = pg.package_id AND d.tag = 'latest') DESC,
                 v.published_at DESC
        LIMIT 1
    ) n ON true
    LEFT JOIN LATERAL (
        SELECT (array_agg(t.tag ORDER BY t.updated_at DESC, t.tag = 'latest' DESC, t.tag DESC))[1] AS latest_tag
        FROM docker_tags t
        WHERE pg.kind = 'docker' AND t.package_repository_id = pg.docker_repository_id AND t.image_name = pg.name
    ) d ON true
) p
JOIN public_repositories r ON r.id = p.repository_id
LEFT JOIN LATERAL (
    SELECT sum(ds.downloads) AS downloads FROM download_stats ds
    WHERE ds.package_repository_id = p.repository_id AND ds.kind = p.kind AND ds.name = p.name AND ds.day > (now() AT TIME ZONE 'utc')::date - 7
) dl ON true
ORDER BY __ORDER__
"#;

/// $1 owner kind, $2 owner slug. Counts what is public for that owner, on the same visibility rules as the search.
const OWNER_SUMMARY: &str = r#"
, owned AS (SELECT * FROM public_repositories r WHERE r.owner_kind = $1 AND r.owner_slug = $2)
SELECT (SELECT owner_display_name FROM owned LIMIT 1) AS display_name,
       (SELECT count(*) FROM owned) AS repository_count,
       (SELECT count(*) FROM owned r JOIN npm_packages k ON k.package_repository_id = r.id
         WHERE r.format = 'npm' AND EXISTS (SELECT 1 FROM npm_package_versions v WHERE v.npm_package_id = k.id)) AS package_count,
       (SELECT count(*) FROM (
            SELECT 1 FROM owned r JOIN docker_tags t ON t.package_repository_id = r.id
            WHERE r.format = 'docker' GROUP BY t.package_repository_id, t.image_name
        ) images) AS image_count,
       (SELECT COALESCE(bool_or(indexing_blocked), false) FROM owned) AS indexing_blocked
"#;

/// $1 the most rows to return. What an owner keeps away from search engines is left out.
const SITEMAP: &str = r#"
SELECT owner_kind, owner_slug, NULL::text AS repository_name, NULL::text AS kind, NULL::text AS name, max(repository_updated_at) AS updated_at
FROM public_repositories WHERE NOT indexing_blocked GROUP BY owner_kind, owner_slug
UNION ALL
SELECT owner_kind, owner_slug, repository_name, NULL, NULL, repository_updated_at FROM public_repositories WHERE NOT indexing_blocked
UNION ALL
SELECT r.owner_kind, r.owner_slug, r.repository_name, 'npm', k.name, max(v.published_at)
FROM public_repositories r JOIN npm_packages k ON k.package_repository_id = r.id JOIN npm_package_versions v ON v.npm_package_id = k.id
WHERE r.format = 'npm' AND NOT r.indexing_blocked GROUP BY r.owner_kind, r.owner_slug, r.repository_name, k.name
UNION ALL
SELECT r.owner_kind, r.owner_slug, r.repository_name, 'docker', t.image_name, max(t.updated_at)
FROM public_repositories r JOIN docker_tags t ON t.package_repository_id = r.id
WHERE r.format = 'docker' AND NOT r.indexing_blocked GROUP BY r.owner_kind, r.owner_slug, r.repository_name, t.image_name
ORDER BY 1, 2, 3 NULLS FIRST, 4 NULLS FIRST, 5 NULLS FIRST
LIMIT $1
"#;

const ENTRY_COUNTS: &str = r#"
SELECT 'npm'::text AS kind, count(*) AS entry_count FROM npm_packages k
JOIN public_repositories r ON r.id = k.package_repository_id AND r.format = 'npm'
WHERE EXISTS (SELECT 1 FROM npm_package_versions v WHERE v.npm_package_id = k.id)
UNION ALL
SELECT 'docker', count(*) FROM (
    SELECT 1 FROM docker_tags t JOIN public_repositories r ON r.id = t.package_repository_id AND r.format = 'docker'
    GROUP BY t.package_repository_id, t.image_name
) images
"#;

#[derive(sqlx::FromRow)]
struct EntryRow {
    kind: String,
    name: String,
    description: Option<String>,
    keywords: Vec<String>,
    latest: Option<String>,
    updated_at: chrono::DateTime<chrono::Utc>,
    downloads_7d: i64,
    tier: Option<i32>,
    repository_id: Uuid,
    repository_type: String,
    repository_name: String,
    owner_kind: String,
    owner_slug: String,
    owner_display_name: String,
    owner_is_public_organization: bool,
    total: i64,
}

impl EntryRow {
    fn into_entry(self) -> Result<CatalogEntry, DomainError> {
        let format = match self.kind.as_str() {
            "npm" => RepositoryFormat::Npm,
            "docker" => RepositoryFormat::Docker,
            other => return Err(DomainError::Infrastructure(format!("unknown catalog entry kind {other}"))),
        };
        let match_kind = self.tier.map(|tier| match tier {
            0 => CatalogMatch::Exact,
            1 => CatalogMatch::Prefix,
            2 => CatalogMatch::Contains,
            3 => CatalogMatch::Text,
            _ => CatalogMatch::Fuzzy,
        });
        Ok(CatalogEntry {
            format,
            name: self.name,
            description: self.description,
            keywords: self.keywords,
            latest: self.latest,
            updated_at: self.updated_at,
            downloads_7d: self.downloads_7d,
            match_kind,
            repository_id: self.repository_id,
            repository_type: if self.repository_type == "proxy" { RepositoryType::Proxy } else { RepositoryType::Hosted },
            repository_name: self.repository_name,
            owner: CatalogOwner {
                kind: if self.owner_kind == "personal" { OwnerKind::Personal } else { OwnerKind::Organization },
                slug: self.owner_slug,
                display_name: self.owner_display_name,
                is_public_organization: self.owner_is_public_organization,
            },
        })
    }
}

fn owner_kind_key(kind: OwnerKind) -> &'static str {
    match kind {
        OwnerKind::Personal => "personal",
        OwnerKind::Organization => "organization",
    }
}

fn format_key(format: RepositoryFormat) -> &'static str {
    match format {
        RepositoryFormat::Npm => "npm",
        RepositoryFormat::Docker => "docker",
    }
}

/// `%`, `_` and the escape character itself must not act as wildcards in user text.
fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

#[async_trait]
impl PublicCatalogPort for PostgresPublicCatalog {
    async fn search(&self, query: &CatalogQuery) -> Result<CatalogPage, DomainError> {
        let offset = i64::from(query.page - 1) * i64::from(query.per_page);
        let rows = self.run(query, i64::from(query.per_page), offset, false, None).await?;
        let total = match rows.first() {
            Some(row) => row.total,
            None if offset > 0 => self.run(query, 1, 0, false, None).await?.first().map_or(0, |row| row.total),
            None => 0,
        };
        let items = rows.into_iter().map(EntryRow::into_entry).collect::<Result<Vec<_>, _>>()?;
        Ok(CatalogPage { items, total })
    }

    async fn suggest(&self, suggest: &SuggestQuery) -> Result<Vec<CatalogSuggestion>, DomainError> {
        let limit = suggest.limit;
        let query = CatalogQuery { scope: CatalogScope::PublicOnly, text: Some(suggest.text.clone()), format: suggest.format, owner: suggest.owner.clone(), sort: CatalogSort::Relevance, page: 1, per_page: limit };
        self.run(&query, i64::from(limit), 0, true, None)
            .await?
            .into_iter()
            .map(|row| {
                let entry = row.into_entry()?;
                Ok(CatalogSuggestion { format: entry.format, name: entry.name, repository_name: entry.repository_name, owner: entry.owner })
            })
            .collect()
    }

    async fn find_entry(&self, owner: &OwnerRef, repository: &str, format: RepositoryFormat, name: &str) -> Result<Option<CatalogEntry>, DomainError> {
        let query = CatalogQuery { scope: CatalogScope::PublicOnly, text: None, format: Some(format), owner: Some(owner.clone()), sort: CatalogSort::Updated, page: 1, per_page: 1 };
        self.run(&query, 1, 0, false, Some((repository, name))).await?.into_iter().next().map(EntryRow::into_entry).transpose()
    }

    async fn repository(&self, owner: &OwnerRef, repository: &str) -> Result<Option<CatalogRepository>, DomainError> {
        let (mut tx, _slot) = self.begin().await?;
        let row: Option<(String, String)> = sqlx::query_as(AssertSqlSafe(format!(
            "{PUBLIC_REPOSITORIES} SELECT r.repository_name, r.format FROM public_repositories r WHERE r.owner_kind = $1 AND r.owner_slug = $2 AND r.repository_name = $3"
        )))
        .bind(owner_kind_key(owner.kind))
        .bind(&owner.slug)
        .bind(repository)
        .fetch_optional(&mut *tx)
        .await
        .map_err(query_err)?;
        Ok(row.and_then(|(name, format)| {
            let format = match format.as_str() {
                "npm" => RepositoryFormat::Npm,
                "docker" => RepositoryFormat::Docker,
                _ => return None,
            };
            Some(CatalogRepository { name, format })
        }))
    }

    async fn sitemap_entries(&self, limit: usize) -> Result<Vec<SitemapEntry>, DomainError> {
        type Row = (String, String, Option<String>, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>);
        let (mut tx, _slot) = self.begin_in(&self.sitemap_slots, SITEMAP_STATEMENT_TIMEOUT_MS).await?;
        let rows: Vec<Row> = sqlx::query_as(AssertSqlSafe(format!("{PUBLIC_REPOSITORIES}{SITEMAP}"))).bind(i64::try_from(limit).unwrap_or(i64::MAX)).fetch_all(&mut *tx).await.map_err(query_err)?;
        rows.into_iter()
            .map(|(owner_kind, owner_slug, repository, kind, name, updated_at)| {
                let owner = OwnerRef { kind: if owner_kind == "personal" { OwnerKind::Personal } else { OwnerKind::Organization }, slug: owner_slug };
                let target = match (repository, kind.as_deref(), name) {
                    (None, _, _) => SitemapTarget::Owner,
                    (Some(repository), None, _) => SitemapTarget::Repository { repository },
                    (Some(repository), Some(kind), Some(name)) => {
                        let format = match kind {
                            "npm" => RepositoryFormat::Npm,
                            "docker" => RepositoryFormat::Docker,
                            other => return Err(DomainError::Infrastructure(format!("unknown sitemap kind {other}"))),
                        };
                        SitemapTarget::Package { repository, format, name }
                    }
                    (Some(_), Some(_), None) => return Err(DomainError::Infrastructure("sitemap package row without a name".to_string())),
                };
                Ok(SitemapEntry { owner, target, updated_at })
            })
            .collect()
    }

    async fn owner_summary(&self, owner: &OwnerRef) -> Result<Option<OwnerSummary>, DomainError> {
        let (mut tx, _slot) = self.begin().await?;
        let (display_name, repository_count, package_count, image_count, indexing_blocked): (Option<String>, i64, i64, i64, bool) =
            sqlx::query_as(AssertSqlSafe(format!("{PUBLIC_REPOSITORIES}{OWNER_SUMMARY}")))
                .bind(owner_kind_key(owner.kind))
                .bind(&owner.slug)
                .fetch_one(&mut *tx)
                .await
                .map_err(query_err)?;
        Ok(display_name.filter(|_| repository_count > 0).map(|display_name| OwnerSummary {
            kind: owner.kind,
            slug: owner.slug.clone(),
            display_name,
            repository_count,
            package_count,
            image_count,
            indexing_blocked,
        }))
    }

    async fn entry_counts(&self) -> Result<Vec<CatalogEntryCount>, DomainError> {
        let (mut tx, _slot) = self.begin().await?;
        let rows: Vec<(String, i64)> = sqlx::query_as(AssertSqlSafe(format!("{PUBLIC_REPOSITORIES}{ENTRY_COUNTS}"))).fetch_all(&mut *tx).await.map_err(query_err)?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, entry_count)| match kind.as_str() {
                "npm" => Some(CatalogEntryCount { format: RepositoryFormat::Npm, entry_count }),
                "docker" => Some(CatalogEntryCount { format: RepositoryFormat::Docker, entry_count }),
                _ => None,
            })
            .filter(|count| count.entry_count > 0)
            .collect())
    }
}

impl PostgresPublicCatalog {
    /// `exact` narrows the (text-less) search to one repository name and one entry name.
    async fn run(&self, query: &CatalogQuery, limit: i64, offset: i64, names_only: bool, exact: Option<(&str, &str)>) -> Result<Vec<EntryRow>, DomainError> {
        let text = query.text.as_deref().map(str::to_lowercase);
        let escaped = text.as_deref().map(escape_like);
        let prefix = escaped.as_ref().map(|e| format!("{e}%"));
        let contains = match (&escaped, names_only && text.as_deref().is_some_and(|t| t.chars().count() < 3)) {
            (Some(_), true) => prefix.clone(),
            (escaped, _) => escaped.as_ref().map(|e| format!("%{e}%")),
        };
        let order = match (query.sort, text.is_some()) {
            (CatalogSort::Relevance, true) => "tier, CASE WHEN tier = 4 THEN similarity(name, $1) END DESC NULLS LAST, downloads_7d DESC, updated_at DESC, name, repository_name",
            (CatalogSort::Popular, _) => "downloads_7d DESC, updated_at DESC, name, repository_name",
            _ => "updated_at DESC, name, repository_name",
        };
        let rank_by_downloads = text.is_some() || query.sort == CatalogSort::Popular;
        let listed: &[Uuid] = match &query.scope {
            CatalogScope::Repositories(ids) => ids,
            _ => &[],
        };
        let scope = if text.is_some() { CANDIDATES_WITH_TEXT } else { CANDIDATES_WITHOUT_TEXT };
        let sql = format!("{}{scope}{CANDIDATES_COMMON}{}", repositories_cte(&query.scope), SEARCH_TAIL.replace("__ORDER__", order));
        let (mut tx, _slot) = self.begin().await?;
        sqlx::query_as(AssertSqlSafe(sql))
            .bind(text.as_deref())
            .bind(contains.as_deref())
            .bind(prefix.as_deref())
            .bind(query.format.map(format_key))
            .bind(limit)
            .bind(offset)
            .bind(query.owner.as_ref().map(|owner| owner_kind_key(owner.kind)))
            .bind(query.owner.as_ref().map(|owner| owner.slug.as_str()))
            .bind(names_only)
            .bind(listed)
            .bind(exact.map(|(repository, _)| repository))
            .bind(exact.map(|(_, name)| name))
            .bind(rank_by_downloads)
            .fetch_all(&mut *tx)
            .await
            .map_err(query_err)
    }
}

#[cfg(test)]
mod tests {
    use artiferris_application::use_cases::personal_repository::personal_organization_slug;
    use chrono::{DateTime, Duration, Utc};
    use uuid::Uuid;

    use super::*;

    const PUBLIC_ORG: &str = "00000000-0000-0000-0000-000000000001";

    fn ago(days: i64) -> DateTime<Utc> {
        Utc::now() - Duration::days(days)
    }

    async fn insert_organization(pool: &PgPool, slug: &str, display_name: &str, is_personal: bool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations (id, slug, display_name, is_personal) VALUES ($1, $2, $3, $4)")
            .bind(id).bind(slug).bind(display_name).bind(is_personal)
            .execute(pool).await.unwrap();
        id
    }

    /// A user plus the personal organization derived from their id, exactly as `ReservePersonalOrganizationUseCase` does.
    async fn insert_personal_owner(pool: &PgPool, username: &str) -> Uuid {
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, password_hash, organization_id) VALUES ($1, $2, 'x', $3::uuid)")
            .bind(user_id).bind(username).bind(PUBLIC_ORG)
            .execute(pool).await.unwrap();
        insert_organization(pool, personal_organization_slug(user_id).as_str(), username, true).await
    }

    async fn insert_repository(pool: &PgPool, organization: impl std::fmt::Display, name: &str, format: &str, repo_type: &str, is_public: bool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, version, is_public) VALUES ($1, $2::uuid, $3, $4, $5, 1, $6)")
            .bind(id).bind(organization.to_string()).bind(name).bind(format).bind(repo_type).bind(is_public)
            .execute(pool).await.unwrap();
        id
    }

    async fn public_hosted(pool: &PgPool, organization: impl std::fmt::Display, name: &str, format: &str) -> Uuid {
        insert_repository(pool, organization, name, format, "hosted", true).await
    }

    struct Version<'a> {
        version: &'a str,
        published: DateTime<Utc>,
        manifest: serde_json::Value,
    }

    fn version(version: &str, published: DateTime<Utc>) -> Version<'_> {
        Version { version, published, manifest: serde_json::json!({ "name": "x", "version": version }) }
    }

    async fn add_npm(pool: &PgPool, repository: Uuid, name: &str, versions: Vec<Version<'_>>, latest_tag: Option<&str>) {
        let package_id = Uuid::new_v4();
        sqlx::query("INSERT INTO npm_packages (id, package_repository_id, name) VALUES ($1, $2, $3)")
            .bind(package_id).bind(repository).bind(name).execute(pool).await.unwrap();
        for v in versions {
            sqlx::query("INSERT INTO npm_package_versions (id, npm_package_id, version, manifest, shasum, integrity, tarball_storage_key, tarball_size_bytes, published_at, origin) VALUES ($1, $2, $3, $4, 's', 'i', 'k', 1, $5, 'local')")
                .bind(Uuid::new_v4()).bind(package_id).bind(v.version).bind(v.manifest).bind(v.published)
                .execute(pool).await.unwrap();
        }
        if let Some(tag) = latest_tag {
            sqlx::query("INSERT INTO npm_dist_tags (npm_package_id, tag, version) VALUES ($1, 'latest', $2)")
                .bind(package_id).bind(tag).execute(pool).await.unwrap();
        }
    }

    async fn add_docker(pool: &PgPool, repository: Uuid, image: &str, tags: &[(&str, DateTime<Utc>)]) {
        for (tag, updated_at) in tags {
            let manifest_id = Uuid::new_v4();
            sqlx::query("INSERT INTO docker_manifests (id, package_repository_id, image_name, digest, media_type, body) VALUES ($1, $2, $3, $4, 'm', ''::bytea)")
                .bind(manifest_id).bind(repository).bind(image).bind(format!("sha256:{manifest_id}"))
                .execute(pool).await.unwrap();
            sqlx::query("INSERT INTO docker_tags (package_repository_id, image_name, tag, manifest_id, updated_at) VALUES ($1, $2, $3, $4, $5)")
                .bind(repository).bind(image).bind(tag).bind(manifest_id).bind(updated_at)
                .execute(pool).await.unwrap();
        }
    }

    fn query(text: Option<&str>) -> CatalogQuery {
        CatalogQuery { scope: CatalogScope::PublicOnly, text: text.map(str::to_string), format: None, owner: None, sort: if text.is_some() { CatalogSort::Relevance } else { CatalogSort::Updated }, page: 1, per_page: 20 }
    }

    async fn names(pool: &PgPool, query: &CatalogQuery) -> Vec<String> {
        PostgresPublicCatalog::new(pool.clone()).search(query).await.unwrap().items.into_iter().map(|e| e.name).collect()
    }

    #[sqlx::test]
    async fn only_public_hosted_live_repositories_are_listed(pool: PgPool) {
        let public = public_hosted(&pool, PUBLIC_ORG, "open-npm", "npm").await;
        let private = insert_repository(&pool, PUBLIC_ORG, "closed-npm", "npm", "hosted", false).await;
        let proxy = insert_repository(&pool, PUBLIC_ORG, "mirror", "npm", "proxy", true).await;
        let group = insert_repository(&pool, PUBLIC_ORG, "grouped", "npm", "group", true).await;
        let deleted = public_hosted(&pool, PUBLIC_ORG, "gone", "npm").await;
        sqlx::query("UPDATE package_repository_projections SET deleted_at = now() WHERE id = $1").bind(deleted).execute(&pool).await.unwrap();
        for (repo, package) in [(public, "visible"), (private, "hidden-private"), (proxy, "hidden-proxy"), (group, "hidden-group"), (deleted, "hidden-deleted")] {
            add_npm(&pool, repo, package, vec![version("1.0.0", ago(1))], None).await;
        }

        assert_eq!(names(&pool, &query(None)).await, vec!["visible"]);
    }

    #[sqlx::test]
    async fn docker_images_follow_the_same_visibility_rules(pool: PgPool) {
        let public = public_hosted(&pool, PUBLIC_ORG, "open-docker", "docker").await;
        let private = insert_repository(&pool, PUBLIC_ORG, "closed-docker", "docker", "hosted", false).await;
        let proxy = insert_repository(&pool, PUBLIC_ORG, "docker-mirror", "docker", "proxy", true).await;
        for (repo, image) in [(public, "visible"), (private, "hidden-private"), (proxy, "hidden-proxy")] {
            add_docker(&pool, repo, image, &[("latest", ago(1))]).await;
        }

        assert_eq!(names(&pool, &query(None)).await, vec!["visible"]);
    }

    #[sqlx::test]
    async fn a_personal_owner_is_the_username_and_an_organization_owner_is_its_slug(pool: PgPool) {
        let personal = insert_personal_owner(&pool, "alice").await;
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let mine = public_hosted(&pool, personal, "alice-project", "npm").await;
        let theirs = public_hosted(&pool, acme, "acme-libs", "npm").await;
        add_npm(&pool, mine, "personal-pkg", vec![version("1.0.0", ago(2))], None).await;
        add_npm(&pool, theirs, "org-pkg", vec![version("1.0.0", ago(1))], None).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(None)).await.unwrap();

        let owners: Vec<_> = page.items.iter().map(|e| (e.name.as_str(), e.owner.kind, e.owner.slug.as_str(), e.owner.display_name.as_str(), e.repository_name.as_str())).collect();
        assert_eq!(
            owners,
            vec![
                ("org-pkg", OwnerKind::Organization, "acme", "Acme Corp", "acme-libs"),
                ("personal-pkg", OwnerKind::Personal, "alice", "alice", "alice-project"),
            ]
        );
    }

    #[sqlx::test]
    async fn a_personal_repository_whose_user_no_longer_exists_is_not_listed(pool: PgPool) {
        let orphan = insert_organization(&pool, personal_organization_slug(Uuid::new_v4()).as_str(), "ghost", true).await;
        let repo = public_hosted(&pool, orphan, "ghost-project", "npm").await;
        add_npm(&pool, repo, "spooky", vec![version("1.0.0", ago(1))], None).await;

        assert!(names(&pool, &query(None)).await.is_empty());
    }

    #[sqlx::test]
    async fn the_latest_dist_tag_wins_over_the_newest_version(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let stable = Version { version: "1.0.0", published: ago(10), manifest: serde_json::json!({ "description": "stable one", "keywords": ["ui"] }) };
        let beta = Version { version: "2.0.0-beta", published: ago(1), manifest: serde_json::json!({ "description": "beta one" }) };
        add_npm(&pool, repo, "widget", vec![stable, beta], Some("1.0.0")).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(None)).await.unwrap();

        let entry = &page.items[0];
        assert_eq!((entry.latest.as_deref(), entry.description.as_deref(), entry.keywords.clone()), (Some("1.0.0"), Some("stable one"), vec!["ui".to_string()]));
        assert_eq!(entry.updated_at.date_naive(), ago(1).date_naive(), "updated_at is the newest publication, whichever version is tagged latest");
    }

    #[sqlx::test]
    async fn without_a_latest_tag_the_newest_version_is_used(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "widget", vec![version("1.0.0", ago(10)), version("1.1.0", ago(2))], None).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(None)).await.unwrap();

        assert_eq!(page.items[0].latest.as_deref(), Some("1.1.0"));
    }

    #[sqlx::test]
    async fn a_package_with_no_version_left_is_not_listed(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "emptied", vec![], None).await;

        assert!(names(&pool, &query(None)).await.is_empty());
    }

    #[sqlx::test]
    async fn a_manifest_with_malformed_keywords_does_not_break_the_search(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let odd = Version { version: "1.0.0", published: ago(1), manifest: serde_json::json!({ "description": 42, "keywords": "not-an-array" }) };
        add_npm(&pool, repo, "odd-one", vec![odd], None).await;

        assert_eq!(names(&pool, &query(Some("odd"))).await, vec!["odd-one"]);
    }

    #[sqlx::test]
    async fn a_docker_image_is_one_entry_with_its_most_recently_updated_tag_as_latest(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "docker-repo", "docker").await;
        add_docker(&pool, repo, "api", &[("1.0", ago(9)), ("1.1", ago(3)), ("nightly", ago(1))]).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(None)).await.unwrap();

        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].latest.as_deref(), Some("nightly"));
        assert_eq!(page.items[0].format, RepositoryFormat::Docker);
    }

    #[sqlx::test]
    async fn when_tags_were_pushed_together_latest_is_the_one_shown(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "docker-repo", "docker").await;
        let pushed = ago(2);
        add_docker(&pool, repo, "api", &[("1.0", pushed), ("latest", pushed), ("0.9", pushed)]).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(None)).await.unwrap();

        assert_eq!(page.items[0].latest.as_deref(), Some("latest"));
    }

    #[sqlx::test]
    async fn results_rank_by_match_tier_then_recency(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let described = Version { version: "1.0.0", published: ago(1), manifest: serde_json::json!({ "description": "a tiny pad helper" }) };
        add_npm(&pool, repo, "left-padding", vec![version("1.0.0", ago(5))], None).await;
        add_npm(&pool, repo, "my-pad", vec![version("1.0.0", ago(4))], None).await;
        add_npm(&pool, repo, "pad", vec![version("1.0.0", ago(30))], None).await;
        add_npm(&pool, repo, "padlock", vec![version("1.0.0", ago(20))], None).await;
        add_npm(&pool, repo, "strings", vec![described], None).await;
        add_npm(&pool, repo, "unrelated", vec![version("1.0.0", ago(1))], None).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(Some("PAD"))).await.unwrap();

        let ranked: Vec<_> = page.items.iter().map(|e| (e.name.as_str(), e.match_kind)).collect();
        assert_eq!(
            ranked,
            vec![
                ("pad", Some(CatalogMatch::Exact)),
                ("padlock", Some(CatalogMatch::Prefix)),
                ("my-pad", Some(CatalogMatch::Contains)),
                ("left-padding", Some(CatalogMatch::Contains)),
                ("strings", Some(CatalogMatch::Text)),
            ]
        );
        assert_eq!(page.total, 5);
    }

    #[sqlx::test]
    async fn keywords_and_docker_tags_are_searchable(pool: PgPool) {
        let npm = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let docker = public_hosted(&pool, PUBLIC_ORG, "docker-repo", "docker").await;
        let keyworded = Version { version: "1.0.0", published: ago(1), manifest: serde_json::json!({ "keywords": ["logging", "structured"] }) };
        add_npm(&pool, npm, "tracer", vec![keyworded], None).await;
        add_docker(&pool, docker, "web", &[("release-7", ago(1))]).await;

        assert_eq!(names(&pool, &query(Some("structured"))).await, vec!["tracer"]);
        assert_eq!(names(&pool, &query(Some("release-7"))).await, vec!["web"]);
    }

    #[sqlx::test]
    async fn like_wildcards_in_the_text_are_matched_literally(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "plain", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, repo, "snake_case", vec![version("1.0.0", ago(2))], None).await;

        assert!(names(&pool, &query(Some("%"))).await.is_empty());
        assert_eq!(names(&pool, &query(Some("_"))).await, vec!["snake_case"]);
        assert!(names(&pool, &query(Some("p_"))).await.is_empty());
    }

    #[sqlx::test]
    async fn without_text_the_most_recent_entries_come_first_and_carry_no_match_kind(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "older", vec![version("1.0.0", ago(9))], None).await;
        add_npm(&pool, repo, "newer", vec![version("1.0.0", ago(1))], None).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(None)).await.unwrap();

        assert_eq!(page.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["newer", "older"]);
        assert!(page.items.iter().all(|e| e.match_kind.is_none()));
    }

    #[sqlx::test]
    async fn the_format_filter_restricts_the_results(pool: PgPool) {
        let npm = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let docker = public_hosted(&pool, PUBLIC_ORG, "docker-repo", "docker").await;
        add_npm(&pool, npm, "shared-name", vec![version("1.0.0", ago(1))], None).await;
        add_docker(&pool, docker, "shared-name", &[("latest", ago(2))]).await;

        let only_docker = CatalogQuery { format: Some(RepositoryFormat::Docker), ..query(Some("shared")) };
        let page = PostgresPublicCatalog::new(pool.clone()).search(&only_docker).await.unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].format, RepositoryFormat::Docker);
        assert_eq!(names(&pool, &query(Some("shared"))).await.len(), 2);
    }

    #[sqlx::test]
    async fn pagination_reports_the_total_even_past_the_last_page(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        for i in 0..5 {
            add_npm(&pool, repo, &format!("pkg-{i}"), vec![version("1.0.0", ago(i))], None).await;
        }
        let catalog = PostgresPublicCatalog::new(pool.clone());

        let second = catalog.search(&CatalogQuery { page: 2, per_page: 2, ..query(None) }).await.unwrap();
        assert_eq!((second.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), second.total), (vec!["pkg-2", "pkg-3"], 5));

        let beyond = catalog.search(&CatalogQuery { page: 9, per_page: 2, ..query(None) }).await.unwrap();
        assert_eq!((beyond.items.len(), beyond.total), (0, 5));

        let none = catalog.search(&query(Some("nothing-like-this"))).await.unwrap();
        assert_eq!((none.items.len(), none.total), (0, 0));
    }

    #[sqlx::test]
    async fn entry_counts_are_per_format_over_public_content_only(pool: PgPool) {
        let npm = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let private = insert_repository(&pool, PUBLIC_ORG, "closed", "npm", "hosted", false).await;
        add_npm(&pool, npm, "one", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, npm, "two", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, private, "secret", vec![version("1.0.0", ago(1))], None).await;

        let counts = PostgresPublicCatalog::new(pool.clone()).entry_counts().await.unwrap();

        assert_eq!(counts, vec![CatalogEntryCount { format: RepositoryFormat::Npm, entry_count: 2 }]);
    }

    #[sqlx::test]
    async fn a_typo_still_finds_the_package_after_every_other_tier(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "lodash", vec![version("1.0.0", ago(5))], None).await;
        add_npm(&pool, repo, "lodash-es", vec![version("1.0.0", ago(4))], None).await;
        add_npm(&pool, repo, "express", vec![version("1.0.0", ago(1))], None).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(Some("lodsh"))).await.unwrap();
        let exact_then_typo = PostgresPublicCatalog::new(pool.clone()).search(&query(Some("lodash"))).await.unwrap();

        assert_eq!(page.items.iter().map(|e| (e.name.as_str(), e.match_kind)).collect::<Vec<_>>(), vec![("lodash", Some(CatalogMatch::Fuzzy)), ("lodash-es", Some(CatalogMatch::Fuzzy))]);
        assert_eq!(exact_then_typo.items[0].match_kind, Some(CatalogMatch::Exact));
    }

    #[sqlx::test]
    async fn fuzzy_results_rank_by_similarity_before_recency(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "reactive-forms-kit", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, repo, "react", vec![version("1.0.0", ago(30))], None).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(Some("reakt"))).await.unwrap();

        assert_eq!(page.items.first().map(|e| e.name.as_str()), Some("react"), "the closer spelling beats the more recent package");
    }

    #[sqlx::test]
    async fn text_shorter_than_three_characters_is_never_fuzzy(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "react", vec![version("1.0.0", ago(1))], None).await;

        assert!(names(&pool, &query(Some("rx"))).await.is_empty());
    }

    #[sqlx::test]
    async fn docker_image_names_are_fuzzy_searchable_too(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "docker-repo", "docker").await;
        add_docker(&pool, repo, "postgres", &[("16", ago(1))]).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(Some("postgers"))).await.unwrap();

        assert_eq!(page.items.iter().map(|e| (e.name.as_str(), e.match_kind)).collect::<Vec<_>>(), vec![("postgres", Some(CatalogMatch::Fuzzy))]);
    }

    #[sqlx::test]
    async fn suggestions_are_names_ranked_best_first_and_capped(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let docker = public_hosted(&pool, PUBLIC_ORG, "docker-repo", "docker").await;
        let described = Version { version: "1.0.0", published: ago(1), manifest: serde_json::json!({ "description": "helps with pad things" }) };
        add_npm(&pool, repo, "pad", vec![version("1.0.0", ago(9))], None).await;
        add_npm(&pool, repo, "padlock", vec![version("1.0.0", ago(8))], None).await;
        add_npm(&pool, repo, "left-pad", vec![version("1.0.0", ago(7))], None).await;
        add_npm(&pool, repo, "unrelated-name", vec![described], None).await;
        add_docker(&pool, docker, "pad-image", &[("latest", ago(2))]).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());

        let suggestions = catalog.suggest(&SuggestQuery { text: "pad".to_string(), limit: 8, format: None, owner: None }).await.unwrap();
        let capped = catalog.suggest(&SuggestQuery { text: "pad".to_string(), limit: 2, format: None, owner: None }).await.unwrap();

        assert_eq!(suggestions.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["pad", "pad-image", "padlock", "left-pad"], "a description match is not a name suggestion");
        assert_eq!((suggestions[1].format, suggestions[1].repository_name.as_str(), suggestions[1].owner.slug.as_str()), (RepositoryFormat::Docker, "docker-repo", "public"));
        assert_eq!(capped.len(), 2);
    }

    #[sqlx::test]
    async fn suggestions_can_be_narrowed_to_a_format_and_an_owner_without_widening_visibility(pool: PgPool) {
        let alice = insert_personal_owner(&pool, "alice").await;
        let acme = insert_organization(&pool, "acme", "Acme", false).await;
        let npm = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let docker = public_hosted(&pool, PUBLIC_ORG, "docker-repo", "docker").await;
        let mine = public_hosted(&pool, alice, "alice-libs", "npm").await;
        let theirs = public_hosted(&pool, acme, "acme-libs", "npm").await;
        let closed = insert_repository(&pool, PUBLIC_ORG, "closed", "npm", "hosted", false).await;
        add_npm(&pool, npm, "pad-public", vec![version("1.0.0", ago(1))], None).await;
        add_docker(&pool, docker, "pad-image", &[("latest", ago(1))]).await;
        add_npm(&pool, mine, "pad-alice", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, theirs, "pad-acme", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, closed, "pad-private", vec![version("1.0.0", ago(1))], None).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());
        let names = |suggestions: Vec<CatalogSuggestion>| {
            let mut names: Vec<String> = suggestions.into_iter().map(|s| s.name).collect();
            names.sort();
            names
        };
        let ask = |format: Option<RepositoryFormat>, owner: Option<OwnerRef>, limit: u32| SuggestQuery { text: "pad".to_string(), limit, format, owner };

        let everything = names(catalog.suggest(&ask(None, None, 8)).await.unwrap());
        let docker_only = names(catalog.suggest(&ask(Some(RepositoryFormat::Docker), None, 8)).await.unwrap());
        let npm_only = names(catalog.suggest(&ask(Some(RepositoryFormat::Npm), None, 8)).await.unwrap());
        let alices = names(catalog.suggest(&ask(None, Some(OwnerRef { kind: OwnerKind::Personal, slug: "alice".to_string() }), 8)).await.unwrap());
        let acmes = names(catalog.suggest(&ask(Some(RepositoryFormat::Npm), Some(OwnerRef { kind: OwnerKind::Organization, slug: "acme".to_string() }), 8)).await.unwrap());
        let alice_docker = catalog.suggest(&ask(Some(RepositoryFormat::Docker), Some(OwnerRef { kind: OwnerKind::Personal, slug: "alice".to_string() }), 8)).await.unwrap();
        let limited = catalog.suggest(&ask(None, None, 2)).await.unwrap();

        assert_eq!(everything, vec!["pad-acme", "pad-alice", "pad-image", "pad-public"], "the private repository never shows");
        assert_eq!(docker_only, vec!["pad-image"]);
        assert_eq!(npm_only, vec!["pad-acme", "pad-alice", "pad-public"]);
        assert_eq!(alices, vec!["pad-alice"]);
        assert_eq!(acmes, vec!["pad-acme"]);
        assert!(alice_docker.is_empty());
        assert_eq!(limited.len(), 2);
    }

    #[sqlx::test]
    async fn two_typed_characters_suggest_prefixes_only(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "react", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, repo, "preact", vec![version("1.0.0", ago(1))], None).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());

        let two = catalog.suggest(&SuggestQuery { text: "re".to_string(), limit: 8, format: None, owner: None }).await.unwrap();
        let three = catalog.suggest(&SuggestQuery { text: "rea".to_string(), limit: 8, format: None, owner: None }).await.unwrap();

        assert_eq!(two.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["react"]);
        assert_eq!(three.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["react", "preact"]);
    }

    #[sqlx::test]
    async fn suggestions_tolerate_a_typo_and_never_leak_private_content(pool: PgPool) {
        let open = public_hosted(&pool, PUBLIC_ORG, "open", "npm").await;
        let closed = insert_repository(&pool, PUBLIC_ORG, "closed", "npm", "hosted", false).await;
        add_npm(&pool, open, "webpack", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, closed, "webpack-secret", vec![version("1.0.0", ago(1))], None).await;

        let suggestions = PostgresPublicCatalog::new(pool.clone()).suggest(&SuggestQuery { text: "wepback".to_string(), limit: 8, format: None, owner: None }).await.unwrap();

        assert_eq!(suggestions.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["webpack"]);
    }

    async fn add_downloads(pool: &PgPool, repository: Uuid, kind: &str, name: &str, days_ago: i64, downloads: i64) {
        sqlx::query("INSERT INTO download_stats (day, package_repository_id, kind, name, downloads) VALUES ((now() AT TIME ZONE 'utc')::date - $1::int, $2, $3, $4, $5) ON CONFLICT (day, package_repository_id, kind, name) DO UPDATE SET downloads = download_stats.downloads + EXCLUDED.downloads")
            .bind(days_ago as i32)
            .bind(repository)
            .bind(kind)
            .bind(name)
            .bind(downloads)
            .execute(pool)
            .await
            .unwrap();
    }

    fn popular() -> CatalogQuery {
        CatalogQuery { sort: CatalogSort::Popular, ..query(None) }
    }

    #[sqlx::test]
    async fn entries_carry_their_downloads_over_the_last_seven_days(pool: PgPool) {
        let npm = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        let docker = public_hosted(&pool, PUBLIC_ORG, "docker-repo", "docker").await;
        add_npm(&pool, npm, "widget", vec![version("1.0.0", ago(1))], None).await;
        add_docker(&pool, docker, "widget", &[("latest", ago(1))]).await;
        add_downloads(&pool, npm, "npm", "widget", 0, 5).await;
        add_downloads(&pool, npm, "npm", "widget", 6, 10).await;
        add_downloads(&pool, npm, "npm", "widget", 7, 1000).await;
        add_downloads(&pool, docker, "docker", "widget", 1, 3).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(None)).await.unwrap();

        let by_format: std::collections::HashMap<_, _> = page.items.iter().map(|e| (e.format, e.downloads_7d)).collect();
        assert_eq!(by_format, std::collections::HashMap::from([(RepositoryFormat::Npm, 15), (RepositoryFormat::Docker, 3)]), "an npm package and a docker image of the same name count apart, and day 7 is outside the week");
    }

    #[sqlx::test]
    async fn the_popular_sort_puts_the_most_downloaded_first_then_the_most_recent(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        for (name, published_days_ago, downloads) in [("quiet-new", 1, 0), ("quiet-old", 9, 0), ("busy", 30, 50), ("busier", 40, 90)] {
            add_npm(&pool, repo, name, vec![version("1.0.0", ago(published_days_ago))], None).await;
            if downloads > 0 {
                add_downloads(&pool, repo, "npm", name, 1, downloads).await;
            }
        }

        let page = PostgresPublicCatalog::new(pool.clone()).search(&popular()).await.unwrap();

        assert_eq!(page.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["busier", "busy", "quiet-new", "quiet-old"]);
    }

    #[sqlx::test]
    async fn the_popular_sort_also_applies_to_a_text_search(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        add_npm(&pool, repo, "left-pad", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, repo, "right-pad", vec![version("1.0.0", ago(2))], None).await;
        add_npm(&pool, repo, "unrelated", vec![version("1.0.0", ago(3))], None).await;
        add_downloads(&pool, repo, "npm", "right-pad", 0, 7).await;
        add_downloads(&pool, repo, "npm", "unrelated", 0, 99).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&CatalogQuery { text: Some("pad".into()), sort: CatalogSort::Popular, ..query(None) }).await.unwrap();

        assert_eq!(page.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["right-pad", "left-pad"]);
    }

    #[sqlx::test]
    async fn downloads_break_ties_inside_a_relevance_tier_but_never_outrank_a_better_tier(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "npm-repo", "npm").await;
        for (name, downloads) in [("pad", 1), ("padlock", 5), ("padding", 50), ("left-pad", 9000)] {
            add_npm(&pool, repo, name, vec![version("1.0.0", ago(1))], None).await;
            add_downloads(&pool, repo, "npm", name, 0, downloads).await;
        }

        let page = PostgresPublicCatalog::new(pool.clone()).search(&query(Some("pad"))).await.unwrap();

        assert_eq!(page.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["pad", "padding", "padlock", "left-pad"], "exact first, then the prefix tier by downloads, then the substring tier");
    }

    #[sqlx::test]
    async fn the_sitemap_lists_every_public_owner_repository_and_package_once(pool: PgPool) {
        let alice = insert_personal_owner(&pool, "alice").await;
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let lib = public_hosted(&pool, alice, "lib", "npm").await;
        let images = public_hosted(&pool, acme, "images", "docker").await;
        let closed = insert_repository(&pool, acme, "closed", "npm", "hosted", false).await;
        add_npm(&pool, lib, "pkg", vec![version("1.0.0", ago(5)), version("1.1.0", ago(2))], None).await;
        add_npm(&pool, closed, "secret", vec![version("1.0.0", ago(1))], None).await;
        add_docker(&pool, images, "api", &[("1.0", ago(9)), ("latest", ago(3))]).await;

        let entries = PostgresPublicCatalog::new(pool.clone()).sitemap_entries(1_000).await.unwrap();

        let summary: Vec<_> = entries.iter().map(|e| (e.owner.slug.as_str(), e.target.clone())).collect();
        assert_eq!(
            summary,
            vec![
                ("acme", SitemapTarget::Owner),
                ("acme", SitemapTarget::Repository { repository: "images".into() }),
                ("acme", SitemapTarget::Package { repository: "images".into(), format: RepositoryFormat::Docker, name: "api".into() }),
                ("alice", SitemapTarget::Owner),
                ("alice", SitemapTarget::Repository { repository: "lib".into() }),
                ("alice", SitemapTarget::Package { repository: "lib".into(), format: RepositoryFormat::Npm, name: "pkg".into() }),
            ],
            "the private repository and its package never appear"
        );
        let package_updated = |name: &str| entries.iter().find(|e| matches!(&e.target, SitemapTarget::Package { name: n, .. } if n == name)).unwrap().updated_at.date_naive();
        assert_eq!(package_updated("pkg"), ago(2).date_naive());
        assert_eq!(package_updated("api"), ago(3).date_naive());
    }

    #[sqlx::test]
    async fn the_sitemap_stops_at_the_limit_it_is_given(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "lib", "npm").await;
        for name in ["a", "b", "c", "d"] {
            add_npm(&pool, repo, name, vec![version("1.0.0", ago(1))], None).await;
        }

        let entries = PostgresPublicCatalog::new(pool.clone()).sitemap_entries(3).await.unwrap();

        assert_eq!(entries.len(), 3);
    }

    async fn set_page_controls(pool: &PgPool, organization: impl std::fmt::Display, public_page_enabled: bool, seo_indexing_blocked: bool) {
        sqlx::query("INSERT INTO system_settings (organization_id, max_login_attempts, login_attempt_window_seconds, session_ttl_hours, public_page_enabled, seo_indexing_blocked) VALUES ($1::uuid, 10, 300, 12, $2, $3) ON CONFLICT (organization_id) DO UPDATE SET public_page_enabled = $2, seo_indexing_blocked = $3")
            .bind(organization.to_string()).bind(public_page_enabled).bind(seo_indexing_blocked)
            .execute(pool).await.unwrap();
    }

    #[sqlx::test]
    async fn an_organization_that_closes_its_public_page_vanishes_from_every_public_view_and_the_others_stay(pool: PgPool) {
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let other = insert_organization(&pool, "other", "Other", false).await;
        let acme_lib = public_hosted(&pool, acme, "lib", "npm").await;
        let other_lib = public_hosted(&pool, other, "lib", "npm").await;
        add_npm(&pool, acme_lib, "acme-pkg", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, other_lib, "other-pkg", vec![version("1.0.0", ago(1))], None).await;
        set_page_controls(&pool, acme, false, false).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());
        let acme_owner = OwnerRef { kind: OwnerKind::Organization, slug: "acme".into() };

        assert_eq!(names(&pool, &query(None)).await, vec!["other-pkg"]);
        assert_eq!(names(&pool, &query(Some("acme"))).await, Vec::<String>::new());
        assert!(catalog.owner_summary(&acme_owner).await.unwrap().is_none());
        assert!(catalog.repository(&acme_owner, "lib").await.unwrap().is_none());
        assert!(catalog.find_entry(&acme_owner, "lib", RepositoryFormat::Npm, "acme-pkg").await.unwrap().is_none());
        assert_eq!(catalog.entry_counts().await.unwrap().iter().map(|c| c.entry_count).sum::<i64>(), 1);
        assert!(catalog.sitemap_entries(100).await.unwrap().iter().all(|e| e.owner.slug != "acme"));

        set_page_controls(&pool, acme, true, false).await;
        assert!(catalog.owner_summary(&acme_owner).await.unwrap().is_some(), "open again");
    }

    #[sqlx::test]
    async fn closing_the_public_organizations_page_closes_the_whole_catalog(pool: PgPool) {
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let lib = public_hosted(&pool, acme, "lib", "npm").await;
        add_npm(&pool, lib, "pkg", vec![version("1.0.0", ago(1))], None).await;
        assert_eq!(names(&pool, &query(None)).await, vec!["pkg"]);

        set_page_controls(&pool, PUBLIC_ORG, false, false).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());

        assert!(names(&pool, &query(None)).await.is_empty());
        assert!(catalog.entry_counts().await.unwrap().is_empty());
        assert!(catalog.sitemap_entries(100).await.unwrap().is_empty());
        assert!(catalog.owner_summary(&OwnerRef { kind: OwnerKind::Organization, slug: "acme".into() }).await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn an_organization_that_blocks_search_engines_stays_visible_but_out_of_the_sitemap(pool: PgPool) {
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let other = insert_organization(&pool, "other", "Other", false).await;
        let acme_lib = public_hosted(&pool, acme, "lib", "npm").await;
        let other_lib = public_hosted(&pool, other, "lib", "npm").await;
        add_npm(&pool, acme_lib, "acme-pkg", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, other_lib, "other-pkg", vec![version("1.0.0", ago(1))], None).await;
        set_page_controls(&pool, acme, true, true).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());

        let summary = |slug: &str| OwnerRef { kind: OwnerKind::Organization, slug: slug.into() };
        assert!(catalog.owner_summary(&summary("acme")).await.unwrap().unwrap().indexing_blocked);
        assert!(!catalog.owner_summary(&summary("other")).await.unwrap().unwrap().indexing_blocked);
        assert_eq!(names(&pool, &query(None)).await.len(), 2, "still listed for visitors");
        let sitemap = catalog.sitemap_entries(100).await.unwrap();
        assert!(sitemap.iter().all(|e| e.owner.slug != "acme") && sitemap.iter().any(|e| e.owner.slug == "other"), "{sitemap:?}");
    }

    #[sqlx::test]
    async fn an_entry_is_found_by_its_exact_owner_repository_format_and_name(pool: PgPool) {
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let other = insert_organization(&pool, "other", "Other", false).await;
        let libs = public_hosted(&pool, acme, "libs", "npm").await;
        let more = public_hosted(&pool, acme, "more", "npm").await;
        let images = public_hosted(&pool, acme, "images", "docker").await;
        let closed = insert_repository(&pool, acme, "closed", "npm", "hosted", false).await;
        let elsewhere = public_hosted(&pool, other, "libs", "npm").await;
        let manifest = serde_json::json!({ "name": "left-pad", "description": "Pads strings" });
        add_npm(&pool, libs, "left-pad", vec![Version { version: "1.0.0", published: ago(3), manifest }, version("1.1.0", ago(1))], Some("1.0.0")).await;
        add_npm(&pool, libs, "left-pad-extra", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, more, "left-pad", vec![version("9.0.0", ago(1))], None).await;
        add_npm(&pool, closed, "secret", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, elsewhere, "left-pad", vec![version("2.0.0", ago(1))], None).await;
        add_docker(&pool, images, "api", &[("1.0", ago(9)), ("latest", ago(3))]).await;
        add_downloads(&pool, libs, "npm", "left-pad", 0, 12).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());
        let acme_ref = OwnerRef { kind: OwnerKind::Organization, slug: "acme".to_string() };

        let found = catalog.find_entry(&acme_ref, "libs", RepositoryFormat::Npm, "left-pad").await.unwrap().unwrap();

        assert_eq!((found.name.as_str(), found.repository_name.as_str(), found.owner.slug.as_str()), ("left-pad", "libs", "acme"));
        assert_eq!((found.description.as_deref(), found.latest.as_deref(), found.downloads_7d), (Some("Pads strings"), Some("1.0.0"), 12));
        let image = catalog.find_entry(&acme_ref, "images", RepositoryFormat::Docker, "api").await.unwrap().unwrap();
        assert_eq!((image.name.as_str(), image.latest.as_deref()), ("api", Some("latest")));
        for (owner, repository, format, name) in [
            (&acme_ref, "libs", RepositoryFormat::Npm, "left-pa"),
            (&acme_ref, "libs", RepositoryFormat::Npm, "LEFT-PAD"),
            (&acme_ref, "libs", RepositoryFormat::Docker, "left-pad"),
            (&acme_ref, "images", RepositoryFormat::Npm, "api"),
            (&acme_ref, "closed", RepositoryFormat::Npm, "secret"),
            (&acme_ref, "nowhere", RepositoryFormat::Npm, "left-pad"),
            (&OwnerRef { kind: OwnerKind::Personal, slug: "acme".to_string() }, "libs", RepositoryFormat::Npm, "left-pad"),
        ] {
            assert!(catalog.find_entry(owner, repository, format, name).await.unwrap().is_none(), "{repository} {name}");
        }
        assert_eq!(catalog.find_entry(&OwnerRef { kind: OwnerKind::Organization, slug: "other".to_string() }, "libs", RepositoryFormat::Npm, "left-pad").await.unwrap().unwrap().latest.as_deref(), Some("2.0.0"));
    }

    #[sqlx::test]
    async fn a_nul_byte_in_a_name_is_refused_by_the_database_so_callers_must_keep_it_out(pool: PgPool) {
        let repo = public_hosted(&pool, PUBLIC_ORG, "lib", "npm").await;
        add_npm(&pool, repo, "pkg", vec![version("1.0.0", ago(1))], None).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());
        let owner = OwnerRef { kind: OwnerKind::Organization, slug: "public".to_string() };

        assert!(catalog.find_entry(&owner, "lib", RepositoryFormat::Npm, "pkg\u{0}").await.is_err());
    }

    #[sqlx::test]
    async fn public_queries_run_under_a_statement_timeout_that_does_not_outlive_them(pool: PgPool) {
        let catalog = PostgresPublicCatalog::new(pool.clone());

        let (mut tx, _slot) = catalog.begin().await.unwrap();
        let (during,): (String,) = sqlx::query_as("SHOW statement_timeout").fetch_one(&mut *tx).await.unwrap();
        drop(tx);
        let (after,): (String,) = sqlx::query_as("SHOW statement_timeout").fetch_one(&pool).await.unwrap();

        assert_eq!((during.as_str(), after.as_str()), ("3s", "0"));
    }

    #[sqlx::test]
    async fn a_slow_public_query_is_cancelled_by_the_timeout(pool: PgPool) {
        let catalog = PostgresPublicCatalog::new(pool.clone());
        let (mut tx, _slot) = catalog.begin().await.unwrap();
        sqlx::query("SET LOCAL statement_timeout = 50").execute(&mut *tx).await.unwrap();

        let outcome = sqlx::query("SELECT pg_sleep(5)").execute(&mut *tx).await;

        let error = outcome.unwrap_err();
        assert!(error.to_string().contains("statement timeout"));
        assert!(matches!(query_err(error), DomainError::Busy(_)), "a timeout is load, not a fault");
    }

    #[sqlx::test]
    async fn any_other_database_error_stays_an_infrastructure_failure(pool: PgPool) {
        let error = sqlx::query("SELECT no_such_column FROM no_such_table").execute(&pool).await.unwrap_err();

        assert!(matches!(query_err(error), DomainError::Infrastructure(_)));
    }

    #[sqlx::test]
    async fn only_a_few_public_queries_run_at_once_and_the_next_one_gives_up_waiting(pool: PgPool) {
        let catalog = PostgresPublicCatalog { slot_wait: std::time::Duration::from_millis(50), ..PostgresPublicCatalog::new(pool.clone()) };
        let mut held = Vec::new();
        for _ in 0..CONCURRENT_QUERIES {
            held.push(catalog.begin().await.unwrap());
        }

        let busy = catalog.entry_counts().await;

        assert!(matches!(busy, Err(DomainError::Busy(message)) if message.contains("busy")));
        drop(held);
        assert!(catalog.entry_counts().await.is_ok(), "the slots come back once the queries end");
    }

    #[sqlx::test]
    async fn a_scoped_search_is_one_prepared_statement_whatever_the_ids(pool: PgPool) {
        let one_connection = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect_with((*pool.connect_options()).clone()).await.unwrap();
        let catalog = PostgresPublicCatalog::new(one_connection.clone());
        let repo = public_hosted(&pool, PUBLIC_ORG, "lib", "npm").await;
        add_npm(&pool, repo, "pkg", vec![version("1.0.0", ago(1))], None).await;

        for ids in [vec![], vec![Uuid::new_v4()], vec![Uuid::new_v4(), Uuid::new_v4()], vec![repo]] {
            catalog.search(&scoped(CatalogScope::Repositories(ids))).await.unwrap();
        }

        let (statements,): (i64,) = sqlx::query_as("SELECT count(*) FROM pg_prepared_statements WHERE statement LIKE '%scope_ids%' AND statement NOT LIKE 'SELECT count%'").fetch_one(&one_connection).await.unwrap();
        assert_eq!(statements, 1);
    }

    #[sqlx::test]
    async fn a_repository_is_found_only_when_it_is_public_hosted_and_the_owner_matches(pool: PgPool) {
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        public_hosted(&pool, acme, "libs", "npm").await;
        insert_repository(&pool, acme, "closed", "npm", "hosted", false).await;
        insert_repository(&pool, acme, "mirror", "npm", "proxy", true).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());
        let acme_ref = OwnerRef { kind: OwnerKind::Organization, slug: "acme".to_string() };

        assert_eq!(catalog.repository(&acme_ref, "libs").await.unwrap(), Some(CatalogRepository { name: "libs".to_string(), format: RepositoryFormat::Npm }));
        for repository in ["closed", "mirror", "missing"] {
            assert_eq!(catalog.repository(&acme_ref, repository).await.unwrap(), None, "{repository}");
        }
        assert_eq!(catalog.repository(&OwnerRef { kind: OwnerKind::Personal, slug: "acme".to_string() }, "libs").await.unwrap(), None);
    }

    fn scoped(scope: CatalogScope) -> CatalogQuery {
        CatalogQuery { scope, ..query(None) }
    }

    #[sqlx::test]
    async fn a_scope_adds_the_listed_private_and_proxy_repositories_to_the_public_ones(pool: PgPool) {
        let public = public_hosted(&pool, PUBLIC_ORG, "open", "npm").await;
        let mine = insert_repository(&pool, PUBLIC_ORG, "mine", "npm", "hosted", false).await;
        let mirror = insert_repository(&pool, PUBLIC_ORG, "mirror", "npm", "proxy", false).await;
        let someone_elses = insert_repository(&pool, PUBLIC_ORG, "theirs", "npm", "hosted", false).await;
        let group = insert_repository(&pool, PUBLIC_ORG, "grouped", "npm", "group", false).await;
        for (repo, name) in [(public, "from-public"), (mine, "from-mine"), (mirror, "from-cache"), (someone_elses, "from-theirs"), (group, "from-group")] {
            add_npm(&pool, repo, name, vec![version("1.0.0", ago(1))], None).await;
        }
        let catalog = PostgresPublicCatalog::new(pool.clone());

        let readable = catalog.search(&scoped(CatalogScope::Repositories(vec![mine, mirror, group]))).await.unwrap();
        let anonymous = catalog.search(&scoped(CatalogScope::PublicOnly)).await.unwrap();

        let mut names: Vec<_> = readable.items.iter().map(|e| e.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["from-cache", "from-mine", "from-public"], "the caller's private and proxy repositories join the public one; someone else's private one and a group stay out");
        assert_eq!(anonymous.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["from-public"]);
        let types: std::collections::HashMap<_, _> = readable.items.iter().map(|e| (e.name.as_str(), (e.repository_id, e.repository_type))).collect();
        assert_eq!(types["from-cache"], (mirror, RepositoryType::Proxy));
        assert_eq!(types["from-mine"], (mine, RepositoryType::Hosted));
    }

    #[sqlx::test]
    async fn an_empty_scope_is_just_the_public_content(pool: PgPool) {
        let public = public_hosted(&pool, PUBLIC_ORG, "open", "npm").await;
        let private = insert_repository(&pool, PUBLIC_ORG, "closed", "npm", "hosted", false).await;
        add_npm(&pool, public, "visible", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, private, "hidden", vec![version("1.0.0", ago(1))], None).await;

        let page = PostgresPublicCatalog::new(pool.clone()).search(&scoped(CatalogScope::Repositories(vec![]))).await.unwrap();

        assert_eq!(page.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["visible"]);
    }

    #[sqlx::test]
    async fn the_all_repositories_scope_covers_hosted_and_proxy_but_never_groups_or_deleted_ones(pool: PgPool) {
        let hosted = insert_repository(&pool, PUBLIC_ORG, "h", "npm", "hosted", false).await;
        let proxy = insert_repository(&pool, PUBLIC_ORG, "p", "npm", "proxy", false).await;
        let group = insert_repository(&pool, PUBLIC_ORG, "g", "npm", "group", false).await;
        let deleted = insert_repository(&pool, PUBLIC_ORG, "d", "npm", "hosted", false).await;
        sqlx::query("UPDATE package_repository_projections SET deleted_at = now() WHERE id = $1").bind(deleted).execute(&pool).await.unwrap();
        for (repo, name) in [(hosted, "a-hosted"), (proxy, "a-proxy"), (group, "a-group"), (deleted, "a-deleted")] {
            add_npm(&pool, repo, name, vec![version("1.0.0", ago(1))], None).await;
        }

        let page = PostgresPublicCatalog::new(pool.clone()).search(&scoped(CatalogScope::AllRepositories)).await.unwrap();

        let mut names: Vec<_> = page.items.iter().map(|e| e.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["a-hosted", "a-proxy"]);
    }

    #[sqlx::test]
    async fn a_scoped_search_ranks_and_filters_like_the_public_one(pool: PgPool) {
        let mine = insert_repository(&pool, PUBLIC_ORG, "mine", "npm", "hosted", false).await;
        for name in ["left-pad", "padlock", "unrelated"] {
            add_npm(&pool, mine, name, vec![version("1.0.0", ago(1))], None).await;
        }

        let page = PostgresPublicCatalog::new(pool.clone()).search(&CatalogQuery { scope: CatalogScope::Repositories(vec![mine]), text: Some("pad".into()), sort: CatalogSort::Relevance, ..query(None) }).await.unwrap();

        assert_eq!(page.items.iter().map(|e| (e.name.as_str(), e.match_kind)).collect::<Vec<_>>(), vec![("padlock", Some(CatalogMatch::Prefix)), ("left-pad", Some(CatalogMatch::Contains))]);
    }

    fn owned_by(kind: OwnerKind, slug: &str) -> CatalogQuery {
        CatalogQuery { owner: Some(OwnerRef { kind, slug: slug.to_string() }), ..query(None) }
    }

    #[sqlx::test]
    async fn the_owner_filter_restricts_results_to_that_owner(pool: PgPool) {
        let alice = insert_personal_owner(&pool, "alice").await;
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let alice_repo = public_hosted(&pool, alice, "alice-lib", "npm").await;
        let acme_repo = public_hosted(&pool, acme, "acme-lib", "npm").await;
        let acme_docker = public_hosted(&pool, acme, "acme-images", "docker").await;
        add_npm(&pool, alice_repo, "alices-package", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, acme_repo, "acmes-package", vec![version("1.0.0", ago(2))], None).await;
        add_docker(&pool, acme_docker, "acmes-image", &[("latest", ago(3))]).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());

        let alices = catalog.search(&owned_by(OwnerKind::Personal, "alice")).await.unwrap();
        let acmes = catalog.search(&owned_by(OwnerKind::Organization, "acme")).await.unwrap();

        assert_eq!(alices.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["alices-package"]);
        assert_eq!(acmes.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["acmes-package", "acmes-image"]);
        assert_eq!(acmes.total, 2);
        assert!(catalog.search(&owned_by(OwnerKind::Organization, "alice")).await.unwrap().items.is_empty(), "the kind is part of the match: a personal username is not an organization slug");
    }

    #[sqlx::test]
    async fn the_owner_filter_combines_with_text_and_format(pool: PgPool) {
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let npm = public_hosted(&pool, acme, "acme-npm", "npm").await;
        let docker = public_hosted(&pool, acme, "acme-docker", "docker").await;
        let other = public_hosted(&pool, PUBLIC_ORG, "other", "npm").await;
        add_npm(&pool, npm, "widget-kit", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, other, "widget-other", vec![version("1.0.0", ago(1))], None).await;
        add_docker(&pool, docker, "widget-image", &[("latest", ago(1))]).await;
        let acme_widgets = CatalogQuery { text: Some("widget".to_string()), sort: CatalogSort::Relevance, ..owned_by(OwnerKind::Organization, "acme") };

        let all = PostgresPublicCatalog::new(pool.clone()).search(&acme_widgets).await.unwrap();
        let npm_only = PostgresPublicCatalog::new(pool.clone()).search(&CatalogQuery { format: Some(RepositoryFormat::Npm), ..acme_widgets }).await.unwrap();

        assert_eq!(all.total, 2);
        assert_eq!(npm_only.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["widget-kit"]);
    }

    #[sqlx::test]
    async fn an_owner_summary_counts_public_repositories_packages_and_images(pool: PgPool) {
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        let npm = public_hosted(&pool, acme, "libs", "npm").await;
        let docker = public_hosted(&pool, acme, "images", "docker").await;
        let private = insert_repository(&pool, acme, "closed", "npm", "hosted", false).await;
        add_npm(&pool, npm, "one", vec![version("1.0.0", ago(1)), version("1.1.0", ago(1))], None).await;
        add_npm(&pool, npm, "two", vec![version("1.0.0", ago(1))], None).await;
        add_npm(&pool, npm, "emptied", vec![], None).await;
        add_npm(&pool, private, "secret", vec![version("1.0.0", ago(1))], None).await;
        add_docker(&pool, docker, "api", &[("1.0", ago(2)), ("latest", ago(1))]).await;

        let summary = PostgresPublicCatalog::new(pool.clone()).owner_summary(&OwnerRef { kind: OwnerKind::Organization, slug: "acme".to_string() }).await.unwrap().unwrap();

        assert_eq!(
            summary,
            OwnerSummary { kind: OwnerKind::Organization, slug: "acme".to_string(), display_name: "Acme Corp".to_string(), repository_count: 2, package_count: 2, image_count: 1, indexing_blocked: false }
        );
    }

    #[sqlx::test]
    async fn a_personal_owner_summary_uses_the_username(pool: PgPool) {
        let alice = insert_personal_owner(&pool, "alice").await;
        let repo = public_hosted(&pool, alice, "lib", "npm").await;
        add_npm(&pool, repo, "pkg", vec![version("1.0.0", ago(1))], None).await;

        let summary = PostgresPublicCatalog::new(pool.clone()).owner_summary(&OwnerRef { kind: OwnerKind::Personal, slug: "alice".to_string() }).await.unwrap().unwrap();

        assert_eq!((summary.display_name.as_str(), summary.repository_count, summary.package_count), ("alice", 1, 1));
    }

    #[sqlx::test]
    async fn an_owner_with_no_public_repository_has_no_summary(pool: PgPool) {
        let alice = insert_personal_owner(&pool, "alice").await;
        let acme = insert_organization(&pool, "acme", "Acme Corp", false).await;
        insert_repository(&pool, alice, "hidden", "npm", "hosted", false).await;
        insert_repository(&pool, acme, "mirror", "npm", "proxy", true).await;
        let catalog = PostgresPublicCatalog::new(pool.clone());

        for owner in [
            OwnerRef { kind: OwnerKind::Personal, slug: "alice".to_string() },
            OwnerRef { kind: OwnerKind::Organization, slug: "acme".to_string() },
            OwnerRef { kind: OwnerKind::Personal, slug: "nobody".to_string() },
        ] {
            assert_eq!(catalog.owner_summary(&owner).await.unwrap(), None, "{owner:?}");
        }
    }

    /// The SQL join re-derives the personal slug; this pins it to the Rust function it mirrors.
    #[sqlx::test]
    async fn the_sql_personal_slug_derivation_matches_the_application_one(pool: PgPool) {
        let id = Uuid::new_v4();
        let (sql_slug,): (String,) = sqlx::query_as("SELECT 'u' || left(replace($1::text, '-', ''), 24)").bind(id).fetch_one(&pool).await.unwrap();

        assert_eq!(sql_slug, personal_organization_slug(id).as_str());
    }

    #[sqlx::test]
    async fn the_sitemap_has_a_slot_and_a_time_limit_of_its_own(pool: PgPool) {
        let catalog = PostgresPublicCatalog { slot_wait: std::time::Duration::from_millis(50), ..PostgresPublicCatalog::new(pool.clone()) };
        let mut held = Vec::new();
        for _ in 0..CONCURRENT_QUERIES {
            held.push(catalog.begin().await.unwrap());
        }

        assert!(catalog.sitemap_entries(10).await.is_ok(), "user-facing queries filling their slots do not starve the sitemap");
        let (mut tx, sitemap_slot) = catalog.begin_in(&catalog.sitemap_slots, SITEMAP_STATEMENT_TIMEOUT_MS).await.unwrap();
        let (timeout,): (String,) = sqlx::query_as("SHOW statement_timeout").fetch_one(&mut *tx).await.unwrap();
        assert_eq!(timeout, "15s");
        assert!(matches!(catalog.sitemap_entries(10).await, Err(DomainError::Busy(_))), "one sitemap build at a time");
        drop((held, sitemap_slot, tx));
        assert!(catalog.entry_counts().await.is_ok());
    }
}
