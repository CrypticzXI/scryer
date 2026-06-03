-- Rolled up from postgres/migrations/0136_notification_subscription_target_indexes.sql
CREATE UNIQUE INDEX idx_notification_subscriptions_target_scope
    ON notification_subscriptions (
        target_kind,
        target_id,
        event_type,
        COALESCE(scope, ''),
        COALESCE(scope_id, '')
    );

CREATE INDEX idx_notification_subscriptions_channel
    ON notification_subscriptions (channel_id)
    WHERE channel_id IS NOT NULL;

CREATE INDEX idx_notification_subscriptions_target
    ON notification_subscriptions (target_kind, target_id);

-- Rolled up from postgres/migrations/0137_user_auth_session_version.sql
ALTER TABLE users ADD COLUMN auth_session_version TEXT;

-- Rolled up from postgres/migrations/0138_drop_banner_images_and_rebuild_image_cache.sql
ALTER TABLE titles DROP COLUMN banner_url;
ALTER TABLE titles DROP COLUMN banner_local_path;

-- Rolled up from postgres/migrations/0139_variant_only_title_image_cache.sql
DELETE FROM title_image_variants;
DELETE FROM title_images;

UPDATE titles
   SET poster_local_path = NULL,
       background_local_path = NULL
 WHERE poster_local_path IS NOT NULL
    OR background_local_path IS NOT NULL;

ALTER TABLE title_image_variants
    RENAME COLUMN sha256 TO digest;

ALTER TABLE title_images
    DROP COLUMN storage_mode,
    DROP COLUMN master_path,
    DROP COLUMN master_format,
    DROP COLUMN master_sha256,
    DROP COLUMN master_width,
    DROP COLUMN master_height,
    DROP COLUMN bytes;
