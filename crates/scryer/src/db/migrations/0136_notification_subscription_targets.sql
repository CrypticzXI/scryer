DROP INDEX IF EXISTS idx_notification_subscriptions_channel_scope;

CREATE TABLE notification_subscriptions_next (
    id TEXT PRIMARY KEY,
    channel_id TEXT,
    target_kind TEXT NOT NULL DEFAULT 'plugin_channel',
    target_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (target_kind IN ('plugin_channel', 'media_server_connection')),
    FOREIGN KEY (channel_id) REFERENCES notification_channels(id) ON DELETE CASCADE
);

INSERT INTO notification_subscriptions_next (
    id,
    channel_id,
    target_kind,
    target_id,
    event_type,
    scope,
    scope_id,
    is_enabled,
    created_at,
    updated_at
)
SELECT
    id,
    channel_id,
    'plugin_channel',
    channel_id,
    event_type,
    scope,
    scope_id,
    is_enabled,
    created_at,
    updated_at
FROM notification_subscriptions;

DROP TABLE notification_subscriptions;

ALTER TABLE notification_subscriptions_next RENAME TO notification_subscriptions;
