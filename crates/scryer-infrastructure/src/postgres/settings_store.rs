use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{AppError, AppResult, QualityProfile, parse_profile_catalog_from_json};
use scryer_domain::Id;
use serde_json::Value;
use sqlx::Row;

use crate::encryption::{EncryptionKey, decrypt_value, encrypt_value, is_encrypted};
use crate::settings_store::{SettingsSql, SettingsStore};
use crate::{MigrationStatus, SettingDefinitionSeed, SettingsValueRecord};

pub type PostgresSettingsStore = SettingsStore<PostgresSettingsSql>;

#[derive(Clone)]
pub struct PostgresSettingsSql {
    pool: sqlx::PgPool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SettingsStore<PostgresSettingsSql> {
    pub fn new(db: &super::PostgresServices) -> Self {
        Self::from_sql(PostgresSettingsSql::new(db))
    }
}

impl PostgresSettingsSql {
    fn new(db: &super::PostgresServices) -> Self {
        Self {
            pool: db.pool().clone(),
            encryption_key: db.encryption_key_state(),
        }
    }

    async fn setting_definition_meta(
        &self,
        scope: &str,
        key_name: &str,
    ) -> AppResult<Option<(String, bool)>> {
        sqlx::query_as::<_, (String, bool)>(
            "SELECT id, is_sensitive
               FROM settings_definitions
              WHERE scope = $1 AND key_name = $2",
        )
        .bind(scope)
        .bind(key_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))
    }
}

#[async_trait]
impl SettingsSql for PostgresSettingsSql {
    fn engine(&self) -> &'static str {
        "postgres"
    }

    fn encryption_key(&self) -> Option<EncryptionKey> {
        self.encryption_key
            .read()
            .ok()
            .and_then(|value| value.clone())
    }

    async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<SettingDefinitionSeed>,
    ) -> AppResult<()> {
        for definition in definitions {
            let default_value_json = parse_json(&definition.default_value_json)?;
            let validation_json = definition
                .validation_json
                .as_deref()
                .map(parse_json)
                .transpose()?;
            let now = Utc::now();
            sqlx::query(
                "INSERT INTO settings_definitions
                    (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8::jsonb, $9, $10)
                 ON CONFLICT (scope, key_name) DO UPDATE
                    SET category = EXCLUDED.category,
                        data_type = EXCLUDED.data_type,
                        default_value_json = EXCLUDED.default_value_json,
                        is_sensitive = EXCLUDED.is_sensitive,
                        validation_json = EXCLUDED.validation_json,
                        updated_at = EXCLUDED.updated_at",
            )
            .bind(Id::new().0)
            .bind(definition.category)
            .bind(definition.scope)
            .bind(definition.key_name)
            .bind(definition.data_type)
            .bind(default_value_json)
            .bind(definition.is_sensitive)
            .bind(validation_json)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }

        Ok(())
    }

    async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<SettingsValueRecord>>> {
        let mut out = Vec::with_capacity(keys.len());
        for (scope, key_name, scope_id) in keys {
            out.push(
                self.get_setting_with_defaults(scope, key_name, scope_id)
                    .await?,
            );
        }
        Ok(out)
    }

    async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        for (scope, key_name, value_json, source) in entries {
            let existing = self
                .get_setting_with_defaults(scope.clone(), key_name.clone(), None)
                .await?;
            if existing
                .as_ref()
                .is_some_and(SettingsValueRecord::has_override)
            {
                continue;
            }
            self.upsert_setting_value(scope, key_name, None, value_json, source, None)
                .await?;
        }
        Ok(())
    }

    async fn list_settings_with_defaults(
        &self,
        scope: String,
        scope_id: Option<String>,
    ) -> AppResult<Vec<SettingsValueRecord>> {
        let normalized_scope_id = normalize_optional(scope_id);
        let rows = sqlx::query(
            "SELECT
                d.id AS definition_id,
                d.category,
                d.scope,
                d.key_name,
                d.data_type,
                d.default_value_json,
                d.is_sensitive,
                d.validation_json,
                COALESCE(sv.value_json, d.default_value_json) AS effective_value_json,
                sv.value_json,
                sv.source,
                sv.scope_id,
                sv.updated_by_user_id,
                sv.created_at::TEXT AS created_at,
                sv.updated_at::TEXT AS updated_at
             FROM settings_definitions d
             LEFT JOIN settings_values sv
               ON sv.setting_definition_id = d.id
              AND sv.scope = d.scope
              AND (($2::TEXT IS NULL AND sv.scope_id IS NULL) OR sv.scope_id = $2)
             WHERE d.scope = $1
             ORDER BY d.category, d.key_name",
        )
        .bind(scope)
        .bind(normalized_scope_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let encryption_key = self.encryption_key();
        rows.iter()
            .map(|row| decode_settings_row(row, encryption_key.as_ref()))
            .collect()
    }

    async fn get_setting_with_defaults(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>> {
        Ok(self
            .list_settings_with_defaults(scope, scope_id)
            .await?
            .into_iter()
            .find(|record| record.key_name == key_name))
    }

    async fn get_setting_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>> {
        let normalized_scope_id = normalize_optional(scope_id);
        let row = sqlx::query(
            "SELECT
                d.id AS definition_id,
                d.category,
                d.scope,
                d.key_name,
                d.data_type,
                d.default_value_json,
                d.is_sensitive,
                d.validation_json,
                sv.value_json AS effective_value_json,
                sv.value_json,
                sv.source,
                sv.scope_id,
                sv.updated_by_user_id,
                sv.created_at::TEXT AS created_at,
                sv.updated_at::TEXT AS updated_at
             FROM settings_definitions d
             JOIN settings_values sv
               ON sv.setting_definition_id = d.id
              AND sv.scope = d.scope
             WHERE d.scope = $1
               AND d.key_name = $2
               AND (($3::TEXT IS NULL AND sv.scope_id IS NULL) OR sv.scope_id = $3)
             LIMIT 1",
        )
        .bind(scope)
        .bind(key_name)
        .bind(normalized_scope_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let encryption_key = self.encryption_key();
        row.as_ref()
            .map(|row| decode_settings_row(row, encryption_key.as_ref()))
            .transpose()
    }

    #[expect(clippy::too_many_arguments)]
    async fn upsert_setting_value(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
        value_json: String,
        source: String,
        updated_by_user_id: Option<String>,
    ) -> AppResult<SettingsValueRecord> {
        let normalized_scope_id = normalize_optional(scope_id);
        let parsed_value = parse_json(&value_json)?;

        let Some((definition_id, is_sensitive)) =
            self.setting_definition_meta(&scope, &key_name).await?
        else {
            return Err(AppError::Validation(format!(
                "unknown setting key: {scope}.{key_name}"
            )));
        };

        let stored_value = if is_sensitive {
            match self.encryption_key() {
                Some(key) => Value::String(encrypt_value(&key, &value_json).map_err(|error| {
                    AppError::Repository(format!("failed to encrypt setting value: {error}"))
                })?),
                None => parsed_value,
            }
        } else {
            parsed_value
        };

        let now = Utc::now();
        let existing_id: Option<String> = sqlx::query_scalar(
            "SELECT id
               FROM settings_values
              WHERE setting_definition_id = $1
                AND (($2::TEXT IS NULL AND scope_id IS NULL) OR scope_id = $2)",
        )
        .bind(&definition_id)
        .bind(&normalized_scope_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        if let Some(existing_id) = existing_id {
            sqlx::query(
                "UPDATE settings_values
                    SET value_json = $2::jsonb,
                        source = $3,
                        updated_by_user_id = $4,
                        updated_at = $5
                  WHERE id = $1",
            )
            .bind(existing_id)
            .bind(stored_value)
            .bind(source)
            .bind(updated_by_user_id)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        } else {
            sqlx::query(
                "INSERT INTO settings_values
                    (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7, $8, $9)",
            )
            .bind(Id::new().0)
            .bind(&definition_id)
            .bind(&scope)
            .bind(&normalized_scope_id)
            .bind(stored_value)
            .bind(source)
            .bind(updated_by_user_id)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }

        self.get_setting_with_defaults(scope, key_name, normalized_scope_id)
            .await?
            .ok_or_else(|| AppError::Repository("setting write did not return a row".to_string()))
    }

    async fn delete_setting_value(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        let Some((definition_id, _)) = self.setting_definition_meta(&scope, &key_name).await?
        else {
            return Ok(());
        };
        let normalized_scope_id = normalize_optional(scope_id);
        sqlx::query(
            "DELETE FROM settings_values
              WHERE setting_definition_id = $1
                AND (($2::TEXT IS NULL AND scope_id IS NULL) OR scope_id = $2)",
        )
        .bind(definition_id)
        .bind(normalized_scope_id)
        .execute(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        let result = sqlx::query("DELETE FROM settings_values WHERE scope_id = $1")
            .bind(scope_id)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(result.rows_affected().min(u32::MAX as u64) as u32)
    }

    async fn list_applied_migrations(&self) -> AppResult<Vec<MigrationStatus>> {
        super::migrations::list_applied_migrations(&self.pool).await
    }

    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        let normalized_scope_id = normalize_optional(scope_id);
        let profiles_json: Option<Value> = sqlx::query_scalar(
            "SELECT profiles_json
               FROM quality_profiles_json
              WHERE scope = $1
                AND (($2::TEXT IS NULL AND scope_id IS NULL) OR scope_id = $2)",
        )
        .bind(scope)
        .bind(normalized_scope_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        match profiles_json {
            Some(value) => parse_profile_catalog_from_json(&value.to_string())
                .map_err(|error| AppError::Repository(error.to_string())),
            None => Ok(Vec::new()),
        }
    }

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        let normalized_scope_id = normalize_optional(scope_id);
        let profiles_json = serde_json::to_value(profiles)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1
                   FROM quality_profiles_json
                  WHERE scope = $1
                    AND (($2::TEXT IS NULL AND scope_id IS NULL) OR scope_id = $2)
             )",
        )
        .bind(scope)
        .bind(&normalized_scope_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let now = Utc::now();
        if existing {
            sqlx::query(
                "UPDATE quality_profiles_json
                    SET profiles_json = $3::jsonb, updated_at = $4
                  WHERE scope = $1
                    AND (($2::TEXT IS NULL AND scope_id IS NULL) OR scope_id = $2)",
            )
            .bind(scope)
            .bind(normalized_scope_id)
            .bind(profiles_json)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        } else {
            sqlx::query(
                "INSERT INTO quality_profiles_json (scope, scope_id, profiles_json, updated_at)
                 VALUES ($1, $2, $3::jsonb, $4)",
            )
            .bind(scope)
            .bind(normalized_scope_id)
            .bind(profiles_json)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }

        Ok(())
    }

    async fn current_migration_version(&self) -> AppResult<Option<String>> {
        let latest = sqlx::query_as::<_, (i64, String)>(
            "SELECT version, description
               FROM _sqlx_migrations
              WHERE success = TRUE
              ORDER BY version DESC, description DESC
              LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        Ok(latest.map(|(version, description)| {
            crate::migration_assets::migration_key_from_version_and_desc(version, &description)
        }))
    }
}

fn parse_json(raw: &str) -> AppResult<Value> {
    match serde_json::from_str(raw) {
        Ok(value) => Ok(value),
        Err(_) => Ok(Value::String(raw.to_string())),
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    })
}

fn json_to_logical_string(
    value: Value,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<String> {
    match value {
        Value::String(stored) if is_encrypted(&stored) => {
            let key = encryption_key.ok_or_else(|| {
                AppError::Repository(
                    "encrypted PostgreSQL setting cannot be read without an encryption key"
                        .to_string(),
                )
            })?;
            decrypt_value(key, &stored).map_err(|error| {
                AppError::Repository(format!("failed to decrypt setting: {error}"))
            })
        }
        value => {
            serde_json::to_string(&value).map_err(|error| AppError::Repository(error.to_string()))
        }
    }
}

fn decode_settings_row(
    row: &sqlx::postgres::PgRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<SettingsValueRecord> {
    let default_value_json = json_to_logical_string(
        row.try_get("default_value_json")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        None,
    )?;
    let validation_json = row
        .try_get::<Option<Value>, _>("validation_json")
        .map_err(|error| AppError::Repository(error.to_string()))?
        .map(|value| json_to_logical_string(value, None))
        .transpose()?;
    let effective_value_json = json_to_logical_string(
        row.try_get("effective_value_json")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        encryption_key,
    )?;
    let value_json = row
        .try_get::<Option<Value>, _>("value_json")
        .map_err(|error| AppError::Repository(error.to_string()))?
        .map(|value| json_to_logical_string(value, encryption_key))
        .transpose()?;

    Ok(SettingsValueRecord {
        definition_id: row
            .try_get("definition_id")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        category: row
            .try_get("category")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        scope: row
            .try_get("scope")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        key_name: row
            .try_get("key_name")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        data_type: row
            .try_get("data_type")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        default_value_json,
        is_sensitive: row
            .try_get("is_sensitive")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        validation_json,
        effective_value_json,
        value_json,
        source: row
            .try_get("source")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        scope_id: row
            .try_get("scope_id")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        updated_by_user_id: row
            .try_get("updated_by_user_id")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|error| AppError::Repository(error.to_string()))?,
    })
}
