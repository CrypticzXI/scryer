use super::*;

/// In-process guard table for request-signature download dedupe.
///
/// Scryer is intentionally single-instance, so the database lookup remains the
/// authoritative duplicate check while this table serializes same-process races.
#[derive(Clone, Default)]
pub struct DownloadSubmissionGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl DownloadSubmissionGuardTable {
    pub async fn acquire(
        &self,
        title_id: &str,
        request_signature: Option<&str>,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let signature = request_signature?;
        let key = format!("{title_id}:{signature}");
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        Some(lock.lock_owned().await)
    }
}

#[derive(Clone)]
pub struct AppRuntimeEventState {
    pub domain_event_broadcast: broadcast::Sender<i64>,
    /// Wake-only high-water hints for the notification dispatcher. Send-side filtering keeps
    /// operational bursts from waking it, while persisted filtered replay remains authoritative.
    pub notification_event_broadcast: broadcast::Sender<i64>,
    pub import_history_broadcast: broadcast::Sender<()>,
    pub settings_changed_broadcast: broadcast::Sender<Vec<String>>,
}

#[derive(Clone)]
pub struct AppRuntimeCatalogState {
    pub(crate) monitored_title_matcher:
        Arc<RwLock<crate::import_title_resolution::MonitoredTitleMatcherCache>>,
    pub title_hydration_wake: Arc<tokio::sync::Notify>,
    pub poster_wake: Arc<tokio::sync::Notify>,
    pub banner_wake: Arc<tokio::sync::Notify>,
    pub fanart_wake: Arc<tokio::sync::Notify>,
}

#[derive(Clone)]
pub struct AppRuntimeAcquisitionState {
    pub acquisition_wake: Arc<tokio::sync::Notify>,
    pub download_submission_guards: DownloadSubmissionGuardTable,
    pub rss_seen_guids: Arc<tokio::sync::RwLock<HashSet<String>>>,
    pub tracked_download_handle: Option<tracked_downloads::TrackedDownloadHandle>,
}

#[derive(Clone)]
pub struct AppRuntimeLibraryState {
    pub library_scan_tracker: LibraryScanTracker,
    pub library_scan_cancellation_tokens:
        Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    pub library_scan_analysis_limit: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct AppRuntimeJobState {
    pub job_run_tracker: JobRunTracker,
}

#[derive(Clone)]
pub struct AppRuntimeHealthState {
    pub results: Arc<tokio::sync::RwLock<Vec<HealthCheckResult>>>,
}

#[derive(Clone)]
pub struct AppRuntimeState {
    pub events: AppRuntimeEventState,
    pub catalog: AppRuntimeCatalogState,
    pub acquisition: AppRuntimeAcquisitionState,
    pub library: AppRuntimeLibraryState,
    pub jobs: AppRuntimeJobState,
    pub health: AppRuntimeHealthState,
}

impl Default for AppRuntimeState {
    fn default() -> Self {
        let (domain_event_tx, _domain_event_rx) = broadcast::channel(256);
        // Match the main domain-event buffer so short notification bursts can queue wake hints
        // while the dispatcher catches up from persisted offsets.
        let (notification_event_tx, _notification_event_rx) = broadcast::channel(256);
        let (import_history_tx, _) = broadcast::channel::<()>(16);
        let (settings_changed_tx, _) = broadcast::channel::<Vec<String>>(16);

        Self {
            events: AppRuntimeEventState {
                domain_event_broadcast: domain_event_tx,
                notification_event_broadcast: notification_event_tx,
                import_history_broadcast: import_history_tx,
                settings_changed_broadcast: settings_changed_tx,
            },
            catalog: AppRuntimeCatalogState {
                monitored_title_matcher: Arc::new(RwLock::new(
                    crate::import_title_resolution::MonitoredTitleMatcherCache::default(),
                )),
                title_hydration_wake: Arc::new(tokio::sync::Notify::new()),
                poster_wake: Arc::new(tokio::sync::Notify::new()),
                banner_wake: Arc::new(tokio::sync::Notify::new()),
                fanart_wake: Arc::new(tokio::sync::Notify::new()),
            },
            acquisition: AppRuntimeAcquisitionState {
                acquisition_wake: Arc::new(tokio::sync::Notify::new()),
                download_submission_guards: DownloadSubmissionGuardTable::default(),
                rss_seen_guids: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
                tracked_download_handle: None,
            },
            library: AppRuntimeLibraryState {
                library_scan_tracker: LibraryScanTracker::new(),
                library_scan_cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
                library_scan_analysis_limit: Arc::new(Semaphore::new(
                    GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY,
                )),
            },
            jobs: AppRuntimeJobState {
                job_run_tracker: JobRunTracker::new(),
            },
            health: AppRuntimeHealthState {
                results: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            },
        }
    }
}

#[derive(Clone)]
pub struct AppAssembly {
    pub services: AppServices,
    pub runtime: AppRuntimeState,
}

#[derive(Clone)]
pub struct AppCatalogServices {
    pub(crate) titles: Arc<dyn TitleRepository>,
    pub(crate) shows: Arc<dyn ShowRepository>,
}

#[derive(Clone)]
pub struct AppIdentityServices {
    pub(crate) users: Arc<dyn UserRepository>,
}

#[derive(Clone)]
pub struct AppEventServices {
    pub(crate) domain_events: Arc<dyn DomainEventRepository>,
    pub(crate) job_runs: Arc<dyn JobRunRepository>,
}

#[derive(Clone, Default)]
pub enum RuntimeFeature<T> {
    #[default]
    Disabled,
    Enabled(T),
}

impl<T> RuntimeFeature<T> {
    pub fn enabled(value: T) -> Self {
        Self::Enabled(value)
    }

    pub fn available(&self) -> Option<&T> {
        match self {
            Self::Disabled => None,
            Self::Enabled(value) => Some(value),
        }
    }
}

#[derive(Clone)]
pub struct AppLibraryServices {
    pub(crate) metadata_gateway: Arc<dyn MetadataGateway>,
    pub(crate) library_scanner: Arc<dyn LibraryScanner>,
    pub(crate) library_renamer: Arc<dyn LibraryRenamer>,
    pub(crate) media_files: Arc<dyn MediaFileRepository>,
    pub(crate) media_analyzer: Arc<dyn MediaAnalyzer>,
    pub(crate) title_images: Arc<dyn TitleImageRepository>,
    pub(crate) title_image_processor: Arc<dyn TitleImageProcessor>,
    pub(crate) library_probe_signatures: Arc<dyn LibraryProbeRepository>,
    pub(crate) library_scan_unmatched_items: Arc<dyn LibraryScanUnmatchedItemRepository>,
}

#[derive(Clone)]
pub struct AppIntegrationServices {
    pub(crate) indexer_configs: Arc<dyn IndexerConfigRepository>,
    pub(crate) indexer_client: Arc<dyn IndexerClient>,
    pub(crate) download_client: Arc<dyn DownloadClient>,
    pub(crate) download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    pub(crate) indexer_stats: Arc<dyn IndexerStatsTracker>,
    pub(crate) plugin_provider: RuntimeFeature<Arc<dyn IndexerPluginProvider>>,
    pub(crate) download_client_plugin_provider:
        RuntimeFeature<Arc<dyn DownloadClientPluginProvider>>,
}

#[derive(Clone)]
pub struct AppWorkflowServices {
    pub(crate) imports: Arc<dyn ImportRepository>,
    pub(crate) external_import_monitor_snapshots: Arc<dyn ExternalImportMonitorSnapshotRepository>,
    pub(crate) download_queue_commands: Arc<dyn DownloadQueueCommandRepository>,
    pub(crate) workflow_operations: Arc<dyn WorkflowOperationRepository>,
    pub(crate) file_importer: Arc<dyn FileImporter>,
    pub(crate) import_artifacts: Arc<dyn ImportArtifactRepository>,
    pub(crate) release_attempts: Arc<dyn ReleaseAttemptRepository>,
    pub(crate) acquisition_state: Arc<dyn AcquisitionStateRepository>,
    pub(crate) download_submissions: Arc<dyn DownloadSubmissionRepository>,
    pub(crate) wanted_items: Arc<dyn WantedItemRepository>,
    pub(crate) housekeeping: Arc<dyn HousekeepingRepository>,
    pub(crate) pending_releases: Arc<dyn PendingReleaseRepository>,
    pub(crate) title_history: Arc<dyn TitleHistoryRepository>,
    pub(crate) blocklist_repo: Arc<dyn BlocklistRepository>,
    pub(crate) subtitle_downloads: Arc<dyn SubtitleDownloadRepository>,
    pub(crate) staged_nzb_store: Arc<dyn StagedNzbStore>,
    pub(crate) staged_nzb_pipeline_limit: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct AppConfigServices {
    pub(crate) settings: Arc<dyn SettingsRepository>,
    pub(crate) quality_profiles: Arc<dyn QualityProfileRepository>,
    pub(crate) system_info: Arc<dyn SystemInfoProvider>,
    pub(crate) db_path: String,
}

#[derive(Clone)]
pub struct AppCustomizationServices {
    pub(crate) rule_sets: Arc<dyn RuleSetRepository>,
    pub(crate) pp_scripts: Arc<dyn PostProcessingScriptRepository>,
    pub(crate) plugin_installations: Arc<dyn PluginInstallationRepository>,
    pub(crate) user_rules: Arc<std::sync::RwLock<scryer_rules::UserRulesEngine>>,
}

#[derive(Clone)]
pub enum AppNotificationServices {
    Disabled,
    Store {
        notification_channels: Arc<dyn NotificationChannelRepository>,
        notification_subscriptions: Arc<dyn NotificationSubscriptionRepository>,
    },
    Provider {
        notification_provider: Arc<dyn NotificationPluginProvider>,
    },
    Runtime {
        notification_channels: Arc<dyn NotificationChannelRepository>,
        notification_subscriptions: Arc<dyn NotificationSubscriptionRepository>,
        notification_provider: Arc<dyn NotificationPluginProvider>,
    },
}

impl AppNotificationServices {
    pub fn notification_channels(&self) -> Option<&Arc<dyn NotificationChannelRepository>> {
        match self {
            Self::Store {
                notification_channels,
                ..
            }
            | Self::Runtime {
                notification_channels,
                ..
            } => Some(notification_channels),
            Self::Disabled | Self::Provider { .. } => None,
        }
    }

    pub fn notification_subscriptions(
        &self,
    ) -> Option<&Arc<dyn NotificationSubscriptionRepository>> {
        match self {
            Self::Store {
                notification_subscriptions,
                ..
            }
            | Self::Runtime {
                notification_subscriptions,
                ..
            } => Some(notification_subscriptions),
            Self::Disabled | Self::Provider { .. } => None,
        }
    }

    pub fn notification_provider(&self) -> Option<&Arc<dyn NotificationPluginProvider>> {
        match self {
            Self::Provider {
                notification_provider,
            }
            | Self::Runtime {
                notification_provider,
                ..
            } => Some(notification_provider),
            Self::Disabled | Self::Store { .. } => None,
        }
    }
}

#[derive(Clone)]
pub struct AppServices {
    pub(crate) catalog: AppCatalogServices,
    pub(crate) identity: AppIdentityServices,
    pub(crate) events: AppEventServices,
    pub(crate) library: AppLibraryServices,
    pub(crate) integrations: AppIntegrationServices,
    pub(crate) workflow: AppWorkflowServices,
    pub(crate) config: AppConfigServices,
    pub(crate) customization: AppCustomizationServices,
    pub(crate) notifications: AppNotificationServices,
}

impl AppServices {
    pub fn builder(
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
    ) -> AppServicesBuilder {
        AppServicesBuilder {
            services: Self::with_placeholder_defaults(
                titles,
                shows,
                users,
                indexer_configs,
                indexer_client,
                download_client,
                download_client_configs,
                release_attempts,
                settings,
                quality_profiles,
                db_path,
            ),
            runtime: AppRuntimeState::default(),
            configured: AppServicesBuildConfiguration::default(),
        }
    }

    fn with_placeholder_defaults(
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
        Self {
            catalog: AppCatalogServices { titles, shows },
            identity: AppIdentityServices { users },
            events: AppEventServices {
                domain_events: Arc::new(NullDomainEventRepository),
                job_runs: Arc::new(null_repositories::NullJobRunRepository),
            },
            library: AppLibraryServices {
                metadata_gateway: Arc::new(crate::library_scan::NullMetadataGateway),
                library_scanner: Arc::new(crate::library_scan::NullLibraryScanner),
                library_renamer: Arc::new(crate::library_rename::NullLibraryRenamer),
                media_files: Arc::new(NullMediaFileRepository),
                media_analyzer: Arc::new(NativeMediaAnalyzer),
                title_images: Arc::new(NullTitleImageRepository),
                title_image_processor: Arc::new(NullTitleImageProcessor),
                library_probe_signatures: Arc::new(null_repositories::NullLibraryProbeRepository),
                library_scan_unmatched_items: Arc::new(
                    null_repositories::NullLibraryScanUnmatchedItemRepository,
                ),
            },
            integrations: AppIntegrationServices {
                indexer_configs,
                indexer_client,
                download_client,
                download_client_configs,
                indexer_stats: Arc::new(NullIndexerStatsTracker),
                plugin_provider: RuntimeFeature::Disabled,
                download_client_plugin_provider: RuntimeFeature::Disabled,
            },
            workflow: AppWorkflowServices {
                imports: Arc::new(NullImportRepository),
                external_import_monitor_snapshots: Arc::new(
                    null_repositories::NullExternalImportMonitorSnapshotRepository,
                ),
                download_queue_commands: Arc::new(
                    null_repositories::NullDownloadQueueCommandRepository,
                ),
                workflow_operations: Arc::new(NullWorkflowOperationRepository),
                file_importer: Arc::new(NullFileImporter),
                import_artifacts: Arc::new(null_repositories::NullImportArtifactRepository),
                release_attempts,
                acquisition_state: Arc::new(NullAcquisitionStateRepository),
                download_submissions: Arc::new(NullDownloadSubmissionRepository),
                wanted_items: Arc::new(NullWantedItemRepository),
                housekeeping: Arc::new(NullHousekeepingRepository),
                pending_releases: Arc::new(NullPendingReleaseRepository),
                title_history: Arc::new(NullTitleHistoryRepository),
                blocklist_repo: Arc::new(NullBlocklistRepository),
                subtitle_downloads: Arc::new(null_repositories::NullSubtitleDownloadRepository),
                staged_nzb_store: Arc::new(null_repositories::NullStagedNzbStore),
                staged_nzb_pipeline_limit: Arc::new(Semaphore::new(4)),
            },
            config: AppConfigServices {
                settings,
                quality_profiles,
                system_info: Arc::new(NullSystemInfoProvider),
                db_path,
            },
            customization: AppCustomizationServices {
                rule_sets: Arc::new(NullRuleSetRepository),
                pp_scripts: Arc::new(NullPostProcessingScriptRepository),
                plugin_installations: Arc::new(NullPluginInstallationRepository),
                user_rules: Arc::new(std::sync::RwLock::new(
                    scryer_rules::UserRulesEngine::empty(),
                )),
            },
            notifications: AppNotificationServices::Disabled,
        }
    }
}

macro_rules! app_services_builder_setter {
    ($name:ident, $($field:ident).+, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = value;
            self
        }
    };
}

macro_rules! app_services_builder_required_setter {
    ($name:ident, $($field:ident).+, $config_field:ident, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = value;
            self.configured.$config_field = true;
            self
        }
    };
}

macro_rules! app_services_builder_runtime_feature_setter {
    ($name:ident, $($field:ident).+, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = RuntimeFeature::enabled(value);
            self
        }
    };
}

pub struct AppServicesBuilder {
    services: AppServices,
    runtime: AppRuntimeState,
    configured: AppServicesBuildConfiguration,
}

#[derive(Default)]
struct AppServicesBuildConfiguration {
    domain_events: bool,
    metadata_gateway: bool,
    library_scanner: bool,
    imports: bool,
    workflow_operations: bool,
    import_artifacts: bool,
    media_files: bool,
    acquisition_state: bool,
    download_submissions: bool,
    wanted_items: bool,
    rule_sets: bool,
    pp_scripts: bool,
    plugin_installations: bool,
    system_info: bool,
    title_images: bool,
    housekeeping: bool,
    pending_releases: bool,
    title_history: bool,
    blocklist_repo: bool,
    subtitle_downloads: bool,
    job_runs: bool,
    library_probe_signatures: bool,
    library_scan_unmatched_items: bool,
}

impl AppServicesBuildConfiguration {
    fn missing_runtime_services(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();

        if !self.domain_events {
            missing.push("domain_events");
        }
        if !self.metadata_gateway {
            missing.push("metadata_gateway");
        }
        if !self.library_scanner {
            missing.push("library_scanner");
        }
        if !self.imports {
            missing.push("imports");
        }
        if !self.workflow_operations {
            missing.push("workflow_operations");
        }
        if !self.import_artifacts {
            missing.push("import_artifacts");
        }
        if !self.media_files {
            missing.push("media_files");
        }
        if !self.acquisition_state {
            missing.push("acquisition_state");
        }
        if !self.download_submissions {
            missing.push("download_submissions");
        }
        if !self.wanted_items {
            missing.push("wanted_items");
        }
        if !self.rule_sets {
            missing.push("rule_sets");
        }
        if !self.pp_scripts {
            missing.push("pp_scripts");
        }
        if !self.plugin_installations {
            missing.push("plugin_installations");
        }
        if !self.system_info {
            missing.push("system_info");
        }
        if !self.title_images {
            missing.push("title_images");
        }
        if !self.housekeeping {
            missing.push("housekeeping");
        }
        if !self.pending_releases {
            missing.push("pending_releases");
        }
        if !self.title_history {
            missing.push("title_history");
        }
        if !self.blocklist_repo {
            missing.push("blocklist_repo");
        }
        if !self.subtitle_downloads {
            missing.push("subtitle_downloads");
        }
        if !self.job_runs {
            missing.push("job_runs");
        }
        if !self.library_probe_signatures {
            missing.push("library_probe_signatures");
        }
        if !self.library_scan_unmatched_items {
            missing.push("library_scan_unmatched_items");
        }

        missing
    }
}

impl AppServicesBuilder {
    app_services_builder_setter!(with_shows, catalog.shows, Arc<dyn ShowRepository>);
    pub fn with_library_state_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: BlocklistRepository
            + HousekeepingRepository
            + LibraryProbeRepository
            + LibraryScanUnmatchedItemRepository
            + MediaFileRepository
            + PendingReleaseRepository
            + SubtitleDownloadRepository
            + TitleHistoryRepository
            + TitleImageRepository
            + WantedItemRepository
            + Send
            + Sync
            + 'static,
    {
        self.services.library.media_files = store.clone();
        self.services.workflow.wanted_items = store.clone();
        self.services.workflow.pending_releases = store.clone();
        self.services.workflow.title_history = store.clone();
        self.services.workflow.blocklist_repo = store.clone();
        self.services.library.library_probe_signatures = store.clone();
        self.services.library.library_scan_unmatched_items = store.clone();
        self.services.library.title_images = store.clone();
        self.services.workflow.housekeeping = store.clone();
        self.services.workflow.subtitle_downloads = store;
        self.configured.media_files = true;
        self.configured.wanted_items = true;
        self.configured.pending_releases = true;
        self.configured.title_history = true;
        self.configured.blocklist_repo = true;
        self.configured.library_probe_signatures = true;
        self.configured.library_scan_unmatched_items = true;
        self.configured.title_images = true;
        self.configured.housekeeping = true;
        self.configured.subtitle_downloads = true;
        self
    }

    pub fn with_customization_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: PluginInstallationRepository
            + PostProcessingScriptRepository
            + RuleSetRepository
            + Send
            + Sync
            + 'static,
    {
        self.services.customization.rule_sets = store.clone();
        self.services.customization.pp_scripts = store.clone();
        self.services.customization.plugin_installations = store;
        self.configured.rule_sets = true;
        self.configured.pp_scripts = true;
        self.configured.plugin_installations = true;
        self
    }

    pub fn with_notification_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: NotificationChannelRepository
            + NotificationSubscriptionRepository
            + Send
            + Sync
            + 'static,
    {
        let notification_channels: Arc<dyn NotificationChannelRepository> = store.clone();
        let notification_subscriptions: Arc<dyn NotificationSubscriptionRepository> = store;
        self.services.notifications = match self.services.notifications {
            AppNotificationServices::Disabled | AppNotificationServices::Store { .. } => {
                AppNotificationServices::Store {
                    notification_channels,
                    notification_subscriptions,
                }
            }
            AppNotificationServices::Provider {
                notification_provider,
            }
            | AppNotificationServices::Runtime {
                notification_provider,
                ..
            } => AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                notification_provider,
            },
        };
        self
    }

    app_services_builder_required_setter!(
        with_metadata_gateway,
        library.metadata_gateway,
        metadata_gateway,
        Arc<dyn MetadataGateway>
    );
    app_services_builder_required_setter!(
        with_library_scanner,
        library.library_scanner,
        library_scanner,
        Arc<dyn LibraryScanner>
    );
    app_services_builder_setter!(
        with_library_renamer,
        library.library_renamer,
        Arc<dyn LibraryRenamer>
    );
    app_services_builder_required_setter!(
        with_domain_events,
        events.domain_events,
        domain_events,
        Arc<dyn DomainEventRepository>
    );
    app_services_builder_required_setter!(
        with_imports,
        workflow.imports,
        imports,
        Arc<dyn ImportRepository>
    );
    app_services_builder_setter!(
        with_external_import_monitor_snapshots,
        workflow.external_import_monitor_snapshots,
        Arc<dyn ExternalImportMonitorSnapshotRepository>
    );
    app_services_builder_setter!(
        with_download_queue_commands,
        workflow.download_queue_commands,
        Arc<dyn DownloadQueueCommandRepository>
    );
    app_services_builder_required_setter!(
        with_workflow_operations,
        workflow.workflow_operations,
        workflow_operations,
        Arc<dyn WorkflowOperationRepository>
    );
    app_services_builder_required_setter!(
        with_import_artifacts,
        workflow.import_artifacts,
        import_artifacts,
        Arc<dyn ImportArtifactRepository>
    );
    app_services_builder_setter!(
        with_file_importer,
        workflow.file_importer,
        Arc<dyn FileImporter>
    );
    app_services_builder_required_setter!(
        with_media_files,
        library.media_files,
        media_files,
        Arc<dyn MediaFileRepository>
    );
    app_services_builder_required_setter!(
        with_download_submissions,
        workflow.download_submissions,
        download_submissions,
        Arc<dyn DownloadSubmissionRepository>
    );
    app_services_builder_required_setter!(
        with_acquisition_state,
        workflow.acquisition_state,
        acquisition_state,
        Arc<dyn AcquisitionStateRepository>
    );
    app_services_builder_required_setter!(
        with_wanted_items,
        workflow.wanted_items,
        wanted_items,
        Arc<dyn WantedItemRepository>
    );
    app_services_builder_required_setter!(
        with_pending_releases,
        workflow.pending_releases,
        pending_releases,
        Arc<dyn PendingReleaseRepository>
    );
    app_services_builder_required_setter!(
        with_title_history,
        workflow.title_history,
        title_history,
        Arc<dyn TitleHistoryRepository>
    );
    app_services_builder_required_setter!(
        with_blocklist_repo,
        workflow.blocklist_repo,
        blocklist_repo,
        Arc<dyn BlocklistRepository>
    );
    app_services_builder_required_setter!(
        with_rule_sets,
        customization.rule_sets,
        rule_sets,
        Arc<dyn RuleSetRepository>
    );
    app_services_builder_required_setter!(
        with_pp_scripts,
        customization.pp_scripts,
        pp_scripts,
        Arc<dyn PostProcessingScriptRepository>
    );
    app_services_builder_required_setter!(
        with_plugin_installations,
        customization.plugin_installations,
        plugin_installations,
        Arc<dyn PluginInstallationRepository>
    );
    app_services_builder_required_setter!(
        with_system_info,
        config.system_info,
        system_info,
        Arc<dyn SystemInfoProvider>
    );
    app_services_builder_required_setter!(
        with_job_runs,
        events.job_runs,
        job_runs,
        Arc<dyn JobRunRepository>
    );
    app_services_builder_required_setter!(
        with_library_probe_signatures,
        library.library_probe_signatures,
        library_probe_signatures,
        Arc<dyn LibraryProbeRepository>
    );
    app_services_builder_required_setter!(
        with_library_scan_unmatched_items,
        library.library_scan_unmatched_items,
        library_scan_unmatched_items,
        Arc<dyn LibraryScanUnmatchedItemRepository>
    );
    app_services_builder_required_setter!(
        with_title_images,
        library.title_images,
        title_images,
        Arc<dyn TitleImageRepository>
    );
    app_services_builder_setter!(
        with_title_image_processor,
        library.title_image_processor,
        Arc<dyn TitleImageProcessor>
    );
    app_services_builder_required_setter!(
        with_housekeeping,
        workflow.housekeeping,
        housekeeping,
        Arc<dyn HousekeepingRepository>
    );
    app_services_builder_required_setter!(
        with_subtitle_downloads,
        workflow.subtitle_downloads,
        subtitle_downloads,
        Arc<dyn SubtitleDownloadRepository>
    );
    app_services_builder_setter!(
        with_staged_nzb_store,
        workflow.staged_nzb_store,
        Arc<dyn StagedNzbStore>
    );
    app_services_builder_setter!(
        with_staged_nzb_pipeline_limit,
        workflow.staged_nzb_pipeline_limit,
        Arc<Semaphore>
    );
    app_services_builder_setter!(
        with_indexer_stats,
        integrations.indexer_stats,
        Arc<dyn IndexerStatsTracker>
    );
    app_services_builder_runtime_feature_setter!(
        with_plugin_provider,
        integrations.plugin_provider,
        Arc<dyn IndexerPluginProvider>
    );
    app_services_builder_runtime_feature_setter!(
        with_download_client_plugin_provider,
        integrations.download_client_plugin_provider,
        Arc<dyn DownloadClientPluginProvider>
    );
    pub fn with_notification_provider(
        mut self,
        value: Arc<dyn NotificationPluginProvider>,
    ) -> Self {
        self.services.notifications = match self.services.notifications {
            AppNotificationServices::Disabled | AppNotificationServices::Provider { .. } => {
                AppNotificationServices::Provider {
                    notification_provider: value,
                }
            }
            AppNotificationServices::Store {
                notification_channels,
                notification_subscriptions,
            }
            | AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                ..
            } => AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                notification_provider: value,
            },
        };
        self
    }
    pub fn with_tracked_download_handle(
        mut self,
        value: tracked_downloads::TrackedDownloadHandle,
    ) -> Self {
        self.runtime.acquisition.tracked_download_handle = Some(value);
        self
    }

    pub fn build(self) -> AppAssembly {
        let missing = self.configured.missing_runtime_services();
        assert!(
            missing.is_empty(),
            "AppServicesBuilder missing required runtime services: {}. Use build_partial_for_tests() only for intentionally partial test assemblies.",
            missing.join(", ")
        );
        self.finish()
    }

    fn finish(self) -> AppAssembly {
        AppAssembly {
            services: self.services,
            runtime: self.runtime,
        }
    }

    pub(crate) fn build_partial_for_tests(self) -> AppAssembly {
        self.finish()
    }
}

#[derive(Clone)]
pub struct AppUseCase {
    pub(crate) services: AppServices,
    pub(crate) runtime: AppRuntimeState,
    pub auth: JwtAuthConfig,
    pub facet_registry: Arc<FacetRegistry>,
    pub(crate) pending_import_resolution_locks: Arc<std::sync::Mutex<HashSet<String>>>,
    pub(crate) jwt_signing_keys: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    pub(crate) jwt_signing_keys_loaded: Arc<OnceCell<()>>,
    pub(crate) jwt_signing_keys_seed_lock: Arc<Mutex<()>>,
}

impl AppUseCase {
    async fn invalidate_monitored_title_matcher(&self) {
        let mut state = self.runtime.catalog.monitored_title_matcher.write().await;
        state.dirty = true;
    }

    pub(crate) async fn monitored_title_matcher(
        &self,
    ) -> AppResult<Arc<crate::import_title_resolution::MonitoredTitleMatcher>> {
        {
            let state = self.runtime.catalog.monitored_title_matcher.read().await;
            if !state.dirty
                && let Some(matcher) = state.matcher.clone()
            {
                return Ok(matcher);
            }
        }

        let titles = self
            .services
            .catalog
            .titles
            .list_for_matching(None, None)
            .await?;
        let matcher = Arc::new(crate::import_title_resolution::MonitoredTitleMatcher::new(
            titles,
        ));

        let mut state = self.runtime.catalog.monitored_title_matcher.write().await;
        if state.dirty || state.matcher.is_none() {
            state.matcher = Some(matcher.clone());
            state.dirty = false;
            return Ok(matcher);
        }

        Ok(state.matcher.clone().unwrap_or(matcher))
    }

    /// Test-only escape hatch for selectively overriding already-assembled services.
    ///
    /// Production assembly should go through `AppServices::builder(...).build()`.
    pub(crate) fn with_test_overrides<F>(&self, configure: F) -> Self
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder,
    {
        let assembly = configure(AppServicesBuilder {
            services: self.services.clone(),
            runtime: self.runtime.clone(),
            configured: AppServicesBuildConfiguration::default(),
        })
        .build_partial_for_tests();
        Self {
            services: assembly.services,
            runtime: assembly.runtime,
            auth: self.auth.clone(),
            facet_registry: self.facet_registry.clone(),
            pending_import_resolution_locks: self.pending_import_resolution_locks.clone(),
            jwt_signing_keys: self.jwt_signing_keys.clone(),
            jwt_signing_keys_loaded: self.jwt_signing_keys_loaded.clone(),
            jwt_signing_keys_seed_lock: self.jwt_signing_keys_seed_lock.clone(),
        }
    }

    pub async fn append_domain_event(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        let stored = self.services.events.domain_events.append(event).await?;
        if should_invalidate_monitored_title_matcher(&stored.payload) {
            self.invalidate_monitored_title_matcher().await;
        }
        let _ = self
            .runtime
            .events
            .domain_event_broadcast
            .send(stored.sequence);
        if crate::notifications::dispatcher::notification_event_type(&stored.payload).is_some() {
            tracing::debug!(
                sequence = stored.sequence,
                event_type = stored.payload.event_type().as_str(),
                "queued notification dispatcher wake for notification-relevant domain event"
            );
            let _ = self
                .runtime
                .events
                .notification_event_broadcast
                .send(stored.sequence);
        }
        Ok(stored)
    }

    pub async fn append_domain_events(
        &self,
        events: Vec<NewDomainEvent>,
    ) -> AppResult<Vec<DomainEvent>> {
        let stored = self
            .services
            .events
            .domain_events
            .append_many(events)
            .await?;
        if stored
            .iter()
            .any(|event| should_invalidate_monitored_title_matcher(&event.payload))
        {
            self.invalidate_monitored_title_matcher().await;
        }
        if let Some(last) = stored.last() {
            let _ = self
                .runtime
                .events
                .domain_event_broadcast
                .send(last.sequence);
        }
        let notification_count = stored
            .iter()
            .filter(|event| {
                crate::notifications::dispatcher::notification_event_type(&event.payload).is_some()
            })
            .count();
        if notification_count > 0
            && let Some(last) = stored.last()
        {
            tracing::debug!(
                high_water_sequence = last.sequence,
                batch_len = stored.len(),
                notification_events = notification_count,
                "queued notification dispatcher wake for notification-relevant domain event batch"
            );
            let _ = self
                .runtime
                .events
                .notification_event_broadcast
                .send(last.sequence);
        }
        Ok(stored)
    }

    pub async fn update_import_status_and_notify(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.services
            .workflow
            .imports
            .update_import_status(import_id, status, result_json.clone())
            .await?;
        if matches!(status, ImportStatus::Completed | ImportStatus::Failed) {
            let _ = self.runtime.events.import_history_broadcast.send(());
        }

        if let Some(ref json) = result_json
            && let Ok(result) = serde_json::from_str::<ImportResult>(json)
            && matches!(status, ImportStatus::Failed | ImportStatus::Skipped)
        {
            let title = match result.title_id.as_ref() {
                Some(title_id) => self
                    .services
                    .catalog
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

    pub fn publish_settings_changed(&self, changed_keys: Vec<String>) {
        let _ = self
            .runtime
            .events
            .settings_changed_broadcast
            .send(changed_keys);
    }

    pub fn indexer_query_stats(&self, actor: &User) -> AppResult<Vec<IndexerQueryStats>> {
        require(actor, &Entitlement::ManageConfig)?;
        Ok(self.services.integrations.indexer_stats.all_stats())
    }

    pub async fn cached_health_check_results(
        &self,
        actor: &User,
    ) -> AppResult<Vec<HealthCheckResult>> {
        require(actor, &Entitlement::ManageConfig)?;
        Ok(self.runtime.health.results.read().await.clone())
    }

    pub async fn list_import_history(
        &self,
        actor: &User,
        limit: usize,
    ) -> AppResult<Vec<ImportRecord>> {
        require(actor, &Entitlement::ViewHistory)?;
        self.services.workflow.imports.list_imports(limit).await
    }

    pub async fn find_download_submission_by_client_item_id(
        &self,
        actor: &User,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services
            .workflow
            .download_submissions
            .find_by_client_item_id(client_type, download_client_item_id)
            .await
    }

    pub async fn search_metadata(
        &self,
        actor: &User,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb_rich(query, type_hint, limit, language, year)
            .await
    }

    pub async fn search_metadata_tvdb(
        &self,
        actor: &User,
        query: &str,
        type_hint: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb(query, type_hint, year)
            .await
    }

    pub async fn search_metadata_batch(
        &self,
        actor: &User,
        queries: &[MetadataSearchQuery],
        language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb_batch(queries, language)
            .await
    }

    pub async fn search_metadata_multi(
        &self,
        actor: &User,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb_multi(query, limit, language)
            .await
    }

    pub async fn get_metadata_movie(
        &self,
        actor: &User,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<MovieMetadata> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .metadata_gateway
            .get_movie(tvdb_id, language)
            .await
    }

    pub async fn get_metadata_series(
        &self,
        actor: &User,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<SeriesMetadata> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .metadata_gateway
            .get_series(tvdb_id, language)
            .await
    }

    pub async fn list_title_media_files(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<TitleMediaFile>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await
    }

    pub async fn get_title_wanted_item(
        &self,
        actor: &User,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .workflow
            .wanted_items
            .get_wanted_item_for_title(title_id, episode_id)
            .await
    }

    pub async fn get_title_for_management(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Option<Title>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services.catalog.titles.get_by_id(title_id).await
    }

    pub async fn get_wanted_item_for_management(
        &self,
        actor: &User,
        wanted_item_id: &str,
    ) -> AppResult<Option<WantedItem>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await
    }

    pub async fn get_title_for_trigger_actions(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Option<Title>> {
        require(actor, &Entitlement::TriggerActions)?;
        self.services.catalog.titles.get_by_id(title_id).await
    }

    pub async fn get_title_tags_for_update(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<String>> {
        require(actor, &Entitlement::ManageTitle)?;
        self.services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .map(|title| title.tags)
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))
    }

    pub async fn get_completed_download(
        &self,
        actor: &User,
        download_client_item_id: &str,
    ) -> AppResult<CompletedDownload> {
        require(actor, &Entitlement::TriggerActions)?;
        let download_client_item_id = download_client_item_id.trim();
        if download_client_item_id.is_empty() {
            return Err(AppError::Validation(
                "download client item id is required".into(),
            ));
        }

        self.services
            .integrations
            .download_client
            .list_completed_downloads()
            .await?
            .into_iter()
            .find(|download| download.download_client_item_id == download_client_item_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("completed download {download_client_item_id}"))
            })
    }

    pub async fn connect_library_scan_tracker(&self) {
        self.runtime
            .library
            .library_scan_tracker
            .set_job_run_tracker(self.runtime.jobs.job_run_tracker.clone())
            .await;
    }

    pub fn wake_title_image_loops(&self) {
        self.runtime.catalog.poster_wake.notify_one();
        self.runtime.catalog.banner_wake.notify_one();
        self.runtime.catalog.fanart_wake.notify_one();
    }

    pub async fn primary_enabled_download_client_config(
        &self,
    ) -> AppResult<Option<DownloadClientConfig>> {
        Ok(self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .min_by_key(|config| config.client_priority))
    }

    pub async fn active_library_scan_sessions(&self) -> Vec<LibraryScanSession> {
        self.runtime
            .library
            .library_scan_tracker
            .list_active()
            .await
    }

    pub fn user_rules_engine_snapshot(&self) -> scryer_rules::UserRulesEngine {
        self.services
            .customization
            .user_rules
            .read()
            .unwrap()
            .clone()
    }
}

fn should_invalidate_monitored_title_matcher(payload: &scryer_domain::DomainEventPayload) -> bool {
    matches!(
        payload,
        scryer_domain::DomainEventPayload::TitleAdded(_)
            | scryer_domain::DomainEventPayload::TitleUpdated(_)
            | scryer_domain::DomainEventPayload::TitleDeleted(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use async_trait::async_trait;
    use scryer_domain::IndexerConfig;

    struct TestIndexerConfigRepository;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(Vec::new())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(&self, _update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    fn test_builder() -> AppServicesBuilder {
        AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepository),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
    }

    #[test]
    #[should_panic(expected = "AppServicesBuilder missing required runtime services")]
    fn build_requires_explicit_runtime_dependencies() {
        let _ = test_builder().build();
    }

    #[test]
    fn build_partial_for_tests_allows_partial_test_assemblies() {
        let _ = test_builder().build_partial_for_tests();
    }
}
