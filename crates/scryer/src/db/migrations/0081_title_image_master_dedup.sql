UPDATE titles
SET banner_local_path = (
    WITH selected_image AS (
        SELECT
            ti.master_sha256 AS master_sha256,
            ti.storage_mode AS storage_mode
        FROM title_images ti
        WHERE ti.title_id = titles.id
          AND ti.kind = 'banner'
        LIMIT 1
    )
    SELECT CASE
        WHEN storage_mode = 'original' THEN
            '/images/titles/' || titles.id || '/banner/original?v=' || substr(master_sha256, 1, 16)
        ELSE
            '/images/titles/' || titles.id || '/banner/master?v=' || substr(master_sha256, 1, 16)
    END
    FROM selected_image
)
WHERE EXISTS (
    SELECT 1
    FROM title_images ti
    WHERE ti.title_id = titles.id
      AND ti.kind = 'banner'
);

UPDATE titles
SET background_local_path = (
    WITH selected_image AS (
        SELECT
            ti.master_sha256 AS master_sha256,
            ti.storage_mode AS storage_mode
        FROM title_images ti
        WHERE ti.title_id = titles.id
          AND ti.kind = 'fanart'
        LIMIT 1
    )
    SELECT CASE
        WHEN storage_mode = 'original' THEN
            '/images/titles/' || titles.id || '/fanart/original?v=' || substr(master_sha256, 1, 16)
        ELSE
            '/images/titles/' || titles.id || '/fanart/master?v=' || substr(master_sha256, 1, 16)
    END
    FROM selected_image
)
WHERE EXISTS (
    SELECT 1
    FROM title_images ti
    WHERE ti.title_id = titles.id
      AND ti.kind = 'fanart'
);

DELETE FROM title_image_variants
WHERE id IN (
    SELECT tiv.id
    FROM title_image_variants tiv
    INNER JOIN title_images ti ON ti.id = tiv.title_image_id
    WHERE tiv.variant_key = 'master'
      AND ti.kind IN ('banner', 'fanart')
);