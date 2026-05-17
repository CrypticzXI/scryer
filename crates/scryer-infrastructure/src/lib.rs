mod backup_import_normalization;
pub(crate) mod commands;
mod config_store;
mod datastore;
mod download_client_config_store;
mod download_clients;
pub mod encryption;
pub mod external_import;
mod file_importer;
mod graphql;
mod indexer_config_store;
mod indexer_stats;
pub mod keystore;
mod library_renamer;
mod library_scanner;
mod library_state_store;
mod library_store;
mod metadata_gateway;
pub mod migration_assets;
mod migration_hook_ids;
pub mod migrations;
mod notification_store;
mod plugin_store;
mod post_processing_script_store;
pub mod postgres;
mod prowlarr;
mod quality_profile_store;
pub mod queries;
mod release_store;
mod rule_set_store;
mod settings_store;
mod show_store;
pub mod smg_enrollment;
mod spellfix;
mod sqlite_backup;
mod sqlite_services;
mod staged_nzb_store;
mod subtitle_provider_config_store;
mod title_image_store;
mod title_images;
mod title_store;
mod types;
mod user_store;
mod workflow_store;

#[cfg(test)]
mod tests;

pub mod sqlite {
    pub use crate::download_client_config_store::DownloadClientConfigStore;
    pub use crate::indexer_config_store::IndexerConfigStore;
    pub use crate::library_state_store::{LibraryProbeStore, LibraryStateStore};
    pub use crate::library_store::LibraryStore;
    pub use crate::notification_store::NotificationStore;
    pub use crate::plugin_store::PluginStore;
    pub use crate::post_processing_script_store::PostProcessingScriptStore;
    pub use crate::quality_profile_store::QualityProfileStore;
    pub use crate::release_store::ReleaseStore;
    pub use crate::rule_set_store::RuleSetStore;
    pub use crate::settings_store::SettingsStore;
    pub use crate::show_store::ShowStore;
    pub use crate::sqlite_backup::SqliteLogicalBackupExporter;
    pub use crate::sqlite_services::{DbRuntime, SqliteServices};
    pub use crate::subtitle_provider_config_store::SubtitleProviderConfigStore;
    pub use crate::title_image_store::TitleImageStore;
    pub use crate::title_images::HttpTitleImageProcessor;
    pub use crate::title_store::TitleStore;
    pub use crate::user_store::UserStore;
    pub use crate::workflow_store::{
        AcquisitionStore, DomainEventStore, DownloadQueueCommandStore, DownloadSubmissionStore,
        ExternalImportMonitorStore, ImportStore, WorkflowOperationStore,
    };
}

pub use datastore::{
    DatastoreAssembly, DatastoreConfig, DatastoreConfigSource, DatastoreCustomizationStore,
    DatastoreEngine, datastore_file_path, resolve_datastore_config_from_env,
    restore_backup_bundle_to_datastore, restore_backup_bundle_to_datastore_path,
    restore_prepared_backup_directory_to_datastore, validate_datastore,
};
pub use download_client_config_store::DownloadClientConfigStore;
pub use download_clients::{
    MultiIndexerSearchClient, NzbgetDownloadClient, PrioritizedDownloadClientRouter,
    SabnzbdDownloadClient, WeaverDownloadClient, resolve_base_url_from_config_json,
    start_weaver_subscription_bridge,
};
pub use encryption::EncryptionKey;
pub use file_importer::FsFileImporter;
pub use indexer_config_store::IndexerConfigStore;
pub use indexer_stats::InMemoryIndexerStatsTracker;
pub use library_renamer::FileSystemLibraryRenamer;
pub use library_scanner::FileSystemLibraryScanner;
pub use library_state_store::{LibraryProbeStore, LibraryStateStore};
pub use library_store::LibraryStore;
pub use metadata_gateway::{MetadataGatewayClient, SmgEnrollmentConfig};
pub use migrations::{list_embedded_migration_keys, list_embedded_migrations};
pub use notification_store::NotificationStore;
pub use plugin_store::PluginStore;
pub use post_processing_script_store::PostProcessingScriptStore;
pub use postgres::{PostgresLogicalBackupExporter, PostgresServices};
pub use prowlarr::{NativeProwlarrIndexerProvider, PROWLARR_PROVIDER_TYPE};
pub use quality_profile_store::QualityProfileStore;
pub use release_store::ReleaseStore;
pub use rule_set_store::RuleSetStore;
pub use settings_store::SettingsStore;
pub use show_store::ShowStore;
pub use spellfix::register_spellfix_auto_extension;
pub use sqlite_backup::SqliteLogicalBackupExporter;
pub use sqlite_services::{DbRuntime, SqliteServices};
pub use staged_nzb_store::FileSystemStagedNzbStore;
pub use subtitle_provider_config_store::SubtitleProviderConfigStore;
pub use title_image_store::TitleImageStore;
pub use title_images::HttpTitleImageProcessor;
pub use title_store::TitleStore;
pub(crate) use types::sqlite_url_with_create;
pub use types::{
    DownloadQueueCommandRecord, EmbeddedMigrationDescriptor, LibraryProbeSignatureRecord,
    MigrationMode, MigrationStatus, SettingDefinitionSeed, SettingsDefinitionRecord,
    SettingsValueRecord, WorkflowOperationRecord,
};
pub use user_store::UserStore;
pub use workflow_store::{
    AcquisitionStore, DomainEventStore, DownloadQueueCommandStore, DownloadSubmissionStore,
    ExternalImportMonitorStore, ImportStore, WorkflowOperationStore,
};
