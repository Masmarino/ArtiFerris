use async_trait::async_trait;
use artiferris_domain::configuration_import::{ConfigurationImportPort, ImportBatch};
use artiferris_domain::error::DomainError;
use sqlx::PgPool;

use crate::error_ext::InfraErr;
use crate::postgres::event_publisher::insert_admin_event;
use crate::postgres::package_repository_store::PostgresPackageRepositoryStore;
use crate::postgres::permission_store;
use crate::postgres::system_settings_repository::update_settings;
use crate::postgres::user_invitation_repository::upsert_invitation;
use crate::postgres::user_repository::insert_user_row;

/// Runs an import in one transaction, through the same write paths the individual stores use.
pub struct PostgresConfigurationImport {
    pool: PgPool,
    repositories: PostgresPackageRepositoryStore,
}

impl PostgresConfigurationImport {
    pub fn new(pool: PgPool, secrets_encryption_key: String) -> Self {
        Self { repositories: PostgresPackageRepositoryStore::new(pool.clone(), secrets_encryption_key), pool }
    }
}

#[async_trait]
impl ConfigurationImportPort for PostgresConfigurationImport {
    async fn apply(&self, batch: &ImportBatch) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        for user in &batch.users {
            insert_user_row(&mut *tx, user, false).await?;
        }
        for stream in &batch.repository_streams {
            self.repositories.append_in_tx(&mut tx, stream.repository_id, stream.expected_version, stream.events.clone(), batch.actor_id).await.infra_err()?;
        }
        for stream in &batch.permission_streams {
            permission_store::append_in_tx(&mut tx, stream.user_id, stream.repository_id, 0, stream.events.clone(), batch.actor_id).await.infra_err()?;
        }
        for invitation in &batch.invitations {
            upsert_invitation(&mut *tx, invitation).await?;
        }
        if let Some(settings) = &batch.system_settings {
            update_settings(&mut *tx, batch.organization_id, settings).await?;
        }
        if let Some(audit) = &batch.audit {
            insert_admin_event(&mut *tx, &audit.event, audit.actor_id).await.infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::audit::{AdminAuditEvent, AdminAuditRecord};
    use artiferris_domain::configuration_import::{PermissionStream, RepositoryStream};
    use artiferris_domain::invitation::UserInvitation;
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use artiferris_domain::package_repository::{PackageRepositoryEvent, RepositoryFormat, RepositoryType};
    use artiferris_domain::permission::{PermissionEvent, Role};
    use artiferris_domain::system_settings::SystemSettings;
    use artiferris_domain::user::{User, Username};
    use chrono::Utc;
    use uuid::Uuid;

    const KEY: &str = "test-secrets-encryption-key-of-enough-length";

    fn user(name: &str) -> User {
        User {
            id: Uuid::new_v4(),
            username: Username::parse(name).unwrap(),
            password_hash: "unusable".to_string(),
            is_super_admin: false,
            is_organization_admin: false,
            organization_id: PUBLIC_ORGANIZATION_ID,
            created_at: Utc::now(),
            tokens_valid_after: Utc::now(),
            email: Some(format!("{name}@example.com")),
        }
    }

    fn created(id: Uuid, name: &str, repo_type: RepositoryType) -> RepositoryStream {
        RepositoryStream {
            repository_id: id,
            expected_version: 0,
            events: vec![PackageRepositoryEvent::Created {
                repository_id: id,
                organization_id: PUBLIC_ORGANIZATION_ID,
                name: name.to_string(),
                format: RepositoryFormat::Npm,
                repo_type,
                remote_url: None,
                remote_username: None,
                remote_password: None,
            }],
        }
    }

    async fn counts(pool: &PgPool) -> (i64, i64, i64, i64) {
        let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(pool).await.unwrap();
        let repositories: i64 = sqlx::query_scalar("SELECT count(*) FROM package_repository_projections").fetch_one(pool).await.unwrap();
        let events: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events").fetch_one(pool).await.unwrap();
        let invitations: i64 = sqlx::query_scalar("SELECT count(*) FROM user_invitations").fetch_one(pool).await.unwrap();
        (users, repositories, events, invitations)
    }

    fn full_batch(actor_id: Uuid) -> (ImportBatch, Uuid, Uuid) {
        let member = user("member");
        let (group_id, hosted_id) = (Uuid::new_v4(), Uuid::new_v4());
        let mut group = created(group_id, "my-group", RepositoryType::Group);
        group.events.push(PackageRepositoryEvent::QuotaSet { repository_id: Uuid::nil(), quota_bytes: Some(1024) });
        let batch = ImportBatch {
            actor_id,
            organization_id: PUBLIC_ORGANIZATION_ID,
            invitations: vec![UserInvitation { user_id: member.id, token_hash: "hash".to_string(), expires_at: Utc::now() + chrono::Duration::hours(24) }],
            permission_streams: vec![PermissionStream { user_id: member.id, repository_id: hosted_id, events: vec![PermissionEvent::Granted { user_id: member.id, repository_id: hosted_id, role: Role::Write }] }],
            users: vec![member],
            repository_streams: vec![
                group,
                created(hosted_id, "my-hosted", RepositoryType::Hosted),
                RepositoryStream { repository_id: group_id, expected_version: 2, events: vec![PackageRepositoryEvent::GroupMemberAdded { repository_id: Uuid::nil(), member_repository_id: hosted_id, position: 0 }] },
            ],
            system_settings: Some(SystemSettings { max_login_attempts: 7, ..SystemSettings::defaults() }),
            audit: Some(AdminAuditRecord { event: AdminAuditEvent::ConfigurationImported { users_created: 1, repositories_created: 2, permissions_granted: 1, failures: 0 }, actor_id: Some(actor_id) }),
        };
        (batch, group_id, hosted_id)
    }

    #[sqlx::test]
    async fn a_valid_batch_lands_completely(pool: PgPool) {
        let actor_id = Uuid::new_v4();
        let (batch, group_id, hosted_id) = full_batch(actor_id);
        let member_id = batch.users[0].id;

        PostgresConfigurationImport::new(pool.clone(), KEY.to_string()).apply(&batch).await.unwrap();

        let store = PostgresPackageRepositoryStore::new(pool.clone(), KEY.to_string());
        use artiferris_domain::package_repository::PackageRepositoryQueryPort;
        let group = store.find_by_id(group_id).await.unwrap().unwrap();
        assert_eq!((group.group_members, group.quota_bytes), (vec![hosted_id], Some(1024)));
        let granted: i64 = sqlx::query_scalar("SELECT count(*) FROM permission_projections WHERE user_id = $1 AND repository_id = $2").bind(member_id).bind(hosted_id).fetch_one(&pool).await.unwrap();
        assert_eq!(granted, 1);
        let attempts: i32 = sqlx::query_scalar("SELECT max_login_attempts FROM system_settings WHERE organization_id = $1").bind(PUBLIC_ORGANIZATION_ID).fetch_one(&pool).await.unwrap();
        assert_eq!(attempts, 7);
        let (users, _, _, invitations) = counts(&pool).await;
        assert_eq!((users, invitations), (1, 1));
        let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE aggregate_type = 'Admin' AND event_type = 'ConfigurationImported'").fetch_one(&pool).await.unwrap();
        assert_eq!(recorded, 1);
    }

    #[sqlx::test]
    async fn a_failure_late_in_the_batch_rolls_back_everything_before_it(pool: PgPool) {
        let actor_id = Uuid::new_v4();
        let (mut batch, _, _) = full_batch(actor_id);
        batch.repository_streams.push(created(Uuid::new_v4(), "my-hosted", RepositoryType::Hosted));
        let before = counts(&pool).await;

        let err = PostgresConfigurationImport::new(pool.clone(), KEY.to_string()).apply(&batch).await.unwrap_err();

        assert!(matches!(err, DomainError::Infrastructure(_)), "{err:?}");
        assert_eq!(counts(&pool).await, before, "not one row of the failed import remains");
        let settings: i64 = sqlx::query_scalar("SELECT count(*) FROM system_settings WHERE max_login_attempts = 7").fetch_one(&pool).await.unwrap();
        assert_eq!(settings, 0);
        batch.repository_streams.pop();
        PostgresConfigurationImport::new(pool.clone(), KEY.to_string()).apply(&batch).await.unwrap();
        assert_eq!(counts(&pool).await.0, before.0 + 1);
    }

    #[sqlx::test]
    async fn a_failing_audit_row_rolls_back_the_import(pool: PgPool) {
        let (batch, _, _) = full_batch(Uuid::new_v4());
        let before = counts(&pool).await;
        sqlx::query("ALTER TABLE domain_events ADD CONSTRAINT no_admin_events CHECK (aggregate_type <> 'Admin')").execute(&pool).await.unwrap();

        let err = PostgresConfigurationImport::new(pool.clone(), KEY.to_string()).apply(&batch).await.unwrap_err();

        assert!(matches!(err, DomainError::Infrastructure(_)), "{err:?}");
        assert_eq!(counts(&pool).await, before);
    }
}
