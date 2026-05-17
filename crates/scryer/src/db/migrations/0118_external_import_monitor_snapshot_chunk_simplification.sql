ALTER TABLE external_import_monitor_snapshot_chunks
    RENAME TO external_import_monitor_snapshot_chunks_old;

CREATE TABLE external_import_monitor_snapshot_chunks (
    facet TEXT NOT NULL
        CHECK (facet IN ('movie', 'series', 'anime')),
    entry_kind TEXT NOT NULL
        CHECK (entry_kind IN ('movie', 'series')),
    chunk_index INTEGER NOT NULL,
    payload_ndjson TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (facet, entry_kind, chunk_index)
);

INSERT INTO external_import_monitor_snapshot_chunks (
    facet,
    entry_kind,
    chunk_index,
    payload_ndjson,
    created_at
)
SELECT
    facet,
    entry_kind,
    chunk_index,
    payload_ndjson,
    created_at
FROM external_import_monitor_snapshot_chunks_old
WHERE facet IN ('movie', 'series', 'anime');

DROP TABLE external_import_monitor_snapshot_chunks_old;
