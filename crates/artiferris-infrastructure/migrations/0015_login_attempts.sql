-- Failed and in-flight login attempts, counted per throttle key, so that every instance of a deployment judges a
-- login against the same budget and an administrator's unlock reaches all of them. The key is a keyed hash: a client
-- address inside it cannot be read back. `username` and `display_key` are set only for the keys that count a login
-- name (they are what an administrator lists and unlocks). Rows are swept once their window is long past.
CREATE TABLE IF NOT EXISTS login_attempts (
    id BIGSERIAL PRIMARY KEY,
    key_hash BYTEA NOT NULL,
    username TEXT,
    display_key TEXT,
    window_seconds INTEGER NOT NULL,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS login_attempts_key_idx ON login_attempts (key_hash, attempted_at);
CREATE INDEX IF NOT EXISTS login_attempts_username_idx ON login_attempts (username) WHERE username IS NOT NULL;
CREATE INDEX IF NOT EXISTS login_attempts_expiry_idx ON login_attempts (expires_at);
