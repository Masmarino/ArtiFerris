use artiferris_domain::error::DomainError;
use artiferris_domain::reserved_names::RESERVED_NAME_PREFIX;
use sqlx::PgPool;

use crate::error_ext::InfraErr;

/// Names created before the `artiferris-` reservation. They keep working, but the operator should rename them, so
/// startup lists them.
pub async fn find_reserved_name_conflicts(pool: &PgPool) -> Result<Vec<String>, DomainError> {
    let pattern = format!("{RESERVED_NAME_PREFIX}%");
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT 'user ' || username FROM users WHERE username ILIKE $1
         UNION ALL
         SELECT 'organization ' || slug FROM organizations WHERE slug ILIKE $1 AND NOT is_personal
         UNION ALL
         SELECT 'repository ' || name FROM package_repository_projections WHERE name ILIKE $1 AND deleted_at IS NULL
         ORDER BY 1",
    )
    .bind(pattern)
    .fetch_all(pool)
    .await
    .infra_err()?;
    Ok(rows.into_iter().map(|(name,)| name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn lists_existing_users_organizations_and_repositories_using_the_prefix(pool: PgPool) {
        sqlx::raw_sql(
            "INSERT INTO users (id, username, password_hash, organization_id) VALUES (gen_random_uuid(), 'artiferris-old', 'x', '00000000-0000-0000-0000-000000000001'), (gen_random_uuid(), 'fine', 'x', '00000000-0000-0000-0000-000000000001');
             INSERT INTO organizations (id, slug, display_name) VALUES (gen_random_uuid(), 'ArtiFerris-inc', 'Inc'), (gen_random_uuid(), 'acme', 'Acme');
             INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, version) VALUES
               (gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'artiferris-npm', 'npm', 'hosted', 1),
               (gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'my-artiferris-npm', 'npm', 'hosted', 1);",
        )
        .execute(&pool)
        .await
        .unwrap();

        let conflicts = find_reserved_name_conflicts(&pool).await.unwrap();

        assert_eq!(conflicts, vec!["organization ArtiFerris-inc", "repository artiferris-npm", "user artiferris-old"]);
    }

    #[sqlx::test]
    async fn a_clean_database_reports_nothing(pool: PgPool) {
        assert!(find_reserved_name_conflicts(&pool).await.unwrap().is_empty());
    }
}
