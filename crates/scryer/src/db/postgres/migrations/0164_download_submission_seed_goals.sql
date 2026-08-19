ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS seeding_profile_id TEXT;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS seed_goal_ratio DOUBLE PRECISION;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS seed_goal_seconds BIGINT;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS seed_never_remove BOOLEAN;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS seed_goal_met_action TEXT;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS seed_goal_source TEXT;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS seed_info_hash TEXT;

CREATE INDEX IF NOT EXISTS idx_download_submissions_seed_info_hash
    ON download_submissions(seed_info_hash);
