ALTER TABLE emby_media_server_details ADD COLUMN server_id TEXT;
ALTER TABLE emby_media_server_details ADD COLUMN connect_enabled INTEGER NOT NULL DEFAULT 0 CHECK (connect_enabled IN (0, 1));

CREATE TABLE user_external_accounts_emby_migrated (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    external_user_id TEXT,
    username TEXT NOT NULL,
    display_name TEXT,
    avatar_url TEXT,
    status TEXT NOT NULL,
    verified_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_login_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id),
    CHECK (provider IN ('plex', 'jellyfin', 'emby')),
    CHECK (status IN ('pending_claim', 'active', 'disabled'))
);

INSERT INTO user_external_accounts_emby_migrated (
    id, user_id, provider, connection_id, external_user_id, username,
    display_name, avatar_url, status, verified_at, created_at, updated_at,
    last_login_at
)
SELECT
    id, user_id, provider, connection_id, external_user_id, username,
    display_name, avatar_url, status, verified_at, created_at, updated_at,
    last_login_at
FROM user_external_accounts;

DROP TABLE user_external_accounts;
ALTER TABLE user_external_accounts_emby_migrated RENAME TO user_external_accounts;

CREATE UNIQUE INDEX idx_user_external_accounts_pending_username
    ON user_external_accounts (provider, connection_id, LOWER(username))
    WHERE status = 'pending_claim' AND external_user_id IS NULL;
CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
    ON user_external_accounts (provider, connection_id, external_user_id);
CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
    ON user_external_accounts (user_id, provider, connection_id);
CREATE INDEX idx_user_external_accounts_user_status
    ON user_external_accounts (user_id, status);
