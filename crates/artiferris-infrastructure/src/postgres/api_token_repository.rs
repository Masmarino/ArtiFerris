use async_trait::async_trait;
use chrono::{DateTime, Utc};
use artiferris_domain::api_token::{ApiToken, ApiTokenRepositoryPort, ApiTokenWithOwner};
use artiferris_domain::audit::SecurityAuditRecord;
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresApiTokenRepository {
    pool: PgPool,
}

impl PostgresApiTokenRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

struct TokenRow {
    id: Uuid,
    user_id: Uuid,
    token_hash: String,
    label: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

impl From<TokenRow> for ApiToken {
    fn from(row: TokenRow) -> Self {
        ApiToken {
            id: row.id,
            user_id: row.user_id,
            token_hash: row.token_hash,
            label: row.label,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
            revoked_at: row.revoked_at,
            expires_at: row.expires_at,
        }
    }
}

struct TokenWithOwnerRow {
    id: Uuid,
    user_id: Uuid,
    token_hash: String,
    label: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    owner_username: String,
    owner_organization_id: Uuid,
    owner_is_super_admin: bool,
}

impl From<TokenWithOwnerRow> for ApiTokenWithOwner {
    fn from(r: TokenWithOwnerRow) -> Self {
        ApiTokenWithOwner {
            token: ApiToken { id: r.id, user_id: r.user_id, token_hash: r.token_hash, label: r.label, created_at: r.created_at, last_used_at: r.last_used_at, revoked_at: r.revoked_at, expires_at: r.expires_at },
            owner_username: r.owner_username,
            owner_organization_id: r.owner_organization_id,
            owner_is_super_admin: r.owner_is_super_admin,
        }
    }
}

#[async_trait]
impl ApiTokenRepositoryPort for PostgresApiTokenRepository {
    async fn insert(&self, token: &ApiToken) -> Result<(), DomainError> {
        sqlx::query!(
            "INSERT INTO api_tokens (id, user_id, token_hash, label, created_at, last_used_at, revoked_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            token.id,
            token.user_id,
            token.token_hash,
            token.label,
            token.created_at,
            token.last_used_at,
            token.revoked_at,
            token.expires_at,
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<ApiToken>, DomainError> {
        let rows = sqlx::query_as!(
            TokenRow,
            "SELECT id, user_id, token_hash, label, created_at, last_used_at, revoked_at, expires_at FROM api_tokens WHERE user_id = $1 AND revoked_at IS NULL ORDER BY created_at DESC",
            user_id
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        Ok(rows.into_iter().map(ApiToken::from).collect())
    }

    async fn list_with_owners(&self, organization_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<ApiTokenWithOwner>, DomainError> {
        let rows = sqlx::query_as!(
            TokenWithOwnerRow,
            r#"SELECT t.id, t.user_id, t.token_hash, t.label, t.created_at, t.last_used_at, t.revoked_at, t.expires_at,
                      u.username AS owner_username, u.organization_id AS owner_organization_id, u.is_super_admin AS owner_is_super_admin
               FROM api_tokens t JOIN users u ON u.id = t.user_id
               WHERE $1::uuid IS NULL OR u.organization_id = $1
               ORDER BY t.created_at DESC, t.id DESC
               LIMIT $2 OFFSET $3"#,
            organization_id,
            limit,
            offset,
        )
        .fetch_all(&self.pool)
        .await
        .infra_err()?;
        Ok(rows.into_iter().map(ApiTokenWithOwner::from).collect())
    }

    async fn find_with_owner(&self, id: Uuid) -> Result<Option<ApiTokenWithOwner>, DomainError> {
        let row = sqlx::query_as!(
            TokenWithOwnerRow,
            r#"SELECT t.id, t.user_id, t.token_hash, t.label, t.created_at, t.last_used_at, t.revoked_at, t.expires_at,
                      u.username AS owner_username, u.organization_id AS owner_organization_id, u.is_super_admin AS owner_is_super_admin
               FROM api_tokens t JOIN users u ON u.id = t.user_id
               WHERE t.id = $1"#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        Ok(row.map(ApiTokenWithOwner::from))
    }

    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<ApiToken>, DomainError> {
        let row = sqlx::query_as!(
            TokenRow,
            "SELECT id, user_id, token_hash, label, created_at, last_used_at, revoked_at, expires_at FROM api_tokens WHERE token_hash = $1",
            token_hash
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        Ok(row.map(ApiToken::from))
    }

    async fn touch_last_used_at(&self, id: Uuid, used_at: DateTime<Utc>) -> Result<(), DomainError> {
        sqlx::query!("UPDATE api_tokens SET last_used_at = $2 WHERE id = $1", id, used_at)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn revoke(&self, id: Uuid, user_id: Uuid) -> Result<bool, DomainError> {
        let result = sqlx::query!("UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND user_id = $2", id, user_id)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(result.rows_affected() > 0)
    }

    async fn revoke_any(&self, id: Uuid, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("UPDATE api_tokens SET revoked_at = now() WHERE id = $1", id)
            .execute(&mut *tx)
            .await
            .infra_err()?;
        if let Some(audit) = audit {
            crate::postgres::event_publisher::insert_security_event(&mut *tx, &audit.event, audit.actor_id).await.infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::api_token::ApiToken;

    fn sample(user_id: Uuid) -> ApiToken {
        ApiToken {
            id: Uuid::new_v4(),
            user_id,
            token_hash: "hash-1".into(),
            label: "laptop".into(),
            created_at: chrono::Utc::now(),
            last_used_at: None,
            revoked_at: None,
            expires_at: None,
        }
    }

    async fn seed_user(pool: &sqlx::PgPool, id: Uuid) {
        sqlx::query!(
            "INSERT INTO users (id, username, password_hash, is_super_admin, organization_id, created_at) VALUES ($1, $2, 'h', FALSE, $3, now())",
            id,
            format!("user-{id}"),
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn inserts_and_finds_by_hash(pool: sqlx::PgPool) {
        let user_id = Uuid::new_v4();
        seed_user(&pool, user_id).await;
        let repo = PostgresApiTokenRepository::new(pool);
        let token = sample(user_id);
        repo.insert(&token).await.unwrap();

        let found = repo.find_by_hash("hash-1").await.unwrap().unwrap();
        assert_eq!(found.id, token.id);
    }

    #[sqlx::test]
    async fn revoke_only_affects_the_owning_user(pool: sqlx::PgPool) {
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        seed_user(&pool, owner).await;
        seed_user(&pool, other).await;
        let repo = PostgresApiTokenRepository::new(pool);
        let token = sample(owner);
        repo.insert(&token).await.unwrap();

        assert!(!repo.revoke(token.id, other).await.unwrap(), "revoke must report no row affected for a non-owner");
        let unaffected = repo.find_by_hash("hash-1").await.unwrap().unwrap();
        assert!(unaffected.revoked_at.is_none());

        assert!(repo.revoke(token.id, owner).await.unwrap(), "revoke must report a row affected for the owner");
        let revoked = repo.find_by_hash("hash-1").await.unwrap().unwrap();
        assert!(revoked.revoked_at.is_some());
    }

    #[sqlx::test]
    async fn list_for_user_excludes_revoked_tokens(pool: sqlx::PgPool) {
        let owner = Uuid::new_v4();
        seed_user(&pool, owner).await;
        let repo = PostgresApiTokenRepository::new(pool);

        let active = sample(owner);
        let mut to_revoke = sample(owner);
        to_revoke.id = Uuid::new_v4();
        to_revoke.token_hash = "hash-2".into();
        repo.insert(&active).await.unwrap();
        repo.insert(&to_revoke).await.unwrap();

        let before = repo.list_for_user(owner).await.unwrap();
        assert_eq!(before.len(), 2, "both tokens should be listed before either is revoked");

        repo.revoke(to_revoke.id, owner).await.unwrap();

        let after = repo.list_for_user(owner).await.unwrap();
        assert_eq!(after.len(), 1, "a revoked token must disappear from the list");
        assert_eq!(after[0].id, active.id);
    }

    async fn seed_user_in(pool: &sqlx::PgPool, id: Uuid, organization_id: Uuid, is_super_admin: bool) {
        sqlx::query!(
            "INSERT INTO users (id, username, password_hash, is_super_admin, organization_id, created_at) VALUES ($1, $2, 'h', $3, $4, now())",
            id,
            format!("user-{id}"),
            is_super_admin,
            organization_id,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_organization(pool: &sqlx::PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!("INSERT INTO organizations (id, slug, display_name, is_public) VALUES ($1, $2, 'Other', FALSE)", id, format!("org-{id}")).execute(pool).await.unwrap();
        id
    }

    #[sqlx::test]
    async fn listing_with_owners_spans_every_user_keeps_revoked_tokens_and_names_the_owner(pool: sqlx::PgPool) {
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        seed_user(&pool, alice).await;
        seed_user_in(&pool, bob, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), true).await;
        let repo = PostgresApiTokenRepository::new(pool);

        let alices_token = sample(alice);
        let mut bobs_token = sample(bob);
        bobs_token.id = Uuid::new_v4();
        bobs_token.token_hash = "hash-2".into();
        repo.insert(&alices_token).await.unwrap();
        repo.insert(&bobs_token).await.unwrap();
        repo.revoke(bobs_token.id, bob).await.unwrap();

        let all = repo.list_with_owners(None, 500, 0).await.unwrap();
        assert_eq!(all.len(), 2);
        let revoked = all.iter().find(|t| t.token.id == bobs_token.id).unwrap();
        assert!(revoked.token.revoked_at.is_some(), "the listing must keep revoked tokens, not drop them");
        assert!(revoked.owner_is_super_admin);
        assert_eq!(revoked.owner_username, format!("user-{bob}"));
        assert!(!all.iter().find(|t| t.token.id == alices_token.id).unwrap().owner_is_super_admin);
    }

    #[sqlx::test]
    async fn listing_by_organization_only_reaches_that_organizations_tokens(pool: sqlx::PgPool) {
        let public = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = seed_organization(&pool).await;
        let (mine, theirs) = (Uuid::new_v4(), Uuid::new_v4());
        seed_user_in(&pool, mine, public, false).await;
        seed_user_in(&pool, theirs, other, false).await;
        let repo = PostgresApiTokenRepository::new(pool);
        let my_token = sample(mine);
        let mut their_token = sample(theirs);
        their_token.id = Uuid::new_v4();
        their_token.token_hash = "hash-2".into();
        repo.insert(&my_token).await.unwrap();
        repo.insert(&their_token).await.unwrap();

        let listed = repo.list_with_owners(Some(public), 500, 0).await.unwrap();

        assert_eq!(listed.iter().map(|t| t.token.id).collect::<Vec<_>>(), vec![my_token.id]);
        assert!(repo.list_with_owners(Some(Uuid::new_v4()), 500, 0).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn listing_is_paged_newest_first_and_a_page_never_exceeds_its_limit(pool: sqlx::PgPool) {
        let owner = Uuid::new_v4();
        seed_user(&pool, owner).await;
        let repo = PostgresApiTokenRepository::new(pool);
        let mut ids = Vec::new();
        for i in 0..5 {
            let mut token = sample(owner);
            token.id = Uuid::new_v4();
            token.token_hash = format!("hash-{i}");
            token.created_at = chrono::Utc::now() - chrono::Duration::minutes(10 - i);
            ids.push(token.id);
            repo.insert(&token).await.unwrap();
        }

        let first = repo.list_with_owners(None, 2, 0).await.unwrap();
        let second = repo.list_with_owners(None, 2, 2).await.unwrap();
        let last = repo.list_with_owners(None, 2, 4).await.unwrap();

        let paged: Vec<Uuid> = first.iter().chain(&second).chain(&last).map(|t| t.token.id).collect();
        ids.reverse();
        assert_eq!(paged, ids);
        assert_eq!((first.len(), second.len(), last.len()), (2, 2, 1));
    }

    #[sqlx::test]
    async fn finding_one_token_by_id_names_its_owner(pool: sqlx::PgPool) {
        let owner = Uuid::new_v4();
        seed_user(&pool, owner).await;
        let repo = PostgresApiTokenRepository::new(pool);
        let token = sample(owner);
        repo.insert(&token).await.unwrap();

        let found = repo.find_with_owner(token.id).await.unwrap().unwrap();

        assert_eq!(found.owner_organization_id, Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        assert!(repo.find_with_owner(Uuid::new_v4()).await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn revoke_any_revokes_regardless_of_owner(pool: sqlx::PgPool) {
        let owner = Uuid::new_v4();
        seed_user(&pool, owner).await;
        let repo = PostgresApiTokenRepository::new(pool);
        let token = sample(owner);
        repo.insert(&token).await.unwrap();

        repo.revoke_any(token.id, None).await.unwrap();

        let revoked = repo.find_by_hash("hash-1").await.unwrap().unwrap();
        assert!(revoked.revoked_at.is_some());
    }
}
