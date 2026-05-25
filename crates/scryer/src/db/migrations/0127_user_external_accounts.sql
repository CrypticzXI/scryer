CREATE TABLE user_external_accounts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    display_name TEXT,
    avatar_url TEXT,
    status TEXT NOT NULL,
    verified_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (provider IN ('plex', 'jellyfin')),
    CHECK (status IN ('pending_claim', 'active', 'disabled'))
);

CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
    ON user_external_accounts (provider, connection_id, external_user_id);

CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
    ON user_external_accounts (user_id, provider, connection_id);

CREATE INDEX idx_user_external_accounts_user_status
    ON user_external_accounts (user_id, status);
