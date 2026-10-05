-- When each passkey last signed its owner in, shown in the account's security settings. Null until its first use; the
-- passkeys registered before this column have no history and start null too.
ALTER TABLE webauthn_credentials ADD COLUMN IF NOT EXISTS last_used_at TIMESTAMPTZ;
