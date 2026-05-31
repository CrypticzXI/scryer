ALTER TABLE notification_channels
    ADD COLUMN media_server_connection_id TEXT;

CREATE TABLE media_server_connections (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    login_enabled INTEGER NOT NULL DEFAULT 0,
    linking_enabled INTEGER NOT NULL DEFAULT 0,
    auto_add_enabled INTEGER NOT NULL DEFAULT 0,
    default_app_permissions INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (provider IN ('jellyfin', 'plex', 'emby'))
);

CREATE TABLE jellyfin_media_server_details (
    connection_id TEXT PRIMARY KEY,
    api_key TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);

CREATE TABLE plex_media_server_details (
    connection_id TEXT PRIMARY KEY,
    machine_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);

CREATE TABLE emby_media_server_details (
    connection_id TEXT PRIMARY KEY,
    api_key TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);

CREATE TABLE media_server_path_mappings (
    id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    destination_path TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);

CREATE TABLE media_server_default_library_grants (
    connection_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    permissions INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (connection_id, library_id),
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);

CREATE INDEX idx_media_server_connections_provider
    ON media_server_connections (provider, enabled);

CREATE INDEX idx_media_server_path_mappings_connection
    ON media_server_path_mappings (connection_id, sort_order);

INSERT OR IGNORE INTO media_server_connections (
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
    trim(json_extract(connection.value, '$.id')),
    'jellyfin',
    COALESCE(
        NULLIF(trim(json_extract(connection.value, '$.displayName')), ''),
        NULLIF(trim(json_extract(connection.value, '$.id')), ''),
        'Jellyfin'
    ),
    rtrim(trim(json_extract(connection.value, '$.baseUrl')), '/'),
    EXISTS (
        SELECT 1
        FROM settings_values allowed_value
        JOIN settings_definitions allowed_definition
          ON allowed_definition.id = allowed_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
        WHERE allowed_definition.key_name = 'auth.providers.allowed'
          AND lower(trim(allowed_provider.value)) = 'jellyfin'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(login_value.value_json) THEN login_value.value_json ELSE '[]' END) login_provider
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(trim(login_provider.value)) = 'jellyfin'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(linking_value.value_json) THEN linking_value.value_json ELSE '[]' END) linking_provider
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(trim(linking_provider.value)) = 'jellyfin'
    ),
    0,
    0,
    datetime('now'),
    datetime('now')
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
JOIN json_each(CASE WHEN json_valid(value.value_json) THEN value.value_json ELSE '[]' END) connection
WHERE definition.key_name = 'auth.providers.jellyfin.connections'
  AND connection.type = 'object'
  AND NULLIF(trim(json_extract(connection.value, '$.id')), '') IS NOT NULL
  AND NULLIF(rtrim(trim(json_extract(connection.value, '$.baseUrl')), '/'), '') IS NOT NULL;

INSERT OR IGNORE INTO media_server_connections (
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
    trim(json_extract(connection.value, '$.id')),
    'plex',
    COALESCE(
        NULLIF(trim(json_extract(connection.value, '$.displayName')), ''),
        NULLIF(trim(json_extract(connection.value, '$.id')), ''),
        'Plex'
    ),
    COALESCE(NULLIF(rtrim(trim(json_extract(connection.value, '$.baseUrl')), '/'), ''), 'https://plex.tv'),
    EXISTS (
        SELECT 1
        FROM settings_values allowed_value
        JOIN settings_definitions allowed_definition
          ON allowed_definition.id = allowed_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
        WHERE allowed_definition.key_name = 'auth.providers.allowed'
          AND lower(trim(allowed_provider.value)) = 'plex'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(login_value.value_json) THEN login_value.value_json ELSE '[]' END) login_provider
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(trim(login_provider.value)) = 'plex'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(linking_value.value_json) THEN linking_value.value_json ELSE '[]' END) linking_provider
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(trim(linking_provider.value)) = 'plex'
    ),
    0,
    0,
    datetime('now'),
    datetime('now')
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
JOIN json_each(CASE WHEN json_valid(value.value_json) THEN value.value_json ELSE '[]' END) connection
WHERE definition.key_name = 'auth.providers.plex.connections'
  AND connection.type = 'object'
  AND NULLIF(trim(json_extract(connection.value, '$.id')), '') IS NOT NULL;

INSERT OR IGNORE INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
SELECT id, NULL, created_at, updated_at
FROM media_server_connections
WHERE provider = 'jellyfin';

INSERT OR IGNORE INTO plex_media_server_details (connection_id, machine_id, created_at, updated_at)
SELECT
    trim(json_extract(connection.value, '$.id')),
    NULLIF(trim(json_extract(connection.value, '$.machineId')), ''),
    datetime('now'),
    datetime('now')
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
JOIN json_each(CASE WHEN json_valid(value.value_json) THEN value.value_json ELSE '[]' END) connection
WHERE definition.key_name = 'auth.providers.plex.connections'
  AND connection.type = 'object'
  AND EXISTS (
      SELECT 1
      FROM media_server_connections existing
      WHERE existing.id = trim(json_extract(connection.value, '$.id'))
        AND existing.provider = 'plex'
  );

UPDATE media_server_connections
SET
    enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values allowed_value
            JOIN settings_definitions allowed_definition
              ON allowed_definition.id = allowed_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
            WHERE allowed_definition.key_name = 'auth.providers.allowed'
              AND lower(trim(allowed_provider.value)) = media_server_connections.provider
        )
        AND (
            NOT EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND NULLIF(trim(allowed_id.value), '') IS NOT NULL
            )
            OR EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND trim(allowed_id.value) = media_server_connections.id
            )
        )
    ),
    login_enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values allowed_value
            JOIN settings_definitions allowed_definition
              ON allowed_definition.id = allowed_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
            WHERE allowed_definition.key_name = 'auth.providers.allowed'
              AND lower(trim(allowed_provider.value)) = media_server_connections.provider
        )
        AND EXISTS (
            SELECT 1
            FROM settings_values login_value
            JOIN settings_definitions login_definition
              ON login_definition.id = login_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(login_value.value_json) THEN login_value.value_json ELSE '[]' END) login_provider
            WHERE login_definition.key_name = 'auth.providers.login_enabled'
              AND lower(trim(login_provider.value)) = media_server_connections.provider
        )
        AND (
            NOT EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND NULLIF(trim(allowed_id.value), '') IS NOT NULL
            )
            OR EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND trim(allowed_id.value) = media_server_connections.id
            )
        )
    ),
    linking_enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values allowed_value
            JOIN settings_definitions allowed_definition
              ON allowed_definition.id = allowed_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
            WHERE allowed_definition.key_name = 'auth.providers.allowed'
              AND lower(trim(allowed_provider.value)) = media_server_connections.provider
        )
        AND EXISTS (
            SELECT 1
            FROM settings_values linking_value
            JOIN settings_definitions linking_definition
              ON linking_definition.id = linking_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(linking_value.value_json) THEN linking_value.value_json ELSE '[]' END) linking_provider
            WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
              AND lower(trim(linking_provider.value)) = media_server_connections.provider
        )
        AND (
            NOT EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND NULLIF(trim(allowed_id.value), '') IS NOT NULL
            )
            OR EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND trim(allowed_id.value) = media_server_connections.id
            )
        )
    )
WHERE provider IN ('jellyfin', 'plex');

INSERT OR IGNORE INTO media_server_connections (
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
    COALESCE(NULLIF(trim(channel.name), ''), 'Jellyfin notifications'),
    rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/'),
    channel.is_enabled,
    0,
    0,
    0,
    0,
    channel.created_at,
    channel.updated_at
FROM notification_channels channel
WHERE channel.channel_type = 'jellyfin'
  AND channel.media_server_connection_id IS NULL
  AND json_valid(channel.config_json)
  AND NULLIF(rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/'), '') IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM media_server_connections existing
      WHERE existing.provider = 'jellyfin'
        AND existing.base_url = rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/')
  );

INSERT OR IGNORE INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
SELECT
    connection.id,
    NULLIF(trim(json_extract(channel.config_json, '$.api_key')), ''),
    connection.created_at,
    connection.updated_at
FROM notification_channels channel
JOIN media_server_connections connection
  ON connection.id = 'jellyfin-notification-' || channel.id
WHERE channel.channel_type = 'jellyfin'
  AND json_valid(channel.config_json);

UPDATE jellyfin_media_server_details
SET api_key = (
    SELECT NULLIF(trim(json_extract(channel.config_json, '$.api_key')), '')
    FROM notification_channels channel
    JOIN media_server_connections connection
      ON connection.provider = 'jellyfin'
     AND connection.base_url = rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/')
    WHERE connection.id = jellyfin_media_server_details.connection_id
      AND channel.channel_type = 'jellyfin'
      AND json_valid(channel.config_json)
      AND NULLIF(trim(json_extract(channel.config_json, '$.api_key')), '') IS NOT NULL
    LIMIT 1
)
WHERE api_key IS NULL
  AND EXISTS (
      SELECT 1
      FROM notification_channels channel
      JOIN media_server_connections connection
        ON connection.provider = 'jellyfin'
       AND connection.base_url = rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/')
      WHERE connection.id = jellyfin_media_server_details.connection_id
        AND channel.channel_type = 'jellyfin'
        AND json_valid(channel.config_json)
        AND NULLIF(trim(json_extract(channel.config_json, '$.api_key')), '') IS NOT NULL
  );

UPDATE notification_channels
SET media_server_connection_id = (
    SELECT connection.id
    FROM media_server_connections connection
    WHERE connection.provider = 'jellyfin'
      AND connection.base_url = rtrim(trim(json_extract(notification_channels.config_json, '$.base_url')), '/')
      AND connection.id <> 'jellyfin-notification-' || notification_channels.id
    ORDER BY connection.id
    LIMIT 1
)
WHERE channel_type = 'jellyfin'
  AND media_server_connection_id IS NULL
  AND json_valid(config_json)
  AND NULLIF(rtrim(trim(json_extract(config_json, '$.base_url')), '/'), '') IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM media_server_connections connection
      WHERE connection.provider = 'jellyfin'
        AND connection.base_url = rtrim(trim(json_extract(notification_channels.config_json, '$.base_url')), '/')
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

WITH RECURSIVE mapping_lines(channel_id, connection_id, remaining, line, sort_order) AS (
    SELECT
        id,
        media_server_connection_id,
        replace(json_extract(config_json, '$.path_mappings'), char(13), '') || char(10),
        '',
        0
    FROM notification_channels
    WHERE channel_type = 'jellyfin'
      AND media_server_connection_id IS NOT NULL
      AND json_valid(config_json)
      AND json_type(CASE WHEN json_valid(config_json) THEN config_json ELSE '{}' END, '$.path_mappings') = 'text'
      AND NULLIF(trim(json_extract(config_json, '$.path_mappings')), '') IS NOT NULL
    UNION ALL
    SELECT
        channel_id,
        connection_id,
        substr(remaining, instr(remaining, char(10)) + 1),
        trim(substr(remaining, 1, instr(remaining, char(10)) - 1)),
        sort_order + 1
    FROM mapping_lines
    WHERE remaining <> ''
      AND instr(remaining, char(10)) > 0
)
INSERT OR IGNORE INTO media_server_path_mappings (
    id,
    connection_id,
    source_path,
    destination_path,
    sort_order
)
SELECT
    'notification-path-mapping-' || channel_id || '-' || sort_order,
    connection_id,
    trim(substr(line, 1, instr(line, '=>') - 1)),
    trim(substr(line, instr(line, '=>') + 2)),
    sort_order - 1
FROM mapping_lines
WHERE line <> ''
  AND instr(line, '=>') > 0
  AND NULLIF(trim(substr(line, 1, instr(line, '=>') - 1)), '') IS NOT NULL
  AND NULLIF(trim(substr(line, instr(line, '=>') + 2)), '') IS NOT NULL;

CREATE TABLE user_external_accounts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    external_user_id TEXT,
    username TEXT NOT NULL,
    display_name TEXT,
    avatar_url TEXT,
    status TEXT NOT NULL,
    verified_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id),
    CHECK (provider IN ('plex', 'jellyfin')),
    CHECK (status IN ('pending_claim', 'active', 'disabled'))
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
