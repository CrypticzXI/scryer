INSERT OR IGNORE INTO entitlements (code, description, category)
VALUES
    ('view_catalog', 'Read access to title and media catalog', 'media'),
    ('manage_title', 'Read activity history and manage catalog entities', 'media'),
    ('manage_users', 'Manage users and security settings', 'system'),
    ('manage_config', 'Manage instance configuration', 'system');

UPDATE users
SET
    entitlements = COALESCE((
        WITH mapped AS (
            SELECT DISTINCT
                CASE LOWER(TRIM(CAST(entitlement.value AS TEXT)))
                    WHEN 'viewcatalog' THEN 'view_catalog'
                    WHEN 'view_catalog' THEN 'view_catalog'
                    WHEN 'monitortitle' THEN 'manage_title'
                    WHEN 'monitor_title' THEN 'manage_title'
                    WHEN 'managetitle' THEN 'manage_title'
                    WHEN 'manage_title' THEN 'manage_title'
                    WHEN 'triggeractions' THEN 'manage_title'
                    WHEN 'trigger_actions' THEN 'manage_title'
                    WHEN 'viewhistory' THEN 'manage_title'
                    WHEN 'view_history' THEN 'manage_title'
                    WHEN 'manageusers' THEN 'manage_users'
                    WHEN 'manage_users' THEN 'manage_users'
                    WHEN 'manageconfig' THEN 'manage_config'
                    WHEN 'manage_config' THEN 'manage_config'
                    ELSE NULL
                END AS code
            FROM json_each(
                CASE
                    WHEN json_valid(users.entitlements) THEN users.entitlements
                    ELSE '[]'
                END
            ) AS entitlement

            UNION

            SELECT 'manage_users'
            WHERE EXISTS (
                SELECT 1
                FROM json_each(
                    CASE
                        WHEN json_valid(users.entitlements) THEN users.entitlements
                        ELSE '[]'
                    END
                ) AS entitlement
                WHERE LOWER(TRIM(CAST(entitlement.value AS TEXT))) IN (
                    'manageconfig',
                    'manage_config'
                )
            )
        )
        SELECT json_group_array(code)
        FROM (
            SELECT code
            FROM mapped
            WHERE code IS NOT NULL
            ORDER BY CASE code
                WHEN 'view_catalog' THEN 1
                WHEN 'manage_title' THEN 2
                WHEN 'manage_users' THEN 3
                WHEN 'manage_config' THEN 4
                ELSE 100
            END
        )
    ), '[]'),
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now');

DELETE FROM user_entitlements;

INSERT INTO user_entitlements (
    user_id,
    entitlement_code,
    granted_by_user_id,
    granted_at,
    expires_at
)
SELECT
    users.id,
    CAST(entitlement.value AS TEXT),
    NULL,
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    NULL
FROM users
JOIN json_each(
    CASE
        WHEN json_valid(users.entitlements) THEN users.entitlements
        ELSE '[]'
    END
) AS entitlement
WHERE CAST(entitlement.value AS TEXT) IN (
    'view_catalog',
    'manage_title',
    'manage_users',
    'manage_config'
);

DELETE FROM entitlements
WHERE code IN ('monitor_title', 'trigger_actions', 'view_history');
