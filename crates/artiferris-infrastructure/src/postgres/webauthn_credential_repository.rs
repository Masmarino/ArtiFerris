use async_trait::async_trait;
use artiferris_domain::audit::SecurityAuditRecord;
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use artiferris_domain::webauthn::{WebauthnCredential, WebauthnCredentialPort};
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresWebauthnCredentialRepository {
    pool: PgPool,
}

impl PostgresWebauthnCredentialRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WebauthnCredentialPort for PostgresWebauthnCredentialRepository {
    async fn holders_among(&self, user_ids: &[Uuid]) -> Result<std::collections::HashSet<Uuid>, DomainError> {
        if user_ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT DISTINCT user_id FROM webauthn_credentials WHERE user_id = ANY($1)").bind(user_ids).fetch_all(&self.pool).await.infra_err()?;
        Ok(rows.into_iter().map(|(user_id,)| user_id).collect())
    }

    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<WebauthnCredential>, DomainError> {
        let rows = sqlx::query!("SELECT id, user_id, name, passkey_data, created_at, last_used_at FROM webauthn_credentials WHERE user_id = $1 ORDER BY created_at", user_id)
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        Ok(rows.into_iter().map(|r| WebauthnCredential { id: r.id, user_id: r.user_id, name: r.name, passkey_data: r.passkey_data, created_at: r.created_at, last_used_at: r.last_used_at }).collect())
    }

    async fn insert(&self, credential: &WebauthnCredential, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!(
            "INSERT INTO webauthn_credentials (id, user_id, name, passkey_data, created_at) VALUES ($1, $2, $3, $4, $5)",
            credential.id,
            credential.user_id,
            credential.name,
            credential.passkey_data,
            credential.created_at,
        )
        .execute(&mut *tx)
        .await
        .infra_err()?;
        crate::postgres::event_publisher::insert_security_audit(&mut tx, audit).await?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn update_passkey_data(&self, id: Uuid, passkey_data: Vec<u8>) -> Result<(), DomainError> {
        sqlx::query!("UPDATE webauthn_credentials SET passkey_data = $1 WHERE id = $2", passkey_data, id)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn mark_used(&self, id: Uuid, at: chrono::DateTime<chrono::Utc>) -> Result<(), DomainError> {
        sqlx::query!("UPDATE webauthn_credentials SET last_used_at = $1 WHERE id = $2", at, id)
            .execute(&self.pool)
            .await
            .infra_err()?;
        Ok(())
    }

    async fn delete(&self, id: Uuid, user_id: Uuid, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("DELETE FROM webauthn_credentials WHERE id = $1 AND user_id = $2", id, user_id).execute(&mut *tx).await.infra_err()?;
        crate::postgres::event_publisher::insert_security_audit(&mut tx, audit).await?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn count_for_user(&self, user_id: Uuid) -> Result<i64, DomainError> {
        let count: i64 = sqlx::query_scalar!("SELECT count(*) FROM webauthn_credentials WHERE user_id = $1", user_id)
            .fetch_one(&self.pool)
            .await
            .infra_err()?
            .unwrap_or(0);
        Ok(count)
    }

    async fn delete_all_for_user(&self, user_id: Uuid) -> Result<(), DomainError> {
        sqlx::query("DELETE FROM webauthn_credentials WHERE user_id = $1").bind(user_id).execute(&self.pool).await.infra_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::user::{User, UserRepositoryPort, Username};

    use crate::postgres::user_repository::PostgresUserRepository;

    async fn seed_user(pool: &PgPool, username: &str) -> Uuid {
        let users = PostgresUserRepository::new(pool.clone());
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse(username).unwrap(),
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

    fn sample(user_id: Uuid) -> WebauthnCredential {
        WebauthnCredential { id: Uuid::new_v4(), user_id, name: "MacBook".to_string(), passkey_data: b"opaque-passkey-bytes".to_vec(), created_at: chrono::Utc::now(), last_used_at: None }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn insert_then_list_for_user_round_trips(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool, "florian").await;
        let repo = PostgresWebauthnCredentialRepository::new(pool);
        let credential = sample(user_id);
        repo.insert(&credential, None).await.unwrap();

        let listed = repo.list_for_user(user_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        let found = listed.into_iter().next().unwrap();
        assert_eq!(
            WebauthnCredential { created_at: found.created_at, ..credential.clone() },
            found
        );
        assert_eq!(found.created_at.timestamp_micros(), credential.created_at.timestamp_micros());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn deleting_every_passkey_of_an_account_leaves_the_others(pool: sqlx::PgPool) {
        let florian = seed_user(&pool, "florian").await;
        let alice = seed_user(&pool, "alice").await;
        let repo = PostgresWebauthnCredentialRepository::new(pool);
        repo.insert(&sample(florian), None).await.unwrap();
        repo.insert(&sample(florian), None).await.unwrap();
        repo.insert(&sample(alice), None).await.unwrap();

        repo.delete_all_for_user(florian).await.unwrap();

        assert_eq!(repo.count_for_user(florian).await.unwrap(), 0);
        assert_eq!(repo.count_for_user(alice).await.unwrap(), 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_passkey_records_when_it_was_last_used(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool, "florian").await;
        let repo = PostgresWebauthnCredentialRepository::new(pool);
        let credential = sample(user_id);
        repo.insert(&credential, None).await.unwrap();
        assert_eq!(repo.list_for_user(user_id).await.unwrap()[0].last_used_at, None);

        let at = chrono::Utc::now();
        repo.mark_used(credential.id, at).await.unwrap();

        let used = repo.list_for_user(user_id).await.unwrap()[0].last_used_at.unwrap();
        assert_eq!(used.timestamp_micros(), at.timestamp_micros());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_credential_and_its_audit_entry_are_stored_together_or_not_at_all(pool: sqlx::PgPool) {
        use artiferris_domain::audit::{SecurityAuditRecord, SecurityEvent};
        let user_id = seed_user(&pool, "florian").await;
        let repo = PostgresWebauthnCredentialRepository::new(pool.clone());
        let credential = sample(user_id);
        let audit = SecurityAuditRecord { event: SecurityEvent::PasskeyAdded { user_id, organization_id: Uuid::nil(), passkey_id: credential.id }, actor_id: Some(user_id) };
        repo.insert(&credential, Some(&audit)).await.unwrap();
        let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE event_type = 'PasskeyAdded'").fetch_one(&pool).await.unwrap();
        assert_eq!(recorded, 1);
        sqlx::query("ALTER TABLE domain_events RENAME TO domain_events_gone").execute(&pool).await.unwrap();

        let second = sample(user_id);
        let refused = repo.insert(&second, Some(&audit)).await;

        assert!(refused.is_err());
        assert_eq!(repo.count_for_user(user_id).await.unwrap(), 1, "the second credential was not stored without its audit entry");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn list_for_user_returns_empty_when_none_registered(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool, "florian").await;
        let repo = PostgresWebauthnCredentialRepository::new(pool);
        assert_eq!(repo.list_for_user(user_id).await.unwrap(), vec![]);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn update_passkey_data_persists(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool, "florian").await;
        let repo = PostgresWebauthnCredentialRepository::new(pool);
        let credential = sample(user_id);
        repo.insert(&credential, None).await.unwrap();

        repo.update_passkey_data(credential.id, b"updated-bytes".to_vec()).await.unwrap();

        let listed = repo.list_for_user(user_id).await.unwrap();
        assert_eq!(listed[0].passkey_data, b"updated-bytes".to_vec());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn delete_only_removes_the_owners_credential(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool, "florian").await;
        let other_user_id = seed_user(&pool, "other-user").await;
        let repo = PostgresWebauthnCredentialRepository::new(pool);
        let credential = sample(user_id);
        repo.insert(&credential, None).await.unwrap();

        repo.delete(credential.id, other_user_id, None).await.unwrap();
        assert_eq!(repo.count_for_user(user_id).await.unwrap(), 1, "deleting with the wrong user_id must not remove the credential");

        repo.delete(credential.id, user_id, None).await.unwrap();
        assert_eq!(repo.count_for_user(user_id).await.unwrap(), 0);
    }
}
