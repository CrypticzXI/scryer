CREATE TABLE IF NOT EXISTS external_import_monitor_snapshot_chunks (
    facet TEXT NOT NULL
        CHECK (facet IN ('movie', 'series', 'anime')),
    entry_kind TEXT NOT NULL
        CHECK (entry_kind IN ('movie', 'series')),
    chunk_index INTEGER NOT NULL,
    payload_ndjson TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (facet, entry_kind, chunk_index)
);
