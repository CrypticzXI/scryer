ALTER TABLE titles ADD COLUMN poster_local_path TEXT;
ALTER TABLE titles ADD COLUMN banner_local_path TEXT;
ALTER TABLE titles ADD COLUMN background_local_path TEXT;

UPDATE titles
SET poster_local_path = (
    WITH selected_image AS (
        SELECT
            ti.master_sha256 AS master_sha256,
            ti.storage_mode AS storage_mode,
            (
                SELECT tiv.sha256
                FROM title_image_variants tiv
                WHERE tiv.title_image_id = ti.id
                  AND tiv.variant_key = 'w500'
                LIMIT 1
            ) AS preferred_sha256
        FROM title_images ti
        WHERE ti.title_id = titles.id
          AND ti.kind = 'poster'
        LIMIT 1
    )
    SELECT CASE
        WHEN storage_mode = 'original' THEN
            '/images/titles/' || titles.id || '/poster/original?v=' || substr(master_sha256, 1, 16)
        WHEN preferred_sha256 IS NOT NULL THEN
            '/images/titles/' || titles.id || '/poster/w500?v=' || substr(preferred_sha256, 1, 16)
        ELSE
            '/images/titles/' || titles.id || '/poster/original?v=' || substr(master_sha256, 1, 16)
    END
    FROM selected_image
)
WHERE EXISTS (
    SELECT 1
    FROM title_images ti
    WHERE ti.title_id = titles.id
      AND ti.kind = 'poster'
);

UPDATE titles
SET banner_local_path = (
    WITH selected_image AS (
        SELECT
            ti.master_sha256 AS master_sha256,
            ti.storage_mode AS storage_mode,
            (
                SELECT tiv.sha256
                FROM title_image_variants tiv
                WHERE tiv.title_image_id = ti.id
                  AND tiv.variant_key = 'master'
                LIMIT 1
            ) AS preferred_sha256
        FROM title_images ti
        WHERE ti.title_id = titles.id
          AND ti.kind = 'banner'
        LIMIT 1
    )
    SELECT CASE
        WHEN storage_mode = 'original' THEN
            '/images/titles/' || titles.id || '/banner/original?v=' || substr(master_sha256, 1, 16)
        WHEN preferred_sha256 IS NOT NULL THEN
            '/images/titles/' || titles.id || '/banner/master?v=' || substr(preferred_sha256, 1, 16)
        ELSE
            '/images/titles/' || titles.id || '/banner/original?v=' || substr(master_sha256, 1, 16)
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
            ti.storage_mode AS storage_mode,
            (
                SELECT tiv.sha256
                FROM title_image_variants tiv
                WHERE tiv.title_image_id = ti.id
                  AND tiv.variant_key = 'master'
                LIMIT 1
            ) AS preferred_sha256
        FROM title_images ti
        WHERE ti.title_id = titles.id
          AND ti.kind = 'fanart'
        LIMIT 1
    )
    SELECT CASE
        WHEN storage_mode = 'original' THEN
            '/images/titles/' || titles.id || '/fanart/original?v=' || substr(master_sha256, 1, 16)
        WHEN preferred_sha256 IS NOT NULL THEN
            '/images/titles/' || titles.id || '/fanart/master?v=' || substr(preferred_sha256, 1, 16)
        ELSE
            '/images/titles/' || titles.id || '/fanart/original?v=' || substr(master_sha256, 1, 16)
    END
    FROM selected_image
)
WHERE EXISTS (
    SELECT 1
    FROM title_images ti
    WHERE ti.title_id = titles.id
      AND ti.kind = 'fanart'
);
