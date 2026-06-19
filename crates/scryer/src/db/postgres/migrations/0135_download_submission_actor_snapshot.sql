ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS actor_kind text;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS actor_user_id text;

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS actor_display_name text;
