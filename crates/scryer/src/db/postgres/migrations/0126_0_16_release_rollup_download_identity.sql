-- Rolled up from postgres/migrations/0126_download_submission_identity.sql
ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS download_id TEXT;

CREATE INDEX IF NOT EXISTS idx_download_submissions_download_id
    ON download_submissions(download_client_id, download_client_type, download_id);

ALTER TABLE imports
    ADD COLUMN IF NOT EXISTS download_id TEXT;

CREATE INDEX IF NOT EXISTS idx_imports_download_id
    ON imports(source_client_id, source_system, download_id);

-- Rolled up from postgres/migrations/0127_download_identity_states.sql
CREATE TABLE IF NOT EXISTS download_identity_states (
    id text PRIMARY KEY,
    identity_key text NOT NULL UNIQUE,
    download_id text,
    client_id text,
    client_type text,
    download_client_item_id text,
    tracked_state text NOT NULL,
    reason text,
    detail text,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CHECK (download_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_download_identity_states_download_id
    ON download_identity_states(client_id, client_type, download_id);

-- Rolled up from postgres/migrations/0128_import_identity_dedupe.sql
DROP INDEX IF EXISTS idx_imports_source_ref;

CREATE UNIQUE INDEX IF NOT EXISTS idx_imports_source_ref
    ON imports (COALESCE(source_client_id, ''), source_system, source_ref, import_type)
    WHERE download_id IS NULL;
