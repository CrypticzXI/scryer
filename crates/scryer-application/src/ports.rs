use super::*;
use async_trait::async_trait;
use scryer_domain::{ImportType, IndexerCapsSnapshot, PersistedPluginWasmPayload};
use std::collections::BTreeMap;

pub const NOTIFICATION_REQUEST_SCHEMA_VERSION: u32 = 1;

#[async_trait]
pub trait TitleRepository: Send + Sync {
    async fn list(&self, facet: Option<MediaFacet>, query: Option<String>)
    -> AppResult<Vec<Title>>;
    async fn list_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list(facet, query).await
    }
    async fn list_for_libraries(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let titles = self.list(facet, query).await?;
        Ok(titles
            .into_iter()
            .filter(|title| library_ids.iter().any(|id| id == &title.library_id))
            .collect())
    }
    async fn list_for_libraries_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let titles = self.list_without_external_ids(facet, query).await?;
        Ok(titles
            .into_iter()
            .filter(|title| library_ids.iter().any(|id| id == &title.library_id))
            .collect())
    }
    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>>;
    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>>;
    async fn get_by_id_without_external_ids(&self, id: &str) -> AppResult<Option<Title>> {
        self.get_by_id(id).await
    }
    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>>;
    async fn get_by_facet_libraries_and_slug(
        &self,
        facet: MediaFacet,
        library_ids: &[String],
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let normalized_slug = slug.trim();
        if normalized_slug.is_empty() || library_ids.is_empty() {
            return Ok(None);
        }

        let matches =
            self.list_for_libraries(Some(facet), library_ids, None)
                .await?
                .into_iter()
                .filter(|title| {
                    title.slug.as_deref().is_some_and(|candidate| {
                        candidate.trim().eq_ignore_ascii_case(normalized_slug)
                    })
                })
                .collect::<Vec<_>>();

        match matches.as_slice() {
            [] => Ok(None),
            [title] => Ok(Some(title.clone())),
            _ => Err(AppError::Validation(
                "multiple titles found for slug lookup".into(),
            )),
        }
    }
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
    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>>;
    async fn list_anime_title_ids_missing_title_anidb_external_ids(
        &self,
        _limit: usize,
    ) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
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
pub trait LibraryRepository: Send + Sync {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>>;
    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>>;
    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library>;
    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library>;
    async fn delete_library(&self, library_id: &str) -> AppResult<bool>;
    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask>;
    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()>;
    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>>;
    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()>;
    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>>;
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

    async fn replace_title_image_and_append_event(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
        event: NewDomainEvent,
    ) -> AppResult<DomainEvent>;

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
    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>>;
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
    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()>;
    async fn delete_collection(&self, collection_id: &str) -> AppResult<()>;
    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>>;
    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>>;
    async fn list_episode_external_ids(&self, episode_id: &str)
    -> AppResult<Vec<ScopedExternalId>>;
    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>>;
    async fn create_episode(&self, episode: Episode) -> AppResult<Episode>;
    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode>;
    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()>;
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
    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>>;
    async fn create(&self, user: User) -> AppResult<User>;
    async fn list_all(&self) -> AppResult<Vec<User>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>>;
    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait DomainEventRepository: Send + Sync {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent>;
    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>>;
    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>>;
    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64>;
    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>>;
    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>>;
    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32>;
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
pub trait IndexerCapsSnapshotRefresher: Send + Sync {
    async fn fetch_for_config(
        &self,
        config: &IndexerConfig,
    ) -> AppResult<Option<IndexerCapsSnapshot>>;
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
pub trait SubtitleProviderConfigRepository: Send + Sync {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<SubtitleProviderConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<SubtitleProviderConfig>>;
    async fn create(&self, config: SubtitleProviderConfig) -> AppResult<SubtitleProviderConfig>;
    async fn update(
        &self,
        update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>>;
    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.get_setting_json(scope, key_name, scope_id).await
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()>;

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()>;

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32>;
}

#[async_trait]
pub trait SystemInfoProvider: Send + Sync {
    async fn datastore_info(&self) -> AppResult<DatastoreInfo>;
    async fn current_migration_version(&self) -> AppResult<Option<String>>;
    async fn current_encryption_key_base64(&self) -> AppResult<Option<String>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatastoreInfo {
    pub engine: String,
    pub current_migration_key: Option<String>,
}

#[async_trait]
pub trait LogicalBackupExporter: Send + Sync {
    async fn export_backup_bundle(
        &self,
        request: crate::BackupBundleExportRequest,
    ) -> AppResult<crate::BackupExportOutcome>;
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
    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32>;
    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32>;
    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32>;
    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>>;
    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32>;
    async fn run_database_maintenance(&self) -> AppResult<()> {
        Ok(())
    }
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
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>>;

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>>;

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>>;
    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
    ) -> AppResult<Option<DownloadSubmission>>;

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()>;

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()>;

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()>;

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait ImportArtifactRepository: Send + Sync {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()>;

    async fn list_by_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Vec<ImportArtifact>>;

    async fn count_by_result_for_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
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
    async fn delete_probe_signatures_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32>;
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
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()>;
    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32>;

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>>;

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
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
        source_identity: DownloadSourceIdentity,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String>;

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>>;

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

    async fn list_imports_for_identities(
        &self,
        identities: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<ImportRecord>>;

    async fn is_already_imported(&self, identity: &DownloadSourceIdentity) -> AppResult<bool>;

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>>;
}

#[async_trait]
pub trait ExternalImportMonitorSnapshotRepository: Send + Sync {
    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &crate::ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()>;

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        facet: MediaFacet,
        entry_kind: crate::ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<crate::ExternalImportMonitorSnapshotChunk>>;

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        facet: MediaFacet,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadQueueCommandRepository: Send + Sync {
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
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
        sources: &[(Option<String>, String, String, bool)],
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

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>>;

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>>;

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>>;

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>>;

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

    #[expect(
        clippy::too_many_arguments,
        reason = "the repository update maps directly onto persisted wanted-item state fields"
    )]
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

    async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>>;

    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64>;

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
            .list_wanted_items(WantedItemsQuery {
                title_id: Some(item.title_id.clone()),
                limit: 500,
                ..WantedItemsQuery::default()
            })
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
        .list_wanted_items(WantedItemsQuery {
            title_id: Some(item.title_id.clone()),
            limit: 500,
            ..WantedItemsQuery::default()
        })
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
    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
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

    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool>;

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
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>>;
    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>>;
    #[expect(
        clippy::too_many_arguments,
        reason = "builtin plugin seeding persists the full published plugin contract explicitly"
    )]
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
    ) -> AppResult<()>;
    async fn upsert_plugin_catalog_source(&self, source: &PluginCatalogSource) -> AppResult<()>;
    async fn list_plugin_catalog_sources(&self) -> AppResult<Vec<PluginCatalogSource>>;
    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<PluginCatalogSource>>;
    async fn upsert_plugin_catalog_status(
        &self,
        status: &PluginCatalogStatusRecord,
    ) -> AppResult<()>;
    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<PluginCatalogStatusRecord>>;
}

#[async_trait]
pub trait IndexerClient: Send + Sync {
    #[expect(
        clippy::too_many_arguments,
        reason = "indexer search forwards the full caller-controlled search envelope to plugins"
    )]
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
    fn management_client_for_provider(
        &self,
        _config: &IndexerConfig,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        None
    }
    fn available_provider_types(&self) -> Vec<String>;
    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }
    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_type_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy>;
    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime restoration".to_string())
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
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
    fn plugin_description_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn default_base_url_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn rate_limit_seconds_for_provider(&self, _provider_type: &str) -> Option<i64> {
        None
    }
    fn management_capabilities_for_provider(
        &self,
        _provider_type: &str,
    ) -> scryer_domain::IndexerManagementCapabilities {
        scryer_domain::IndexerManagementCapabilities::default()
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
            ..Default::default()
        }
    }
}

#[async_trait]
pub trait IndexerManagementClient: Send + Sync {
    async fn validate_connection(&self) -> AppResult<IndexerValidationResult>;
    async fn plan_sync(&self, _parent_config_id: &str) -> AppResult<IndexerSyncPlan> {
        Err(AppError::Repository(
            "managed child sync is not supported for this provider".to_string(),
        ))
    }
    fn name(&self) -> &str;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExternalPluginWasm<'a> {
    pub bytes: &'a [u8],
    pub first_party: bool,
}

#[derive(Clone, Debug)]
pub struct RuntimePluginLoad {
    pub descriptor: scryer_plugin_sdk::PluginDescriptor,
    pub wasm_bytes: Vec<u8>,
    pub first_party: bool,
}

pub trait DownloadClientPluginProvider: Send + Sync {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }
    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
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
    fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
        vec![]
    }
    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime restoration".to_string())
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
    }
    fn plugin_description_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationAppPayload {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationExternalIdsPayload {
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub anidb_id: Option<String>,
    pub tvmaze_id: Option<String>,
    pub anilist_ids: Vec<String>,
    pub mal_ids: Vec<String>,
    pub kitsu_ids: Vec<String>,
    pub by_source: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationActorPayload {
    pub user_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotificationSeverityPayload {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationTitlePayload {
    pub id: Option<String>,
    pub name: String,
    pub facet: String,
    pub year: Option<i32>,
    pub slug: Option<String>,
    pub path: Option<String>,
    pub overview: Option<String>,
    pub sort_title: Option<String>,
    pub poster_url: Option<String>,
    pub banner_url: Option<String>,
    pub background_url: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub original_language: Option<String>,
    pub original_country: Option<String>,
    pub external_ids: NotificationExternalIdsPayload,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationEpisodePayload {
    pub id: Option<String>,
    pub episode_ids: Vec<String>,
    pub display: Option<String>,
    pub collection_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub absolute_number: Option<String>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub air_date_utc: Option<String>,
    pub episode_type: Option<String>,
    pub finale_type: Option<String>,
    pub tvdb_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationReleasePayload {
    pub source_title: Option<String>,
    pub source_hint: Option<String>,
    pub quality: Option<String>,
    pub provider: Option<String>,
    pub language: Option<String>,
    pub release_group: Option<String>,
    pub protocol: Option<String>,
    pub indexer: Option<String>,
    pub languages: Vec<String>,
    pub custom_scores: BTreeMap<String, i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationDownloadPayload {
    pub download_id: Option<String>,
    pub client_id: Option<String>,
    pub client_name: Option<String>,
    pub client_type: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub status_message: Option<String>,
    pub size_bytes: Option<i64>,
    pub progress_percent: Option<i32>,
    pub output_path: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationImportPayload {
    pub import_id: Option<String>,
    pub source_system: Option<String>,
    pub source_ref: Option<String>,
    pub source_title: Option<String>,
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub imported_count: Option<i32>,
    pub status: Option<String>,
    pub skipped_count: Option<i32>,
    pub rejected_count: Option<i32>,
    pub upgrade: bool,
    pub deleted_paths: Vec<String>,
    pub replaced_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationHealthPayload {
    pub status: Option<String>,
    pub message: Option<String>,
    pub severity: Option<String>,
    pub code: Option<String>,
    pub details: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationMediaUpdateTypePayload {
    Created,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationMediaUpdatePayload {
    pub path: String,
    pub update_type: NotificationMediaUpdateTypePayload,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationFilePayload {
    pub primary_path: Option<String>,
    pub media_updates: Vec<NotificationMediaUpdatePayload>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationMediaFilePayload {
    pub id: Option<String>,
    pub path: String,
    pub previous_path: Option<String>,
    pub recycle_bin_path: Option<String>,
    pub size_bytes: Option<i64>,
    pub quality: Option<String>,
    pub release_group: Option<String>,
    pub scene_name: Option<String>,
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    pub video_frame_rate: Option<String>,
    pub container_format: Option<String>,
    pub edition: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationApplicationUpdatePayload {
    pub current_version: Option<String>,
    pub target_version: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationManualInteractionPayload {
    pub kind: Option<String>,
    pub reason: Option<String>,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationPayload {
    pub schema_version: u32,
    pub event_type: scryer_domain::NotificationEventType,
    pub event_id: Option<String>,
    pub occurred_at: Option<String>,
    pub correlation_id: Option<String>,
    pub actor: Option<NotificationActorPayload>,
    pub severity: Option<NotificationSeverityPayload>,
    pub is_test: bool,
    pub summary_title: String,
    pub summary_message: String,
    pub app: NotificationAppPayload,
    pub title: Option<NotificationTitlePayload>,
    pub episode: Option<NotificationEpisodePayload>,
    pub episodes: Vec<NotificationEpisodePayload>,
    pub release: Option<NotificationReleasePayload>,
    pub download: Option<NotificationDownloadPayload>,
    pub import: Option<NotificationImportPayload>,
    pub health: Option<NotificationHealthPayload>,
    pub file: Option<NotificationFilePayload>,
    pub media_files: Vec<NotificationMediaFilePayload>,
    pub application_update: Option<NotificationApplicationUpdatePayload>,
    pub manual_interaction: Option<NotificationManualInteractionPayload>,
}

#[async_trait]
pub trait NotificationClient: Send + Sync {
    async fn send_notification(&self, payload: &NotificationPayload) -> AppResult<()>;
}

#[async_trait]
pub trait SubtitleProviderClient: Send + Sync {
    async fn search(
        &self,
        query: &crate::subtitles::SubtitleQuery,
    ) -> AppResult<Vec<crate::subtitles::SubtitleMatch>>;
    async fn download(&self, provider_file_id: &str) -> AppResult<crate::subtitles::SubtitleFile>;
    async fn validate_connection(&self) -> AppResult<SubtitleProviderValidationResult>;
    async fn generate(
        &self,
        _request: &SubtitleGenerationInput,
    ) -> AppResult<crate::subtitles::SubtitleFile> {
        Err(AppError::Repository(
            "subtitle generation is not supported for this provider".to_string(),
        ))
    }
    fn name(&self) -> &str;
}

pub trait NotificationPluginProvider: Send + Sync {
    fn client_for_channel(
        &self,
        config: &scryer_domain::NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }
    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn supported_events_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::NotificationEventType> {
        vec![]
    }
    fn supports_test_for_provider(&self, _provider_type: &str) -> bool {
        false
    }
    fn config_fields_for_provider(&self, provider_type: &str)
    -> Vec<scryer_domain::ConfigFieldDef>;
    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String>;
    fn plugin_description_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime restoration".to_string())
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
    }
}

pub trait SubtitlePluginProvider: Send + Sync {
    fn client_for_config(
        &self,
        config: &scryer_domain::SubtitleProviderConfig,
        host_bindings: &std::collections::HashMap<scryer_domain::PluginHostBindingId, String>,
    ) -> Option<Arc<dyn SubtitleProviderClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }
    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn supports_catalog_search_for_provider(&self, provider_type: &str) -> bool;
    fn recommended_facets_for_provider(&self, provider_type: &str) -> Vec<String>;
    fn config_fields_for_provider(&self, provider_type: &str)
    -> Vec<scryer_domain::ConfigFieldDef>;
    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String>;
    fn plugin_description_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime restoration".to_string())
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
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
        items.sort_by_key(|item| std::cmp::Reverse(item.completed_at));
        items.truncate(limit);
        Ok(items)
    }

    async fn pause_queue_item(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "pause is not supported for this download client".to_string(),
        ))
    }

    async fn pause_queue_item_for_client(&self, _client_id: &str, id: &str) -> AppResult<()> {
        self.pause_queue_item(id).await
    }

    async fn resume_queue_item(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "resume is not supported for this download client".to_string(),
        ))
    }

    async fn resume_queue_item_for_client(&self, _client_id: &str, id: &str) -> AppResult<()> {
        self.resume_queue_item(id).await
    }

    async fn delete_queue_item(&self, _id: &str, _is_history: bool) -> AppResult<()> {
        Err(AppError::Repository(
            "delete is not supported for this download client".to_string(),
        ))
    }

    async fn delete_queue_item_for_client_id(
        &self,
        _client_id: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        self.delete_queue_item(id, is_history).await
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

    async fn get_client_status_for_client_id(
        &self,
        _client_id: &str,
    ) -> AppResult<DownloadClientStatus> {
        self.get_client_status().await
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
    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<crate::subtitles::ExternalSubtitleProbeCacheEntry>>;
    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>>;
    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()>;
    async fn upsert_probe_cache_entry(
        &self,
        entry: &crate::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()>;
    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()>;
    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>>;
    async fn delete_probe_cache_entry(&self, media_file_id: &str, file_path: &str)
    -> AppResult<()>;
    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool>;
    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()>;
}
