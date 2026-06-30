CREATE TABLE IF NOT EXISTS title_more_like_this_items (
    source_title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    base_generation_id TEXT,
    source_run_kind TEXT NOT NULL,
    section_id TEXT,
    sort_index INTEGER NOT NULL DEFAULT 0,
    target_key TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0,
    resolved_title_id TEXT,
    display_title TEXT NOT NULL,
    original_title TEXT,
    sort_title TEXT,
    year INTEGER,
    poster_path TEXT,
    poster_url TEXT,
    background_url TEXT,
    overview TEXT,
    content_type TEXT,
    rating REAL,
    best_source TEXT,
    source_count INTEGER,
    edge_count INTEGER,
    relation_count INTEGER,
    source_subject_count INTEGER,
    rank_score REAL,
    matched_subject_count INTEGER NOT NULL DEFAULT 0,
    tmdb_collection_id TEXT,
    tmdb_collection_name TEXT,
    owned_in_input INTEGER NOT NULL DEFAULT 0,
    tombstoned_by_run_id TEXT,
    tombstoned_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (source_title_id, target_key)
);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_items_source_order
    ON title_more_like_this_items(source_title_id, sort_index ASC, rank_score DESC, id ASC);

CREATE TABLE IF NOT EXISTS title_more_like_this_item_terms (
    item_id TEXT NOT NULL REFERENCES title_more_like_this_items(id) ON DELETE CASCADE,
    source_title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    term_kind TEXT NOT NULL,
    term_category TEXT NOT NULL DEFAULT '',
    term_value TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (item_id, term_kind, term_category, term_value)
);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_item_terms_source_kind_value
    ON title_more_like_this_item_terms(source_title_id, term_kind, term_value, item_id);
