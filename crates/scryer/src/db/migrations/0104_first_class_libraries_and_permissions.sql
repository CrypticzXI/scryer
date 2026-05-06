CREATE TABLE libraries (
    id TEXT PRIMARY KEY,
    facet TEXT NOT NULL,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_libraries_facet_slug
    ON libraries(facet, slug);

CREATE TABLE library_roots (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_library_roots_normalized_path
    ON library_roots(normalized_path);

CREATE INDEX idx_library_roots_library
    ON library_roots(library_id, is_default DESC, path ASC);

INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
VALUES
    ('movie_default_library', 'movie', 'Movies', 'movies', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('series_default_library', 'series', 'Series', 'series', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('anime_default_library', 'anime', 'Anime', 'anime', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

CREATE TEMP TABLE _default_library_roots (
    library_id TEXT NOT NULL,
    path TEXT NOT NULL,
    is_default INTEGER NOT NULL
);

INSERT INTO _default_library_roots (library_id, path, is_default)
SELECT
    CASE sd.key_name
        WHEN 'movies.root_folders' THEN 'movie_default_library'
        WHEN 'series.root_folders' THEN 'series_default_library'
        WHEN 'anime.root_folders' THEN 'anime_default_library'
    END,
    TRIM(json_extract(root.value, '$.path')),
    CASE
        WHEN json_extract(root.value, '$.isDefault') THEN 1
        ELSE 0
    END
FROM settings_values sv
JOIN settings_definitions sd ON sd.id = sv.setting_definition_id
JOIN json_each(
    CASE
        WHEN json_valid(sv.value_json) AND json_type(sv.value_json) = 'array' THEN sv.value_json
        ELSE '[]'
    END
) AS root
WHERE sd.key_name IN ('movies.root_folders', 'series.root_folders', 'anime.root_folders')
  AND TRIM(COALESCE(json_extract(root.value, '$.path'), '')) != '';

INSERT INTO _default_library_roots (library_id, path, is_default)
SELECT
    CASE sd.key_name
        WHEN 'movies.path' THEN 'movie_default_library'
        WHEN 'series.path' THEN 'series_default_library'
        WHEN 'anime.path' THEN 'anime_default_library'
    END,
    TRIM(json_extract(sv.value_json, '$')),
    1
FROM settings_values sv
JOIN settings_definitions sd ON sd.id = sv.setting_definition_id
WHERE sd.key_name IN ('movies.path', 'series.path', 'anime.path')
  AND TRIM(COALESCE(json_extract(sv.value_json, '$'), '')) != ''
  AND NOT EXISTS (
      SELECT 1
      FROM _default_library_roots existing
      WHERE existing.library_id = CASE sd.key_name
          WHEN 'movies.path' THEN 'movie_default_library'
          WHEN 'series.path' THEN 'series_default_library'
          WHEN 'anime.path' THEN 'anime_default_library'
      END
        AND existing.path = TRIM(json_extract(sv.value_json, '$'))
  );

INSERT INTO _default_library_roots (library_id, path, is_default)
SELECT 'movie_default_library', '/data/movies', 1
WHERE NOT EXISTS (SELECT 1 FROM _default_library_roots WHERE library_id = 'movie_default_library');

INSERT INTO _default_library_roots (library_id, path, is_default)
SELECT 'series_default_library', '/data/series', 1
WHERE NOT EXISTS (SELECT 1 FROM _default_library_roots WHERE library_id = 'series_default_library');

INSERT INTO _default_library_roots (library_id, path, is_default)
SELECT 'anime_default_library', '/data/anime', 1
WHERE NOT EXISTS (SELECT 1 FROM _default_library_roots WHERE library_id = 'anime_default_library');

INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at)
WITH roots AS (
    SELECT library_id, path, MAX(is_default) AS is_default
    FROM _default_library_roots
    GROUP BY library_id, lower(rtrim(path, '/'))
),
ranked_roots AS (
    SELECT
        library_id,
        path,
        ROW_NUMBER() OVER (
            PARTITION BY library_id
            ORDER BY CASE WHEN is_default != 0 THEN 0 ELSE 1 END, lower(rtrim(path, '/'))
        ) AS default_rank
    FROM roots
)
SELECT
    lower(hex(randomblob(16))),
    library_id,
    path,
    lower(rtrim(path, '/')),
    CASE WHEN default_rank = 1 THEN 1 ELSE 0 END,
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM ranked_roots;

DROP TABLE _default_library_roots;

ALTER TABLE titles ADD COLUMN library_id TEXT;

UPDATE titles
SET library_id = CASE facet
    WHEN 'movie' THEN 'movie_default_library'
    WHEN 'series' THEN 'series_default_library'
    WHEN 'anime' THEN 'anime_default_library'
    ELSE 'movie_default_library'
END
WHERE library_id IS NULL OR TRIM(library_id) = '';

CREATE INDEX idx_titles_library_name
    ON titles(library_id, LOWER(name), id);

ALTER TABLE title_external_ids ADD COLUMN library_id TEXT;

UPDATE title_external_ids
SET library_id = (
    SELECT titles.library_id
    FROM titles
    WHERE titles.id = title_external_ids.title_id
)
WHERE library_id IS NULL OR TRIM(library_id) = '';

DROP INDEX IF EXISTS idx_title_external_ids_facet_lookup;

CREATE UNIQUE INDEX idx_title_external_ids_library_lookup
    ON title_external_ids(library_id, source, external_id);

ALTER TABLE library_scan_unmatched_items ADD COLUMN library_id TEXT;

UPDATE library_scan_unmatched_items
SET library_id = CASE facet
    WHEN 'movie' THEN 'movie_default_library'
    WHEN 'series' THEN 'series_default_library'
    WHEN 'anime' THEN 'anime_default_library'
    ELSE 'movie_default_library'
END
WHERE library_id IS NULL OR TRIM(library_id) = '';

CREATE INDEX idx_library_scan_unmatched_items_library_updated
    ON library_scan_unmatched_items(library_id, updated_at DESC);

DROP INDEX IF EXISTS idx_library_scan_unmatched_items_facet_path;

CREATE UNIQUE INDEX idx_library_scan_unmatched_items_library_path
    ON library_scan_unmatched_items(library_id, item_path);

CREATE TABLE user_app_permission_masks (
    user_id TEXT NOT NULL,
    permission_mask INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE TABLE user_library_permission_masks (
    user_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    permission_mask INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, library_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO user_app_permission_masks (user_id, permission_mask, updated_at)
SELECT user_id, permission_mask, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM (
    SELECT users.id AS user_id,
           (CASE WHEN EXISTS (
                SELECT 1 FROM json_each(CASE WHEN json_valid(users.entitlements) THEN users.entitlements ELSE '[]' END) entitlement
                WHERE CAST(entitlement.value AS TEXT) = 'manage_users'
            ) THEN 3 ELSE 0 END)
           |
           (CASE WHEN EXISTS (
                SELECT 1 FROM json_each(CASE WHEN json_valid(users.entitlements) THEN users.entitlements ELSE '[]' END) entitlement
                WHERE CAST(entitlement.value AS TEXT) = 'manage_config'
            ) THEN 12 ELSE 0 END) AS permission_mask
    FROM users
)
WHERE permission_mask != 0;

INSERT OR IGNORE INTO user_library_permission_masks (user_id, library_id, permission_mask, updated_at)
SELECT user_id, library_id, permission_mask, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM (
    SELECT users.id AS user_id,
           libraries.id AS library_id,
           (CASE WHEN EXISTS (
                SELECT 1 FROM json_each(CASE WHEN json_valid(users.entitlements) THEN users.entitlements ELSE '[]' END) entitlement
                WHERE CAST(entitlement.value AS TEXT) = 'view_catalog'
            ) THEN 1 ELSE 0 END)
           |
           (CASE WHEN EXISTS (
                SELECT 1 FROM json_each(CASE WHEN json_valid(users.entitlements) THEN users.entitlements ELSE '[]' END) entitlement
                WHERE CAST(entitlement.value AS TEXT) = 'manage_title'
            ) THEN 6 ELSE 0 END)
           |
           (CASE WHEN EXISTS (
                SELECT 1 FROM json_each(CASE WHEN json_valid(users.entitlements) THEN users.entitlements ELSE '[]' END) entitlement
                WHERE CAST(entitlement.value AS TEXT) = 'manage_config'
            ) THEN 8 ELSE 0 END) AS permission_mask
    FROM users
    JOIN libraries
)
WHERE permission_mask != 0;
