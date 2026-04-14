CREATE TABLE IF NOT EXISTS external_import_monitor_snapshots (
    facet TEXT PRIMARY KEY
        CHECK (facet IN ('movie', 'series', 'anime')),
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
