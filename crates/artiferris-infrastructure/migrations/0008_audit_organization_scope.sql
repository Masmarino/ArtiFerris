-- Fast on any table size: a nullable column is a catalog change and the indexes build in seconds. Events written before this column existed are stamped by the startup job in `postgres/audit_backfill.rs`; until it has run, the organization-scoped audit views miss them (the instance-wide view is unaffected).

-- The organization an event belongs to, so the audit views filter in SQL. No foreign key: a deleted organization's history stays. NULL means instance-level (pre-login failures, configuration export/import) or unresolvable; only the super-admin's unscoped view shows those.
ALTER TABLE domain_events ADD COLUMN IF NOT EXISTS organization_id UUID;

-- What the event writers stamp repository-scoped events with; a repository's Created event carries its organization for good.
CREATE OR REPLACE FUNCTION repository_organization(repository_id TEXT) RETURNS UUID
LANGUAGE sql STABLE AS $$
    SELECT (payload ->> 'organization_id')::uuid FROM domain_events
    WHERE aggregate_type = 'PackageRepository' AND aggregate_id = repository_id AND event_type = 'Created'
$$;

-- Progress of one-off background data jobs, so a restart resumes. The lease keeps two replicas from running the same job; it expires, so a crashed replica's job is taken over.
CREATE TABLE IF NOT EXISTS data_backfills (
    name TEXT PRIMARY KEY,
    position BIGINT,
    completed_at TIMESTAMPTZ,
    lease_owner UUID,
    lease_expires_at TIMESTAMPTZ
);

-- The org admin's view, newest first, keyset-paged on (occurred_at, id).
CREATE INDEX IF NOT EXISTS domain_events_organization_idx ON domain_events (organization_id, occurred_at DESC, id DESC);
-- The same order for the instance-wide view; replaces the occurred_at-only index.
CREATE INDEX IF NOT EXISTS domain_events_occurred_at_id_idx ON domain_events (occurred_at DESC, id DESC);
DROP INDEX IF EXISTS domain_events_occurred_at_idx;
-- The security log filters on the aggregate type before paging.
CREATE INDEX IF NOT EXISTS domain_events_aggregate_type_idx ON domain_events (aggregate_type, occurred_at DESC, id DESC);
-- The retention sweep only looks at these two aggregates; without this it walks every old package and repository event first.
CREATE INDEX IF NOT EXISTS domain_events_audit_occurred_at_idx ON domain_events (occurred_at) WHERE aggregate_type IN ('Security', 'Admin');
-- Same columns as the unique constraint's own index.
DROP INDEX IF EXISTS domain_events_aggregate_idx;
