CREATE TABLE IF NOT EXISTS indexer_search_runs (
    id TEXT PRIMARY KEY,
    indexer_id TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    query_signature TEXT NOT NULL,
    branch TEXT NOT NULL,
    page INTEGER,
    range_min_size BIGINT,
    range_max_size BIGINT,
    result_count INTEGER NOT NULL,
    completion_state TEXT NOT NULL,
    retry_at TIMESTAMPTZ,
    error_summary TEXT,
    indexer_fingerprint TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_runs_scope_created
    ON indexer_search_runs(scope_key, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_indexer_search_runs_indexer_created
    ON indexer_search_runs(indexer_id, created_at DESC);

CREATE TABLE IF NOT EXISTS indexer_search_candidates (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES indexer_search_runs(id) ON DELETE CASCADE,
    indexer_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    query_signature TEXT NOT NULL,
    provider_ref TEXT,
    source TEXT NOT NULL,
    title TEXT NOT NULL,
    download_url TEXT,
    link_url TEXT,
    size_bytes BIGINT,
    published_at TEXT,
    source_kind TEXT,
    thumbs_up INTEGER,
    thumbs_down INTEGER,
    grabs BIGINT,
    response_tvdb_id TEXT,
    response_tmdb_id TEXT,
    response_imdb_id TEXT,
    season BIGINT,
    episode BIGINT,
    absolute_episode BIGINT,
    release_group TEXT,
    provider_source TEXT,
    info_hash TEXT,
    seeders BIGINT,
    peers BIGINT,
    download_volume_factor DOUBLE PRECISION,
    upload_volume_factor DOUBLE PRECISION,
    protected BOOLEAN,
    created_at TIMESTAMPTZ NOT NULL,
    reusable_until TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS indexer_search_candidate_values (
    candidate_id TEXT NOT NULL REFERENCES indexer_search_candidates(id) ON DELETE CASCADE,
    value_kind TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (candidate_id, value_kind, ordinal)
);

CREATE TABLE IF NOT EXISTS indexer_search_candidate_url_credentials (
    candidate_id TEXT NOT NULL REFERENCES indexer_search_candidates(id) ON DELETE CASCADE,
    url_kind TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    query_key TEXT NOT NULL,
    PRIMARY KEY (candidate_id, url_kind, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_candidates_scope_expiry
    ON indexer_search_candidates(scope_key, expires_at);
CREATE INDEX IF NOT EXISTS idx_indexer_search_candidates_scope_reusable
    ON indexer_search_candidates(indexer_id, scope_key, reusable_until);
CREATE INDEX IF NOT EXISTS idx_indexer_search_candidates_run
    ON indexer_search_candidates(run_id);
