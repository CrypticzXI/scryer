ALTER TABLE indexer_search_runs ADD COLUMN IF NOT EXISTS search_session_id TEXT NOT NULL DEFAULT '';

ALTER TABLE indexer_search_candidates ADD COLUMN IF NOT EXISTS search_session_id TEXT NOT NULL DEFAULT '';
ALTER TABLE indexer_search_candidates ADD COLUMN IF NOT EXISTS session_identity_hash TEXT NOT NULL DEFAULT '';

CREATE UNIQUE INDEX IF NOT EXISTS indexer_search_candidates_session_identity_unique
    ON indexer_search_candidates (search_session_id, session_identity_hash)
    WHERE search_session_id <> '';
