use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, IndexerConfigUpdate};
use scryer_domain::IndexerConfig;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::encryption::EncryptionKey;

use super::common::{parse_optional_utc_datetime, parse_utc_datetime};

fn row_to_indexer_config(
    row: &sqlx::sqlite::SqliteRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<IndexerConfig> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let provider_type: String = row
        .try_get("provider_type")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let base_url: String = row
        .try_get("base_url")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let api_key_encrypted: Option<String> = row
        .try_get("api_key_encrypted")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let rate_limit_seconds: Option<i64> = row
        .try_get("rate_limit_seconds")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let rate_limit_burst: Option<i64> = row
        .try_get("rate_limit_burst")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let disabled_until_raw: Option<String> = row
        .try_get("disabled_until")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let disabled_until = parse_optional_utc_datetime(disabled_until_raw)?;
    let is_enabled: i64 = row
        .try_get("is_enabled")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let enable_interactive_search: i64 = row.try_get("enable_interactive_search").unwrap_or(1);
    let enable_auto_search: i64 = row.try_get("enable_auto_search").unwrap_or(1);
    let managed_parent_config_id: Option<String> =
        row.try_get("managed_parent_config_id").unwrap_or(None);
    let managed_child_key: Option<String> = row.try_get("managed_child_key").unwrap_or(None);
    let managed_metadata_json: Option<String> =
        row.try_get("managed_metadata_json").unwrap_or(None);
    let last_health_status: Option<String> = row
        .try_get("last_health_status")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let last_error_at_raw: Option<String> = row
        .try_get("last_error_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let last_error_at = parse_optional_utc_datetime(last_error_at_raw)?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let updated_at_raw: String = row
        .try_get("updated_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let config_json_raw: Option<String> = row.try_get("config_json").unwrap_or(None);

    // Decrypt config_json if encrypted
    let config_json = match config_json_raw {
        Some(v) if crate::encryption::is_encrypted(&v) => {
            if let Some(key) = encryption_key {
                Some(crate::encryption::decrypt_value(key, &v).map_err(|e| {
                    AppError::Repository(format!("failed to decrypt config_json: {e}"))
                })?)
            } else {
                Some(v)
            }
        }
        other => other,
    };

    // Decrypt api_key_encrypted if it's encrypted
    let api_key_encrypted =
        match api_key_encrypted {
            Some(v) if crate::encryption::is_encrypted(&v) => {
                if let Some(key) = encryption_key {
                    Some(crate::encryption::decrypt_value(key, &v).map_err(|e| {
                        AppError::Repository(format!("failed to decrypt API key: {e}"))
                    })?)
                } else {
                    Some(v)
                }
            }
            other => other,
        };

    Ok(IndexerConfig {
        id,
        name,
        provider_type,
        base_url,
        api_key_encrypted,
        rate_limit_seconds,
        rate_limit_burst,
        disabled_until,
        is_enabled: is_enabled != 0,
        enable_interactive_search: enable_interactive_search != 0,
        enable_auto_search: enable_auto_search != 0,
        managed_parent_config_id,
        managed_child_key,
        managed_metadata_json,
        last_health_status,
        last_error_at,
        config_json,
        created_at: parse_utc_datetime(&created_at_raw)?,
        updated_at: parse_utc_datetime(&updated_at_raw)?,
    })
}

fn maybe_encrypt_config_json(
    key: Option<&EncryptionKey>,
    config_json: Option<&String>,
) -> AppResult<Option<String>> {
    let Some(config_json) = config_json else {
        return Ok(None);
    };
    let Some(key) = key else {
        return Ok(Some(config_json.clone()));
    };
    crate::encryption::encrypt_value(key, config_json)
        .map(Some)
        .map_err(|e| AppError::Repository(format!("failed to encrypt config_json: {e}")))
}

pub(crate) async fn list_indexer_configs_query(
    pool: &SqlitePool,
    provider_type: Option<String>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<IndexerConfig>> {
    let mut sql = String::from(
        "SELECT id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
                rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
                enable_auto_search, managed_parent_config_id, managed_child_key,
                managed_metadata_json, last_health_status, last_error_at, config_json, created_at, updated_at
         FROM indexers",
    );

    if provider_type.is_some() {
        sql.push_str(" WHERE provider_type = ?");
    }

    sql.push_str(" ORDER BY created_at DESC");

    let mut statement = sqlx::query(&sql);
    if let Some(provider) = provider_type {
        statement = statement.bind(provider);
    }

    let rows = statement
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_indexer_config(&row, encryption_key)?);
    }

    Ok(out)
}

pub(crate) async fn get_indexer_config_query(
    pool: &SqlitePool,
    id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<IndexerConfig>> {
    let row = sqlx::query(
        "SELECT id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
                rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
                enable_auto_search, managed_parent_config_id, managed_child_key,
                managed_metadata_json, last_health_status, last_error_at, config_json, created_at, updated_at
         FROM indexers WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.map(|row| row_to_indexer_config(&row, encryption_key))
        .transpose()
}

async fn get_indexer_config_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<IndexerConfig>> {
    let row = sqlx::query(
        "SELECT id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
                rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
                enable_auto_search, managed_parent_config_id, managed_child_key,
                managed_metadata_json, last_health_status, last_error_at, config_json, created_at, updated_at
         FROM indexers WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.map(|row| row_to_indexer_config(&row, encryption_key))
        .transpose()
}

pub(crate) async fn migrate_legacy_indexer_config_sources_query(
    pool: &SqlitePool,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<u64> {
    let configs = list_indexer_configs_query(pool, None, encryption_key).await?;
    let mut migrated = 0_u64;

    for config in configs {
        if config.base_url.trim().is_empty() && config.api_key_encrypted.is_none() {
            continue;
        }

        let mut object = config
            .config_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();

        let connection_key = legacy_indexer_connection_key(&config.provider_type);
        let mut changed = false;
        if !config.base_url.trim().is_empty() && config_value_missing(object.get(connection_key)) {
            object.insert(
                connection_key.to_string(),
                serde_json::Value::String(config.base_url.trim().to_string()),
            );
            changed = true;
        }

        let legacy_api_key = config
            .api_key_encrypted
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(api_key) = legacy_api_key
            && config_value_missing(object.get("api_key"))
        {
            object.insert(
                "api_key".to_string(),
                serde_json::Value::String(api_key.to_string()),
            );
            changed = true;
        }

        let clear_legacy_api_key = config.api_key_encrypted.is_some();
        if !changed && !clear_legacy_api_key {
            continue;
        }

        let now = Utc::now().to_rfc3339();
        if changed {
            let normalized = serde_json::Value::Object(object).to_string();
            let stored = maybe_encrypt_config_json(encryption_key, Some(&normalized))?;
            sqlx::query(
                "UPDATE indexers
                 SET config_json = ?, api_key_encrypted = NULL, updated_at = ?
                 WHERE id = ?",
            )
            .bind(stored)
            .bind(&now)
            .bind(&config.id)
            .execute(pool)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        } else {
            sqlx::query(
                "UPDATE indexers
                 SET api_key_encrypted = NULL, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&now)
            .bind(&config.id)
            .execute(pool)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        }
        migrated += 1;
    }

    Ok(migrated)
}

fn legacy_indexer_connection_key(provider_type: &str) -> &'static str {
    if provider_type.eq_ignore_ascii_case("torrent_rss") {
        "feed_url"
    } else {
        "base_url"
    }
}

fn config_value_missing(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(value)) => value.trim().is_empty(),
        _ => false,
    }
}

pub(crate) async fn create_indexer_config_query(
    pool: &SqlitePool,
    config: &IndexerConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<IndexerConfig> {
    let stored_api_key: Option<String> = None;
    let stored_config_json =
        maybe_encrypt_config_json(encryption_key, config.config_json.as_ref())?;

    sqlx::query(
        "INSERT INTO indexers
         (id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
          rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
            enable_auto_search, managed_parent_config_id, managed_child_key,
            managed_metadata_json, last_health_status, last_error_at, config_json, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.provider_type)
    .bind(&config.base_url)
    .bind(&stored_api_key)
    .bind(config.rate_limit_seconds)
    .bind(config.rate_limit_burst)
    .bind(
        config
            .disabled_until
            .as_ref()
            .map(DateTime::<Utc>::to_rfc3339),
    )
    .bind(if config.is_enabled { 1_i64 } else { 0_i64 })
    .bind(if config.enable_interactive_search { 1_i64 } else { 0_i64 })
    .bind(if config.enable_auto_search { 1_i64 } else { 0_i64 })
    .bind(&config.managed_parent_config_id)
    .bind(&config.managed_child_key)
    .bind(&config.managed_metadata_json)
    .bind(&config.last_health_status)
    .bind(
        config
            .last_error_at
            .as_ref()
            .map(DateTime::<Utc>::to_rfc3339),
    )
    .bind(&stored_config_json)
    .bind(config.created_at.to_rfc3339())
    .bind(config.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(config.clone())
}

pub(crate) async fn update_indexer_config_query(
    pool: &SqlitePool,
    update: &IndexerConfigUpdate,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<IndexerConfig> {
    let mut assignments = vec!["updated_at = ?".to_string()];

    if update.name.is_some() {
        assignments.push("name = ?".to_string());
    }
    if update.provider_type.is_some() {
        assignments.push("provider_type = ?".to_string());
    }
    if update.derived_base_url.is_some() {
        assignments.push("base_url = ?".to_string());
    }
    if update.rate_limit_seconds.is_some() {
        assignments.push("rate_limit_seconds = ?".to_string());
    }
    if update.rate_limit_burst.is_some() {
        assignments.push("rate_limit_burst = ?".to_string());
    }
    if update.is_enabled.is_some() {
        assignments.push("is_enabled = ?".to_string());
    }
    if update.enable_interactive_search.is_some() {
        assignments.push("enable_interactive_search = ?".to_string());
    }
    if update.enable_auto_search.is_some() {
        assignments.push("enable_auto_search = ?".to_string());
    }
    if update.managed_parent_config_id.is_some() {
        assignments.push("managed_parent_config_id = ?".to_string());
    }
    if update.managed_child_key.is_some() {
        assignments.push("managed_child_key = ?".to_string());
    }
    if update.managed_metadata_json.is_some() {
        assignments.push("managed_metadata_json = ?".to_string());
    }
    if update.config_json.is_some() {
        assignments.push("config_json = ?".to_string());
    }

    if assignments.len() == 1 {
        return Err(AppError::Validation(
            "at least one indexer config field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE indexers SET ");
    sql.push_str(&assignments.join(", "));
    sql.push_str(" WHERE id = ?");

    let mut statement = sqlx::query(&sql);
    statement = statement.bind(Utc::now().to_rfc3339());

    if let Some(name) = update.name.as_ref() {
        statement = statement.bind(name);
    }
    if let Some(provider_type) = update.provider_type.as_ref() {
        statement = statement.bind(provider_type);
    }
    if let Some(base_url) = update.derived_base_url.as_ref() {
        statement = statement.bind(base_url);
    }
    if let Some(rate_limit_seconds) = update.rate_limit_seconds {
        statement = statement.bind(rate_limit_seconds);
    }
    if let Some(rate_limit_burst) = update.rate_limit_burst {
        statement = statement.bind(rate_limit_burst);
    }
    if let Some(is_enabled) = update.is_enabled {
        statement = statement.bind(if is_enabled { 1_i64 } else { 0_i64 });
    }
    if let Some(enable_interactive_search) = update.enable_interactive_search {
        statement = statement.bind(if enable_interactive_search {
            1_i64
        } else {
            0_i64
        });
    }
    if let Some(enable_auto_search) = update.enable_auto_search {
        statement = statement.bind(if enable_auto_search { 1_i64 } else { 0_i64 });
    }
    if let Some(managed_parent_config_id) = update.managed_parent_config_id.as_ref() {
        statement = statement.bind(managed_parent_config_id.as_deref());
    }
    if let Some(managed_child_key) = update.managed_child_key.as_ref() {
        statement = statement.bind(managed_child_key.as_deref());
    }
    if let Some(managed_metadata_json) = update.managed_metadata_json.as_ref() {
        statement = statement.bind(managed_metadata_json.as_deref());
    }
    if let Some(config_json) = update.config_json.as_ref() {
        let stored =
            maybe_encrypt_config_json(encryption_key, Some(config_json))?.unwrap_or_default();
        statement = statement.bind(stored);
    }

    statement = statement.bind(&update.id);

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let result = statement
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("indexer config {}", update.id)));
    }

    let config = get_indexer_config_tx(&mut tx, &update.id, encryption_key)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("indexer config {}", update.id)))?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(config)
}

pub(crate) async fn touch_indexer_last_error_query(
    pool: &SqlitePool,
    provider_type: &str,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE indexers
         SET last_error_at = ?, updated_at = ?
         WHERE provider_type = ?",
    )
    .bind(now.as_str())
    .bind(now.as_str())
    .bind(provider_type)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn delete_indexer_config_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM indexers WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("indexer config {}", id)));
    }

    Ok(())
}

// ── Indexer API quotas ──────────────────────────────────────────────────

/// Upsert quota snapshot for an indexer after a search response.
pub(crate) async fn upsert_indexer_quota(
    pool: &SqlitePool,
    indexer_id: &str,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO indexer_api_quotas (indexer_id, api_current, api_max, grab_current, grab_max, queries_today, last_query_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 1, datetime('now'), datetime('now'))
         ON CONFLICT(indexer_id) DO UPDATE SET
           api_current = excluded.api_current,
           api_max = excluded.api_max,
           grab_current = excluded.grab_current,
           grab_max = excluded.grab_max,
           queries_today = CASE
             WHEN julianday('now') - julianday(indexer_api_quotas.last_reset_at) >= 1.0
             THEN 1
             ELSE indexer_api_quotas.queries_today + 1
           END,
           last_reset_at = CASE
             WHEN julianday('now') - julianday(indexer_api_quotas.last_reset_at) >= 1.0
             THEN datetime('now')
             ELSE indexer_api_quotas.last_reset_at
           END,
           last_query_at = datetime('now'),
           updated_at = datetime('now')",
    )
    .bind(indexer_id)
    .bind(api_current.map(|v| v as i64))
    .bind(api_max.map(|v| v as i64))
    .bind(grab_current.map(|v| v as i64))
    .bind(grab_max.map(|v| v as i64))
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

/// Check if an indexer is at or near its API quota.
/// Returns true if the indexer should be skipped.
#[allow(dead_code)]
pub(crate) async fn is_indexer_at_quota(pool: &SqlitePool, indexer_id: &str) -> AppResult<bool> {
    let row =
        sqlx::query("SELECT api_current, api_max FROM indexer_api_quotas WHERE indexer_id = ?")
            .bind(indexer_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Repository(e.to_string()))?;

    let Some(row) = row else { return Ok(false) };
    let current: Option<i64> = row.try_get("api_current").unwrap_or(None);
    let max: Option<i64> = row.try_get("api_max").unwrap_or(None);

    match (current, max) {
        (Some(c), Some(m)) if m > 0 => Ok(c >= (m * 95 / 100)),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::{get_indexer_config_query, migrate_legacy_indexer_config_sources_query};

    async fn create_test_indexers_table(pool: &sqlx::SqlitePool) {
        sqlx::query(
            "CREATE TABLE indexers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                provider_type TEXT NOT NULL,
                base_url TEXT NOT NULL DEFAULT '',
                api_key_encrypted TEXT,
                rate_limit_seconds INTEGER,
                rate_limit_burst INTEGER,
                disabled_until TEXT,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                enable_interactive_search INTEGER NOT NULL DEFAULT 1,
                enable_auto_search INTEGER NOT NULL DEFAULT 1,
                managed_parent_config_id TEXT,
                managed_child_key TEXT,
                managed_metadata_json TEXT,
                last_health_status TEXT,
                last_error_at TEXT,
                config_json TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await
        .expect("indexers table should be created");
    }

    #[tokio::test]
    async fn migration_clears_legacy_api_key_when_config_json_already_has_it() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_test_indexers_table(&pool).await;

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO indexers (
                id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
                rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
                enable_auto_search, last_health_status, last_error_at, config_json, created_at,
                updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("idx-1")
        .bind("Test Indexer")
        .bind("newznab")
        .bind("")
        .bind("legacy-secret")
        .bind(None::<i64>)
        .bind(None::<i64>)
        .bind(None::<String>)
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(r#"{"api_key":"config-secret"}"#)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("legacy row should insert");

        let migrated = migrate_legacy_indexer_config_sources_query(&pool, None)
            .await
            .expect("migration should succeed");
        assert_eq!(migrated, 1);

        let config = get_indexer_config_query(&pool, "idx-1", None)
            .await
            .expect("query should succeed")
            .expect("config should exist");
        assert_eq!(
            config.config_json.as_deref(),
            Some(r#"{"api_key":"config-secret"}"#)
        );
        assert_eq!(config.api_key_encrypted, None);
    }
}
