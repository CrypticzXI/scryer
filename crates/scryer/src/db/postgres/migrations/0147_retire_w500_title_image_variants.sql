CREATE TEMP TABLE migration_0147_w500_blob_digests (
  digest text PRIMARY KEY
);

INSERT INTO migration_0147_w500_blob_digests (digest)
SELECT DISTINCT blob_digest
FROM title_image_variants
WHERE variant_key = 'w500'
ON CONFLICT (digest) DO NOTHING;

UPDATE titles t
SET poster_local_path = split_part(t.poster_local_path, '/poster/w500', 1) ||
    '/poster/w250?v=' ||
    left(
      CASE
        WHEN position(':' in tiv.blob_digest) > 0
          THEN split_part(tiv.blob_digest, ':', 2)
        ELSE tiv.blob_digest
      END,
      16
    )
FROM title_images ti
JOIN title_image_variants tiv ON tiv.title_image_id = ti.id
WHERE ti.title_id = t.id
  AND ti.kind = 'poster'
  AND tiv.variant_key = 'w250'
  AND t.poster_local_path LIKE '%/poster/w500%';

UPDATE titles
SET poster_local_path = NULL
WHERE poster_local_path LIKE '%/poster/w500%';

DELETE FROM title_image_variants
WHERE variant_key = 'w500';

DELETE FROM title_image_blobs tib
USING migration_0147_w500_blob_digests retired
WHERE tib.digest = retired.digest
  AND NOT EXISTS (
    SELECT 1
    FROM title_image_variants tiv
    WHERE tiv.blob_digest = tib.digest
  );

DROP TABLE migration_0147_w500_blob_digests;
