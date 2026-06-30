CREATE TABLE IF NOT EXISTS title_rating_summaries (
    title_id TEXT PRIMARY KEY NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    rating REAL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS title_rating_sources (
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (title_id, source)
);

CREATE TABLE IF NOT EXISTS title_external_ratings (
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    value REAL,
    score REAL,
    normalized REAL NOT NULL,
    votes INTEGER,
    url TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (title_id, source)
);

CREATE INDEX IF NOT EXISTS idx_title_rating_sources_title_order
    ON title_rating_sources(title_id, sort_index ASC, source ASC);

CREATE INDEX IF NOT EXISTS idx_title_external_ratings_title_order
    ON title_external_ratings(title_id, sort_index ASC, source ASC);
