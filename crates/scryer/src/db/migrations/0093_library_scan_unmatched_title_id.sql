ALTER TABLE library_scan_unmatched_items
    ADD COLUMN title_id TEXT;

CREATE INDEX idx_library_scan_unmatched_items_facet_title_status_updated
    ON library_scan_unmatched_items (facet, title_id, status, updated_at DESC);
