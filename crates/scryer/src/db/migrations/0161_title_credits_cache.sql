CREATE TABLE IF NOT EXISTS title_credits (
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    person_id TEXT NOT NULL,
    person_name TEXT NOT NULL DEFAULT '',
    person_original_name TEXT NOT NULL DEFAULT '',
    person_image_url TEXT NOT NULL DEFAULT '',
    person_source TEXT NOT NULL DEFAULT '',
    person_external_id TEXT NOT NULL DEFAULT '',
    character_name TEXT NOT NULL DEFAULT '',
    language TEXT NOT NULL DEFAULT '',
    billing_order INTEGER NOT NULL DEFAULT 0,
    episode_count INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (title_id, position)
);

CREATE INDEX IF NOT EXISTS idx_title_credits_title_kind
    ON title_credits(title_id, kind);
