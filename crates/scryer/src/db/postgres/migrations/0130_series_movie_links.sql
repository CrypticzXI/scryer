CREATE TABLE movie_entities (
    id text PRIMARY KEY,
    title text NOT NULL,
    sort_title text,
    slug text,
    year integer,
    overview text,
    poster_url text,
    background_url text,
    language text,
    runtime_minutes integer,
    content_status text,
    genres_json text DEFAULT '[]'::text NOT NULL,
    studio text,
    digital_release_date text,
    imdb_id text,
    tvdb_id text,
    tmdb_id text,
    mal_id text,
    anidb_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
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
    id text PRIMARY KEY,
    series_title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id text NOT NULL REFERENCES movie_entities(id) ON DELETE CASCADE,
    placement text,
    narrative_order text,
    after_season integer,
    before_season integer,
    linked_episode_id text REFERENCES episodes(id) ON DELETE SET NULL,
    association_confidence text,
    continuity_status text,
    movie_form text,
    confidence text,
    signal_summary text,
    source text,
    monitored boolean DEFAULT true NOT NULL,
    legacy_collection_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
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
    file_id text NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    series_movie_link_id text NOT NULL REFERENCES series_movie_links(id) ON DELETE CASCADE,
    PRIMARY KEY (file_id, series_movie_link_id)
);

CREATE INDEX idx_file_series_movie_link_map_link
    ON file_series_movie_link_map(series_movie_link_id);

ALTER TABLE wanted_items ADD COLUMN series_movie_link_id text;
ALTER TABLE wanted_items
    ADD CONSTRAINT wanted_items_series_movie_link_id_fkey
    FOREIGN KEY (series_movie_link_id) REFERENCES series_movie_links(id) ON DELETE SET NULL;

ALTER TABLE download_submissions ADD COLUMN series_movie_link_id text;
ALTER TABLE workflow_operations ADD COLUMN series_movie_link_id text;
ALTER TABLE workflow_operations
    ADD CONSTRAINT workflow_operations_series_movie_link_id_fkey
    FOREIGN KEY (series_movie_link_id) REFERENCES series_movie_links(id) ON DELETE SET NULL;

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
            || regexp_replace(
                lower(COALESCE(
                    NULLIF(c.interstitial_slug, ''),
                    NULLIF(c.interstitial_sort_title, ''),
                    NULLIF(c.interstitial_name, ''),
                    NULLIF(c.label, ''),
                    c.id
                )),
                '[^a-z0-9]+',
                '-',
                'g'
            )
            || '-'
            || COALESCE(c.interstitial_year::text, 'unknown')
    END AS movie_entity_id,
    COALESCE(NULLIF(c.interstitial_name, ''), NULLIF(c.label, ''), 'Series Movie') AS movie_title,
    c.interstitial_sort_title AS sort_title,
    c.interstitial_slug AS slug,
    c.interstitial_year AS year,
    c.interstitial_overview AS overview,
    c.interstitial_poster_url AS poster_url,
    c.interstitial_language AS language,
    c.interstitial_runtime_minutes::integer AS runtime_minutes,
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
        WHEN c.collection_index ~ '^[0-9]+\.'
            THEN split_part(c.collection_index, '.', 1)::integer
        ELSE NULL
    END AS after_season,
    c.interstitial_association_confidence AS association_confidence,
    c.interstitial_continuity_status AS continuity_status,
    c.interstitial_movie_form AS movie_form,
    c.interstitial_confidence AS confidence,
    c.interstitial_signal_summary AS signal_summary,
    COALESCE(c.monitored, true) AS monitored,
    c.ordered_path,
    c.interstitial_season_episode AS season_episode,
    CASE
        WHEN c.interstitial_season_episode ~ '^S[0-9]+E[0-9]+'
            THEN substring(c.interstitial_season_episode from '^S([0-9]+)E')::integer
        ELSE NULL
    END AS linked_season,
    CASE
        WHEN c.interstitial_season_episode ~ '^S[0-9]+E[0-9]+'
            THEN substring(c.interstitial_season_episode from '^S[0-9]+E([0-9]+)')::integer
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

INSERT INTO collections (
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
    true,
    now(),
    now()
FROM _legacy_series_movie_specials
ON CONFLICT (id) DO NOTHING;

UPDATE episodes e
SET collection_id = specials.specials_collection_id
FROM _legacy_series_movie_specials specials
WHERE e.collection_id = specials.collection_id;

UPDATE episodes e
SET collection_id = specials.specials_collection_id
FROM _legacy_series_movie_specials specials
INNER JOIN _legacy_series_movies legacy
    ON legacy.collection_id = specials.collection_id
WHERE legacy.title_id = e.title_id
  AND legacy.linked_season = CASE
      WHEN e.season_number ~ '^[0-9]+$' THEN e.season_number::integer
      ELSE NULL
  END
  AND legacy.linked_episode = CASE
      WHEN e.episode_number ~ '^[0-9]+$' THEN e.episode_number::integer
      ELSE NULL
  END;

INSERT INTO movie_entities (
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
GROUP BY movie_entity_id
ON CONFLICT (id) DO NOTHING;

INSERT INTO series_movie_links (
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
          AND legacy.linked_season = CASE
              WHEN e.season_number ~ '^[0-9]+$' THEN e.season_number::integer
              ELSE NULL
          END
          AND legacy.linked_episode = CASE
              WHEN e.episode_number ~ '^[0-9]+$' THEN e.episode_number::integer
              ELSE NULL
          END
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
FROM _legacy_series_movies legacy
ON CONFLICT (id) DO NOTHING;

INSERT INTO file_series_movie_link_map (file_id, series_movie_link_id)
SELECT
    mf.id,
    sml.id
FROM _legacy_series_movies legacy
INNER JOIN series_movie_links sml
    ON sml.legacy_collection_id = legacy.collection_id
INNER JOIN media_files mf
    ON mf.file_path = legacy.ordered_path
WHERE legacy.ordered_path IS NOT NULL
ON CONFLICT (file_id, series_movie_link_id) DO NOTHING;

INSERT INTO file_episode_map (file_id, episode_id)
SELECT
    mf.id,
    sml.linked_episode_id
FROM _legacy_series_movies legacy
INNER JOIN series_movie_links sml
    ON sml.legacy_collection_id = legacy.collection_id
INNER JOIN media_files mf
    ON mf.file_path = legacy.ordered_path
WHERE legacy.ordered_path IS NOT NULL
  AND sml.linked_episode_id IS NOT NULL
ON CONFLICT (file_id, episode_id) DO NOTHING;

UPDATE wanted_items w
SET
    series_movie_link_id = sml.id,
    media_type = 'series_movie',
    collection_id = NULL,
    updated_at = now()
FROM series_movie_links sml
WHERE sml.legacy_collection_id = w.collection_id
  AND w.collection_id IN (SELECT collection_id FROM _legacy_series_movies);

UPDATE download_submissions ds
SET
    series_movie_link_id = sml.id,
    collection_id = NULL
FROM series_movie_links sml
WHERE sml.legacy_collection_id = ds.collection_id
  AND ds.collection_id IN (SELECT collection_id FROM _legacy_series_movies);

UPDATE workflow_operations wo
SET
    series_movie_link_id = sml.id,
    collection_id = NULL
FROM series_movie_links sml
WHERE sml.legacy_collection_id = wo.collection_id
  AND wo.collection_id IN (SELECT collection_id FROM _legacy_series_movies);

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
