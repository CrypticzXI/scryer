CREATE TABLE IF NOT EXISTS canonical_media_rating_summaries (
    subject_id TEXT PRIMARY KEY NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    rating REAL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS canonical_media_rating_sources (
    subject_id TEXT NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (subject_id, source)
);

CREATE TABLE IF NOT EXISTS canonical_media_external_ratings (
    subject_id TEXT NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    value REAL,
    score REAL,
    normalized REAL NOT NULL,
    votes INTEGER,
    url TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (subject_id, source)
);

INSERT INTO canonical_media_rating_summaries (subject_id, rating, created_at, updated_at)
SELECT DISTINCT dt.canonical_subject_id, dt.rating, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
  FROM discovery_titles dt
 WHERE dt.canonical_subject_id IS NOT NULL
   AND dt.rating IS NOT NULL
ON CONFLICT (subject_id) DO NOTHING;

INSERT INTO canonical_media_rating_sources (subject_id, source, sort_index, created_at, updated_at)
SELECT dt.canonical_subject_id,
       dr.rating_source,
       MIN(dr.sort_index),
       CURRENT_TIMESTAMP,
       CURRENT_TIMESTAMP
  FROM discovery_titles dt
  JOIN discovery_title_ratings dr ON dr.discovery_title_id = dt.id
 WHERE dt.canonical_subject_id IS NOT NULL
   AND TRIM(dr.rating_source) <> ''
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
       CURRENT_TIMESTAMP,
       CURRENT_TIMESTAMP
  FROM discovery_titles dt
  JOIN discovery_title_ratings dr ON dr.discovery_title_id = dt.id
 WHERE dt.canonical_subject_id IS NOT NULL
   AND TRIM(dr.rating_source) <> ''
   AND dr.normalized IS NOT NULL
 GROUP BY dt.canonical_subject_id, dr.rating_source
ON CONFLICT (subject_id, source) DO NOTHING;

INSERT INTO canonical_media_rating_summaries (subject_id, rating, created_at, updated_at)
SELECT s.id,
       trs.rating,
       COALESCE(trs.created_at, CURRENT_TIMESTAMP),
       COALESCE(trs.updated_at, CURRENT_TIMESTAMP)
  FROM canonical_media_subjects s
  JOIN title_rating_summaries trs ON trs.title_id = s.title_id
 WHERE s.title_id IS NOT NULL
ON CONFLICT (subject_id) DO UPDATE SET
    rating = excluded.rating,
    updated_at = excluded.updated_at;

INSERT INTO canonical_media_rating_sources (subject_id, source, sort_index, created_at, updated_at)
SELECT s.id,
       trsrc.source,
       trsrc.sort_index,
       COALESCE(trsrc.created_at, CURRENT_TIMESTAMP),
       COALESCE(trsrc.updated_at, CURRENT_TIMESTAMP)
  FROM canonical_media_subjects s
  JOIN title_rating_sources trsrc ON trsrc.title_id = s.title_id
 WHERE s.title_id IS NOT NULL
   AND TRIM(trsrc.source) <> ''
ON CONFLICT (subject_id, source) DO UPDATE SET
    sort_index = excluded.sort_index,
    updated_at = excluded.updated_at;

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
       COALESCE(ter.created_at, CURRENT_TIMESTAMP),
       COALESCE(ter.updated_at, CURRENT_TIMESTAMP)
  FROM canonical_media_subjects s
  JOIN title_external_ratings ter ON ter.title_id = s.title_id
 WHERE s.title_id IS NOT NULL
   AND TRIM(ter.source) <> ''
ON CONFLICT (subject_id, source) DO UPDATE SET
    sort_index = excluded.sort_index,
    value = excluded.value,
    score = excluded.score,
    normalized = excluded.normalized,
    votes = excluded.votes,
    url = excluded.url,
    updated_at = excluded.updated_at;

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
