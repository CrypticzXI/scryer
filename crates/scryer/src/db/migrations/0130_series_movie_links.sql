CREATE TABLE movie_entities (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT,
    slug TEXT,
    year INTEGER,
    overview TEXT,
    poster_url TEXT,
    background_url TEXT,
    language TEXT,
    runtime_minutes INTEGER,
    content_status TEXT,
    genres_json TEXT NOT NULL DEFAULT '[]',
    studio TEXT,
    digital_release_date TEXT,
    imdb_id TEXT,
    tvdb_id TEXT,
    tmdb_id TEXT,
    mal_id TEXT,
    anidb_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_movie_entities_tvdb_id
    ON movie_entities(tvdb_id)
    WHERE tvdb_id IS NOT NULL AND tvdb_id <> '';
CREATE INDEX idx_movie_entities_tmdb_id
    ON movie_entities(tmdb_id)
    WHERE tmdb_id IS NOT NULL AND tmdb_id <> '';
CREATE INDEX idx_movie_entities_imdb_id
    ON movie_entities(imdb_id)
    WHERE imdb_id IS NOT NULL AND imdb_id <> '';
CREATE INDEX idx_movie_entities_mal_id
    ON movie_entities(mal_id)
    WHERE mal_id IS NOT NULL AND mal_id <> '';
CREATE INDEX idx_movie_entities_anidb_id
    ON movie_entities(anidb_id)
    WHERE anidb_id IS NOT NULL AND anidb_id <> '';

CREATE TABLE series_movie_links (
    id TEXT PRIMARY KEY NOT NULL,
    series_title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT NOT NULL REFERENCES movie_entities(id) ON DELETE CASCADE,
    placement TEXT,
    narrative_order TEXT,
    after_season INTEGER,
    before_season INTEGER,
    linked_episode_id TEXT REFERENCES episodes(id) ON DELETE SET NULL,
    association_confidence TEXT,
    continuity_status TEXT,
    movie_form TEXT,
    confidence TEXT,
    signal_summary TEXT,
    source TEXT,
    monitored INTEGER NOT NULL DEFAULT 1,
    legacy_collection_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(legacy_collection_id)
);

CREATE UNIQUE INDEX idx_series_movie_links_legacy_collection
    ON series_movie_links(legacy_collection_id)
    WHERE legacy_collection_id IS NOT NULL;
CREATE INDEX idx_series_movie_links_title
    ON series_movie_links(series_title_id);
CREATE INDEX idx_series_movie_links_movie
    ON series_movie_links(movie_entity_id);

CREATE TABLE file_series_movie_link_map (
    file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    series_movie_link_id TEXT NOT NULL REFERENCES series_movie_links(id) ON DELETE CASCADE,
    PRIMARY KEY (file_id, series_movie_link_id)
);

CREATE INDEX idx_file_series_movie_link_map_link
    ON file_series_movie_link_map(series_movie_link_id);

ALTER TABLE wanted_items ADD COLUMN series_movie_link_id TEXT REFERENCES series_movie_links(id);
ALTER TABLE download_submissions ADD COLUMN series_movie_link_id TEXT;
ALTER TABLE workflow_operations ADD COLUMN series_movie_link_id TEXT;

DROP INDEX IF EXISTS idx_wanted_items_movie_unique;

CREATE TEMP TABLE _legacy_series_movies AS
SELECT
    c.id AS collection_id,
    c.title_id,
    CASE
        WHEN NULLIF(c.interstitial_tvdb_id, '') IS NOT NULL
            THEN 'legacy-movie-tvdb-' || c.interstitial_tvdb_id
        WHEN NULLIF(c.interstitial_movie_tmdb_id, '') IS NOT NULL
            THEN 'legacy-movie-tmdb-' || c.interstitial_movie_tmdb_id
        WHEN NULLIF(c.interstitial_imdb_id, '') IS NOT NULL
            THEN 'legacy-movie-imdb-' || c.interstitial_imdb_id
        WHEN NULLIF(c.interstitial_movie_mal_id, '') IS NOT NULL
            THEN 'legacy-movie-mal-' || c.interstitial_movie_mal_id
        WHEN NULLIF(c.interstitial_movie_anidb_id, '') IS NOT NULL
            THEN 'legacy-movie-anidb-' || c.interstitial_movie_anidb_id
        ELSE 'legacy-movie-title-'
            || lower(replace(replace(
                COALESCE(
                    NULLIF(c.interstitial_slug, ''),
                    NULLIF(c.interstitial_sort_title, ''),
                    NULLIF(c.interstitial_name, ''),
                    NULLIF(c.label, ''),
                    c.id
                ),
                ' ',
                '-'
            ), '/', '-'))
            || '-'
            || COALESCE(CAST(c.interstitial_year AS TEXT), 'unknown')
    END AS movie_entity_id,
    COALESCE(NULLIF(c.interstitial_name, ''), NULLIF(c.label, ''), 'Series Movie') AS movie_title,
    c.interstitial_sort_title AS sort_title,
    c.interstitial_slug AS slug,
    c.interstitial_year AS year,
    c.interstitial_overview AS overview,
    c.interstitial_poster_url AS poster_url,
    c.interstitial_language AS language,
    c.interstitial_runtime_minutes AS runtime_minutes,
    c.interstitial_content_status AS content_status,
    COALESCE(NULLIF(c.interstitial_genres_json, ''), '[]') AS genres_json,
    c.interstitial_studio AS studio,
    c.interstitial_digital_release_date AS digital_release_date,
    c.interstitial_imdb_id AS imdb_id,
    c.interstitial_tvdb_id AS tvdb_id,
    c.interstitial_movie_tmdb_id AS tmdb_id,
    c.interstitial_movie_mal_id AS mal_id,
    c.interstitial_movie_anidb_id AS anidb_id,
    c.interstitial_placement AS placement,
    c.narrative_order,
    c.collection_index,
    CASE
        WHEN instr(c.collection_index, '.') > 0
            THEN CAST(substr(c.collection_index, 1, instr(c.collection_index, '.') - 1) AS INTEGER)
        ELSE NULL
    END AS after_season,
    c.interstitial_association_confidence AS association_confidence,
    c.interstitial_continuity_status AS continuity_status,
    c.interstitial_movie_form AS movie_form,
    c.interstitial_confidence AS confidence,
    c.interstitial_signal_summary AS signal_summary,
    COALESCE(c.monitored, 1) AS monitored,
    c.ordered_path,
    c.interstitial_season_episode AS season_episode,
    CASE
        WHEN c.interstitial_season_episode LIKE 'S%E%'
            THEN CAST(substr(c.interstitial_season_episode, 2, instr(c.interstitial_season_episode, 'E') - 2) AS INTEGER)
        ELSE NULL
    END AS linked_season,
    CASE
        WHEN c.interstitial_season_episode LIKE 'S%E%'
            THEN CAST(substr(c.interstitial_season_episode, instr(c.interstitial_season_episode, 'E') + 1) AS INTEGER)
        ELSE NULL
    END AS linked_episode,
    c.created_at,
    COALESCE(c.updated_at, c.created_at) AS updated_at
FROM collections c
WHERE c.collection_type = 'interstitial'
  AND COALESCE(NULLIF(c.interstitial_name, ''), NULLIF(c.label, '')) IS NOT NULL;

CREATE TEMP TABLE _legacy_series_movie_specials AS
SELECT
    legacy.collection_id,
    legacy.title_id,
    COALESCE(
        (
            SELECT s.id
            FROM collections s
            WHERE s.title_id = legacy.title_id
              AND s.collection_type = 'specials'
            ORDER BY s.created_at
            LIMIT 1
        ),
        'legacy-specials-' || legacy.title_id
    ) AS specials_collection_id
FROM _legacy_series_movies legacy;

INSERT OR IGNORE INTO collections (
    id,
    title_id,
    collection_type,
    collection_index,
    label,
    narrative_order,
    monitored,
    created_at,
    updated_at
)
SELECT DISTINCT
    specials_collection_id,
    title_id,
    'specials',
    '0',
    'Specials',
    '0',
    1,
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM _legacy_series_movie_specials;

UPDATE episodes
SET collection_id = (
    SELECT specials_collection_id
    FROM _legacy_series_movie_specials
    WHERE collection_id = episodes.collection_id
)
WHERE collection_id IN (SELECT collection_id FROM _legacy_series_movies);

UPDATE episodes
SET collection_id = (
    SELECT specials.specials_collection_id
    FROM _legacy_series_movie_specials specials
    INNER JOIN _legacy_series_movies legacy
        ON legacy.collection_id = specials.collection_id
    WHERE legacy.title_id = episodes.title_id
      AND legacy.linked_season = CAST(episodes.season_number AS INTEGER)
      AND legacy.linked_episode = CAST(episodes.episode_number AS INTEGER)
    LIMIT 1
)
WHERE EXISTS (
    SELECT 1
    FROM _legacy_series_movies legacy
    WHERE legacy.title_id = episodes.title_id
      AND legacy.linked_season = CAST(episodes.season_number AS INTEGER)
      AND legacy.linked_episode = CAST(episodes.episode_number AS INTEGER)
);

INSERT OR IGNORE INTO movie_entities (
    id,
    title,
    sort_title,
    slug,
    year,
    overview,
    poster_url,
    background_url,
    language,
    runtime_minutes,
    content_status,
    genres_json,
    studio,
    digital_release_date,
    imdb_id,
    tvdb_id,
    tmdb_id,
    mal_id,
    anidb_id,
    created_at,
    updated_at
)
SELECT
    movie_entity_id,
    MIN(movie_title),
    MIN(sort_title),
    MIN(slug),
    MIN(year),
    MIN(overview),
    MIN(poster_url),
    NULL,
    MIN(language),
    MIN(runtime_minutes),
    MIN(content_status),
    MIN(genres_json),
    MIN(studio),
    MIN(digital_release_date),
    MIN(imdb_id),
    MIN(tvdb_id),
    MIN(tmdb_id),
    MIN(mal_id),
    MIN(anidb_id),
    MIN(created_at),
    MAX(updated_at)
FROM _legacy_series_movies
GROUP BY movie_entity_id;

INSERT OR IGNORE INTO series_movie_links (
    id,
    series_title_id,
    movie_entity_id,
    placement,
    narrative_order,
    after_season,
    before_season,
    linked_episode_id,
    association_confidence,
    continuity_status,
    movie_form,
    confidence,
    signal_summary,
    source,
    monitored,
    legacy_collection_id,
    created_at,
    updated_at
)
SELECT
    'legacy-series-movie-' || legacy.collection_id,
    legacy.title_id,
    legacy.movie_entity_id,
    legacy.placement,
    COALESCE(legacy.narrative_order, legacy.collection_index),
    legacy.after_season,
    NULL,
    (
        SELECT e.id
        FROM episodes e
        WHERE e.title_id = legacy.title_id
          AND legacy.linked_season = CAST(e.season_number AS INTEGER)
          AND legacy.linked_episode = CAST(e.episode_number AS INTEGER)
        ORDER BY e.created_at
        LIMIT 1
    ),
    legacy.association_confidence,
    legacy.continuity_status,
    legacy.movie_form,
    legacy.confidence,
    legacy.signal_summary,
    'legacy_interstitial',
    legacy.monitored,
    legacy.collection_id,
    legacy.created_at,
    legacy.updated_at
FROM _legacy_series_movies legacy;

INSERT OR IGNORE INTO file_series_movie_link_map (file_id, series_movie_link_id)
SELECT
    mf.id,
    sml.id
FROM _legacy_series_movies legacy
INNER JOIN series_movie_links sml
    ON sml.legacy_collection_id = legacy.collection_id
INNER JOIN media_files mf
    ON mf.file_path = legacy.ordered_path
WHERE legacy.ordered_path IS NOT NULL;

INSERT OR IGNORE INTO file_episode_map (file_id, episode_id)
SELECT
    mf.id,
    sml.linked_episode_id
FROM _legacy_series_movies legacy
INNER JOIN series_movie_links sml
    ON sml.legacy_collection_id = legacy.collection_id
INNER JOIN media_files mf
    ON mf.file_path = legacy.ordered_path
WHERE legacy.ordered_path IS NOT NULL
  AND sml.linked_episode_id IS NOT NULL;

UPDATE wanted_items
SET
    series_movie_link_id = (
        SELECT id
        FROM series_movie_links
        WHERE legacy_collection_id = wanted_items.collection_id
    ),
    media_type = 'series_movie',
    collection_id = NULL,
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE collection_id IN (SELECT collection_id FROM _legacy_series_movies)
  AND EXISTS (
      SELECT 1
      FROM series_movie_links
      WHERE legacy_collection_id = wanted_items.collection_id
  );

UPDATE download_submissions
SET
    series_movie_link_id = (
        SELECT id
        FROM series_movie_links
        WHERE legacy_collection_id = download_submissions.collection_id
    ),
    collection_id = NULL
WHERE collection_id IN (SELECT collection_id FROM _legacy_series_movies)
  AND EXISTS (
      SELECT 1
      FROM series_movie_links
      WHERE legacy_collection_id = download_submissions.collection_id
  );

UPDATE workflow_operations
SET
    series_movie_link_id = (
        SELECT id
        FROM series_movie_links
        WHERE legacy_collection_id = workflow_operations.collection_id
    ),
    collection_id = NULL
WHERE collection_id IN (SELECT collection_id FROM _legacy_series_movies)
  AND EXISTS (
      SELECT 1
      FROM series_movie_links
      WHERE legacy_collection_id = workflow_operations.collection_id
  );

DELETE FROM collections
WHERE id IN (SELECT collection_id FROM _legacy_series_movies);

CREATE UNIQUE INDEX idx_wanted_items_series_movie_link
    ON wanted_items(series_movie_link_id)
    WHERE series_movie_link_id IS NOT NULL;

CREATE UNIQUE INDEX idx_wanted_items_movie_unique
    ON wanted_items(title_id)
    WHERE episode_id IS NULL
      AND collection_id IS NULL
      AND series_movie_link_id IS NULL;

DROP TABLE _legacy_series_movie_specials;
DROP TABLE _legacy_series_movies;
