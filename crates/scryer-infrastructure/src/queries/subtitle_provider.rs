use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, SubtitleProviderConfigUpdate};
use scryer_domain::SubtitleProviderConfig;
use serde_json::Value;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::encryption::EncryptionKey;

use super::common::{parse_optional_utc_datetime, parse_utc_datetime};

fn row_to_subtitle_provider_config(
    row: &sqlx::sqlite::SqliteRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<SubtitleProviderConfig> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let provider_type: String = row
        .try_get("provider_type")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let config_json_raw: String = row
        .try_get("config_json")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let is_enabled: i64 = row
        .try_get("is_enabled")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let enabled_facets_raw: String = row
        .try_get("enabled_facets")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let last_health_status: Option<String> = row
        .try_get("last_health_status")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let last_error: Option<String> = row
        .try_get("last_error")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let last_error_at_raw: Option<String> = row
        .try_get("last_error_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let disabled_until_raw: Option<String> = row
        .try_get("disabled_until")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let updated_at_raw: String = row
        .try_get("updated_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let config_json = if crate::encryption::is_encrypted(&config_json_raw) {
        if let Some(key) = encryption_key {
            crate::encryption::decrypt_value(key, &config_json_raw)
                .map_err(|e| AppError::Repository(format!("failed to decrypt config_json: {e}")))?
        } else {
            return Err(AppError::Repository(
                "encrypted config_json requires encryption key".to_string(),
            ));
        }
    } else {
        config_json_raw
    };

    Ok(SubtitleProviderConfig {
        id,
        name,
        provider_type,
        config_json,
        enabled_facets: parse_enabled_facets(&enabled_facets_raw)?,
        is_enabled: is_enabled != 0,
        last_health_status,
        last_error,
        last_error_at: parse_optional_utc_datetime(last_error_at_raw)?,
        disabled_until: parse_optional_utc_datetime(disabled_until_raw)?,
        created_at: parse_utc_datetime(&created_at_raw)?,
        updated_at: parse_utc_datetime(&updated_at_raw)?,
    })
}

fn parse_enabled_facets(raw: &str) -> AppResult<Vec<String>> {
    let value = serde_json::from_str::<Value>(raw)
        .map_err(|err| AppError::Repository(format!("invalid enabled_facets JSON: {err}")))?;
    let Value::Array(values) = value else {
        return Err(AppError::Repository(
            "enabled_facets must be a JSON array".to_string(),
        ));
    };

    Ok(values
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect())
}

fn serialize_enabled_facets(enabled_facets: &[String]) -> AppResult<String> {
    serde_json::to_string(enabled_facets)
        .map_err(|err| AppError::Repository(format!("invalid enabled_facets: {err}")))
}

fn maybe_encrypt_config_json(key: Option<&EncryptionKey>, config_json: &str) -> AppResult<String> {
    let Some(key) = key else {
        return Ok(config_json.to_string());
    };
    crate::encryption::encrypt_value(key, config_json)
        .map_err(|e| AppError::Repository(format!("failed to encrypt config_json: {e}")))
}

pub(crate) async fn list_subtitle_provider_configs_query(
    pool: &SqlitePool,
    provider_type: Option<String>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SubtitleProviderConfig>> {
    let mut sql = String::from(
        "SELECT id, name, provider_type, config_json, is_enabled,
                enabled_facets,
                last_health_status, last_error, last_error_at, disabled_until,
                created_at, updated_at
           FROM subtitle_provider_configs",
    );

    if provider_type.is_some() {
        sql.push_str(" WHERE provider_type = ?");
    }

    sql.push_str(" ORDER BY created_at DESC");

    let mut statement = sqlx::query(&sql);
    if let Some(provider_type) = provider_type {
        statement = statement.bind(provider_type);
    }

    let rows = statement
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_subtitle_provider_config(&row, encryption_key)?);
    }

    Ok(out)
}

pub(crate) async fn get_subtitle_provider_config_query(
    pool: &SqlitePool,
    id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<SubtitleProviderConfig>> {
    let row = sqlx::query(
        "SELECT id, name, provider_type, config_json, is_enabled,
                enabled_facets,
                last_health_status, last_error, last_error_at, disabled_until,
                created_at, updated_at
           FROM subtitle_provider_configs
          WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.map(|row| row_to_subtitle_provider_config(&row, encryption_key))
        .transpose()
}

async fn get_subtitle_provider_config_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<SubtitleProviderConfig>> {
    let row = sqlx::query(
        "SELECT id, name, provider_type, config_json, is_enabled,
                enabled_facets,
                last_health_status, last_error, last_error_at, disabled_until,
                created_at, updated_at
           FROM subtitle_provider_configs
          WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.map(|row| row_to_subtitle_provider_config(&row, encryption_key))
        .transpose()
}

pub(crate) async fn create_subtitle_provider_config_query(
    pool: &SqlitePool,
    config: &SubtitleProviderConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<SubtitleProviderConfig> {
    let stored_config_json = maybe_encrypt_config_json(encryption_key, &config.config_json)?;
    let enabled_facets_json = serialize_enabled_facets(&config.enabled_facets)?;

    sqlx::query(
        "INSERT INTO subtitle_provider_configs
            (id, name, provider_type, config_json, is_enabled,
             enabled_facets, last_health_status, last_error, last_error_at,
             disabled_until, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.provider_type)
    .bind(&stored_config_json)
    .bind(if config.is_enabled { 1_i64 } else { 0_i64 })
    .bind(&enabled_facets_json)
    .bind(&config.last_health_status)
    .bind(&config.last_error)
    .bind(
        config
            .last_error_at
            .as_ref()
            .map(DateTime::<Utc>::to_rfc3339),
    )
    .bind(
        config
            .disabled_until
            .as_ref()
            .map(DateTime::<Utc>::to_rfc3339),
    )
    .bind(config.created_at.to_rfc3339())
    .bind(config.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(config.clone())
}

pub(crate) async fn update_subtitle_provider_config_query(
    pool: &SqlitePool,
    update: &SubtitleProviderConfigUpdate,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<SubtitleProviderConfig> {
    let mut assignments = vec!["updated_at = ?".to_string()];

    if update.name.is_some() {
        assignments.push("name = ?".to_string());
    }
    if update.provider_type.is_some() {
        assignments.push("provider_type = ?".to_string());
    }
    if update.config_json.is_some() {
        assignments.push("config_json = ?".to_string());
    }
    if update.enabled_facets.is_some() {
        assignments.push("enabled_facets = ?".to_string());
    }
    if update.is_enabled.is_some() {
        assignments.push("is_enabled = ?".to_string());
    }
    if update.last_health_status.is_some() {
        assignments.push("last_health_status = ?".to_string());
    }
    if update.last_error.is_some() {
        assignments.push("last_error = ?".to_string());
    }
    if update.last_error_at.is_some() {
        assignments.push("last_error_at = ?".to_string());
    }

    if assignments.len() == 1 {
        return Err(AppError::Validation(
            "at least one subtitle provider config field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE subtitle_provider_configs SET ");
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
    if let Some(config_json) = update.config_json.as_ref() {
        let stored = maybe_encrypt_config_json(encryption_key, config_json)?;
        statement = statement.bind(stored);
    }
    if let Some(enabled_facets) = update.enabled_facets.as_ref() {
        statement = statement.bind(serialize_enabled_facets(enabled_facets)?);
    }
    if let Some(is_enabled) = update.is_enabled {
        statement = statement.bind(if is_enabled { 1_i64 } else { 0_i64 });
    }
    if let Some(last_health_status) = update.last_health_status.as_ref() {
        statement = statement.bind(last_health_status);
    }
    if let Some(last_error) = update.last_error.as_ref() {
        statement = statement.bind(last_error.as_ref());
    }
    if let Some(last_error_at) = update.last_error_at.as_ref() {
        statement = statement.bind(last_error_at.as_ref().map(DateTime::<Utc>::to_rfc3339));
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
        return Err(AppError::NotFound(format!(
            "subtitle provider config {}",
            update.id
        )));
    }

    let config = get_subtitle_provider_config_tx(&mut tx, &update.id, encryption_key)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("subtitle provider config {}", update.id)))?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(config)
}

pub(crate) async fn delete_subtitle_provider_config_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM subtitle_provider_configs WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "subtitle provider config {}",
            id
        )));
    }

    Ok(())
}
