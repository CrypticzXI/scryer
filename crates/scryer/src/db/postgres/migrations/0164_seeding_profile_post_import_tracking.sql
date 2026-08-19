ALTER TABLE seeding_profiles
    ADD COLUMN IF NOT EXISTS post_import_tracking TEXT NOT NULL DEFAULT 'park';

ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS seed_post_import_tracking TEXT;
