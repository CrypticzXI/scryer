ALTER TABLE indexers ADD COLUMN managed_parent_config_id TEXT;
ALTER TABLE indexers ADD COLUMN managed_child_key TEXT;
ALTER TABLE indexers ADD COLUMN managed_metadata_json TEXT;

CREATE INDEX IF NOT EXISTS idx_indexers_managed_parent ON indexers(managed_parent_config_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_indexers_managed_child_identity
ON indexers(managed_parent_config_id, managed_child_key)
WHERE managed_parent_config_id IS NOT NULL AND managed_child_key IS NOT NULL;

INSERT OR IGNORE INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
VALUES
    ('movie_default_library', 'movie', 'Movies', 'movies', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('series_default_library', 'series', 'Series', 'series', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('anime_default_library', 'anime', 'Anime', 'anime', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

CREATE UNIQUE INDEX IF NOT EXISTS idx_libraries_facet_slug
    ON libraries(facet, slug);

CREATE UNIQUE INDEX IF NOT EXISTS idx_library_roots_normalized_path
    ON library_roots(normalized_path);

CREATE INDEX IF NOT EXISTS idx_library_roots_library
    ON library_roots(library_id, is_default DESC, path ASC);

UPDATE library_roots
SET normalized_path = lower(rtrim(path, '/'))
WHERE normalized_path != lower(rtrim(path, '/'));

CREATE TEMP TABLE _scryer_0105_default_library_bootstrap (
    library_id TEXT PRIMARY KEY,
    bootstrap_path TEXT NOT NULL
);

INSERT INTO _scryer_0105_default_library_bootstrap (library_id, bootstrap_path)
VALUES
    ('movie_default_library', '/data/movies'),
    ('series_default_library', '/data/series'),
    ('anime_default_library', '/data/anime');

CREATE TEMP TABLE _scryer_0105_legacy_default_library_roots (
    library_id TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    is_default INTEGER NOT NULL
);

INSERT INTO _scryer_0105_legacy_default_library_roots (library_id, path, normalized_path, is_default)
SELECT
    CASE sd.key_name
        WHEN 'movies.root_folders' THEN 'movie_default_library'
        WHEN 'series.root_folders' THEN 'series_default_library'
        WHEN 'anime.root_folders' THEN 'anime_default_library'
    END,
    TRIM(json_extract(root.value, '$.path')),
    lower(rtrim(TRIM(json_extract(root.value, '$.path')), '/')),
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

INSERT INTO _scryer_0105_legacy_default_library_roots (library_id, path, normalized_path, is_default)
SELECT
    CASE sd.key_name
        WHEN 'movies.path' THEN 'movie_default_library'
        WHEN 'series.path' THEN 'series_default_library'
        WHEN 'anime.path' THEN 'anime_default_library'
    END,
    CASE
        WHEN json_valid(sv.value_json) THEN TRIM(json_extract(sv.value_json, '$'))
        ELSE TRIM(sv.value_json)
    END,
    lower(rtrim(
        CASE
            WHEN json_valid(sv.value_json) THEN TRIM(json_extract(sv.value_json, '$'))
            ELSE TRIM(sv.value_json)
        END,
        '/'
    )),
    1
FROM settings_values sv
JOIN settings_definitions sd ON sd.id = sv.setting_definition_id
WHERE sd.key_name IN ('movies.path', 'series.path', 'anime.path')
  AND CASE
      WHEN json_valid(sv.value_json) THEN TRIM(COALESCE(json_extract(sv.value_json, '$'), ''))
      ELSE TRIM(COALESCE(sv.value_json, ''))
  END != ''
  AND NOT EXISTS (
      SELECT 1
      FROM _scryer_0105_legacy_default_library_roots existing
      WHERE existing.library_id = CASE sd.key_name
          WHEN 'movies.path' THEN 'movie_default_library'
          WHEN 'series.path' THEN 'series_default_library'
          WHEN 'anime.path' THEN 'anime_default_library'
      END
        AND existing.normalized_path = lower(rtrim(
            CASE
                WHEN json_valid(sv.value_json) THEN TRIM(json_extract(sv.value_json, '$'))
                ELSE TRIM(sv.value_json)
            END,
            '/'
        ))
  );

CREATE TEMP TABLE _scryer_0105_default_library_root_replacements (
    library_id TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    is_default INTEGER NOT NULL
);

INSERT INTO _scryer_0105_default_library_root_replacements (library_id, path, normalized_path, is_default)
WITH legacy_summary AS (
    SELECT
        defaults.library_id,
        COUNT(legacy.library_id) AS legacy_count,
        SUM(CASE WHEN legacy.normalized_path = lower(rtrim(defaults.bootstrap_path, '/')) THEN 1 ELSE 0 END) AS bootstrap_match_count
    FROM _scryer_0105_default_library_bootstrap defaults
    LEFT JOIN _scryer_0105_legacy_default_library_roots legacy
      ON legacy.library_id = defaults.library_id
    GROUP BY defaults.library_id, defaults.bootstrap_path
),
current_summary AS (
    SELECT
        defaults.library_id,
        COUNT(roots.id) AS current_count,
        SUM(CASE WHEN lower(rtrim(roots.path, '/')) = lower(rtrim(defaults.bootstrap_path, '/')) THEN 1 ELSE 0 END) AS bootstrap_match_count
    FROM _scryer_0105_default_library_bootstrap defaults
    LEFT JOIN library_roots roots
      ON roots.library_id = defaults.library_id
    GROUP BY defaults.library_id, defaults.bootstrap_path
),
needs_legacy_replacement AS (
    SELECT current_summary.library_id
    FROM current_summary
    JOIN legacy_summary USING (library_id)
    WHERE (current_summary.current_count = 0 OR current_summary.bootstrap_match_count = current_summary.current_count)
      AND legacy_summary.legacy_count > 0
      AND legacy_summary.bootstrap_match_count < legacy_summary.legacy_count
),
deduped_legacy AS (
    SELECT
        library_id,
        MIN(path) AS path,
        normalized_path,
        MAX(is_default) AS is_default
    FROM _scryer_0105_legacy_default_library_roots
    GROUP BY library_id, normalized_path
),
ranked_legacy AS (
    SELECT
        library_id,
        path,
        normalized_path,
        ROW_NUMBER() OVER (
            PARTITION BY library_id
            ORDER BY CASE WHEN is_default != 0 THEN 0 ELSE 1 END, normalized_path
        ) AS default_rank
    FROM deduped_legacy
)
SELECT
    ranked_legacy.library_id,
    ranked_legacy.path,
    ranked_legacy.normalized_path,
    CASE WHEN ranked_legacy.default_rank = 1 THEN 1 ELSE 0 END
FROM ranked_legacy
JOIN needs_legacy_replacement
  ON needs_legacy_replacement.library_id = ranked_legacy.library_id;

INSERT INTO _scryer_0105_default_library_root_replacements (library_id, path, normalized_path, is_default)
WITH current_summary AS (
    SELECT
        defaults.library_id,
        COUNT(roots.id) AS current_count
    FROM _scryer_0105_default_library_bootstrap defaults
    LEFT JOIN library_roots roots
      ON roots.library_id = defaults.library_id
    GROUP BY defaults.library_id
)
SELECT
    defaults.library_id,
    defaults.bootstrap_path,
    lower(rtrim(defaults.bootstrap_path, '/')),
    1
FROM _scryer_0105_default_library_bootstrap defaults
JOIN current_summary
  ON current_summary.library_id = defaults.library_id
WHERE current_summary.current_count = 0
  AND NOT EXISTS (
      SELECT 1
      FROM _scryer_0105_default_library_root_replacements replacements
      WHERE replacements.library_id = defaults.library_id
  );

DELETE FROM library_roots
WHERE library_id IN (
    SELECT DISTINCT library_id
    FROM _scryer_0105_default_library_root_replacements
);

INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at)
SELECT
    lower(hex(randomblob(16))),
    library_id,
    path,
    normalized_path,
    is_default,
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM _scryer_0105_default_library_root_replacements;

DROP TABLE _scryer_0105_default_library_root_replacements;
DROP TABLE _scryer_0105_legacy_default_library_roots;
DROP TABLE _scryer_0105_default_library_bootstrap;

UPDATE titles
SET library_id = CASE facet
    WHEN 'movie' THEN 'movie_default_library'
    WHEN 'series' THEN 'series_default_library'
    WHEN 'anime' THEN 'anime_default_library'
    ELSE 'movie_default_library'
END
WHERE library_id IS NULL OR TRIM(library_id) = '';

CREATE INDEX IF NOT EXISTS idx_titles_library_name
    ON titles(library_id, LOWER(name), id);

UPDATE title_external_ids
SET library_id = (
    SELECT titles.library_id
    FROM titles
    WHERE titles.id = title_external_ids.title_id
)
WHERE library_id IS NULL OR TRIM(library_id) = '';

DROP INDEX IF EXISTS idx_title_external_ids_facet_lookup;

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_external_ids_library_lookup
    ON title_external_ids(library_id, source, external_id);

UPDATE library_scan_unmatched_items
SET library_id = CASE facet
    WHEN 'movie' THEN 'movie_default_library'
    WHEN 'series' THEN 'series_default_library'
    WHEN 'anime' THEN 'anime_default_library'
    ELSE 'movie_default_library'
END
WHERE library_id IS NULL OR TRIM(library_id) = '';

CREATE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_library_updated
    ON library_scan_unmatched_items(library_id, updated_at DESC);

DROP INDEX IF EXISTS idx_library_scan_unmatched_items_facet_path;

CREATE UNIQUE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_library_path
    ON library_scan_unmatched_items(library_id, item_path);
