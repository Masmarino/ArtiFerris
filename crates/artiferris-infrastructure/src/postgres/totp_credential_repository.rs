use async_trait::async_trait;
use chrono::{DateTime, Utc};
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use artiferris_domain::mfa::{TotpCredential, TotpCredentialPort};
use sqlx::PgPool;
use uuid::Uuid;

use crate::secret_box;

pub struct PostgresTotpCredentialRepository {
    pool: PgPool,
    secrets_encryption_key: String,
}

impl PostgresTotpCredentialRepository {
    pub fn new(pool: PgPool, secrets_encryption_key: String) -> Self {
        Self { pool, secrets_encryption_key }
    }
}

#[async_trait]
impl TotpCredentialPort for PostgresTotpCredentialRepository {
    /// Reads `confirmed` only, never the encrypted secret.
    async fn confirmed_among(&self, user_ids: &[Uuid]) -> Result<std::collections::HashSet<Uuid>, DomainError> {
        if user_ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT user_id FROM totp_credentials WHERE confirmed AND user_id = ANY($1)").bind(user_ids).fetch_all(&self.pool).await.infra_err()?;
        Ok(rows.into_iter().map(|(user_id,)| user_id).collect())
    }

    async fn get(&self, user_id: Uuid) -> Result<Option<TotpCredential>, DomainError> {
        let row = sqlx::query!(
            "SELECT user_id, encrypted_secret, secret_nonce, confirmed, last_used_step, created_at FROM totp_credentials WHERE user_id = $1",
            user_id
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        let Some(row) = row else {
            return Ok(None);
        };
        let secret = secret_box::open(&row.encrypted_secret, &row.secret_nonce, &self.secrets_encryption_key, secret_box::TOTP_SEED)
            .inspect_err(|e| tracing::error!(%user_id, "the stored TOTP seed cannot be read, this user cannot pass MFA: {e}"))?;
        Ok(Some(TotpCredential { user_id: row.user_id, secret, confirmed: row.confirmed, last_used_step: row.last_used_step, created_at: row.created_at }))
    }

    async fn begin_enrollment(&self, user_id: Uuid, secret: &str, created_at: DateTime<Utc>) -> Result<bool, DomainError> {
        let (encrypted_secret, secret_nonce) = secret_box::seal(secret, &self.secrets_encryption_key, secret_box::TOTP_SEED);
        // The WHERE on the conflict branch is what keeps a confirmed credential from being replaced.
        let result = sqlx::query!(
            "INSERT INTO totp_credentials (user_id, encrypted_secret, secret_nonce, confirmed, last_used_step, created_at) \
             VALUES ($1, $2, $3, false, NULL, $4) \
             ON CONFLICT (user_id) DO UPDATE SET \
             encrypted_secret = EXCLUDED.encrypted_secret, secret_nonce = EXCLUDED.secret_nonce, \
             confirmed = false, last_used_step = NULL, created_at = EXCLUDED.created_at \
             WHERE NOT totp_credentials.confirmed",
            user_id,
            encrypted_secret,
            secret_nonce,
            created_at,
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(result.rows_affected() > 0)
    }

    async fn confirm(&self, user_id: Uuid, enrollment_created_at: DateTime<Utc>, step: i64) -> Result<bool, DomainError> {
        let result = sqlx::query!(
            "UPDATE totp_credentials SET confirmed = true, last_used_step = $3 WHERE user_id = $1 AND created_at = $2 AND NOT confirmed",
            user_id,
            enrollment_created_at,
            step,
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(result.rows_affected() > 0)
    }

    async fn set_last_used_step(&self, user_id: Uuid, step: i64) -> Result<bool, DomainError> {
        // The AND guard makes this a compare-and-swap — only the first concurrent caller wins.
        let result = sqlx::query!(
            "UPDATE totp_credentials SET last_used_step = $1 WHERE user_id = $2 AND (last_used_step IS NULL OR last_used_step < $1)",
            step,
            user_id
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete(&self, user_id: Uuid) -> Result<(), DomainError> {
        sqlx::query!("DELETE FROM totp_credentials WHERE user_id = $1", user_id).execute(&self.pool).await.infra_err()?;
        Ok(())
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
            username: Username::parse("mfauser").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        };
        users.insert(&user).await.unwrap();
        user.id
    }

    const SECRET: &str = "JBSWY3DPEHPK3PXP";

    async fn enrolled(repo: &PostgresTotpCredentialRepository, user_id: Uuid) -> DateTime<Utc> {
        let created_at = Utc::now();
        assert!(repo.begin_enrollment(user_id, SECRET, created_at).await.unwrap());
        repo.get(user_id).await.unwrap().unwrap().created_at
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn get_returns_none_when_never_enrolled(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        assert_eq!(repo.get(user_id).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn enrolling_then_getting_round_trips_including_the_secret(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        let created_at = Utc::now();

        assert!(repo.begin_enrollment(user_id, SECRET, created_at).await.unwrap());

        let found = repo.get(user_id).await.unwrap().unwrap();
        assert_eq!(found.secret, SECRET);
        assert!(!found.confirmed);
        assert_eq!(found.last_used_step, None);
        assert_eq!(found.created_at.timestamp_micros(), created_at.timestamp_micros());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_new_attempt_replaces_an_unconfirmed_one(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        enrolled(&repo, user_id).await;

        assert!(repo.begin_enrollment(user_id, "MFRGGZDFMZTWQ2LK", Utc::now()).await.unwrap());

        assert_eq!(repo.get(user_id).await.unwrap().unwrap().secret, "MFRGGZDFMZTWQ2LK");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn enrolling_never_replaces_a_confirmed_credential(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        let created_at = enrolled(&repo, user_id).await;
        assert!(repo.confirm(user_id, created_at, 7).await.unwrap());

        assert!(!repo.begin_enrollment(user_id, "MFRGGZDFMZTWQ2LK", Utc::now()).await.unwrap());

        let found = repo.get(user_id).await.unwrap().unwrap();
        assert!(found.confirmed, "the confirmed factor must survive a racing enrol");
        assert_eq!(found.secret, SECRET);
        assert_eq!(found.last_used_step, Some(7));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn confirming_needs_the_attempt_that_was_started(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        let first_attempt = enrolled(&repo, user_id).await;
        repo.begin_enrollment(user_id, "MFRGGZDFMZTWQ2LK", first_attempt + chrono::Duration::seconds(1)).await.unwrap();

        assert!(!repo.confirm(user_id, first_attempt, 7).await.unwrap(), "a confirm for a replaced attempt must not confirm the new secret");

        assert!(!repo.get(user_id).await.unwrap().unwrap().confirmed);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn of_two_parallel_confirms_only_one_wins(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = std::sync::Arc::new(PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string()));
        let created_at = enrolled(&repo, user_id).await;

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let repo = repo.clone();
                tokio::spawn(async move { repo.confirm(user_id, created_at, 7).await.unwrap() })
            })
            .collect();
        let winners = futures_util::future::join_all(handles).await.into_iter().filter(|r| *r.as_ref().unwrap()).count();

        assert_eq!(winners, 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn set_last_used_step_persists(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        enrolled(&repo, user_id).await;

        let advanced = repo.set_last_used_step(user_id, 42).await.unwrap();

        assert!(advanced);
        assert_eq!(repo.get(user_id).await.unwrap().unwrap().last_used_step, Some(42));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn set_last_used_step_is_a_compare_and_swap_not_an_unconditional_write(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        enrolled(&repo, user_id).await;

        let first = repo.set_last_used_step(user_id, 100).await.unwrap();
        let second = repo.set_last_used_step(user_id, 100).await.unwrap();
        let backward = repo.set_last_used_step(user_id, 99).await.unwrap();

        assert!(first, "the first caller to advance the step must succeed");
        assert!(!second, "a second caller presenting the same step must not also succeed");
        assert!(!backward, "advancing to an older step than what's already stored must fail");
        assert_eq!(repo.get(user_id).await.unwrap().unwrap().last_used_step, Some(100));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn delete_removes_the_credential(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        enrolled(&repo, user_id).await;

        repo.delete(user_id).await.unwrap();

        assert_eq!(repo.get(user_id).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_secret_is_never_stored_in_plaintext(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresTotpCredentialRepository::new(pool, "jwt-secret".to_string());
        enrolled(&repo, user_id).await;

        let row: (Vec<u8>,) = sqlx::query_as("SELECT encrypted_secret FROM totp_credentials WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        let stored = String::from_utf8_lossy(&row.0);
        assert!(!stored.contains(SECRET));
    }
}
