ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS download_request_id TEXT;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS download_fingerprint TEXT;

CREATE INDEX IF NOT EXISTS idx_download_submissions_request_id
    ON download_submissions(download_request_id);

CREATE INDEX IF NOT EXISTS idx_download_submissions_fingerprint
    ON download_submissions(download_fingerprint);

ALTER TABLE imports
    ADD COLUMN IF NOT EXISTS download_request_id TEXT;

ALTER TABLE imports
    ADD COLUMN IF NOT EXISTS download_fingerprint TEXT;

CREATE INDEX IF NOT EXISTS idx_imports_download_request_id
    ON imports(download_request_id);

CREATE INDEX IF NOT EXISTS idx_imports_download_fingerprint
    ON imports(download_fingerprint);
