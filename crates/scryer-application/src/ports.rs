use super::*;
use async_trait::async_trait;
use scryer_domain::ImportType;

#[async_trait]
pub trait TitleRepository: Send + Sync {
    async fn list(&self, facet: Option<MediaFacet>, query: Option<String>)
    -> AppResult<Vec<Title>>;
    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>>;
    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>>;
    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>>;
    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>>;
    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>>;
    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome>;
    async fn create(&self, title: Title) -> AppResult<Title>;
    async fn list_titles_due_for_hydration(
        &self,
        limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>>;
    async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()>;
    async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()>;
    async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()>;
    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title>;
    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title>;
    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title>;
    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title>;
    async fn delete(&self, id: &str) -> AppResult<()>;
    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()>;
    async fn clear_folder_path(&self, id: &str) -> AppResult<()>;
    async fn clear_metadata_language_for_all(&self) -> AppResult<u64>;
}

#[async_trait]
pub trait TitleImageRepository: Send + Sync {
    async fn list_titles_requiring_image_refresh(
        &self,
        kind: TitleImageKind,
        limit: usize,
    ) -> AppResult<Vec<TitleImageSyncTask>>;

    async fn replace_title_image(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
    ) -> AppResult<()>;

    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>>;
}

#[async_trait]
pub trait TitleImageProcessor: Send + Sync {
    async fn fetch_and_process_image(
        &self,
        kind: TitleImageKind,
        source_url: &str,
    ) -> AppResult<TitleImageReplacement>;
}

#[async_trait]
pub trait ShowRepository: Send + Sync {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>>;
    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>>;
    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>>;
    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>>;
    async fn create_collection(&self, collection: Collection) -> AppResult<Collection>;
    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection>;
    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection>;
    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection>;
    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()>;
    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()>;
    async fn delete_collection(&self, collection_id: &str) -> AppResult<()>;
    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>>;
    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>>;
    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>>;
    async fn create_episode(&self, episode: Episode) -> AppResult<Episode>;
    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode>;
    async fn delete_episode(&self, episode_id: &str) -> AppResult<()>;
    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>>;
    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>>;
    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>>;
    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>>;
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>>;
    async fn create(&self, user: User) -> AppResult<User>;
    async fn list_all(&self) -> AppResult<Vec<User>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>>;
    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User>;
    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait DomainEventRepository: Send + Sync {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent>;
    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>>;
    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>>;
    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>>;
    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64>;
    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()>;
}

#[async_trait]
pub trait IndexerConfigRepository: Send + Sync {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>>;
    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig>;
    async fn touch_last_error(&self, provider_type: &str) -> AppResult<()>;
    async fn update(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadClientConfigRepository: Send + Sync {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>>;
    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig>;
    async fn update(&self, update: DownloadClientConfigUpdate) -> AppResult<DownloadClientConfig>;
    async fn delete(&self, id: &str) -> AppResult<()>;
    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()>;
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>>;

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait SystemInfoProvider: Send + Sync {
    async fn current_migration_version(&self) -> AppResult<Option<String>>;
    async fn pending_migration_count(&self) -> AppResult<usize>;
    async fn smg_cert_expires_at(&self) -> AppResult<Option<String>>;
    async fn vacuum_into(&self, dest_path: &str) -> AppResult<()>;
}

#[async_trait]
pub trait HousekeepingRepository: Send + Sync {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32>;
    async fn delete_title_history_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_terminal_download_queue_commands_older_than(&self, days: i64)
    -> AppResult<u32>;
    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32>;
    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>>;
    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32>;
}

pub trait IndexerStatsTracker: Send + Sync {
    fn record_query(&self, indexer_id: &str, indexer_name: &str, success: bool);
    fn record_api_limits(
        &self,
        indexer_id: &str,
        api_current: Option<u32>,
        api_max: Option<u32>,
        grab_current: Option<u32>,
        grab_max: Option<u32>,
    );
    fn all_stats(&self) -> Vec<IndexerQueryStats>;

    fn is_at_quota(&self, indexer_id: &str) -> bool {
        self.all_stats()
            .iter()
            .find(|s| s.indexer_id == indexer_id)
            .map(|s| match (s.api_current, s.api_max) {
                (Some(c), Some(m)) if m > 0 => c >= m * 95 / 100,
                _ => false,
            })
            .unwrap_or(false)
    }
}

#[async_trait]
pub trait QualityProfileRepository: Send + Sync {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>>;
    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait ReleaseAttemptRepository: Send + Sync {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()>;

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>>;

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>>;

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait AcquisitionStateRepository: Send + Sync {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadSubmissionRepository: Send + Sync {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()>;

    async fn find_by_client_item_id(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadSubmission>>;

    async fn list_for_client_items(
        &self,
        client_items: &[(String, String)],
    ) -> AppResult<Vec<DownloadSubmission>>;

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>>;
    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
    ) -> AppResult<Option<DownloadSubmission>>;

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()>;

    async fn delete_by_client_item_id(&self, download_client_item_id: &str) -> AppResult<()>;

    async fn update_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
        tracked_state: &str,
    ) -> AppResult<()>;

    async fn get_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait ImportArtifactRepository: Send + Sync {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()>;

    async fn list_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Vec<ImportArtifact>>;

    async fn count_by_result(
        &self,
        source_system: &str,
        source_ref: &str,
        result: &str,
    ) -> AppResult<u64>;
}

#[async_trait]
pub trait JobRunRepository: Send + Sync {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord>;

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord>;

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>>;

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>>;

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>>;
}

#[async_trait]
pub trait LibraryProbeRepository: Send + Sync {
    async fn get_probe_signature(&self, title_id: &str)
    -> AppResult<Option<LibraryProbeSignature>>;

    async fn upsert_probe_signature(&self, probe: &LibraryProbeSignature) -> AppResult<()>;
}

#[async_trait]
pub trait LibraryScanUnmatchedItemRepository: Send + Sync {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String>;

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>>;

    async fn delete_library_scan_unmatched_item(
        &self,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()>;

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>>;

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
    ) -> AppResult<i64>;
}

#[async_trait]
pub trait StagedNzbStore: Send + Sync {
    async fn create_pending_staged_nzb(
        &self,
        source_url: &str,
        title_id: Option<&str>,
    ) -> AppResult<PendingStagedNzb>;

    async fn finalize_pending_staged_nzb(
        &self,
        pending: PendingStagedNzb,
        raw_size_bytes: u64,
    ) -> AppResult<StagedNzbRef>;

    async fn delete_staged_nzb(&self, staged_nzb: &StagedNzbRef) -> AppResult<bool>;

    async fn prune_staged_nzbs_older_than(&self, older_than: DateTime<Utc>) -> AppResult<u32>;

    fn mark_artifact_active(&self, path: &Path) -> AppResult<()>;

    fn mark_artifact_inactive(&self, path: &Path) -> AppResult<()>;
}

#[async_trait]
pub trait ImportRepository: Send + Sync {
    async fn queue_import_request(
        &self,
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String>;

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>>;

    async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>>;

    async fn get_import_by_source_ref_and_type(
        &self,
        source_system: &str,
        source_ref: &str,
        import_type: ImportType,
    ) -> AppResult<Option<ImportRecord>>;

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()>;

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64>;

    async fn recover_stale_processing_imports_for_type(
        &self,
        import_type: ImportType,
        stale_seconds: i64,
    ) -> AppResult<u64>;

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>>;

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>>;

    async fn list_imports_for_sources(
        &self,
        sources: &[(String, String)],
    ) -> AppResult<Vec<ImportRecord>>;

    async fn is_already_imported(&self, source_system: &str, source_ref: &str) -> AppResult<bool>;

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>>;
}

#[async_trait]
pub trait ExternalImportMonitorSnapshotRepository: Send + Sync {
    async fn upsert_external_import_monitor_snapshot(
        &self,
        snapshot: &crate::ExternalImportMonitorSnapshot,
    ) -> AppResult<()>;

    async fn get_external_import_monitor_snapshot(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Option<crate::ExternalImportMonitorSnapshot>>;

    async fn delete_external_import_monitor_snapshot(&self, facet: &MediaFacet) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadQueueCommandRepository: Send + Sync {
    async fn queue_delete_command(
        &self,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<crate::DownloadQueueCommandRecord>;

    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64>;

    async fn list_pending_delete_commands(
        &self,
    ) -> AppResult<Vec<crate::DownloadQueueCommandRecord>>;

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()>;

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()>;

    async fn mark_delete_command_failed(&self, id: &str, error_text: Option<&str>)
    -> AppResult<()>;

    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(String, String, bool)],
    ) -> AppResult<Vec<crate::DownloadQueueCommandRecord>>;

    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32>;
}

#[derive(Debug, Clone)]
pub struct WorkflowOperationInfo {
    pub id: String,
    pub operation_type: String,
    pub status: String,
    pub actor_user_id: Option<String>,
    pub progress_json: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[async_trait]
pub trait WorkflowOperationRepository: Send + Sync {
    async fn create_workflow_operation(
        &self,
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<WorkflowOperationInfo>;
}

#[async_trait]
pub trait FileImporter: Send + Sync {
    async fn import_file(&self, source: &Path, dest: &Path) -> AppResult<ImportFileResult>;
}

#[async_trait]
pub trait MediaAnalyzer: Send + Sync {
    async fn analyze_file(&self, path: PathBuf) -> AppResult<MediaAnalysisOutcome>;
}

#[async_trait]
pub trait MediaFileRepository: Send + Sync {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String>;

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()>;

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>>;

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>>;

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>>;

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>>;

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()>;

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()>;

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()>;

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()>;

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>>;

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>>;

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait WantedItemRepository: Send + Sync {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String>;

    async fn ensure_wanted_item_seeded(&self, item: &WantedItem) -> AppResult<String> {
        let existing = find_existing_wanted_item_seed(self, item).await?;
        let mut seeded = item.clone();

        if let Some(existing) = existing.as_ref() {
            seeded.id = existing.id.clone();
            if existing.search_count > 0 {
                seeded.next_search_at = existing.next_search_at.clone();
            }
            if item.status == WantedStatus::Wanted && existing.status != WantedStatus::Wanted {
                seeded.status = existing.status;
            }
        }

        self.upsert_wanted_item(&seeded).await?;
        Ok(existing.map_or(item.id.clone(), |item| item.id))
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<WantedItem>>;

    async fn update_wanted_item_status(
        &self,
        id: &str,
        status: &str,
        next_search_at: Option<&str>,
        last_search_at: Option<&str>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()>;

    async fn schedule_wanted_item_search(
        &self,
        transition: &WantedSearchTransition,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            &transition.id,
            WantedStatus::Wanted.as_str(),
            transition.next_search_at.as_deref(),
            transition.last_search_at.as_deref(),
            transition.search_count,
            transition.current_score,
            transition.grabbed_release.as_deref(),
        )
        .await
    }

    async fn transition_wanted_to_grabbed(
        &self,
        transition: &WantedGrabTransition,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            &transition.id,
            WantedStatus::Grabbed.as_str(),
            None,
            transition.last_search_at.as_deref(),
            transition.search_count,
            transition.current_score,
            Some(&transition.grabbed_release),
        )
        .await
    }

    async fn transition_wanted_to_completed(
        &self,
        transition: &WantedCompleteTransition,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            &transition.id,
            WantedStatus::Completed.as_str(),
            None,
            transition.last_search_at.as_deref(),
            transition.search_count,
            transition.current_score,
            transition.grabbed_release.as_deref(),
        )
        .await
    }

    async fn complete_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        current_score: Option<i32>,
    ) -> AppResult<bool> {
        let Some(wanted) = self.get_wanted_item_for_title(title_id, episode_id).await? else {
            return Ok(false);
        };

        self.transition_wanted_to_completed(&WantedCompleteTransition {
            id: wanted.id,
            last_search_at: last_search_at.map(str::to_string),
            search_count: wanted.search_count,
            current_score: current_score.or(wanted.current_score),
            grabbed_release: wanted.grabbed_release,
        })
        .await?;

        Ok(true)
    }

    async fn transition_wanted_to_paused(
        &self,
        transition: &WantedPauseTransition,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            &transition.id,
            WantedStatus::Paused.as_str(),
            None,
            transition.last_search_at.as_deref(),
            transition.search_count,
            transition.current_score,
            transition.grabbed_release.as_deref(),
        )
        .await
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>>;

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()>;

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()>;

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()>;

    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64>;

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String>;

    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>>;

    async fn list_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<WantedItem>>;

    async fn count_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
    ) -> AppResult<i64>;

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>>;

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>>;
}

async fn find_existing_wanted_item_seed<R: WantedItemRepository + ?Sized>(
    repo: &R,
    item: &WantedItem,
) -> AppResult<Option<WantedItem>> {
    if let Some(collection_id) = item.collection_id.as_deref() {
        return Ok(repo
            .list_wanted_items(None, None, Some(&item.title_id), 500, 0)
            .await?
            .into_iter()
            .find(|existing| existing.collection_id.as_deref() == Some(collection_id)));
    }

    if let Some(episode_id) = item.episode_id.as_deref() {
        return repo
            .get_wanted_item_for_title(&item.title_id, Some(episode_id))
            .await;
    }

    Ok(repo
        .list_wanted_items(None, None, Some(&item.title_id), 500, 0)
        .await?
        .into_iter()
        .find(|existing| existing.episode_id.is_none() && existing.collection_id.is_none()))
}

#[async_trait]
pub trait PendingReleaseRepository: Send + Sync {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String>;
    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>>;
    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>>;
    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>>;
    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>>;
    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()>;
    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>>;
    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()>;
    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>>;
    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool>;
    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()>;
    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait BlocklistRepository: Send + Sync {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String>;

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>>;

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)>;

    async fn remove(&self, id: &str) -> AppResult<()>;

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool>;

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait RuleSetRepository: Send + Sync {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>>;
    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>>;
    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>>;
    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()>;
    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()>;
    async fn delete_rule_set(&self, id: &str) -> AppResult<()>;
    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()>;
    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>>;
    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()>;
    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>>;
}

#[async_trait]
pub trait PostProcessingScriptRepository: Send + Sync {
    async fn list_scripts(&self) -> AppResult<Vec<scryer_domain::PostProcessingScript>>;
    async fn get_script(&self, id: &str) -> AppResult<Option<scryer_domain::PostProcessingScript>>;
    async fn create_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript>;
    async fn update_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript>;
    async fn delete_script(&self, id: &str) -> AppResult<()>;
    async fn list_enabled_for_facet(
        &self,
        facet: &str,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScript>>;
    async fn record_run(&self, run: scryer_domain::PostProcessingScriptRun) -> AppResult<()>;
    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>>;
    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>>;
}

#[async_trait]
pub trait PluginInstallationRepository: Send + Sync {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>>;
    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>>;
    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation>;
    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation>;
    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()>;
    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>>;
    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        provider_type: &str,
    ) -> AppResult<()>;
    async fn store_registry_cache(&self, json: &str) -> AppResult<()>;
    async fn get_registry_cache(&self) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait IndexerClient: Send + Sync {
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse>;
}

pub trait IndexerPluginProvider: Send + Sync {
    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy>;
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }
    fn plugin_name_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn default_base_url_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn rate_limit_seconds_for_provider(&self, _provider_type: &str) -> Option<i64> {
        None
    }
    fn capabilities_for_provider(
        &self,
        _provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        scryer_domain::IndexerProviderCapabilities {
            rss: true,
            supported_ids: std::collections::HashMap::from([
                ("movie".into(), vec!["imdb_id".into()]),
                ("series".into(), vec!["tvdb_id".into()]),
            ]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: true,
            tvdb_search: true,
            anidb_search: false,
        }
    }
}

pub trait DownloadClientPluginProvider: Send + Sync {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }
    fn plugin_name_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn default_base_url_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
        vec![]
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
}

#[async_trait]
pub trait NotificationClient: Send + Sync {
    async fn send_notification(
        &self,
        event_type: &str,
        title: &str,
        message: &str,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> AppResult<()>;
}

pub trait NotificationPluginProvider: Send + Sync {
    fn client_for_channel(
        &self,
        config: &scryer_domain::NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn config_fields_for_provider(&self, provider_type: &str)
    -> Vec<scryer_domain::ConfigFieldDef>;
    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String>;
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
}

#[async_trait]
pub trait NotificationChannelRepository: Send + Sync {
    async fn list_channels(&self) -> AppResult<Vec<scryer_domain::NotificationChannelConfig>>;
    async fn get_channel(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_domain::NotificationChannelConfig>>;
    async fn create_channel(
        &self,
        config: scryer_domain::NotificationChannelConfig,
    ) -> AppResult<scryer_domain::NotificationChannelConfig>;
    async fn update_channel(
        &self,
        config: scryer_domain::NotificationChannelConfig,
    ) -> AppResult<scryer_domain::NotificationChannelConfig>;
    async fn delete_channel(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait NotificationSubscriptionRepository: Send + Sync {
    async fn list_subscriptions(&self) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn list_subscriptions_for_channel(
        &self,
        channel_id: &str,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn list_subscriptions_for_event(
        &self,
        event_type: scryer_domain::NotificationEventType,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn create_subscription(
        &self,
        sub: scryer_domain::NotificationSubscription,
    ) -> AppResult<scryer_domain::NotificationSubscription>;
    async fn update_subscription(
        &self,
        sub: scryer_domain::NotificationSubscription,
    ) -> AppResult<scryer_domain::NotificationSubscription>;
    async fn delete_subscription(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadClient: Send + Sync {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult>;

    async fn submit_to_download_queue(
        &self,
        title: &Title,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> AppResult<DownloadGrabResult> {
        let request = DownloadClientAddRequest::from_legacy(
            title,
            source_hint,
            source_kind,
            source_title,
            source_password,
            category,
        );
        self.submit_download(&request).await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Err(AppError::Repository(
            "download queue listing is not supported for this client".to_string(),
        ))
    }

    async fn list_queue_for_title(&self, _title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_queue().await
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Err(AppError::Repository(
            "download history listing is not supported for this client".to_string(),
        ))
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let items = self.list_history().await?;
        Ok(items.into_iter().skip(offset).take(limit).collect())
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_history_page(0, limit).await
    }

    async fn list_recent_activity_for_title(
        &self,
        _title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_recent_activity(limit).await
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        Err(AppError::Repository(
            "completed download listing is not supported for this client".to_string(),
        ))
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut items = self.list_completed_downloads().await?;
        items.sort_by(|left, right| right.completed_at.cmp(&left.completed_at));
        items.truncate(limit);
        Ok(items)
    }

    async fn pause_queue_item(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "pause is not supported for this download client".to_string(),
        ))
    }

    async fn resume_queue_item(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "resume is not supported for this download client".to_string(),
        ))
    }

    async fn delete_queue_item(&self, _id: &str, _is_history: bool) -> AppResult<()> {
        Err(AppError::Repository(
            "delete is not supported for this download client".to_string(),
        ))
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        let _ = client_type;
        self.delete_queue_item(id, is_history).await
    }

    async fn mark_imported(&self, _request: &DownloadClientMarkImportedRequest) -> AppResult<()> {
        Err(AppError::Repository(
            "mark_imported is not supported for this download client".to_string(),
        ))
    }

    async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
        Err(AppError::Repository(
            "client status is not supported for this download client".to_string(),
        ))
    }

    async fn test_connection(&self) -> AppResult<String> {
        Err(AppError::Repository(
            "test connection is not supported for this download client".to_string(),
        ))
    }
}

#[async_trait]
pub trait SubtitleDownloadRepository: Send + Sync {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>>;
    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>>;
    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>>;
    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()>;
    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()>;
    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>>;
    async fn is_blacklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool>;
    async fn blacklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()>;
}
