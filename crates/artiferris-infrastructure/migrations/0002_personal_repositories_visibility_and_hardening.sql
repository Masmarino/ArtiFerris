-- Idempotent throughout: the tests re-run the username backfill on an already-migrated database,
-- and a dev database that ran the pre-merge 0002-0008 can be repointed by deleting its
-- _sqlx_migrations rows from version 2 up and starting the server again.

-- A personal namespace's own hidden organization — never returned by any organization listing
-- endpoint, never selectable in an organization picker.
ALTER TABLE organizations ADD COLUMN IF NOT EXISTS is_personal BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE package_repository_projections ADD COLUMN IF NOT EXISTS is_public BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;

-- Usernames are lowercased before every lookup and matched exactly, so a row stored as "Florian"
-- can never be found again. Two accounts differing only in case are genuinely ambiguous, so this
-- refuses to continue rather than picking one. The check runs before the unique index below,
-- otherwise the index build would fail first with a message that says nothing actionable.
DO $$
DECLARE
    collision TEXT;
BEGIN
    SELECT lower(username) INTO collision
    FROM users
    GROUP BY lower(username)
    HAVING count(*) > 1
    LIMIT 1;

    IF collision IS NOT NULL THEN
        RAISE EXCEPTION 'cannot normalize usernames: % is held by more than one account differing only in letter case. Rename or merge those accounts manually, then restart to re-run this migration.', collision;
    END IF;
END
$$;

UPDATE users SET username = lower(username) WHERE username <> lower(username);

CREATE UNIQUE INDEX IF NOT EXISTS users_username_lower_unique ON users (lower(username));

-- Supports the hourly sweep that deletes every docker_blob_uploads row past its expires_at, plus its
-- staging file. Without it that sweep is a sequential scan of the whole table on every run.
CREATE INDEX IF NOT EXISTS docker_blob_uploads_expires_at_idx ON docker_blob_uploads (expires_at);

-- Lookups by blob_digest alone: the NOT EXISTS re-checks in blob deletion and the repository
-- deletion sweep, plus the foreign key check whenever a docker_blobs row is deleted. The primary
-- key leads with package_repository_id, so without this each of those descends the index once per
-- repository (Postgres 18 skip scan) instead of once in total.
CREATE INDEX IF NOT EXISTS docker_repository_blobs_blob_digest_idx ON docker_repository_blobs (blob_digest);
