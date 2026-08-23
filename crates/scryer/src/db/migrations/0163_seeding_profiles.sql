CREATE TABLE IF NOT EXISTS seeding_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    ratio REAL,
    seed_time_minutes INTEGER,
    season_pack_mode TEXT NOT NULL DEFAULT 'inherit',
    season_pack_ratio REAL,
    season_pack_seed_time_minutes INTEGER,
    honor_tracker_minimums INTEGER NOT NULL DEFAULT 1,
    goal_met_action TEXT NOT NULL DEFAULT 'remove_entry',
    never_remove INTEGER NOT NULL DEFAULT 0,
    minimum_seeders INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_seeding_profiles_name
    ON seeding_profiles(LOWER(name));

ALTER TABLE indexers
    ADD COLUMN seeding_profile_id TEXT;

CREATE INDEX IF NOT EXISTS idx_indexers_seeding_profile_id
    ON indexers(seeding_profile_id);
