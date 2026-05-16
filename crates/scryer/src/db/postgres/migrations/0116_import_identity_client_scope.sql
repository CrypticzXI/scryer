ALTER TABLE imports
    ADD COLUMN IF NOT EXISTS source_client_id text;

ALTER TABLE download_import_artifacts
    ADD COLUMN IF NOT EXISTS source_client_id text;

UPDATE imports
SET source_client_id = NULLIF(BTRIM(payload_json->>'client_id'), '')
WHERE source_client_id IS NULL
  AND jsonb_typeof(payload_json) = 'object'
  AND payload_json ? 'client_id';

UPDATE download_import_artifacts
SET source_client_id = imports.source_client_id
FROM imports
WHERE download_import_artifacts.source_client_id IS NULL
  AND download_import_artifacts.import_id = imports.id;

DROP INDEX IF EXISTS idx_imports_source_ref;
CREATE UNIQUE INDEX IF NOT EXISTS idx_imports_source_ref
    ON imports ((COALESCE(source_client_id, '')), source_system, source_ref, import_type);

DROP INDEX IF EXISTS idx_download_import_artifacts_source;
CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_source
    ON download_import_artifacts ((COALESCE(source_client_id, '')), source_system, source_ref, created_at);
