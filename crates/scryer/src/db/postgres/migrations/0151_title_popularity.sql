ALTER TABLE titles ADD COLUMN IF NOT EXISTS popularity double precision;

CREATE INDEX IF NOT EXISTS idx_titles_popularity
    ON titles(popularity);
