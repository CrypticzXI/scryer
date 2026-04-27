ALTER TABLE library_scan_unmatched_items
    ADD COLUMN status TEXT NOT NULL DEFAULT 'pending';

CREATE INDEX idx_library_scan_unmatched_items_facet_status_updated
    ON library_scan_unmatched_items (facet, status, updated_at DESC);

CREATE INDEX idx_library_scan_unmatched_items_root_status_updated
    ON library_scan_unmatched_items (facet, scan_root, status, updated_at DESC);