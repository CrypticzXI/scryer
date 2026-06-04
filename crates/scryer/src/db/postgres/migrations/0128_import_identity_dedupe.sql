DROP INDEX IF EXISTS idx_imports_source_ref;

CREATE UNIQUE INDEX IF NOT EXISTS idx_imports_source_ref
    ON imports (COALESCE(source_client_id, ''), source_system, source_ref, import_type)
    WHERE download_request_id IS NULL
      AND download_fingerprint IS NULL;
