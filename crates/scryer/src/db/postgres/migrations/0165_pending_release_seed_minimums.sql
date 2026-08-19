ALTER TABLE pending_releases
    ADD COLUMN IF NOT EXISTS minimum_seed_ratio DOUBLE PRECISION;

ALTER TABLE pending_releases
    ADD COLUMN IF NOT EXISTS minimum_seed_time_minutes BIGINT;

ALTER TABLE pending_releases
    ADD COLUMN IF NOT EXISTS season_pack_seed_ratio DOUBLE PRECISION;

ALTER TABLE pending_releases
    ADD COLUMN IF NOT EXISTS season_pack_seed_time_minutes BIGINT;
