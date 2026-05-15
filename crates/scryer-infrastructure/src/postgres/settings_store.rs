use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, QualityProfile, QualityProfileCriteria, ScoringConfig,
};
use scryer_domain::Id;
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction};

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
            let id = format!(
                "{}:{}:{}",
                definition.category.trim(),
                definition.scope.trim(),
                definition.key_name.trim()
            );
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
                 ON CONFLICT (category, scope, key_name) DO UPDATE
                    SET category = EXCLUDED.category,
                        data_type = EXCLUDED.data_type,
                        default_value_json = EXCLUDED.default_value_json,
                        is_sensitive = EXCLUDED.is_sensitive,
                        validation_json = EXCLUDED.validation_json,
                        updated_at = EXCLUDED.updated_at",
            )
            .bind(id)
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
        list_quality_profiles_from_relational(&self.pool, scope, scope_id).await
    }

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        replace_quality_profiles_relational(&self.pool, scope, scope_id, profiles).await
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

async fn list_quality_profiles_from_relational(
    pool: &sqlx::PgPool,
    scope: &str,
    scope_id: Option<String>,
) -> AppResult<Vec<QualityProfile>> {
    let scope = scope.trim().to_string();
    if scope.is_empty() {
        return Err(AppError::Validation(
            "scope is required to list quality profiles".into(),
        ));
    }

    let normalized_scope_id = normalize_optional(scope_id);
    let rows = sqlx::query(
        "SELECT id, name, archival_quality, allow_unknown_quality,
                atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux,
                allow_bd_disk, allow_upgrades, prefer_dual_audio, required_audio_languages,
                scoring_config
           FROM quality_profiles
          WHERE scope = $1
            AND (($2::TEXT IS NULL AND scope_id IS NULL) OR scope_id = $2)
          ORDER BY name",
    )
    .bind(&scope)
    .bind(&normalized_scope_id)
    .fetch_all(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row
            .try_get("id")
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let archival_quality: Option<String> = row
            .try_get("archival_quality")
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let required_audio_languages: Vec<String> = row
            .try_get::<Value, _>("required_audio_languages")
            .ok()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let scoring_config: ScoringConfig = row
            .try_get::<Value, _>("scoring_config")
            .ok()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();

        out.push(QualityProfile {
            id: id.clone(),
            name: row
                .try_get("name")
                .map_err(|error| AppError::Repository(error.to_string()))?,
            criteria: QualityProfileCriteria {
                quality_tiers: list_quality_profile_values(
                    pool,
                    "quality_profile_quality_tiers",
                    "quality_tier",
                    "sort_order ASC",
                    &id,
                )
                .await?,
                archival_quality: archival_quality.and_then(|value| {
                    let value = value.trim().to_string();
                    if value.is_empty() { None } else { Some(value) }
                }),
                allow_unknown_quality: row
                    .try_get("allow_unknown_quality")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                source_allowlist: list_quality_profile_values(
                    pool,
                    "quality_profile_source_allowlist",
                    "source",
                    "source ASC",
                    &id,
                )
                .await?,
                source_blocklist: list_quality_profile_values(
                    pool,
                    "quality_profile_source_blocklist",
                    "source",
                    "source ASC",
                    &id,
                )
                .await?,
                video_codec_allowlist: list_quality_profile_values(
                    pool,
                    "quality_profile_video_codec_allowlist",
                    "codec",
                    "codec ASC",
                    &id,
                )
                .await?,
                video_codec_blocklist: list_quality_profile_values(
                    pool,
                    "quality_profile_video_codec_blocklist",
                    "codec",
                    "codec ASC",
                    &id,
                )
                .await?,
                audio_codec_allowlist: list_quality_profile_values(
                    pool,
                    "quality_profile_audio_codec_allowlist",
                    "codec",
                    "codec ASC",
                    &id,
                )
                .await?,
                audio_codec_blocklist: list_quality_profile_values(
                    pool,
                    "quality_profile_audio_codec_blocklist",
                    "codec",
                    "codec ASC",
                    &id,
                )
                .await?,
                atmos_preferred: row
                    .try_get("atmos_preferred")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                dolby_vision_allowed: row
                    .try_get("dolby_vision_allowed")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                detected_hdr_allowed: row
                    .try_get("detected_hdr_allowed")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                prefer_remux: row
                    .try_get("prefer_remux")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                allow_bd_disk: row
                    .try_get("allow_bd_disk")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                allow_upgrades: row
                    .try_get("allow_upgrades")
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                prefer_dual_audio: row.try_get("prefer_dual_audio").unwrap_or(false),
                required_audio_languages,
                scoring_persona: scoring_config.scoring_persona,
                scoring_overrides: scoring_config.scoring_overrides,
                cutoff_tier: scoring_config.cutoff_tier,
                min_score_to_grab: scoring_config.min_score_to_grab,
                facet_persona_overrides: scoring_config.facet_persona_overrides,
            },
        });
    }

    Ok(out)
}

async fn list_quality_profile_values(
    pool: &sqlx::PgPool,
    table: &str,
    column: &str,
    order_by: &str,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let sql =
        format!("SELECT {column} AS value FROM {table} WHERE profile_id = $1 ORDER BY {order_by}");
    let rows = sqlx::query(&sql)
        .bind(profile_id)
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    rows.into_iter()
        .map(|row| {
            row.try_get("value")
                .map_err(|error| AppError::Repository(error.to_string()))
        })
        .collect()
}

async fn replace_quality_profiles_relational(
    pool: &sqlx::PgPool,
    scope: &str,
    scope_id: Option<String>,
    profiles: Vec<QualityProfile>,
) -> AppResult<()> {
    let scope = scope.trim().to_string();
    if scope.is_empty() {
        return Err(AppError::Validation(
            "scope is required to replace quality profiles".into(),
        ));
    }

    let normalized_scope_id = normalize_optional(scope_id);
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    if normalized_scope_id.is_some() {
        sqlx::query("DELETE FROM quality_profiles WHERE scope = $1 AND scope_id = $2")
            .bind(&scope)
            .bind(&normalized_scope_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
    } else {
        sqlx::query("DELETE FROM quality_profiles WHERE scope = $1 AND scope_id IS NULL")
            .bind(&scope)
            .execute(&mut *tx)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
    }

    for profile in profiles {
        upsert_quality_profile_relational(&mut tx, &scope, normalized_scope_id.as_ref(), profile)
            .await?;
    }

    tx.commit()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))
}

async fn upsert_quality_profile_relational(
    tx: &mut Transaction<'_, Postgres>,
    scope: &str,
    scope_id: Option<&String>,
    profile: QualityProfile,
) -> AppResult<()> {
    let id = profile.id.trim().to_string();
    if id.is_empty() {
        return Ok(());
    }

    let name = profile.name.trim().to_string();
    let criteria = profile.criteria;
    let archival_quality = criteria
        .archival_quality
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let quality_tiers = normalize_profile_string_values(criteria.quality_tiers);
    let source_allowlist = normalize_profile_string_values(criteria.source_allowlist);
    let source_blocklist = normalize_profile_string_values(criteria.source_blocklist);
    let video_codec_allowlist = normalize_profile_string_values(criteria.video_codec_allowlist);
    let video_codec_blocklist = normalize_profile_string_values(criteria.video_codec_blocklist);
    let audio_codec_allowlist = normalize_profile_string_values(criteria.audio_codec_allowlist);
    let audio_codec_blocklist = normalize_profile_string_values(criteria.audio_codec_blocklist);
    let required_audio_languages = serde_json::to_value(&criteria.required_audio_languages)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let scoring_config = serde_json::to_value(ScoringConfig {
        scoring_persona: criteria.scoring_persona,
        scoring_overrides: criteria.scoring_overrides,
        cutoff_tier: criteria.cutoff_tier,
        min_score_to_grab: criteria.min_score_to_grab,
        facet_persona_overrides: criteria.facet_persona_overrides,
    })
    .map_err(|error| AppError::Repository(error.to_string()))?;

    clear_quality_profile_value_rows_pg(tx, &id).await?;

    sqlx::query(
        "INSERT INTO quality_profiles
            (id, name, scope, scope_id, archival_quality, allow_unknown_quality,
             atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux,
             allow_bd_disk, allow_upgrades, prefer_dual_audio, required_audio_languages,
             scoring_config, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14::jsonb, $15::jsonb, $16)
         ON CONFLICT(id) DO UPDATE SET
            name = EXCLUDED.name,
            scope = EXCLUDED.scope,
            scope_id = EXCLUDED.scope_id,
            archival_quality = EXCLUDED.archival_quality,
            allow_unknown_quality = EXCLUDED.allow_unknown_quality,
            atmos_preferred = EXCLUDED.atmos_preferred,
            dolby_vision_allowed = EXCLUDED.dolby_vision_allowed,
            detected_hdr_allowed = EXCLUDED.detected_hdr_allowed,
            prefer_remux = EXCLUDED.prefer_remux,
            allow_bd_disk = EXCLUDED.allow_bd_disk,
            allow_upgrades = EXCLUDED.allow_upgrades,
            prefer_dual_audio = EXCLUDED.prefer_dual_audio,
            required_audio_languages = EXCLUDED.required_audio_languages,
            scoring_config = EXCLUDED.scoring_config",
    )
    .bind(&id)
    .bind(name)
    .bind(scope)
    .bind(scope_id)
    .bind(archival_quality)
    .bind(criteria.allow_unknown_quality)
    .bind(criteria.atmos_preferred)
    .bind(criteria.dolby_vision_allowed)
    .bind(criteria.detected_hdr_allowed)
    .bind(criteria.prefer_remux)
    .bind(criteria.allow_bd_disk)
    .bind(criteria.allow_upgrades)
    .bind(criteria.prefer_dual_audio)
    .bind(required_audio_languages)
    .bind(scoring_config)
    .bind(Utc::now())
    .execute(&mut **tx)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;

    insert_quality_profile_quality_tiers(tx, &id, &quality_tiers).await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_source_allowlist",
        "source",
        &id,
        &source_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_source_blocklist",
        "source",
        &id,
        &source_blocklist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_video_codec_allowlist",
        "codec",
        &id,
        &video_codec_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_video_codec_blocklist",
        "codec",
        &id,
        &video_codec_blocklist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_audio_codec_allowlist",
        "codec",
        &id,
        &audio_codec_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_audio_codec_blocklist",
        "codec",
        &id,
        &audio_codec_blocklist,
    )
    .await?;

    Ok(())
}

async fn clear_quality_profile_value_rows_pg(
    tx: &mut Transaction<'_, Postgres>,
    profile_id: &str,
) -> AppResult<()> {
    for table in [
        "quality_profile_quality_tiers",
        "quality_profile_source_allowlist",
        "quality_profile_source_blocklist",
        "quality_profile_video_codec_allowlist",
        "quality_profile_video_codec_blocklist",
        "quality_profile_audio_codec_allowlist",
        "quality_profile_audio_codec_blocklist",
    ] {
        let sql = format!("DELETE FROM {table} WHERE profile_id = $1");
        sqlx::query(&sql)
            .bind(profile_id)
            .execute(&mut **tx)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
    }
    Ok(())
}

async fn insert_quality_profile_quality_tiers(
    tx: &mut Transaction<'_, Postgres>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for (index, value) in values.iter().enumerate() {
        sqlx::query(
            "INSERT INTO quality_profile_quality_tiers(profile_id, quality_tier, sort_order)
             VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(profile_id)
        .bind(value)
        .bind(index as i64)
        .execute(&mut **tx)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    }
    Ok(())
}

async fn insert_quality_profile_values(
    tx: &mut Transaction<'_, Postgres>,
    table: &str,
    column: &str,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    let sql =
        format!("INSERT INTO {table}(profile_id, {column}) VALUES ($1, $2) ON CONFLICT DO NOTHING");
    for value in values {
        sqlx::query(&sql)
            .bind(profile_id)
            .bind(value)
            .execute(&mut **tx)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
    }
    Ok(())
}

fn normalize_profile_string_values(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }
    normalized
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
