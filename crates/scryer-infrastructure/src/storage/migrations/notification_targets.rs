use scryer_application::{AppError, AppResult};
use sqlx::Row;

use crate::encryption::{EncryptionKey, decrypt_value, encrypt_value, is_encrypted};

#[derive(Clone, Debug)]
struct LegacyJellyfinChannel {
    id: String,
    name: String,
    config_json: String,
    media_server_connection_id: Option<String>,
    is_enabled: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Clone, Debug)]
struct LegacyJellyfinConfig {
    base_url: String,
    api_key: String,
    path_mappings: Vec<(String, String)>,
}

enum LegacyJellyfinConfigRead {
    Valid(LegacyJellyfinConfig),
    EncryptedWithoutKey,
    Malformed(String),
}

pub(crate) async fn migrate_jellyfin_notification_channels_to_media_server_targets_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()> {
    let channels = sqlite_jellyfin_channels(tx).await?;
    for channel in channels {
        let config_read = read_jellyfin_config(&channel.config_json, encryption_key);
        if let Some(connection_id) = sqlite_existing_channel_connection(tx, &channel).await? {
            match config_read {
                LegacyJellyfinConfigRead::Valid(config) => {
                    sqlite_upsert_jellyfin_details(
                        tx,
                        &connection_id,
                        &channel,
                        &config,
                        encryption_key,
                    )
                    .await?;
                    sqlite_insert_path_mappings(tx, &channel.id, &connection_id, &config).await?;
                }
                LegacyJellyfinConfigRead::EncryptedWithoutKey => {
                    tracing::warn!(
                        channel_id = channel.id.as_str(),
                        connection_id = connection_id.as_str(),
                        "legacy Jellyfin notification channel already points at a media server connection; moving subscriptions without decrypting channel config"
                    );
                }
                LegacyJellyfinConfigRead::Malformed(reason) => {
                    tracing::warn!(
                        channel_id = channel.id.as_str(),
                        connection_id = connection_id.as_str(),
                        reason,
                        "legacy Jellyfin notification channel already points at a media server connection; moving subscriptions without channel config"
                    );
                }
            }
            sqlite_move_channel_subscriptions_to_connection(tx, &channel.id, &connection_id)
                .await?;
            sqlite_delete_notification_channel(tx, &channel.id).await?;
            continue;
        }

        match config_read {
            LegacyJellyfinConfigRead::Valid(config) => {
                let connection_id =
                    sqlite_create_or_reuse_jellyfin_connection(tx, &channel, &config).await?;
                sqlite_upsert_jellyfin_details(
                    tx,
                    &connection_id,
                    &channel,
                    &config,
                    encryption_key,
                )
                .await?;
                sqlite_insert_path_mappings(tx, &channel.id, &connection_id, &config).await?;
                sqlite_move_channel_subscriptions_to_connection(tx, &channel.id, &connection_id)
                    .await?;
                sqlite_delete_notification_channel(tx, &channel.id).await?;
            }
            LegacyJellyfinConfigRead::EncryptedWithoutKey => {
                sqlite_disable_malformed_jellyfin_channel(
                    tx,
                    &channel.id,
                    "encrypted config could not be migrated before encryption bootstrap",
                )
                .await?;
            }
            LegacyJellyfinConfigRead::Malformed(reason) => {
                sqlite_disable_malformed_jellyfin_channel(tx, &channel.id, &reason).await?;
            }
        }
    }
    Ok(())
}

pub(crate) async fn migrate_jellyfin_notification_channels_to_media_server_targets_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()> {
    let channels = postgres_jellyfin_channels(tx).await?;
    for channel in channels {
        let config_read = read_jellyfin_config(&channel.config_json, encryption_key);
        if let Some(connection_id) = postgres_existing_channel_connection(tx, &channel).await? {
            match config_read {
                LegacyJellyfinConfigRead::Valid(config) => {
                    postgres_upsert_jellyfin_details(
                        tx,
                        &connection_id,
                        &channel,
                        &config,
                        encryption_key,
                    )
                    .await?;
                    postgres_insert_path_mappings(tx, &channel.id, &connection_id, &config).await?;
                }
                LegacyJellyfinConfigRead::EncryptedWithoutKey => {
                    tracing::warn!(
                        channel_id = channel.id.as_str(),
                        connection_id = connection_id.as_str(),
                        "legacy Jellyfin notification channel already points at a media server connection; moving subscriptions without decrypting channel config"
                    );
                }
                LegacyJellyfinConfigRead::Malformed(reason) => {
                    tracing::warn!(
                        channel_id = channel.id.as_str(),
                        connection_id = connection_id.as_str(),
                        reason,
                        "legacy Jellyfin notification channel already points at a media server connection; moving subscriptions without channel config"
                    );
                }
            }
            postgres_move_channel_subscriptions_to_connection(tx, &channel.id, &connection_id)
                .await?;
            postgres_delete_notification_channel(tx, &channel.id).await?;
            continue;
        }

        match config_read {
            LegacyJellyfinConfigRead::Valid(config) => {
                let connection_id =
                    postgres_create_or_reuse_jellyfin_connection(tx, &channel, &config).await?;
                postgres_upsert_jellyfin_details(
                    tx,
                    &connection_id,
                    &channel,
                    &config,
                    encryption_key,
                )
                .await?;
                postgres_insert_path_mappings(tx, &channel.id, &connection_id, &config).await?;
                postgres_move_channel_subscriptions_to_connection(tx, &channel.id, &connection_id)
                    .await?;
                postgres_delete_notification_channel(tx, &channel.id).await?;
            }
            LegacyJellyfinConfigRead::EncryptedWithoutKey => {
                postgres_disable_malformed_jellyfin_channel(
                    tx,
                    &channel.id,
                    "encrypted config could not be migrated before encryption bootstrap",
                )
                .await?;
            }
            LegacyJellyfinConfigRead::Malformed(reason) => {
                postgres_disable_malformed_jellyfin_channel(tx, &channel.id, &reason).await?;
            }
        }
    }
    Ok(())
}

async fn sqlite_jellyfin_channels(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<LegacyJellyfinChannel>> {
    let rows = sqlx::query(
        "SELECT id, name, config_json, media_server_connection_id, is_enabled, created_at, updated_at
           FROM notification_channels
          WHERE channel_type = 'jellyfin'",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;

    rows.into_iter()
        .map(|row| {
            Ok(LegacyJellyfinChannel {
                id: row.try_get("id").map_err(repo_err)?,
                name: row.try_get("name").map_err(repo_err)?,
                config_json: row.try_get("config_json").map_err(repo_err)?,
                media_server_connection_id: row
                    .try_get("media_server_connection_id")
                    .map_err(repo_err)?,
                is_enabled: row.try_get::<i64, _>("is_enabled").map_err(repo_err)? != 0,
                created_at: row.try_get("created_at").map_err(repo_err)?,
                updated_at: row.try_get("updated_at").map_err(repo_err)?,
            })
        })
        .collect()
}

async fn postgres_jellyfin_channels(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<LegacyJellyfinChannel>> {
    let rows = sqlx::query(
        "SELECT
             id,
             name,
             config_json,
             media_server_connection_id,
             is_enabled,
             created_at::TEXT AS created_at_text,
             updated_at::TEXT AS updated_at_text
           FROM notification_channels
          WHERE channel_type = 'jellyfin'",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;

    rows.into_iter()
        .map(|row| {
            Ok(LegacyJellyfinChannel {
                id: row.try_get("id").map_err(repo_err)?,
                name: row.try_get("name").map_err(repo_err)?,
                config_json: row.try_get("config_json").map_err(repo_err)?,
                media_server_connection_id: row
                    .try_get("media_server_connection_id")
                    .map_err(repo_err)?,
                is_enabled: row.try_get("is_enabled").map_err(repo_err)?,
                created_at: row.try_get("created_at_text").map_err(repo_err)?,
                updated_at: row.try_get("updated_at_text").map_err(repo_err)?,
            })
        })
        .collect()
}

fn read_jellyfin_config(
    raw_config_json: &str,
    encryption_key: Option<&EncryptionKey>,
) -> LegacyJellyfinConfigRead {
    let config_json = if is_encrypted(raw_config_json) {
        let Some(key) = encryption_key else {
            return LegacyJellyfinConfigRead::EncryptedWithoutKey;
        };
        match decrypt_value(key, raw_config_json) {
            Ok(value) => value,
            Err(error) => {
                return LegacyJellyfinConfigRead::Malformed(format!(
                    "encrypted config could not be decrypted: {error}"
                ));
            }
        }
    } else {
        raw_config_json.to_string()
    };

    let value = match serde_json::from_str::<serde_json::Value>(&config_json) {
        Ok(value) => value,
        Err(error) => {
            return LegacyJellyfinConfigRead::Malformed(format!(
                "config_json is not valid JSON: {error}"
            ));
        }
    };
    let Some(object) = value.as_object() else {
        return LegacyJellyfinConfigRead::Malformed("config_json is not an object".to_string());
    };

    let Some(base_url) = object
        .get("base_url")
        .and_then(|value| value.as_str())
        .map(normalize_base_url)
        .filter(|value| !value.is_empty())
    else {
        return LegacyJellyfinConfigRead::Malformed("config_json is missing base_url".to_string());
    };
    let Some(api_key) = object
        .get("api_key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return LegacyJellyfinConfigRead::Malformed("config_json is missing api_key".to_string());
    };
    let path_mappings = object
        .get("path_mappings")
        .and_then(|value| value.as_str())
        .map(parse_path_mappings)
        .unwrap_or_default();

    LegacyJellyfinConfigRead::Valid(LegacyJellyfinConfig {
        base_url,
        api_key,
        path_mappings,
    })
}

async fn sqlite_create_or_reuse_jellyfin_connection(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel: &LegacyJellyfinChannel,
    config: &LegacyJellyfinConfig,
) -> AppResult<String> {
    if let Some(connection_id) = channel
        .media_server_connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && sqlite_connection_exists(tx, connection_id).await?
    {
        return Ok(connection_id.to_string());
    }

    if let Some(existing_id) = sqlx::query_scalar::<_, String>(
        "SELECT id
           FROM media_server_connections
          WHERE provider = 'jellyfin'
            AND rtrim(base_url, '/') = ?1
          ORDER BY id
          LIMIT 1",
    )
    .bind(&config.base_url)
    .fetch_optional(&mut **tx)
    .await
    .map_err(repo_err)?
    {
        return Ok(existing_id);
    }

    let connection_id = format!("jellyfin-notification-{}", channel.id);
    sqlx::query(
        "INSERT OR IGNORE INTO media_server_connections (
             id, provider, display_name, base_url, enabled, login_enabled,
             linking_enabled, auto_add_enabled, default_app_permissions, created_at, updated_at
         ) VALUES (?1, 'jellyfin', ?2, ?3, ?4, 0, 0, 0, 0, ?5, ?6)",
    )
    .bind(&connection_id)
    .bind(&channel.name)
    .bind(&config.base_url)
    .bind(channel.is_enabled)
    .bind(&channel.created_at)
    .bind(&channel.updated_at)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(connection_id)
}

async fn postgres_create_or_reuse_jellyfin_connection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel: &LegacyJellyfinChannel,
    config: &LegacyJellyfinConfig,
) -> AppResult<String> {
    if let Some(connection_id) = channel
        .media_server_connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && postgres_connection_exists(tx, connection_id).await?
    {
        return Ok(connection_id.to_string());
    }

    if let Some(existing_id) = sqlx::query_scalar::<_, String>(
        "SELECT id
           FROM media_server_connections
          WHERE provider = 'jellyfin'
            AND rtrim(base_url, '/') = $1
          ORDER BY id
          LIMIT 1",
    )
    .bind(&config.base_url)
    .fetch_optional(&mut **tx)
    .await
    .map_err(repo_err)?
    {
        return Ok(existing_id);
    }

    let connection_id = format!("jellyfin-notification-{}", channel.id);
    sqlx::query(
        "INSERT INTO media_server_connections (
             id, provider, display_name, base_url, enabled, login_enabled,
             linking_enabled, auto_add_enabled, default_app_permissions, created_at, updated_at
         ) VALUES ($1, 'jellyfin', $2, $3, $4, FALSE, FALSE, FALSE, 0, $5::timestamptz, $6::timestamptz)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&connection_id)
    .bind(&channel.name)
    .bind(&config.base_url)
    .bind(channel.is_enabled)
    .bind(&channel.created_at)
    .bind(&channel.updated_at)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(connection_id)
}

async fn sqlite_connection_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    connection_id: &str,
) -> AppResult<bool> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
           FROM media_server_connections
          WHERE id = ?1
            AND provider = 'jellyfin'",
    )
    .bind(connection_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(exists > 0)
}

async fn postgres_connection_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    connection_id: &str,
) -> AppResult<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1
               FROM media_server_connections
              WHERE id = $1
                AND provider = 'jellyfin'
         )",
    )
    .bind(connection_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(exists)
}

async fn sqlite_existing_channel_connection(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel: &LegacyJellyfinChannel,
) -> AppResult<Option<String>> {
    let Some(connection_id) = channel
        .media_server_connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if sqlite_connection_exists(tx, connection_id).await? {
        Ok(Some(connection_id.to_string()))
    } else {
        Ok(None)
    }
}

async fn postgres_existing_channel_connection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel: &LegacyJellyfinChannel,
) -> AppResult<Option<String>> {
    let Some(connection_id) = channel
        .media_server_connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if postgres_connection_exists(tx, connection_id).await? {
        Ok(Some(connection_id.to_string()))
    } else {
        Ok(None)
    }
}

async fn sqlite_move_channel_subscriptions_to_connection(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel_id: &str,
    connection_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE notification_subscriptions
            SET is_enabled = 1,
                updated_at = CURRENT_TIMESTAMP
          WHERE target_kind = 'media_server_connection'
            AND target_id = ?1
            AND is_enabled = 0
            AND EXISTS (
                SELECT 1
                  FROM notification_subscriptions AS duplicate
                 WHERE duplicate.channel_id = ?2
                   AND duplicate.event_type = notification_subscriptions.event_type
                   AND COALESCE(duplicate.scope, '') = COALESCE(notification_subscriptions.scope, '')
                   AND COALESCE(duplicate.scope_id, '') = COALESCE(notification_subscriptions.scope_id, '')
                   AND duplicate.is_enabled != 0
            )",
    )
    .bind(connection_id)
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    sqlx::query(
        "DELETE FROM notification_subscriptions
          WHERE channel_id = ?2
            AND EXISTS (
                SELECT 1
                  FROM notification_subscriptions AS existing
                 WHERE existing.target_kind = 'media_server_connection'
                   AND existing.target_id = ?1
                   AND existing.event_type = notification_subscriptions.event_type
                   AND COALESCE(existing.scope, '') = COALESCE(notification_subscriptions.scope, '')
                   AND COALESCE(existing.scope_id, '') = COALESCE(notification_subscriptions.scope_id, '')
            )",
    )
    .bind(connection_id)
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    sqlx::query(
        "UPDATE notification_subscriptions
            SET channel_id = NULL,
                target_kind = 'media_server_connection',
                target_id = ?1
          WHERE channel_id = ?2",
    )
    .bind(connection_id)
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    Ok(())
}

async fn postgres_move_channel_subscriptions_to_connection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: &str,
    connection_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE notification_subscriptions
            SET is_enabled = TRUE,
                updated_at = NOW()
          WHERE target_kind = 'media_server_connection'
            AND target_id = $1
            AND is_enabled = FALSE
            AND EXISTS (
                SELECT 1
                  FROM notification_subscriptions AS duplicate
                 WHERE duplicate.channel_id = $2
                   AND duplicate.event_type = notification_subscriptions.event_type
                   AND COALESCE(duplicate.scope, '') = COALESCE(notification_subscriptions.scope, '')
                   AND COALESCE(duplicate.scope_id, '') = COALESCE(notification_subscriptions.scope_id, '')
                   AND duplicate.is_enabled = TRUE
            )",
    )
    .bind(connection_id)
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    sqlx::query(
        "DELETE FROM notification_subscriptions
          WHERE channel_id = $2
            AND EXISTS (
                SELECT 1
                  FROM notification_subscriptions AS existing
                 WHERE existing.target_kind = 'media_server_connection'
                   AND existing.target_id = $1
                   AND existing.event_type = notification_subscriptions.event_type
                   AND COALESCE(existing.scope, '') = COALESCE(notification_subscriptions.scope, '')
                   AND COALESCE(existing.scope_id, '') = COALESCE(notification_subscriptions.scope_id, '')
            )",
    )
    .bind(connection_id)
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    sqlx::query(
        "UPDATE notification_subscriptions
            SET channel_id = NULL,
                target_kind = 'media_server_connection',
                target_id = $1
          WHERE channel_id = $2",
    )
    .bind(connection_id)
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    Ok(())
}

async fn sqlite_delete_notification_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM notification_channels WHERE id = ?1")
        .bind(channel_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    Ok(())
}

async fn postgres_delete_notification_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM notification_channels WHERE id = $1")
        .bind(channel_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    Ok(())
}

async fn sqlite_upsert_jellyfin_details(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    connection_id: &str,
    channel: &LegacyJellyfinChannel,
    config: &LegacyJellyfinConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()> {
    let api_key = stored_api_key(encryption_key, &config.api_key)?;
    sqlx::query(
        "INSERT INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(connection_id) DO UPDATE SET
             api_key = excluded.api_key,
             updated_at = excluded.updated_at",
    )
    .bind(connection_id)
    .bind(api_key)
    .bind(&channel.created_at)
    .bind(&channel.updated_at)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn postgres_upsert_jellyfin_details(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    connection_id: &str,
    channel: &LegacyJellyfinChannel,
    config: &LegacyJellyfinConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()> {
    let api_key = stored_api_key(encryption_key, &config.api_key)?;
    sqlx::query(
        "INSERT INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
         VALUES ($1, $2, $3::timestamptz, $4::timestamptz)
         ON CONFLICT(connection_id) DO UPDATE SET
             api_key = excluded.api_key,
             updated_at = excluded.updated_at",
    )
    .bind(connection_id)
    .bind(api_key)
    .bind(&channel.created_at)
    .bind(&channel.updated_at)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn sqlite_insert_path_mappings(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel_id: &str,
    connection_id: &str,
    config: &LegacyJellyfinConfig,
) -> AppResult<()> {
    for (index, (source_path, destination_path)) in config.path_mappings.iter().enumerate() {
        sqlx::query(
            "INSERT OR IGNORE INTO media_server_path_mappings (
                 id, connection_id, source_path, destination_path, sort_order
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(format!("notification-path-mapping-{channel_id}-{index}"))
        .bind(connection_id)
        .bind(source_path)
        .bind(destination_path)
        .bind(i64::try_from(index).unwrap_or(i64::MAX))
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    Ok(())
}

async fn postgres_insert_path_mappings(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: &str,
    connection_id: &str,
    config: &LegacyJellyfinConfig,
) -> AppResult<()> {
    for (index, (source_path, destination_path)) in config.path_mappings.iter().enumerate() {
        sqlx::query(
            "INSERT INTO media_server_path_mappings (
                 id, connection_id, source_path, destination_path, sort_order
             ) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(format!("notification-path-mapping-{channel_id}-{index}"))
        .bind(connection_id)
        .bind(source_path)
        .bind(destination_path)
        .bind(i64::try_from(index).unwrap_or(i64::MAX))
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    Ok(())
}

async fn sqlite_disable_malformed_jellyfin_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel_id: &str,
    reason: &str,
) -> AppResult<()> {
    tracing::warn!(
        channel_id,
        reason,
        "legacy Jellyfin notification channel could not be migrated; leaving row disabled"
    );
    sqlx::query(
        "UPDATE notification_channels
            SET is_enabled = 0,
                updated_at = CURRENT_TIMESTAMP
          WHERE id = ?1",
    )
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn postgres_disable_malformed_jellyfin_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: &str,
    reason: &str,
) -> AppResult<()> {
    tracing::warn!(
        channel_id,
        reason,
        "legacy Jellyfin notification channel could not be migrated; leaving row disabled"
    );
    sqlx::query(
        "UPDATE notification_channels
            SET is_enabled = FALSE,
                updated_at = NOW()
          WHERE id = $1",
    )
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(())
}

fn stored_api_key(encryption_key: Option<&EncryptionKey>, api_key: &str) -> AppResult<String> {
    match encryption_key {
        Some(key) => encrypt_value(key, api_key)
            .map_err(|error| AppError::Repository(format!("failed to encrypt api_key: {error}"))),
        None => Ok(api_key.to_string()),
    }
}

fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn parse_path_mappings(value: &str) -> Vec<(String, String)> {
    value
        .replace('\r', "")
        .lines()
        .filter_map(|line| {
            let (source, destination) = line.split_once("=>")?;
            let source = source.trim();
            let destination = destination.trim();
            if source.is_empty() || destination.is_empty() {
                return None;
            }
            Some((source.to_string(), destination.to_string()))
        })
        .collect()
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    const NOW: &str = "2026-05-30T00:00:00Z";

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool should open");
        sqlx::raw_sql(
            "
            CREATE TABLE notification_channels (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                channel_type TEXT NOT NULL,
                config_json TEXT NOT NULL,
                media_server_connection_id TEXT,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE notification_subscriptions (
                id TEXT PRIMARY KEY,
                channel_id TEXT,
                target_kind TEXT NOT NULL DEFAULT 'plugin_channel',
                target_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                scope TEXT NOT NULL,
                scope_id TEXT,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

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
                updated_at TEXT NOT NULL
            );

            CREATE TABLE jellyfin_media_server_details (
                connection_id TEXT PRIMARY KEY,
                api_key TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE media_server_path_mappings (
                id TEXT PRIMARY KEY,
                connection_id TEXT NOT NULL,
                source_path TEXT NOT NULL,
                destination_path TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0
            );
            ",
        )
        .execute(&pool)
        .await
        .expect("test schema should create");
        pool
    }

    async fn insert_connection(pool: &sqlx::SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO media_server_connections (
                 id, provider, display_name, base_url, enabled, login_enabled,
                 linking_enabled, auto_add_enabled, default_app_permissions, created_at, updated_at
             ) VALUES (?1, 'jellyfin', 'Jellyfin', 'http://jellyfin:8096', 1, 0, 0, 0, 0, ?2, ?2)",
        )
        .bind(id)
        .bind(NOW)
        .execute(pool)
        .await
        .expect("connection should insert");
    }

    async fn insert_jellyfin_channel(
        pool: &sqlx::SqlitePool,
        id: &str,
        config_json: &str,
        connection_id: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO notification_channels (
                 id, name, channel_type, config_json, media_server_connection_id,
                 is_enabled, created_at, updated_at
             ) VALUES (?1, ?1, 'jellyfin', ?2, ?3, 1, ?4, ?4)",
        )
        .bind(id)
        .bind(config_json)
        .bind(connection_id)
        .bind(NOW)
        .execute(pool)
        .await
        .expect("channel should insert");
    }

    async fn insert_subscription(
        pool: &sqlx::SqlitePool,
        id: &str,
        channel_id: &str,
        enabled: i64,
    ) {
        sqlx::query(
            "INSERT INTO notification_subscriptions (
                 id, channel_id, target_kind, target_id, event_type, scope,
                 scope_id, is_enabled, created_at, updated_at
             ) VALUES (?1, ?2, 'plugin_channel', ?2, 'grab', 'global', NULL, ?3, ?4, ?4)",
        )
        .bind(id)
        .bind(channel_id)
        .bind(enabled)
        .bind(NOW)
        .execute(pool)
        .await
        .expect("subscription should insert");
    }

    async fn run_sqlite_hook(pool: &sqlx::SqlitePool) {
        let mut tx = pool.begin().await.expect("transaction should begin");
        migrate_jellyfin_notification_channels_to_media_server_targets_sqlite(&mut tx, None)
            .await
            .expect("migration hook should run");
        tx.commit().await.expect("transaction should commit");
    }

    #[tokio::test]
    async fn sqlite_hook_moves_pointer_only_jellyfin_subscriptions_to_media_server_target() {
        let pool = test_pool().await;
        insert_connection(&pool, "conn-1").await;
        insert_jellyfin_channel(&pool, "channel-1", "{}", Some("conn-1")).await;
        insert_subscription(&pool, "sub-1", "channel-1", 1).await;

        run_sqlite_hook(&pool).await;

        let target: (Option<String>, String, String) = sqlx::query_as(
            "SELECT channel_id, target_kind, target_id
               FROM notification_subscriptions
              WHERE id = 'sub-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("subscription should load");
        assert_eq!(target.0, None);
        assert_eq!(target.1, "media_server_connection");
        assert_eq!(target.2, "conn-1");

        let channel_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM notification_channels WHERE id = 'channel-1'")
                .fetch_one(&pool)
                .await
                .expect("channel count should load");
        assert_eq!(channel_count, 0);
    }

    #[tokio::test]
    async fn sqlite_hook_merges_duplicate_legacy_subscriptions_before_unique_index() {
        let pool = test_pool().await;
        insert_connection(&pool, "conn-1").await;
        insert_jellyfin_channel(&pool, "channel-a", "{}", Some("conn-1")).await;
        insert_jellyfin_channel(&pool, "channel-b", "{}", Some("conn-1")).await;
        insert_subscription(&pool, "sub-a", "channel-a", 0).await;
        insert_subscription(&pool, "sub-b", "channel-b", 1).await;

        run_sqlite_hook(&pool).await;

        sqlx::raw_sql(
            "CREATE UNIQUE INDEX idx_notification_subscriptions_target_scope
                ON notification_subscriptions (
                    target_kind,
                    target_id,
                    event_type,
                    COALESCE(scope, ''),
                    COALESCE(scope_id, '')
                );",
        )
        .execute(&pool)
        .await
        .expect("post-hook unique target index should create");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
               FROM notification_subscriptions
              WHERE target_kind = 'media_server_connection'
                AND target_id = 'conn-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("target subscription count should load");
        assert_eq!(count, 1);

        let enabled: i64 = sqlx::query_scalar(
            "SELECT is_enabled
               FROM notification_subscriptions
              WHERE target_kind = 'media_server_connection'
                AND target_id = 'conn-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("target subscription should load");
        assert_eq!(enabled, 1);
    }

    #[tokio::test]
    async fn sqlite_hook_creates_connection_for_plaintext_jellyfin_channel() {
        let pool = test_pool().await;
        insert_jellyfin_channel(
            &pool,
            "channel-1",
            r#"{"base_url":"http://jellyfin:8096/","api_key":"secret","path_mappings":"/data => /media"}"#,
            None,
        )
        .await;
        insert_subscription(&pool, "sub-1", "channel-1", 1).await;

        run_sqlite_hook(&pool).await;

        let connection: (String, String) = sqlx::query_as(
            "SELECT id, base_url
               FROM media_server_connections
              WHERE provider = 'jellyfin'",
        )
        .fetch_one(&pool)
        .await
        .expect("connection should load");
        assert_eq!(connection.0, "jellyfin-notification-channel-1");
        assert_eq!(connection.1, "http://jellyfin:8096");

        let target_id: String = sqlx::query_scalar(
            "SELECT target_id
               FROM notification_subscriptions
              WHERE id = 'sub-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("subscription should load");
        assert_eq!(target_id, "jellyfin-notification-channel-1");
    }
}
