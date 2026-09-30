use async_trait::async_trait;
use artiferris_domain::audit::AdminAuditRecord;
use artiferris_domain::error::DomainError;
use artiferris_domain::sso::{IdentityProviderConfig, IdentityProviderRepositoryPort, LdapConfig, OidcConfig};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error_ext::InfraErr;
use crate::secret_box;

pub struct PostgresIdentityProviderRepository {
    pool: PgPool,
    /// Derives the AES-256 key for `secret_box` — never stored, never logged.
    secrets_encryption_key: String,
}

impl PostgresIdentityProviderRepository {
    pub fn new(pool: PgPool, secrets_encryption_key: String) -> Self {
        Self { pool, secrets_encryption_key }
    }
}

/// The JSONB row shape — `bind_password_encrypted` is `secret_box::seal_packed`'s output, never the plaintext password.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredConfig {
    Ldap {
        server_url: String,
        bind_dn: String,
        bind_password_encrypted: String,
        user_search_base: String,
        user_search_filter: String,
        email_attribute: String,
    },
    Oidc {
        issuer_url: String,
        client_id: String,
        client_secret_encrypted: String,
    },
}

impl StoredConfig {
    fn from_domain(config: &IdentityProviderConfig, secrets_encryption_key: &str) -> Self {
        match config {
            IdentityProviderConfig::Ldap(ldap) => StoredConfig::Ldap {
                server_url: ldap.server_url.clone(),
                bind_dn: ldap.bind_dn.clone(),
                bind_password_encrypted: secret_box::seal_packed(&ldap.bind_password, secrets_encryption_key, secret_box::LDAP_BIND_PASSWORD),
                user_search_base: ldap.user_search_base.clone(),
                user_search_filter: ldap.user_search_filter.clone(),
                email_attribute: ldap.email_attribute.clone(),
            },
            IdentityProviderConfig::Oidc(oidc) => StoredConfig::Oidc {
                issuer_url: oidc.issuer_url.clone(),
                client_id: oidc.client_id.clone(),
                client_secret_encrypted: secret_box::seal_packed(&oidc.client_secret, secrets_encryption_key, secret_box::OIDC_CLIENT_SECRET),
            },
        }
    }

    fn into_domain(self, secrets_encryption_key: &str) -> Result<IdentityProviderConfig, DomainError> {
        match self {
            StoredConfig::Ldap { server_url, bind_dn, bind_password_encrypted, user_search_base, user_search_filter, email_attribute } => {
                Ok(IdentityProviderConfig::Ldap(LdapConfig {
                    server_url,
                    bind_dn,
                    bind_password: secret_box::open_packed(&bind_password_encrypted, secrets_encryption_key, secret_box::LDAP_BIND_PASSWORD)?,
                    user_search_base,
                    user_search_filter,
                    email_attribute,
                }))
            }
            StoredConfig::Oidc { issuer_url, client_id, client_secret_encrypted } => {
                Ok(IdentityProviderConfig::Oidc(OidcConfig {
                    issuer_url,
                    client_id,
                    client_secret: secret_box::open_packed(&client_secret_encrypted, secrets_encryption_key, secret_box::OIDC_CLIENT_SECRET)?,
                }))
            }
        }
    }
}

#[async_trait]
impl IdentityProviderRepositoryPort for PostgresIdentityProviderRepository {
    async fn get(&self, organization_id: Uuid) -> Result<Option<IdentityProviderConfig>, DomainError> {
        let row = sqlx::query!("SELECT config FROM organization_identity_providers WHERE organization_id = $1", organization_id)
            .fetch_optional(&self.pool)
            .await
            .infra_err()?;
        let Some(row) = row else {
            return Ok(None);
        };
        let stored: StoredConfig = serde_json::from_value(row.config).map_err(|e| DomainError::Infrastructure(e.to_string()))?;
        let kind = match &stored {
            StoredConfig::Ldap { .. } => "LDAP bind password",
            StoredConfig::Oidc { .. } => "OIDC client secret",
        };
        let config = stored
            .into_domain(&self.secrets_encryption_key)
            .inspect_err(|e| tracing::error!(%organization_id, "the stored {kind} cannot be read, SSO login is down until an admin enters it again: {e}"))?;
        Ok(Some(config))
    }

    async fn set(&self, organization_id: Uuid, config: &IdentityProviderConfig, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError> {
        let stored = StoredConfig::from_domain(config, &self.secrets_encryption_key);
        let json = serde_json::to_value(&stored).map_err(|e| DomainError::Infrastructure(e.to_string()))?;
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!(
            "INSERT INTO organization_identity_providers (organization_id, config, updated_at) VALUES ($1, $2, now()) \
             ON CONFLICT (organization_id) DO UPDATE SET config = EXCLUDED.config, updated_at = now()",
            organization_id,
            json,
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

    async fn clear(&self, organization_id: Uuid, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError> {
        let mut tx = self.pool.begin().await.infra_err()?;
        sqlx::query!("DELETE FROM organization_identity_providers WHERE organization_id = $1", organization_id)
            .execute(&mut *tx)
            .await
            .infra_err()?;
        if let Some(audit) = audit {
            crate::postgres::event_publisher::insert_admin_event(&mut *tx, &audit.event, audit.actor_id).await.infra_err()?;
        }
        tx.commit().await.infra_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IdentityProviderConfig {
        IdentityProviderConfig::Ldap(LdapConfig {
            server_url: "ldap://dc.corp.example:389".to_string(),
            bind_dn: "cn=service,dc=corp,dc=example".to_string(),
            bind_password: "s3cret!".to_string(),
            user_search_base: "ou=people,dc=corp,dc=example".to_string(),
            user_search_filter: "(uid={username})".to_string(),
            email_attribute: "mail".to_string(),
        })
    }

    /// `organization_id` is a foreign key, so tests using a fresh id need a matching row.
    async fn insert_organization(pool: &PgPool, organization_id: Uuid) {
        sqlx::query!(
            "INSERT INTO organizations (id, slug, display_name, is_public) VALUES ($1, $2, 'Test Org', false)",
            organization_id,
            organization_id.to_string(),
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn get_returns_none_when_never_configured(pool: PgPool) {
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());
        assert_eq!(repo.get(Uuid::new_v4()).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn set_then_get_round_trips_including_the_password(pool: PgPool) {
        let organization_id = Uuid::new_v4();
        insert_organization(&pool, organization_id).await;
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());

        repo.set(organization_id, &sample(), None).await.unwrap();

        assert_eq!(repo.get(organization_id).await.unwrap(), Some(sample()));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_second_set_overwrites_the_first_rather_than_inserting_a_row(pool: PgPool) {
        let organization_id = Uuid::new_v4();
        insert_organization(&pool, organization_id).await;
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());
        repo.set(organization_id, &sample(), None).await.unwrap();

        let IdentityProviderConfig::Ldap(mut updated) = sample() else { unreachable!("sample() is always Ldap") };
        updated.server_url = "ldaps://dc2.corp.example:636".to_string();
        repo.set(organization_id, &IdentityProviderConfig::Ldap(updated), None).await.unwrap();

        let count: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM organization_identity_providers WHERE organization_id = $1", organization_id)
            .fetch_one(&repo.pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(count, 1);
        let IdentityProviderConfig::Ldap(found) = repo.get(organization_id).await.unwrap().unwrap() else { unreachable!("stored config is Ldap") };
        assert_eq!(found.server_url, "ldaps://dc2.corp.example:636");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn clearing_an_unconfigured_organization_is_a_no_op(pool: PgPool) {
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());
        repo.clear(Uuid::new_v4(), None).await.unwrap();
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn clearing_removes_the_configuration(pool: PgPool) {
        let organization_id = Uuid::new_v4();
        insert_organization(&pool, organization_id).await;
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());
        repo.set(organization_id, &sample(), None).await.unwrap();

        repo.clear(organization_id, None).await.unwrap();

        assert_eq!(repo.get(organization_id).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_bind_password_is_never_stored_in_plaintext(pool: PgPool) {
        let organization_id = Uuid::new_v4();
        insert_organization(&pool, organization_id).await;
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());
        repo.set(organization_id, &sample(), None).await.unwrap();

        let row: (serde_json::Value,) = sqlx::query_as("SELECT config FROM organization_identity_providers WHERE organization_id = $1")
            .bind(organization_id)
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert!(!row.0.to_string().contains("s3cret!"), "the plaintext bind password must never appear in the stored JSON");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn decrypting_with_the_wrong_secrets_encryption_key_fails_loudly(pool: PgPool) {
        let organization_id = Uuid::new_v4();
        insert_organization(&pool, organization_id).await;
        let write_repo = PostgresIdentityProviderRepository::new(pool.clone(), "jwt-secret-a".to_string());
        write_repo.set(organization_id, &sample(), None).await.unwrap();

        let read_repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret-b".to_string());
        assert!(read_repo.get(organization_id).await.is_err(), "a mismatched key must be a hard error, not a silent garbled decrypt");
    }

    fn sample_oidc() -> IdentityProviderConfig {
        IdentityProviderConfig::Oidc(OidcConfig {
            issuer_url: "https://accounts.example.com".to_string(),
            client_id: "artiferris".to_string(),
            client_secret: "s3cret!".to_string(),
        })
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn set_then_get_round_trips_an_oidc_config_including_the_client_secret(pool: PgPool) {
        let organization_id = Uuid::new_v4();
        insert_organization(&pool, organization_id).await;
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());

        repo.set(organization_id, &sample_oidc(), None).await.unwrap();

        assert_eq!(repo.get(organization_id).await.unwrap(), Some(sample_oidc()));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_oidc_client_secret_is_never_stored_in_plaintext(pool: PgPool) {
        let organization_id = Uuid::new_v4();
        insert_organization(&pool, organization_id).await;
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());
        repo.set(organization_id, &sample_oidc(), None).await.unwrap();

        let row: (serde_json::Value,) = sqlx::query_as("SELECT config FROM organization_identity_providers WHERE organization_id = $1")
            .bind(organization_id)
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert!(!row.0.to_string().contains("s3cret!"), "the plaintext client secret must never appear in the stored JSON");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn switching_from_ldap_to_oidc_replaces_the_configuration_rather_than_merging_it(pool: PgPool) {
        let organization_id = Uuid::new_v4();
        insert_organization(&pool, organization_id).await;
        let repo = PostgresIdentityProviderRepository::new(pool, "jwt-secret".to_string());
        repo.set(organization_id, &sample(), None).await.unwrap(); // sample() is the existing LDAP fixture already in this file

        repo.set(organization_id, &sample_oidc(), None).await.unwrap();

        assert_eq!(repo.get(organization_id).await.unwrap(), Some(sample_oidc()));
    }
}
