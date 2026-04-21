use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmissionScope {
    Episode { episode_id: String },
    Collection { collection_id: String },
    Title,
    Orphan,
}

impl SubmissionScope {
    pub fn from_persisted(
        title_id: &str,
        episode_id: Option<String>,
        collection_id: Option<String>,
    ) -> Self {
        if let Some(episode_id) = episode_id {
            return Self::Episode { episode_id };
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

    pub fn persisted_episode_id(&self) -> Option<&str> {
        self.episode_id()
    }

    pub fn persisted_collection_id(&self) -> Option<&str> {
        self.collection_id()
    }
}

#[derive(Clone, Debug)]
pub struct DownloadSubmission {
    pub title_id: String,
    pub facet: String,
    pub download_client_type: String,
    pub download_client_item_id: String,
    pub source_hint: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
    pub request_signature: Option<String>,
    pub scope: SubmissionScope,
}

#[derive(Clone, Debug)]
pub struct SuccessfulGrabCommit {
    pub wanted_item_id: String,
    pub search_count: i64,
    pub current_score: Option<i32>,
    pub grabbed_release: String,
    pub last_search_at: Option<String>,
    pub download_submission: DownloadSubmission,
    pub grabbed_pending_release_id: Option<String>,
    pub grabbed_at: Option<String>,
}

/// Per-file import outcome history for completion verification across passes.
#[derive(Clone, Debug)]
pub struct ImportArtifact {
    pub id: String,
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
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_auto_search: Option<bool>,
    pub config_json: Option<String>,
}

impl IndexerConfigUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.provider_type.is_some()
            || self.base_url.is_some()
            || self.api_key_encrypted.is_some()
            || self.rate_limit_seconds.is_some()
            || self.rate_limit_burst.is_some()
            || self.is_enabled.is_some()
            || self.enable_interactive_search.is_some()
            || self.enable_auto_search.is_some()
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerSearchRequest {
    pub query: String,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub anidb_id: Option<String>,
    pub category: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerEpisodeSearchRequest {
    pub title: String,
    pub season: String,
    pub episode: String,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub anidb_id: Option<String>,
    pub category: Option<String>,
    pub absolute_episode: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerSeasonSearchRequest {
    pub title: String,
    pub season: String,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub anidb_id: Option<String>,
    pub category: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueuedReleaseSelection {
    pub source_hint: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollectionUpdate {
    pub collection_type: Option<CollectionType>,
    pub collection_index: Option<String>,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
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
            || self.first_episode_number.is_some()
            || self.last_episode_number.is_some()
            || self.monitored.is_some()
    }

    pub fn has_non_monitor_changes(&self) -> bool {
        self.collection_type.is_some()
            || self.collection_index.is_some()
            || self.label.is_some()
            || self.ordered_path.is_some()
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
    pub status: Option<String>,
    pub media_type: Option<String>,
    pub title_id: Option<String>,
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
#[derive(Clone, Debug)]
pub struct MediaFileAnalysis {
    pub video_codec: Option<String>,
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
    pub raw_json: String,
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
    pub source_signature_scheme: Option<String>,
    pub source_signature_value: Option<String>,
    pub quality_label: Option<String>,
    pub scene_name: Option<String>,
    pub release_group: Option<String>,
    pub source_type: Option<String>,
    pub resolution: Option<String>,
    pub video_codec_parsed: Option<String>,
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

#[derive(Clone, Debug)]
pub struct NewTitleHistoryEvent {
    pub title_id: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub event_type: TitleHistoryEventType,
    pub source_title: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub data: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default)]
pub struct TitleHistoryFilter {
    pub event_types: Option<Vec<TitleHistoryEventType>>,
    pub title_ids: Option<Vec<String>>,
    pub download_id: Option<String>,
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
