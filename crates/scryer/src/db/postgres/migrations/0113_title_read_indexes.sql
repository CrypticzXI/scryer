CREATE INDEX IF NOT EXISTS idx_titles_facet_library_name_id
    ON titles (facet, library_id, lower(name), id);

CREATE INDEX IF NOT EXISTS idx_titles_facet_slug_library_id
    ON titles (facet, lower(slug), library_id)
    WHERE slug IS NOT NULL;
