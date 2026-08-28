CREATE TABLE IF NOT EXISTS indexer_search_runs (
    id TEXT PRIMARY KEY,
    indexer_id TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    query_signature TEXT NOT NULL,
    branch TEXT NOT NULL,
    page INTEGER,
    -- Reserved for the per-strategy search corpus (plan 151): the provider
    -- offset this run requested and the next offset it advertised. Nothing
    -- reads or writes them yet.
    provider_offset INTEGER,
    next_provider_offset INTEGER,
    range_min_size INTEGER,
    range_max_size INTEGER,
    result_count INTEGER NOT NULL,
    completion_state TEXT NOT NULL,
    retry_at TEXT,
    error_summary TEXT,
    indexer_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL
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
    size_bytes INTEGER,
    published_at TEXT,
    source_kind TEXT,
    thumbs_up INTEGER,
    thumbs_down INTEGER,
    grabs INTEGER,
    response_tvdb_id TEXT,
    response_tmdb_id TEXT,
    response_imdb_id TEXT,
    season INTEGER,
    episode INTEGER,
    absolute_episode INTEGER,
    release_group TEXT,
    provider_source TEXT,
    info_hash TEXT,
    seeders INTEGER,
    peers INTEGER,
    download_volume_factor REAL,
    upload_volume_factor REAL,
    protected INTEGER,
    created_at TEXT NOT NULL,
    reusable_until TEXT NOT NULL,
    expires_at TEXT NOT NULL
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
