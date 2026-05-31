DROP INDEX IF EXISTS idx_notification_subscriptions_channel_scope;

ALTER TABLE notification_subscriptions
    ADD COLUMN target_kind TEXT NOT NULL DEFAULT 'plugin_channel';

ALTER TABLE notification_subscriptions
    ADD COLUMN target_id TEXT;

UPDATE notification_subscriptions
   SET target_id = channel_id
 WHERE target_id IS NULL;

ALTER TABLE notification_subscriptions
    ALTER COLUMN target_id SET NOT NULL;

ALTER TABLE notification_subscriptions
    ALTER COLUMN channel_id DROP NOT NULL;

ALTER TABLE notification_subscriptions
    ADD CONSTRAINT notification_subscriptions_target_kind_check
    CHECK (target_kind IN ('plugin_channel', 'media_server_connection'));
