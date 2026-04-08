use super::*;

#[derive(Clone, Debug)]
pub struct DownloadSubmission {
    pub title_id: String,
    pub facet: String,
    pub download_client_type: String,
    pub download_client_item_id: String,
    pub source_title: Option<String>,
    pub collection_id: Option<String>,
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

/// Parsed media properties from media analysis — application-layer DTO.
/// A single audio stream, mirroring `scryer_mediainfo::AudioStreamDetail`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioStreamDetail {
    pub codec: Option<String>,
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
