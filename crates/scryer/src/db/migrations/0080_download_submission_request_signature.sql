ALTER TABLE download_submissions ADD COLUMN source_hint TEXT;
ALTER TABLE download_submissions ADD COLUMN source_kind TEXT;
ALTER TABLE download_submissions ADD COLUMN request_signature TEXT;
ALTER TABLE download_submissions ADD COLUMN episode_id TEXT;

CREATE INDEX IF NOT EXISTS idx_download_submissions_title_request_signature
    ON download_submissions(title_id, request_signature);
