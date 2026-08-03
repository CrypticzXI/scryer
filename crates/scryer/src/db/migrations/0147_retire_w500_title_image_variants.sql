CREATE TEMP TABLE migration_0147_w500_blob_digests (
  digest TEXT PRIMARY KEY
);

INSERT OR IGNORE INTO migration_0147_w500_blob_digests (digest)
SELECT blob_digest
FROM title_image_variants
WHERE variant_key = 'w500';

UPDATE titles
SET poster_local_path = (
  SELECT substr(
           titles.poster_local_path,
           1,
           instr(titles.poster_local_path, '/poster/w500') - 1
         ) || '/poster/w250?v=' ||
         substr(
           CASE
             WHEN instr(tiv.blob_digest, ':') > 0
               THEN substr(tiv.blob_digest, instr(tiv.blob_digest, ':') + 1)
             ELSE tiv.blob_digest
           END,
           1,
           16
         )
  FROM title_images ti
  JOIN title_image_variants tiv ON tiv.title_image_id = ti.id
  WHERE ti.title_id = titles.id
    AND ti.kind = 'poster'
    AND tiv.variant_key = 'w250'
  LIMIT 1
)
WHERE (
    poster_local_path LIKE '%/poster/w500'
    OR poster_local_path LIKE '%/poster/w500?%'
  )
  AND EXISTS (
    SELECT 1
    FROM title_images ti
    JOIN title_image_variants tiv ON tiv.title_image_id = ti.id
    WHERE ti.title_id = titles.id
      AND ti.kind = 'poster'
      AND tiv.variant_key = 'w250'
  );

UPDATE titles
SET poster_local_path = NULL
WHERE poster_local_path LIKE '%/poster/w500'
   OR poster_local_path LIKE '%/poster/w500?%';

DELETE FROM title_image_variants
WHERE variant_key = 'w500';

DELETE FROM title_image_blobs
WHERE digest IN (SELECT digest FROM migration_0147_w500_blob_digests)
  AND NOT EXISTS (
    SELECT 1
    FROM title_image_variants
    WHERE title_image_variants.blob_digest = title_image_blobs.digest
  );

DROP TABLE migration_0147_w500_blob_digests;
