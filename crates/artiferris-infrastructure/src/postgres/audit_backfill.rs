//! Stamps `domain_events.organization_id` on events that predate migration 0008, in the background after startup. Until it
//! finishes, the organization-scoped audit views miss the events still unstamped. One replica at a time works, holding a lease
//! row in `data_backfills` that it renews after every batch.

use std::time::Duration;

use artiferris_domain::error::DomainError;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error_ext::InfraErr;

/// Roughly five thousand events per batch.
pub const BATCH_BLOCKS: i64 = 150;

const JOB_NAME: &str = "audit_organization_id";

/// A replica that stops heartbeating loses the job after this long.
const LEASE_SECONDS: f64 = 300.0;

/// Walks the table once in physical order, a block range at a time: an id-ordered walk jumps around the heap for every row
/// and was many times slower on 2.5M events. Rows that cannot be attributed stay NULL and are not revisited after a restart.
const STAMP_BLOCKS: &str = "\
WITH batch AS (
    SELECT id, aggregate_type, aggregate_id, event_type, actor_id, payload ->> 'organization_id' AS named_organization
    FROM domain_events
    WHERE ctid >= ('(' || $1 || ',0)')::tid AND ctid < ('(' || $2 || ',0)')::tid
      AND organization_id IS NULL AND aggregate_type IN ('PackageRepository', 'DockerRegistry', 'Permission', 'NpmPackage', 'Security')
), resolved AS (
    SELECT b.id, CASE b.aggregate_type
        WHEN 'PackageRepository' THEN repository_organization(b.aggregate_id)
        WHEN 'DockerRegistry' THEN repository_organization(b.aggregate_id)
        WHEN 'Permission' THEN repository_organization(split_part(b.aggregate_id, ':', 2))
        WHEN 'NpmPackage' THEN (
            SELECT repository_organization(p.package_repository_id::text) FROM npm_packages p
            WHERE p.id = CASE WHEN b.aggregate_id ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' THEN b.aggregate_id::uuid END)
        WHEN 'Security' THEN COALESCE(
            CASE WHEN b.event_type = 'OidcLoginFailed' THEN b.named_organization::uuid END,
            (SELECT u.organization_id FROM users u WHERE u.id = b.actor_id))
    END AS organization_id
    FROM batch b
), stamped AS (
    UPDATE domain_events e SET organization_id = r.organization_id FROM resolved r WHERE e.id = r.id AND r.organization_id IS NOT NULL RETURNING e.id
)
SELECT (SELECT count(*) FROM batch), (SELECT count(*) FROM stamped)";

#[derive(Debug, PartialEq, Eq)]
pub enum BackfillOutcome {
    /// `candidates`: unstamped events of an attributable kind; `stamped`: the ones that got an organization.
    Finished { candidates: u64, stamped: u64, batches: u64 },
    AlreadyDone,
    /// Another replica holds the lease, or took it over mid-run.
    RunningElsewhere,
}

enum Claim {
    Ours { block: i64 },
    Done,
    Held,
}

/// Retries while another replica holds the lease, so a replica that died mid-run is taken over.
pub async fn backfill_until_done(pool: &PgPool, pause: Duration, retry_after: Duration) {
    loop {
        match backfill_audit_organizations(pool, pause).await {
            Ok(BackfillOutcome::RunningElsewhere) => tokio::time::sleep(retry_after).await,
            Ok(_) => return,
            Err(e) => {
                tracing::warn!("audit organization backfill stopped, it continues at the next start: {e}");
                return;
            }
        }
    }
}

/// Holds a pool connection only while a batch runs; `pause` is slept between batches to leave room for real traffic.
pub async fn backfill_audit_organizations(pool: &PgPool, pause: Duration) -> Result<BackfillOutcome, DomainError> {
    let owner = Uuid::new_v4();
    let block = match claim(pool, owner).await? {
        Claim::Done => return Ok(BackfillOutcome::AlreadyDone),
        Claim::Held => return Ok(BackfillOutcome::RunningElsewhere),
        Claim::Ours { block } => block,
    };
    let result = run(pool, owner, block, pause).await;
    if result.is_err() {
        let _ = sqlx::query("UPDATE data_backfills SET lease_owner = NULL, lease_expires_at = NULL WHERE name = $1 AND lease_owner = $2").bind(JOB_NAME).bind(owner).execute(pool).await;
    }
    result
}

async fn claim(pool: &PgPool, owner: Uuid) -> Result<Claim, DomainError> {
    let taken: Option<(Option<i64>,)> = sqlx::query_as(
        "INSERT INTO data_backfills AS job (name, lease_owner, lease_expires_at) VALUES ($1, $2, now() + make_interval(secs => $3))          ON CONFLICT (name) DO UPDATE SET lease_owner = EXCLUDED.lease_owner, lease_expires_at = EXCLUDED.lease_expires_at          WHERE job.completed_at IS NULL AND (job.lease_owner IS NULL OR job.lease_expires_at < now())          RETURNING position",
    )
    .bind(JOB_NAME)
    .bind(owner)
    .bind(LEASE_SECONDS)
    .fetch_optional(pool)
    .await
    .infra_err()?;
    if let Some((position,)) = taken {
        return Ok(Claim::Ours { block: position.unwrap_or(0) });
    }
    let completed: bool = sqlx::query_scalar("SELECT completed_at IS NOT NULL FROM data_backfills WHERE name = $1").bind(JOB_NAME).fetch_one(pool).await.infra_err()?;
    Ok(if completed { Claim::Done } else { Claim::Held })
}

async fn run(pool: &PgPool, owner: Uuid, mut block: i64, pause: Duration) -> Result<BackfillOutcome, DomainError> {
    if block == 0 {
        tracing::info!("stamping the organization on audit events written before it was recorded; the organization audit views fill in as this goes");
    } else {
        tracing::info!(block, "resuming the audit organization backfill");
    }

    let (mut candidates_total, mut stamped_total, mut batches) = (0u64, 0u64, 0u64);
    loop {
        let mut batch = pool.begin().await.infra_err()?;
        // Re-read each round: updated rows land at the end of the table.
        let total_blocks: i64 = sqlx::query_scalar("SELECT pg_relation_size('domain_events') / current_setting('block_size')::bigint").fetch_one(&mut *batch).await.infra_err()?;
        if block >= total_blocks {
            break;
        }
        let (candidates, stamped): (i64, i64) = sqlx::query_as(STAMP_BLOCKS).bind(block).bind(block + BATCH_BLOCKS).fetch_one(&mut *batch).await.infra_err()?;
        let next = block + BATCH_BLOCKS;
        // Saves the position and renews the lease in one go; no row means another replica took the job over.
        let renewed = sqlx::query("UPDATE data_backfills SET position = $3, lease_expires_at = now() + make_interval(secs => $4) WHERE name = $1 AND lease_owner = $2")
            .bind(JOB_NAME)
            .bind(owner)
            .bind(next)
            .bind(LEASE_SECONDS)
            .execute(&mut *batch)
            .await
            .infra_err()?;
        if renewed.rows_affected() == 0 {
            return Ok(BackfillOutcome::RunningElsewhere);
        }
        batch.commit().await.infra_err()?;
        candidates_total += candidates as u64;
        stamped_total += stamped as u64;
        batches += 1;
        block = next;
        if batches % 20 == 0 {
            tracing::info!(block, total_blocks, stamped = stamped_total, "audit organization backfill in progress");
        }
        tokio::time::sleep(pause).await;
    }

    sqlx::query("UPDATE data_backfills SET completed_at = now(), lease_owner = NULL, lease_expires_at = NULL WHERE name = $1 AND lease_owner = $2").bind(JOB_NAME).bind(owner).execute(pool).await.infra_err()?;
    tracing::info!(stamped = stamped_total, "audit organization backfill finished");
    Ok(BackfillOutcome::Finished { candidates: candidates_total, stamped: stamped_total, batches })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    async fn seed_unstamped_security_events(pool: &PgPool, count: i64) -> Uuid {
        let organization_id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations (id, slug, display_name) VALUES ($1, $2, $2)").bind(organization_id).bind(organization_id.simple().to_string()).execute(pool).await.unwrap();
        let actor_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, password_hash, organization_id) VALUES ($1, 'actor', 'x', $2)").bind(actor_id).bind(organization_id).execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version, actor_id) \
             SELECT 'Security', gen_random_uuid()::text, 'AccessDenied', '{}'::jsonb, 1, $1 FROM generate_series(1, $2)",
        )
        .bind(actor_id)
        .bind(count)
        .execute(pool)
        .await
        .unwrap();
        organization_id
    }

    async fn stamped_count(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE organization_id IS NOT NULL").fetch_one(pool).await.unwrap()
    }

    #[sqlx::test]
    async fn every_batch_is_stamped_and_the_job_then_reports_itself_done(pool: PgPool) {
        let organization_id = seed_unstamped_security_events(&pool, 20_000).await;

        let outcome = backfill_audit_organizations(&pool, Duration::ZERO).await.unwrap();

        match outcome {
            BackfillOutcome::Finished { candidates: 20_000, stamped: 20_000, batches } => assert!(batches >= 3, "{batches} batches"),
            other => panic!("{other:?}"),
        }
        let elsewhere: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE organization_id IS DISTINCT FROM $1").bind(organization_id).fetch_one(&pool).await.unwrap();
        assert_eq!(elsewhere, 0);
        assert_eq!(backfill_audit_organizations(&pool, Duration::ZERO).await.unwrap(), BackfillOutcome::AlreadyDone);
    }

    #[sqlx::test]
    async fn an_interrupted_run_continues_after_its_saved_position(pool: PgPool) {
        seed_unstamped_security_events(&pool, 5_000).await;
        let position = 60;
        sqlx::query("INSERT INTO data_backfills (name, position) VALUES ($1, $2)").bind(JOB_NAME).bind(position).execute(&pool).await.unwrap();
        let after_position: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE (ctid::text::point)[0] >= $1").bind(position as f64).fetch_one(&pool).await.unwrap();
        assert!(after_position > 0 && after_position < 5_000, "the seed must straddle the saved position, got {after_position}");

        backfill_audit_organizations(&pool, Duration::ZERO).await.unwrap();

        assert_eq!(stamped_count(&pool).await, after_position, "everything after the saved position, nothing before it");
    }

    #[sqlx::test]
    async fn events_that_cannot_be_attributed_stay_unstamped_and_do_not_block_the_rest(pool: PgPool) {
        let organization_id = seed_unstamped_security_events(&pool, 3).await;
        sqlx::query("INSERT INTO domain_events (aggregate_type, aggregate_id, event_type, payload, version) VALUES ('Security', 'nobody', 'LoginFailed', '{}'::jsonb, 1), ('NpmPackage', 'not-a-uuid', 'PackageDeleted', '{}'::jsonb, 1)")
            .execute(&pool)
            .await
            .unwrap();

        let outcome = backfill_audit_organizations(&pool, Duration::ZERO).await.unwrap();

        assert!(matches!(outcome, BackfillOutcome::Finished { candidates: 5, stamped: 3, .. }), "{outcome:?}");
        let stamped: i64 = sqlx::query_scalar("SELECT count(*) FROM domain_events WHERE organization_id = $1").bind(organization_id).fetch_one(&pool).await.unwrap();
        assert_eq!(stamped, 3);
    }

    async fn hold_lease(pool: &PgPool, expires_in_seconds: f64) -> Uuid {
        let holder = Uuid::new_v4();
        sqlx::query("INSERT INTO data_backfills (name, lease_owner, lease_expires_at) VALUES ($1, $2, now() + make_interval(secs => $3)) ON CONFLICT (name) DO UPDATE SET lease_owner = EXCLUDED.lease_owner, lease_expires_at = EXCLUDED.lease_expires_at")
            .bind(JOB_NAME)
            .bind(holder)
            .bind(expires_in_seconds)
            .execute(pool)
            .await
            .unwrap();
        holder
    }

    #[sqlx::test]
    async fn a_second_replica_leaves_the_work_to_the_one_holding_the_lease(pool: PgPool) {
        seed_unstamped_security_events(&pool, 10).await;
        hold_lease(&pool, 60.0).await;

        let outcome = backfill_audit_organizations(&pool, Duration::ZERO).await.unwrap();

        assert_eq!(outcome, BackfillOutcome::RunningElsewhere);
        assert_eq!(stamped_count(&pool).await, 0);
    }

    #[sqlx::test]
    async fn a_lease_that_expired_is_taken_over_and_the_job_resumes_from_the_saved_position(pool: PgPool) {
        seed_unstamped_security_events(&pool, 10).await;
        hold_lease(&pool, -1.0).await;

        let outcome = backfill_audit_organizations(&pool, Duration::ZERO).await.unwrap();

        assert!(matches!(outcome, BackfillOutcome::Finished { stamped: 10, .. }), "{outcome:?}");
        let (owner, done): (Option<Uuid>, bool) = sqlx::query_as("SELECT lease_owner, completed_at IS NOT NULL FROM data_backfills WHERE name = $1").bind(JOB_NAME).fetch_one(&pool).await.unwrap();
        assert!(owner.is_none() && done, "a finished job gives its lease back");
    }

    #[sqlx::test]
    async fn a_run_stops_when_another_replica_takes_its_lease_over(pool: PgPool) {
        seed_unstamped_security_events(&pool, 20_000).await;
        let running = tokio::spawn({
            let pool = pool.clone();
            async move { backfill_audit_organizations(&pool, Duration::from_millis(400)).await }
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        let thief = hold_lease(&pool, 60.0).await;

        assert_eq!(running.await.unwrap().unwrap(), BackfillOutcome::RunningElsewhere);
        let (owner, done): (Option<Uuid>, bool) = sqlx::query_as("SELECT lease_owner, completed_at IS NOT NULL FROM data_backfills WHERE name = $1").bind(JOB_NAME).fetch_one(&pool).await.unwrap();
        assert_eq!((owner, done), (Some(thief), false));
    }

    #[sqlx::test]
    async fn a_replica_keeps_trying_until_the_other_one_lets_go(pool: PgPool) {
        seed_unstamped_security_events(&pool, 10).await;
        hold_lease(&pool, 0.5).await;

        tokio::time::timeout(Duration::from_secs(10), backfill_until_done(&pool, Duration::ZERO, Duration::from_millis(200))).await.unwrap();

        assert_eq!(stamped_count(&pool).await, 10);
    }

    #[sqlx::test]
    async fn the_pool_stays_usable_between_batches(pool: PgPool) {
        seed_unstamped_security_events(&pool, 20_000).await;
        let single = sqlx::postgres::PgPoolOptions::new().max_connections(1).acquire_timeout(Duration::from_secs(30)).connect_with((*pool.connect_options()).clone()).await.unwrap();
        let running = tokio::spawn({
            let single = single.clone();
            async move { backfill_audit_organizations(&single, Duration::from_secs(1)).await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;

        let started = std::time::Instant::now();
        let _other_work = single.acquire().await.unwrap();

        assert!(started.elapsed() < Duration::from_millis(700), "the backfill kept the only connection through its pause");
        drop(_other_work);
        running.abort();
    }
}
