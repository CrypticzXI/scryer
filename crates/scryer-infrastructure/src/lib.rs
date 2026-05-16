mod backup_import_normalization;
mod catalog_store;
pub(crate) mod commands;
mod config_store;
mod customization_store;
mod datastore;
mod download_clients;
pub mod encryption;
pub mod external_import;
mod file_importer;
mod graphql;
mod indexer_stats;
pub mod keystore;
mod library_renamer;
mod library_scanner;
mod library_state_store;
mod metadata_gateway;
pub mod migration_assets;
mod migration_hook_ids;
pub mod migrations;
mod notification_store;
pub mod postgres;
mod prowlarr;
pub mod queries;
mod release_store;
mod settings_store;
pub mod smg_enrollment;
mod spellfix;
mod sqlite_backup;
mod sqlite_services;
mod staged_nzb_store;
mod title_images;
mod types;
mod workflow_store;

#[cfg(test)]
mod tests;

pub mod sqlite {
    pub use crate::catalog_store::SqliteCatalogStore;
    pub use crate::config_store::SqliteConfigStore;
    pub use crate::customization_store::SqliteCustomizationStore;
    pub use crate::library_state_store::SqliteLibraryStateStore;
    pub use crate::notification_store::SqliteNotificationStore;
    pub use crate::release_store::SqliteReleaseStore;
    pub use crate::settings_store::SqliteSettingsStore;
    pub use crate::sqlite_backup::SqliteLogicalBackupExporter;
    pub use crate::sqlite_services::{DbRuntime, SqliteServices};
    pub use crate::title_images::SqliteTitleImageProcessor;
    pub use crate::workflow_store::SqliteWorkflowStore;
}

pub use catalog_store::SqliteCatalogStore;
pub use config_store::SqliteConfigStore;
pub use customization_store::SqliteCustomizationStore;
pub use datastore::{
    DatastoreAssembly, DatastoreConfig, DatastoreConfigSource, DatastoreCustomizationStore,
    DatastoreEngine, DatastoreSettingsStore, datastore_file_path,
    resolve_datastore_config_from_env, restore_backup_bundle_to_datastore,
    restore_backup_bundle_to_datastore_path, restore_prepared_backup_directory_to_datastore,
    validate_datastore,
};
pub use download_clients::{
    MultiIndexerSearchClient, NzbgetDownloadClient, PrioritizedDownloadClientRouter,
    SabnzbdDownloadClient, WeaverDownloadClient, resolve_base_url_from_config_json,
    start_weaver_subscription_bridge,
};
pub use encryption::EncryptionKey;
pub use file_importer::FsFileImporter;
pub use indexer_stats::InMemoryIndexerStatsTracker;
pub use library_renamer::FileSystemLibraryRenamer;
pub use library_scanner::FileSystemLibraryScanner;
pub use library_state_store::SqliteLibraryStateStore;
pub use metadata_gateway::{MetadataGatewayClient, SmgEnrollmentConfig};
pub use migrations::{list_embedded_migration_keys, list_embedded_migrations};
pub use notification_store::SqliteNotificationStore;
pub use postgres::{
    PostgresCatalogStore, PostgresConfigStore, PostgresCustomizationStore,
    PostgresLogicalBackupExporter, PostgresReleaseStore, PostgresServices, PostgresSettingsStore,
};
pub use prowlarr::{NativeProwlarrIndexerProvider, PROWLARR_PROVIDER_TYPE};
pub use release_store::SqliteReleaseStore;
pub use settings_store::SqliteSettingsStore;
pub use spellfix::register_spellfix_auto_extension;
pub use sqlite_backup::SqliteLogicalBackupExporter;
pub use sqlite_services::{DbRuntime, SqliteServices};
pub use staged_nzb_store::FileSystemStagedNzbStore;
pub use title_images::SqliteTitleImageProcessor;
pub(crate) use types::sqlite_url_with_create;
pub use types::{
    DownloadQueueCommandRecord, EmbeddedMigrationDescriptor, LibraryProbeSignatureRecord,
    MigrationMode, MigrationStatus, SettingDefinitionSeed, SettingsDefinitionRecord,
    SettingsValueRecord, WorkflowOperationRecord,
};
pub use workflow_store::SqliteWorkflowStore;
