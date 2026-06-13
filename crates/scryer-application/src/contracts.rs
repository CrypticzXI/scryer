use super::*;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmissionScope {
    Episode { episode_id: String },
    EpisodeSet { episode_ids: Vec<String> },
    SeriesMovie { series_movie_link_id: String },
    Collection { collection_id: String },
    Title,
    Orphan,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DownloadSubmissionPurpose {
    #[default]
    Standard,
    AdditionalFile,
}

impl DownloadSubmissionPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::AdditionalFile => "additional_file",
        }
    }

    pub fn from_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "additional_file" => Self::AdditionalFile,
            _ => Self::Standard,
        }
    }

    pub fn is_additional_file(self) -> bool {
        self == Self::AdditionalFile
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaFileRole {
    #[default]
    Primary,
    Additional,
}

impl MediaFileRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Additional => "additional",
        }
    }

    pub fn from_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "additional" => Self::Additional,
            _ => Self::Primary,
        }
    }

    pub fn is_primary(self) -> bool {
        self == Self::Primary
    }

    pub fn is_additional(self) -> bool {
        self == Self::Additional
    }
}

impl SubmissionScope {
    pub fn from_persisted(
        title_id: &str,
        episode_id: Option<String>,
        collection_id: Option<String>,
        series_movie_link_id: Option<String>,
        episode_set_ids: Option<Vec<String>>,
    ) -> Self {
        if let Some(mut episode_ids) = episode_set_ids {
            episode_ids.retain(|episode_id| !episode_id.trim().is_empty());
            episode_ids.sort();
            episode_ids.dedup();
            if !episode_ids.is_empty() {
                return Self::EpisodeSet { episode_ids };
            }
        }

        if let Some(episode_id) = episode_id {
            return Self::Episode { episode_id };
        }

        if let Some(series_movie_link_id) = series_movie_link_id {
            return Self::SeriesMovie {
                series_movie_link_id,
            };
        }

        if let Some(collection_id) = collection_id {
            return Self::Collection { collection_id };
        }

        if title_id.trim().is_empty() {
            Self::Orphan
        } else {
            Self::Title
        }
    }

    pub fn episode_id(&self) -> Option<&str> {
        match self {
            Self::Episode { episode_id } => Some(episode_id.as_str()),
            _ => None,
        }
    }

    pub fn collection_id(&self) -> Option<&str> {
        match self {
            Self::Collection { collection_id } => Some(collection_id.as_str()),
            _ => None,
        }
    }

    pub fn series_movie_link_id(&self) -> Option<&str> {
        match self {
            Self::SeriesMovie {
                series_movie_link_id,
            } => Some(series_movie_link_id.as_str()),
            _ => None,
        }
    }

    pub fn persisted_episode_id(&self) -> Option<&str> {
        self.episode_id()
    }

    pub fn persisted_collection_id(&self) -> Option<&str> {
        self.collection_id()
    }

    pub fn persisted_series_movie_link_id(&self) -> Option<&str> {
        self.series_movie_link_id()
    }

    pub fn episode_ids(&self) -> Option<&[String]> {
        match self {
            Self::EpisodeSet { episode_ids } => Some(episode_ids.as_slice()),
            Self::Episode { episode_id } => Some(std::slice::from_ref(episode_id)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DownloadSubmission {
    pub title_id: String,
    pub facet: String,
    pub download_client_id: Option<String>,
    pub download_client_type: String,
    pub download_client_item_id: String,
    pub source_hint: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
    pub request_signature: Option<String>,
    pub purpose: DownloadSubmissionPurpose,
    pub scope: SubmissionScope,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadSubmissionIdentity {
    pub download_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DownloadSourceIdentity {
    pub client_id: Option<String>,
    pub client_type: String,
    pub item_id: String,
}

impl DownloadSourceIdentity {
    pub fn new(
        client_id: Option<&str>,
        client_type: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> Self {
        let client_id = client_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Self {
            client_id,
            client_type: client_type.as_ref().trim().to_ascii_lowercase(),
            item_id: item_id.as_ref().trim().to_string(),
        }
    }

    pub fn from_submission(submission: &DownloadSubmission) -> Self {
        Self::new(
            submission.download_client_id.as_deref(),
            &submission.download_client_type,
            &submission.download_client_item_id,
        )
    }

    pub fn has_client_id(&self) -> bool {
        self.client_id.is_some()
    }

    pub fn client_id_or_empty(&self) -> &str {
        self.client_id.as_deref().unwrap_or("")
    }
}

#[derive(Clone, Debug)]
pub struct SuccessfulGrabCommit {
    pub wanted_item_id: String,
    pub covered_wanted_item_ids: Vec<String>,
    pub search_count: i64,
    pub current_score: Option<i32>,
    pub grabbed_release: String,
    pub last_search_at: Option<String>,
    pub download_submission: DownloadSubmission,
    pub download_submission_identity: Option<DownloadSubmissionIdentity>,
    pub grabbed_pending_release_id: Option<String>,
    pub grabbed_at: Option<String>,
}

/// Per-file import outcome history for completion verification across passes.
#[derive(Clone, Debug)]
pub struct ImportArtifact {
    pub id: String,
    pub source_client_id: Option<String>,
    pub source_system: String,
    pub source_ref: String,
    pub import_id: Option<String>,
    pub relative_path: Option<String>,
    pub normalized_file_name: String,
    pub media_kind: String,
    pub title_id: Option<String>,
    pub episode_id: Option<String>,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub result: String,
    pub reason_code: Option<String>,
    pub imported_media_file_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ImportArtifact {
    pub fn source_identity(&self) -> DownloadSourceIdentity {
        DownloadSourceIdentity::new(
            self.source_client_id.as_deref(),
            &self.source_system,
            &self.source_ref,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedNzbRef {
    pub id: String,
    pub compressed_path: PathBuf,
    pub raw_size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct PendingStagedNzb {
    pub id: String,
    pub compressed_path: PathBuf,
    pub partial_path: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct IndexerConfigUpdate {
    pub id: String,
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub derived_base_url: Option<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_auto_search: Option<bool>,
    pub managed_parent_config_id: Option<Option<String>>,
    pub managed_child_key: Option<Option<String>>,
    pub managed_metadata_json: Option<Option<String>>,
    pub caps_snapshot_json: Option<Option<String>>,
    pub config_json: Option<String>,
}

impl IndexerConfigUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.provider_type.is_some()
            || self.derived_base_url.is_some()
            || self.rate_limit_seconds.is_some()
            || self.rate_limit_burst.is_some()
            || self.is_enabled.is_some()
            || self.enable_interactive_search.is_some()
            || self.enable_auto_search.is_some()
            || self.managed_parent_config_id.is_some()
            || self.managed_child_key.is_some()
            || self.managed_metadata_json.is_some()
            || self.caps_snapshot_json.is_some()
            || self.config_json.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DownloadClientConfigUpdate {
    pub id: String,
    pub name: Option<String>,
    pub client_type: Option<String>,
    pub config_json: Option<String>,
    pub is_enabled: Option<bool>,
}

impl DownloadClientConfigUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.client_type.is_some()
            || self.config_json.is_some()
            || self.is_enabled.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct SubtitleProviderConfigUpdate {
    pub id: String,
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub config_json: Option<String>,
    pub enabled_facets: Option<Vec<String>>,
    pub is_enabled: Option<bool>,
    pub last_health_status: Option<String>,
    pub last_error: Option<Option<String>>,
    pub last_error_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub disabled_until: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

impl SubtitleProviderConfigUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.provider_type.is_some()
            || self.config_json.is_some()
            || self.enabled_facets.is_some()
            || self.is_enabled.is_some()
            || self.last_health_status.is_some()
            || self.last_error.is_some()
            || self.last_error_at.is_some()
            || self.disabled_until.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubtitleProviderValidationResult {
    pub status: String,
    pub message: Option<String>,
    pub retry_after_seconds: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerValidationResult {
    pub status: String,
    pub message: Option<String>,
    pub retry_after_seconds: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedIndexerRoutingScope {
    pub scope_id: String,
    pub categories: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedIndexerChildPlan {
    pub child_key: String,
    pub name: String,
    pub provider_type: String,
    pub config_json: String,
    pub is_enabled: bool,
    pub enable_interactive_search: bool,
    pub enable_auto_search: bool,
    pub managed_metadata_json: Option<String>,
    pub caps_snapshot_json: Option<String>,
    pub routing_scopes: Vec<ManagedIndexerRoutingScope>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerSyncPlan {
    pub children: Vec<ManagedIndexerChildPlan>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerConfigSyncResult {
    pub parent_config_id: String,
    pub created_ids: Vec<String>,
    pub updated_ids: Vec<String>,
    pub deleted_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleGenerationInput {
    pub media_kind: String,
    pub facet: Option<String>,
    pub input_path: PathBuf,
    pub mime_type: String,
    pub duration_seconds: i64,
    pub size_bytes: i64,
    pub checksum: String,
    pub languages: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueuedReleaseSelection {
    pub source_hint: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionConflictPolicy {
    Abort,
    Skip,
    ReplaceEarly,
}

impl SubmissionConflictPolicy {
    pub fn from_replace_flag(replace_in_progress: bool) -> Self {
        if replace_in_progress {
            Self::ReplaceEarly
        } else {
            Self::Abort
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionScopeConflict {
    pub title_id: String,
    pub title_name: String,
    pub download_client_id: Option<String>,
    pub download_client_type: String,
    pub download_client_item_id: String,
    pub source_title: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub scope: SubmissionScope,
    pub state: Option<DownloadQueueState>,
    pub replaceable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedDownloadResult {
    pub job_id: String,
    pub queued_release: QueuedReleaseSelection,
    pub reused_existing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueDownloadOutcome {
    Queued(QueuedDownloadResult),
    Conflict(SubmissionScopeConflict),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WantedSearchOutcome {
    pub queued_count: usize,
    pub skipped_in_progress_count: usize,
    pub conflict: Option<SubmissionScopeConflict>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollectionUpdate {
    pub collection_type: Option<CollectionType>,
    pub collection_index: Option<String>,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
    pub clear_ordered_path: bool,
    pub first_episode_number: Option<String>,
    pub last_episode_number: Option<String>,
    pub monitored: Option<bool>,
}

impl CollectionUpdate {
    pub fn has_changes(&self) -> bool {
        self.collection_type.is_some()
            || self.collection_index.is_some()
            || self.label.is_some()
            || self.ordered_path.is_some()
            || self.clear_ordered_path
            || self.first_episode_number.is_some()
            || self.last_episode_number.is_some()
            || self.monitored.is_some()
    }

    pub fn has_non_monitor_changes(&self) -> bool {
        self.collection_type.is_some()
            || self.collection_index.is_some()
            || self.label.is_some()
            || self.ordered_path.is_some()
            || self.clear_ordered_path
            || self.first_episode_number.is_some()
            || self.last_episode_number.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpisodeUpdate {
    pub episode_type: Option<scryer_domain::EpisodeType>,
    pub episode_number: Option<String>,
    pub season_number: Option<String>,
    pub episode_label: Option<String>,
    pub title: Option<String>,
    pub air_date: Option<String>,
    pub duration_seconds: Option<i64>,
    pub has_multi_audio: Option<bool>,
    pub has_subtitle: Option<bool>,
    pub monitored: Option<bool>,
    pub collection_id: Option<String>,
    pub overview: Option<String>,
    pub tvdb_id: Option<String>,
    pub image_url: Option<String>,
    pub clear_image_url: bool,
}

impl EpisodeUpdate {
    pub fn has_changes(&self) -> bool {
        self.episode_type.is_some()
            || self.episode_number.is_some()
            || self.season_number.is_some()
            || self.episode_label.is_some()
            || self.title.is_some()
            || self.air_date.is_some()
            || self.duration_seconds.is_some()
            || self.has_multi_audio.is_some()
            || self.has_subtitle.is_some()
            || self.monitored.is_some()
            || self.collection_id.is_some()
            || self.overview.is_some()
            || self.tvdb_id.is_some()
            || self.image_url.is_some()
            || self.clear_image_url
    }

    pub fn has_non_monitor_changes(&self) -> bool {
        self.episode_type.is_some()
            || self.episode_number.is_some()
            || self.season_number.is_some()
            || self.episode_label.is_some()
            || self.title.is_some()
            || self.air_date.is_some()
            || self.duration_seconds.is_some()
            || self.has_multi_audio.is_some()
            || self.has_subtitle.is_some()
            || self.collection_id.is_some()
            || self.overview.is_some()
            || self.tvdb_id.is_some()
            || self.image_url.is_some()
            || self.clear_image_url
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum NotificationScopeIdUpdate {
    #[default]
    NoChange,
    Clear,
    Set(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteExecutionConfirmation {
    pub preview_fingerprint: String,
    pub typed_confirmation: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WantedItemsQuery {
    pub statuses: Vec<String>,
    pub media_types: Vec<String>,
    pub title_id: Option<String>,
    pub library_ids: Vec<String>,
    pub title_search: Option<String>,
    pub latest_decision_codes: Vec<String>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseDecisionsQuery {
    pub wanted_item_id: Option<String>,
    pub title_id: Option<String>,
    pub limit: i64,
}

/// Parsed media properties from media analysis — application-layer DTO.
/// A single audio stream, mirroring `scryer_mediainfo::AudioStreamDetail`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioStreamDetail {
    pub codec: Option<String>,
    pub profile: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub bitrate_kbps: Option<i32>,
}

/// A single subtitle stream, mirroring `scryer_mediainfo::SubtitleStreamDetail`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubtitleStreamDetail {
    pub codec: Option<String>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub forced: bool,
    pub default: bool,
}

/// Mirrors `scryer_mediainfo::MediaAnalysis` without depending on that crate.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MediaFileAnalysis {
    pub video_codec: Option<crate::release_parser::VideoCodec>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    pub video_frame_rate: Option<String>,
    pub video_profile: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_profile: Option<String>,
    pub audio_channels: Option<i32>,
    pub audio_bitrate_kbps: Option<i32>,
    pub audio_languages: Vec<String>,
    pub audio_streams: Vec<AudioStreamDetail>,
    pub subtitle_languages: Vec<String>,
    pub subtitle_codecs: Vec<String>,
    pub subtitle_streams: Vec<SubtitleStreamDetail>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub num_chapters: Option<i32>,
    pub container_format: Option<String>,
}

#[derive(Clone, Debug)]
pub enum MediaAnalysisOutcome {
    Valid(Box<MediaFileAnalysis>),
    Invalid(String),
}

/// Input for inserting a media file record with rich metadata.
#[derive(Clone, Debug, Default)]
pub struct InsertMediaFileInput {
    pub title_id: String,
    pub file_path: String,
    pub size_bytes: i64,
    pub role: MediaFileRole,
    pub source_signature_scheme: Option<String>,
    pub source_signature_value: Option<String>,
    pub quality_label: Option<String>,
    pub scene_name: Option<String>,
    pub release_group: Option<String>,
    pub source_type: Option<String>,
    pub resolution: Option<String>,
    pub video_codec_parsed: Option<crate::release_parser::VideoCodec>,
    pub audio_codec_parsed: Option<String>,
    pub audio_channels_parsed: Option<String>,
    pub acquisition_score: Option<i32>,
    pub scoring_log: Option<String>,
    pub indexer_source: Option<String>,
    pub grabbed_release_title: Option<String>,
    pub grabbed_at: Option<String>,
    pub edition: Option<String>,
    pub original_file_path: Option<String>,
    pub release_hash: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TitleHistoryFilter {
    pub event_types: Option<Vec<TitleHistoryEventType>>,
    pub title_ids: Option<Vec<String>>,
    pub library_ids: Option<Vec<String>>,
    pub title_search: Option<String>,
    pub download_id: Option<String>,
    pub episode_id: Option<String>,
    pub group_by_event: bool,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug)]
pub struct TitleHistoryPage {
    pub records: Vec<TitleHistoryRecord>,
    pub total_count: i64,
}

#[derive(Clone, Debug)]
pub struct NewBlocklistEntry {
    pub title_id: String,
    pub source_title: Option<String>,
    pub source_hint: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub reason: Option<String>,
    pub data: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    Interactive,
    Auto,
}

/// Per-indexer routing entry resolved from the `indexer.routing:<scope>` setting.
#[derive(Clone, Debug)]
pub struct IndexerRoutingEntry {
    pub enabled: bool,
    pub categories: Vec<String>,
    pub priority: i64,
}

/// Per-indexer routing plan for a given facet scope.
/// When `Some`, indexers not in the map use default behavior; indexers
/// with `enabled: false` are skipped entirely for this scope.
#[derive(Clone, Debug)]
pub struct IndexerRoutingPlan {
    pub entries: std::collections::HashMap<String, IndexerRoutingEntry>,
}

#[derive(Clone, Debug)]
pub struct DownloadClientAddRequest {
    pub title: Title,
    pub purpose: DownloadSubmissionPurpose,
    pub download_id: Option<String>,
    pub source_hint: Option<String>,
    pub staged_nzb: Option<StagedNzbRef>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
    pub source_password: Option<String>,
    pub category: Option<String>,
    pub queue_priority: Option<String>,
    pub download_directory: Option<String>,
    pub release_title: Option<String>,
    pub indexer_name: Option<String>,
    pub info_hash_hint: Option<String>,
    pub seed_goal_ratio: Option<f64>,
    pub seed_goal_seconds: Option<i64>,
    pub is_recent: Option<bool>,
    pub season_pack: Option<bool>,
}

impl DownloadClientAddRequest {
    pub fn from_legacy(
        title: &Title,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> Self {
        Self {
            title: title.clone(),
            purpose: DownloadSubmissionPurpose::Standard,
            download_id: None,
            source_hint,
            staged_nzb: None,
            source_kind,
            source_title,
            source_password,
            category,
            queue_priority: None,
            download_directory: None,
            release_title: None,
            indexer_name: None,
            info_hash_hint: None,
            seed_goal_ratio: None,
            seed_goal_seconds: None,
            is_recent: None,
            season_pack: None,
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DownloadClientStatus {
    pub version: Option<String>,
    pub is_localhost: Option<bool>,
    pub remote_output_roots: Vec<String>,
    pub removes_completed_downloads: Option<bool>,
    pub sorting_mode: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DownloadClientMarkImportedRequest {
    pub client_item_id: String,
    pub info_hash: Option<String>,
    pub title_id: Option<String>,
    pub title_name: Option<String>,
    pub category: Option<String>,
    pub imported_path: Option<String>,
    pub download_path: Option<String>,
}
