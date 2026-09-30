use async_trait::async_trait;
use artiferris_application::use_cases::mfa::verify_backup_code;
use artiferris_domain::audit::SecurityAuditRecord;
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use artiferris_domain::mfa::BackupCodePort;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresBackupCodeRepository {
    pool: PgPool,
}

impl PostgresBackupCodeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BackupCodePort for PostgresBackupCodeRepository {
    async fn replace_all(&self, user_id: Uuid, code_hashes: &[String], audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("DELETE FROM mfa_backup_codes WHERE user_id = $1", user_id).execute(&mut *tx).await.infra_err()?;
        if !code_hashes.is_empty() {
            sqlx::query!(
                "INSERT INTO mfa_backup_codes (user_id, code_hash) SELECT $1, * FROM UNNEST($2::text[])",
                user_id,
                code_hashes
            )
            .execute(&mut *tx)
            .await
            .infra_err()?;
        }
        crate::postgres::event_publisher::insert_security_audit(&mut tx, audit).await?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn try_consume(&self, user_id: Uuid, plaintext_code: &str) -> Result<bool, DomainError> {
        let candidates = sqlx::query!("SELECT id, code_hash FROM mfa_backup_codes WHERE user_id = $1 AND used_at IS NULL", user_id)
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        let Some(matched) = candidates.iter().find(|row| verify_backup_code(plaintext_code, &row.code_hash)) else {
            return Ok(false);
        };
        // Re-guard with `used_at IS NULL` here too: a concurrent call could have consumed this exact
        // row between the SELECT above and this UPDATE, and only one of the two must win.
        let result = sqlx::query!("UPDATE mfa_backup_codes SET used_at = now() WHERE id = $1 AND used_at IS NULL", matched.id).execute(&self.pool).await.infra_err()?;
        Ok(result.rows_affected() > 0)
    }

    async fn count_unused(&self, user_id: Uuid) -> Result<i64, DomainError> {
        let count: i64 = sqlx::query_scalar!("SELECT count(*) FROM mfa_backup_codes WHERE user_id = $1 AND used_at IS NULL", user_id)
            .fetch_one(&self.pool)
            .await
            .infra_err()?
            .unwrap_or(0);
        Ok(count)
    }

    async fn delete_all(&self, user_id: Uuid) -> Result<(), DomainError> {
        sqlx::query!("DELETE FROM mfa_backup_codes WHERE user_id = $1", user_id).execute(&self.pool).await.infra_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_application::use_cases::mfa::hash_backup_code;
    use artiferris_domain::user::{User, UserRepositoryPort, Username};
    use sha2::Digest;

    use crate::postgres::user_repository::PostgresUserRepository;

    async fn seed_user(pool: &PgPool) -> Uuid {
        let users = PostgresUserRepository::new(pool.clone());
        let username = format!("mfauser{}", &Uuid::new_v4().simple().to_string()[..8]);
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse(&username).unwrap(),
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

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn replace_all_then_try_consume_finds_the_new_codes(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresBackupCodeRepository::new(pool);
        repo.replace_all(user_id, &[hash_backup_code("code-a"), hash_backup_code("code-b")], None).await.unwrap();

        assert!(!repo.try_consume(user_id, "code-c").await.unwrap(), "an unknown code must not consume");
        assert_eq!(repo.count_unused(user_id).await.unwrap(), 2);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_second_replace_all_wipes_the_first_set(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresBackupCodeRepository::new(pool);
        repo.replace_all(user_id, &[hash_backup_code("code-a")], None).await.unwrap();
        repo.replace_all(user_id, &[hash_backup_code("code-b")], None).await.unwrap();

        assert!(!repo.try_consume(user_id, "code-a").await.unwrap(), "the old code must no longer be valid");
        assert_eq!(repo.count_unused(user_id).await.unwrap(), 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn try_consume_makes_the_code_single_use(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresBackupCodeRepository::new(pool);
        repo.replace_all(user_id, &[hash_backup_code("code-a")], None).await.unwrap();

        assert!(repo.try_consume(user_id, "code-a").await.unwrap());
        assert!(!repo.try_consume(user_id, "code-a").await.unwrap(), "the same code must not be usable twice");
        assert_eq!(repo.count_unused(user_id).await.unwrap(), 0);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn delete_all_removes_every_code(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresBackupCodeRepository::new(pool);
        repo.replace_all(user_id, &[hash_backup_code("code-a"), hash_backup_code("code-b")], None).await.unwrap();

        repo.delete_all(user_id).await.unwrap();

        assert_eq!(repo.count_unused(user_id).await.unwrap(), 0);
    }

    /// Codes issued before M-5 were stored as a bare `Sha256::digest(plaintext)` hex string with no
    /// `<salt>:` prefix. A user who enrolled before that fix and still holds an unused legacy code
    /// must still be able to consume it through the real Postgres-backed port, not just the fake
    /// used by the application-layer tests (Task 4 fix round 1, Critical finding).
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_legacy_pre_fix_unsalted_hash_still_verifies_and_consumes(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresBackupCodeRepository::new(pool);
        let legacy_hash = hex::encode(sha2::Sha256::digest(b"legacy-code"));
        assert!(!legacy_hash.contains(':'), "sanity: a legacy hash has no salt separator");
        repo.replace_all(user_id, &[legacy_hash], None).await.unwrap();

        assert!(repo.try_consume(user_id, "legacy-code").await.unwrap(), "a still-unused legacy backup code must keep working after the M-5 salting fix");
        assert!(!repo.try_consume(user_id, "legacy-code").await.unwrap(), "the same code must not be usable twice");
        assert_eq!(repo.count_unused(user_id).await.unwrap(), 0);
    }

    /// Two different users generating the identical plaintext code by coincidence must not collide
    /// or interfere with each other now that each stored hash carries its own salt (M-5).
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn two_users_with_the_same_plaintext_code_each_consume_independently(pool: sqlx::PgPool) {
        let user_a = seed_user(&pool).await;
        let user_b = seed_user(&pool).await;
        let repo = PostgresBackupCodeRepository::new(pool);
        repo.replace_all(user_a, &[hash_backup_code("shared-code")], None).await.unwrap();
        repo.replace_all(user_b, &[hash_backup_code("shared-code")], None).await.unwrap();

        assert!(repo.try_consume(user_a, "shared-code").await.unwrap());
        assert_eq!(repo.count_unused(user_a).await.unwrap(), 0);
        assert_eq!(repo.count_unused(user_b).await.unwrap(), 1, "consuming user A's code must not touch user B's identical plaintext code");
        assert!(repo.try_consume(user_b, "shared-code").await.unwrap());
    }
}
