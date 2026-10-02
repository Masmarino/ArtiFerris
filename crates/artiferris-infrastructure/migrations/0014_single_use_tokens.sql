-- Tokens that may be used only once (the pending-MFA token of a login), so that a spent one is refused by every
-- instance, not just by the one that saw it. Only a digest is kept, never the token. Rows expire with the token's
-- lifetime and are swept as new ones are written.
CREATE TABLE IF NOT EXISTS spent_single_use_tokens (
    digest BYTEA PRIMARY KEY,
    user_id UUID NOT NULL,
    spent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS spent_single_use_tokens_user_idx ON spent_single_use_tokens (user_id, spent_at);
CREATE INDEX IF NOT EXISTS spent_single_use_tokens_expiry_idx ON spent_single_use_tokens (expires_at);
