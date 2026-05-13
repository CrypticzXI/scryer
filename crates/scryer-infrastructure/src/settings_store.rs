use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use scryer_application::{
    AppResult, QualityProfile as ApplicationQualityProfile, QualityProfileRepository,
    SettingsRepository, SystemInfoProvider,
};

use crate::SqliteServices;
use crate::encryption::EncryptionKey;
use crate::types::{MigrationStatus, SettingDefinitionSeed, SettingsValueRecord};

#[async_trait]
pub trait SettingsSql: Clone + Send + Sync + 'static {
    fn engine(&self) -> &'static str;
    fn encryption_key(&self) -> Option<EncryptionKey>;

    async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<SettingDefinitionSeed>,
    ) -> AppResult<()>;

    async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<SettingsValueRecord>>>;

    async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()>;

    async fn list_settings_with_defaults(
        &self,
        scope: String,
        scope_id: Option<String>,
    ) -> AppResult<Vec<SettingsValueRecord>>;

    async fn get_setting_with_defaults(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>>;

    async fn get_setting_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>>;

    async fn upsert_setting_value(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
        value_json: String,
        source: String,
        updated_by_user_id: Option<String>,
    ) -> AppResult<SettingsValueRecord>;

    async fn delete_setting_value(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
    ) -> AppResult<()>;

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32>;
    async fn list_applied_migrations(&self) -> AppResult<Vec<MigrationStatus>>;

    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<ApplicationQualityProfile>>;

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<ApplicationQualityProfile>,
    ) -> AppResult<()>;

    async fn current_migration_version(&self) -> AppResult<Option<String>>;
}

#[derive(Clone)]
pub struct SettingsStore<S> {
    sql: S,
}

impl<S> SettingsStore<S> {
    pub(crate) fn from_sql(sql: S) -> Self {
        Self { sql }
    }
}

impl SettingsStore<SqliteSettingsSql> {
    pub fn new(db: &SqliteServices) -> Self {
        Self::from_sql(SqliteSettingsSql::new(db))
    }
}

impl<S: SettingsSql> SettingsStore<S> {
    pub async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<SettingDefinitionSeed>,
    ) -> AppResult<()> {
        self.sql.batch_ensure_setting_definitions(definitions).await
    }

    pub async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<SettingsValueRecord>>> {
        self.sql.batch_get_settings_with_defaults(keys).await
    }

    pub async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        self.sql
            .batch_upsert_settings_if_not_overridden(entries)
            .await
    }

    pub async fn list_settings_with_defaults(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Vec<SettingsValueRecord>> {
        self.sql
            .list_settings_with_defaults(scope.into(), scope_id)
            .await
    }

    pub async fn get_setting_with_defaults(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>> {
        self.sql
            .get_setting_with_defaults(scope.into(), key_name.into(), scope_id)
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
        self.sql
            .upsert_setting_value(
                scope.into(),
                key_name.into(),
                scope_id,
                value_json.into(),
                source.into(),
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
        self.sql
            .delete_setting_value(scope.into(), key_name.into(), scope_id)
            .await
    }

    pub async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        self.sql.delete_values_for_scope_id(scope_id).await
    }

    pub async fn list_applied_migrations(&self) -> AppResult<Vec<MigrationStatus>> {
        self.sql.list_applied_migrations().await
    }
}

pub type SqliteSettingsStore = SettingsStore<SqliteSettingsSql>;

#[derive(Clone)]
pub struct SqliteSettingsSql {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SqliteSettingsSql {
    fn new(db: &SqliteServices) -> Self {
        Self {
            db: db.clone(),
            pool: db.pool().clone(),
            encryption_key: db.encryption_key_state(),
        }
    }
}

#[async_trait]
impl SettingsSql for SqliteSettingsSql {
    fn engine(&self) -> &'static str {
        "sqlite"
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
        self.db.batch_ensure_setting_definitions(definitions).await
    }

    async fn batch_get_settings_with_defaults(
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

    async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        self.db
            .batch_upsert_settings_if_not_overridden(entries)
            .await
    }

    async fn list_settings_with_defaults(
        &self,
        scope: String,
        scope_id: Option<String>,
    ) -> AppResult<Vec<SettingsValueRecord>> {
        let encryption_key = self.encryption_key();
        crate::queries::settings::list_settings_with_defaults_query(
            &self.pool,
            &scope,
            scope_id,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_setting_with_defaults(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>> {
        let encryption_key = self.encryption_key();
        crate::queries::settings::get_setting_with_defaults_query(
            &self.pool,
            &scope,
            &key_name,
            scope_id,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_setting_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>> {
        let encryption_key = self.encryption_key();
        crate::queries::settings::get_setting_explicit_query(
            &self.pool,
            scope,
            key_name,
            scope_id,
            encryption_key.as_ref(),
        )
        .await
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

    async fn delete_setting_value(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.db
            .delete_setting_value(scope, key_name, scope_id)
            .await
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        crate::queries::settings::delete_settings_values_for_scope_id_query(&self.pool, scope_id)
            .await
    }

    async fn list_applied_migrations(&self) -> AppResult<Vec<MigrationStatus>> {
        crate::migrations::list_applied_migrations(&self.pool).await
    }

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
}

#[async_trait]
impl<S: SettingsSql> SettingsRepository for SettingsStore<S> {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .sql
            .get_setting_with_defaults(scope.to_string(), key_name.to_string(), scope_id)
            .await?
            .map(|record| record.effective_value_json))
    }

    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .sql
            .get_setting_explicit(scope, key_name, scope_id)
            .await?
            .map(|record| record.effective_value_json))
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
        self.sql
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
        SettingsStore::delete_setting_value(self, scope.to_string(), key_name.to_string(), scope_id)
            .await
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        SettingsStore::delete_values_for_scope_id(self, scope_id).await
    }
}

#[async_trait]
impl<S: SettingsSql> QualityProfileRepository for SettingsStore<S> {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<ApplicationQualityProfile>> {
        self.sql.list_quality_profiles(scope, scope_id).await
    }

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<ApplicationQualityProfile>,
    ) -> AppResult<()> {
        self.sql
            .replace_quality_profiles(scope, scope_id, profiles)
            .await
    }
}

#[async_trait]
impl<S: SettingsSql> SystemInfoProvider for SettingsStore<S> {
    async fn datastore_info(&self) -> AppResult<scryer_application::DatastoreInfo> {
        Ok(scryer_application::DatastoreInfo {
            engine: self.sql.engine().to_string(),
            current_migration_key: self.current_migration_version().await?,
        })
    }

    async fn current_migration_version(&self) -> AppResult<Option<String>> {
        self.sql.current_migration_version().await
    }

    async fn current_encryption_key_base64(&self) -> AppResult<Option<String>> {
        Ok(self.sql.encryption_key().map(|key| key.to_base64()))
    }
}
