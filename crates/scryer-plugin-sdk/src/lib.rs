use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};

pub const SDK_VERSION: &str = "1.0.0";

pub const EXPORT_DESCRIBE: &str = "scryer_describe";
pub const EXPORT_VALIDATE_CONFIG: &str = "scryer_validate_config";
pub const EXPORT_INDEXER_SEARCH: &str = "scryer_indexer_search";
pub const EXPORT_DOWNLOAD_ADD: &str = "scryer_download_add";
pub const EXPORT_DOWNLOAD_LIST_QUEUE: &str = "scryer_download_list_queue";
pub const EXPORT_DOWNLOAD_LIST_HISTORY: &str = "scryer_download_list_history";
pub const EXPORT_DOWNLOAD_LIST_COMPLETED: &str = "scryer_download_list_completed";
pub const EXPORT_DOWNLOAD_CONTROL: &str = "scryer_download_control";
pub const EXPORT_DOWNLOAD_MARK_IMPORTED: &str = "scryer_download_mark_imported";
pub const EXPORT_DOWNLOAD_STATUS: &str = "scryer_download_status";
pub const EXPORT_DOWNLOAD_TEST_CONNECTION: &str = "scryer_download_test_connection";
pub const EXPORT_NOTIFICATION_SEND: &str = "scryer_notification_send";
pub const EXPORT_SUBTITLE_SEARCH: &str = "scryer_subtitle_search";
pub const EXPORT_SUBTITLE_DOWNLOAD: &str = "scryer_subtitle_download";
pub const EXPORT_SUBTITLE_GENERATE: &str = "scryer_subtitle_generate";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    Indexer,
    DownloadClient,
    Notification,
    SubtitleProvider,
}

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Indexer => "indexer",
            Self::DownloadClient => "download_client",
            Self::Notification => "notification",
            Self::SubtitleProvider => "subtitle_provider",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexerSourceKind {
    #[default]
    Generic,
    Usenet,
    Torrent,
}

impl IndexerSourceKind {
    pub fn plugin_type(self) -> &'static str {
        match self {
            Self::Generic => "indexer",
            Self::Usenet => "usenet_indexer",
            Self::Torrent => "torrent_indexer",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub sdk_version: String,
    pub provider: ProviderDescriptor,
}

impl PluginDescriptor {
    pub fn kind(&self) -> PluginKind {
        match &self.provider {
            ProviderDescriptor::Indexer(_) => PluginKind::Indexer,
            ProviderDescriptor::DownloadClient(_) => PluginKind::DownloadClient,
            ProviderDescriptor::Notification(_) => PluginKind::Notification,
            ProviderDescriptor::Subtitle(_) => PluginKind::SubtitleProvider,
        }
    }

    pub fn plugin_type(&self) -> &'static str {
        match &self.provider {
            ProviderDescriptor::Indexer(indexer) => indexer.source_kind.plugin_type(),
            ProviderDescriptor::DownloadClient(_) => PluginKind::DownloadClient.as_str(),
            ProviderDescriptor::Notification(_) => PluginKind::Notification.as_str(),
            ProviderDescriptor::Subtitle(_) => PluginKind::SubtitleProvider.as_str(),
        }
    }

    pub fn provider_type(&self) -> &str {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.provider_type.as_str(),
            ProviderDescriptor::DownloadClient(provider) => provider.provider_type.as_str(),
            ProviderDescriptor::Notification(provider) => provider.provider_type.as_str(),
            ProviderDescriptor::Subtitle(provider) => provider.provider_type.as_str(),
        }
    }

    pub fn provider_aliases(&self) -> &[String] {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.provider_aliases.as_slice(),
            ProviderDescriptor::DownloadClient(provider) => provider.provider_aliases.as_slice(),
            ProviderDescriptor::Notification(provider) => provider.provider_aliases.as_slice(),
            ProviderDescriptor::Subtitle(provider) => provider.provider_aliases.as_slice(),
        }
    }

    pub fn config_fields(&self) -> &[ConfigFieldDef] {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.config_fields.as_slice(),
            ProviderDescriptor::DownloadClient(provider) => provider.config_fields.as_slice(),
            ProviderDescriptor::Notification(provider) => provider.config_fields.as_slice(),
            ProviderDescriptor::Subtitle(provider) => provider.config_fields.as_slice(),
        }
    }

    pub fn config_fields_mut(&mut self) -> &mut Vec<ConfigFieldDef> {
        match &mut self.provider {
            ProviderDescriptor::Indexer(provider) => &mut provider.config_fields,
            ProviderDescriptor::DownloadClient(provider) => &mut provider.config_fields,
            ProviderDescriptor::Notification(provider) => &mut provider.config_fields,
            ProviderDescriptor::Subtitle(provider) => &mut provider.config_fields,
        }
    }

    pub fn allowed_hosts(&self) -> &[String] {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.allowed_hosts.as_slice(),
            ProviderDescriptor::DownloadClient(provider) => provider.allowed_hosts.as_slice(),
            ProviderDescriptor::Notification(provider) => provider.allowed_hosts.as_slice(),
            ProviderDescriptor::Subtitle(provider) => provider.allowed_hosts.as_slice(),
        }
    }

    pub fn default_base_url(&self) -> Option<&str> {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.default_base_url.as_deref(),
            ProviderDescriptor::DownloadClient(provider) => provider.default_base_url.as_deref(),
            ProviderDescriptor::Notification(provider) => provider.default_base_url.as_deref(),
            ProviderDescriptor::Subtitle(provider) => provider.default_base_url.as_deref(),
        }
    }

    pub fn set_default_base_url(&mut self, value: Option<String>) {
        match &mut self.provider {
            ProviderDescriptor::Indexer(provider) => provider.default_base_url = value,
            ProviderDescriptor::DownloadClient(provider) => provider.default_base_url = value,
            ProviderDescriptor::Notification(provider) => provider.default_base_url = value,
            ProviderDescriptor::Subtitle(provider) => provider.default_base_url = value,
        }
    }

    pub fn indexer(&self) -> Option<&IndexerDescriptor> {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => Some(provider),
            _ => None,
        }
    }

    pub fn notification(&self) -> Option<&NotificationDescriptor> {
        match &self.provider {
            ProviderDescriptor::Notification(provider) => Some(provider),
            _ => None,
        }
    }

    pub fn download_client(&self) -> Option<&DownloadClientDescriptor> {
        match &self.provider {
            ProviderDescriptor::DownloadClient(provider) => Some(provider),
            _ => None,
        }
    }

    pub fn subtitle(&self) -> Option<&SubtitleDescriptor> {
        match &self.provider {
            ProviderDescriptor::Subtitle(provider) => Some(provider),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderDescriptor {
    Indexer(IndexerDescriptor),
    DownloadClient(DownloadClientDescriptor),
    Notification(NotificationDescriptor),
    Subtitle(SubtitleDescriptor),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IndexerDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default)]
    pub source_kind: IndexerSourceKind,
    #[serde(default)]
    pub capabilities: IndexerCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scoring_policies: Vec<PluginScoringPolicy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadClientDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_inputs: Vec<DownloadInputKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isolation_modes: Vec<DownloadIsolationMode>,
    #[serde(default)]
    pub capabilities: DownloadClientCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub capabilities: NotificationCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub capabilities: SubtitleCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginScoringPolicy {
    pub name: String,
    pub rego_source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applied_facets: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct IndexerCapabilities {
    #[serde(default = "default_true")]
    pub rss: bool,
    #[serde(default)]
    pub supported_ids: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub deduplicates_aliases: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_param: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_param: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_param: Option<String>,
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub imdb_search: bool,
    #[serde(default)]
    pub tvdb_search: bool,
    #[serde(default)]
    pub anidb_search: bool,
}

impl Default for IndexerCapabilities {
    fn default() -> Self {
        Self {
            rss: true,
            supported_ids: HashMap::new(),
            deduplicates_aliases: false,
            season_param: None,
            episode_param: None,
            query_param: None,
            search: false,
            imdb_search: false,
            tvdb_search: false,
            anidb_search: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct NotificationCapabilities {
    #[serde(default)]
    pub supports_rich_text: bool,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_events: Vec<NotificationEventType>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DownloadClientCapabilities {
    #[serde(default)]
    pub pause: bool,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub remove: bool,
    #[serde(default)]
    pub remove_with_data: bool,
    #[serde(default)]
    pub mark_imported: bool,
    #[serde(default)]
    pub prepare_for_import: bool,
    #[serde(default)]
    pub client_status: bool,
    #[serde(default)]
    pub queue_priority: bool,
    #[serde(default)]
    pub seed_limits: bool,
    #[serde(default)]
    pub start_paused: bool,
    #[serde(default)]
    pub force_start: bool,
    #[serde(default)]
    pub per_download_directory: bool,
    #[serde(default)]
    pub host_fs_required: bool,
    #[serde(default)]
    pub test_connection: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleProviderMode {
    #[default]
    Catalog,
    Generator,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleCapabilities {
    pub mode: SubtitleProviderMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_media_kinds: Vec<SubtitleQueryMediaKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommended_facets: Vec<String>,
    #[serde(default)]
    pub supports_hash_lookup: bool,
    #[serde(default)]
    pub supports_forced: bool,
    #[serde(default)]
    pub supports_hearing_impaired: bool,
    #[serde(default)]
    pub supports_ai_translated: bool,
    #[serde(default)]
    pub supports_machine_translated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_languages: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldType {
    #[default]
    String,
    #[serde(alias = "secret")]
    Password,
    Multiline,
    Bool,
    Select,
    Number,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldValueSource {
    #[default]
    User,
    HostBinding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum PluginHostBindingId {
    #[serde(rename = "smg.opensubtitles_api_key")]
    SmgOpenSubtitlesApiKey,
}

impl PluginHostBindingId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SmgOpenSubtitlesApiKey => "smg.opensubtitles_api_key",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ConfigFieldDef {
    pub key: String,
    pub label: String,
    pub field_type: ConfigFieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(default)]
    pub value_source: ConfigFieldValueSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_binding: Option<PluginHostBindingId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ConfigFieldOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ConfigFieldOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginErrorCode {
    InvalidConfig,
    AuthFailed,
    RateLimited,
    UpstreamUnavailable,
    Unsupported,
    Temporary,
    Permanent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginError {
    pub code: PluginErrorCode,
    pub public_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginResult<T> {
    Ok(T),
    Err(PluginError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadInputKind {
    Nzb,
    NzbUrl,
    TorrentFile,
    MagnetUri,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadIsolationMode {
    Category,
    Tag,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadItemState {
    Queued,
    Downloading,
    Verifying,
    Repairing,
    Extracting,
    Paused,
    Completed,
    ImportPending,
    Failed,
    Error,
    Warning,
    Seeding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadControlAction {
    Pause,
    Resume,
    Remove,
    ForceStart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEventType {
    Grab,
    Download,
    Upgrade,
    ImportComplete,
    ImportRejected,
    Rename,
    TitleAdded,
    TitleDeleted,
    FileDeleted,
    FileDeletedForUpgrade,
    PostProcessingCompleted,
    SubtitleDownloaded,
    SubtitleSearchFailed,
    HealthIssue,
    HealthRestored,
    ApplicationUpdate,
    ManualInteractionRequired,
    Test,
}

impl NotificationEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grab => "grab",
            Self::Download => "download",
            Self::Upgrade => "upgrade",
            Self::ImportComplete => "import_complete",
            Self::ImportRejected => "import_rejected",
            Self::Rename => "rename",
            Self::TitleAdded => "title_added",
            Self::TitleDeleted => "title_deleted",
            Self::FileDeleted => "file_deleted",
            Self::FileDeletedForUpgrade => "file_deleted_for_upgrade",
            Self::PostProcessingCompleted => "post_processing_completed",
            Self::SubtitleDownloaded => "subtitle_downloaded",
            Self::SubtitleSearchFailed => "subtitle_search_failed",
            Self::HealthIssue => "health_issue",
            Self::HealthRestored => "health_restored",
            Self::ApplicationUpdate => "application_update",
            Self::ManualInteractionRequired => "manual_interaction_required",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleValidateConfigStatus {
    Valid,
    InvalidConfig,
    AuthFailed,
    RateLimited,
    Unreachable,
    Unsupported,
    MissingHostBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleMatchHintKind {
    Hash,
    ImdbId,
    SeriesImdbId,
    ExternalId,
    AbsoluteEpisode,
    Release,
    Title,
    SeasonEpisode,
    Language,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleMatchHint {
    pub kind: SubtitleMatchHintKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleQueryMediaKind {
    Movie,
    Episode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginSearchRequest {
    pub media_kind: SubtitleQueryMediaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_imdb_id: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub title_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub title_candidates: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute_episode: Option<i32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub external_ids: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hearing_impaired: Option<bool>,
    #[serde(default)]
    pub include_ai_translated: bool,
    #[serde(default)]
    pub include_machine_translated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginCandidate {
    pub provider_file_id: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_info: Option<String>,
    #[serde(default)]
    pub hearing_impaired: bool,
    #[serde(default)]
    pub forced: bool,
    #[serde(default)]
    pub ai_translated: bool,
    #[serde(default)]
    pub machine_translated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploader: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_hints: Vec<SubtitleMatchHint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginSearchResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<SubtitlePluginCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginDownloadRequest {
    pub provider_file_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginDownloadResponse {
    pub content_base64: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginValidateConfigRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_instance_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginValidateConfigResponse {
    pub status: SubtitleValidateConfigStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleGeneratorInputRef {
    pub path: PathBuf,
    pub mime_type: String,
    pub duration_seconds: i64,
    pub size_bytes: i64,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginGenerateRequest {
    pub media_kind: SubtitleQueryMediaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
    pub input: SubtitleGeneratorInputRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginGenerateResponse {
    pub content_base64: String,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientAddRequest {
    pub source: PluginDownloadSource,
    pub release: PluginDownloadRelease,
    pub title: PluginDownloadTitle,
    pub routing: PluginDownloadRouting,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadSource {
    pub kind: DownloadInputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnet_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_bytes_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_password: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadRelease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_recent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_pack: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadTitle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    pub title_name: String,
    pub media_facet: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadRouting {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientAddResponse {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadItem {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    pub title: String,
    pub state: DownloadItemState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_output_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_move_files: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_remove: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginCompletedDownload {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    pub name: String,
    pub dest_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientControlRequest {
    pub action: DownloadControlAction,
    pub client_item_id: String,
    #[serde(default)]
    pub remove_data: bool,
    #[serde(default)]
    pub is_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientMarkImportedRequest {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_localhost: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_output_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removes_completed_downloads: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sorting_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationExternalIds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anidb_id: Option<String>,
}

impl PluginNotificationExternalIds {
    fn is_empty(&self) -> bool {
        self.tmdb_id.is_none()
            && self.imdb_id.is_none()
            && self.tvdb_id.is_none()
            && self.anidb_id.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationApp {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationTitle {
    pub name: String,
    pub facet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "PluginNotificationExternalIds::is_empty"
    )]
    pub external_ids: PluginNotificationExternalIds,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationEpisode {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episode_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationRelease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationDownload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationImport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationMediaUpdateType {
    Created,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationMediaUpdate {
    pub path: String,
    pub update_type: NotificationMediaUpdateType,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_updates: Vec<PluginNotificationMediaUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationRequest {
    pub event_type: NotificationEventType,
    pub summary_title: String,
    pub summary_message: String,
    pub app: PluginNotificationApp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<PluginNotificationTitle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<PluginNotificationEpisode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<PluginNotificationRelease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<PluginNotificationDownload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<PluginNotificationImport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<PluginNotificationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PluginNotificationFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationResponse {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginSearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub ids: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default)]
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute_episode: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tagged_aliases: Vec<TaggedAlias>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginSearchResponse {
    #[serde(default)]
    pub results: Vec<PluginSearchResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_current: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab_current: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab_max: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginSearchResult {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grabs: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbs_up: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbs_down: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected: Option<bool>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_extra: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaggedAlias {
    pub name: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct PluginSdkSchemaDocument {
    descriptor: PluginDescriptor,
    indexer_search_request: PluginSearchRequest,
    indexer_search_response: PluginSearchResponse,
    subtitle_search_request: SubtitlePluginSearchRequest,
    subtitle_search_result: PluginResult<SubtitlePluginSearchResponse>,
    subtitle_download_request: SubtitlePluginDownloadRequest,
    subtitle_download_result: PluginResult<SubtitlePluginDownloadResponse>,
    subtitle_validate_config_request: SubtitlePluginValidateConfigRequest,
    subtitle_validate_config_result: PluginResult<SubtitlePluginValidateConfigResponse>,
    subtitle_generate_request: SubtitlePluginGenerateRequest,
    subtitle_generate_result: PluginResult<SubtitlePluginGenerateResponse>,
    download_add_request: PluginDownloadClientAddRequest,
    download_add_result: PluginResult<PluginDownloadClientAddResponse>,
    download_queue_result: PluginResult<Vec<PluginDownloadItem>>,
    download_history_result: PluginResult<Vec<PluginCompletedDownload>>,
    download_completed_result: PluginResult<Vec<PluginCompletedDownload>>,
    download_control_request: PluginDownloadClientControlRequest,
    download_control_result: PluginResult<()>,
    download_mark_imported_request: PluginDownloadClientMarkImportedRequest,
    download_mark_imported_result: PluginResult<()>,
    download_status_result: PluginResult<PluginDownloadClientStatus>,
    notification_request: PluginNotificationRequest,
    notification_result: PluginResult<PluginNotificationResponse>,
}

pub fn plugin_sdk_schema_json() -> String {
    serde_json::to_string_pretty(&schema_for!(PluginSdkSchemaDocument)).unwrap() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_descriptor_round_trips() {
        let descriptor = PluginDescriptor {
            id: "newznab".into(),
            name: "Newznab".into(),
            version: "1.0.0".into(),
            sdk_version: SDK_VERSION.into(),
            provider: ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "newznab".into(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Usenet,
                capabilities: IndexerCapabilities::default(),
                scoring_policies: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
        };

        let json = serde_json::to_string(&descriptor).unwrap();
        let parsed: PluginDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "newznab");
        assert_eq!(parsed.provider_type(), "newznab");
        assert_eq!(parsed.plugin_type(), "usenet_indexer");
    }

    #[test]
    fn unknown_download_state_is_rejected() {
        let json = r#"{"client_item_id":"1","title":"x","state":"mystery"}"#;
        assert!(serde_json::from_str::<PluginDownloadItem>(json).is_err());
    }

    #[test]
    fn committed_schema_matches_generated_types() {
        let schema_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/plugin-sdk-v1.schema.json");
        let expected = std::fs::read_to_string(schema_path).unwrap();
        assert_eq!(expected, plugin_sdk_schema_json());
    }
}
