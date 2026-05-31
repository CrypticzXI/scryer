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
