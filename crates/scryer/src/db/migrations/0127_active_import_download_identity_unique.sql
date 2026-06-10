UPDATE imports
SET status = 'skipped',
    result_json = COALESCE(
        result_json,
        '{"error":"duplicate active download identity import skipped during migration"}'
    ),
    finished_at = COALESCE(finished_at, CURRENT_TIMESTAMP),
    updated_at = CURRENT_TIMESTAMP
WHERE id IN (
    SELECT id
    FROM (
        SELECT
            id,
            ROW_NUMBER() OVER (
                PARTITION BY COALESCE(source_client_id, ''), source_system, download_id
                ORDER BY updated_at DESC, created_at DESC, id DESC
            ) AS duplicate_rank
        FROM imports
        WHERE download_id IS NOT NULL
          AND status IN ('pending', 'running', 'processing')
    ) ranked
    WHERE duplicate_rank > 1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_imports_active_download_id
    ON imports (COALESCE(source_client_id, ''), source_system, download_id)
    WHERE download_id IS NOT NULL
      AND status IN ('pending', 'running', 'processing');
