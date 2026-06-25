CREATE TABLE IF NOT EXISTS indexer_system_backoffs (
    indexer_id TEXT PRIMARY KEY NOT NULL,
    disabled_until TEXT NOT NULL,
    escalation_level INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(indexer_id) REFERENCES indexers(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_indexer_system_backoffs_disabled_until
    ON indexer_system_backoffs(disabled_until);
