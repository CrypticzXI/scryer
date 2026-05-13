use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, AppServices, AppServicesBuilder, DownloadClient,
    DownloadClientConfigRepository, IndexerClient, IndexerConfigRepository, IndexerStatsTracker,
    LibraryRepository, LogicalBackupExporter, NullTitleImageProcessor,
    PluginInstallationRepository, PostProcessingScriptRepository, QualityProfile,
    QualityProfileRepository, RuleSetRepository, SettingsRepository, ShowRepository,
    SubtitleProviderConfigRepository, SystemInfoProvider, TitleImageRepository, TitleRepository,
    UserRepository,
};

use crate::postgres::{
    PostgresCatalogStore, PostgresConfigStore, PostgresCustomizationStore,
    PostgresLibraryStateStore, PostgresLogicalBackupExporter, PostgresNotificationStore,
    PostgresReleaseStore, PostgresServices, PostgresSettingsStore, PostgresWorkflowStore,
    restore_backup_bundle_into_postgres_pool,
};
use crate::{
    FileSystemStagedNzbStore, InMemoryIndexerStatsTracker, MetadataGatewayClient, MigrationMode,
    SmgEnrollmentConfig, SqliteCatalogStore, SqliteConfigStore, SqliteCustomizationStore,
    SqliteLibraryStateStore, SqliteLogicalBackupExporter, SqliteNotificationStore,
    SqliteReleaseStore, SqliteServices, SqliteSettingsStore, SqliteTitleImageProcessor,
    SqliteWorkflowStore,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatastoreEngine {
    Sqlite,
    Postgres,
}

impl DatastoreEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatastoreConfigSource {
    EnvDbUrl,
    EnvDbPath,
    DefaultSqlite,
}

impl DatastoreConfigSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EnvDbUrl => "SCRYER_DB_URL",
            Self::EnvDbPath => "SCRYER_DB_PATH",
            Self::DefaultSqlite => "default_sqlite",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DatastoreConfig {
    pub engine: DatastoreEngine,
    pub database_url: String,
    pub redacted_database_url: String,
    pub source: DatastoreConfigSource,
    pub data_dir: PathBuf,
    pub migration_mode: MigrationMode,
}

impl DatastoreConfig {
    pub fn sqlite(
        database_url: impl Into<String>,
        data_dir: impl Into<PathBuf>,
        migration_mode: MigrationMode,
    ) -> Self {
        Self::sqlite_with_source(
            database_url,
            DatastoreConfigSource::EnvDbPath,
            data_dir,
            migration_mode,
        )
    }

    pub fn sqlite_with_source(
        database_url: impl Into<String>,
        source: DatastoreConfigSource,
        data_dir: impl Into<PathBuf>,
        migration_mode: MigrationMode,
    ) -> Self {
        let database_url = database_url.into();
        Self {
            engine: DatastoreEngine::Sqlite,
            redacted_database_url: database_url.clone(),
            database_url,
            source,
            data_dir: data_dir.into(),
            migration_mode,
        }
    }

    pub fn postgres(
        database_url: impl Into<String>,
        redacted_database_url: impl Into<String>,
        source: DatastoreConfigSource,
        data_dir: impl Into<PathBuf>,
        migration_mode: MigrationMode,
    ) -> Self {
        Self {
            engine: DatastoreEngine::Postgres,
            database_url: database_url.into(),
            redacted_database_url: redacted_database_url.into(),
            source,
            data_dir: data_dir.into(),
            migration_mode,
        }
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn safe_database_url(&self) -> &str {
        if self.redacted_database_url.is_empty() {
            &self.database_url
        } else {
            &self.redacted_database_url
        }
    }
}

pub fn resolve_datastore_config_from_env(
    data_dir: impl Into<PathBuf>,
    migration_mode: MigrationMode,
) -> AppResult<DatastoreConfig> {
    let data_dir = data_dir.into();
    if let Some(raw_url) = env_string("SCRYER_DB_URL") {
        return datastore_config_from_url(
            raw_url,
            DatastoreConfigSource::EnvDbUrl,
            data_dir,
            migration_mode,
        );
    }

    if let Some(db_path) = env_string("SCRYER_DB_PATH") {
        return Ok(DatastoreConfig::sqlite_with_source(
            db_path,
            DatastoreConfigSource::EnvDbPath,
            data_dir,
            migration_mode,
        ));
    }

    Ok(DatastoreConfig::sqlite_with_source(
        format!("sqlite://{}", data_dir.join("scryer.db").display()),
        DatastoreConfigSource::DefaultSqlite,
        data_dir,
        migration_mode,
    ))
}

fn datastore_config_from_url(
    raw_url: String,
    source: DatastoreConfigSource,
    data_dir: PathBuf,
    migration_mode: MigrationMode,
) -> AppResult<DatastoreConfig> {
    let parsed = url::Url::parse(&raw_url)
        .map_err(|error| AppError::Validation(format!("invalid SCRYER_DB_URL: {error}")))?;
    match parsed.scheme() {
        "sqlite" => Ok(DatastoreConfig::sqlite_with_source(
            raw_url,
            source,
            data_dir,
            migration_mode,
        )),
        "postgres" | "postgresql" => {
            let (database_url, redacted_url) = resolve_postgres_url(parsed)?;
            Ok(DatastoreConfig::postgres(
                database_url,
                redacted_url,
                source,
                data_dir,
                migration_mode,
            ))
        }
        scheme => Err(AppError::Validation(format!(
            "unsupported datastore URL scheme '{scheme}'; expected sqlite, postgres, or postgresql"
        ))),
    }
}

fn resolve_postgres_url(mut url: url::Url) -> AppResult<(String, String)> {
    if url.host_str().is_none_or(|host| host.trim().is_empty()) {
        return Err(AppError::Validation(
            "PostgreSQL datastore URL must include a host".to_string(),
        ));
    }

    let database_name = url.path().trim_start_matches('/').trim();
    if database_name.is_empty() {
        return Err(AppError::Validation(
            "PostgreSQL datastore URL must include a database name".to_string(),
        ));
    }

    let sslmode = url
        .query_pairs()
        .find(|(key, _)| key == "sslmode")
        .map(|(_, value)| value.to_string());
    let Some(sslmode) = sslmode else {
        return Err(AppError::Validation(
            "PostgreSQL datastore URL must include an explicit sslmode".to_string(),
        ));
    };
    if !matches!(
        sslmode.as_str(),
        "disable" | "prefer" | "require" | "verify-ca" | "verify-full"
    ) {
        return Err(AppError::Validation(format!(
            "unsupported PostgreSQL sslmode '{sslmode}'; expected disable, prefer, require, verify-ca, or verify-full"
        )));
    }

    let username = env_string("SCRYER_DB_USER")
        .or_else(|| {
            let username = url.username().trim();
            if username.is_empty() {
                None
            } else {
                Some(username.to_string())
            }
        })
        .ok_or_else(|| {
            AppError::Validation(
                "PostgreSQL datastore requires SCRYER_DB_USER or a URL username".to_string(),
            )
        })?;

    let password = postgres_password(&url)?;

    url.set_username(&username).map_err(|_| {
        AppError::Validation("failed to set PostgreSQL username on datastore URL".to_string())
    })?;
    url.set_password(Some(&password)).map_err(|_| {
        AppError::Validation("failed to set PostgreSQL password on datastore URL".to_string())
    })?;

    let mut redacted = url.clone();
    let _ = redacted.set_username("<redacted>");
    let _ = redacted.set_password(Some("<redacted>"));

    Ok((url.to_string(), redacted.to_string()))
}

fn postgres_password(url: &url::Url) -> AppResult<String> {
    if let Some(password_file) = env_string("SCRYER_DB_PASSWORD_FILE") {
        let password = std::fs::read_to_string(&password_file).map_err(|error| {
            AppError::Validation(format!(
                "failed to read SCRYER_DB_PASSWORD_FILE {}: {error}",
                password_file
            ))
        })?;
        let password = password.trim_end().to_string();
        if password.is_empty() {
            return Err(AppError::Validation(
                "SCRYER_DB_PASSWORD_FILE did not contain a password".to_string(),
            ));
        }
        return Ok(password);
    }

    if let Some(password) = env_string_raw("SCRYER_DB_PASSWORD") {
        return Ok(password);
    }

    url.password()
        .map(str::to_string)
        .filter(|password| !password.trim().is_empty())
        .ok_or_else(|| {
            AppError::Validation(
                "PostgreSQL datastore requires SCRYER_DB_PASSWORD, SCRYER_DB_PASSWORD_FILE, or a URL password"
                    .to_string(),
            )
        })
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_string_raw(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[async_trait]
trait DatastoreSettingsStoreInner:
    SettingsRepository + QualityProfileRepository + SystemInfoProvider + Send + Sync
{
    async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<crate::SettingDefinitionSeed>,
    ) -> AppResult<()>;

    async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<crate::SettingsValueRecord>>>;

    async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()>;

    async fn get_setting_with_defaults(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
    ) -> AppResult<Option<crate::SettingsValueRecord>>;

    async fn upsert_setting_value(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
        value_json: String,
        source: String,
        updated_by_user_id: Option<String>,
    ) -> AppResult<crate::SettingsValueRecord>;

    async fn list_settings_with_defaults(
        &self,
        scope: String,
        scope_id: Option<String>,
    ) -> AppResult<Vec<crate::SettingsValueRecord>>;

    async fn list_applied_migrations(&self) -> AppResult<Vec<crate::MigrationStatus>>;
}

#[async_trait]
impl<S> DatastoreSettingsStoreInner for crate::settings_store::SettingsStore<S>
where
    S: crate::settings_store::SettingsSql,
{
    async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<crate::SettingDefinitionSeed>,
    ) -> AppResult<()> {
        crate::settings_store::SettingsStore::batch_ensure_setting_definitions(self, definitions)
            .await
    }

    async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<crate::SettingsValueRecord>>> {
        crate::settings_store::SettingsStore::batch_get_settings_with_defaults(self, keys).await
    }

    async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        crate::settings_store::SettingsStore::batch_upsert_settings_if_not_overridden(self, entries)
            .await
    }

    async fn get_setting_with_defaults(
        &self,
        scope: String,
        key_name: String,
        scope_id: Option<String>,
    ) -> AppResult<Option<crate::SettingsValueRecord>> {
        crate::settings_store::SettingsStore::get_setting_with_defaults(
            self, scope, key_name, scope_id,
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
    ) -> AppResult<crate::SettingsValueRecord> {
        crate::settings_store::SettingsStore::upsert_setting_value(
            self,
            scope,
            key_name,
            scope_id,
            value_json,
            source,
            updated_by_user_id,
        )
        .await
    }

    async fn list_settings_with_defaults(
        &self,
        scope: String,
        scope_id: Option<String>,
    ) -> AppResult<Vec<crate::SettingsValueRecord>> {
        crate::settings_store::SettingsStore::list_settings_with_defaults(self, scope, scope_id)
            .await
    }

    async fn list_applied_migrations(&self) -> AppResult<Vec<crate::MigrationStatus>> {
        crate::settings_store::SettingsStore::list_applied_migrations(self).await
    }
}

#[derive(Clone)]
pub struct DatastoreSettingsStore {
    inner: Arc<dyn DatastoreSettingsStoreInner>,
}

impl DatastoreSettingsStore {
    fn from_inner<T>(inner: T) -> Self
    where
        T: DatastoreSettingsStoreInner + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn from_sqlite(inner: SqliteSettingsStore) -> Self {
        Self::from_inner(inner)
    }

    pub fn from_postgres(inner: PostgresSettingsStore) -> Self {
        Self::from_inner(inner)
    }

    pub async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<crate::SettingDefinitionSeed>,
    ) -> AppResult<()> {
        self.inner
            .batch_ensure_setting_definitions(definitions)
            .await
    }

    pub async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<crate::SettingsValueRecord>>> {
        self.inner.batch_get_settings_with_defaults(keys).await
    }

    pub async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        self.inner
            .batch_upsert_settings_if_not_overridden(entries)
            .await
    }

    pub async fn get_setting_with_defaults(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Option<crate::SettingsValueRecord>> {
        self.inner
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
    ) -> AppResult<crate::SettingsValueRecord> {
        self.inner
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

    pub async fn list_settings_with_defaults(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Vec<crate::SettingsValueRecord>> {
        self.inner
            .list_settings_with_defaults(scope.into(), scope_id)
            .await
    }

    pub async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .delete_setting_value(scope, key_name, scope_id)
            .await
    }

    pub async fn list_applied_migrations(&self) -> AppResult<Vec<crate::MigrationStatus>> {
        self.inner.list_applied_migrations().await
    }
}

#[async_trait]
impl SettingsRepository for DatastoreSettingsStore {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.inner.get_setting_json(scope, key_name, scope_id).await
    }

    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.inner
            .get_setting_json_explicit(scope, key_name, scope_id)
            .await
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
        self.inner
            .upsert_setting_json(
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
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .delete_setting_value(scope, key_name, scope_id)
            .await
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        self.inner.delete_values_for_scope_id(scope_id).await
    }
}

#[async_trait]
impl QualityProfileRepository for DatastoreSettingsStore {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        self.inner.list_quality_profiles(scope, scope_id).await
    }

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        self.inner
            .replace_quality_profiles(scope, scope_id, profiles)
            .await
    }
}

#[async_trait]
impl SystemInfoProvider for DatastoreSettingsStore {
    async fn datastore_info(&self) -> AppResult<scryer_application::DatastoreInfo> {
        self.inner.datastore_info().await
    }

    async fn current_migration_version(&self) -> AppResult<Option<String>> {
        self.inner.current_migration_version().await
    }

    async fn current_encryption_key_base64(&self) -> AppResult<Option<String>> {
        self.inner.current_encryption_key_base64().await
    }
}

#[async_trait]
trait DatastoreCustomizationStoreInner:
    RuleSetRepository + PostProcessingScriptRepository + PluginInstallationRepository + Send + Sync
{
    async fn delete_incompatible_external_plugin_installations(&self) -> AppResult<Vec<String>>;
}

#[async_trait]
impl<S> DatastoreCustomizationStoreInner for crate::customization_store::CustomizationStore<S>
where
    S: crate::customization_store::CustomizationSql,
{
    async fn delete_incompatible_external_plugin_installations(&self) -> AppResult<Vec<String>> {
        crate::customization_store::CustomizationStore::delete_incompatible_external_plugin_installations(self)
            .await
    }
}

#[derive(Clone)]
pub struct DatastoreCustomizationStore {
    inner: Arc<dyn DatastoreCustomizationStoreInner>,
}

impl DatastoreCustomizationStore {
    fn from_inner<T>(inner: T) -> Self
    where
        T: DatastoreCustomizationStoreInner + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn from_sqlite(inner: SqliteCustomizationStore) -> Self {
        Self::from_inner(inner)
    }

    pub fn from_postgres(inner: PostgresCustomizationStore) -> Self {
        Self::from_inner(inner)
    }

    pub async fn delete_incompatible_external_plugin_installations(
        &self,
    ) -> AppResult<Vec<String>> {
        self.inner
            .delete_incompatible_external_plugin_installations()
            .await
    }
}

#[async_trait]
impl RuleSetRepository for DatastoreCustomizationStore {
    async fn list_rule_sets(&self) -> AppResult<Vec<scryer_domain::RuleSet>> {
        self.inner.list_rule_sets().await
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<scryer_domain::RuleSet>> {
        self.inner.list_enabled_rule_sets().await
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<scryer_domain::RuleSet>> {
        self.inner.get_rule_set(id).await
    }

    async fn create_rule_set(&self, rule_set: &scryer_domain::RuleSet) -> AppResult<()> {
        self.inner.create_rule_set(rule_set).await
    }

    async fn update_rule_set(&self, rule_set: &scryer_domain::RuleSet) -> AppResult<()> {
        self.inner.update_rule_set(rule_set).await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        self.inner.delete_rule_set(id).await
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        self.inner
            .record_rule_set_history(rule_set_id, action, rego_source, actor_id)
            .await
    }

    async fn get_rule_set_by_managed_key(
        &self,
        key: &str,
    ) -> AppResult<Option<scryer_domain::RuleSet>> {
        self.inner.get_rule_set_by_managed_key(key).await
    }

    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        self.inner.delete_rule_set_by_managed_key(key).await
    }

    async fn list_rule_sets_by_managed_key_prefix(
        &self,
        prefix: &str,
    ) -> AppResult<Vec<scryer_domain::RuleSet>> {
        self.inner
            .list_rule_sets_by_managed_key_prefix(prefix)
            .await
    }
}

#[async_trait]
impl PostProcessingScriptRepository for DatastoreCustomizationStore {
    async fn list_scripts(&self) -> AppResult<Vec<scryer_domain::PostProcessingScript>> {
        self.inner.list_scripts().await
    }

    async fn get_script(&self, id: &str) -> AppResult<Option<scryer_domain::PostProcessingScript>> {
        self.inner.get_script(id).await
    }

    async fn create_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript> {
        self.inner.create_script(script).await
    }

    async fn update_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript> {
        self.inner.update_script(script).await
    }

    async fn delete_script(&self, id: &str) -> AppResult<()> {
        self.inner.delete_script(id).await
    }

    async fn list_enabled_for_facet(
        &self,
        facet: &str,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScript>> {
        self.inner.list_enabled_for_facet(facet).await
    }

    async fn record_run(&self, run: scryer_domain::PostProcessingScriptRun) -> AppResult<()> {
        self.inner.record_run(run).await
    }

    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>> {
        self.inner.list_runs_for_script(script_id, limit).await
    }

    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>> {
        self.inner.list_runs_for_title(title_id, limit).await
    }
}

#[async_trait]
impl PluginInstallationRepository for DatastoreCustomizationStore {
    async fn list_plugin_installations(&self) -> AppResult<Vec<scryer_domain::PluginInstallation>> {
        self.inner.list_plugin_installations().await
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<scryer_domain::PluginInstallation>> {
        self.inner.get_plugin_installation(plugin_id).await
    }

    async fn create_plugin_installation(
        &self,
        installation: &scryer_domain::PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<scryer_domain::PluginInstallation> {
        self.inner
            .create_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn update_plugin_installation(
        &self,
        installation: &scryer_domain::PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<scryer_domain::PluginInstallation> {
        self.inner
            .update_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        self.inner.delete_plugin_installation(plugin_id).await
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<
        Vec<(
            scryer_domain::PluginInstallation,
            Option<scryer_domain::PersistedPluginWasmPayload>,
        )>,
    > {
        self.inner.get_enabled_plugin_wasm_bytes().await
    }

    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<scryer_domain::PersistedPluginWasmPayload>> {
        self.inner
            .get_plugin_installation_wasm_payload(plugin_id)
            .await
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        sdk_version: &str,
        sdk_constraint: &str,
        plugin_type: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        self.inner
            .seed_builtin(
                plugin_id,
                name,
                description,
                version,
                sdk_version,
                sdk_constraint,
                plugin_type,
                provider_type,
            )
            .await
    }

    async fn upsert_plugin_catalog_source(
        &self,
        source: &scryer_domain::PluginCatalogSource,
    ) -> AppResult<()> {
        self.inner.upsert_plugin_catalog_source(source).await
    }

    async fn list_plugin_catalog_sources(
        &self,
    ) -> AppResult<Vec<scryer_domain::PluginCatalogSource>> {
        self.inner.list_plugin_catalog_sources().await
    }

    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<scryer_domain::PluginCatalogSource>> {
        self.inner.get_plugin_catalog_source(source_key).await
    }

    async fn upsert_plugin_catalog_status(
        &self,
        status: &scryer_domain::PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        self.inner.upsert_plugin_catalog_status(status).await
    }

    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<scryer_domain::PluginCatalogStatusRecord>> {
        self.inner.get_plugin_catalog_status(status_key).await
    }
}

#[derive(Clone)]
pub struct DatastoreAssembly {
    config: DatastoreConfig,
    stores: DatastoreStores,
}

#[derive(Clone)]
enum DatastoreStores {
    Sqlite {
        db: SqliteServices,
        catalog_store: Arc<SqliteCatalogStore>,
        config_store: Arc<SqliteConfigStore>,
        customization_store: Arc<SqliteCustomizationStore>,
        library_state_store: Arc<SqliteLibraryStateStore>,
        notification_store: Arc<SqliteNotificationStore>,
        release_store: Arc<SqliteReleaseStore>,
        settings_store: Arc<SqliteSettingsStore>,
        workflow_store: Arc<SqliteWorkflowStore>,
        backup_exporter: Arc<SqliteLogicalBackupExporter>,
    },
    Postgres {
        db: PostgresServices,
        catalog_store: Arc<PostgresCatalogStore>,
        config_store: Arc<PostgresConfigStore>,
        customization_store: Arc<PostgresCustomizationStore>,
        library_state_store: Arc<PostgresLibraryStateStore>,
        notification_store: Arc<PostgresNotificationStore>,
        release_store: Arc<PostgresReleaseStore>,
        settings_store: Arc<PostgresSettingsStore>,
        workflow_store: Arc<PostgresWorkflowStore>,
        backup_exporter: Arc<PostgresLogicalBackupExporter>,
    },
}

impl DatastoreAssembly {
    pub async fn connect(config: DatastoreConfig) -> Result<Self, AppError> {
        match config.engine {
            DatastoreEngine::Sqlite => Self::connect_sqlite(config).await,
            DatastoreEngine::Postgres => Self::connect_postgres(config).await,
        }
    }

    async fn connect_sqlite(config: DatastoreConfig) -> Result<Self, AppError> {
        let db = SqliteServices::new_with_mode(config.database_url.clone(), config.migration_mode)
            .await?;
        let catalog_store = Arc::new(SqliteCatalogStore::new(&db));
        let config_store = Arc::new(SqliteConfigStore::new(&db));
        let customization_store = Arc::new(SqliteCustomizationStore::new(&db));
        let library_state_store = Arc::new(SqliteLibraryStateStore::new(&db));
        let notification_store = Arc::new(SqliteNotificationStore::new(&db));
        let release_store = Arc::new(SqliteReleaseStore::new(&db));
        let settings_store = Arc::new(SqliteSettingsStore::new(&db));
        let workflow_store = Arc::new(SqliteWorkflowStore::new(&db));
        let backup_exporter = Arc::new(SqliteLogicalBackupExporter::new(
            config.database_url.clone(),
        ));

        let stores = DatastoreStores::Sqlite {
            db,
            catalog_store,
            config_store,
            customization_store,
            library_state_store,
            notification_store,
            release_store,
            settings_store,
            workflow_store,
            backup_exporter,
        };

        Ok(Self { config, stores })
    }

    async fn connect_postgres(config: DatastoreConfig) -> Result<Self, AppError> {
        let db =
            PostgresServices::new_with_mode(config.database_url.clone(), config.migration_mode)
                .await?;
        let catalog_store = Arc::new(PostgresCatalogStore::new(&db));
        let config_store = Arc::new(PostgresConfigStore::new(&db));
        let customization_store = Arc::new(PostgresCustomizationStore::new(&db));
        let library_state_store = Arc::new(PostgresLibraryStateStore::new(&db));
        let notification_store = Arc::new(PostgresNotificationStore::new(&db));
        let release_store = Arc::new(PostgresReleaseStore::new(&db));
        let settings_store = Arc::new(PostgresSettingsStore::new(&db));
        let workflow_store = Arc::new(PostgresWorkflowStore::new(&db));
        let backup_exporter = Arc::new(PostgresLogicalBackupExporter::new(&db));

        let stores = DatastoreStores::Postgres {
            db,
            catalog_store,
            config_store,
            customization_store,
            library_state_store,
            notification_store,
            release_store,
            settings_store,
            workflow_store,
            backup_exporter,
        };

        Ok(Self { config, stores })
    }

    pub fn engine(&self) -> DatastoreEngine {
        self.config.engine
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.config.backup_dir()
    }

    pub fn staged_nzb_path(&self) -> PathBuf {
        match self.config.engine {
            DatastoreEngine::Sqlite => {
                FileSystemStagedNzbStore::path_for_main_db(&self.config.database_url)
            }
            DatastoreEngine::Postgres => self.config.data_dir.join("staged-nzbs"),
        }
    }

    pub fn bootstrap_settings_store(&self) -> DatastoreSettingsStore {
        match &self.stores {
            DatastoreStores::Sqlite { settings_store, .. } => {
                DatastoreSettingsStore::from_sqlite((**settings_store).clone())
            }
            DatastoreStores::Postgres { settings_store, .. } => {
                DatastoreSettingsStore::from_postgres((**settings_store).clone())
            }
        }
    }

    pub fn customization_store(&self) -> DatastoreCustomizationStore {
        match &self.stores {
            DatastoreStores::Sqlite {
                customization_store,
                ..
            } => DatastoreCustomizationStore::from_sqlite((**customization_store).clone()),
            DatastoreStores::Postgres {
                customization_store,
                ..
            } => DatastoreCustomizationStore::from_postgres((**customization_store).clone()),
        }
    }

    pub async fn bootstrap_encryption(&self) -> Result<u64, String> {
        match &self.stores {
            DatastoreStores::Sqlite { db, .. } => {
                let encryption_key = crate::encryption::ensure_encryption_key(
                    db,
                    Some(self.config.data_dir.clone()),
                )
                .await?;
                db.set_encryption_key(encryption_key)
                    .await
                    .map_err(|error| error.to_string())?;
                db.migrate_legacy_indexer_config_sources()
                    .await
                    .map_err(|error| error.to_string())
            }
            DatastoreStores::Postgres { db, .. } => {
                let encryption_key = crate::encryption::ensure_encryption_key_without_legacy(Some(
                    self.config.data_dir.clone(),
                ))
                .await?;
                db.set_encryption_key(encryption_key)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(0)
            }
        }
    }

    pub fn indexer_configs(&self) -> Arc<dyn IndexerConfigRepository> {
        match &self.stores {
            DatastoreStores::Sqlite { config_store, .. } => config_store.clone(),
            DatastoreStores::Postgres { config_store, .. } => config_store.clone(),
        }
    }

    pub fn download_client_configs(&self) -> Arc<dyn DownloadClientConfigRepository> {
        match &self.stores {
            DatastoreStores::Sqlite { config_store, .. } => config_store.clone(),
            DatastoreStores::Postgres { config_store, .. } => config_store.clone(),
        }
    }

    pub fn subtitle_provider_configs(&self) -> Arc<dyn SubtitleProviderConfigRepository> {
        match &self.stores {
            DatastoreStores::Sqlite { config_store, .. } => config_store.clone(),
            DatastoreStores::Postgres { config_store, .. } => config_store.clone(),
        }
    }

    pub fn settings(&self) -> Arc<dyn SettingsRepository> {
        match &self.stores {
            DatastoreStores::Sqlite { settings_store, .. } => settings_store.clone(),
            DatastoreStores::Postgres { settings_store, .. } => settings_store.clone(),
        }
    }

    pub fn quality_profiles(&self) -> Arc<dyn QualityProfileRepository> {
        match &self.stores {
            DatastoreStores::Sqlite { settings_store, .. } => settings_store.clone(),
            DatastoreStores::Postgres { settings_store, .. } => settings_store.clone(),
        }
    }

    pub fn title_images(&self) -> Arc<dyn TitleImageRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                library_state_store,
                ..
            } => library_state_store.clone(),
            DatastoreStores::Postgres {
                library_state_store,
                ..
            } => library_state_store.clone(),
        }
    }

    pub fn logical_backup_exporter(&self) -> Arc<dyn LogicalBackupExporter> {
        match &self.stores {
            DatastoreStores::Sqlite {
                backup_exporter, ..
            } => backup_exporter.clone(),
            DatastoreStores::Postgres {
                backup_exporter, ..
            } => backup_exporter.clone(),
        }
    }

    pub fn indexer_stats_tracker(&self) -> Arc<dyn IndexerStatsTracker> {
        match &self.stores {
            DatastoreStores::Sqlite { db, .. } => {
                Arc::new(InMemoryIndexerStatsTracker::new(Some(db.pool().clone())))
            }
            DatastoreStores::Postgres { .. } => Arc::new(InMemoryIndexerStatsTracker::new(None)),
        }
    }

    pub fn metadata_gateway_client(
        &self,
        endpoint: String,
        accept_invalid_certs: bool,
        enrollment_config: SmgEnrollmentConfig,
    ) -> MetadataGatewayClient {
        match &self.stores {
            DatastoreStores::Sqlite { settings_store, .. } => {
                MetadataGatewayClient::new_with_enrollment_store(
                    endpoint,
                    accept_invalid_certs,
                    settings_store.clone(),
                    enrollment_config,
                )
            }
            DatastoreStores::Postgres { settings_store, .. } => {
                MetadataGatewayClient::new_with_enrollment_store(
                    endpoint,
                    accept_invalid_certs,
                    settings_store.clone(),
                    enrollment_config,
                )
            }
        }
    }

    pub fn app_services_builder(
        &self,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
    ) -> AppServicesBuilder {
        match &self.stores {
            DatastoreStores::Sqlite {
                catalog_store,
                release_store,
                library_state_store,
                customization_store,
                workflow_store,
                notification_store,
                settings_store,
                ..
            } => {
                let titles: Arc<dyn TitleRepository> = catalog_store.clone();
                let shows: Arc<dyn ShowRepository> = catalog_store.clone();
                let users: Arc<dyn UserRepository> = catalog_store.clone();
                let libraries: Arc<dyn LibraryRepository> = catalog_store.clone();

                AppServices::builder(
                    titles,
                    shows,
                    users,
                    self.indexer_configs(),
                    indexer_client,
                    download_client,
                    self.download_client_configs(),
                    release_store.clone(),
                    self.settings(),
                    self.quality_profiles(),
                    self.backup_dir(),
                )
                .with_libraries(libraries)
                .with_library_state_store(library_state_store.clone())
                .with_customization_store(customization_store.clone())
                .with_acquisition_state(workflow_store.clone())
                .with_domain_events(workflow_store.clone())
                .with_download_submissions(workflow_store.clone())
                .with_download_queue_commands(workflow_store.clone())
                .with_external_import_monitor_snapshots(workflow_store.clone())
                .with_import_artifacts(workflow_store.clone())
                .with_imports(workflow_store.clone())
                .with_job_runs(workflow_store.clone())
                .with_notification_store(notification_store.clone())
                .with_system_info(settings_store.clone())
                .with_logical_backup_exporter(self.logical_backup_exporter())
                .with_title_image_processor(Arc::new(SqliteTitleImageProcessor::new()))
                .with_workflow_operations(workflow_store.clone())
            }
            DatastoreStores::Postgres {
                catalog_store,
                customization_store,
                library_state_store,
                notification_store,
                release_store,
                settings_store,
                workflow_store,
                ..
            } => {
                let titles: Arc<dyn TitleRepository> = catalog_store.clone();
                let shows: Arc<dyn ShowRepository> = catalog_store.clone();
                let users: Arc<dyn UserRepository> = catalog_store.clone();
                let libraries: Arc<dyn LibraryRepository> = catalog_store.clone();

                AppServices::builder(
                    titles,
                    shows,
                    users,
                    self.indexer_configs(),
                    indexer_client,
                    download_client,
                    self.download_client_configs(),
                    release_store.clone(),
                    self.settings(),
                    self.quality_profiles(),
                    self.backup_dir(),
                )
                .with_libraries(libraries)
                .with_library_state_store(library_state_store.clone())
                .with_customization_store(customization_store.clone())
                .with_acquisition_state(workflow_store.clone())
                .with_domain_events(workflow_store.clone())
                .with_download_submissions(workflow_store.clone())
                .with_download_queue_commands(workflow_store.clone())
                .with_external_import_monitor_snapshots(workflow_store.clone())
                .with_import_artifacts(workflow_store.clone())
                .with_imports(workflow_store.clone())
                .with_job_runs(workflow_store.clone())
                .with_notification_store(notification_store.clone())
                .with_system_info(settings_store.clone())
                .with_logical_backup_exporter(self.logical_backup_exporter())
                .with_title_image_processor(Arc::new(NullTitleImageProcessor))
                .with_workflow_operations(workflow_store.clone())
            }
        }
    }
}

pub async fn validate_datastore(config: DatastoreConfig) -> Result<(), AppError> {
    match config.engine {
        DatastoreEngine::Sqlite => {
            SqliteServices::new_with_mode(config.database_url, config.migration_mode).await?;
            Ok(())
        }
        DatastoreEngine::Postgres => {
            PostgresServices::new_with_mode(config.database_url, config.migration_mode).await?;
            Ok(())
        }
    }
}

pub async fn restore_backup_bundle_to_datastore(
    config: DatastoreConfig,
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<scryer_application::BackupRestorePreparedBundle> {
    match config.engine {
        DatastoreEngine::Sqlite => {
            let target_db_path = datastore_file_path(&config.database_url);
            restore_backup_bundle_to_datastore_path(
                &target_db_path,
                config.migration_mode,
                bundle_path,
                passphrase,
            )
            .await
        }
        DatastoreEngine::Postgres => {
            let services =
                PostgresServices::new_with_mode(config.database_url, config.migration_mode).await?;
            let restore_result =
                restore_backup_bundle_into_postgres_pool(services.pool(), bundle_path, passphrase)
                    .await;
            services.pool().close().await;
            restore_result
        }
    }
}

pub async fn restore_backup_bundle_to_datastore_path(
    target_db_path: &Path,
    migration_mode: MigrationMode,
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<scryer_application::BackupRestorePreparedBundle> {
    let services =
        SqliteServices::new_with_mode(target_db_path.to_string_lossy(), migration_mode).await?;
    let restore_result = crate::sqlite_backup::restore_backup_bundle_into_sqlite_pool(
        services.pool(),
        bundle_path,
        passphrase,
    )
    .await;

    let checkpoint_result = if restore_result.is_ok() {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(services.pool())
            .await
            .map(|_| ())
            .map_err(|error| {
                AppError::Repository(format!("failed to checkpoint restored database: {error}"))
            })
    } else {
        Ok(())
    };

    services.pool().close().await;
    drop(services);
    let prepared = restore_result?;
    checkpoint_result?;
    Ok(prepared)
}

pub fn datastore_file_path(database_url: &str) -> PathBuf {
    let raw = database_url
        .strip_prefix("sqlite://")
        .unwrap_or(database_url);
    let raw = raw.split('?').next().unwrap_or(raw);
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    const DATASTORE_ENV_KEYS: &[&str] = &[
        "SCRYER_DB_URL",
        "SCRYER_DB_PATH",
        "SCRYER_DB_USER",
        "SCRYER_DB_PASSWORD",
        "SCRYER_DB_PASSWORD_FILE",
    ];

    struct EnvSnapshot {
        _guard: MutexGuard<'static, ()>,
        values: Vec<(&'static str, Option<String>)>,
    }

    impl EnvSnapshot {
        fn new() -> Self {
            let guard = ENV_LOCK.lock().expect("env lock");
            let values = DATASTORE_ENV_KEYS
                .iter()
                .map(|key| (*key, std::env::var(key).ok()))
                .collect::<Vec<_>>();
            for key in DATASTORE_ENV_KEYS {
                clear_env(key);
            }
            Self {
                _guard: guard,
                values,
            }
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            for (key, value) in &self.values {
                match value {
                    Some(value) => set_env(key, value),
                    None => clear_env(key),
                }
            }
        }
    }

    fn set_env(key: &str, value: &str) {
        // Tests serialize env mutation with ENV_LOCK.
        unsafe { std::env::set_var(key, value) };
    }

    fn clear_env(key: &str) {
        // Tests serialize env mutation with ENV_LOCK.
        unsafe { std::env::remove_var(key) };
    }

    fn data_dir() -> PathBuf {
        std::env::temp_dir().join("scryer-datastore-config-tests")
    }

    fn validation_message(result: AppResult<DatastoreConfig>) -> String {
        match result {
            Err(AppError::Validation(message)) => message,
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn resolves_sqlite_default_and_db_path_fallback() {
        let _env = EnvSnapshot::new();
        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("default sqlite config");
        assert_eq!(config.engine, DatastoreEngine::Sqlite);
        assert_eq!(config.source, DatastoreConfigSource::DefaultSqlite);
        assert!(config.database_url.ends_with("/scryer.db"));
        assert_eq!(config.database_url, config.safe_database_url());

        set_env("SCRYER_DB_PATH", "sqlite:///custom/scryer.db");
        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("db path config");
        assert_eq!(config.engine, DatastoreEngine::Sqlite);
        assert_eq!(config.source, DatastoreConfigSource::EnvDbPath);
        assert_eq!(config.database_url, "sqlite:///custom/scryer.db");
    }

    #[test]
    fn db_url_precedes_db_path_and_redacts_postgres_credentials() {
        let _env = EnvSnapshot::new();
        set_env("SCRYER_DB_PATH", "sqlite:///ignored.db");
        set_env(
            "SCRYER_DB_URL",
            "postgres://url_user:url_pass@db:5432/scryer?sslmode=require",
        );
        set_env("SCRYER_DB_USER", "env_user");
        set_env("SCRYER_DB_PASSWORD", "env_pass");

        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("postgres config");
        assert_eq!(config.engine, DatastoreEngine::Postgres);
        assert_eq!(config.source, DatastoreConfigSource::EnvDbUrl);
        assert!(
            config
                .database_url
                .starts_with("postgres://env_user:env_pass@")
        );
        assert!(!config.safe_database_url().contains("env_user"));
        assert!(!config.safe_database_url().contains("env_pass"));
        assert!(config.safe_database_url().contains("%3Credacted%3E"));
    }

    #[test]
    fn password_file_overrides_password_env_and_url_password() {
        let _env = EnvSnapshot::new();
        let password_path = data_dir().join(format!("password-{}.txt", std::process::id()));
        std::fs::create_dir_all(password_path.parent().expect("password parent"))
            .expect("password dir");
        std::fs::write(&password_path, "file_pass\n").expect("password file");

        set_env(
            "SCRYER_DB_URL",
            "postgres://url_user:url_pass@db:5432/scryer?sslmode=require",
        );
        set_env("SCRYER_DB_PASSWORD", "env_pass");
        set_env(
            "SCRYER_DB_PASSWORD_FILE",
            password_path.to_str().expect("utf-8 password path"),
        );

        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("postgres config");
        assert!(config.database_url.contains("file_pass"));
        assert!(!config.database_url.contains("env_pass"));
        assert!(!config.database_url.contains("url_pass"));

        let _ = std::fs::remove_file(password_path);
    }

    #[test]
    fn password_env_preserves_operator_secret_bytes() {
        let _env = EnvSnapshot::new();
        set_env(
            "SCRYER_DB_URL",
            "postgres://url_user:url_pass@db:5432/scryer?sslmode=require",
        );
        set_env("SCRYER_DB_PASSWORD", "  env pass  ");

        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("postgres config");
        let parsed = url::Url::parse(&config.database_url).expect("valid postgres url");
        assert_eq!(parsed.password(), Some("%20%20env%20pass%20%20"));
    }

    #[test]
    fn postgres_url_requires_database_credentials_and_sslmode() {
        let _env = EnvSnapshot::new();

        set_env(
            "SCRYER_DB_URL",
            "postgres://user:pass@/scryer?sslmode=require",
        );
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("host")
        );

        set_env("SCRYER_DB_URL", "postgres://user:pass@db:5432/scryer");
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("explicit sslmode")
        );

        set_env(
            "SCRYER_DB_URL",
            "postgres://user:pass@db:5432/?sslmode=require",
        );
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("database name")
        );

        set_env("SCRYER_DB_URL", "postgres://db:5432/scryer?sslmode=require");
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("SCRYER_DB_USER")
        );

        set_env("SCRYER_DB_USER", "user");
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("SCRYER_DB_PASSWORD")
        );
    }

    #[test]
    fn rejects_unsupported_datastore_url_scheme_and_sslmode() {
        let _env = EnvSnapshot::new();

        set_env("SCRYER_DB_URL", "mysql://db/scryer");
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("unsupported datastore URL scheme")
        );

        set_env(
            "SCRYER_DB_URL",
            "postgres://user:pass@db:5432/scryer?sslmode=allow",
        );
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("unsupported PostgreSQL sslmode")
        );
    }
}
