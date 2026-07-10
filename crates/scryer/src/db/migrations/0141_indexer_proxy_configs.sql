CREATE TABLE IF NOT EXISTS indexer_proxy_configs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    protocol TEXT NOT NULL,
    base_url TEXT NOT NULL,
    request_timeout_seconds INTEGER NOT NULL DEFAULT 60,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    last_health_status TEXT,
    last_error_message TEXT,
    last_error_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE indexers
    ADD COLUMN indexer_proxy_config_id TEXT;

CREATE INDEX IF NOT EXISTS idx_indexers_indexer_proxy_config_id
    ON indexers(indexer_proxy_config_id);

CREATE INDEX IF NOT EXISTS idx_indexer_proxy_configs_provider_type
    ON indexer_proxy_configs(provider_type);
