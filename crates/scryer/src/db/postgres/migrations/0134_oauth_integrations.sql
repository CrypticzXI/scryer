CREATE TABLE IF NOT EXISTS oauth_authorization_codes (
    id TEXT PRIMARY KEY,
    code_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    scope TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_oauth_authorization_codes_user_id
    ON oauth_authorization_codes(user_id);

CREATE INDEX IF NOT EXISTS idx_oauth_authorization_codes_expires_at
    ON oauth_authorization_codes(expires_at);

CREATE TABLE IF NOT EXISTS oauth_refresh_grants (
    id TEXT PRIMARY KEY,
    family_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    auth_session_version TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revoked_reason TEXT
);

CREATE INDEX IF NOT EXISTS idx_oauth_refresh_grants_user_id
    ON oauth_refresh_grants(user_id);

CREATE INDEX IF NOT EXISTS idx_oauth_refresh_grants_family_id
    ON oauth_refresh_grants(family_id);

CREATE TABLE IF NOT EXISTS oauth_refresh_tokens (
    id TEXT PRIMARY KEY,
    grant_id TEXT NOT NULL REFERENCES oauth_refresh_grants(id) ON DELETE CASCADE,
    family_id TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_oauth_refresh_tokens_grant_id
    ON oauth_refresh_tokens(grant_id);

CREATE INDEX IF NOT EXISTS idx_oauth_refresh_tokens_family_id
    ON oauth_refresh_tokens(family_id);
