CREATE TABLE IF NOT EXISTS indexer_proxy_configs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    protocol TEXT NOT NULL,
    base_url TEXT NOT NULL,
    request_timeout_seconds INTEGER NOT NULL DEFAULT 60,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    last_health_status TEXT,
    last_error_message TEXT,
    last_error_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE indexers
    ADD COLUMN IF NOT EXISTS indexer_proxy_config_id TEXT
    REFERENCES indexer_proxy_configs(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_indexers_indexer_proxy_config_id
    ON indexers(indexer_proxy_config_id);

CREATE INDEX IF NOT EXISTS idx_indexer_proxy_configs_provider_type
    ON indexer_proxy_configs(provider_type);
