-- A passkey ceremony between its start and its finish: the two requests may reach different instances. The state is
-- the serialized webauthn state, a few hundred bytes with a one-time challenge in it, which lives at most a few minutes.
CREATE TABLE IF NOT EXISTS passkey_ceremonies (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    kind SMALLINT NOT NULL,
    state BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS passkey_ceremonies_user_idx ON passkey_ceremonies (user_id, created_at);
CREATE INDEX IF NOT EXISTS passkey_ceremonies_expiry_idx ON passkey_ceremonies (expires_at);
CREATE INDEX IF NOT EXISTS passkey_ceremonies_age_idx ON passkey_ceremonies (created_at);
