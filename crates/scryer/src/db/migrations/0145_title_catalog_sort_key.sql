ALTER TABLE titles ADD COLUMN catalog_sort_key TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_titles_catalog_sort_key
    ON titles(catalog_sort_key, name, year, id);
