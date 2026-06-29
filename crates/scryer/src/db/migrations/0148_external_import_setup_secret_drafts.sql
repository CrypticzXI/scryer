CREATE TABLE IF NOT EXISTS external_import_setup_secret_drafts (
    draft_key TEXT PRIMARY KEY NOT NULL
        CHECK (draft_key = 'active'),
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS external_import_setup_instance_api_keys (
    draft_key TEXT NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    instance_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('sonarr', 'radarr', 'prowlarr')),
    api_key_encrypted TEXT NOT NULL,
    position INTEGER NOT NULL
        CHECK (position >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (draft_key, instance_id),
    UNIQUE (draft_key, position)
);

CREATE TABLE IF NOT EXISTS external_import_setup_download_client_api_key_overrides (
    draft_key TEXT NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key TEXT NOT NULL,
    api_key_encrypted TEXT NOT NULL,
    position INTEGER NOT NULL
        CHECK (position >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);

CREATE TABLE IF NOT EXISTS external_import_setup_download_client_password_overrides (
    draft_key TEXT NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key TEXT NOT NULL,
    password_encrypted TEXT NOT NULL,
    position INTEGER NOT NULL
        CHECK (position >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);

CREATE TABLE IF NOT EXISTS external_import_setup_indexer_api_key_overrides (
    draft_key TEXT NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key TEXT NOT NULL,
    api_key_encrypted TEXT NOT NULL,
    position INTEGER NOT NULL
        CHECK (position >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);
