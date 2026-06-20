ALTER TABLE download_submissions
    ADD COLUMN actor_kind TEXT;

ALTER TABLE download_submissions
    ADD COLUMN actor_user_id TEXT;

ALTER TABLE download_submissions
    ADD COLUMN actor_display_name TEXT;
