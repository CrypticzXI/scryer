#![allow(clippy::module_inception, clippy::too_many_arguments)]

mod acquisition;
mod catalog;
mod contracts;
mod events;
mod health;
mod helpers;
mod import;
mod integration;
mod jobs;
mod library;
#[path = "library/scan/scanner.rs"]
mod library_scan;
#[path = "library/scan/coordinator.rs"]
mod library_scan_coordinator;
#[path = "library/scan/helpers.rs"]
mod library_scan_helpers;
#[path = "library/scan/metadata.rs"]
mod library_scan_metadata;
#[path = "library/scan/progress.rs"]
mod library_scan_progress;
#[path = "library/scan/titles.rs"]
mod library_scan_titles;
#[path = "library/scan/unmatched.rs"]
mod library_scan_unmatched;
mod media;
mod notifications;
mod null_repositories;
mod plugins;
mod polling_worker;
mod ports;
mod quality;
mod rules;
mod security;
mod services;
mod settings;
pub mod subtitles;
pub mod testing;
mod types;

pub(crate) use acquisition::acquisition as acquisition_workflow;
pub(crate) use acquisition::coverage as acquisition_coverage;
pub(crate) use acquisition::decision_helpers as acquisition_decision_helpers;
pub(crate) use acquisition::delay_profile;
pub(crate) use acquisition::policy as acquisition_policy;
pub(crate) use acquisition::release_search as acquisition_release_search;
pub(crate) use acquisition::rss as app_usecase_rss;
pub(crate) use acquisition::search_queries as acquisition_search_queries;
pub(crate) use catalog::catalog as catalog_workflow;
pub(crate) use catalog::discovery as app_usecase_discovery;
pub(crate) use catalog::facets::handler as facet_handler;
pub(crate) use catalog::helpers as catalog_helpers;
pub(crate) use events::activity;
pub(crate) use events::domain_events;
pub(crate) use events::event_views;
pub(crate) use import::archive_extractor;
pub(crate) use import::checks as import_checks;
pub(crate) use import::import as import_workflow;
pub(crate) use import::parameters as import_parameters;
pub(crate) use import::post_download_gate;
pub(crate) use import::title_resolution as import_title_resolution;
pub(crate) use integration::integration as app_usecase_integration;
pub(crate) use library::discovery as library_discovery;
pub(crate) use library::nfo;
pub(crate) use library::rename as library_rename;
pub(crate) use library::title_matching;
pub(crate) use media::audio_requirements;
pub(crate) use media::language_data as media_language_data;
pub(crate) use quality::profile as quality_profile;
pub(crate) use quality::release_group_db;
pub(crate) use quality::release_parser;
pub(crate) use quality::scoring_weights;
pub(crate) use rules::user_rule_input;

pub use import::completed_download as completed_download_handler;
pub(crate) mod normalize;
pub use import::failed_download as failed_download_handler;
pub use import::post_processing as app_usecase_post_processing;
pub use import::upgrade;
pub use integration::tracked_downloads;
pub use library::filesystem_walk;
pub use library::recycle_bin;
pub use plugins::managed_rules;
pub use quality::release_dedup;
pub const LIBRARY_SCAN_MAX_RECURSIVE_DEPTH: usize =
    library::discovery::LIBRARY_SCAN_MAX_RECURSIVE_DEPTH;

use chrono::{DateTime, Duration, Utc};
use rand_core::OsRng;
use ring::digest as ring_digest;
use scryer_domain::{
    BlocklistEntry, CalendarEpisode, Collection, CollectionType, CompletedDownload, DomainEvent,
    DomainEventFilter, DomainEventType, DownloadClientConfig, DownloadQueueItem,
    DownloadQueueState, Entitlement, Episode, ExternalId, HistoryEvent, Id, ImportFileResult,
    ImportRecord, ImportResult, ImportStatus, IndexerConfig, MediaFacet, NewDomainEvent,
    NewDownloadClientConfig, NewIndexerConfig, NewTitle, PluginInstallation, PolicyInput,
    PolicyOutput, RuleSet, SubtitleProviderConfig, TaggedAlias, Title, TitleHistoryEventType,
    TitleHistoryRecord, User,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell, RwLock, Semaphore, broadcast};

pub type AppResult<T> = Result<T, AppError>;

use crate::quality_profile::resolve_profile_id_for_title;
pub use acquisition::delay_profile::{
    DELAY_PROFILE_CATALOG_KEY, DelayDecision, DelayProfile, PreferredProtocol, is_usenet_source,
    parse_delay_profile_catalog, resolve_delay_decision, resolve_delay_profile,
    validate_delay_profile_catalog,
};
pub use acquisition::policy::AcquisitionThresholds;
pub use acquisition_workflow::start_background_acquisition_poller;
pub use app_usecase_integration::derive_download_queue_display_state;
pub use app_usecase_integration::enrich_download_queue_items_from_submissions;
pub use app_usecase_integration::matches_download_activity_filter;
pub use app_usecase_integration::matches_download_queue_filter;
pub use app_usecase_integration::start_download_queue_poller;
pub use app_usecase_post_processing::{PostProcessingContext, run_post_processing};
pub use app_usecase_rss::RssSyncReport;
pub(crate) use audio_requirements::{
    missing_required_audio_languages, normalize_required_audio_languages,
    release_audio_language_hints, required_audio_languages_match,
};
pub use catalog::facets::handler::{
    FacetHandler, HydrationResult, movie_to_hydration_result, series_to_hydration_result,
};
pub use catalog::facets::movie::MovieFacetHandler;
pub use catalog::facets::registry::FacetRegistry;
pub use catalog::facets::series::SeriesFacetHandler;
pub use catalog::title_hydration::start_background_title_hydration_loop;
pub use catalog::title_images::start_background_banner_loop;
pub use catalog::title_images::start_background_fanart_loop;
pub use catalog::title_images::start_background_poster_loop;
pub use contracts::{
    AudioStreamDetail, CollectionUpdate, DeleteExecutionConfirmation, DownloadClientAddRequest,
    DownloadClientConfigUpdate, DownloadClientMarkImportedRequest, DownloadClientStatus,
    DownloadSourceIdentity, DownloadSubmission, EpisodeUpdate, ImportArtifact, IndexerConfigUpdate,
    IndexerRoutingEntry, IndexerRoutingPlan, InsertMediaFileInput, MediaAnalysisOutcome,
    MediaFileAnalysis, NewBlocklistEntry, NotificationScopeIdUpdate, PendingStagedNzb,
    QueueDownloadOutcome, QueuedDownloadResult, QueuedReleaseSelection, ReleaseDecisionsQuery,
    SearchMode, StagedNzbRef, SubmissionConflictPolicy, SubmissionScope, SubmissionScopeConflict,
    SubtitleGenerationInput, SubtitleProviderConfigUpdate, SubtitleProviderValidationResult,
    SubtitleStreamDetail, SuccessfulGrabCommit, TitleHistoryFilter, TitleHistoryPage,
    WantedItemsQuery, WantedSearchOutcome,
};
pub use event_views::{
    apply_download_queue_projection_event, apply_job_next_run_projection_event,
    apply_job_run_projection_event, apply_library_scan_projection_event, replay_active_job_runs,
    replay_download_queue_state, replay_job_next_runs, replay_library_scan_state,
    sorted_download_queue_items,
};
pub use events::activity::{ActivityChannel, ActivityEvent, ActivityKind, ActivitySeverity};
pub use events::activity_api::{
    is_supported_title_history_event_type, supported_title_history_event_types,
};
pub(crate) use import_workflow::fail_active_manual_import_for_source;
pub use import_workflow::{
    ManualImportExecutionResult, ManualImportFileMapping, ManualImportFilePreview,
    ManualImportFileResult, ManualImportPreview, ManualImportRequestPayload, execute_manual_import,
    execute_queued_manual_import, import_completed_download, preview_manual_import,
    retry_failed_import, start_background_manual_import_poller, try_import_completed_downloads,
};
pub use integration::download_queue_commands::start_background_download_delete_poller;
pub(crate) use integration::integration::ManualImportSourceResolution;
pub use jobs::jobs::start_background_library_refresh_loop;
pub use library::rename::{
    LibraryRenamer, NullLibraryRenamer, RenameApplyItemResult, RenameApplyResult,
    RenameApplyStatus, RenameCollisionPolicy, RenameMissingMetadataPolicy, RenamePlan,
    RenamePlanItem, RenameWriteAction, build_rename_plan_fingerprint, render_rename_template,
};
pub use media::language::{
    normalize_detected_audio_language_code, normalize_detected_audio_languages,
    normalize_detected_subtitle_language_code, normalize_detected_subtitle_languages,
};
pub use plugins::plugins::{RegistryPlugin, RulePackRegistryEntry, RulePackTemplate};
pub use security::backup::BackupService;
pub use settings::settings::{
    AcquisitionSettings, DownloadClientRoutingSettingsEntry, ExternalImportLibraryPathsSelection,
    FacetScoringPersonaSelection, GeneralSettings, IndexerRoutingSettingsEntry,
    LibraryPathsSettings, MediaSettings, QualityProfileSelection, QualityProfileSettings,
    SaveQualityProfileSettings, ServiceSettings, SubtitleSettings,
    UpdateFacetScoringPersonaSelection, UpdateGeneralSettings, UpdateLibraryPaths,
    UpdateMediaSettings, UpdateQualityProfileSelection, UpdateServiceSettings,
    UpdateSubtitleSettings,
};
pub use subtitles::orchestration::{
    spawn_subtitle_search_for_file, start_background_subtitle_poller,
};

pub const DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE: &str =
    "download feedback timed out after 10s; queue status is temporarily unavailable";

pub(crate) const GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY: usize = 4;
pub use acquisition::release_search::release_strategy_kind_for_label;
pub use app_usecase_integration::publish_download_queue_snapshot_events;
#[cfg(unix)]
pub(crate) use helpers::statvfs_path;
pub(crate) use helpers::{
    INDEXER_PROVIDER_NZBGEEK, INHERIT_QUALITY_PROFILE_VALUE, NATIVE_DOWNLOAD_CLIENT_TYPES,
    await_cancellable, await_cancellable_app_result, normalize_release_attempt_hint,
    normalize_release_attempt_title, normalize_release_password,
    normalize_release_selection_signature, normalize_show_text_opt, normalize_tags,
    parsed_episode_lookup_season, require, sanitize_ids, sha256_hex, to_hex,
};
pub use helpers::{accepted_inputs_for_client, nice_thread};
pub use jobs::definitions::{
    JobCategory, JobDefinition, JobKey, JobRun, JobRunRecord, JobRunStatus, JobRunTracker,
    JobScheduleInfo, JobScheduleKind, JobSection, JobTriggerSource, LibraryProbeSignature,
};
pub use library::user_delete::DeletePreview;
pub use library_scan::{
    AnimeEpisodeMapping, AnimeMapping, AnimeMovie, BulkMetadataResult, EpisodeMetadata,
    LibraryDirectoryScanResult, LibraryFile, LibraryFileBatch, LibraryFileBatchReceiver,
    LibraryScanSummary, LibraryScanner, MetadataGateway, MetadataSearchItem, MetadataSearchQuery,
    MovieMetadata, MultiMetadataSearchResult, RichMetadataSearchItem, SeasonMetadata,
    SeriesMetadata, source_signature_from_std_metadata,
};
pub use library_scan_progress::{
    LibraryScanMode, LibraryScanPhaseProgress, LibraryScanSession, LibraryScanStatus,
    LibraryScanTracker,
};
pub use media::analyzer::NativeMediaAnalyzer;
pub use notifications::dispatcher::start_notification_dispatcher;
pub use null_repositories::{
    NullAcquisitionStateRepository, NullBlocklistRepository, NullDomainEventRepository,
    NullDownloadQueueCommandRepository, NullDownloadSubmissionRepository,
    NullExternalImportMonitorSnapshotRepository, NullFileImporter, NullHousekeepingRepository,
    NullImportRepository, NullIndexerStatsTracker, NullJobRunRepository,
    NullLibraryProbeRepository, NullLibraryScanUnmatchedItemRepository, NullMediaFileRepository,
    NullNotificationChannelRepository, NullNotificationSubscriptionRepository,
    NullPendingReleaseRepository, NullPluginInstallationRepository,
    NullPostProcessingScriptRepository, NullRuleSetRepository, NullSettingsRepository,
    NullStagedNzbStore, NullSystemInfoProvider, NullTitleImageProcessor, NullTitleImageRepository,
    NullWantedItemRepository, NullWorkflowOperationRepository,
};
pub use ports::{
    AcquisitionStateRepository, BlocklistRepository, DomainEventRepository, DownloadClient,
    DownloadClientConfigRepository, DownloadClientPluginProvider, DownloadQueueCommandRepository,
    DownloadSubmissionRepository, ExternalImportMonitorSnapshotRepository, ExternalPluginWasm,
    FileImporter, HousekeepingRepository, ImportArtifactRepository, ImportRepository,
    IndexerClient, IndexerConfigRepository, IndexerPluginProvider, IndexerStatsTracker,
    JobRunRepository, LibraryProbeRepository, LibraryScanUnmatchedItemRepository, MediaAnalyzer,
    MediaFileRepository, NOTIFICATION_REQUEST_SCHEMA_VERSION, NotificationActorPayload,
    NotificationAppPayload, NotificationApplicationUpdatePayload, NotificationChannelRepository,
    NotificationClient, NotificationDownloadPayload, NotificationEpisodePayload,
    NotificationExternalIdsPayload, NotificationFilePayload, NotificationHealthPayload,
    NotificationImportPayload, NotificationManualInteractionPayload, NotificationMediaFilePayload,
    NotificationMediaUpdatePayload, NotificationMediaUpdateTypePayload, NotificationPayload,
    NotificationPluginProvider, NotificationReleasePayload, NotificationSeverityPayload,
    NotificationSubscriptionRepository, NotificationTitlePayload, PendingReleaseRepository,
    PluginInstallationRepository, PostProcessingScriptRepository, QualityProfileRepository,
    ReleaseAttemptRepository, RuleSetRepository, SettingsRepository, ShowRepository,
    StagedNzbStore, SubtitleDownloadRepository, SubtitlePluginProvider, SubtitleProviderClient,
    SubtitleProviderConfigRepository, SystemInfoProvider, TitleImageProcessor,
    TitleImageRepository, TitleRepository, UserRepository, WantedItemRepository,
    WorkflowOperationInfo, WorkflowOperationRepository,
};
pub use quality::release_parser::{
    ParsedEpisodeMetadata, ParsedEpisodeReleaseType, ParsedReleaseMetadata, ParsedSpecialKind,
    ReleaseParseAnalysis, ReleaseParseContext, TargetedReleaseParseAnalysis,
    analyze_release_against_targets, analyze_release_for_target, best_parse_for_target,
    build_candidate_bank_contexts, build_release_parse_context,
    build_release_parse_context_for_title, parse_release_metadata,
    parse_release_metadata_for_target,
};
pub use quality::scoring_weights::{
    ScoringOverrides, ScoringPersona, ScoringWeights, build_weights, build_weights_for_category,
};
pub use quality_profile::{
    BLOCK_SCORE, QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY,
    QUALITY_PROFILE_INHERIT_VALUE, QualityProfile, QualityProfileCriteria, QualityProfileDecision,
    ScoringConfig, ScoringEntry, ScoringSource, apply_age_scoring, apply_size_scoring_for_category,
    default_quality_profile_1080p_for_search, default_quality_profile_for_search,
    evaluate_against_profile, parse_profile_catalog_from_json,
};
pub use services::{AppServices, AppServicesBuilder, AppUseCase, ProviderCatalogFamily};
pub use settings::keys::{
    ANIME_FILLER_POLICY_KEY, ANIME_INTER_SEASON_MOVIES_KEY, ANIME_MONITOR_FILLER_MOVIES_KEY,
    ANIME_MONITOR_SPECIALS_KEY, ANIME_PATH_KEY, ANIME_RECAP_POLICY_KEY, ANIME_ROOT_FOLDERS_KEY,
    AUDIO_PERSONA_MIGRATION_SENTINEL_KEY, DEFAULT_ANIME_LIBRARY_PATH, DEFAULT_FILLER_POLICY,
    DEFAULT_MOVIE_LIBRARY_PATH, DEFAULT_RECAP_POLICY, DEFAULT_RENAME_COLLISION_POLICY,
    DEFAULT_RENAME_MISSING_METADATA_POLICY, DEFAULT_RENAME_TEMPLATE_ANIME,
    DEFAULT_RENAME_TEMPLATE_MOVIE, DEFAULT_RENAME_TEMPLATE_SERIES, DEFAULT_SERIES_LIBRARY_PATH,
    DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY, DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
    HISTORY_KEEP_FOREVER_KEY, HISTORY_RETENTION_DAYS_KEY, INDEXER_ROUTING_SETTINGS_KEY,
    LEGACY_NZBGET_CATEGORY_SETTING_KEY, LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
    METADATA_LANGUAGE_KEY, MOVIES_PATH_KEY, MOVIES_ROOT_FOLDERS_KEY, NFO_WRITE_ON_IMPORT_ANIME_KEY,
    NFO_WRITE_ON_IMPORT_MOVIE_KEY, NFO_WRITE_ON_IMPORT_SERIES_KEY,
    NZBGET_OLDER_PRIORITY_SETTING_KEY, NZBGET_RECENT_PRIORITY_SETTING_KEY,
    PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY, PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY,
    POST_PROCESSING_SCRIPT_ANIME_KEY, POST_PROCESSING_SCRIPT_MOVIE_KEY,
    POST_PROCESSING_SCRIPT_SERIES_KEY, POST_PROCESSING_TIMEOUT_KEY,
    RENAME_COLLISION_POLICY_ANIME_GLOBAL_KEY, RENAME_COLLISION_POLICY_GLOBAL_KEY,
    RENAME_COLLISION_POLICY_KEY, RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY,
    RENAME_COLLISION_POLICY_SERIES_GLOBAL_KEY, RENAME_MISSING_METADATA_POLICY_ANIME_GLOBAL_KEY,
    RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY, RENAME_MISSING_METADATA_POLICY_KEY,
    RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY,
    RENAME_MISSING_METADATA_POLICY_SERIES_GLOBAL_KEY, RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
    RENAME_TEMPLATE_KEY, RENAME_TEMPLATE_MOVIE_GLOBAL_KEY, RENAME_TEMPLATE_SERIES_GLOBAL_KEY,
    REQUIRED_AUDIO_LANGUAGES_KEY, SCORING_PERSONA_KEY, SERIES_PATH_KEY, SERIES_ROOT_FOLDERS_KEY,
    SETTINGS_SCOPE_MEDIA, SETTINGS_SCOPE_SYSTEM, SETTINGS_SOURCE_TYPED_GRAPHQL, SETUP_COMPLETE_KEY,
    TITLE_REQUIRED_AUDIO_OVERRIDE_KEY, TLS_CERT_PATH_KEY, TLS_KEY_PATH_KEY,
};
pub(crate) use types::JwtClaims;
pub use types::SmgVersionCompatibilityNotice;
pub use types::{
    AddTitleAndQueueDownloadOutcome, AddTitleHydrationState, AddTitleOutcome, BackupInfo,
    CancelLibraryScanResult, CreateTitleOutcome, CutoffUnmetTitle, DecisionCodeCount,
    DiskSpaceInfo, DownloadActivityFilter, DownloadDisplayState, DownloadGrabResult,
    DownloadHistoryFilter, DownloadHistoryPage, DownloadHistorySort, DownloadHistorySortKey,
    DownloadImportFilter, DownloadImportPage, DownloadQueueCommandRecord, DownloadSourceKind,
    EpisodeScopedMediaFile, FixTitleMatchResult, HealthCheckResult, HealthCheckStatus,
    HousekeepingReport, IgnorePendingImportResult, IndexerQueryStats, JwtAuthConfig,
    LibraryScanUnmatchedItem, LibraryScanUnmatchedSearchAttempt, PendingImportBindingFilePreview,
    PendingImportBindingPreview, PendingImportConnection, PendingImportCounts, PendingImportItem,
    PendingImportSearchAttempt, PendingImportStatus, PendingRelease, PendingReleaseStatus,
    PendingReleaseStatusCount, PendingTitleHydration, PrimaryCollectionSummary, ReleaseDecision,
    ReleaseDownloadAttemptOutcome, ReleaseDownloadFailureSignature, ResolvePendingImportResult,
    ScopedExternalId, SortDirection, SystemHealth, TitleAcquisitionDiagnostics,
    TitleEpisodeProgressSummary, TitleImageBlob, TitleImageKind, TitleImageReplacement,
    TitleImageStorageMode, TitleImageSyncTask, TitleImageVariantRecord, TitleMediaFile,
    TitleMediaSizeSummary, TitleMetadataUpdate, TitleQualitySummary, TitleReleaseBlocklistEntry,
    WantedCompleteTransition, WantedGrabTransition, WantedItem, WantedPauseTransition,
    WantedSearchTransition, WantedStatus, WantedStatusCount,
};
pub use types::{
    ExternalImportMonitorEpisodeEntry, ExternalImportMonitorMovieEntry,
    ExternalImportMonitorSeasonEntry, ExternalImportMonitorSeriesEntry,
    ExternalImportMonitorSnapshot, ExternalImportMonitorSnapshotPayload,
};
pub use types::{
    IndexerSearchResponse, IndexerSearchResult, ReleaseCandidateProvenance,
    ReleaseSearchSubjectKind, ReleaseStrategyKind,
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("validation: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    DownloadFeedbackTimeout(String),

    #[error("repository: {0}")]
    Repository(String),
}

#[cfg(test)]
mod lib_tests;
