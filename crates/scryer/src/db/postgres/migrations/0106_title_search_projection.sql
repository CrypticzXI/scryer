ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS episode_set_ids TEXT;

CREATE TABLE IF NOT EXISTS import_artifacts (
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_id TEXT,
    relative_path TEXT,
    normalized_file_name TEXT NOT NULL,
    media_kind TEXT NOT NULL,
    title_id TEXT,
    episode_id TEXT,
    season_number BIGINT,
    episode_number BIGINT,
    result TEXT NOT NULL,
    reason_code TEXT,
    imported_media_file_id TEXT,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS download_queue_commands (
    id TEXT PRIMARY KEY,
    action TEXT NOT NULL,
    client_id TEXT NOT NULL DEFAULT '',
    client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    is_history BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL,
    error_text TEXT,
    requested_by_user_id TEXT,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_download_queue_commands_active_unique
    ON download_queue_commands(action, client_id, client_type, download_client_item_id, is_history)
    WHERE status IN ('queued', 'running');

DROP TABLE IF EXISTS title_search_terms;

CREATE TABLE IF NOT EXISTS title_search_terms (
    term_id BIGSERIAL PRIMARY KEY,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    term_kind TEXT NOT NULL,
    raw_term TEXT NOT NULL,
    normalized_term TEXT NOT NULL,
    weight BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_title_search_terms_title
    ON title_search_terms(title_id);

CREATE INDEX IF NOT EXISTS idx_title_search_terms_facet_normalized
    ON title_search_terms(facet, normalized_term);

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_search_terms_unique
    ON title_search_terms(title_id, term_kind, normalized_term);

INSERT INTO title_search_terms (title_id, facet, term_kind, raw_term, normalized_term, weight, updated_at)
SELECT id, facet, 'name', name, lower(name), 0, NOW()
FROM titles;
