-- Idempotent: the backfill only runs when the column is first added.

-- Only an address the account has proven (or an admin/identity provider vouched for) may be linked by SSO. Self-registration leaves it false.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'users' AND column_name = 'email_verified') THEN
        ALTER TABLE users ADD COLUMN email_verified BOOLEAN NOT NULL DEFAULT false;

        -- Outside the public organization an account can only come from an invitation or SSO. Inside it, self-registered accounts cannot be told apart from the rest, but they had to enrol a second factor to ever log in, while SSO-created ones never have one.
        UPDATE users u SET email_verified = true
        WHERE u.email IS NOT NULL
          AND (
              NOT EXISTS (SELECT 1 FROM organizations o WHERE o.id = u.organization_id AND o.is_public)
              OR (
                  NOT EXISTS (SELECT 1 FROM totp_credentials t WHERE t.user_id = u.id AND t.confirmed)
                  AND NOT EXISTS (SELECT 1 FROM webauthn_credentials w WHERE w.user_id = u.id)
              )
          );
    END IF;
END
$$;

-- An unverified address claims nothing, so it can be registered any number of times. Only verified ones stay unique, per organization and case-insensitively: another tenant's account never blocks or reveals an address.
DO $$
DECLARE
    collision TEXT;
BEGIN
    SELECT lower(email) INTO collision
    FROM users
    WHERE email IS NOT NULL AND email_verified
    GROUP BY organization_id, lower(email)
    HAVING count(*) > 1
    LIMIT 1;

    IF collision IS NOT NULL THEN
        RAISE EXCEPTION 'cannot enforce case-insensitive email uniqueness: % is held by more than one verified account of the same organization. Change or delete the extra accounts, then restart to re-run this migration.', collision;
    END IF;
END
$$;

DROP INDEX IF EXISTS users_email_unique;
CREATE UNIQUE INDEX IF NOT EXISTS users_verified_email_per_organization_unique ON users (organization_id, lower(email)) WHERE email IS NOT NULL AND email_verified;
