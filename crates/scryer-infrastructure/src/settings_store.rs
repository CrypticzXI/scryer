use async_trait::async_trait;
use scryer_application::{
    AppResult, QualityProfile as ApplicationQualityProfile, QualityProfileRepository,
    SettingsRepository, SystemInfoProvider,
};
use std::sync::{Arc, RwLock};

use crate::SqliteServices;
use crate::encryption::EncryptionKey;
use crate::types::{MigrationStatus, SettingDefinitionSeed, SettingsValueRecord};

#[derive(Clone)]
pub struct SqliteSettingsStore {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SqliteSettingsStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self {
            db: db.clone(),
            pool: db.pool().clone(),
            encryption_key: db.encryption_key_state(),
        }
    }

    fn encryption_key(&self) -> Option<EncryptionKey> {
        self.encryption_key
            .read()
            .ok()
            .and_then(|value| value.clone())
    }

    pub async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<SettingDefinitionSeed>,
    ) -> AppResult<()> {
        self.db.batch_ensure_setting_definitions(definitions).await
    }

    pub async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<SettingsValueRecord>>> {
        let encryption_key = self.encryption_key();
        crate::queries::settings::batch_get_settings_with_defaults_query(
            &self.pool,
            &keys,
            encryption_key.as_ref(),
        )
        .await
    }

    pub async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        self.db
            .batch_upsert_settings_if_not_overridden(entries)
            .await
    }

    pub async fn list_settings_with_defaults(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Vec<SettingsValueRecord>> {
        let encryption_key = self.encryption_key();
        let scope = scope.into();
        crate::queries::settings::list_settings_with_defaults_query(
            &self.pool,
            &scope,
            scope_id,
            encryption_key.as_ref(),
        )
        .await
    }

    pub async fn get_setting_with_defaults(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>> {
        let encryption_key = self.encryption_key();
        let scope = scope.into();
        let key_name = key_name.into();
        crate::queries::settings::get_setting_with_defaults_query(
            &self.pool,
            &scope,
            &key_name,
            scope_id,
            encryption_key.as_ref(),
        )
        .await
    }

    pub async fn upsert_setting_value(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
        value_json: impl Into<String>,
        source: impl Into<String>,
        updated_by_user_id: Option<String>,
    ) -> AppResult<SettingsValueRecord> {
        self.db
            .upsert_setting_value(
                scope,
                key_name,
                scope_id,
                value_json,
                source,
                updated_by_user_id,
            )
            .await
    }

    pub async fn delete_setting_value(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.db
            .delete_setting_value(scope, key_name, scope_id)
            .await
    }

    pub async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        crate::queries::settings::delete_settings_values_for_scope_id_query(&self.pool, scope_id)
            .await
    }

    pub async fn list_applied_migrations(&self) -> AppResult<Vec<MigrationStatus>> {
        crate::migrations::list_applied_migrations(&self.pool).await
    }
}

#[async_trait]
impl SettingsRepository for SqliteSettingsStore {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        let encryption_key = self.encryption_key();
        match crate::queries::settings::get_setting_with_defaults_query(
            &self.pool,
            scope,
            key_name,
            scope_id,
            encryption_key.as_ref(),
        )
        .await?
        {
            Some(record) => Ok(Some(record.effective_value_json)),
            None => Ok(None),
        }
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        self.db
            .upsert_setting_value(
                scope.to_string(),
                key_name.to_string(),
                scope_id,
                value_json,
                source.to_string(),
                updated_by_user_id,
            )
            .await?;
        Ok(())
    }

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.db
            .delete_setting_value(scope.to_string(), key_name.to_string(), scope_id)
            .await
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        SqliteSettingsStore::delete_values_for_scope_id(self, scope_id).await
    }
}

#[async_trait]
impl QualityProfileRepository for SqliteSettingsStore {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<ApplicationQualityProfile>> {
        crate::queries::quality::list_quality_profiles_query(&self.pool, scope, scope_id).await
    }

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<ApplicationQualityProfile>,
    ) -> AppResult<()> {
        self.db
            .replace_quality_profiles(scope, scope_id, profiles)
            .await
    }
}

#[async_trait]
impl SystemInfoProvider for SqliteSettingsStore {
    async fn current_migration_version(&self) -> AppResult<Option<String>> {
        let latest = sqlx::query_as::<_, (i64, String)>(
            "SELECT version, description
               FROM _sqlx_migrations
              WHERE success = 1
              ORDER BY version DESC, description DESC
              LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| scryer_application::AppError::Repository(error.to_string()))?;

        Ok(latest.map(|(version, description)| {
            crate::migration_assets::migration_key_from_version_and_desc(version, &description)
        }))
    }

    async fn vacuum_into(&self, dest_path: &str) -> AppResult<()> {
        self.db.vacuum_into(dest_path).await
    }
}
