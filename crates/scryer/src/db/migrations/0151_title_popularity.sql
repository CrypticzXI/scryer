ALTER TABLE titles ADD COLUMN popularity REAL;

CREATE INDEX IF NOT EXISTS idx_titles_popularity
    ON titles(popularity);
