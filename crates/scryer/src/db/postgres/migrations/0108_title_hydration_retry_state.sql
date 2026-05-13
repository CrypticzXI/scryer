ALTER TABLE titles
    ADD COLUMN IF NOT EXISTS metadata_hydration_next_attempt_at TIMESTAMPTZ;

ALTER TABLE titles
    ADD COLUMN IF NOT EXISTS metadata_hydration_attempt_count BIGINT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_titles_metadata_hydration_due
    ON titles(metadata_hydration_next_attempt_at, id)
    WHERE metadata_hydration_next_attempt_at IS NOT NULL;
