CREATE TABLE IF NOT EXISTS seeding_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    ratio DOUBLE PRECISION,
    seed_time_minutes BIGINT,
    season_pack_mode TEXT NOT NULL DEFAULT 'inherit',
    season_pack_ratio DOUBLE PRECISION,
    season_pack_seed_time_minutes BIGINT,
    honor_tracker_minimums BOOLEAN NOT NULL DEFAULT TRUE,
    goal_met_action TEXT NOT NULL DEFAULT 'remove_entry',
    never_remove BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_seeding_profiles_name
    ON seeding_profiles(LOWER(name));

ALTER TABLE indexers
    ADD COLUMN IF NOT EXISTS seeding_profile_id TEXT;

CREATE INDEX IF NOT EXISTS idx_indexers_seeding_profile_id
    ON indexers(seeding_profile_id);
