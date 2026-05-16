CREATE TABLE IF NOT EXISTS external_import_monitor_snapshot_chunks (
    scope_kind TEXT NOT NULL
        CHECK (scope_kind IN ('warmup_session', 'facet')),
    scope_key TEXT NOT NULL,
    entry_kind TEXT NOT NULL
        CHECK (entry_kind IN ('movie', 'series')),
    chunk_index INTEGER NOT NULL,
    payload_ndjson TEXT NOT NULL,
    entry_count INTEGER NOT NULL,
    byte_len BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (scope_kind, scope_key, entry_kind, chunk_index)
);
