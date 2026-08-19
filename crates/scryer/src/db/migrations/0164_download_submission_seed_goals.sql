ALTER TABLE download_submissions
    ADD COLUMN seeding_profile_id TEXT;

ALTER TABLE download_submissions
    ADD COLUMN seed_goal_ratio REAL;

ALTER TABLE download_submissions
    ADD COLUMN seed_goal_seconds INTEGER;

ALTER TABLE download_submissions
    ADD COLUMN seed_never_remove INTEGER;

ALTER TABLE download_submissions
    ADD COLUMN seed_goal_met_action TEXT;

ALTER TABLE download_submissions
    ADD COLUMN seed_goal_source TEXT;

ALTER TABLE download_submissions
    ADD COLUMN seed_info_hash TEXT;

CREATE INDEX IF NOT EXISTS idx_download_submissions_seed_info_hash
    ON download_submissions(seed_info_hash);
