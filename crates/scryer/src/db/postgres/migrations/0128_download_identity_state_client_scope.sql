UPDATE download_identity_states
SET identity_key =
        'client:' || COALESCE(NULLIF(TRIM(client_id), ''), '') ||
        ':' || LOWER(TRIM(client_type)) ||
        ':download:' || TRIM(download_id),
    updated_at = CURRENT_TIMESTAMP
WHERE download_id IS NOT NULL
  AND TRIM(download_id) <> ''
  AND identity_key = 'download:' || TRIM(download_id)
  AND COALESCE(TRIM(client_type), '') <> ''
  AND LOWER(TRIM(download_id)) NOT LIKE 'scryer-download:%'
  AND NOT (
      LENGTH(TRIM(download_id)) IN (40, 64)
      AND TRIM(download_id) ~ '^[0-9A-Fa-f]+$'
  )
  AND NOT EXISTS (
      SELECT 1
      FROM download_identity_states existing
      WHERE existing.identity_key =
              'client:' || COALESCE(NULLIF(TRIM(download_identity_states.client_id), ''), '') ||
              ':' || LOWER(TRIM(download_identity_states.client_type)) ||
              ':download:' || TRIM(download_identity_states.download_id)
        AND existing.id <> download_identity_states.id
  );
