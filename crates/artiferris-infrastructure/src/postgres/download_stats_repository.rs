use async_trait::async_trait;
use artiferris_domain::download_stats::{DownloadCount, DownloadStatsPort, RECENT_WINDOW_DAYS};
use artiferris_domain::error::DomainError;
use artiferris_domain::package_repository::RepositoryFormat;
use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error_ext::InfraErr;

pub struct PostgresDownloadStats {
    pool: PgPool,
}

impl PostgresDownloadStats {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn kind(format: RepositoryFormat) -> &'static str {
    match format {
        RepositoryFormat::Npm => "npm",
        RepositoryFormat::Docker => "docker",
    }
}

#[async_trait]
impl DownloadStatsPort for PostgresDownloadStats {
    async fn add_batch(&self, counts: &[DownloadCount]) -> Result<(), DomainError> {
        if counts.is_empty() {
            return Ok(());
        }
        // One statement cannot touch the same row twice, so a repeated key is summed first.
        let mut merged: std::collections::HashMap<(NaiveDate, Uuid, RepositoryFormat, &str), i64> = std::collections::HashMap::new();
        for c in counts {
            *merged.entry((c.day, c.repository_id, c.format, c.name.as_str())).or_insert(0) += c.downloads;
        }
        let counts: Vec<DownloadCount> = merged.into_iter().map(|((day, repository_id, format, name), downloads)| DownloadCount { day, repository_id, format, name: name.to_string(), downloads }).collect();
        let days: Vec<NaiveDate> = counts.iter().map(|c| c.day).collect();
        let repositories: Vec<Uuid> = counts.iter().map(|c| c.repository_id).collect();
        let kinds: Vec<&str> = counts.iter().map(|c| kind(c.format)).collect();
        let names: Vec<&str> = counts.iter().map(|c| c.name.as_str()).collect();
        let downloads: Vec<i64> = counts.iter().map(|c| c.downloads).collect();
        // Additive, so several instances (or a retry after a lost acknowledgement) can flush into the same rows. A repository
        // deleted since the download was counted simply has no row to attach to.
        sqlx::query(
            "INSERT INTO download_stats (day, package_repository_id, kind, name, downloads)
             SELECT c.day, c.package_repository_id, c.kind, c.name, c.downloads
             FROM unnest($1::date[], $2::uuid[], $3::text[], $4::text[], $5::bigint[]) AS c(day, package_repository_id, kind, name, downloads)
             JOIN package_repository_projections p ON p.id = c.package_repository_id
             ON CONFLICT (day, package_repository_id, kind, name) DO UPDATE SET downloads = download_stats.downloads + EXCLUDED.downloads",
        )
        .bind(days)
        .bind(repositories)
        .bind(kinds)
        .bind(names)
        .bind(downloads)
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn downloads_last_7_days(&self, repository_id: Uuid, format: RepositoryFormat, name: &str) -> Result<i64, DomainError> {
        let (total,): (i64,) = sqlx::query_as(
            "SELECT coalesce(sum(downloads), 0)::bigint FROM download_stats
             WHERE package_repository_id = $1 AND kind = $2 AND name = $3 AND day > (now() AT TIME ZONE 'utc')::date - $4::int",
        )
        .bind(repository_id)
        .bind(kind(format))
        .bind(name)
        .bind(RECENT_WINDOW_DAYS as i32)
        .fetch_one(&self.pool)
        .await
        .infra_err()?;
        Ok(total)
    }

    async fn prune_before(&self, day: NaiveDate) -> Result<u64, DomainError> {
        Ok(sqlx::query("DELETE FROM download_stats WHERE day < $1").bind(day).execute(&self.pool).await.infra_err()?.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;

    const PUBLIC_ORG: &str = "00000000-0000-0000-0000-000000000001";

    async fn repository(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, version) VALUES ($1, $2::uuid, $3, 'npm', 'hosted', 1)")
            .bind(id)
            .bind(PUBLIC_ORG)
            .bind(format!("repo-{id}"))
            .execute(pool)
            .await
            .unwrap();
        id
    }

    fn count(repository_id: Uuid, name: &str, days_ago: i64, downloads: i64) -> DownloadCount {
        DownloadCount { day: Utc::now().date_naive() - Duration::days(days_ago), repository_id, format: RepositoryFormat::Npm, name: name.to_string(), downloads }
    }

    #[sqlx::test]
    async fn counts_are_added_to_what_is_already_stored(pool: PgPool) {
        let repo = repository(&pool).await;
        let stats = PostgresDownloadStats::new(pool.clone());

        stats.add_batch(&[count(repo, "left-pad", 0, 3)]).await.unwrap();
        stats.add_batch(&[count(repo, "left-pad", 0, 4), count(repo, "right-pad", 0, 1)]).await.unwrap();

        assert_eq!(stats.downloads_last_7_days(repo, RepositoryFormat::Npm, "left-pad").await.unwrap(), 7);
        assert_eq!(stats.downloads_last_7_days(repo, RepositoryFormat::Npm, "right-pad").await.unwrap(), 1);
    }

    #[sqlx::test]
    async fn a_batch_can_repeat_a_key_within_itself(pool: PgPool) {
        let repo = repository(&pool).await;
        let stats = PostgresDownloadStats::new(pool.clone());

        stats.add_batch(&[count(repo, "a", 0, 1), count(repo, "a", 0, 1)]).await.unwrap();

        assert_eq!(stats.downloads_last_7_days(repo, RepositoryFormat::Npm, "a").await.unwrap(), 2);
    }

    #[sqlx::test]
    async fn the_window_is_today_and_the_six_days_before(pool: PgPool) {
        let repo = repository(&pool).await;
        let stats = PostgresDownloadStats::new(pool.clone());
        stats.add_batch(&[count(repo, "pkg", 0, 1), count(repo, "pkg", 6, 10), count(repo, "pkg", 7, 100), count(repo, "pkg", 30, 1000)]).await.unwrap();

        assert_eq!(stats.downloads_last_7_days(repo, RepositoryFormat::Npm, "pkg").await.unwrap(), 11);
    }

    #[sqlx::test]
    async fn npm_and_docker_names_and_repositories_are_counted_apart(pool: PgPool) {
        let one = repository(&pool).await;
        let two = repository(&pool).await;
        let stats = PostgresDownloadStats::new(pool.clone());
        let docker = DownloadCount { format: RepositoryFormat::Docker, ..count(one, "shared", 0, 5) };
        stats.add_batch(&[count(one, "shared", 0, 1), docker, count(two, "shared", 0, 20)]).await.unwrap();

        assert_eq!(stats.downloads_last_7_days(one, RepositoryFormat::Npm, "shared").await.unwrap(), 1);
        assert_eq!(stats.downloads_last_7_days(one, RepositoryFormat::Docker, "shared").await.unwrap(), 5);
        assert_eq!(stats.downloads_last_7_days(two, RepositoryFormat::Npm, "shared").await.unwrap(), 20);
        assert_eq!(stats.downloads_last_7_days(one, RepositoryFormat::Npm, "unknown").await.unwrap(), 0);
    }

    #[sqlx::test]
    async fn a_download_of_a_repository_that_no_longer_exists_is_dropped_quietly(pool: PgPool) {
        let stats = PostgresDownloadStats::new(pool.clone());

        stats.add_batch(&[count(Uuid::new_v4(), "ghost", 0, 1)]).await.unwrap();

        let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM download_stats").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
    }

    #[sqlx::test]
    async fn pruning_removes_only_days_before_the_cutoff(pool: PgPool) {
        let repo = repository(&pool).await;
        let stats = PostgresDownloadStats::new(pool.clone());
        stats.add_batch(&[count(repo, "pkg", 0, 1), count(repo, "pkg", 400, 1), count(repo, "pkg", 500, 1)]).await.unwrap();

        let removed = stats.prune_before(Utc::now().date_naive() - Duration::days(396)).await.unwrap();

        assert_eq!(removed, 2);
        assert_eq!(stats.downloads_last_7_days(repo, RepositoryFormat::Npm, "pkg").await.unwrap(), 1);
    }

    #[sqlx::test]
    async fn deleting_a_repository_takes_its_counts_with_it(pool: PgPool) {
        let repo = repository(&pool).await;
        let stats = PostgresDownloadStats::new(pool.clone());
        stats.add_batch(&[count(repo, "pkg", 0, 1)]).await.unwrap();

        sqlx::query("DELETE FROM package_repository_projections WHERE id = $1").bind(repo).execute(&pool).await.unwrap();

        let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM download_stats").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
    }
}
