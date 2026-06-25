CREATE TABLE IF NOT EXISTS indexer_system_backoffs (
    indexer_id TEXT PRIMARY KEY NOT NULL REFERENCES indexers(id) ON DELETE CASCADE,
    disabled_until TIMESTAMPTZ NOT NULL,
    escalation_level INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_indexer_system_backoffs_disabled_until
    ON indexer_system_backoffs(disabled_until);
