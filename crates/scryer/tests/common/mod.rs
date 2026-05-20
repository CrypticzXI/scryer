#![allow(dead_code)]

use std::sync::Arc;

use async_graphql_axum::GraphQLRequest;
use async_trait::async_trait;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::json;
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use scryer_application::{
    AppResult, AppServices, AppUseCase, BlocklistRepository, FacetRegistry, HousekeepingRepository,
    IndexerPluginProvider, JwtAuthConfig, MovieFacetHandler, PendingReleaseRepository,
    SeriesFacetHandler, SubtitleDownloadRepository, WantedItemRepository,
};
use scryer_infrastructure::sqlite::{
    LibraryStore, PluginStore, PostProcessingScriptStore, QualityProfileStore, RuleSetStore,
    SettingsStore, ShowStore, TitleStore, UserStore,
};
use scryer_infrastructure::{
    AcquisitionStore, DomainEventStore, DownloadClientConfigStore, DownloadQueueCommandStore,
    DownloadSubmissionStore, ExternalImportMonitorStore, FileSystemLibraryScanner,
    FileSystemStagedNzbStore, HousekeepingStore, ImportStore, IndexerConfigStore,
    LibraryProbeStore, LibraryScanUnmatchedStore, MediaFileStore, MetadataGatewayClient,
    MultiIndexerSearchClient, NzbgetDownloadClient, PendingReleaseStore, ReleaseStore,
    SmgEnrollmentConfig, SqliteServices, SubtitleDownloadStore, TitleImageStore, WantedStore,
    WorkflowOperationStore,
};
use scryer_interface::context::{AuthRuntimeStateHandle, AuthRuntimeStateSnapshot};
use scryer_interface::{ApiSchema, build_schema};

/// Shared integration-test context.
///
/// Boots wiremock servers for external APIs, in-memory SQLite, real
/// infrastructure clients pointed at wiremock, a full `AppUseCase`,
/// GraphQL schema, and an axum server on a random port.
pub struct TestContext {
    pub nzbget_server: MockServer,
    pub nzbgeek_server: MockServer,
    pub smg_server: MockServer,
    /// Base URL of the test axum server (e.g. `http://127.0.0.1:12345`).
    pub app_url: String,
    pub schema: ApiSchema,
    pub auth_runtime: AuthRuntimeStateHandle,
    pub app: AppUseCase,
    pub titles: TitleStore,
    pub shows: ShowStore,
    pub libraries: LibraryStore,
    pub users: UserStore,
    pub customization: PluginStore,
    pub library_probe: LibraryProbeStore,
    pub library_state: TestLibraryStateStore,
    pub library_scan_unmatched: LibraryScanUnmatchedStore,
    pub media_files: MediaFileStore,
    pub db: SqliteServices,
    pub settings_store: Arc<SettingsStore>,
    pub app_data_dir: tempfile::TempDir,
    pub staged_nzb_store: Arc<FileSystemStagedNzbStore>,
    pub staged_nzb_dir: tempfile::TempDir,
}

#[derive(Clone)]
pub struct TestLibraryStateStore {
    pub wanted: WantedStore,
    pub pending_releases: PendingReleaseStore,
    pub blocklist: scryer_infrastructure::BlocklistStore,
    pub housekeeping: HousekeepingStore,
    pub subtitle_downloads: SubtitleDownloadStore,
}

#[async_trait]
impl WantedItemRepository for TestLibraryStateStore {
    async fn upsert_wanted_item(&self, item: &scryer_application::WantedItem) -> AppResult<String> {
        self.wanted.upsert_wanted_item(item).await
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[scryer_domain::MediaFacet],
    ) -> AppResult<Vec<scryer_application::WantedItem>> {
        self.wanted
            .list_due_wanted_items(now, batch_limit, excluded_facets)
            .await
    }

    async fn update_wanted_item_status(
        &self,
        id: &str,
        status: &str,
        next_search_at: Option<&str>,
        last_search_at: Option<&str>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        self.wanted
            .update_wanted_item_status(
                id,
                status,
                next_search_at,
                last_search_at,
                search_count,
                current_score,
                grabbed_release,
            )
            .await
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<scryer_application::WantedItem>> {
        self.wanted
            .get_wanted_item_for_title(title_id, episode_id)
            .await
    }

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        self.wanted.delete_wanted_items_for_title(title_id).await
    }

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        self.wanted
            .delete_wanted_items_for_collection(collection_id)
            .await
    }

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        self.wanted
            .delete_wanted_items_for_episode(episode_id)
            .await
    }

    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        self.wanted.reset_fruitless_wanted_items(now).await
    }

    async fn insert_release_decision(
        &self,
        decision: &scryer_application::ReleaseDecision,
    ) -> AppResult<String> {
        self.wanted.insert_release_decision(decision).await
    }

    async fn get_wanted_item_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_application::WantedItem>> {
        self.wanted.get_wanted_item_by_id(id).await
    }

    async fn list_wanted_items(
        &self,
        query: scryer_application::WantedItemsQuery,
    ) -> AppResult<Vec<scryer_application::WantedItem>> {
        self.wanted.list_wanted_items(query).await
    }

    async fn count_wanted_items(
        &self,
        query: scryer_application::WantedItemsQuery,
    ) -> AppResult<i64> {
        self.wanted.count_wanted_items(query).await
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<scryer_application::ReleaseDecision>> {
        self.wanted
            .list_release_decisions_for_title(title_id, limit)
            .await
    }

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<scryer_application::ReleaseDecision>> {
        self.wanted
            .list_release_decisions_for_wanted_item(wanted_item_id, limit)
            .await
    }
}

#[async_trait]
impl PendingReleaseRepository for TestLibraryStateStore {
    async fn insert_pending_release(
        &self,
        release: &scryer_application::PendingRelease,
    ) -> AppResult<String> {
        self.pending_releases.insert_pending_release(release).await
    }

    async fn list_expired_pending_releases(
        &self,
        now: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_expired_pending_releases(now)
            .await
    }

    async fn list_waiting_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases.list_waiting_pending_releases().await
    }

    async fn get_pending_release(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_application::PendingRelease>> {
        self.pending_releases.get_pending_release(id).await
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_pending_releases_for_title(title_id)
            .await
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        self.pending_releases
            .update_pending_release_status(id, status, grabbed_at)
            .await
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        self.pending_releases
            .delete_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_all_standby_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_all_standby_pending_releases()
            .await
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: scryer_application::PendingReleaseStatus,
        next_status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        self.pending_releases
            .compare_and_set_pending_release_status(id, current_status, next_status, grabbed_at)
            .await
    }

    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        self.pending_releases
            .supersede_pending_releases_for_wanted_item(wanted_item_id, except_id)
            .await
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        self.pending_releases
            .delete_pending_releases_for_title(title_id)
            .await
    }
}

#[async_trait]
impl BlocklistRepository for TestLibraryStateStore {
    async fn add(&self, entry: &scryer_application::NewBlocklistEntry) -> AppResult<String> {
        self.blocklist.add(entry).await
    }

    async fn list_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::BlocklistEntry>> {
        self.blocklist.list_for_title(title_id, limit).await
    }

    async fn list_all(
        &self,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<scryer_domain::BlocklistEntry>, i64)> {
        self.blocklist.list_all(limit, offset).await
    }

    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool> {
        self.blocklist
            .has_recorded_download_failure(title_id, source_title)
            .await
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        self.blocklist.remove(id).await
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        self.blocklist.is_blocklisted(title_id, source_title).await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.blocklist.delete_for_title(title_id).await
    }
}

#[async_trait]
impl HousekeepingRepository for TestLibraryStateStore {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_release_decisions_older_than(days)
            .await
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_release_attempts_older_than(days)
            .await
    }

    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_dispatched_event_outboxes_older_than(days)
            .await
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_history_events_older_than(days)
            .await
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[scryer_domain::DomainEventType],
    ) -> AppResult<u32> {
        self.housekeeping
            .delete_domain_events_older_than_for_types(days, event_types)
            .await
    }

    async fn delete_title_history_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_title_history_older_than(days)
            .await
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_download_import_artifacts_older_than(days)
            .await
    }

    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_terminal_imports_older_than(days)
            .await
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        self.housekeeping
            .delete_terminal_download_queue_commands_older_than(days)
            .await
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_rule_set_history_older_than(days)
            .await
    }

    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.housekeeping
            .delete_history_events_for_title_ids(title_ids)
            .await
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        self.housekeeping
            .delete_download_import_artifacts_for_title_ids(title_ids)
            .await
    }

    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.housekeeping
            .delete_release_attempts_for_title_ids(title_ids)
            .await
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        self.housekeeping.list_all_media_file_paths().await
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        self.housekeeping.delete_media_files_by_ids(ids).await
    }

    async fn run_database_maintenance(&self) -> AppResult<()> {
        self.housekeeping.run_database_maintenance().await
    }
}

#[async_trait]
impl SubtitleDownloadRepository for TestLibraryStateStore {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        self.subtitle_downloads.list_for_title(title_id).await
    }

    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        self.subtitle_downloads.get(id).await
    }

    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        self.subtitle_downloads
            .list_for_media_file(media_file_id)
            .await
    }

    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_application::subtitles::ExternalSubtitleProbeCacheEntry>> {
        self.subtitle_downloads
            .list_probe_cache_for_media_file(media_file_id)
            .await
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>> {
        self.subtitle_downloads
            .list_blocklist_for_media_file(media_file_id)
            .await
    }

    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        self.subtitle_downloads.insert(download).await
    }

    async fn upsert_probe_cache_entry(
        &self,
        entry: &scryer_application::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        self.subtitle_downloads
            .upsert_probe_cache_entry(entry)
            .await
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        self.subtitle_downloads.set_synced(id, synced).await
    }

    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        self.subtitle_downloads.delete(id).await
    }

    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        self.subtitle_downloads
            .delete_probe_cache_entry(media_file_id, file_path)
            .await
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        self.subtitle_downloads
            .is_blocklisted(media_file_id, provider, provider_file_id)
            .await
    }

    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        self.subtitle_downloads
            .blocklist(media_file_id, provider, provider_file_id, language, reason)
            .await
    }
}

pub fn disabled_auth_runtime_handle() -> AuthRuntimeStateHandle {
    AuthRuntimeStateHandle::new(AuthRuntimeStateSnapshot {
        form_login_enabled: false,
        skip_login_for_local_ips: false,
        effective_form_login_enabled: false,
        env_override_active: false,
        env_override_description: None,
        epoch: 0,
    })
}

impl TestContext {
    pub async fn new() -> Self {
        // Start wiremock mock servers for each external API
        let nzbget_server = MockServer::start().await;
        let nzbgeek_server = MockServer::start().await;
        let smg_server = MockServer::start().await;
        mount_default_smg_metadata_mocks(&smg_server).await;

        // In-memory SQLite with migrations applied
        let db = SqliteServices::new(":memory:")
            .await
            .expect("failed to create in-memory SQLite");
        let app_data_dir = tempfile::Builder::new()
            .prefix("scryer-test-data-")
            .tempdir_in("/tmp")
            .expect("failed to create app data tempdir");
        let staged_nzb_dir = tempfile::TempDir::new().expect("failed to create staged nzb tempdir");
        let staged_nzb_store = Arc::new(
            FileSystemStagedNzbStore::new(staged_nzb_dir.path())
                .await
                .expect("failed to create staged nzb store"),
        );
        let staged_nzb_pipeline_limit = Arc::new(tokio::sync::Semaphore::new(4));
        let datastore = db.datastore();
        let release_store = Arc::new(ReleaseStore::new(datastore.clone()));
        let settings_store = Arc::new(SettingsStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let quality_profile_store = Arc::new(QualityProfileStore::new(datastore.clone()));

        // Real clients pointed at wiremock URLs
        let nzbget = NzbgetDownloadClient::with_staged_nzb_store(
            nzbget_server.uri(),
            Some("test-user".to_string()),
            Some("test-pass".to_string()),
            "SCORE".to_string(),
            staged_nzb_store.clone(),
            staged_nzb_pipeline_limit.clone(),
        );

        let indexer_config_store = Arc::new(IndexerConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let download_client_config_store = Arc::new(DownloadClientConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));

        // Build indexer client backed by built-in WASM plugins (using DynamicPluginProvider
        // so reload_plugins works in integration tests)
        let plugin_provider: Arc<dyn IndexerPluginProvider> =
            Arc::new(scryer_plugins::DynamicPluginProvider::new(
                scryer_plugins::WasmIndexerPluginProvider::empty()
                    .with_builtin_asset(scryer_plugins::builtins::NZBGEEK)
                    .with_builtin_asset(scryer_plugins::builtins::NEWZNAB),
            ));
        let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> = Arc::new(
            scryer_infrastructure::InMemoryIndexerStatsTracker::new(None),
        );
        let indexer_client = MultiIndexerSearchClient::new(
            indexer_config_store.clone(),
            indexer_stats.clone(),
            plugin_provider.clone(),
        );

        let metadata_gateway = MetadataGatewayClient::new(
            format!("{}/graphql", smg_server.uri()),
            true, // accept invalid certs (wiremock is plain HTTP)
            db.clone(),
            SmgEnrollmentConfig {
                registration_secret: None,
                ca_cert: None,
            },
        );

        // Build repository implementations from the shared DB runtime.
        let title_store = TitleStore::new(datastore.clone());
        let show_store = ShowStore::new(datastore.clone());
        let library_store = LibraryStore::new(datastore.clone());
        let user_store = UserStore::new(datastore.clone());
        let titles: Arc<dyn scryer_application::TitleRepository> = Arc::new(title_store.clone());
        let shows: Arc<dyn scryer_application::ShowRepository> = Arc::new(show_store.clone());
        let users: Arc<dyn scryer_application::UserRepository> = Arc::new(user_store.clone());
        let indexer_configs: Arc<dyn scryer_application::IndexerConfigRepository> =
            indexer_config_store;
        let download_client_configs: Arc<dyn scryer_application::DownloadClientConfigRepository> =
            download_client_config_store;
        let release_attempts: Arc<dyn scryer_application::ReleaseAttemptRepository> = release_store;
        let settings: Arc<dyn scryer_application::SettingsRepository> = settings_store.clone();
        let quality_profiles: Arc<dyn scryer_application::QualityProfileRepository> =
            quality_profile_store.clone();

        let library_probe_store = LibraryProbeStore::new(datastore.clone());
        let wanted_store = WantedStore::new(datastore.clone());
        let pending_release_store = PendingReleaseStore::new(datastore.clone());
        let blocklist_store = scryer_infrastructure::BlocklistStore::new(datastore.clone());
        let housekeeping_store = HousekeepingStore::new(datastore.clone());
        let subtitle_download_store = SubtitleDownloadStore::new(datastore.clone());
        let library_state_store = TestLibraryStateStore {
            wanted: wanted_store.clone(),
            pending_releases: pending_release_store.clone(),
            blocklist: blocklist_store.clone(),
            housekeeping: housekeeping_store.clone(),
            subtitle_downloads: subtitle_download_store.clone(),
        };
        let library_scan_unmatched_store = LibraryScanUnmatchedStore::new(datastore.clone());
        let media_file_store = MediaFileStore::new(datastore.clone());
        let title_image_store = TitleImageStore::new(datastore.clone());
        let rule_set_store = RuleSetStore::new(datastore.clone());
        let post_processing_script_store = PostProcessingScriptStore::new(datastore.clone());
        let plugin_store = PluginStore::new(datastore.clone());
        let domain_event_store = Arc::new(DomainEventStore::new(datastore.clone()));
        let acquisition_store = Arc::new(AcquisitionStore::new(datastore.clone()));
        let download_submission_store = Arc::new(DownloadSubmissionStore::new(datastore.clone()));
        let import_store = Arc::new(ImportStore::new(datastore.clone()));
        let external_import_monitor_store =
            Arc::new(ExternalImportMonitorStore::new(datastore.clone()));
        let download_queue_command_store =
            Arc::new(DownloadQueueCommandStore::new(datastore.clone()));
        let workflow_operation_store = Arc::new(WorkflowOperationStore::new(datastore.clone()));
        let services = AppServices::builder(
            titles,
            shows,
            users,
            indexer_configs,
            Arc::new(indexer_client),
            Arc::new(nzbget),
            download_client_configs,
            release_attempts,
            settings,
            quality_profiles,
            app_data_dir.path().display().to_string(),
        )
        .with_media_files(Arc::new(media_file_store.clone()))
        .with_wanted_items(Arc::new(wanted_store))
        .with_pending_releases(Arc::new(pending_release_store))
        .with_blocklist_repo(Arc::new(blocklist_store))
        .with_library_probe_signatures(Arc::new(library_probe_store.clone()))
        .with_library_scan_unmatched_items(Arc::new(library_scan_unmatched_store.clone()))
        .with_title_images(Arc::new(title_image_store))
        .with_housekeeping(Arc::new(housekeeping_store))
        .with_subtitle_downloads(Arc::new(subtitle_download_store))
        .with_libraries(Arc::new(library_store.clone()))
        .with_rule_set_store(Arc::new(rule_set_store))
        .with_post_processing_script_store(Arc::new(post_processing_script_store))
        .with_plugin_installation_store(Arc::new(plugin_store.clone()))
        .with_acquisition_state(acquisition_store)
        .with_domain_events(domain_event_store)
        .with_download_queue_commands(download_queue_command_store)
        .with_download_submissions(download_submission_store)
        .with_external_import_monitor_snapshots(external_import_monitor_store)
        .with_import_artifacts(import_store.clone())
        .with_imports(import_store)
        .with_job_runs(workflow_operation_store.clone())
        .with_system_info(settings_store.clone())
        .with_metadata_gateway(Arc::new(metadata_gateway))
        .with_library_scanner(Arc::new(FileSystemLibraryScanner::new()))
        .with_indexer_stats(indexer_stats)
        .with_plugin_provider(plugin_provider)
        .with_staged_nzb_store(staged_nzb_store.clone())
        .with_staged_nzb_pipeline_limit(staged_nzb_pipeline_limit)
        .with_workflow_operations(workflow_operation_store)
        .build();

        // Facet registry with all built-in facets
        let mut registry = FacetRegistry::new();
        registry.register(Arc::new(MovieFacetHandler));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Series,
        )));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Anime,
        )));
        let facet_registry = Arc::new(registry);

        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            facet_registry,
        );

        // Build the GraphQL schema with authentication disabled.
        let auth_runtime = disabled_auth_runtime_handle();
        let schema = build_schema(app.clone(), auth_runtime.clone());

        // Start axum server on a random port
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind test server");
        let addr = listener.local_addr().expect("failed to get local addr");
        let app_url = format!("http://{addr}");

        let router = build_test_router(app.clone(), schema.clone(), auth_runtime.clone());
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("test server failed");
        });

        Self {
            nzbget_server,
            nzbgeek_server,
            smg_server,
            app_url,
            schema,
            auth_runtime,
            app,
            titles: title_store,
            shows: show_store,
            libraries: library_store,
            users: user_store,
            customization: plugin_store,
            library_probe: library_probe_store,
            library_state: library_state_store,
            library_scan_unmatched: library_scan_unmatched_store,
            media_files: media_file_store,
            db,
            settings_store,
            app_data_dir,
            staged_nzb_store,
            staged_nzb_dir,
        }
    }

    /// URL for the GraphQL endpoint.
    pub fn graphql_url(&self) -> String {
        format!("{}/graphql", self.app_url)
    }

    /// Build a reqwest client suitable for hitting the test server.
    pub fn http_client(&self) -> reqwest::Client {
        scryer_outbound_http::install_default_rustls_provider();
        reqwest::Client::builder()
            .build()
            .expect("failed to build reqwest client")
    }
}

async fn mount_default_smg_metadata_mocks(server: &MockServer) {
    let fixture = json!({
        "data": {
            "m0": {
                "movie": {
                    "tvdb_id": 123456,
                    "name": "Test Movie Title",
                    "slug": "test-movie-title",
                    "year": 2024,
                    "status": "Released",
                    "overview": "A gripping tale of testing integration.",
                    "poster_url": "https://artworks.thetvdb.com/banners/movies/123456/posters/test.jpg",
                    "language": "eng",
                    "runtime_minutes": 142,
                    "sort_title": "Test Movie Title",
                    "imdb_id": "tt1234567",
                    "genres": ["Action", "Thriller"],
                    "studio": "Test Studios",
                    "tmdb_release_date": "2024-06-15"
                }
            },
            "s0": {
                "series": {
                    "tvdb_id": 345678,
                    "name": "Test Show Name",
                    "sort_name": "Test Show Name",
                    "slug": "test-show-name",
                    "status": "Continuing",
                    "year": 2023,
                    "first_aired": "2023-09-15",
                    "overview": "A compelling drama about software testing.",
                    "network": "Test Network",
                    "runtime_minutes": 45,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/345678/posters/test.jpg",
                    "country": "usa",
                    "genres": ["Drama", "Thriller"],
                    "aliases": ["Testing Show", "QA Chronicles"],
                    "tagged_aliases": [],
                    "seasons": [
                        {
                            "tvdb_id": 1000001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2000001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Pilot",
                            "aired": "2023-09-15",
                            "runtime_minutes": 60,
                            "is_filler": false,
                            "is_recap": false,
                            "language": "eng",
                            "overview": "The team assembles.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }
            }
        }
    })
    .to_string();

    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .with_priority(100)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .with_priority(100)
        .mount(server)
        .await;
}

/// Load a fixture file relative to the workspace `tests/fixtures/` directory.
pub fn load_fixture(path: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
        .join(path);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to load fixture {}: {e}", fixture_path.display()))
}

/// Build a minimal axum router with a GraphQL endpoint and authentication disabled.
fn build_test_router(
    app: AppUseCase,
    schema: ApiSchema,
    auth_runtime: AuthRuntimeStateHandle,
) -> Router {
    Router::new().route(
        "/graphql",
        post(test_graphql_handler).with_state((app, schema, auth_runtime)),
    )
}

/// Minimal GraphQL handler that replicates auth-disabled default-user injection.
async fn test_graphql_handler(
    State((app, schema, auth_runtime)): State<(AppUseCase, ApiSchema, AuthRuntimeStateHandle)>,
    req: GraphQLRequest,
) -> Response {
    let user = if auth_runtime.snapshot().effective_form_login_enabled {
        None
    } else {
        app.find_or_create_default_user().await.ok()
    };
    let mut request = req.into_inner();
    let response_status = graphql_response_status(&mut request);
    if let Some(u) = user {
        request = request.data(u);
    }
    let mut response =
        async_graphql_axum::GraphQLResponse::from(schema.execute(request).await).into_response();
    *response.status_mut() = response_status;
    response
}

fn graphql_response_status(request: &mut async_graphql::Request) -> StatusCode {
    let _ = request;
    StatusCode::OK
}
