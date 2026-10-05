use async_trait::async_trait;
use artiferris_domain::audit::AdminAuditRecord;
use artiferris_domain::error::DomainError;
use crate::error_ext::InfraErr;
use artiferris_domain::invitation::{UserInvitation, UserInvitationPort};
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresUserInvitationRepository {
    pool: PgPool,
}

impl PostgresUserInvitationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

pub(crate) async fn upsert_invitation<'e>(executor: impl sqlx::PgExecutor<'e>, invitation: &UserInvitation) -> Result<(), DomainError> {
    sqlx::query!(
        "INSERT INTO user_invitations (user_id, token_hash, expires_at) VALUES ($1, $2, $3) \
         ON CONFLICT (user_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, expires_at = EXCLUDED.expires_at",
        invitation.user_id,
        invitation.token_hash,
        invitation.expires_at,
    )
    .execute(executor)
    .await
    .infra_err()?;
    Ok(())
}

#[async_trait]
impl UserInvitationPort for PostgresUserInvitationRepository {
    async fn invitation_expiries(&self, user_ids: &[Uuid]) -> Result<std::collections::HashMap<Uuid, chrono::DateTime<chrono::Utc>>, DomainError> {
        if user_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as("SELECT user_id, expires_at FROM user_invitations WHERE user_id = ANY($1)").bind(user_ids).fetch_all(&self.pool).await.infra_err()?;
        Ok(rows.into_iter().collect())
    }

    async fn upsert(&self, invitation: &UserInvitation, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        upsert_invitation(&mut *tx, invitation).await?;
        crate::postgres::event_publisher::insert_admin_audit(&mut tx, audit).await?;
        tx.commit().await.infra_err()?;
        Ok(())
    }

    async fn find_by_token_hash(&self, token_hash: &str) -> Result<Option<UserInvitation>, DomainError> {
        let row = sqlx::query!("SELECT user_id, token_hash, expires_at FROM user_invitations WHERE token_hash = $1", token_hash)
            .fetch_optional(&self.pool)
            .await
            .infra_err()?;
        Ok(row.map(|r| UserInvitation { user_id: r.user_id, token_hash: r.token_hash, expires_at: r.expires_at }))
    }

    async fn find_by_user_id(&self, user_id: Uuid) -> Result<Option<UserInvitation>, DomainError> {
        let row = sqlx::query!("SELECT user_id, token_hash, expires_at FROM user_invitations WHERE user_id = $1", user_id)
            .fetch_optional(&self.pool)
            .await
            .infra_err()?;
        Ok(row.map(|r| UserInvitation { user_id: r.user_id, token_hash: r.token_hash, expires_at: r.expires_at }))
    }

    async fn list_pending_user_ids(&self, user_ids: &[Uuid]) -> Result<std::collections::HashSet<Uuid>, DomainError> {
        if user_ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let rows = sqlx::query!("SELECT user_id FROM user_invitations WHERE user_id = ANY($1)", user_ids)
            .fetch_all(&self.pool)
            .await
            .infra_err()?;
        Ok(rows.into_iter().map(|r| r.user_id).collect())
    }

    async fn delete(&self, user_id: Uuid) -> Result<(), DomainError> {
        sqlx::query!("DELETE FROM user_invitations WHERE user_id = $1", user_id).execute(&self.pool).await.infra_err()?;
        Ok(())
    }

    async fn redeem(&self, token_hash: &str) -> Result<Option<UserInvitation>, DomainError> {
        let row = sqlx::query!(
            "DELETE FROM user_invitations WHERE token_hash = $1 AND expires_at > now() RETURNING user_id, token_hash, expires_at",
            token_hash
        )
        .fetch_optional(&self.pool)
        .await
        .infra_err()?;
        Ok(row.map(|r| UserInvitation { user_id: r.user_id, token_hash: r.token_hash, expires_at: r.expires_at }))
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
            username: Username::parse("invitee").unwrap(),
            password_hash: "unusable".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            created_at: chrono::Utc::now(),
            tokens_valid_after: chrono::Utc::now(),
            email: Some("invitee@example.com".to_string()),
        };
        users.insert(&user).await.unwrap();
        user.id
    }

    fn sample(user_id: Uuid) -> UserInvitation {
        UserInvitation { user_id, token_hash: "hash-a".to_string(), expires_at: chrono::Utc::now() + chrono::Duration::hours(24) }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn upsert_then_find_by_token_hash_round_trips(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresUserInvitationRepository::new(pool);
        repo.upsert(&sample(user_id), None).await.unwrap();

        let found = repo.find_by_token_hash("hash-a").await.unwrap().unwrap();
        assert_eq!(found.user_id, user_id);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn invitation_expiries_reads_the_expiry_of_those_who_have_an_invitation(pool: sqlx::PgPool) {
        let invited = seed_user(&pool).await;
        let repo = PostgresUserInvitationRepository::new(pool);
        let invitation = sample(invited);
        repo.upsert(&invitation, None).await.unwrap();

        let expiries = repo.invitation_expiries(&[invited, Uuid::new_v4()]).await.unwrap();

        assert_eq!(expiries.len(), 1);
        assert_eq!(expiries[&invited].timestamp(), invitation.expires_at.timestamp());
        assert!(repo.invitation_expiries(&[]).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn find_by_token_hash_returns_none_when_unknown(pool: sqlx::PgPool) {
        let repo = PostgresUserInvitationRepository::new(pool);
        assert_eq!(repo.find_by_token_hash("nope").await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_second_upsert_for_the_same_user_replaces_the_token_rather_than_inserting_a_row(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresUserInvitationRepository::new(pool);
        repo.upsert(&sample(user_id), None).await.unwrap();
        let reissued = UserInvitation { token_hash: "hash-b".to_string(), ..sample(user_id) };
        repo.upsert(&reissued, None).await.unwrap();

        assert_eq!(repo.find_by_token_hash("hash-a").await.unwrap(), None, "the old token must no longer resolve");
        assert_eq!(repo.find_by_token_hash("hash-b").await.unwrap().unwrap().user_id, user_id);
        assert_eq!(repo.find_by_user_id(user_id).await.unwrap().unwrap().token_hash, "hash-b");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn delete_removes_the_invitation(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresUserInvitationRepository::new(pool);
        repo.upsert(&sample(user_id), None).await.unwrap();

        repo.delete(user_id).await.unwrap();

        assert_eq!(repo.find_by_user_id(user_id).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn redeeming_returns_the_invitation_once_and_removes_it(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresUserInvitationRepository::new(pool);
        repo.upsert(&sample(user_id), None).await.unwrap();

        assert_eq!(repo.redeem("hash-a").await.unwrap().unwrap().user_id, user_id);

        assert_eq!(repo.redeem("hash-a").await.unwrap(), None);
        assert_eq!(repo.find_by_user_id(user_id).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn parallel_redemptions_of_one_token_let_exactly_one_through(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = std::sync::Arc::new(PostgresUserInvitationRepository::new(pool));
        repo.upsert(&sample(user_id), None).await.unwrap();

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let repo = repo.clone();
                tokio::spawn(async move { repo.redeem("hash-a").await.unwrap() })
            })
            .collect();
        let winners = futures_util::future::join_all(handles).await.into_iter().filter(|r| r.as_ref().unwrap().is_some()).count();

        assert_eq!(winners, 1);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_expired_invitation_cannot_be_redeemed(pool: sqlx::PgPool) {
        let user_id = seed_user(&pool).await;
        let repo = PostgresUserInvitationRepository::new(pool);
        repo.upsert(&UserInvitation { expires_at: chrono::Utc::now() - chrono::Duration::hours(1), ..sample(user_id) }, None).await.unwrap();

        assert_eq!(repo.redeem("hash-a").await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn deleting_an_invitation_that_does_not_exist_is_a_no_op(pool: sqlx::PgPool) {
        let repo = PostgresUserInvitationRepository::new(pool);
        repo.delete(Uuid::new_v4()).await.unwrap();
    }
}
