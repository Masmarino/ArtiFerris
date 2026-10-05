use async_trait::async_trait;
use artiferris_domain::error::DomainError;
use artiferris_domain::password_reset::{PasswordReset, PasswordResetPort};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error_ext::InfraErr;

/// Runtime queries, not `query!`: no offline data to regenerate for a table this small.
pub struct PostgresPasswordResetRepository {
    pool: PgPool,
}

impl PostgresPasswordResetRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type ResetRow = (Uuid, String, DateTime<Utc>);

fn from_row((user_id, token_hash, expires_at): ResetRow) -> PasswordReset {
    PasswordReset { user_id, token_hash, expires_at }
}

#[async_trait]
impl PasswordResetPort for PostgresPasswordResetRepository {
    async fn replace(&self, reset: &PasswordReset) -> Result<(), DomainError> {
        sqlx::query(
            "INSERT INTO password_resets (user_id, token_hash, expires_at) VALUES ($1, $2, $3) \
             ON CONFLICT (user_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, expires_at = EXCLUDED.expires_at",
        )
        .bind(reset.user_id)
        .bind(&reset.token_hash)
        .bind(reset.expires_at)
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }

    async fn consume(&self, token_hash: &str) -> Result<Option<PasswordReset>, DomainError> {
        let row: Option<ResetRow> = sqlx::query_as("DELETE FROM password_resets WHERE token_hash = $1 AND expires_at > now() RETURNING user_id, token_hash, expires_at")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .infra_err()?;
        Ok(row.map(from_row))
    }

    async fn restore(&self, reset: &PasswordReset) -> Result<bool, DomainError> {
        let restored = sqlx::query("INSERT INTO password_resets (user_id, token_hash, expires_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(reset.user_id)
            .bind(&reset.token_hash)
            .bind(reset.expires_at)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(restored.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::user::{User, UserRepositoryPort, Username};

    use crate::postgres::user_repository::PostgresUserRepository;

    async fn seed_user(pool: &PgPool) -> Uuid {
        let users = PostgresUserRepository::new(pool.clone());
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse("florian").unwrap(),
            password_hash: "unusable".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: Utc::now(),
            tokens_valid_after: Utc::now(),
            email: Some("florian@example.com".to_string()),
        };
        users.insert(&user).await.unwrap();
        user.id
    }

    fn reset(user_id: Uuid, token_hash: &str, expires_in: chrono::Duration) -> PasswordReset {
        PasswordReset { user_id, token_hash: token_hash.to_string(), expires_at: Utc::now() + expires_in }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_link_is_consumed_once(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresPasswordResetRepository::new(pool);
        repo.replace(&reset(user_id, "hash-a", chrono::Duration::hours(1))).await.unwrap();

        assert_eq!(repo.consume("hash-a").await.unwrap().unwrap().user_id, user_id);
        assert_eq!(repo.consume("hash-a").await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_new_link_voids_the_previous_one(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresPasswordResetRepository::new(pool);
        repo.replace(&reset(user_id, "hash-a", chrono::Duration::hours(1))).await.unwrap();
        repo.replace(&reset(user_id, "hash-b", chrono::Duration::hours(1))).await.unwrap();

        assert_eq!(repo.consume("hash-a").await.unwrap(), None);
        assert!(repo.consume("hash-b").await.unwrap().is_some());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_expired_link_is_refused(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresPasswordResetRepository::new(pool);
        repo.replace(&reset(user_id, "hash-a", chrono::Duration::minutes(-1))).await.unwrap();

        assert_eq!(repo.consume("hash-a").await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_restored_link_never_replaces_a_newer_one(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresPasswordResetRepository::new(pool);
        repo.replace(&reset(user_id, "hash-a", chrono::Duration::hours(1))).await.unwrap();
        let consumed = repo.consume("hash-a").await.unwrap().unwrap();

        assert!(repo.restore(&consumed).await.unwrap(), "put back while nothing newer exists");
        repo.consume("hash-a").await.unwrap().unwrap();
        repo.replace(&reset(user_id, "hash-b", chrono::Duration::hours(1))).await.unwrap();

        assert!(!repo.restore(&consumed).await.unwrap());
        assert!(repo.consume("hash-b").await.unwrap().is_some());
    }
}
