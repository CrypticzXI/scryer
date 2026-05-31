ALTER TABLE notification_channels
    ADD COLUMN IF NOT EXISTS media_server_connection_id text;

CREATE TABLE media_server_connections (
    id text PRIMARY KEY,
    provider text NOT NULL CHECK (provider IN ('jellyfin', 'plex', 'emby')),
    display_name text NOT NULL,
    base_url text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    login_enabled boolean NOT NULL DEFAULT false,
    linking_enabled boolean NOT NULL DEFAULT false,
    auto_add_enabled boolean NOT NULL DEFAULT false,
    default_app_permissions bigint NOT NULL DEFAULT 0,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE jellyfin_media_server_details (
    connection_id text PRIMARY KEY REFERENCES media_server_connections(id) ON DELETE CASCADE,
    api_key text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE plex_media_server_details (
    connection_id text PRIMARY KEY REFERENCES media_server_connections(id) ON DELETE CASCADE,
    machine_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE emby_media_server_details (
    connection_id text PRIMARY KEY REFERENCES media_server_connections(id) ON DELETE CASCADE,
    api_key text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE media_server_path_mappings (
    id text PRIMARY KEY,
    connection_id text NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    source_path text NOT NULL,
    destination_path text NOT NULL,
    sort_order bigint NOT NULL DEFAULT 0
);

CREATE TABLE media_server_default_library_grants (
    connection_id text NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    permissions bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (connection_id, library_id)
);

CREATE INDEX idx_media_server_connections_provider
    ON media_server_connections (provider, enabled);

CREATE INDEX idx_media_server_path_mappings_connection
    ON media_server_path_mappings (connection_id, sort_order);

INSERT INTO media_server_connections (
    id,
    provider,
    display_name,
    base_url,
    enabled,
    login_enabled,
    linking_enabled,
    auto_add_enabled,
    default_app_permissions,
    created_at,
    updated_at
)
SELECT
    btrim(connection.value ->> 'id'),
    'jellyfin',
    COALESCE(
        NULLIF(btrim(connection.value ->> 'displayName'), ''),
        NULLIF(btrim(connection.value ->> 'id'), ''),
        'Jellyfin'
    ),
    rtrim(btrim(connection.value ->> 'baseUrl'), '/'),
    EXISTS (
        SELECT 1
        FROM settings_values allowed_value
        JOIN settings_definitions allowed_definition
          ON allowed_definition.id = allowed_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(allowed_value.value_json::jsonb) = 'array'
                THEN allowed_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS allowed_provider(value)
        WHERE allowed_definition.key_name = 'auth.providers.allowed'
          AND lower(btrim(allowed_provider.value #>> '{}')) = 'jellyfin'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(login_value.value_json::jsonb) = 'array'
                THEN login_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS login_provider(value)
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(btrim(login_provider.value #>> '{}')) = 'jellyfin'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(linking_value.value_json::jsonb) = 'array'
                THEN linking_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS linking_provider(value)
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(btrim(linking_provider.value #>> '{}')) = 'jellyfin'
    ),
    false,
    0,
    CURRENT_TIMESTAMP,
    CURRENT_TIMESTAMP
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
CROSS JOIN LATERAL jsonb_array_elements(
    CASE
        WHEN jsonb_typeof(value.value_json::jsonb) = 'array'
        THEN value.value_json::jsonb
        ELSE '[]'::jsonb
    END
) AS connection(value)
WHERE definition.key_name = 'auth.providers.jellyfin.connections'
  AND jsonb_typeof(connection.value) = 'object'
  AND NULLIF(btrim(connection.value ->> 'id'), '') IS NOT NULL
  AND NULLIF(rtrim(btrim(connection.value ->> 'baseUrl'), '/'), '') IS NOT NULL
ON CONFLICT (id) DO NOTHING;

INSERT INTO media_server_connections (
    id,
    provider,
    display_name,
    base_url,
    enabled,
    login_enabled,
    linking_enabled,
    auto_add_enabled,
    default_app_permissions,
    created_at,
    updated_at
)
SELECT
    btrim(connection.value ->> 'id'),
    'plex',
    COALESCE(
        NULLIF(btrim(connection.value ->> 'displayName'), ''),
        NULLIF(btrim(connection.value ->> 'id'), ''),
        'Plex'
    ),
    COALESCE(NULLIF(rtrim(btrim(connection.value ->> 'baseUrl'), '/'), ''), 'https://plex.tv'),
    EXISTS (
        SELECT 1
        FROM settings_values allowed_value
        JOIN settings_definitions allowed_definition
          ON allowed_definition.id = allowed_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(allowed_value.value_json::jsonb) = 'array'
                THEN allowed_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS allowed_provider(value)
        WHERE allowed_definition.key_name = 'auth.providers.allowed'
          AND lower(btrim(allowed_provider.value #>> '{}')) = 'plex'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(login_value.value_json::jsonb) = 'array'
                THEN login_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS login_provider(value)
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(btrim(login_provider.value #>> '{}')) = 'plex'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(linking_value.value_json::jsonb) = 'array'
                THEN linking_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS linking_provider(value)
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(btrim(linking_provider.value #>> '{}')) = 'plex'
    ),
    false,
    0,
    CURRENT_TIMESTAMP,
    CURRENT_TIMESTAMP
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
CROSS JOIN LATERAL jsonb_array_elements(
    CASE
        WHEN jsonb_typeof(value.value_json::jsonb) = 'array'
        THEN value.value_json::jsonb
        ELSE '[]'::jsonb
    END
) AS connection(value)
WHERE definition.key_name = 'auth.providers.plex.connections'
  AND jsonb_typeof(connection.value) = 'object'
  AND NULLIF(btrim(connection.value ->> 'id'), '') IS NOT NULL
ON CONFLICT (id) DO NOTHING;

INSERT INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
SELECT id, NULL, created_at, updated_at
FROM media_server_connections
WHERE provider = 'jellyfin'
ON CONFLICT (connection_id) DO NOTHING;

INSERT INTO plex_media_server_details (connection_id, machine_id, created_at, updated_at)
SELECT
    btrim(connection.value ->> 'id'),
    NULLIF(btrim(connection.value ->> 'machineId'), ''),
    CURRENT_TIMESTAMP,
    CURRENT_TIMESTAMP
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
CROSS JOIN LATERAL jsonb_array_elements(
    CASE
        WHEN jsonb_typeof(value.value_json::jsonb) = 'array'
        THEN value.value_json::jsonb
        ELSE '[]'::jsonb
    END
) AS connection(value)
WHERE definition.key_name = 'auth.providers.plex.connections'
  AND jsonb_typeof(connection.value) = 'object'
  AND EXISTS (
      SELECT 1
      FROM media_server_connections existing
      WHERE existing.id = btrim(connection.value ->> 'id')
        AND existing.provider = 'plex'
  )
ON CONFLICT (connection_id) DO NOTHING;

UPDATE media_server_connections
SET
    enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values allowed_value
            JOIN settings_definitions allowed_definition
              ON allowed_definition.id = allowed_value.setting_definition_id
            CROSS JOIN LATERAL jsonb_array_elements(
                CASE
                    WHEN jsonb_typeof(allowed_value.value_json::jsonb) = 'array'
                    THEN allowed_value.value_json::jsonb
                    ELSE '[]'::jsonb
                END
            ) AS allowed_provider(value)
            WHERE allowed_definition.key_name = 'auth.providers.allowed'
              AND lower(btrim(allowed_provider.value #>> '{}')) = media_server_connections.provider
        )
        AND (
            NOT EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                CROSS JOIN LATERAL jsonb_array_elements(
                    CASE
                        WHEN jsonb_typeof(ids_value.value_json::jsonb) = 'array'
                        THEN ids_value.value_json::jsonb
                        ELSE '[]'::jsonb
                    END
                ) AS allowed_id(value)
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND NULLIF(btrim(allowed_id.value #>> '{}'), '') IS NOT NULL
            )
            OR EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                CROSS JOIN LATERAL jsonb_array_elements(
                    CASE
                        WHEN jsonb_typeof(ids_value.value_json::jsonb) = 'array'
                        THEN ids_value.value_json::jsonb
                        ELSE '[]'::jsonb
                    END
                ) AS allowed_id(value)
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND btrim(allowed_id.value #>> '{}') = media_server_connections.id
            )
        )
    ),
    login_enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values login_value
            JOIN settings_definitions login_definition
              ON login_definition.id = login_value.setting_definition_id
            CROSS JOIN LATERAL jsonb_array_elements(
                CASE
                    WHEN jsonb_typeof(login_value.value_json::jsonb) = 'array'
                    THEN login_value.value_json::jsonb
                    ELSE '[]'::jsonb
                END
            ) AS login_provider(value)
            WHERE login_definition.key_name = 'auth.providers.login_enabled'
              AND lower(btrim(login_provider.value #>> '{}')) = media_server_connections.provider
        )
        AND enabled
    ),
    linking_enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values linking_value
            JOIN settings_definitions linking_definition
              ON linking_definition.id = linking_value.setting_definition_id
            CROSS JOIN LATERAL jsonb_array_elements(
                CASE
                    WHEN jsonb_typeof(linking_value.value_json::jsonb) = 'array'
                    THEN linking_value.value_json::jsonb
                    ELSE '[]'::jsonb
                END
            ) AS linking_provider(value)
            WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
              AND lower(btrim(linking_provider.value #>> '{}')) = media_server_connections.provider
        )
        AND enabled
    )
WHERE provider IN ('jellyfin', 'plex');

UPDATE media_server_connections
SET
    login_enabled = enabled AND EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(login_value.value_json::jsonb) = 'array'
                THEN login_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS login_provider(value)
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(btrim(login_provider.value #>> '{}')) = media_server_connections.provider
    ),
    linking_enabled = enabled AND EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(linking_value.value_json::jsonb) = 'array'
                THEN linking_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS linking_provider(value)
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(btrim(linking_provider.value #>> '{}')) = media_server_connections.provider
    )
WHERE provider IN ('jellyfin', 'plex');

CREATE OR REPLACE FUNCTION pg_temp.scryer_try_jsonb(value text)
RETURNS jsonb
LANGUAGE plpgsql
AS $$
BEGIN
    RETURN value::jsonb;
EXCEPTION WHEN others THEN
    RETURN NULL;
END
$$;

INSERT INTO media_server_connections (
    id,
    provider,
    display_name,
    base_url,
    enabled,
    login_enabled,
    linking_enabled,
    auto_add_enabled,
    default_app_permissions,
    created_at,
    updated_at
)
SELECT
    'jellyfin-notification-' || channel.id,
    'jellyfin',
    COALESCE(NULLIF(btrim(channel.name), ''), 'Jellyfin notifications'),
    rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/'),
    channel.is_enabled,
    false,
    false,
    false,
    0,
    channel.created_at,
    channel.updated_at
FROM notification_channels channel
WHERE channel.channel_type = 'jellyfin'
  AND channel.media_server_connection_id IS NULL
  AND pg_temp.scryer_try_jsonb(channel.config_json) IS NOT NULL
  AND NULLIF(rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/'), '') IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM media_server_connections existing
      WHERE existing.provider = 'jellyfin'
        AND existing.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/')
  )
ON CONFLICT (id) DO NOTHING;

INSERT INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
SELECT
    connection.id,
    NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'api_key'), ''),
    connection.created_at,
    connection.updated_at
FROM notification_channels channel
JOIN media_server_connections connection
  ON connection.id = 'jellyfin-notification-' || channel.id
WHERE channel.channel_type = 'jellyfin'
ON CONFLICT (connection_id) DO NOTHING;

UPDATE jellyfin_media_server_details
SET api_key = (
    SELECT NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'api_key'), '')
    FROM notification_channels channel
    JOIN media_server_connections connection
      ON connection.provider = 'jellyfin'
     AND connection.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/')
    WHERE connection.id = jellyfin_media_server_details.connection_id
      AND channel.channel_type = 'jellyfin'
      AND pg_temp.scryer_try_jsonb(channel.config_json) IS NOT NULL
      AND NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'api_key'), '') IS NOT NULL
    LIMIT 1
)
WHERE api_key IS NULL
  AND EXISTS (
      SELECT 1
      FROM notification_channels channel
      JOIN media_server_connections connection
        ON connection.provider = 'jellyfin'
       AND connection.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/')
      WHERE connection.id = jellyfin_media_server_details.connection_id
        AND channel.channel_type = 'jellyfin'
        AND pg_temp.scryer_try_jsonb(channel.config_json) IS NOT NULL
        AND NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'api_key'), '') IS NOT NULL
  );

UPDATE notification_channels
SET media_server_connection_id = (
    SELECT connection.id
    FROM media_server_connections connection
    WHERE connection.provider = 'jellyfin'
      AND connection.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(notification_channels.config_json) ->> 'base_url'), '/')
      AND connection.id <> 'jellyfin-notification-' || notification_channels.id
    ORDER BY connection.id
    LIMIT 1
)
WHERE channel_type = 'jellyfin'
  AND media_server_connection_id IS NULL
  AND pg_temp.scryer_try_jsonb(config_json) IS NOT NULL
  AND NULLIF(rtrim(btrim(pg_temp.scryer_try_jsonb(config_json) ->> 'base_url'), '/'), '') IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM media_server_connections connection
      WHERE connection.provider = 'jellyfin'
        AND connection.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(notification_channels.config_json) ->> 'base_url'), '/')
        AND connection.id <> 'jellyfin-notification-' || notification_channels.id
  );

UPDATE notification_channels
SET media_server_connection_id = 'jellyfin-notification-' || id
WHERE channel_type = 'jellyfin'
  AND media_server_connection_id IS NULL
  AND EXISTS (
      SELECT 1
      FROM media_server_connections connection
      WHERE connection.id = 'jellyfin-notification-' || notification_channels.id
  );

INSERT INTO media_server_path_mappings (
    id,
    connection_id,
    source_path,
    destination_path,
    sort_order
)
SELECT
    'notification-path-mapping-' || channel.id || '-' || mapping.ordinality,
    channel.media_server_connection_id,
    parsed.source_path,
    parsed.destination_path,
    mapping.ordinality - 1
FROM notification_channels channel
CROSS JOIN LATERAL regexp_split_to_table(
    replace(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'path_mappings', E'\r', ''),
    E'\n'
) WITH ORDINALITY AS mapping(line, ordinality)
CROSS JOIN LATERAL (
    SELECT
        btrim(substr(mapping.line, 1, position('=>' in mapping.line) - 1)) AS source_path,
        btrim(substr(mapping.line, position('=>' in mapping.line) + 2)) AS destination_path
) AS parsed
WHERE channel.channel_type = 'jellyfin'
  AND channel.media_server_connection_id IS NOT NULL
  AND pg_temp.scryer_try_jsonb(channel.config_json) IS NOT NULL
  AND NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'path_mappings'), '') IS NOT NULL
  AND position('=>' in mapping.line) > 0
  AND NULLIF(parsed.source_path, '') IS NOT NULL
  AND NULLIF(parsed.destination_path, '') IS NOT NULL
ON CONFLICT (id) DO NOTHING;

CREATE TABLE user_external_accounts (
    id text PRIMARY KEY,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider text NOT NULL CHECK (provider IN ('plex', 'jellyfin')),
    connection_id text NOT NULL REFERENCES media_server_connections(id),
    external_user_id text,
    username text NOT NULL,
    display_name text,
    avatar_url text,
    status text NOT NULL CHECK (status IN ('pending_claim', 'active', 'disabled')),
    verified_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
    ON user_external_accounts (provider, connection_id, external_user_id);

CREATE UNIQUE INDEX idx_user_external_accounts_pending_username
    ON user_external_accounts (provider, connection_id, LOWER(username))
    WHERE status = 'pending_claim' AND external_user_id IS NULL;

CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
    ON user_external_accounts (user_id, provider, connection_id);

CREATE INDEX idx_user_external_accounts_user_status
    ON user_external_accounts (user_id, status);
