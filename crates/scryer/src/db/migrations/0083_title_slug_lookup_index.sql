CREATE INDEX IF NOT EXISTS idx_titles_facet_normalized_slug
ON titles (facet, LOWER(TRIM(slug)))
WHERE slug IS NOT NULL AND TRIM(slug) <> '';