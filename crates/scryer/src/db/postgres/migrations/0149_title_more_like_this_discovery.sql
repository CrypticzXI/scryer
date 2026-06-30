CREATE TABLE IF NOT EXISTS title_more_like_this_items (
    source_title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    id text PRIMARY KEY NOT NULL,
    run_id text NOT NULL,
    base_generation_id text,
    source_run_kind text NOT NULL,
    section_id text,
    sort_index integer NOT NULL DEFAULT 0,
    target_key text NOT NULL,
    target_kind text NOT NULL,
    resolved boolean NOT NULL DEFAULT false,
    resolved_title_id text,
    display_title text NOT NULL,
    original_title text,
    sort_title text,
    year integer,
    poster_path text,
    poster_url text,
    background_url text,
    overview text,
    content_type text,
    rating double precision,
    best_source text,
    source_count integer,
    edge_count integer,
    relation_count integer,
    source_subject_count integer,
    rank_score double precision,
    matched_subject_count integer NOT NULL DEFAULT 0,
    tmdb_collection_id text,
    tmdb_collection_name text,
    owned_in_input boolean NOT NULL DEFAULT false,
    tombstoned_by_run_id text,
    tombstoned_at timestamptz,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    UNIQUE (source_title_id, target_key)
);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_items_source_order
    ON title_more_like_this_items(source_title_id, sort_index ASC, rank_score DESC, id ASC);

CREATE TABLE IF NOT EXISTS title_more_like_this_item_terms (
    item_id text NOT NULL REFERENCES title_more_like_this_items(id) ON DELETE CASCADE,
    source_title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    term_kind text NOT NULL,
    term_category text NOT NULL DEFAULT '',
    term_value text NOT NULL,
    sort_index integer NOT NULL DEFAULT 0,
    PRIMARY KEY (item_id, term_kind, term_category, term_value)
);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_item_terms_source_kind_value
    ON title_more_like_this_item_terms(source_title_id, term_kind, term_value, item_id);
