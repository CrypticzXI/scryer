CREATE TABLE IF NOT EXISTS canonical_media_rating_summaries (
    subject_id text PRIMARY KEY NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    rating double precision,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS canonical_media_rating_sources (
    subject_id text NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    source text NOT NULL,
    sort_index integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    PRIMARY KEY (subject_id, source)
);

CREATE TABLE IF NOT EXISTS canonical_media_external_ratings (
    subject_id text NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    source text NOT NULL,
    sort_index integer NOT NULL DEFAULT 0,
    value double precision,
    score double precision,
    normalized double precision NOT NULL,
    votes integer,
    url text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    PRIMARY KEY (subject_id, source)
);

INSERT INTO canonical_media_rating_summaries (subject_id, rating, created_at, updated_at)
SELECT DISTINCT dt.canonical_subject_id, dt.rating, NOW(), NOW()
  FROM discovery_titles dt
 WHERE dt.canonical_subject_id IS NOT NULL
   AND dt.rating IS NOT NULL
ON CONFLICT (subject_id) DO NOTHING;

INSERT INTO canonical_media_rating_sources (subject_id, source, sort_index, created_at, updated_at)
SELECT dt.canonical_subject_id,
       dr.rating_source,
       MIN(dr.sort_index),
       NOW(),
       NOW()
  FROM discovery_titles dt
  JOIN discovery_title_ratings dr ON dr.discovery_title_id = dt.id
 WHERE dt.canonical_subject_id IS NOT NULL
   AND BTRIM(dr.rating_source) <> ''
 GROUP BY dt.canonical_subject_id, dr.rating_source
ON CONFLICT (subject_id, source) DO NOTHING;

INSERT INTO canonical_media_external_ratings (
    subject_id, source, sort_index, value, score, normalized, votes, url, created_at, updated_at
)
SELECT dt.canonical_subject_id,
       dr.rating_source,
       MIN(dr.sort_index),
       MAX(dr.rating_value),
       MAX(dr.rating_score),
       COALESCE(MAX(dr.normalized), 0.0),
       MAX(dr.votes),
       COALESCE(MAX(dr.url), ''),
       NOW(),
       NOW()
  FROM discovery_titles dt
  JOIN discovery_title_ratings dr ON dr.discovery_title_id = dt.id
 WHERE dt.canonical_subject_id IS NOT NULL
   AND BTRIM(dr.rating_source) <> ''
   AND dr.normalized IS NOT NULL
 GROUP BY dt.canonical_subject_id, dr.rating_source
ON CONFLICT (subject_id, source) DO NOTHING;

INSERT INTO canonical_media_rating_summaries (subject_id, rating, created_at, updated_at)
SELECT s.id,
       trs.rating,
       COALESCE(trs.created_at, NOW()),
       COALESCE(trs.updated_at, NOW())
  FROM canonical_media_subjects s
  JOIN title_rating_summaries trs ON trs.title_id = s.title_id
 WHERE s.title_id IS NOT NULL
ON CONFLICT (subject_id) DO UPDATE SET
    rating = EXCLUDED.rating,
    updated_at = EXCLUDED.updated_at;

INSERT INTO canonical_media_rating_sources (subject_id, source, sort_index, created_at, updated_at)
SELECT s.id,
       trsrc.source,
       trsrc.sort_index,
       COALESCE(trsrc.created_at, NOW()),
       COALESCE(trsrc.updated_at, NOW())
  FROM canonical_media_subjects s
  JOIN title_rating_sources trsrc ON trsrc.title_id = s.title_id
 WHERE s.title_id IS NOT NULL
   AND BTRIM(trsrc.source) <> ''
ON CONFLICT (subject_id, source) DO UPDATE SET
    sort_index = EXCLUDED.sort_index,
    updated_at = EXCLUDED.updated_at;

INSERT INTO canonical_media_external_ratings (
    subject_id, source, sort_index, value, score, normalized, votes, url, created_at, updated_at
)
SELECT s.id,
       ter.source,
       ter.sort_index,
       ter.value,
       ter.score,
       ter.normalized,
       ter.votes,
       ter.url,
       COALESCE(ter.created_at, NOW()),
       COALESCE(ter.updated_at, NOW())
  FROM canonical_media_subjects s
  JOIN title_external_ratings ter ON ter.title_id = s.title_id
 WHERE s.title_id IS NOT NULL
   AND BTRIM(ter.source) <> ''
ON CONFLICT (subject_id, source) DO UPDATE SET
    sort_index = EXCLUDED.sort_index,
    value = EXCLUDED.value,
    score = EXCLUDED.score,
    normalized = EXCLUDED.normalized,
    votes = EXCLUDED.votes,
    url = EXCLUDED.url,
    updated_at = EXCLUDED.updated_at;

CREATE INDEX IF NOT EXISTS idx_canonical_media_rating_sources_order
    ON canonical_media_rating_sources(subject_id, sort_index ASC, source ASC);

CREATE INDEX IF NOT EXISTS idx_canonical_media_external_ratings_order
    ON canonical_media_external_ratings(subject_id, sort_index ASC, source ASC);

CREATE INDEX IF NOT EXISTS idx_canonical_media_external_ratings_source_norm
    ON canonical_media_external_ratings(source, normalized, subject_id);

DROP TABLE IF EXISTS discovery_title_ratings;
DROP TABLE IF EXISTS title_external_ratings;
DROP TABLE IF EXISTS title_rating_sources;
DROP TABLE IF EXISTS title_rating_summaries;
