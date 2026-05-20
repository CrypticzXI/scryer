ALTER TABLE imports
    ADD COLUMN source_client_id TEXT;

ALTER TABLE download_import_artifacts
    ADD COLUMN source_client_id TEXT;

UPDATE imports
SET source_client_id = NULLIF(TRIM(json_extract(payload_json, '$.client_id')), '')
WHERE source_client_id IS NULL
  AND json_valid(payload_json)
  AND json_type(payload_json, '$.client_id') = 'text';

UPDATE download_import_artifacts
SET source_client_id = (
    SELECT imports.source_client_id
    FROM imports
    WHERE imports.id = download_import_artifacts.import_id
)
WHERE source_client_id IS NULL
  AND import_id IS NOT NULL;

DROP INDEX IF EXISTS idx_imports_source_ref;
CREATE UNIQUE INDEX IF NOT EXISTS idx_imports_source_ref
    ON imports (COALESCE(source_client_id, ''), source_system, source_ref, import_type);

DROP INDEX IF EXISTS idx_download_import_artifacts_source;
CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_source
    ON download_import_artifacts (COALESCE(source_client_id, ''), source_system, source_ref, created_at);
