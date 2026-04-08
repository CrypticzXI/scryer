CREATE TABLE library_scan_unmatched_items (
    id TEXT PRIMARY KEY,
    facet TEXT NOT NULL,
    scan_session_id TEXT NOT NULL,
    scan_root TEXT NOT NULL,
    item_path TEXT NOT NULL,
    display_name TEXT NOT NULL,
    query TEXT NOT NULL,
    year_hint INTEGER,
    reason_code TEXT NOT NULL,
    error_message TEXT,
    search_attempts_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_library_scan_unmatched_items_facet_path
    ON library_scan_unmatched_items (facet, item_path);

CREATE INDEX idx_library_scan_unmatched_items_facet_updated
    ON library_scan_unmatched_items (facet, updated_at DESC);

CREATE INDEX idx_library_scan_unmatched_items_root_updated
    ON library_scan_unmatched_items (facet, scan_root, updated_at DESC);