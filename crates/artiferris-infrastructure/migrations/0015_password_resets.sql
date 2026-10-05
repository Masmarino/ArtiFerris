-- Links an administrator issues so someone can choose a new password: one per account, the latest replacing the
-- previous one. Only the SHA-256 of the mailed token is stored. The row goes when the link is used, or with the account.
CREATE TABLE IF NOT EXISTS password_resets (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL
);
