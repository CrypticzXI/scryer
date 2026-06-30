CREATE TABLE IF NOT EXISTS title_rating_summaries (
    title_id text PRIMARY KEY NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    rating double precision,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL
);

CREATE TABLE IF NOT EXISTS title_rating_sources (
    title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    source text NOT NULL,
    sort_index integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (title_id, source)
);

CREATE TABLE IF NOT EXISTS title_external_ratings (
    title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    source text NOT NULL,
    sort_index integer NOT NULL DEFAULT 0,
    value double precision,
    score double precision,
    normalized double precision NOT NULL,
    votes integer,
    url text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (title_id, source)
);

CREATE INDEX IF NOT EXISTS idx_title_rating_sources_title_order
    ON title_rating_sources(title_id, sort_index ASC, source ASC);

CREATE INDEX IF NOT EXISTS idx_title_external_ratings_title_order
    ON title_external_ratings(title_id, sort_index ASC, source ASC);
