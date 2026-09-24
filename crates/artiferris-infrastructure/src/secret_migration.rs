//! Startup pass that moves every stored secret to the current `secret_box` format, and, after a
//! key rotation, from the previous key to the current one. Rows already in the current format are left alone.
//!
//! Nothing is rewritten unless `apply` is set: releases before the versioned format cannot open it, so until then the pass only
//! reports what it would change.

use artiferris_domain::error::DomainError;
use sqlx::PgPool;

use crate::error_ext::InfraErr;
use crate::secret_box;

/// Two servers starting together take turns instead of racing on the same rows.
const ADVISORY_LOCK_KEY: i64 = 0x0061_7274_6966_6572;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReencryptionReport {
    /// Values rewritten by this run.
    pub upgraded: usize,
    /// Values a run with `apply` would rewrite; only counted when it is off.
    pub pending: usize,
    /// Values that could not be read with the given keys. They stay as they are.
    pub failed: usize,
}

enum Outcome {
    Current,
    Upgraded,
    Pending,
}

impl ReencryptionReport {
    fn record(&mut self, context: &str, row: &str, outcome: Result<Outcome, DomainError>) {
        match outcome {
            Ok(Outcome::Upgraded) => self.upgraded += 1,
            Ok(Outcome::Pending) => self.pending += 1,
            Ok(Outcome::Current) => {}
            Err(e) => {
                self.failed += 1;
                tracing::error!("the stored {context} of {row} cannot be read with SECRETS_ENCRYPTION_KEY or SECRETS_ENCRYPTION_KEY_PREVIOUS, whatever needs it stays broken until an admin enters it again: {e}");
            }
        }
    }
}

/// Rows are read `FOR UPDATE`, so a settings write racing the pass either lands first and is seen, or waits for the commit; it is never overwritten by a re-sealed older value.
pub async fn reencrypt_secrets(pool: &PgPool, key: &str, previous_key: Option<&str>, apply: bool) -> Result<ReencryptionReport, DomainError> {
    let mut tx = pool.begin().await.infra_err()?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(ADVISORY_LOCK_KEY).execute(&mut *tx).await.infra_err()?;
    let mut report = ReencryptionReport::default();

    let totp: Vec<(uuid::Uuid, Vec<u8>, Vec<u8>)> =
        sqlx::query_as("SELECT user_id, encrypted_secret, secret_nonce FROM totp_credentials FOR UPDATE").fetch_all(&mut *tx).await.infra_err()?;
    for (user_id, ciphertext, nonce) in totp {
        let outcome = match secret_box::reseal(&ciphertext, &nonce, key, previous_key, secret_box::TOTP_SEED) {
            Ok(Some((ciphertext, nonce))) if apply => {
                sqlx::query("UPDATE totp_credentials SET encrypted_secret = $1, secret_nonce = $2 WHERE user_id = $3")
                    .bind(ciphertext)
                    .bind(nonce)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await
                    .infra_err()?;
                Ok(Outcome::Upgraded)
            }
            Ok(Some(_)) => Ok(Outcome::Pending),
            Ok(None) => Ok(Outcome::Current),
            Err(e) => Err(e),
        };
        report.record(secret_box::TOTP_SEED, &format!("user {user_id}"), outcome);
    }

    let smtp: Vec<(uuid::Uuid, Vec<u8>, Vec<u8>)> =
        sqlx::query_as("SELECT organization_id, encrypted_password, password_nonce FROM smtp_settings FOR UPDATE").fetch_all(&mut *tx).await.infra_err()?;
    for (organization_id, ciphertext, nonce) in smtp {
        let outcome = match secret_box::reseal(&ciphertext, &nonce, key, previous_key, secret_box::SMTP_PASSWORD) {
            Ok(Some((ciphertext, nonce))) if apply => {
                sqlx::query("UPDATE smtp_settings SET encrypted_password = $1, password_nonce = $2 WHERE organization_id = $3")
                    .bind(ciphertext)
                    .bind(nonce)
                    .bind(organization_id)
                    .execute(&mut *tx)
                    .await
                    .infra_err()?;
                Ok(Outcome::Upgraded)
            }
            Ok(Some(_)) => Ok(Outcome::Pending),
            Ok(None) => Ok(Outcome::Current),
            Err(e) => Err(e),
        };
        report.record(secret_box::SMTP_PASSWORD, &format!("organization {organization_id}"), outcome);
    }

    let providers: Vec<(uuid::Uuid, serde_json::Value)> =
        sqlx::query_as("SELECT organization_id, config FROM organization_identity_providers FOR UPDATE").fetch_all(&mut *tx).await.infra_err()?;
    for (organization_id, mut config) in providers {
        let (field, context) = match config.get("type").and_then(|t| t.as_str()) {
            Some("ldap") => ("bind_password_encrypted", secret_box::LDAP_BIND_PASSWORD),
            Some("oidc") => ("client_secret_encrypted", secret_box::OIDC_CLIENT_SECRET),
            _ => continue,
        };
        let Some(stored) = config.get(field).and_then(|v| v.as_str()).map(str::to_string) else { continue };
        let outcome = match secret_box::reseal_packed(&stored, key, previous_key, context) {
            Ok(Some(sealed)) if apply => {
                config[field] = serde_json::Value::String(sealed);
                sqlx::query("UPDATE organization_identity_providers SET config = $1 WHERE organization_id = $2").bind(&config).bind(organization_id).execute(&mut *tx).await.infra_err()?;
                Ok(Outcome::Upgraded)
            }
            Ok(Some(_)) => Ok(Outcome::Pending),
            Ok(None) => Ok(Outcome::Current),
            Err(e) => Err(e),
        };
        report.record(context, &format!("organization {organization_id}"), outcome);
    }

    let projections: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT id, remote_password FROM package_repository_projections WHERE remote_password IS NOT NULL FOR UPDATE").fetch_all(&mut *tx).await.infra_err()?;
    for (id, stored) in projections {
        let outcome = match secret_box::reseal_packed(&stored, key, previous_key, secret_box::PROXY_REMOTE_PASSWORD) {
            Ok(Some(sealed)) if apply => {
                sqlx::query("UPDATE package_repository_projections SET remote_password = $1 WHERE id = $2").bind(sealed).bind(id).execute(&mut *tx).await.infra_err()?;
                Ok(Outcome::Upgraded)
            }
            Ok(Some(_)) => Ok(Outcome::Pending),
            Ok(None) => Ok(Outcome::Current),
            Err(e) => Err(e),
        };
        report.record(secret_box::PROXY_REMOTE_PASSWORD, &format!("repository {id}"), outcome);
    }

    // The journal holds the same value in each repository's Created event and is replayed on every write to that repository.
    let journal: Vec<(uuid::Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT id, payload FROM domain_events WHERE aggregate_type = 'PackageRepository' AND event_type = 'Created' AND jsonb_typeof(payload -> 'remote_password') = 'string' FOR UPDATE",
    )
    .fetch_all(&mut *tx)
    .await
    .infra_err()?;
    for (id, mut payload) in journal {
        let Some(stored) = payload.get("remote_password").and_then(|v| v.as_str()).map(str::to_string) else { continue };
        let outcome = match secret_box::reseal_packed(&stored, key, previous_key, secret_box::PROXY_REMOTE_PASSWORD) {
            Ok(Some(sealed)) if apply => {
                payload["remote_password"] = serde_json::Value::String(sealed);
                sqlx::query("UPDATE domain_events SET payload = $1 WHERE id = $2").bind(&payload).bind(id).execute(&mut *tx).await.infra_err()?;
                Ok(Outcome::Upgraded)
            }
            Ok(Some(_)) => Ok(Outcome::Pending),
            Ok(None) => Ok(Outcome::Current),
            Err(e) => Err(e),
        };
        report.record(secret_box::PROXY_REMOTE_PASSWORD, &format!("repository journal event {id}"), outcome);
    }

    tx.commit().await.infra_err()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use artiferris_domain::email::{SmtpSecurity, SmtpSettings, SmtpSettingsPort};
    use artiferris_domain::mfa::TotpCredentialPort;
    use artiferris_domain::organization::PUBLIC_ORGANIZATION_ID;
    use artiferris_domain::package_repository::{PackageRepositoryEvent, PackageRepositoryEventStorePort, PackageRepositoryQueryPort, RepositoryFormat, RepositoryType};
    use artiferris_domain::sso::{IdentityProviderConfig, IdentityProviderRepositoryPort, LdapConfig, OidcConfig};
    use uuid::Uuid;

    use super::*;
    use crate::postgres::identity_provider_repository::PostgresIdentityProviderRepository;
    use crate::postgres::package_repository_store::PostgresPackageRepositoryStore;
    use crate::postgres::smtp_settings_repository::PostgresSmtpSettingsRepository;
    use crate::postgres::totp_credential_repository::PostgresTotpCredentialRepository;

    const KEY: &str = "current-key-current-key-current-key";

    struct Seeded {
        user_id: Uuid,
        repository_id: Uuid,
    }

    /// Writes one of every kind of secret through the real repositories, then swaps the stored form back to the legacy one.
    async fn seed_legacy(pool: &PgPool, key: &str) -> Seeded {
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, password_hash, organization_id) VALUES ($1, 'mfa-user', 'x', $2)").bind(user_id).bind(PUBLIC_ORGANIZATION_ID).execute(pool).await.unwrap();
        let totp = PostgresTotpCredentialRepository::new(pool.clone(), key.to_string());
        let created_at = chrono::Utc::now();
        totp.begin_enrollment(user_id, "TOTPSEED", created_at).await.unwrap();
        totp.confirm(user_id, created_at, 0).await.unwrap();
        let (ciphertext, nonce) = secret_box::legacy_encrypt("TOTPSEED", key);
        sqlx::query("UPDATE totp_credentials SET encrypted_secret = $1, secret_nonce = $2").bind(ciphertext).bind(nonce).execute(pool).await.unwrap();

        PostgresSmtpSettingsRepository::new(pool.clone(), key.to_string())
            .update(
                PUBLIC_ORGANIZATION_ID,
                &SmtpSettings {
                    host: "smtp.example.com".to_string(),
                    port: 587,
                    username: "u".to_string(),
                    password: "smtp-password".to_string(),
                    from_name: "A".to_string(),
                    from_address: "a@example.com".to_string(),
                    security: SmtpSecurity::StartTls,
                },
                None,
            )
            .await
            .unwrap();
        let (ciphertext, nonce) = secret_box::legacy_encrypt("smtp-password", key);
        sqlx::query("UPDATE smtp_settings SET encrypted_password = $1, password_nonce = $2").bind(ciphertext).bind(nonce).execute(pool).await.unwrap();

        let other_org = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations (id, slug, display_name, is_public) VALUES ($1, 'acme', 'Acme', false)").bind(other_org).execute(pool).await.unwrap();
        let providers = PostgresIdentityProviderRepository::new(pool.clone(), key.to_string());
        providers
            .set(
                PUBLIC_ORGANIZATION_ID,
                &IdentityProviderConfig::Ldap(LdapConfig {
                    server_url: "ldaps://ldap.example.com".to_string(),
                    bind_dn: "cn=svc".to_string(),
                    bind_password: "ldap-password".to_string(),
                    user_search_base: "ou=people".to_string(),
                    user_search_filter: "(uid={username})".to_string(),
                    email_attribute: "mail".to_string(),
                }),
                None,
            )
            .await
            .unwrap();
        providers
            .set(other_org, &IdentityProviderConfig::Oidc(OidcConfig { issuer_url: "https://idp.example.com".to_string(), client_id: "c".to_string(), client_secret: "oidc-secret".to_string() }), None)
            .await
            .unwrap();
        sqlx::query("UPDATE organization_identity_providers SET config = jsonb_set(config, '{bind_password_encrypted}', to_jsonb($1::text)) WHERE organization_id = $2")
            .bind(secret_box::legacy_encrypt_packed("ldap-password", key))
            .bind(PUBLIC_ORGANIZATION_ID)
            .execute(pool)
            .await
            .unwrap();
        // A value stored before encryption existed at all.
        sqlx::query("UPDATE organization_identity_providers SET config = jsonb_set(config, '{client_secret_encrypted}', to_jsonb('oidc-secret'::text)) WHERE organization_id = $1")
            .bind(other_org)
            .execute(pool)
            .await
            .unwrap();

        let repository_id = Uuid::new_v4();
        let store = PostgresPackageRepositoryStore::new(pool.clone(), key.to_string());
        store
            .append(
                repository_id,
                0,
                vec![PackageRepositoryEvent::Created {
                    repository_id,
                    organization_id: PUBLIC_ORGANIZATION_ID,
                    name: "proxy".to_string(),
                    format: RepositoryFormat::Npm,
                    repo_type: RepositoryType::Proxy,
                    remote_url: Some("https://registry.example.com".to_string()),
                    remote_username: Some("svc".to_string()),
                    remote_password: Some("proxy-password".to_string()),
                }],
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        let legacy = secret_box::legacy_encrypt_packed("proxy-password", key);
        sqlx::query("UPDATE package_repository_projections SET remote_password = $1").bind(&legacy).execute(pool).await.unwrap();
        sqlx::query("UPDATE domain_events SET payload = jsonb_set(payload, '{remote_password}', to_jsonb($1::text)) WHERE aggregate_type = 'PackageRepository'")
            .bind(&legacy)
            .execute(pool)
            .await
            .unwrap();

        Seeded { user_id, repository_id }
    }

    async fn assert_readable(pool: &PgPool, key: &str, seeded: &Seeded) {
        let totp = PostgresTotpCredentialRepository::new(pool.clone(), key.to_string()).get(seeded.user_id).await.unwrap().unwrap();
        assert_eq!(totp.secret, "TOTPSEED");
        let smtp = PostgresSmtpSettingsRepository::new(pool.clone(), key.to_string()).get(PUBLIC_ORGANIZATION_ID).await.unwrap().unwrap();
        assert_eq!(smtp.password, "smtp-password");
        let providers = PostgresIdentityProviderRepository::new(pool.clone(), key.to_string());
        match providers.get(PUBLIC_ORGANIZATION_ID).await.unwrap().unwrap() {
            IdentityProviderConfig::Ldap(ldap) => assert_eq!(ldap.bind_password, "ldap-password"),
            other => panic!("unexpected {other:?}"),
        }
        let store = PostgresPackageRepositoryStore::new(pool.clone(), key.to_string());
        let summary = store.find_by_id(seeded.repository_id).await.unwrap().unwrap();
        assert_eq!(summary.remote_password.as_deref(), Some("proxy-password"));
        let (_, events) = store.load(seeded.repository_id).await.unwrap();
        assert!(matches!(&events[0], PackageRepositoryEvent::Created { remote_password: Some(pw), .. } if pw == "proxy-password"));
    }

    async fn assert_readable_and_current(pool: &PgPool, key: &str, seeded: &Seeded) {
        assert_readable(pool, key, seeded).await;
        let stored: (String, String, String) = sqlx::query_as(
            "SELECT (SELECT config ->> 'client_secret_encrypted' FROM organization_identity_providers WHERE config ->> 'type' = 'oidc'), \
                    (SELECT remote_password FROM package_repository_projections), \
                    (SELECT payload ->> 'remote_password' FROM domain_events WHERE aggregate_type = 'PackageRepository')",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        for value in [&stored.0, &stored.1, &stored.2] {
            assert!(value.starts_with("af1."), "still legacy: {value}");
        }
        let (ciphertext,): (Vec<u8>,) = sqlx::query_as("SELECT encrypted_secret FROM totp_credentials").fetch_one(pool).await.unwrap();
        assert!(ciphertext.starts_with(b"AFS\x01"));
        let (ciphertext,): (Vec<u8>,) = sqlx::query_as("SELECT encrypted_password FROM smtp_settings").fetch_one(pool).await.unwrap();
        assert!(ciphertext.starts_with(b"AFS\x01"));
    }

    #[sqlx::test]
    async fn legacy_values_of_every_kind_are_upgraded_and_stay_readable(pool: PgPool) {
        let seeded = seed_legacy(&pool, KEY).await;

        let report = reencrypt_secrets(&pool, KEY, None, true).await.unwrap();

        assert_eq!(report, ReencryptionReport { upgraded: 6, pending: 0, failed: 0 });
        assert_readable_and_current(&pool, KEY, &seeded).await;
    }

    #[sqlx::test]
    async fn without_apply_nothing_is_rewritten_and_the_legacy_values_stay_readable(pool: PgPool) {
        let seeded = seed_legacy(&pool, KEY).await;

        let report = reencrypt_secrets(&pool, KEY, None, false).await.unwrap();

        assert_eq!(report, ReencryptionReport { upgraded: 0, pending: 6, failed: 0 });
        assert_readable(&pool, KEY, &seeded).await;
        let (ciphertext,): (Vec<u8>,) = sqlx::query_as("SELECT encrypted_secret FROM totp_credentials").fetch_one(&pool).await.unwrap();
        assert!(!ciphertext.starts_with(b"AFS\x01"), "the previous release must still be able to open it");
        let (stored,): (String,) = sqlx::query_as("SELECT remote_password FROM package_repository_projections").fetch_one(&pool).await.unwrap();
        assert!(!stored.starts_with("af1."), "still legacy: {stored}");
    }

    #[sqlx::test]
    async fn a_value_that_cannot_be_read_is_reported_even_without_apply(pool: PgPool) {
        seed_legacy(&pool, "some-other-key-some-other-key-0000").await;

        let report = reencrypt_secrets(&pool, KEY, None, false).await.unwrap();

        assert_eq!(report, ReencryptionReport { upgraded: 0, pending: 1, failed: 5 });
    }

    /// A settings write that has locked its row when the pass starts must survive it: the pass waits, then sees the new value instead of re-sealing the one it read before.
    #[sqlx::test]
    async fn a_settings_write_racing_the_pass_is_not_overwritten(pool: PgPool) {
        seed_legacy(&pool, KEY).await;
        let (fresh_ciphertext, fresh_nonce) = secret_box::seal("changed-during-the-pass", KEY, secret_box::SMTP_PASSWORD);
        let mut writer = pool.begin().await.unwrap();
        sqlx::query("UPDATE smtp_settings SET encrypted_password = $1, password_nonce = $2").bind(fresh_ciphertext).bind(fresh_nonce).execute(&mut *writer).await.unwrap();

        let pass = tokio::spawn({
            let pool = pool.clone();
            async move { reencrypt_secrets(&pool, KEY, None, true).await.unwrap() }
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(!pass.is_finished(), "the pass has to wait for the row lock");
        writer.commit().await.unwrap();
        let report = pass.await.unwrap();

        assert_eq!(report.failed, 0);
        assert_eq!(report.upgraded, 5, "everything but the row that was already current");
        let smtp = PostgresSmtpSettingsRepository::new(pool.clone(), KEY.to_string()).get(PUBLIC_ORGANIZATION_ID).await.unwrap().unwrap();
        assert_eq!(smtp.password, "changed-during-the-pass");
    }

    #[sqlx::test]
    async fn a_second_run_finds_nothing_to_do(pool: PgPool) {
        seed_legacy(&pool, KEY).await;
        reencrypt_secrets(&pool, KEY, None, true).await.unwrap();

        assert_eq!(reencrypt_secrets(&pool, KEY, None, true).await.unwrap(), ReencryptionReport::default());
    }

    #[sqlx::test]
    async fn a_rotation_moves_everything_to_the_new_key(pool: PgPool) {
        let old_key = "old-key-old-key-old-key-old-key-1234";
        let new_key = "new-key-new-key-new-key-new-key-5678";
        let seeded = seed_legacy(&pool, old_key).await;
        reencrypt_secrets(&pool, old_key, None, true).await.unwrap();

        let report = reencrypt_secrets(&pool, new_key, Some(old_key), true).await.unwrap();

        assert_eq!(report, ReencryptionReport { upgraded: 6, pending: 0, failed: 0 });
        assert_readable_and_current(&pool, new_key, &seeded).await;
    }

    #[sqlx::test]
    async fn a_value_that_cannot_be_read_is_reported_and_left_alone(pool: PgPool) {
        let seeded = seed_legacy(&pool, "some-other-key-some-other-key-0000").await;

        let report = reencrypt_secrets(&pool, KEY, None, true).await.unwrap();

        // The plaintext OIDC secret needs no key; the five real ciphertexts do.
        assert_eq!(report, ReencryptionReport { upgraded: 1, pending: 0, failed: 5 });
        let (still_legacy,): (Vec<u8>,) = sqlx::query_as("SELECT encrypted_secret FROM totp_credentials WHERE user_id = $1").bind(seeded.user_id).fetch_one(&pool).await.unwrap();
        assert!(!still_legacy.starts_with(b"AFS\x01"));
    }
}
