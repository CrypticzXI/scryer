use super::*;

#[derive(Clone)]
pub struct AppServices {
    pub titles: Arc<dyn TitleRepository>,
    pub shows: Arc<dyn ShowRepository>,
    pub users: Arc<dyn UserRepository>,
    pub domain_events: Arc<dyn DomainEventRepository>,
    pub indexer_configs: Arc<dyn IndexerConfigRepository>,
    pub indexer_client: Arc<dyn IndexerClient>,
    pub download_client: Arc<dyn DownloadClient>,
    pub metadata_gateway: Arc<dyn MetadataGateway>,
    pub library_scanner: Arc<dyn LibraryScanner>,
    pub library_renamer: Arc<dyn LibraryRenamer>,
    pub imports: Arc<dyn ImportRepository>,
    pub file_importer: Arc<dyn FileImporter>,
    pub media_files: Arc<dyn MediaFileRepository>,
    pub media_analyzer: Arc<dyn MediaAnalyzer>,
    pub download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    pub release_attempts: Arc<dyn ReleaseAttemptRepository>,
    pub acquisition_state: Arc<dyn AcquisitionStateRepository>,
    pub download_submissions: Arc<dyn DownloadSubmissionRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub quality_profiles: Arc<dyn QualityProfileRepository>,
    pub wanted_items: Arc<dyn WantedItemRepository>,
    pub rule_sets: Arc<dyn RuleSetRepository>,
    pub pp_scripts: Arc<dyn PostProcessingScriptRepository>,
    pub plugin_installations: Arc<dyn PluginInstallationRepository>,
    pub system_info: Arc<dyn SystemInfoProvider>,
    pub title_images: Arc<dyn TitleImageRepository>,
    pub title_image_processor: Arc<dyn TitleImageProcessor>,
    pub indexer_stats: Arc<dyn IndexerStatsTracker>,
    pub user_rules: Arc<std::sync::RwLock<scryer_rules::UserRulesEngine>>,
    pub plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
    pub download_client_plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    pub notification_channels: Option<Arc<dyn NotificationChannelRepository>>,
    pub notification_subscriptions: Option<Arc<dyn NotificationSubscriptionRepository>>,
    pub notification_provider: Option<Arc<dyn NotificationPluginProvider>>,
    pub db_path: String,
    pub domain_event_broadcast: broadcast::Sender<i64>,
    pub import_history_broadcast: broadcast::Sender<()>,
    pub settings_changed_broadcast: broadcast::Sender<Vec<String>>,
    pub library_scan_tracker: LibraryScanTracker,
    pub job_run_tracker: JobRunTracker,
    pub acquisition_wake: Arc<tokio::sync::Notify>,
    pub poster_wake: Arc<tokio::sync::Notify>,
    pub banner_wake: Arc<tokio::sync::Notify>,
    pub fanart_wake: Arc<tokio::sync::Notify>,
    pub housekeeping: Arc<dyn HousekeepingRepository>,
    pub health_check_results: Arc<tokio::sync::RwLock<Vec<HealthCheckResult>>>,
    pub pending_releases: Arc<dyn PendingReleaseRepository>,
    pub title_history: Arc<dyn TitleHistoryRepository>,
    pub blocklist_repo: Arc<dyn BlocklistRepository>,
    pub rss_seen_guids: Arc<tokio::sync::RwLock<HashSet<String>>>,
    pub subtitle_downloads: Arc<dyn SubtitleDownloadRepository>,
    pub import_artifacts: Arc<dyn ImportArtifactRepository>,
    pub job_runs: Arc<dyn JobRunRepository>,
    pub library_probe_signatures: Arc<dyn LibraryProbeRepository>,
    pub library_scan_unmatched_items: Arc<dyn LibraryScanUnmatchedItemRepository>,
    pub staged_nzb_store: Arc<dyn StagedNzbStore>,
    pub staged_nzb_pipeline_limit: Arc<Semaphore>,
    pub library_scan_analysis_limit: Arc<Semaphore>,
    pub tracked_download_handle: Option<tracked_downloads::TrackedDownloadHandle>,
}

impl AppServices {
    pub fn with_default_channels(
        titles: Arc<dyn TitleRepository>,
        shows: Arc<dyn ShowRepository>,
        users: Arc<dyn UserRepository>,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        release_attempts: Arc<dyn ReleaseAttemptRepository>,
        settings: Arc<dyn SettingsRepository>,
        quality_profiles: Arc<dyn QualityProfileRepository>,
        db_path: String,
    ) -> Self {
        let (domain_event_tx, _domain_event_rx) = broadcast::channel(256);
        let (import_history_tx, _) = broadcast::channel::<()>(16);
        let (settings_changed_tx, _) = broadcast::channel::<Vec<String>>(16);
        Self {
            titles,
            shows,
            users,
            domain_events: Arc::new(NullDomainEventRepository),
            indexer_configs,
            indexer_client,
            download_client,
            metadata_gateway: Arc::new(crate::library_scan::NullMetadataGateway),
            library_scanner: Arc::new(crate::library_scan::NullLibraryScanner),
            library_renamer: Arc::new(crate::library_rename::NullLibraryRenamer),
            imports: Arc::new(NullImportRepository),
            file_importer: Arc::new(NullFileImporter),
            media_files: Arc::new(NullMediaFileRepository),
            media_analyzer: Arc::new(NativeMediaAnalyzer),
            download_client_configs,
            release_attempts,
            acquisition_state: Arc::new(NullAcquisitionStateRepository),
            download_submissions: Arc::new(NullDownloadSubmissionRepository),
            settings,
            quality_profiles,
            wanted_items: Arc::new(NullWantedItemRepository),
            rule_sets: Arc::new(NullRuleSetRepository),
            pp_scripts: Arc::new(NullPostProcessingScriptRepository),
            plugin_installations: Arc::new(NullPluginInstallationRepository),
            system_info: Arc::new(NullSystemInfoProvider),
            title_images: Arc::new(NullTitleImageRepository),
            title_image_processor: Arc::new(NullTitleImageProcessor),
            indexer_stats: Arc::new(NullIndexerStatsTracker),
            user_rules: Arc::new(std::sync::RwLock::new(
                scryer_rules::UserRulesEngine::empty(),
            )),
            plugin_provider: None,
            download_client_plugin_provider: None,
            notification_channels: None,
            notification_subscriptions: None,
            notification_provider: None,
            db_path,
            domain_event_broadcast: domain_event_tx,
            import_history_broadcast: import_history_tx,
            settings_changed_broadcast: settings_changed_tx,
            library_scan_tracker: LibraryScanTracker::new(),
            job_run_tracker: JobRunTracker::new(),
            acquisition_wake: Arc::new(tokio::sync::Notify::new()),
            poster_wake: Arc::new(tokio::sync::Notify::new()),
            banner_wake: Arc::new(tokio::sync::Notify::new()),
            fanart_wake: Arc::new(tokio::sync::Notify::new()),
            housekeeping: Arc::new(NullHousekeepingRepository),
            pending_releases: Arc::new(NullPendingReleaseRepository),
            title_history: Arc::new(NullTitleHistoryRepository),
            blocklist_repo: Arc::new(NullBlocklistRepository),
            subtitle_downloads: Arc::new(null_repositories::NullSubtitleDownloadRepository),
            health_check_results: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            rss_seen_guids: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
            import_artifacts: Arc::new(null_repositories::NullImportArtifactRepository),
            job_runs: Arc::new(null_repositories::NullJobRunRepository),
            library_probe_signatures: Arc::new(null_repositories::NullLibraryProbeRepository),
            library_scan_unmatched_items: Arc::new(
                null_repositories::NullLibraryScanUnmatchedItemRepository,
            ),
            staged_nzb_store: Arc::new(null_repositories::NullStagedNzbStore),
            staged_nzb_pipeline_limit: Arc::new(Semaphore::new(4)),
            library_scan_analysis_limit: Arc::new(Semaphore::new(
                GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY,
            )),
            tracked_download_handle: None,
        }
    }

    pub async fn append_domain_event(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        let stored = self.domain_events.append(event).await?;
        let _ = self.domain_event_broadcast.send(stored.sequence);
        Ok(stored)
    }

    pub async fn append_domain_events(
        &self,
        events: Vec<NewDomainEvent>,
    ) -> AppResult<Vec<DomainEvent>> {
        let stored = self.domain_events.append_many(events).await?;
        if let Some(last) = stored.last() {
            let _ = self.domain_event_broadcast.send(last.sequence);
        }
        Ok(stored)
    }

    pub async fn update_import_status_and_notify(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.imports
            .update_import_status(import_id, status, result_json.clone())
            .await?;
        if matches!(status, ImportStatus::Completed | ImportStatus::Failed) {
            let _ = self.import_history_broadcast.send(());
        }

        if let Some(ref json) = result_json
            && let Ok(result) = serde_json::from_str::<ImportResult>(json)
            && matches!(status, ImportStatus::Failed | ImportStatus::Skipped)
        {
            let title = match result.title_id.as_ref() {
                Some(title_id) => self
                    .titles
                    .get_by_id(title_id)
                    .await?
                    .map(|title| crate::domain_events::title_context_snapshot(&title)),
                None => None,
            };
            let reason = result
                .error_message
                .clone()
                .or_else(|| result.skip_reason.map(|reason| reason.as_str().to_string()));

            let event = if let Some(title_id) = result.title_id.as_ref() {
                let facet = title.as_ref().map(|snapshot| snapshot.facet.clone());
                NewDomainEvent {
                    event_id: Id::new().0,
                    occurred_at: Utc::now(),
                    actor_user_id: None,
                    title_id: Some(title_id.clone()),
                    facet,
                    correlation_id: None,
                    causation_id: None,
                    schema_version: 1,
                    stream: scryer_domain::DomainEventStream::Title {
                        title_id: title_id.clone(),
                    },
                    payload: scryer_domain::DomainEventPayload::ImportRejected(
                        scryer_domain::ImportRejectedEventData {
                            title,
                            status,
                            source_path: Some(result.source_path.clone()),
                            reason,
                            episode_ids: Vec::new(),
                        },
                    ),
                }
            } else {
                crate::domain_events::new_global_domain_event(
                    None,
                    scryer_domain::DomainEventPayload::ImportRejected(
                        scryer_domain::ImportRejectedEventData {
                            title: None,
                            status,
                            source_path: Some(result.source_path.clone()),
                            reason,
                            episode_ids: Vec::new(),
                        },
                    ),
                )
            };

            let _ = self.append_domain_event(event).await;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct AppUseCase {
    pub services: AppServices,
    pub auth: JwtAuthConfig,
    pub facet_registry: Arc<FacetRegistry>,
    pub(crate) jwt_signing_keys: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    pub(crate) jwt_signing_keys_loaded: Arc<OnceCell<()>>,
    pub(crate) jwt_signing_keys_seed_lock: Arc<Mutex<()>>,
}
