ALTER TABLE seeding_profiles
    ADD COLUMN post_import_tracking TEXT NOT NULL DEFAULT 'park';

ALTER TABLE download_submissions
    ADD COLUMN seed_post_import_tracking TEXT;
