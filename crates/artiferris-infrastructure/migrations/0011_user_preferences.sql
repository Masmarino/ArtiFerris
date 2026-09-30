-- Per-user interface settings. A separate table (not a column of `users`) so that the account row loaded on every authenticated request stays as it is; a user without a row has not chosen a language yet.
CREATE TABLE user_preferences (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    language TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
