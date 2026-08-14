UPDATE titles
SET metadata_hydration_next_attempt_at = NOW(),
    metadata_hydration_attempt_count = 0
WHERE EXISTS (
    SELECT 1
    FROM title_external_ids
    WHERE title_external_ids.title_id = titles.id
      AND LOWER(TRIM(title_external_ids.source)) = 'tvdb'
      AND TRIM(title_external_ids.external_id) != ''
);
