ALTER TABLE library_scan_unmatched_items
    ADD COLUMN IF NOT EXISTS title_id TEXT;

CREATE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_facet_title_status_updated
    ON library_scan_unmatched_items (facet, title_id, status, updated_at DESC);
