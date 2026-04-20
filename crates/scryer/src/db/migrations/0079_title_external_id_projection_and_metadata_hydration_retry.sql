ALTER TABLE titles ADD COLUMN metadata_hydration_next_attempt_at TEXT;
ALTER TABLE titles ADD COLUMN metadata_hydration_attempt_count INTEGER NOT NULL DEFAULT 0;

ALTER TABLE title_external_ids ADD COLUMN facet TEXT;

CREATE TEMP TABLE _title_external_id_projection_check (
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    UNIQUE(facet, source, external_id)
);

INSERT INTO _title_external_id_projection_check (
    title_id,
    facet,
    source,
    external_id
)
SELECT
    canonical.title_id,
    canonical.facet,
    canonical.source,
    canonical.external_id
FROM (
    SELECT
        t.id AS title_id,
        t.facet AS facet,
        LOWER(TRIM(json_extract(external_id.value, '$.source'))) AS source,
        TRIM(json_extract(external_id.value, '$.value')) AS external_id
    FROM titles AS t
    JOIN json_each(t.external_ids) AS external_id
    WHERE TRIM(COALESCE(json_extract(external_id.value, '$.source'), '')) != ''
      AND TRIM(COALESCE(json_extract(external_id.value, '$.value'), '')) != ''
    GROUP BY
        t.id,
        t.facet,
        LOWER(TRIM(json_extract(external_id.value, '$.source'))),
        TRIM(json_extract(external_id.value, '$.value'))
) AS canonical;

DELETE FROM title_external_ids;

DROP INDEX IF EXISTS idx_title_external_ids_lookup;

INSERT INTO title_external_ids (
    id,
    title_id,
    facet,
    source,
    external_id,
    created_at,
    updated_at
)
SELECT
    lower(hex(randomblob(16))),
    projected.title_id,
    projected.facet,
    projected.source,
    projected.external_id,
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM _title_external_id_projection_check AS projected;

DROP TABLE _title_external_id_projection_check;

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_external_ids_facet_lookup
    ON title_external_ids(facet, source, external_id);

CREATE INDEX IF NOT EXISTS idx_title_external_ids_title_id
    ON title_external_ids(title_id);

CREATE INDEX IF NOT EXISTS idx_titles_metadata_hydration_due
    ON titles(metadata_hydration_next_attempt_at, metadata_fetched_at);

UPDATE titles
SET metadata_hydration_next_attempt_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    metadata_hydration_attempt_count = 0
WHERE metadata_fetched_at IS NULL
  AND EXISTS (
      SELECT 1
      FROM json_each(titles.external_ids) AS external_id
      WHERE LOWER(TRIM(COALESCE(json_extract(external_id.value, '$.source'), ''))) = 'tvdb'
        AND TRIM(COALESCE(json_extract(external_id.value, '$.value'), '')) != ''
  );
