CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_retention
    ON download_import_artifacts (created_at, import_id);
