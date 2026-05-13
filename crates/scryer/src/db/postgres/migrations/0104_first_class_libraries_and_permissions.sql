CREATE TABLE IF NOT EXISTS libraries (
    id TEXT PRIMARY KEY,
    facet TEXT NOT NULL,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

ALTER TABLE libraries ADD COLUMN IF NOT EXISTS is_default BOOLEAN NOT NULL DEFAULT FALSE;

CREATE UNIQUE INDEX IF NOT EXISTS idx_libraries_facet_slug
    ON libraries(facet, slug);

CREATE TABLE IF NOT EXISTS library_roots (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_library_roots_normalized_path
    ON library_roots(normalized_path);

CREATE INDEX IF NOT EXISTS idx_library_roots_library
    ON library_roots(library_id, is_default DESC, path ASC);

INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
VALUES
    ('movie_default_library', 'movie', 'Movies', 'movies', TRUE, NOW(), NOW()),
    ('series_default_library', 'series', 'Series', 'series', TRUE, NOW(), NOW()),
    ('anime_default_library', 'anime', 'Anime', 'anime', TRUE, NOW(), NOW())
ON CONFLICT (id) DO NOTHING;

INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at)
VALUES
    ('movie_default_library_root', 'movie_default_library', '/data/movies', '/data/movies', TRUE, NOW(), NOW()),
    ('series_default_library_root', 'series_default_library', '/data/series', '/data/series', TRUE, NOW(), NOW()),
    ('anime_default_library_root', 'anime_default_library', '/data/anime', '/data/anime', TRUE, NOW(), NOW())
ON CONFLICT (id) DO NOTHING;

ALTER TABLE titles ADD COLUMN IF NOT EXISTS library_id TEXT;

UPDATE titles
SET library_id = CASE facet
    WHEN 'movie' THEN 'movie_default_library'
    WHEN 'series' THEN 'series_default_library'
    WHEN 'anime' THEN 'anime_default_library'
    ELSE 'movie_default_library'
END
WHERE library_id IS NULL OR BTRIM(library_id) = '';

CREATE INDEX IF NOT EXISTS idx_titles_library_name
    ON titles(library_id, LOWER(name), id);

ALTER TABLE title_external_ids ADD COLUMN IF NOT EXISTS library_id TEXT;

UPDATE title_external_ids
SET library_id = titles.library_id
FROM titles
WHERE titles.id = title_external_ids.title_id
  AND (title_external_ids.library_id IS NULL OR BTRIM(title_external_ids.library_id) = '');

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_external_ids_library_lookup
    ON title_external_ids(library_id, source, external_id);

ALTER TABLE library_scan_unmatched_items ADD COLUMN IF NOT EXISTS library_id TEXT;

UPDATE library_scan_unmatched_items
SET library_id = CASE facet
    WHEN 'movie' THEN 'movie_default_library'
    WHEN 'series' THEN 'series_default_library'
    WHEN 'anime' THEN 'anime_default_library'
    ELSE 'movie_default_library'
END
WHERE library_id IS NULL OR BTRIM(library_id) = '';

CREATE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_library_updated
    ON library_scan_unmatched_items(library_id, updated_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_library_path
    ON library_scan_unmatched_items(library_id, item_path);

CREATE TABLE IF NOT EXISTS user_app_permission_masks (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    permission_mask BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS user_library_permission_masks (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    permission_mask BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, library_id)
);
