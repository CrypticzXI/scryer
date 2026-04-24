CREATE TABLE IF NOT EXISTS subtitle_provider_configs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    config_json TEXT NOT NULL,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    last_health_status TEXT,
    last_error TEXT,
    last_error_at TEXT,
    disabled_until TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_subtitle_provider_configs_provider_type
    ON subtitle_provider_configs(provider_type);

CREATE INDEX IF NOT EXISTS idx_subtitle_provider_configs_enabled
    ON subtitle_provider_configs(is_enabled);

CREATE INDEX IF NOT EXISTS idx_subtitle_provider_configs_disabled_until
    ON subtitle_provider_configs(disabled_until);
