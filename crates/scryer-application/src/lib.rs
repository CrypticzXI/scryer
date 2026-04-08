#![allow(clippy::too_many_arguments)]

#[path = "acquisition/decision_helpers.rs"]
mod acquisition_decision_helpers;
#[path = "acquisition/policy.rs"]
mod acquisition_policy;
#[path = "acquisition/search_queries.rs"]
mod acquisition_search_queries;
#[path = "acquisition/acquisition.rs"]
mod acquisition_workflow;
#[path = "events/activity.rs"]
mod activity;
#[path = "events/activity_api.rs"]
mod app_usecase_activity;
#[path = "security/admin.rs"]
mod app_usecase_admin;
#[path = "security/backup.rs"]
mod app_usecase_backup;
#[path = "catalog/discovery.rs"]
mod app_usecase_discovery;
#[path = "health/health.rs"]
mod app_usecase_health;
#[path = "jobs/housekeeping.rs"]
mod app_usecase_housekeeping;
#[path = "integration/indexer_test.rs"]
mod app_usecase_indexer_test;
#[path = "integration/integration.rs"]
mod app_usecase_integration;
#[path = "jobs/jobs.rs"]
mod app_usecase_jobs;
#[path = "notifications/notifications.rs"]
mod app_usecase_notifications;
#[path = "acquisition/pending.rs"]
mod app_usecase_pending;
#[path = "plugins/plugins.rs"]
mod app_usecase_plugins;
#[path = "import/post_processing.rs"]
pub mod app_usecase_post_processing;
#[path = "acquisition/rss.rs"]
pub(crate) mod app_usecase_rss;
#[path = "rules/rules.rs"]
mod app_usecase_rules;
#[path = "security/security.rs"]
mod app_usecase_security;
#[path = "settings/settings.rs"]
mod app_usecase_settings;
#[path = "subtitles/orchestration.rs"]
mod app_usecase_subtitles;
#[path = "catalog/title_images.rs"]
mod app_usecase_title_images;
#[path = "import/archive_extractor.rs"]
pub(crate) mod archive_extractor;
#[path = "media/audio_requirements.rs"]
mod audio_requirements;
#[path = "catalog/helpers.rs"]
mod catalog_helpers;
#[path = "catalog/catalog.rs"]
mod catalog_workflow;
#[path = "import/completed_download.rs"]
pub mod completed_download_handler;
mod contracts;
#[path = "acquisition/delay_profile.rs"]
mod delay_profile;
#[path = "events/domain_events.rs"]
mod domain_events;
#[path = "events/event_views.rs"]
mod event_views;
#[path = "catalog/facets/handler.rs"]
pub(crate) mod facet_handler;
#[path = "catalog/facets/movie.rs"]
mod facet_movie;
#[path = "catalog/facets/registry.rs"]
mod facet_registry;
#[path = "catalog/facets/series.rs"]
mod facet_series;
#[path = "import/failed_download.rs"]
pub mod failed_download_handler;
#[path = "library/filesystem_walk.rs"]
pub mod filesystem_walk;
mod helpers;
#[path = "import/checks.rs"]
pub(crate) mod import_checks;
#[path = "import/parameters.rs"]
mod import_parameters;
#[path = "import/title_resolution.rs"]
mod import_title_resolution;
#[path = "import/import.rs"]
mod import_workflow;
#[path = "jobs/definitions.rs"]
mod jobs;
#[path = "library/discovery.rs"]
mod library_discovery;
#[path = "library/rename.rs"]
mod library_rename;
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
#[path = "library/library.rs"]
pub(crate) mod library_workflow;
#[path = "plugins/managed_rules.rs"]
pub mod managed_rules;
#[path = "media/analyzer.rs"]
mod media_analyzer;
#[path = "media/language.rs"]
mod media_language;
#[path = "media/language_data.rs"]
mod media_language_data;
#[path = "library/nfo.rs"]
pub(crate) mod nfo;
pub(crate) mod normalize;
#[path = "notifications/dispatcher.rs"]
mod notification_dispatcher;
mod null_repositories;
mod ports;
#[path = "import/post_download_gate.rs"]
mod post_download_gate;
#[path = "quality/profile.rs"]
mod quality_profile;
#[path = "library/recycle_bin.rs"]
pub mod recycle_bin;
#[path = "quality/release_dedup.rs"]
pub mod release_dedup;
#[path = "quality/release_group_db.rs"]
mod release_group_db;
#[path = "quality/release_parser.rs"]
mod release_parser;
#[path = "quality/scoring_weights.rs"]
mod scoring_weights;
mod services;
#[path = "settings/keys.rs"]
mod settings_keys;
pub mod subtitles;
#[path = "library/title_matching.rs"]
mod title_matching;
#[path = "integration/tracked_downloads.rs"]
pub mod tracked_downloads;
mod types;
#[path = "import/upgrade.rs"]
pub mod upgrade;
#[path = "library/user_delete.rs"]
mod user_delete;
#[path = "rules/user_rule_input.rs"]
mod user_rule_input;

use chrono::{DateTime, Duration, Utc};
use rand_core::OsRng;
use ring::digest as ring_digest;
use scryer_domain::{
    BlocklistEntry, CalendarEpisode, Collection, CollectionType, CompletedDownload, DomainEvent,
    DomainEventFilter, DomainEventType, DownloadClientConfig, DownloadQueueItem,
    DownloadQueueState, Entitlement, Episode, ExternalId, HistoryEvent, Id, ImportFileResult,
    ImportRecord, ImportResult, ImportStatus, IndexerConfig, MediaFacet, NewDomainEvent,
    NewDownloadClientConfig, NewIndexerConfig, NewTitle, PluginInstallation, PolicyInput,
    PolicyOutput, RuleSet, TaggedAlias, Title, TitleHistoryEventType, TitleHistoryRecord, User,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell, RwLock, Semaphore, broadcast};

pub type AppResult<T> = Result<T, AppError>;

use crate::quality_profile::resolve_profile_id_for_title;
pub use acquisition_policy::AcquisitionThresholds;
pub use acquisition_workflow::start_background_acquisition_poller;
pub use activity::{ActivityChannel, ActivityEvent, ActivityKind, ActivitySeverity};
pub use app_usecase_backup::BackupService;
pub use app_usecase_integration::start_download_queue_poller;
pub use app_usecase_jobs::start_background_library_refresh_loop;
pub use app_usecase_plugins::{RegistryPlugin, RulePackRegistryEntry, RulePackTemplate};
pub use app_usecase_post_processing::{PostProcessingContext, run_post_processing};
pub use app_usecase_rss::RssSyncReport;
pub use app_usecase_settings::{AcquisitionSettings, SubtitleSettings, UpdateSubtitleSettings};
pub use app_usecase_subtitles::{spawn_subtitle_search_for_file, start_background_subtitle_poller};
pub use app_usecase_title_images::start_background_banner_loop;
pub use app_usecase_title_images::start_background_fanart_loop;
pub use app_usecase_title_images::start_background_poster_loop;
pub(crate) use audio_requirements::{
    missing_required_audio_languages, normalize_required_audio_languages,
    release_audio_language_hints,
};
pub use catalog_workflow::{
    DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
};
pub use contracts::{
    AudioStreamDetail, DownloadClientAddRequest, DownloadClientMarkImportedRequest,
    DownloadClientStatus, DownloadSubmission, ImportArtifact, IndexerRoutingEntry,
    IndexerRoutingPlan, InsertMediaFileInput, MediaAnalysisOutcome, MediaFileAnalysis,
    NewBlocklistEntry, NewTitleHistoryEvent, PendingStagedNzb, SearchMode, StagedNzbRef,
    SubtitleStreamDetail, SuccessfulGrabCommit, TitleHistoryFilter, TitleHistoryPage,
};
pub use delay_profile::{
    DELAY_PROFILE_CATALOG_KEY, DelayDecision, DelayProfile, PreferredProtocol, is_usenet_source,
    parse_delay_profile_catalog, resolve_delay_decision, resolve_delay_profile,
    validate_delay_profile_catalog,
};
pub use event_views::{
    apply_download_queue_projection_event, apply_job_next_run_projection_event,
    apply_job_run_projection_event, apply_library_scan_projection_event, replay_active_job_runs,
    replay_download_queue_state, replay_job_next_runs, replay_library_scan_state,
    sorted_download_queue_items,
};
pub use facet_handler::{
    FacetHandler, HydrationResult, movie_to_hydration_result, series_to_hydration_result,
};
pub use facet_movie::MovieFacetHandler;
pub use facet_registry::FacetRegistry;
pub use facet_series::SeriesFacetHandler;
pub use import_workflow::{
    ManualImportFileMapping, ManualImportFilePreview, ManualImportFileResult, ManualImportPreview,
    execute_manual_import, import_completed_download, preview_manual_import, retry_failed_import,
    try_import_completed_downloads,
};
pub use library_rename::{
    LibraryRenamer, NullLibraryRenamer, RenameApplyItemResult, RenameApplyResult,
    RenameApplyStatus, RenameCollisionPolicy, RenameMissingMetadataPolicy, RenamePlan,
    RenamePlanItem, RenameWriteAction, build_rename_plan_fingerprint, render_rename_template,
};
pub use media_language::{
    normalize_detected_audio_language_code, normalize_detected_audio_languages,
    normalize_detected_subtitle_language_code, normalize_detected_subtitle_languages,
};

pub(crate) const GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY: usize = 4;
pub use app_usecase_integration::publish_download_queue_snapshot_events;
pub(crate) use helpers::{
    INDEXER_PROVIDER_NZBGEEK, INHERIT_QUALITY_PROFILE_VALUE, NATIVE_DOWNLOAD_CLIENT_TYPES,
    SETTINGS_SCOPE_MEDIA, SETTINGS_SCOPE_SYSTEM, normalize_release_attempt_hint,
    normalize_release_attempt_title, normalize_release_password, normalize_show_text_opt,
    normalize_tags, parsed_episode_lookup_season, require, sanitize_ids, sha256_hex, statvfs_path,
    to_hex,
};
pub use helpers::{accepted_inputs_for_client, nice_thread};
pub use jobs::{
    JobCategory, JobDefinition, JobKey, JobRun, JobRunRecord, JobRunStatus, JobRunTracker,
    JobScheduleInfo, JobScheduleKind, JobSection, JobTriggerSource, LibraryProbeSignature,
};
pub use library_scan::{
    AnibridgeSourceMapping, AnimeEpisodeMapping, AnimeMapping, AnimeMovie, BulkMetadataResult,
    EpisodeMetadata, LibraryDirectoryScanResult, LibraryFile, LibraryFileBatch,
    LibraryFileBatchReceiver, LibraryScanSummary, LibraryScanner, MetadataGateway,
    MetadataSearchItem, MetadataSearchQuery, MovieMetadata, MultiMetadataSearchResult,
    RichMetadataSearchItem, SeasonMetadata, SeriesMetadata, source_signature_from_std_metadata,
};
pub use library_scan_progress::{
    LibraryScanMode, LibraryScanPhaseProgress, LibraryScanSession, LibraryScanStatus,
    LibraryScanTracker,
};
pub use media_analyzer::NativeMediaAnalyzer;
pub use notification_dispatcher::start_notification_dispatcher;
pub use null_repositories::{
    NullAcquisitionStateRepository, NullBlocklistRepository, NullDomainEventRepository,
    NullDownloadSubmissionRepository, NullFileImporter, NullHousekeepingRepository,
    NullImportRepository, NullIndexerStatsTracker, NullJobRunRepository,
    NullLibraryProbeRepository, NullLibraryScanUnmatchedItemRepository, NullMediaFileRepository,
    NullNotificationChannelRepository, NullNotificationSubscriptionRepository,
    NullPendingReleaseRepository, NullPluginInstallationRepository,
    NullPostProcessingScriptRepository, NullRuleSetRepository, NullSettingsRepository,
    NullStagedNzbStore, NullSystemInfoProvider, NullTitleHistoryRepository,
    NullTitleImageProcessor, NullTitleImageRepository, NullWantedItemRepository,
};
pub use ports::{
    AcquisitionStateRepository, BlocklistRepository, DomainEventRepository, DownloadClient,
    DownloadClientConfigRepository, DownloadClientPluginProvider, DownloadSubmissionRepository,
    FileImporter, HousekeepingRepository, ImportArtifactRepository, ImportRepository,
    IndexerClient, IndexerConfigRepository, IndexerPluginProvider, IndexerStatsTracker,
    JobRunRepository, LibraryProbeRepository, LibraryScanUnmatchedItemRepository, MediaAnalyzer,
    MediaFileRepository, NotificationChannelRepository, NotificationClient,
    NotificationPluginProvider, NotificationSubscriptionRepository, PendingReleaseRepository,
    PluginInstallationRepository, PostProcessingScriptRepository, QualityProfileRepository,
    ReleaseAttemptRepository, RuleSetRepository, SettingsRepository, ShowRepository,
    StagedNzbStore, SubtitleDownloadRepository, SystemInfoProvider, TitleHistoryRepository,
    TitleImageProcessor, TitleImageRepository, TitleRepository, UserRepository,
    WantedItemRepository,
};
pub use quality_profile::{
    BLOCK_SCORE, QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY,
    QUALITY_PROFILE_INHERIT_VALUE, QualityProfile, QualityProfileCriteria, QualityProfileDecision,
    ScoringConfig, ScoringEntry, ScoringSource, apply_age_scoring, apply_size_scoring_for_category,
    default_quality_profile_1080p_for_search, default_quality_profile_for_search,
    evaluate_against_profile, parse_profile_catalog_from_json,
};
pub use release_parser::{
    ParsedEpisodeMetadata, ParsedEpisodeReleaseType, ParsedReleaseMetadata, ParsedSpecialKind,
    parse_release_metadata,
};
pub use scoring_weights::{
    ScoringOverrides, ScoringPersona, ScoringWeights, build_weights, build_weights_for_category,
};
pub use services::{AppServices, AppUseCase};
pub use settings_keys::{
    AUDIO_PERSONA_MIGRATION_SENTINEL_KEY, REQUIRED_AUDIO_LANGUAGES_KEY, SCORING_PERSONA_KEY,
    TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
};
pub(crate) use types::JwtClaims;
pub use types::{
    BackupInfo, DiskSpaceInfo, DownloadGrabResult, DownloadHistoryPage, DownloadSourceKind,
    FixTitleMatchResult, HealthCheckResult, HealthCheckStatus, HousekeepingReport,
    IndexerQueryStats, IndexerSearchResponse, IndexerSearchResult, JwtAuthConfig,
    LibraryScanUnmatchedItem, LibraryScanUnmatchedSearchAttempt, PendingRelease,
    PendingReleaseStatus, PrimaryCollectionSummary, ReleaseDecision, ReleaseDownloadAttemptOutcome,
    ReleaseDownloadFailureSignature, SystemHealth, TitleEpisodeProgressSummary, TitleImageBlob,
    TitleImageKind, TitleImageReplacement, TitleImageStorageMode, TitleImageSyncTask,
    TitleImageVariantRecord, TitleMediaFile, TitleMediaSizeSummary, TitleMetadataUpdate,
    TitleReleaseBlocklistEntry, WantedCompleteTransition, WantedGrabTransition, WantedItem,
    WantedPauseTransition, WantedSearchTransition, WantedStatus,
};
pub use user_delete::DeletePreview;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("validation: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("repository: {0}")]
    Repository(String),
}

#[cfg(test)]
mod lib_tests;
