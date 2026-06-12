-- Rolled up from migrations/0126_download_submission_identity.sql
ALTER TABLE download_submissions
    ADD COLUMN download_id TEXT;

CREATE INDEX IF NOT EXISTS idx_download_submissions_download_id
    ON download_submissions(download_client_id, download_client_type, download_id);

ALTER TABLE imports
    ADD COLUMN download_id TEXT;

CREATE INDEX IF NOT EXISTS idx_imports_download_id
    ON imports(source_client_id, source_system, download_id);

-- Rolled up from migrations/0127_download_identity_states.sql
CREATE TABLE IF NOT EXISTS download_identity_states (
    id TEXT PRIMARY KEY,
    identity_key TEXT NOT NULL UNIQUE,
    download_id TEXT,
    client_id TEXT,
    client_type TEXT,
    download_client_item_id TEXT,
    tracked_state TEXT NOT NULL,
    reason TEXT,
    detail TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (download_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_download_identity_states_download_id
    ON download_identity_states(client_id, client_type, download_id);

-- Rolled up from migrations/0128_import_identity_dedupe.sql
DROP INDEX IF EXISTS idx_imports_source_ref;

CREATE UNIQUE INDEX IF NOT EXISTS idx_imports_source_ref
    ON imports (COALESCE(source_client_id, ''), source_system, source_ref, import_type)
    WHERE download_id IS NULL;
