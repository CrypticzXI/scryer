use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use scryer_application::{
    AppError, AppResult, AppServices, AppServicesBuilder, DownloadClient,
    DownloadClientConfigRepository, IndexerClient, IndexerConfigRepository, IndexerStatsTracker,
    LibraryRepository, LogicalBackupExporter, QualityProfileRepository, SettingsRepository,
    ShowRepository, SubtitleProviderConfigRepository, TitleImageRepository, TitleRepository,
    UserRepository,
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
}

impl DatastoreEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DatastoreConfig {
    pub engine: DatastoreEngine,
    pub database_url: String,
    pub data_dir: PathBuf,
    pub migration_mode: MigrationMode,
}

impl DatastoreConfig {
    pub fn sqlite(
        database_url: impl Into<String>,
        data_dir: impl Into<PathBuf>,
        migration_mode: MigrationMode,
    ) -> Self {
        Self {
            engine: DatastoreEngine::Sqlite,
            database_url: database_url.into(),
            data_dir: data_dir.into(),
            migration_mode,
        }
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }
}

#[derive(Clone)]
pub struct DatastoreSettingsStore {
    inner: SqliteSettingsStore,
}

impl DatastoreSettingsStore {
    pub fn from_sqlite(inner: SqliteSettingsStore) -> Self {
        Self { inner }
    }
}

impl Deref for DatastoreSettingsStore {
    type Target = SqliteSettingsStore;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Clone)]
pub struct DatastoreCustomizationStore {
    inner: SqliteCustomizationStore,
}

impl DatastoreCustomizationStore {
    pub fn from_sqlite(inner: SqliteCustomizationStore) -> Self {
        Self { inner }
    }
}

impl Deref for DatastoreCustomizationStore {
    type Target = SqliteCustomizationStore;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Clone)]
pub struct DatastoreAssembly {
    config: DatastoreConfig,
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
}

impl DatastoreAssembly {
    pub async fn connect(config: DatastoreConfig) -> Result<Self, AppError> {
        match config.engine {
            DatastoreEngine::Sqlite => Self::connect_sqlite(config).await,
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

        Ok(Self {
            config,
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
        })
    }

    pub fn engine(&self) -> DatastoreEngine {
        self.config.engine
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.config.backup_dir()
    }

    pub fn staged_nzb_path(&self) -> PathBuf {
        FileSystemStagedNzbStore::path_for_main_db(&self.config.database_url)
    }

    pub fn bootstrap_settings_store(&self) -> DatastoreSettingsStore {
        DatastoreSettingsStore {
            inner: (*self.settings_store).clone(),
        }
    }

    pub fn customization_store(&self) -> DatastoreCustomizationStore {
        DatastoreCustomizationStore {
            inner: (*self.customization_store).clone(),
        }
    }

    pub async fn bootstrap_encryption(&self) -> Result<u64, String> {
        let encryption_key =
            crate::encryption::ensure_encryption_key(&self.db, Some(self.config.data_dir.clone()))
                .await?;
        self.db
            .set_encryption_key(encryption_key)
            .await
            .map_err(|error| error.to_string())?;
        self.db
            .migrate_legacy_indexer_config_sources()
            .await
            .map_err(|error| error.to_string())
    }

    pub fn indexer_configs(&self) -> Arc<dyn IndexerConfigRepository> {
        self.config_store.clone()
    }

    pub fn download_client_configs(&self) -> Arc<dyn DownloadClientConfigRepository> {
        self.config_store.clone()
    }

    pub fn subtitle_provider_configs(&self) -> Arc<dyn SubtitleProviderConfigRepository> {
        self.config_store.clone()
    }

    pub fn settings(&self) -> Arc<dyn SettingsRepository> {
        self.settings_store.clone()
    }

    pub fn quality_profiles(&self) -> Arc<dyn QualityProfileRepository> {
        self.settings_store.clone()
    }

    pub fn title_images(&self) -> Arc<dyn TitleImageRepository> {
        self.library_state_store.clone()
    }

    pub fn logical_backup_exporter(&self) -> Arc<dyn LogicalBackupExporter> {
        self.backup_exporter.clone()
    }

    pub fn indexer_stats_tracker(&self) -> Arc<dyn IndexerStatsTracker> {
        Arc::new(InMemoryIndexerStatsTracker::new(Some(
            self.db.pool().clone(),
        )))
    }

    pub fn metadata_gateway_client(
        &self,
        endpoint: String,
        accept_invalid_certs: bool,
        enrollment_config: SmgEnrollmentConfig,
    ) -> MetadataGatewayClient {
        MetadataGatewayClient::new(
            endpoint,
            accept_invalid_certs,
            self.db.clone(),
            enrollment_config,
        )
    }

    pub fn app_services_builder(
        &self,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
    ) -> AppServicesBuilder {
        let titles: Arc<dyn TitleRepository> = self.catalog_store.clone();
        let shows: Arc<dyn ShowRepository> = self.catalog_store.clone();
        let users: Arc<dyn UserRepository> = self.catalog_store.clone();
        let libraries: Arc<dyn LibraryRepository> = self.catalog_store.clone();

        AppServices::builder(
            titles,
            shows,
            users,
            self.indexer_configs(),
            indexer_client,
            download_client,
            self.download_client_configs(),
            self.release_store.clone(),
            self.settings(),
            self.quality_profiles(),
            self.backup_dir(),
        )
        .with_libraries(libraries)
        .with_library_state_store(self.library_state_store.clone())
        .with_customization_store(self.customization_store.clone())
        .with_acquisition_state(self.workflow_store.clone())
        .with_domain_events(self.workflow_store.clone())
        .with_download_submissions(self.workflow_store.clone())
        .with_download_queue_commands(self.workflow_store.clone())
        .with_external_import_monitor_snapshots(self.workflow_store.clone())
        .with_import_artifacts(self.workflow_store.clone())
        .with_imports(self.workflow_store.clone())
        .with_job_runs(self.workflow_store.clone())
        .with_notification_store(self.notification_store.clone())
        .with_system_info(self.settings_store.clone())
        .with_logical_backup_exporter(self.logical_backup_exporter())
        .with_title_image_processor(Arc::new(SqliteTitleImageProcessor::new()))
        .with_workflow_operations(self.workflow_store.clone())
    }
}

pub async fn validate_datastore(config: DatastoreConfig) -> Result<(), AppError> {
    match config.engine {
        DatastoreEngine::Sqlite => {
            SqliteServices::new_with_mode(config.database_url, config.migration_mode).await?;
            Ok(())
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
