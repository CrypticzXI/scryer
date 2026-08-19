ALTER TABLE pending_releases
    ADD COLUMN minimum_seed_ratio REAL;

ALTER TABLE pending_releases
    ADD COLUMN minimum_seed_time_minutes INTEGER;

ALTER TABLE pending_releases
    ADD COLUMN season_pack_seed_ratio REAL;

ALTER TABLE pending_releases
    ADD COLUMN season_pack_seed_time_minutes INTEGER;
