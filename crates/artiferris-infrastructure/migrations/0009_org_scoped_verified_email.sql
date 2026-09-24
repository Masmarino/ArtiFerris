-- Idempotent. 0006 now creates the per-organization index directly; this only replaces the global one on a database that applied an earlier 0006.
-- The per-organization index is weaker than the global one, so existing data always satisfies it.
DROP INDEX IF EXISTS users_verified_email_unique;
CREATE UNIQUE INDEX IF NOT EXISTS users_verified_email_per_organization_unique ON users (organization_id, lower(email)) WHERE email IS NOT NULL AND email_verified;
