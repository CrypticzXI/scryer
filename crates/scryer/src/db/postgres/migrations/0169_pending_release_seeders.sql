ALTER TABLE pending_releases
    ADD COLUMN IF NOT EXISTS seeders BIGINT;
