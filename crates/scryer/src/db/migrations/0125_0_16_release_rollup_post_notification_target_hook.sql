-- Rolled up from migrations/0136_notification_subscription_target_indexes.sql
CREATE UNIQUE INDEX idx_notification_subscriptions_target_scope
    ON notification_subscriptions (
        target_kind,
        target_id,
        event_type,
        COALESCE(scope, ''),
        COALESCE(scope_id, '')
    );

CREATE INDEX idx_notification_subscriptions_channel
    ON notification_subscriptions (channel_id);

CREATE INDEX idx_notification_subscriptions_target
    ON notification_subscriptions (target_kind, target_id);

-- Rolled up from migrations/0137_user_auth_session_version.sql
ALTER TABLE users ADD COLUMN auth_session_version TEXT;

-- Rolled up from migrations/0138_drop_banner_images_and_rebuild_image_cache.sql
ALTER TABLE titles DROP COLUMN banner_url;
ALTER TABLE titles DROP COLUMN banner_local_path;

-- Rolled up from migrations/0139_variant_only_title_image_cache.sql
DELETE FROM title_image_variants;
DELETE FROM title_images;

UPDATE titles
   SET poster_local_path = NULL,
       background_local_path = NULL
 WHERE poster_local_path IS NOT NULL
    OR background_local_path IS NOT NULL;

ALTER TABLE title_image_variants RENAME TO title_image_variants_old;
ALTER TABLE title_images RENAME TO title_images_old;

DROP TABLE title_image_variants_old;
DROP TABLE title_images_old;

CREATE TABLE title_images (
  id TEXT PRIMARY KEY,
  title_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  provider_image_id TEXT,
  kind TEXT NOT NULL CHECK (kind IN ('poster', 'fanart')),
  source_url TEXT NOT NULL,
  source_etag TEXT,
  source_last_modified TEXT,
  source_format TEXT NOT NULL,
  source_width INTEGER NOT NULL,
  source_height INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (title_id, kind),
  FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);

CREATE TABLE title_image_variants (
  id TEXT PRIMARY KEY,
  title_image_id TEXT NOT NULL,
  variant_key TEXT NOT NULL,
  path TEXT,
  format TEXT NOT NULL,
  width INTEGER NOT NULL,
  height INTEGER NOT NULL,
  bytes BLOB NOT NULL,
  digest TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (title_image_id, variant_key),
  FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE
);

CREATE INDEX idx_title_images_title_kind ON title_images(title_id, kind);
CREATE INDEX idx_title_image_variants_image_variant
  ON title_image_variants(title_image_id, variant_key);
