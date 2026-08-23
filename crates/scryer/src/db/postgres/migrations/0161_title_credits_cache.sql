CREATE TABLE IF NOT EXISTS title_credits (
    title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    position integer NOT NULL,
    kind text NOT NULL,
    person_id text NOT NULL,
    person_name text NOT NULL DEFAULT '',
    person_original_name text NOT NULL DEFAULT '',
    person_image_url text NOT NULL DEFAULT '',
    person_source text NOT NULL DEFAULT '',
    person_external_id text NOT NULL DEFAULT '',
    character_name text NOT NULL DEFAULT '',
    language text NOT NULL DEFAULT '',
    billing_order integer NOT NULL DEFAULT 0,
    episode_count integer,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    PRIMARY KEY (title_id, position)
);

CREATE INDEX IF NOT EXISTS idx_title_credits_title_kind
    ON title_credits(title_id, kind);
