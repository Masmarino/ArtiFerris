
pub mod api_token_repository;
pub mod audit_backfill;
pub mod backup_code_repository;
pub mod branding_repository;
pub mod configuration_import;
pub mod docker_image_scan_repository;
pub mod docker_manifest_repository;
pub mod docker_upload_session_repository;
pub mod download_stats_repository;
pub mod metrics_snapshot_repository;
pub mod event_publisher;
pub mod health;
pub mod identity_provider_repository;
pub mod npm_dependency_audit_repository;
pub mod npm_package_repository;
pub mod organization_repository;
pub mod package_repository_store;
pub mod password_reset_repository;
pub mod permission_store;
pub mod public_catalog_repository;
pub mod rate_limit_store;
pub mod reserved_name_audit;
pub mod smtp_settings_repository;
pub mod system_settings_repository;
pub mod totp_credential_repository;
pub mod user_invitation_repository;
pub mod user_preferences_repository;
pub mod user_repository;
pub mod webauthn_credential_repository;

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub const DEFAULT_DB_MAX_CONNECTIONS: u32 = 10;

pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    // A short acquire timeout fails a request fast under pool exhaustion instead of hanging it.
    PgPoolOptions::new().max_connections(max_connections).min_connections(max_connections.min(1)).acquire_timeout(Duration::from_secs(10)).connect(database_url).await
}

/// Migrations recorded by a newer release are ignored, so rolling the image back to this release still starts.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.set_ignore_missing(true);
    migrator.run(pool).await
}

#[cfg(test)]
mod tests {
    #[sqlx::test]
    async fn migrations_create_the_expected_tables(pool: sqlx::PgPool) {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let names: Vec<String> = rows.into_iter().map(|(n,)| n).collect();
        for expected in [
            "users",
            "domain_events",
            "permission_projections",
            "package_repository_projections",
            "package_repository_group_members",
        ] {
            assert!(names.contains(&expected.to_string()), "missing table {expected}");
        }
    }

    #[sqlx::test]
    async fn migrations_recorded_by_a_newer_release_do_not_stop_startup(pool: sqlx::PgPool) {
        sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (99999999999999, 'from the future', true, '\\x00', 0)")
            .execute(&pool)
            .await
            .unwrap();

        super::run_migrations(&pool).await.unwrap();
    }

    /// The real file. `sqlx::test` already migrated this database, so the tests plant rows and re-run it; every
    /// statement is idempotent.
    const NORMALIZE_USERNAMES_MIGRATION: &str = include_str!("../../migrations/0002_personal_repositories_visibility_and_hardening.sql");

    const PUBLIC_ORGANIZATION_ID: &str = "00000000-0000-0000-0000-000000000001";

    async fn insert_raw_user(pool: &sqlx::PgPool, username: &str) {
        sqlx::query("INSERT INTO users (id, username, password_hash, organization_id) VALUES (gen_random_uuid(), $1, 'x', $2::uuid)")
            .bind(username)
            .bind(PUBLIC_ORGANIZATION_ID)
            .execute(pool)
            .await
            .unwrap();
    }

    /// Lookups use the lowercased name with an exact match, so a row stored as "Florian" would be unreachable and the
    /// account locked out.
    #[sqlx::test]
    async fn the_backfill_lowercases_an_existing_mixed_case_username(pool: sqlx::PgPool) {
        insert_raw_user(&pool, "Florian").await;

        sqlx::raw_sql(NORMALIZE_USERNAMES_MIGRATION).execute(&pool).await.unwrap();

        let (username,): (String,) = sqlx::query_as("SELECT username FROM users").fetch_one(&pool).await.unwrap();
        assert_eq!(username, "florian");
    }

    /// Two such accounts cannot be merged automatically, so startup refuses and the message names the collision. Needs
    /// the lower(username) unique index dropped.
    #[sqlx::test]
    async fn the_backfill_fails_loudly_when_two_accounts_differ_only_in_case(pool: sqlx::PgPool) {
        sqlx::raw_sql("DROP INDEX users_username_lower_unique").execute(&pool).await.unwrap();
        insert_raw_user(&pool, "Alice").await;
        insert_raw_user(&pool, "alice").await;

        let error = sqlx::raw_sql(NORMALIZE_USERNAMES_MIGRATION).execute(&pool).await.unwrap_err();

        let message = error.to_string();
        assert!(message.contains("alice") && message.contains("differing only in letter case"), "the error must name the ambiguous username in actionable terms, got: {message}");
        let (remaining,): (i64,) = sqlx::query_as("SELECT count(*) FROM users WHERE username = 'Alice'").fetch_one(&pool).await.unwrap();
        assert_eq!(remaining, 1, "nothing may be silently renamed or dropped when the outcome is ambiguous");
    }

    const PER_ORGANIZATION_EMAIL_MIGRATION: &str = include_str!("../../migrations/0009_org_scoped_verified_email.sql");

    async fn insert_organization(pool: &sqlx::PgPool, slug: &str) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO organizations (id, slug, display_name) VALUES ($1, $2, $2)").bind(id).bind(slug).execute(pool).await.unwrap();
        id
    }

    async fn insert_user_with_email(pool: &sqlx::PgPool, organization_id: uuid::Uuid, username: &str, email: &str) {
        sqlx::query("INSERT INTO users (id, username, password_hash, organization_id, email) VALUES (gen_random_uuid(), $1, 'x', $2, $3)")
            .bind(username)
            .bind(organization_id)
            .bind(email)
            .execute(pool)
            .await
            .unwrap();
    }

    /// Case-sensitive `users_email_unique` let these through before 0006, so the upgrade must not refuse them.
    #[sqlx::test(migrations = false)]
    async fn the_same_address_in_another_case_in_two_organizations_does_not_stop_the_upgrade(pool: sqlx::PgPool) {
        sqlx::migrate!("./migrations").run_to(5, &pool).await.unwrap();
        let acme = insert_organization(&pool, "acme").await;
        let globex = insert_organization(&pool, "globex").await;
        insert_user_with_email(&pool, acme, "alice-acme", "Alice@Example.com").await;
        insert_user_with_email(&pool, globex, "alice-globex", "alice@example.com").await;

        super::run_migrations(&pool).await.unwrap();

        let (verified,): (i64,) = sqlx::query_as("SELECT count(*) FROM users WHERE email_verified").fetch_one(&pool).await.unwrap();
        assert_eq!(verified, 2);
        let (global_index,): (i64,) = sqlx::query_as("SELECT count(*) FROM pg_indexes WHERE indexname = 'users_verified_email_unique'").fetch_one(&pool).await.unwrap();
        assert_eq!(global_index, 0);
    }

    #[sqlx::test(migrations = false)]
    async fn two_verified_accounts_of_one_organization_differing_only_in_case_stop_the_upgrade_with_a_clear_message(pool: sqlx::PgPool) {
        sqlx::migrate!("./migrations").run_to(5, &pool).await.unwrap();
        let acme = insert_organization(&pool, "acme").await;
        insert_user_with_email(&pool, acme, "alice", "Alice@Example.com").await;
        insert_user_with_email(&pool, acme, "alice-two", "alice@example.com").await;

        let error = super::run_migrations(&pool).await.unwrap_err().to_string();

        assert!(error.contains("alice@example.com") && error.contains("same organization"), "{error}");
    }

    /// A database that applied the earlier 0006 holds the global index; 0009 replaces it.
    #[sqlx::test]
    async fn a_database_that_applied_the_earlier_email_migration_ends_up_with_the_per_organization_rule(pool: sqlx::PgPool) {
        sqlx::raw_sql("DROP INDEX users_verified_email_per_organization_unique; CREATE UNIQUE INDEX users_verified_email_unique ON users (lower(email)) WHERE email IS NOT NULL AND email_verified;")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(PER_ORGANIZATION_EMAIL_MIGRATION).execute(&pool).await.unwrap();
        sqlx::raw_sql(PER_ORGANIZATION_EMAIL_MIGRATION).execute(&pool).await.unwrap();

        let acme = insert_organization(&pool, "acme").await;
        let globex = insert_organization(&pool, "globex").await;
        for (organization, username, email) in [(acme, "a", "Alice@Example.com"), (globex, "b", "alice@example.com")] {
            insert_user_with_email(&pool, organization, username, email).await;
            sqlx::query("UPDATE users SET email_verified = true WHERE username = $1").bind(username).execute(&pool).await.unwrap();
        }
        insert_user_with_email(&pool, acme, "c", "ALICE@example.com").await;
        let duplicate = sqlx::query("UPDATE users SET email_verified = true WHERE username = 'c'").execute(&pool).await;
        assert!(duplicate.is_err(), "the same organization still cannot verify one address twice");
    }
}
