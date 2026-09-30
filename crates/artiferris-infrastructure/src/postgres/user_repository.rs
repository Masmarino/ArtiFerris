use async_trait::async_trait;
use artiferris_domain::audit::{AdminAuditRecord, AuditRecord, SecurityAuditRecord};
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use artiferris_domain::user::{User, UserRepositoryPort, UserSecurityPort, Username};
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresUserRepository {
    pool: PgPool,
}

impl PostgresUserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

struct UserRow {
    id: Uuid,
    username: String,
    password_hash: String,
    is_super_admin: bool,
    is_organization_admin: bool,
    organization_id: Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
    tokens_valid_after: chrono::DateTime<chrono::Utc>,
    email: Option<String>,
}

/// Escapes ILIKE's own wildcard metacharacters — `%`, `_`, and the default `\` escape character
/// itself — in user-supplied search text before it's wrapped in a `%...%` pattern. Without this, a
/// caller's own `%`/`_` would be interpreted as ILIKE wildcards instead of literal characters
/// (e.g. searching for a literal underscore would instead match every username).
fn escape_ilike_wildcards(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

impl UserRow {
    fn into_domain(self) -> Result<User, DomainError> {
        Ok(User {
            id: self.id,
            username: Username::parse(&self.username)?,
            password_hash: self.password_hash,
            is_super_admin: self.is_super_admin,
            is_organization_admin: self.is_organization_admin,
            organization_id: self.organization_id,
            created_at: self.created_at,
            tokens_valid_after: self.tokens_valid_after,
            email: self.email,
        })
    }
}

#[async_trait]
impl UserRepositoryPort for PostgresUserRepository {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, DomainError> {
        let row = sqlx::query_as!(
            UserRow,
            "SELECT id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email FROM users WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(UserRow::into_domain).transpose()
    }

    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, DomainError> {
        let row = sqlx::query_as!(
            UserRow,
            "SELECT id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email FROM users WHERE username = $1",
            username.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(UserRow::into_domain).transpose()
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<User>, DomainError> {
        let row = sqlx::query_as!(
            UserRow,
            "SELECT id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email FROM users WHERE email = $1",
            email,
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(UserRow::into_domain).transpose()
    }

    async fn list_all(&self) -> Result<Vec<User>, DomainError> {
        let rows = sqlx::query_as!(
            UserRow,
            "SELECT id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email FROM users ORDER BY created_at"
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(UserRow::into_domain).collect()
    }

    async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<User>, DomainError> {
        let rows = sqlx::query_as!(
            UserRow,
            "SELECT id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email \
             FROM users WHERE id = ANY($1)",
            ids
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(UserRow::into_domain).collect()
    }

    async fn count_by_organization(&self, organization_id: Uuid) -> Result<i64, DomainError> {
        let count: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM users WHERE organization_id = $1", organization_id)
            .fetch_one(&self.pool)
            .await
            .infra_err()?
            .unwrap_or(0);
        Ok(count)
    }

    async fn search_by_organization(&self, organization_id: Uuid, query: &str, limit: i64) -> Result<Vec<User>, DomainError> {
        let pattern = format!("%{}%", escape_ilike_wildcards(query));
        let rows = sqlx::query_as!(
            UserRow,
            "SELECT id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email \
             FROM users WHERE organization_id = $1 AND username ILIKE $2 ORDER BY username LIMIT $3",
            organization_id,
            pattern,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(UserRow::into_domain).collect()
    }

    async fn search_all_organizations(&self, query: &str, limit: i64) -> Result<Vec<User>, DomainError> {
        let pattern = format!("%{}%", escape_ilike_wildcards(query));
        let rows = sqlx::query_as!(
            UserRow,
            "SELECT id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email \
             FROM users WHERE username ILIKE $1 ORDER BY username LIMIT $2",
            pattern,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        rows.into_iter().map(UserRow::into_domain).collect()
    }

    async fn insert(&self, user: &User) -> Result<(), DomainError> {
        self.insert_row(user, false).await
    }

    async fn delete(&self, id: Uuid) -> Result<(), DomainError> {
        sqlx::query!("DELETE FROM users WHERE id = $1", id)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn update_password(&self, id: Uuid, new_password_hash: String, audit: Option<&AuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!(
            "UPDATE users SET password_hash = $1, tokens_valid_after = $2 WHERE id = $3",
            new_password_hash,
            chrono::Utc::now(),
            id
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        crate::postgres::event_publisher::insert_audit(&mut tx, audit).await?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn set_super_admin(&self, id: Uuid, is_super_admin: bool) -> Result<(), DomainError> {
        sqlx::query!(
            "UPDATE users SET is_super_admin = $1, tokens_valid_after = CASE WHEN $1 THEN tokens_valid_after ELSE $2 END WHERE id = $3",
            is_super_admin,
            chrono::Utc::now(),
            id
        )
        .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn set_organization_admin(&self, id: Uuid, is_organization_admin: bool, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!(
            "UPDATE users SET is_organization_admin = $1, tokens_valid_after = CASE WHEN $1 THEN tokens_valid_after ELSE $2 END WHERE id = $3",
            is_organization_admin,
            chrono::Utc::now(),
            id
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        if let Some(audit) = audit {
            crate::postgres::event_publisher::insert_admin_event(&mut *tx, &audit.event, audit.actor_id).await.infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn delete_unless_last_super_admin(&self, id: Uuid, audit: Option<&AdminAuditRecord>) -> Result<bool, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;

        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext('user_super_admin_guard'))")
            .execute(&mut *tx)
            .await
            .infra_err()?;

        let target_is_admin: Option<bool> = sqlx::query_scalar!("SELECT is_super_admin FROM users WHERE id = $1", id)
            .fetch_optional(&mut *tx)
            .await
            .infra_err()?;

        if target_is_admin == Some(true) {
            let remaining: i64 = sqlx::query_scalar!("SELECT count(*) FROM users WHERE is_super_admin AND id <> $1", id)
                .fetch_one(&mut *tx)
                .await
                .infra_err()?
                .unwrap_or(0);
            if remaining == 0 {
                return Ok(false); // tx drops here without commit -> rolls back
            }
        }

        sqlx::query!("DELETE FROM users WHERE id = $1", id)
            .execute(&mut *tx)
            .await
            .infra_err()?;
        if let Some(audit) = audit {
            crate::postgres::event_publisher::insert_admin_event(&mut *tx, &audit.event, audit.actor_id).await.infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(true)
    }

    async fn set_super_admin_unless_last(&self, id: Uuid, is_super_admin: bool, audit: Option<&AdminAuditRecord>) -> Result<bool, DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;

        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext('user_super_admin_guard'))")
            .execute(&mut *tx)
            .await
            .infra_err()?;

        if !is_super_admin {
            let target_is_admin: Option<bool> = sqlx::query_scalar!("SELECT is_super_admin FROM users WHERE id = $1", id)
                .fetch_optional(&mut *tx)
                .await
                .infra_err()?;
            if target_is_admin == Some(true) {
                let remaining: i64 = sqlx::query_scalar!("SELECT count(*) FROM users WHERE is_super_admin AND id <> $1", id)
                    .fetch_one(&mut *tx)
                    .await
                    .infra_err()?
                    .unwrap_or(0);
                if remaining == 0 {
                    return Ok(false);
                }
            }
        }

        sqlx::query!(
            "UPDATE users SET is_super_admin = $1, tokens_valid_after = CASE WHEN $1 THEN tokens_valid_after ELSE $2 END WHERE id = $3",
            is_super_admin,
            chrono::Utc::now(),
            id
        )
        .execute(&mut *tx)
            .await
            .infra_err()?;
        if let Some(audit) = audit {
            crate::postgres::event_publisher::insert_admin_event(&mut *tx, &audit.event, audit.actor_id).await.infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(true)
    }
}

#[async_trait]
impl UserSecurityPort for PostgresUserRepository {
    async fn revoke_sessions(&self, id: Uuid, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("UPDATE users SET tokens_valid_after = $1 WHERE id = $2", chrono::Utc::now(), id).execute(&mut *tx).await.infra_err()?;
        crate::postgres::event_publisher::insert_security_audit(&mut tx, audit).await?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn find_by_verified_email(&self, organization_id: Uuid, email: &str) -> Result<Option<User>, DomainError> {
        let row = sqlx::query_as!(
            UserRow,
            "SELECT id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email FROM users WHERE organization_id = $1 AND lower(email) = lower($2) AND email_verified",
            organization_id,
            email,
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        row.map(UserRow::into_domain).transpose()
    }

    async fn insert_with_verified_email(&self, user: &User) -> Result<(), DomainError> {
        self.insert_row(user, true).await
    }

    async fn mark_email_verified(&self, id: Uuid) -> Result<bool, DomainError> {
        let result = sqlx::query!(
            "UPDATE users SET email_verified = true \
             WHERE id = $1 AND email IS NOT NULL AND NOT email_verified \
               AND NOT EXISTS (SELECT 1 FROM users other WHERE other.organization_id = users.organization_id AND other.id <> users.id AND other.email_verified AND lower(other.email) = lower(users.email))",
            id,
        )
        .execute(&self.pool)
        .await;
        match result {
            Ok(result) => Ok(result.rows_affected() > 0),
            // Lost a race with another account claiming the same address.
            Err(sqlx::Error::Database(db_err)) if db_err.constraint() == Some(VERIFIED_EMAIL_INDEX) => Ok(false),
            Err(e) => Err(DomainError::Infrastructure(e.to_string())),
        }
    }
}

const VERIFIED_EMAIL_INDEX: &str = "users_verified_email_per_organization_unique";

impl PostgresUserRepository {
    async fn insert_row(&self, user: &User, email_verified: bool) -> Result<(), DomainError> {
        insert_user_row(&self.pool, user, email_verified).await
    }
}

pub(crate) async fn insert_user_row<'e>(executor: impl sqlx::PgExecutor<'e>, user: &User, email_verified: bool) -> Result<(), DomainError> {
    sqlx::query!(
        "INSERT INTO users (id, username, password_hash, is_super_admin, is_organization_admin, organization_id, created_at, tokens_valid_after, email, email_verified) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        user.id,
        user.username.as_str(),
        user.password_hash,
        user.is_super_admin,
        user.is_organization_admin,
        user.organization_id,
        user.created_at,
        user.tokens_valid_after,
        user.email,
        email_verified,
    )
    .execute(executor)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.constraint() == Some(VERIFIED_EMAIL_INDEX) => DomainError::EmailTaken,
        sqlx::Error::Database(db_err) if matches!(db_err.constraint(), Some("users_username_key" | "users_username_lower_unique")) => DomainError::UsernameTaken,
        _ => DomainError::Infrastructure(e.to_string()),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::audit::AdminAuditEvent;
    use std::sync::Arc;

    #[sqlx::test]
    async fn inserts_and_finds_a_user_by_username(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: true,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        };
        repo.insert(&user).await.unwrap();

        let found = repo.find_by_username(&Username::parse("florian").unwrap()).await.unwrap().unwrap();
        assert_eq!(found.id, user.id);
        assert!(found.is_super_admin);
    }

    #[sqlx::test]
    async fn returns_none_for_an_unknown_username(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let found = repo.find_by_username(&Username::parse("ghost").unwrap()).await.unwrap();
        assert!(found.is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn finds_a_user_by_email(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: Some("florian@example.com".to_string()),
        };
        repo.insert(&user).await.unwrap();

        let found = repo.find_by_email("florian@example.com").await.unwrap().unwrap();
        assert_eq!(found.id, user.id);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn finding_by_an_unknown_email_returns_none(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        assert!(repo.find_by_email("nobody@example.com").await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn deletes_a_user(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse("todelete").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        };
        repo.insert(&user).await.unwrap();
        repo.delete(user.id).await.unwrap();
        assert!(repo.find_by_id(user.id).await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn only_one_of_two_concurrent_demotions_of_different_admins_succeeds(pool: sqlx::PgPool) {
        let repo = Arc::new(PostgresUserRepository::new(pool));
        let admin_a = User {
            id: Uuid::new_v4(),
            username: Username::parse("admin-a").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: true,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        };
        let admin_b = User {
            id: Uuid::new_v4(),
            username: Username::parse("admin-b").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: true,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        };
        repo.insert(&admin_a).await.unwrap();
        repo.insert(&admin_b).await.unwrap();

        let repo_a = repo.clone();
        let repo_b = repo.clone();
        let id_a = admin_a.id;
        let id_b = admin_b.id;

        let (result_a, result_b) = tokio::join!(
            tokio::spawn(async move { repo_a.set_super_admin_unless_last(id_a, false, None).await.unwrap() }),
            tokio::spawn(async move { repo_b.set_super_admin_unless_last(id_b, false, None).await.unwrap() }),
        );
        let result_a = result_a.unwrap();
        let result_b = result_b.unwrap();

        assert_ne!(result_a, result_b, "exactly one of the two concurrent demotions must succeed, got a={result_a} b={result_b}");

        let remaining_admins = repo.list_all().await.unwrap().into_iter().filter(|u| u.is_super_admin).count();
        assert_eq!(remaining_admins, 1, "the system must never end up with zero super-admins");
    }

    #[sqlx::test]
    async fn updates_the_password_hash(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "old-hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        };
        repo.insert(&user).await.unwrap();

        repo.update_password(user.id, "new-hash".to_string(), None).await.unwrap();

        let found = repo.find_by_id(user.id).await.unwrap().unwrap();
        assert_eq!(found.password_hash, "new-hash");
    }

    fn user(username: &str, organization_id: Uuid) -> User {
        User {
            id: Uuid::new_v4(),
            username: Username::parse(username).unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id,
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        }
    }

    async fn seed_organization(pool: &sqlx::PgPool, id: Uuid, slug: &str) {
        sqlx::query!("INSERT INTO organizations (id, slug, display_name) VALUES ($1, $2, $3)", id, slug, slug).execute(pool).await.unwrap();
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_by_organization_only_matches_a_substring_within_that_organization(pool: sqlx::PgPool) {
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();
        seed_organization(&pool, org_a, "org-a").await;
        seed_organization(&pool, org_b, "org-b").await;
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user("florian-simon", org_a)).await.unwrap();
        repo.insert(&user("florian-other-org", org_b)).await.unwrap();
        repo.insert(&user("someone-else", org_a)).await.unwrap();

        let results = repo.search_by_organization(org_a, "flor", 10).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].username.as_str(), "florian-simon");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_by_organization_is_capped_at_the_given_limit(pool: sqlx::PgPool) {
        let org_id = Uuid::new_v4();
        seed_organization(&pool, org_id, "org-with-many-matches").await;
        let repo = PostgresUserRepository::new(pool);
        for i in 0..5 {
            repo.insert(&user(&format!("match-user-{i}"), org_id)).await.unwrap();
        }

        let results = repo.search_by_organization(org_id, "match", 3).await.unwrap();

        assert_eq!(results.len(), 3, "the limit must be enforced by the query itself, not just truncated afterwards");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_by_organization_results_are_sorted_by_username(pool: sqlx::PgPool) {
        let org_id = Uuid::new_v4();
        seed_organization(&pool, org_id, "org-for-sort-order").await;
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user("match-charlie", org_id)).await.unwrap();
        repo.insert(&user("match-alice", org_id)).await.unwrap();
        repo.insert(&user("match-bob", org_id)).await.unwrap();

        let results = repo.search_by_organization(org_id, "match", 10).await.unwrap();

        assert_eq!(results.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), vec!["match-alice", "match-bob", "match-charlie"]);
    }

    /// A literal `_` in the query must not act as ILIKE's single-character wildcard — otherwise
    /// searching for `a_b` would also match `axb`, `a5b`, etc.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_by_organization_treats_an_underscore_in_the_query_as_a_literal_character(pool: sqlx::PgPool) {
        let org_id = Uuid::new_v4();
        seed_organization(&pool, org_id, "org-for-wildcard-escaping").await;
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user("under_score", org_id)).await.unwrap();
        repo.insert(&user("underxscore", org_id)).await.unwrap();

        let results = repo.search_by_organization(org_id, "under_score", 10).await.unwrap();

        assert_eq!(results.len(), 1, "an underscore in the query must be escaped, not treated as ILIKE's single-character wildcard");
        assert_eq!(results[0].username.as_str(), "under_score");
    }

    /// A literal `%` in the query must not act as ILIKE's any-substring wildcard.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_by_organization_treats_a_percent_sign_in_the_query_as_a_literal_character(pool: sqlx::PgPool) {
        let org_id = Uuid::new_v4();
        seed_organization(&pool, org_id, "org-for-percent-escaping").await;
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user("has-percent", org_id)).await.unwrap();

        let results = repo.search_by_organization(org_id, "%", 10).await.unwrap();

        assert!(results.is_empty(), "a bare percent sign in the query must be escaped, not treated as ILIKE's any-substring wildcard");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn search_all_organizations_matches_across_every_organization(pool: sqlx::PgPool) {
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();
        seed_organization(&pool, org_a, "org-a-cross").await;
        seed_organization(&pool, org_b, "org-b-cross").await;
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user("cross-alice", org_a)).await.unwrap();
        repo.insert(&user("cross-bob", org_b)).await.unwrap();

        let results = repo.search_all_organizations("cross", 10).await.unwrap();

        assert_eq!(results.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), vec!["cross-alice", "cross-bob"]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn find_by_ids_returns_only_the_requested_users(pool: sqlx::PgPool) {
        let org_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let repo = PostgresUserRepository::new(pool);
        let alice = user("alice", org_id);
        let bob = user("bob", org_id);
        let carol = user("carol", org_id);
        repo.insert(&alice).await.unwrap();
        repo.insert(&bob).await.unwrap();
        repo.insert(&carol).await.unwrap();

        let mut results = repo.find_by_ids(&[alice.id, bob.id]).await.unwrap();
        results.sort_by_key(|u| u.id);
        let mut expected_ids = vec![alice.id, bob.id];
        expected_ids.sort();

        assert_eq!(results.len(), 2, "must return exactly the requested users, not every user");
        assert_eq!(results.iter().map(|u| u.id).collect::<Vec<_>>(), expected_ids);
        assert!(results.iter().all(|u| u.id != carol.id), "must not include a user that wasn't requested");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn find_by_ids_with_an_empty_slice_returns_no_users(pool: sqlx::PgPool) {
        let org_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user("solo", org_id)).await.unwrap();

        let results = repo.find_by_ids(&[]).await.unwrap();

        assert!(results.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn count_by_organization_only_counts_that_organizations_users(pool: sqlx::PgPool) {
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();
        seed_organization(&pool, org_a, "count-org-a").await;
        seed_organization(&pool, org_b, "count-org-b").await;
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user("count-alice", org_a)).await.unwrap();
        repo.insert(&user("count-bob", org_a)).await.unwrap();
        repo.insert(&user("count-carol", org_b)).await.unwrap();

        assert_eq!(repo.count_by_organization(org_a).await.unwrap(), 2);
        assert_eq!(repo.count_by_organization(org_b).await.unwrap(), 1);
    }

    #[sqlx::test]
    async fn sets_organization_admin_status(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        };
        repo.insert(&user).await.unwrap();

        repo.set_organization_admin(user.id, true, None).await.unwrap();

        let found = repo.find_by_id(user.id).await.unwrap().unwrap();
        assert!(found.is_organization_admin);
    }

    fn user_with_stale_tokens(username: &str, is_super_admin: bool, is_organization_admin: bool) -> User {
        User {
            is_super_admin,
            is_organization_admin,
            tokens_valid_after: chrono::Utc::now() - chrono::Duration::hours(1),
            ..user(username, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
        }
    }

    async fn tokens_were_revoked(repo: &PostgresUserRepository, id: Uuid) -> bool {
        repo.find_by_id(id).await.unwrap().unwrap().tokens_valid_after > chrono::Utc::now() - chrono::Duration::minutes(1)
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn revoking_sessions_bumps_tokens_valid_after(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let user = user_with_stale_tokens("florian", false, false);
        repo.insert(&user).await.unwrap();

        repo.revoke_sessions(user.id, None).await.unwrap();

        assert!(tokens_were_revoked(&repo, user.id).await);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn demoting_a_super_admin_revokes_their_tokens_but_promoting_does_not(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let promoted = user_with_stale_tokens("promoted", false, false);
        let demoted = user_with_stale_tokens("demoted", true, false);
        let kept = user_with_stale_tokens("kept-admin", true, false);
        repo.insert(&promoted).await.unwrap();
        repo.insert(&demoted).await.unwrap();
        repo.insert(&kept).await.unwrap();

        repo.set_super_admin(promoted.id, true).await.unwrap();
        repo.set_super_admin(demoted.id, false).await.unwrap();

        assert!(!tokens_were_revoked(&repo, promoted.id).await);
        assert!(tokens_were_revoked(&repo, demoted.id).await);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn demoting_a_super_admin_with_the_last_admin_guard_revokes_their_tokens(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let demoted = user_with_stale_tokens("demoted", true, false);
        let other = user_with_stale_tokens("other-admin", true, false);
        repo.insert(&demoted).await.unwrap();
        repo.insert(&other).await.unwrap();

        assert!(repo.set_super_admin_unless_last(demoted.id, false, None).await.unwrap());

        assert!(tokens_were_revoked(&repo, demoted.id).await);
        assert!(!tokens_were_revoked(&repo, other.id).await);
    }

    async fn audit_event_types(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar("SELECT event_type FROM domain_events WHERE aggregate_type = 'Admin' ORDER BY occurred_at").fetch_all(pool).await.unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_audit_entry_is_written_with_the_super_admin_change_and_dropped_when_it_is_refused(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool.clone());
        let promoted = user_with_stale_tokens("promoted", false, false);
        let only_admin = user_with_stale_tokens("only-admin", true, false);
        repo.insert(&promoted).await.unwrap();
        repo.insert(&only_admin).await.unwrap();
        let record = |user: &User, granted: bool| {
            let (user_id, organization_id) = (user.id, user.organization_id);
            let event = if granted { AdminAuditEvent::SuperAdminGranted { user_id, organization_id } } else { AdminAuditEvent::SuperAdminRevoked { user_id, organization_id } };
            AdminAuditRecord { event, actor_id: Some(only_admin.id) }
        };

        assert!(repo.set_super_admin_unless_last(promoted.id, true, Some(&record(&promoted, true))).await.unwrap());
        assert_eq!(audit_event_types(&pool).await, vec!["SuperAdminGranted"]);

        // Two super-admins now: demote one, then the guard refuses to demote the last and must not leave its entry behind.
        assert!(repo.set_super_admin_unless_last(promoted.id, false, Some(&record(&promoted, false))).await.unwrap());
        assert!(!repo.set_super_admin_unless_last(only_admin.id, false, Some(&record(&only_admin, false))).await.unwrap());
        assert_eq!(audit_event_types(&pool).await, vec!["SuperAdminGranted", "SuperAdminRevoked"]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn demoting_an_organization_admin_revokes_their_tokens_but_promoting_does_not(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let promoted = user_with_stale_tokens("promoted", false, false);
        let demoted = user_with_stale_tokens("demoted", false, true);
        repo.insert(&promoted).await.unwrap();
        repo.insert(&demoted).await.unwrap();

        repo.set_organization_admin(promoted.id, true, None).await.unwrap();
        repo.set_organization_admin(demoted.id, false, None).await.unwrap();

        assert!(!tokens_were_revoked(&repo, promoted.id).await);
        assert!(tokens_were_revoked(&repo, demoted.id).await);
    }

    const PUBLIC_ORG: Uuid = Uuid::from_u128(1);

    fn user_with_email(username: &str, email: &str) -> User {
        User { email: Some(email.to_string()), ..user(username, PUBLIC_ORG) }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_unverified_email_is_not_found_by_verified_email_lookup(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user_with_email("registered", "alice@corp.example")).await.unwrap();

        assert!(repo.find_by_verified_email(PUBLIC_ORG, "alice@corp.example").await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_verified_email_is_found_regardless_of_case(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let account = user_with_email("invited", "alice@corp.example");
        repo.insert_with_verified_email(&account).await.unwrap();

        let found = repo.find_by_verified_email(PUBLIC_ORG, "Alice@Corp.Example").await.unwrap().unwrap();

        assert_eq!(found.id, account.id);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn unverified_registrations_of_the_same_email_can_coexist_with_a_verified_account(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        repo.insert(&user_with_email("squatter-one", "alice@corp.example")).await.unwrap();
        repo.insert(&user_with_email("squatter-two", "ALICE@corp.example")).await.unwrap();
        let real = user_with_email("alice", "alice@corp.example");

        repo.insert_with_verified_email(&real).await.unwrap();

        assert_eq!(repo.find_by_verified_email(PUBLIC_ORG, "alice@corp.example").await.unwrap().unwrap().id, real.id);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_verified_accounts_cannot_share_an_email_even_with_different_case(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        repo.insert_with_verified_email(&user_with_email("first", "alice@corp.example")).await.unwrap();

        let err = repo.insert_with_verified_email(&user_with_email("second", "Alice@Corp.Example")).await.unwrap_err();

        assert_eq!(err, DomainError::EmailTaken);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_organizations_can_each_hold_the_same_verified_email(pool: sqlx::PgPool) {
        let acme = Uuid::new_v4();
        seed_organization(&pool, acme, "acme").await;
        let repo = PostgresUserRepository::new(pool);
        let public = user_with_email("alice", "alice@corp.example");
        let in_acme = User { organization_id: acme, ..user_with_email("alice-acme", "alice@corp.example") };

        repo.insert_with_verified_email(&public).await.unwrap();
        repo.insert_with_verified_email(&in_acme).await.unwrap();

        assert_eq!(repo.find_by_verified_email(PUBLIC_ORG, "alice@corp.example").await.unwrap().unwrap().id, public.id);
        assert_eq!(repo.find_by_verified_email(acme, "alice@corp.example").await.unwrap().unwrap().id, in_acme.id);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_verified_email_of_another_organization_is_not_found(pool: sqlx::PgPool) {
        let acme = Uuid::new_v4();
        seed_organization(&pool, acme, "acme").await;
        let repo = PostgresUserRepository::new(pool);
        repo.insert_with_verified_email(&user_with_email("alice", "alice@corp.example")).await.unwrap();

        assert!(repo.find_by_verified_email(acme, "alice@corp.example").await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn marking_an_email_verified_makes_it_findable(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let invited = user_with_email("invited", "alice@corp.example");
        repo.insert(&invited).await.unwrap();

        assert!(repo.mark_email_verified(invited.id).await.unwrap());

        assert_eq!(repo.find_by_verified_email(PUBLIC_ORG, "alice@corp.example").await.unwrap().unwrap().id, invited.id);
        assert!(!repo.mark_email_verified(invited.id).await.unwrap(), "already verified, nothing changes");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_email_already_verified_in_the_organization_cannot_be_marked_verified_again(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        let holder = user_with_email("holder", "alice@corp.example");
        repo.insert_with_verified_email(&holder).await.unwrap();
        let latecomer = user_with_email("latecomer", "Alice@corp.example");
        repo.insert(&latecomer).await.unwrap();

        assert!(!repo.mark_email_verified(latecomer.id).await.unwrap());

        assert_eq!(repo.find_by_verified_email(PUBLIC_ORG, "alice@corp.example").await.unwrap().unwrap().id, holder.id);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_taken_username_is_reported_as_such(pool: sqlx::PgPool) {
        let repo = PostgresUserRepository::new(pool);
        repo.insert_with_verified_email(&user_with_email("alice", "alice@corp.example")).await.unwrap();

        let err = repo.insert_with_verified_email(&user_with_email("Alice", "other@corp.example")).await.unwrap_err();

        assert_eq!(err, DomainError::UsernameTaken);
    }
}
