use async_trait::async_trait;
use artiferris_domain::audit::{
    AdminAuditEvent, AdminAuditRecord, AuditCursor, AuditEntry, AuditPage, AuditQueryFilter, AuditRecord, AuditRetentionPort, DockerRegistryEvent, EventPublisherPort, NpmPackageEvent, SecurityAuditRecord, SecurityEvent,
};
use artiferris_domain::error::{DomainError, EventStoreError};
use crate::error_ext::{InfraErr, StorageErr};
use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;

pub struct PostgresEventPublisher {
    pool: PgPool,
}

impl PostgresEventPublisher {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// For a repository method that takes an optional record: written on the connection of the change it describes, so both commit or neither does.
pub(crate) async fn insert_admin_audit(connection: &mut PgConnection, audit: Option<&AdminAuditRecord>) -> Result<(), DomainError> {
    if let Some(audit) = audit {
        insert_admin_event(&mut *connection, &audit.event, audit.actor_id).await.infra_err()?;
    }
    Ok(())
}

pub(crate) async fn insert_security_audit(connection: &mut PgConnection, audit: Option<&SecurityAuditRecord>) -> Result<(), DomainError> {
    if let Some(audit) = audit {
        insert_security_event(&mut *connection, &audit.event, audit.actor_id).await.infra_err()?;
    }
    Ok(())
}

pub(crate) async fn insert_audit(connection: &mut PgConnection, audit: Option<&AuditRecord>) -> Result<(), DomainError> {
    match audit {
        Some(AuditRecord::Admin(record)) => insert_admin_audit(connection, Some(record)).await,
        Some(AuditRecord::Security(record)) => insert_security_audit(connection, Some(record)).await,
        None => Ok(()),
    }
}

/// An event that names no organization takes the acting user's.
pub(crate) async fn insert_security_event<'e>(executor: impl PgExecutor<'e>, event: &SecurityEvent, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
    let payload = serde_json::to_value(event).storage_err()?;
    sqlx::query!(
        "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, actor_id, organization_id) \
         VALUES ('Security', $1, $2, $3, 1, $4, COALESCE($5, (SELECT organization_id FROM users WHERE id = $4)))",
        Uuid::new_v4().to_string(),
        event.event_type(),
        payload,
        actor_id,
        event.organization_id()
    )
    .execute(executor)
    .await
    .storage_err()?;
    Ok(())
}

/// One insert, so a repository can call it inside the transaction of the change it describes.
pub(crate) async fn insert_admin_event<'e>(executor: impl PgExecutor<'e>, event: &AdminAuditEvent, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
    let payload = serde_json::to_value(event).storage_err()?;
    sqlx::query!(
        "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, actor_id, organization_id) \
         VALUES ('Admin', $1, $2, $3, 1, $4, $5)",
        Uuid::new_v4().to_string(),
        event.event_type(),
        payload,
        actor_id,
        event.organization_id()
    )
    .execute(executor)
    .await
    .storage_err()?;
    Ok(())
}

/// The aggregate list must match the partial index `domain_events_audit_occurred_at_idx`, or the sweep walks every old package event first.
const PRUNE_AUDIT_EVENTS: &str = "DELETE FROM domain_events WHERE id IN (\
     SELECT id FROM domain_events WHERE aggregate_type IN ('Security', 'Admin') AND occurred_at < $1 ORDER BY occurred_at LIMIT $2)";

#[async_trait]
impl AuditRetentionPort for PostgresEventPublisher {
    async fn delete_audit_events_before(&self, cutoff: chrono::DateTime<chrono::Utc>, limit: i64) -> Result<u64, EventStoreError> {
        let result = sqlx::query(PRUNE_AUDIT_EVENTS)
        .bind(cutoff)
        .bind(limit)
        .execute(&self.pool)
        .await
        .storage_err()?;
        Ok(result.rows_affected())
    }
}

#[async_trait]
impl EventPublisherPort for PostgresEventPublisher {
    async fn publish_security_event(&self, event: SecurityEvent, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
        insert_security_event(&self.pool, &event, actor_id).await
    }

    async fn publish_admin_event(&self, event: AdminAuditEvent, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
        insert_admin_event(&self.pool, &event, actor_id).await
    }

    async fn query_audit_log(&self, filter: AuditQueryFilter) -> Result<AuditPage, EventStoreError> {
        let page_size = filter.page_size();
        let rows = sqlx::query!(
            r#"
            SELECT id, aggregate_type, aggregate_id, event_type, payload, occurred_at, actor_id, organization_id
            FROM domain_events
            WHERE ($1::text IS NULL OR aggregate_type = $1)
              AND ($2::text IS NULL OR aggregate_id = $2)
              AND ($3::uuid IS NULL OR actor_id = $3)
              AND ($4::timestamptz IS NULL OR occurred_at >= $4)
              AND ($5::timestamptz IS NULL OR occurred_at <= $5)
              AND ($6::text IS NULL OR aggregate_type != $6)
              AND ($7::uuid IS NULL OR organization_id = $7)
              AND ($8::timestamptz IS NULL OR (occurred_at, id) < ($8, $9::uuid))
            ORDER BY occurred_at DESC, id DESC
            LIMIT $10
            "#,
            filter.aggregate_type,
            filter.aggregate_id,
            filter.actor_id,
            filter.from,
            filter.to,
            filter.exclude_aggregate_type,
            filter.organization_id,
            filter.cursor.map(|c| c.occurred_at),
            filter.cursor.map(|c| c.id),
            page_size + 1
        )
        .fetch_all(&self.pool)
        .await
        .storage_err()?;

        let mut entries: Vec<AuditEntry> = rows
            .into_iter()
            .map(|row| AuditEntry {
                id: row.id,
                aggregate_type: row.aggregate_type,
                aggregate_id: row.aggregate_id,
                event_type: row.event_type,
                payload: row.payload,
                occurred_at: row.occurred_at,
                actor_id: row.actor_id,
                organization_id: row.organization_id,
            })
            .collect();
        let has_more = entries.len() as i64 > page_size;
        entries.truncate(page_size as usize);
        let next_cursor = if has_more { entries.last().map(|e| AuditCursor { occurred_at: e.occurred_at, id: e.id }) } else { None };
        Ok(AuditPage { entries, next_cursor })
    }

    async fn publish_npm_event(&self, event: NpmPackageEvent, npm_package_id: Uuid, package_repository_id: Uuid, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
        let payload = serde_json::to_value(&event).storage_err()?;

        let mut tx = self.pool.begin().await.storage_err()?;

        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", npm_package_id.to_string())
            .execute(&mut *tx)
            .await
            .storage_err()?;

        let current_version: i64 = sqlx::query_scalar!(
            "SELECT version FROM domain_events \
             WHERE aggregate_type = 'NpmPackage' AND aggregate_id = $1 ORDER BY version DESC LIMIT 1 FOR UPDATE",
            npm_package_id.to_string()
        )
        .fetch_optional(&mut *tx)
        .await
        .storage_err()?
        .unwrap_or(0);
        let next_version = current_version + 1;

        sqlx::query!(
            "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, actor_id, organization_id) \
             VALUES ('NpmPackage', $1, $2, $3, $4, $5, repository_organization($6))",
            npm_package_id.to_string(),
            event.event_type(),
            payload,
            next_version,
            actor_id,
            package_repository_id.to_string()
        )
        .execute(&mut *tx)
        .await
        .storage_err()?;

        tx.commit().await.storage_err()?;
        Ok(())
    }

    async fn publish_docker_event(&self, event: DockerRegistryEvent, package_repository_id: Uuid, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
        let payload = serde_json::to_value(&event).storage_err()?;

        let mut tx = self.pool.begin().await.storage_err()?;

        sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", package_repository_id.to_string())
            .execute(&mut *tx)
            .await
            .storage_err()?;

        let current_version: i64 = sqlx::query_scalar!(
            "SELECT version FROM domain_events \
             WHERE aggregate_type = 'DockerRegistry' AND aggregate_id = $1 ORDER BY version DESC LIMIT 1 FOR UPDATE",
            package_repository_id.to_string()
        )
        .fetch_optional(&mut *tx)
        .await
        .storage_err()?
        .unwrap_or(0);
        let next_version = current_version + 1;

        sqlx::query!(
            "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, actor_id, organization_id) \
             VALUES ('DockerRegistry', $1, $2, $3, $4, $5, repository_organization($1))",
            package_repository_id.to_string(),
            event.event_type(),
            payload,
            next_version,
            actor_id
        )
        .execute(&mut *tx)
        .await
        .storage_err()?;

        tx.commit().await.storage_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn publishing_two_npm_events_for_the_same_package_does_not_collide_on_version(pool: sqlx::PgPool) {
        let publisher = PostgresEventPublisher::new(pool);
        let package_id = Uuid::new_v4();

        publisher
            .publish_npm_event(
                NpmPackageEvent::PackagePushed { package_name: "left-pad".to_string(), version: "1.0.0".to_string() },
                package_id,
                Uuid::new_v4(),
                None,
            )
            .await
            .unwrap();
        publisher
            .publish_npm_event(
                NpmPackageEvent::DistTagChanged { package_name: "left-pad".to_string(), tag: "beta".to_string(), version: "1.0.0".to_string() },
                package_id,
                Uuid::new_v4(),
                None,
            )
            .await
            .unwrap();

        let entries = publisher
            .query_audit_log(AuditQueryFilter { aggregate_type: Some("NpmPackage".to_string()), ..Default::default() })
            .await
            .unwrap().entries;
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.aggregate_id == package_id.to_string()));
    }

    #[sqlx::test]
    async fn publishes_and_queries_a_security_event(pool: sqlx::PgPool) {
        let publisher = PostgresEventPublisher::new(pool);
        publisher
            .publish_security_event(SecurityEvent::LoginFailed { username: "florian".to_string(), ip: "127.0.0.1".to_string() }, None)
            .await
            .unwrap();

        let entries = publisher
            .query_audit_log(AuditQueryFilter { aggregate_type: Some("Security".to_string()), ..Default::default() })
            .await
            .unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "LoginFailed");
    }

    #[sqlx::test]
    async fn filters_by_actor_id(pool: sqlx::PgPool) {
        let publisher = PostgresEventPublisher::new(pool);
        let actor_id = Uuid::new_v4();
        publisher
            .publish_security_event(
                SecurityEvent::AccessDenied { user_id: actor_id, repository_id: Uuid::new_v4(), action: "push".to_string() },
                Some(actor_id),
            )
            .await
            .unwrap();
        publisher
            .publish_security_event(SecurityEvent::LoginFailed { username: "other".to_string(), ip: "10.0.0.1".to_string() }, None)
            .await
            .unwrap();

        let entries = publisher.query_audit_log(AuditQueryFilter { actor_id: Some(actor_id), ..Default::default() }).await.unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "AccessDenied");
    }

    async fn insert_aged_event(pool: &sqlx::PgPool, aggregate_type: &str, days_ago: i32) {
        sqlx::query(
            "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, occurred_at) \
             VALUES ($1, $2, 'Event', '{}'::jsonb, 1, now() - make_interval(days => $3))",
        )
        .bind(aggregate_type)
        .bind(Uuid::new_v4().to_string())
        .bind(days_ago)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn remaining(pool: &sqlx::PgPool, aggregate_type: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE aggregate_type = $1").bind(aggregate_type).fetch_one(pool).await.unwrap()
    }

    #[sqlx::test]
    async fn the_retention_sweep_removes_only_old_security_and_admin_events(pool: sqlx::PgPool) {
        for aggregate_type in ["Security", "Admin", "NpmPackage", "DockerRegistry", "PackageRepository", "Permission"] {
            insert_aged_event(&pool, aggregate_type, 400).await;
            insert_aged_event(&pool, aggregate_type, 10).await;
        }
        let publisher = PostgresEventPublisher::new(pool.clone());

        let removed = publisher.delete_audit_events_before(chrono::Utc::now() - chrono::Duration::days(365), 100).await.unwrap();

        assert_eq!(removed, 2);
        assert_eq!(remaining(&pool, "Security").await, 1);
        assert_eq!(remaining(&pool, "Admin").await, 1);
        for kept in ["NpmPackage", "DockerRegistry", "PackageRepository", "Permission"] {
            assert_eq!(remaining(&pool, kept).await, 2, "{kept} events must survive");
        }
    }

    #[sqlx::test]
    async fn the_retention_sweep_deletes_the_oldest_events_first_up_to_the_limit(pool: sqlx::PgPool) {
        for days_ago in [500, 450, 400] {
            insert_aged_event(&pool, "Security", days_ago).await;
        }
        let publisher = PostgresEventPublisher::new(pool.clone());

        let removed = publisher.delete_audit_events_before(chrono::Utc::now() - chrono::Duration::days(365), 2).await.unwrap();

        assert_eq!(removed, 2);
        let oldest_left: i64 = sqlx::query_scalar("SELECT extract(day FROM now() - min(occurred_at))::bigint FROM domain_events").fetch_one(&pool).await.unwrap();
        assert_eq!(oldest_left, 400);
    }

    #[sqlx::test]
    async fn excluding_an_aggregate_type_leaves_the_row_budget_to_the_business_events(pool: sqlx::PgPool) {
        for index in 0..25 {
            sqlx::query(
                "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version) \
                 VALUES ('Security', $1, 'LoginFailed', '{}'::jsonb, 1)",
            )
            .bind(format!("security-{index}"))
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version) \
             VALUES ('Permission', 'perm-1', 'PermissionGranted', '{}'::jsonb, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version) \
             VALUES ('PackageRepository', 'repo-1', 'PackageRepositoryCreated', '{}'::jsonb, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let publisher = PostgresEventPublisher::new(pool);

        let all = publisher.query_audit_log(AuditQueryFilter::default()).await.unwrap().entries;
        assert_eq!(all.len(), 27);

        let entries = publisher
            .query_audit_log(AuditQueryFilter { exclude_aggregate_type: Some("Security".to_string()), ..Default::default() })
            .await
            .unwrap().entries;
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.aggregate_type != "Security"));
        let mut event_types: Vec<_> = entries.iter().map(|entry| entry.event_type.as_str()).collect();
        event_types.sort_unstable();
        assert_eq!(event_types, vec!["PackageRepositoryCreated", "PermissionGranted"]);
    }

    async fn insert_event_at(pool: &PgPool, organization_id: Option<Uuid>, event_type: &str, occurred_at: chrono::DateTime<chrono::Utc>) {
        sqlx::query(
            "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, occurred_at, organization_id) \
             VALUES ('Admin', $1, $2, '{}'::jsonb, 1, $3, $4)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(event_type)
        .bind(occurred_at)
        .bind(organization_id)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn a_small_organizations_events_are_not_pushed_out_by_a_busy_one(pool: sqlx::PgPool) {
        let (quiet, busy) = (Uuid::new_v4(), Uuid::new_v4());
        let start = chrono::Utc::now() - chrono::Duration::hours(1);
        for offset in 0..3 {
            insert_event_at(&pool, Some(quiet), "UserInvited", start + chrono::Duration::seconds(offset)).await;
        }
        for offset in 0..250 {
            insert_event_at(&pool, Some(busy), "UserInvited", start + chrono::Duration::minutes(1) + chrono::Duration::seconds(offset)).await;
        }
        insert_event_at(&pool, None, "ConfigurationExported", start + chrono::Duration::minutes(30)).await;
        let publisher = PostgresEventPublisher::new(pool);

        let quiet_page = publisher.query_audit_log(AuditQueryFilter { organization_id: Some(quiet), ..Default::default() }).await.unwrap();

        assert_eq!(quiet_page.entries.len(), 3);
        assert!(quiet_page.entries.iter().all(|entry| entry.organization_id == Some(quiet)));
        assert!(quiet_page.next_cursor.is_none());
    }

    #[sqlx::test]
    async fn events_without_an_organization_show_in_no_organizations_view_but_in_the_unscoped_one(pool: sqlx::PgPool) {
        let organization_id = Uuid::new_v4();
        let publisher = PostgresEventPublisher::new(pool);
        publisher.publish_security_event(SecurityEvent::LoginFailed { username: "nobody".to_string(), ip: "10.0.0.1".to_string() }, None).await.unwrap();
        publisher.publish_admin_event(AdminAuditEvent::ConfigurationExported { users: 1, repositories: 1, permissions: 1 }, Some(Uuid::new_v4())).await.unwrap();
        publisher
            .publish_admin_event(AdminAuditEvent::UserActivated { user_id: Uuid::new_v4(), organization_id }, None)
            .await
            .unwrap();

        let scoped = publisher.query_audit_log(AuditQueryFilter { organization_id: Some(organization_id), ..Default::default() }).await.unwrap();
        let unscoped = publisher.query_audit_log(AuditQueryFilter::default()).await.unwrap();

        assert_eq!(scoped.entries.iter().map(|e| e.event_type.as_str()).collect::<Vec<_>>(), vec!["UserActivated"]);
        assert_eq!(unscoped.entries.len(), 3);
        assert!(unscoped.entries.iter().filter(|e| e.organization_id.is_none()).count() == 2);
    }

    #[sqlx::test]
    async fn pages_walk_the_log_newest_first_without_gaps_or_repeats_across_equal_timestamps(pool: sqlx::PgPool) {
        let organization_id = Uuid::new_v4();
        let same_instant = chrono::Utc::now() - chrono::Duration::minutes(5);
        for _ in 0..12 {
            insert_event_at(&pool, Some(organization_id), "UserInvited", same_instant).await;
        }
        for offset in 1..=13 {
            insert_event_at(&pool, Some(organization_id), "UserDeleted", same_instant + chrono::Duration::seconds(offset)).await;
        }
        let publisher = PostgresEventPublisher::new(pool);

        let mut seen = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        loop {
            let page = publisher.query_audit_log(AuditQueryFilter { organization_id: Some(organization_id), cursor, limit: Some(10), ..Default::default() }).await.unwrap();
            pages += 1;
            assert!(page.entries.len() <= 10);
            seen.extend(page.entries.iter().map(|e| (e.occurred_at, e.id)));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        assert_eq!(pages, 3);
        assert_eq!(seen.len(), 25);
        let mut expected = seen.clone();
        expected.sort_by(|a, b| b.cmp(a));
        expected.dedup();
        assert_eq!(seen, expected, "strictly descending on (occurred_at, id), so no repeats");
    }

    #[sqlx::test]
    async fn a_page_exactly_as_long_as_the_limit_has_no_next_cursor(pool: sqlx::PgPool) {
        for offset in 0..10 {
            insert_event_at(&pool, None, "UserInvited", chrono::Utc::now() - chrono::Duration::seconds(offset)).await;
        }
        let publisher = PostgresEventPublisher::new(pool);

        let page = publisher.query_audit_log(AuditQueryFilter { limit: Some(10), ..Default::default() }).await.unwrap();

        assert_eq!(page.entries.len(), 10);
        assert!(page.next_cursor.is_none());
    }

    #[sqlx::test]
    async fn the_page_size_is_capped(pool: sqlx::PgPool) {
        for offset in 0..205 {
            insert_event_at(&pool, None, "UserInvited", chrono::Utc::now() - chrono::Duration::seconds(offset)).await;
        }
        let publisher = PostgresEventPublisher::new(pool);

        let page = publisher.query_audit_log(AuditQueryFilter { limit: Some(10_000), ..Default::default() }).await.unwrap();

        assert_eq!(page.entries.len(), 200);
        assert!(page.next_cursor.is_some());
    }

    #[sqlx::test]
    async fn a_security_event_names_its_own_organization_or_takes_the_actors(pool: sqlx::PgPool) {
        let (actor_org, named_org) = (Uuid::new_v4(), Uuid::new_v4());
        let actor_id = Uuid::new_v4();
        for org in [actor_org, named_org] {
            sqlx::query("INSERT INTO organizations (id, slug, display_name) VALUES ($1, $2, $2)").bind(org).bind(org.simple().to_string()).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO users (id, username, password_hash, organization_id) VALUES ($1, 'actor', 'x', $2)").bind(actor_id).bind(actor_org).execute(&pool).await.unwrap();
        let publisher = PostgresEventPublisher::new(pool);

        publisher.publish_security_event(SecurityEvent::PasskeyVerificationFailed { user_id: actor_id }, Some(actor_id)).await.unwrap();
        publisher.publish_security_event(SecurityEvent::OidcLoginFailed { organization_id: named_org }, None).await.unwrap();
        publisher.publish_security_event(SecurityEvent::PasswordChanged { user_id: actor_id, organization_id: named_org }, Some(actor_id)).await.unwrap();

        let of = |entries: &[AuditEntry], event_type: &str| entries.iter().find(|e| e.event_type == event_type).unwrap().organization_id;
        let entries = publisher.query_audit_log(AuditQueryFilter::default()).await.unwrap().entries;
        assert_eq!(of(&entries, "PasskeyVerificationFailed"), Some(actor_org));
        assert_eq!(of(&entries, "OidcLoginFailed"), Some(named_org));
        assert_eq!(of(&entries, "PasswordChanged"), Some(named_org), "the event's own organization wins over the actor's");
    }

    #[sqlx::test]
    async fn an_admin_event_is_stored_once_with_its_actor_and_organization(pool: sqlx::PgPool) {
        let (actor_id, organization_id) = (Uuid::new_v4(), Uuid::new_v4());
        let publisher = PostgresEventPublisher::new(pool);

        publisher
            .publish_admin_event(AdminAuditEvent::OrganizationCreated { organization_id, slug: "acme".to_string(), display_name: "Acme".to_string() }, Some(actor_id))
            .await
            .unwrap();

        let entries = publisher.query_audit_log(AuditQueryFilter { aggregate_type: Some("Admin".to_string()), ..Default::default() }).await.unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "OrganizationCreated");
        assert_eq!(entries[0].actor_id, Some(actor_id));
        assert_eq!(entries[0].organization_id, Some(organization_id));
        assert_eq!(entries[0].payload["slug"], "acme");
    }

    #[sqlx::test]
    async fn npm_and_docker_events_are_filed_under_their_repositorys_organization_even_after_the_package_is_gone(pool: sqlx::PgPool) {
        let (organization_id, repository_id) = (Uuid::new_v4(), Uuid::new_v4());
        sqlx::query(
            "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version) \
             VALUES ('PackageRepository', $1, 'Created', jsonb_build_object('event_type', 'Created', 'organization_id', $2::text), 1)",
        )
        .bind(repository_id.to_string())
        .bind(organization_id.to_string())
        .execute(&pool)
        .await
        .unwrap();
        let publisher = PostgresEventPublisher::new(pool);

        publisher.publish_npm_event(NpmPackageEvent::PackageDeleted { package_name: "left-pad".to_string() }, Uuid::new_v4(), repository_id, None).await.unwrap();
        publisher
            .publish_docker_event(DockerRegistryEvent::ImagePushed { image_name: "app".to_string(), digest: "sha256:abc".to_string() }, repository_id, None)
            .await
            .unwrap();

        let entries = publisher.query_audit_log(AuditQueryFilter { organization_id: Some(organization_id), ..Default::default() }).await.unwrap().entries;
        let mut types: Vec<_> = entries.iter().map(|e| e.aggregate_type.as_str()).collect();
        types.sort_unstable();
        assert_eq!(types, vec!["DockerRegistry", "NpmPackage"]);
    }

    #[sqlx::test]
    async fn the_retention_sweep_reads_only_the_audit_index_never_package_or_repository_history(pool: sqlx::PgPool) {
        sqlx::query("INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, occurred_at) SELECT 'DockerRegistry', 'repo', 'ImagePushed', '{}'::jsonb, g, now() - interval '400 days' + g * interval '1 second' FROM generate_series(1, 2000) g")
            .execute(&pool)
            .await
            .unwrap();
        insert_event_at(&pool, None, "UserInvited", chrono::Utc::now() - chrono::Duration::days(500)).await;
        sqlx::query("ANALYZE domain_events").execute(&pool).await.unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("SET enable_seqscan = off").execute(&mut *connection).await.unwrap();

        let plan: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!("EXPLAIN {PRUNE_AUDIT_EVENTS}"))).bind(chrono::Utc::now() - chrono::Duration::days(365)).bind(5_000_i64).fetch_all(&mut *connection).await.unwrap();

        let plan = plan.into_iter().map(|(line,)| line).collect::<Vec<_>>().join("\n");
        assert!(plan.contains("domain_events_audit_occurred_at_idx"), "{plan}");
    }

    /// Migrates to 0007, loads existing rows, applies 0008 as an upgrade would, then runs the startup backfill.
    #[sqlx::test(migrations = false)]
    async fn migration_0008_backfills_the_organization_of_existing_events(pool: sqlx::PgPool) {
        let mut before = sqlx::migrate!("./migrations");
        before.migrations = before.migrations.iter().filter(|m| m.version < 8).cloned().collect::<Vec<_>>().into();
        before.run(&pool).await.unwrap();

        let (org_a, org_b) = (Uuid::new_v4(), Uuid::new_v4());
        let (repo_a, repo_b, npm_package, user_a, user_b) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        for org in [org_a, org_b] {
            sqlx::query("INSERT INTO organizations (id, slug, display_name) VALUES ($1, $2, $2)").bind(org).bind(org.simple().to_string()).execute(&pool).await.unwrap();
        }
        for (user, org) in [(user_a, org_a), (user_b, org_b)] {
            sqlx::query("INSERT INTO users (id, username, password_hash, organization_id) VALUES ($1, $2, 'x', $3)").bind(user).bind(user.simple().to_string()).bind(org).execute(&pool).await.unwrap();
        }
        for (repo, org) in [(repo_a, org_a), (repo_b, org_b)] {
            sqlx::query("INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, version) VALUES ($1, $2, $3, 'npm', 'hosted', 1)")
                .bind(repo)
                .bind(org)
                .bind(repo.simple().to_string())
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO npm_packages (id, package_repository_id, name) VALUES ($1, $2, 'left-pad')").bind(npm_package).bind(repo_a).execute(&pool).await.unwrap();

        let legacy = |aggregate_type: &'static str, aggregate_id: String, event_type: &'static str, payload: serde_json::Value, version: i64, actor: Option<Uuid>| {
            let pool = pool.clone();
            async move {
                sqlx::query("INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, actor_id) VALUES ($1, $2, $3, $4, $5, $6)")
                    .bind(aggregate_type)
                    .bind(aggregate_id)
                    .bind(event_type)
                    .bind(payload)
                    .bind(version)
                    .bind(actor)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        };
        legacy("PackageRepository", repo_a.to_string(), "Created", serde_json::json!({ "event_type": "Created", "organization_id": org_a }), 1, None).await;
        legacy("PackageRepository", repo_a.to_string(), "QuotaSet", serde_json::json!({ "event_type": "QuotaSet" }), 2, None).await;
        legacy("PackageRepository", repo_b.to_string(), "Created", serde_json::json!({ "event_type": "Created", "organization_id": org_b }), 1, None).await;
        legacy("DockerRegistry", repo_b.to_string(), "ImagePushed", serde_json::json!({}), 1, None).await;
        legacy("Permission", format!("{user_a}:{repo_a}"), "Granted", serde_json::json!({}), 1, None).await;
        legacy("NpmPackage", npm_package.to_string(), "PackagePushed", serde_json::json!({}), 1, None).await;
        legacy("NpmPackage", Uuid::new_v4().to_string(), "PackageDeleted", serde_json::json!({}), 1, None).await;
        legacy("Security", Uuid::new_v4().to_string(), "AccessDenied", serde_json::json!({}), 1, Some(user_b)).await;
        legacy("Security", Uuid::new_v4().to_string(), "OidcLoginFailed", serde_json::json!({ "organization_id": org_a }), 1, None).await;
        legacy("Security", Uuid::new_v4().to_string(), "LoginFailed", serde_json::json!({}), 1, None).await;

        sqlx::raw_sql(include_str!("../../migrations/0008_audit_organization_scope.sql")).execute(&pool).await.unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0008_audit_organization_scope.sql")).execute(&pool).await.unwrap();
        let already_stamped: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE organization_id IS NOT NULL").fetch_one(&pool).await.unwrap();
        assert_eq!(already_stamped, 0, "the migration itself leaves the old rows alone");

        let outcome = crate::postgres::audit_backfill::backfill_audit_organizations(&pool, std::time::Duration::ZERO).await.unwrap();
        assert!(matches!(outcome, crate::postgres::audit_backfill::BackfillOutcome::Finished { stamped: 8, .. }), "{outcome:?}");
        assert_eq!(
            crate::postgres::audit_backfill::backfill_audit_organizations(&pool, std::time::Duration::ZERO).await.unwrap(),
            crate::postgres::audit_backfill::BackfillOutcome::AlreadyDone
        );

        let rows: Vec<(String, String, Option<Uuid>)> = sqlx::query_as("SELECT aggregate_type, event_type, organization_id FROM domain_events ORDER BY aggregate_type, event_type, version")
            .fetch_all(&pool)
            .await
            .unwrap();
        let organization_of = |aggregate_type: &str, event_type: &str| -> Vec<Option<Uuid>> {
            rows.iter().filter(|(a, e, _)| a == aggregate_type && e == event_type).map(|(_, _, org)| *org).collect()
        };
        let mut created = organization_of("PackageRepository", "Created");
        created.sort();
        assert_eq!(created, if org_a < org_b { vec![Some(org_a), Some(org_b)] } else { vec![Some(org_b), Some(org_a)] });
        assert_eq!(organization_of("PackageRepository", "QuotaSet"), vec![Some(org_a)]);
        assert_eq!(organization_of("DockerRegistry", "ImagePushed"), vec![Some(org_b)]);
        assert_eq!(organization_of("Permission", "Granted"), vec![Some(org_a)]);
        assert_eq!(organization_of("Security", "AccessDenied"), vec![Some(org_b)], "the actor's organization");
        assert_eq!(organization_of("Security", "OidcLoginFailed"), vec![Some(org_a)]);
        assert_eq!(organization_of("Security", "LoginFailed"), vec![None], "no actor, no organization");
        let mut npm = organization_of("NpmPackage", "PackagePushed");
        npm.extend(organization_of("NpmPackage", "PackageDeleted"));
        assert_eq!(npm, vec![Some(org_a), None], "a package that no longer exists cannot be resolved");
    }
}
