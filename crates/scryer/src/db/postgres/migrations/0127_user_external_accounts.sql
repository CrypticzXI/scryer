CREATE TABLE user_external_accounts (
    id text PRIMARY KEY,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider text NOT NULL CHECK (provider IN ('plex', 'jellyfin')),
    connection_id text NOT NULL,
    external_user_id text,
    username text NOT NULL,
    display_name text,
    avatar_url text,
    status text NOT NULL CHECK (status IN ('pending_claim', 'active', 'disabled')),
    verified_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
    ON user_external_accounts (provider, connection_id, external_user_id);

CREATE UNIQUE INDEX idx_user_external_accounts_pending_username
    ON user_external_accounts (provider, connection_id, LOWER(username))
    WHERE status = 'pending_claim' AND external_user_id IS NULL;

CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
    ON user_external_accounts (user_id, provider, connection_id);

CREATE INDEX idx_user_external_accounts_user_status
    ON user_external_accounts (user_id, status);
