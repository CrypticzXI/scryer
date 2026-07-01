CREATE TABLE IF NOT EXISTS title_more_like_this_items (
    source_title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL DEFAULT 0,
    rank_score REAL,
    best_source TEXT,
    source_count INTEGER,
    edge_count INTEGER,
    relation_count INTEGER,
    source_subject_count INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (source_title_id, discovery_title_id)
);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_items_source_order
    ON title_more_like_this_items(source_title_id, sort_index ASC, rank_score DESC);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_items_title
    ON title_more_like_this_items(discovery_title_id);
