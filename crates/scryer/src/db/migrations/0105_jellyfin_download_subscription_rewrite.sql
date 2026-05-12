-- Rewrite legacy Jellyfin download subscriptions to import_complete and
-- collapse any duplicates introduced by the rewrite.

CREATE TEMP TABLE _jellyfin_subscription_keepers AS
WITH ranked AS (
    SELECT
        ns.id,
        ns.channel_id,
        ns.scope,
        ns.scope_id,
        ns.is_enabled,
        ns.created_at,
        ns.updated_at,
        MIN(ns.created_at) OVER (
            PARTITION BY ns.channel_id, ns.scope, COALESCE(ns.scope_id, '')
        ) AS oldest_created_at,
        ROW_NUMBER() OVER (
            PARTITION BY ns.channel_id, ns.scope, COALESCE(ns.scope_id, '')
            ORDER BY ns.is_enabled DESC, ns.created_at ASC, ns.id ASC
        ) AS row_rank
    FROM notification_subscriptions ns
    JOIN notification_channels nc ON nc.id = ns.channel_id
    WHERE lower(nc.channel_type) = 'jellyfin'
      AND ns.event_type IN ('download', 'import_complete')
)
SELECT
    id,
    channel_id,
    'import_complete' AS event_type,
    scope,
    scope_id,
    is_enabled,
    oldest_created_at AS created_at,
    updated_at
FROM ranked
WHERE row_rank = 1;

DELETE FROM notification_subscriptions
WHERE id IN (
    SELECT ns.id
    FROM notification_subscriptions ns
    JOIN notification_channels nc ON nc.id = ns.channel_id
    WHERE lower(nc.channel_type) = 'jellyfin'
      AND ns.event_type IN ('download', 'import_complete')
);

INSERT INTO notification_subscriptions (
    id,
    channel_id,
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
    event_type,
    scope,
    scope_id,
    is_enabled,
    created_at,
    updated_at
FROM _jellyfin_subscription_keepers;

DROP TABLE _jellyfin_subscription_keepers;
