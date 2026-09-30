use async_trait::async_trait;
use artiferris_domain::error::DomainError;
use artiferris_domain::user_preferences::{Language, UserPreferencesPort};
use crate::error_ext::InfraErr;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresUserPreferencesRepository {
    pool: PgPool,
}

impl PostgresUserPreferencesRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserPreferencesPort for PostgresUserPreferencesRepository {
    async fn language(&self, user_id: Uuid) -> Result<Option<Language>, DomainError> {
        let row = sqlx::query!("SELECT language FROM user_preferences WHERE user_id = $1", user_id)
            .fetch_optional(&self.pool)
            .await
            .infra_err()?;
        // A code stored by an older build that the interface no longer offers reads as "not chosen".
        Ok(row.and_then(|row| Language::parse(&row.language).ok()))
    }

    async fn set_language(&self, user_id: Uuid, language: Language) -> Result<(), DomainError> {
        sqlx::query!(
            "INSERT INTO user_preferences (user_id, language) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET language = EXCLUDED.language, updated_at = now()",
            user_id,
            language.as_str(),
        )
        .execute(&self.pool)
        .await
        .infra_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::user::{User, UserRepositoryPort, Username};
    use crate::postgres::user_repository::PostgresUserRepository;

    async fn a_user(pool: &PgPool) -> Uuid {
        let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let user = User {
            id: Uuid::new_v4(),
            username: Username::parse("alice").unwrap(),
            password_hash: "hash".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id,
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: None,
        };
        PostgresUserRepository::new(pool.clone()).insert(&user).await.unwrap();
        user.id
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_user_who_never_chose_has_no_language(pool: PgPool) {
        let id = a_user(&pool).await;

        assert_eq!(PostgresUserPreferencesRepository::new(pool).language(id).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_chosen_language_is_kept_and_can_change(pool: PgPool) {
        let id = a_user(&pool).await;
        let repo = PostgresUserPreferencesRepository::new(pool);

        repo.set_language(id, Language::parse("de").unwrap()).await.unwrap();
        assert_eq!(repo.language(id).await.unwrap().map(|l| l.as_str()), Some("de"));

        repo.set_language(id, Language::parse("fr").unwrap()).await.unwrap();
        assert_eq!(repo.language(id).await.unwrap().map(|l| l.as_str()), Some("fr"));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_language_the_interface_dropped_reads_as_not_chosen(pool: PgPool) {
        let id = a_user(&pool).await;
        sqlx::query("INSERT INTO user_preferences (user_id, language) VALUES ($1, 'xx')").bind(id).execute(&pool).await.unwrap();

        assert_eq!(PostgresUserPreferencesRepository::new(pool).language(id).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn deleting_the_user_removes_the_preference(pool: PgPool) {
        let id = a_user(&pool).await;
        let repo = PostgresUserPreferencesRepository::new(pool.clone());
        repo.set_language(id, Language::parse("it").unwrap()).await.unwrap();

        sqlx::query("DELETE FROM users WHERE id = $1").bind(id).execute(&pool).await.unwrap();

        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM user_preferences").fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0);
    }
}
