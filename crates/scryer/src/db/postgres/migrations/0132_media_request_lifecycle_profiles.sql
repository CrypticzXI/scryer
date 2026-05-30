ALTER TABLE media_requests
    DROP CONSTRAINT IF EXISTS media_requests_status_check;

ALTER TABLE media_requests
    ADD COLUMN requested_quality_profile_id text,
    ADD COLUMN requested_quality_profile_name text,
    ADD COLUMN resolved_by_user_id text REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN resolved_at timestamp with time zone,
    ADD COLUMN created_title_id text REFERENCES titles(id) ON DELETE SET NULL,
    ADD COLUMN approved_quality_profile_id text,
    ADD COLUMN approved_quality_profile_name text,
    ADD CONSTRAINT media_requests_status_check CHECK (status IN ('pending', 'approved', 'rejected'));

CREATE INDEX idx_media_requests_created_title
    ON media_requests (created_title_id);
