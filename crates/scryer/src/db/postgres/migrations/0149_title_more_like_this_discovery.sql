CREATE TABLE IF NOT EXISTS title_more_like_this_items (
    source_title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    discovery_title_id text NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    sort_index integer NOT NULL DEFAULT 0,
    rank_score double precision,
    best_source text,
    source_count integer,
    edge_count integer,
    relation_count integer,
    source_subject_count integer,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    UNIQUE (source_title_id, discovery_title_id)
);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_items_source_order
    ON title_more_like_this_items(source_title_id, sort_index ASC, rank_score DESC);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_items_title
    ON title_more_like_this_items(discovery_title_id);
