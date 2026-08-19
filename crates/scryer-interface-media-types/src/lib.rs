use async_graphql::{
    Enum, ID, InputObject, InputValueError, InputValueResult, Json, MaybeUndefined, OneofObject,
    Scalar, ScalarType, SimpleObject, Value,
};
use chrono::{DateTime, NaiveDate, Utc};
use scryer_domain::{
    AppPermission, CollectionType, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource,
    DomainEventActorKind, DomainEventType, DownloadQueueState, EpisodeType, ExecutionMode,
    ImportDecision, ImportErrorCode, ImportMode, ImportSkipReason, ImportStatus,
    ImportTransferPhase, ImportType, LibraryPermission, MediaFacet, MediaRequestStatus,
    TitleHistoryEventType, TitleMatchType, TrackedDownloadState, TrackedDownloadStatus,
};

/// Signed 64-bit integer scalar for counts, sizes, sequences, and other values that exceed GraphQL `Int` range.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Long(pub i64);

impl Long {
    pub fn from_u64_saturating(value: u64) -> Self {
        Self(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<i64> for Long {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<Long> for i64 {
    fn from(value: Long) -> Self {
        value.0
    }
}

/// Signed 64-bit integer scalar for counts, sizes, sequences, and other values that exceed GraphQL `Int` range.
#[Scalar(name = "Long")]
impl ScalarType for Long {
    fn parse(value: Value) -> InputValueResult<Self> {
        match value {
            Value::Number(number) => number
                .as_i64()
                .map(Self)
                .ok_or_else(|| InputValueError::custom("Long must be a signed 64-bit integer")),
            other => Err(InputValueError::expected_type(other)),
        }
    }

    fn is_valid(value: &Value) -> bool {
        matches!(value, Value::Number(number) if number.as_i64().is_some())
    }

    fn to_value(&self) -> Value {
        Value::Number(self.0.into())
    }
}

/// Calendar date scalar serialized as an ISO-8601 `YYYY-MM-DD` string without a time zone.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Date(pub NaiveDate);

impl Date {
    pub fn parse_iso(value: &str) -> Result<Self, chrono::ParseError> {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").map(Self)
    }

    pub fn to_iso_string(self) -> String {
        self.0.format("%Y-%m-%d").to_string()
    }
}

impl From<NaiveDate> for Date {
    fn from(value: NaiveDate) -> Self {
        Self(value)
    }
}

impl From<Date> for NaiveDate {
    fn from(value: Date) -> Self {
        value.0
    }
}

/// Calendar date scalar serialized as an ISO-8601 `YYYY-MM-DD` string without a time zone.
#[Scalar(name = "Date")]
impl ScalarType for Date {
    fn parse(value: Value) -> InputValueResult<Self> {
        match value {
            Value::String(value) => {
                Self::parse_iso(&value).map_err(|error| InputValueError::custom(error.to_string()))
            }
            other => Err(InputValueError::expected_type(other)),
        }
    }

    fn is_valid(value: &Value) -> bool {
        matches!(value, Value::String(value) if Self::parse_iso(value).is_ok())
    }

    fn to_value(&self) -> Value {
        Value::String(self.to_iso_string())
    }
}

/// Media facet used to distinguish movie, series, and anime catalog records.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaFacetValue {
    /// Movie content.
    Movie,
    /// Series content.
    Series,
    /// Anime content.
    Anime,
}

/// Library permission granted within a specific library.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryPermissionValue {
    /// Allows viewing library content.
    View,
    /// Allows managing titles in the library.
    ManageTitles,
    /// Allows resolving imports for the library.
    ResolveImports,
    /// Allows changing library configuration.
    ManageLibrary,
    /// Allows submitting requests for the library.
    Request,
    /// Allows automatically approving requests for the library.
    AutoApproveRequests,
}

/// Application-wide permission independent of a library.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum AppPermissionValue {
    /// Allows creating, changing, and deleting users.
    ManageUsers,
    /// Allows changing user permission grants.
    ManagePermissions,
    /// Allows changing system settings.
    ManageSystemSettings,
    /// Allows changing catalog settings.
    ManageCatalogSettings,
}

/// Account origin used for login and authorization behavior.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UserAccountKindValue {
    /// Account created and managed locally.
    Local,
    /// Account created automatically from an external provider login.
    ExternalAutoProvisioned,
}

impl UserAccountKindValue {
    pub fn from_domain(kind: scryer_domain::UserAccountKind) -> Self {
        match kind {
            scryer_domain::UserAccountKind::Local => Self::Local,
            scryer_domain::UserAccountKind::ExternalAutoProvisioned => {
                Self::ExternalAutoProvisioned
            }
        }
    }
}

impl AppPermissionValue {
    pub fn into_domain(self) -> AppPermission {
        match self {
            Self::ManageUsers => AppPermission::ManageUsers,
            Self::ManagePermissions => AppPermission::ManagePermissions,
            Self::ManageSystemSettings => AppPermission::ManageSystemSettings,
            Self::ManageCatalogSettings => AppPermission::ManageCatalogSettings,
        }
    }

    pub fn from_domain(permission: AppPermission) -> Self {
        match permission {
            AppPermission::ManageUsers => Self::ManageUsers,
            AppPermission::ManagePermissions => Self::ManagePermissions,
            AppPermission::ManageSystemSettings => Self::ManageSystemSettings,
            AppPermission::ManageCatalogSettings => Self::ManageCatalogSettings,
        }
    }
}

impl LibraryPermissionValue {
    pub fn into_domain(self) -> LibraryPermission {
        match self {
            Self::View => LibraryPermission::View,
            Self::ManageTitles => LibraryPermission::ManageTitles,
            Self::ResolveImports => LibraryPermission::ResolveImports,
            Self::ManageLibrary => LibraryPermission::ManageLibrary,
            Self::Request => LibraryPermission::Request,
            Self::AutoApproveRequests => LibraryPermission::AutoApproveRequests,
        }
    }

    pub fn from_domain(permission: LibraryPermission) -> Self {
        match permission {
            LibraryPermission::View => Self::View,
            LibraryPermission::ManageTitles => Self::ManageTitles,
            LibraryPermission::ResolveImports => Self::ResolveImports,
            LibraryPermission::ManageLibrary => Self::ManageLibrary,
            LibraryPermission::Request => Self::Request,
            LibraryPermission::AutoApproveRequests => Self::AutoApproveRequests,
        }
    }
}

impl MediaFacetValue {
    pub fn as_scope_id(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Anime => "anime",
        }
    }

    pub fn into_domain(self) -> MediaFacet {
        match self {
            Self::Movie => MediaFacet::Movie,
            Self::Series => MediaFacet::Series,
            Self::Anime => MediaFacet::Anime,
        }
    }

    pub fn from_domain(value: MediaFacet) -> Self {
        match value {
            MediaFacet::Movie => Self::Movie,
            MediaFacet::Series => Self::Series,
            MediaFacet::Anime => Self::Anime,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            "anime" => Some(Self::Anime),
            _ => None,
        }
    }
}

/// State of a pending import record.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PendingImportStatusValue {
    /// Import is awaiting a decision or action.
    Pending,
    /// Import was intentionally ignored.
    Ignored,
}

/// Lifecycle state of a media request.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaRequestStatusValue {
    /// Request awaits a decision.
    Pending,
    /// Request was approved.
    Approved,
    /// Request was rejected.
    Rejected,
    /// Request was canceled.
    Canceled,
}

impl MediaRequestStatusValue {
    pub fn from_domain(value: MediaRequestStatus) -> Self {
        match value {
            MediaRequestStatus::Pending => Self::Pending,
            MediaRequestStatus::Approved => Self::Approved,
            MediaRequestStatus::Rejected => Self::Rejected,
            MediaRequestStatus::Canceled => Self::Canceled,
        }
    }

    pub fn into_domain(self) -> MediaRequestStatus {
        match self {
            Self::Pending => MediaRequestStatus::Pending,
            Self::Approved => MediaRequestStatus::Approved,
            Self::Rejected => MediaRequestStatus::Rejected,
            Self::Canceled => MediaRequestStatus::Canceled,
        }
    }
}

/// Content scope used by settings and catalog operations.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ContentScopeValue {
    /// Movie scope.
    Movie,
    /// Series scope.
    Series,
    /// Anime scope.
    Anime,
}

impl ContentScopeValue {
    pub fn as_scope_id(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Anime => "anime",
        }
    }

    pub fn into_media_facet(self) -> MediaFacet {
        match self {
            Self::Movie => MediaFacet::Movie,
            Self::Series => MediaFacet::Series,
            Self::Anime => MediaFacet::Anime,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            "anime" => Some(Self::Anime),
            _ => None,
        }
    }
}

/// Scoring persona that selects a quality-scoring strategy.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ScoringPersonaValue {
    /// Balanced scoring across quality and compatibility.
    Balanced,
    /// Favors audio quality.
    Audiophile,
    /// Favors efficient storage or delivery.
    Efficient,
    /// Favors broad playback compatibility.
    Compatible,
}

/// Monitoring mode applied to episodic content.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MonitorTypeValue {
    /// Monitor all selected episodes.
    Monitored,
    /// Monitor no episodes.
    Unmonitored,
    /// Monitor future episodes only.
    FutureEpisodes,
    /// Monitor missing and future episodes.
    MissingAndFutureEpisodes,
    /// Monitor every episode, including already released episodes.
    AllEpisodes,
    /// Explicitly select no episodes; exposed as `NONE` in GraphQL.
    #[graphql(name = "NONE")]
    NoneSelected,
}

impl MonitorTypeValue {
    pub fn as_tag_value(self) -> &'static str {
        match self {
            Self::Monitored => "monitored",
            Self::Unmonitored => "unmonitored",
            Self::FutureEpisodes => "futureepisodes",
            Self::MissingAndFutureEpisodes => "missingandfutureepisodes",
            Self::AllEpisodes => "allepisodes",
            Self::NoneSelected => "none",
        }
    }

    pub fn from_tag_value(value: &str) -> Option<Self> {
        match value.trim() {
            "monitored" => Some(Self::Monitored),
            "unmonitored" => Some(Self::Unmonitored),
            "futureepisodes" | "futureEpisodes" => Some(Self::FutureEpisodes),
            "missingandfutureepisodes" | "missingAndFutureEpisodes" => {
                Some(Self::MissingAndFutureEpisodes)
            }
            "allepisodes" | "allEpisodes" => Some(Self::AllEpisodes),
            "none" => Some(Self::NoneSelected),
            _ => None,
        }
    }
}

/// Source form used to acquire a release.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadSourceKindValue {
    /// NZB supplied as a file.
    NzbFile,
    /// NZB supplied by URL.
    NzbUrl,
    /// Torrent supplied as a file.
    TorrentFile,
    /// Torrent supplied as a magnet URI.
    MagnetUri,
}

/// Reason a queued download was requested.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum QueueDownloadPurposeValue {
    /// Normal download request.
    Standard,
    /// Download requested for an additional file.
    AdditionalFile,
}

/// Path syntax used by the runtime.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RuntimePathStyleValue {
    /// Unix path syntax.
    Unix,
    /// Windows path syntax.
    Windows,
}

/// Preferred download protocol for a delay profile.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DelayProfilePreferredProtocolValue {
    /// Prefer Usenet.
    Usenet,
    /// Prefer torrents.
    Torrent,
}

/// Processing state of a queued download.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueStateValue {
    /// Queued but not started.
    Queued,
    /// Download is in progress.
    Downloading,
    /// Download is being verified.
    Verifying,
    /// Download is being repaired.
    Repairing,
    /// Download is being extracted.
    Extracting,
    /// Download is paused.
    Paused,
    /// Download completed.
    Completed,
    /// Download awaits import.
    ImportPending,
    /// Download failed.
    Failed,
}

impl DownloadQueueStateValue {
    pub fn from_domain(value: DownloadQueueState) -> Self {
        match value {
            DownloadQueueState::Queued => Self::Queued,
            DownloadQueueState::Downloading => Self::Downloading,
            DownloadQueueState::Verifying => Self::Verifying,
            DownloadQueueState::Repairing => Self::Repairing,
            DownloadQueueState::Extracting => Self::Extracting,
            DownloadQueueState::Paused => Self::Paused,
            DownloadQueueState::Completed => Self::Completed,
            DownloadQueueState::ImportPending => Self::ImportPending,
            DownloadQueueState::Failed => Self::Failed,
        }
    }
}

/// Display state combining download and post-processing lifecycle information.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadDisplayStateValue {
    /// Waiting in the download queue.
    Queued,
    /// Downloading data.
    Downloading,
    /// Paused by an operator or policy.
    Paused,
    /// Running post-processing.
    PostProcessing,
    /// Download and processing completed.
    Completed,
    /// Download or processing failed.
    Failed,
    /// Import is running.
    Importing,
    /// Import is waiting to run.
    ImportPending,
    /// Import is blocked by policy or prerequisites.
    ImportBlocked,
    /// Import failed.
    ImportFailed,
    /// Item was ignored.
    Ignored,
    /// Removal is running.
    Removing,
    /// Removal failed.
    RemoveFailed,
}

/// Filter for active download activity.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadActivityFilterValue {
    /// Include every activity state.
    All,
    /// Include downloading items.
    Downloading,
    /// Include queued items.
    Queued,
    /// Include paused items.
    Paused,
    /// Include post-processing items.
    PostProcessing,
}

/// Filter for import activity.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadImportFilterValue {
    /// Include every import state.
    All,
    /// Include imports currently running.
    Importing,
    /// Include imports awaiting action.
    Pending,
    /// Include imports blocked by policy.
    Blocked,
    /// Include failed imports.
    Failed,
}

/// Filter for download history outcomes.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadHistoryFilterValue {
    /// Include all history entries.
    All,
    /// Include successful entries.
    Success,
    /// Include failed entries.
    Failed,
}

/// Sort key for download history.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadHistorySortKeyValue {
    /// Sort by title.
    Title,
    /// Sort by download client.
    Client,
    /// Sort by status.
    Status,
    /// Sort by progress.
    Progress,
    /// Sort by size.
    Size,
}

/// Sort key for the active download queue.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueSortKeyValue {
    /// Sort by title.
    Title,
    /// Sort by download client.
    Client,
    /// Sort by status.
    Status,
    /// Sort by progress.
    Progress,
    /// Sort by size.
    Size,
}

/// Direction for sortable list results.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum SortDirectionValue {
    /// Lowest or earliest values first.
    Asc,
    /// Highest or latest values first.
    Desc,
}

/// Domain event name emitted by the application.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DomainEventTypeValue {
    /// A media request was submitted.
    MediaRequestSubmitted,
    /// A media request changed.
    MediaRequestUpdated,
    /// A media request was approved.
    MediaRequestApproved,
    /// A media request was rejected.
    MediaRequestRejected,
    /// A media request was canceled.
    MediaRequestCanceled,
    /// A title was added.
    TitleAdded,
    /// A title was updated.
    TitleUpdated,
    /// A title was rematched.
    TitleRematched,
    /// A title was deleted.
    TitleDeleted,
    /// Configuration changed.
    ConfigurationChanged,
    /// A discovery search completed.
    DiscoverySearchCompleted,
    /// Metadata hydration changed.
    MetadataHydrationUpdated,
    /// A release was grabbed.
    ReleaseGrabbed,
    /// A download failed.
    DownloadFailed,
    /// A download was ignored.
    DownloadIgnored,
    /// A release was blocklisted.
    ReleaseBlocklisted,
    /// An import completed.
    ImportCompleted,
    /// An import was rejected.
    ImportRejected,
    /// A media file was imported.
    MediaFileImported,
    /// A media file was analyzed.
    MediaFileAnalyzed,
    /// A media file was renamed.
    MediaFileRenamed,
    /// A media file was deleted.
    MediaFileDeleted,
    /// A media file was upgraded.
    MediaFileUpgraded,
    /// An acquisition search completed.
    AcquisitionSearchCompleted,
    /// An acquisition candidate was rejected.
    AcquisitionCandidateRejected,
    /// An import was requested.
    ImportRequested,
    /// Import recovery completed.
    ImportRecoveryCompleted,
    /// A download queue command was issued.
    DownloadQueueItemCommandIssued,
    /// Post-processing completed.
    PostProcessingCompleted,
    /// A subtitle was downloaded.
    SubtitleDownloaded,
    /// A subtitle search failed.
    SubtitleSearchFailed,
    /// A library scan started.
    LibraryScanStarted,
    /// A title was discovered during a library scan.
    LibraryScanTitleDiscovered,
    /// A library scan delta was recorded.
    LibraryScanDeltaRecorded,
    /// Library scan progress changed.
    LibraryScanProgressed,
    /// A library scan completed.
    LibraryScanCompleted,
    /// A library scan was canceled.
    LibraryScanCanceled,
    /// A library scan failed.
    LibraryScanFailed,
    /// A job run started.
    JobRunStarted,
    /// A job run completed.
    JobRunCompleted,
    /// A job run failed.
    JobRunFailed,
    /// A job's next-run time changed.
    JobNextRunUpdated,
    /// A download queue item was added or updated.
    DownloadQueueItemUpserted,
    /// A download queue item was removed.
    DownloadQueueItemRemoved,
}

impl DomainEventTypeValue {
    pub fn from_domain(value: DomainEventType) -> Self {
        match value {
            DomainEventType::MediaRequestSubmitted => Self::MediaRequestSubmitted,
            DomainEventType::MediaRequestUpdated => Self::MediaRequestUpdated,
            DomainEventType::MediaRequestApproved => Self::MediaRequestApproved,
            DomainEventType::MediaRequestRejected => Self::MediaRequestRejected,
            DomainEventType::MediaRequestCanceled => Self::MediaRequestCanceled,
            DomainEventType::TitleAdded => Self::TitleAdded,
            DomainEventType::TitleUpdated => Self::TitleUpdated,
            DomainEventType::TitleRematched => Self::TitleRematched,
            DomainEventType::TitleDeleted => Self::TitleDeleted,
            DomainEventType::ConfigurationChanged => Self::ConfigurationChanged,
            DomainEventType::DiscoverySearchCompleted => Self::DiscoverySearchCompleted,
            DomainEventType::MetadataHydrationUpdated => Self::MetadataHydrationUpdated,
            DomainEventType::ReleaseGrabbed => Self::ReleaseGrabbed,
            DomainEventType::DownloadFailed => Self::DownloadFailed,
            DomainEventType::DownloadIgnored => Self::DownloadIgnored,
            DomainEventType::ReleaseBlocklisted => Self::ReleaseBlocklisted,
            DomainEventType::ImportCompleted => Self::ImportCompleted,
            DomainEventType::ImportRejected => Self::ImportRejected,
            DomainEventType::MediaFileImported => Self::MediaFileImported,
            DomainEventType::MediaFileAnalyzed => Self::MediaFileAnalyzed,
            DomainEventType::MediaFileRenamed => Self::MediaFileRenamed,
            DomainEventType::MediaFileDeleted => Self::MediaFileDeleted,
            DomainEventType::MediaFileUpgraded => Self::MediaFileUpgraded,
            DomainEventType::AcquisitionSearchCompleted => Self::AcquisitionSearchCompleted,
            DomainEventType::AcquisitionCandidateRejected => Self::AcquisitionCandidateRejected,
            DomainEventType::ImportRequested => Self::ImportRequested,
            DomainEventType::ImportRecoveryCompleted => Self::ImportRecoveryCompleted,
            DomainEventType::DownloadQueueItemCommandIssued => Self::DownloadQueueItemCommandIssued,
            DomainEventType::PostProcessingCompleted => Self::PostProcessingCompleted,
            DomainEventType::SubtitleDownloaded => Self::SubtitleDownloaded,
            DomainEventType::SubtitleSearchFailed => Self::SubtitleSearchFailed,
            DomainEventType::LibraryScanStarted => Self::LibraryScanStarted,
            DomainEventType::LibraryScanTitleDiscovered => Self::LibraryScanTitleDiscovered,
            DomainEventType::LibraryScanDeltaRecorded => Self::LibraryScanDeltaRecorded,
            DomainEventType::LibraryScanProgressed => Self::LibraryScanProgressed,
            DomainEventType::LibraryScanCompleted => Self::LibraryScanCompleted,
            DomainEventType::LibraryScanCanceled => Self::LibraryScanCanceled,
            DomainEventType::LibraryScanFailed => Self::LibraryScanFailed,
            DomainEventType::JobRunStarted => Self::JobRunStarted,
            DomainEventType::JobRunCompleted => Self::JobRunCompleted,
            DomainEventType::JobRunFailed => Self::JobRunFailed,
            DomainEventType::JobNextRunUpdated => Self::JobNextRunUpdated,
            DomainEventType::DownloadQueueItemUpserted => Self::DownloadQueueItemUpserted,
            DomainEventType::DownloadQueueItemRemoved => Self::DownloadQueueItemRemoved,
        }
    }

    pub fn into_domain(self) -> DomainEventType {
        match self {
            Self::MediaRequestSubmitted => DomainEventType::MediaRequestSubmitted,
            Self::MediaRequestUpdated => DomainEventType::MediaRequestUpdated,
            Self::MediaRequestApproved => DomainEventType::MediaRequestApproved,
            Self::MediaRequestRejected => DomainEventType::MediaRequestRejected,
            Self::MediaRequestCanceled => DomainEventType::MediaRequestCanceled,
            Self::TitleAdded => DomainEventType::TitleAdded,
            Self::TitleUpdated => DomainEventType::TitleUpdated,
            Self::TitleRematched => DomainEventType::TitleRematched,
            Self::TitleDeleted => DomainEventType::TitleDeleted,
            Self::ConfigurationChanged => DomainEventType::ConfigurationChanged,
            Self::DiscoverySearchCompleted => DomainEventType::DiscoverySearchCompleted,
            Self::MetadataHydrationUpdated => DomainEventType::MetadataHydrationUpdated,
            Self::ReleaseGrabbed => DomainEventType::ReleaseGrabbed,
            Self::DownloadFailed => DomainEventType::DownloadFailed,
            Self::DownloadIgnored => DomainEventType::DownloadIgnored,
            Self::ReleaseBlocklisted => DomainEventType::ReleaseBlocklisted,
            Self::ImportCompleted => DomainEventType::ImportCompleted,
            Self::ImportRejected => DomainEventType::ImportRejected,
            Self::MediaFileImported => DomainEventType::MediaFileImported,
            Self::MediaFileAnalyzed => DomainEventType::MediaFileAnalyzed,
            Self::MediaFileRenamed => DomainEventType::MediaFileRenamed,
            Self::MediaFileDeleted => DomainEventType::MediaFileDeleted,
            Self::MediaFileUpgraded => DomainEventType::MediaFileUpgraded,
            Self::AcquisitionSearchCompleted => DomainEventType::AcquisitionSearchCompleted,
            Self::AcquisitionCandidateRejected => DomainEventType::AcquisitionCandidateRejected,
            Self::ImportRequested => DomainEventType::ImportRequested,
            Self::ImportRecoveryCompleted => DomainEventType::ImportRecoveryCompleted,
            Self::DownloadQueueItemCommandIssued => DomainEventType::DownloadQueueItemCommandIssued,
            Self::PostProcessingCompleted => DomainEventType::PostProcessingCompleted,
            Self::SubtitleDownloaded => DomainEventType::SubtitleDownloaded,
            Self::SubtitleSearchFailed => DomainEventType::SubtitleSearchFailed,
            Self::LibraryScanStarted => DomainEventType::LibraryScanStarted,
            Self::LibraryScanTitleDiscovered => DomainEventType::LibraryScanTitleDiscovered,
            Self::LibraryScanDeltaRecorded => DomainEventType::LibraryScanDeltaRecorded,
            Self::LibraryScanProgressed => DomainEventType::LibraryScanProgressed,
            Self::LibraryScanCompleted => DomainEventType::LibraryScanCompleted,
            Self::LibraryScanCanceled => DomainEventType::LibraryScanCanceled,
            Self::LibraryScanFailed => DomainEventType::LibraryScanFailed,
            Self::JobRunStarted => DomainEventType::JobRunStarted,
            Self::JobRunCompleted => DomainEventType::JobRunCompleted,
            Self::JobRunFailed => DomainEventType::JobRunFailed,
            Self::JobNextRunUpdated => DomainEventType::JobNextRunUpdated,
            Self::DownloadQueueItemUpserted => DomainEventType::DownloadQueueItemUpserted,
            Self::DownloadQueueItemRemoved => DomainEventType::DownloadQueueItemRemoved,
        }
    }
}

/// Lifecycle state of a tracked download.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TrackedDownloadStateValue {
    /// Download is in progress.
    Downloading,
    /// Download awaits import.
    ImportPending,
    /// Import is in progress.
    Importing,
    /// Import completed successfully.
    Imported,
    /// Import is blocked.
    ImportBlocked,
    /// Failure awaits retry or handling.
    FailedPending,
    /// Download or import failed.
    Failed,
    /// Download was ignored.
    Ignored,
}

impl TrackedDownloadStateValue {
    pub fn from_domain(value: TrackedDownloadState) -> Self {
        match value {
            TrackedDownloadState::Downloading => Self::Downloading,
            TrackedDownloadState::ImportPending => Self::ImportPending,
            TrackedDownloadState::Importing => Self::Importing,
            TrackedDownloadState::Imported => Self::Imported,
            TrackedDownloadState::ImportBlocked => Self::ImportBlocked,
            TrackedDownloadState::FailedPending => Self::FailedPending,
            TrackedDownloadState::Failed => Self::Failed,
            TrackedDownloadState::Ignored => Self::Ignored,
        }
    }
}

/// Severity of a tracked-download health result.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TrackedDownloadStatusValue {
    /// No issue was detected.
    Ok,
    /// The item needs attention but is not failed.
    Warning,
    /// The item has an error.
    Error,
}

impl TrackedDownloadStatusValue {
    pub fn from_domain(value: TrackedDownloadStatus) -> Self {
        match value {
            TrackedDownloadStatus::Ok => Self::Ok,
            TrackedDownloadStatus::Warning => Self::Warning,
            TrackedDownloadStatus::Error => Self::Error,
        }
    }
}

/// Source used to match a release to a title.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TitleMatchTypeValue {
    /// Matched from the original submission.
    Submission,
    /// Matched from a client-supplied parameter.
    ClientParameter,
    /// Matched by parsing the title.
    TitleParse,
    /// Matched by an explicit ID.
    IdOnly,
    /// No title match was found.
    Unmatched,
}

impl TitleMatchTypeValue {
    pub fn from_domain(value: TitleMatchType) -> Self {
        match value {
            TitleMatchType::Submission => Self::Submission,
            TitleMatchType::ClientParameter => Self::ClientParameter,
            TitleMatchType::TitleParse => Self::TitleParse,
            TitleMatchType::IdOnly => Self::IdOnly,
            TitleMatchType::Unmatched => Self::Unmatched,
        }
    }
}

/// Lifecycle state of an import operation.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportStatusValue {
    /// Import is queued.
    Pending,
    /// Import work is running.
    Running,
    /// Import is processing results.
    Processing,
    /// Import completed successfully.
    Completed,
    /// Import failed.
    Failed,
    /// Import was skipped.
    Skipped,
}

impl ImportStatusValue {
    pub fn from_domain(value: ImportStatus) -> Self {
        match value {
            ImportStatus::Pending => Self::Pending,
            ImportStatus::Running => Self::Running,
            ImportStatus::Processing => Self::Processing,
            ImportStatus::Completed => Self::Completed,
            ImportStatus::Failed => Self::Failed,
            ImportStatus::Skipped => Self::Skipped,
        }
    }
}

/// Kind of import or rename operation.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportTypeValue {
    /// Movie download import.
    MovieDownload,
    /// Series download import.
    SeriesDownload,
    /// User-requested manual import.
    ManualImport,
    /// Rename plan preview.
    RenamePreview,
    /// Rename operation for one title.
    RenameApplyTitle,
    /// Rename operation for one facet.
    RenameApplyFacet,
    /// Rename operation result handling.
    RenameApplyResult,
    /// Rename operation failed during I/O.
    RenameIoFailed,
    /// Rename operation moved a file.
    RenameMove,
    /// Rename plan was stale.
    RenameStalePlan,
}

impl ImportTypeValue {
    pub fn from_domain(value: ImportType) -> Self {
        match value {
            ImportType::MovieDownload => Self::MovieDownload,
            ImportType::SeriesDownload => Self::SeriesDownload,
            ImportType::ManualImport => Self::ManualImport,
            ImportType::RenamePreview => Self::RenamePreview,
            ImportType::RenameApplyTitle => Self::RenameApplyTitle,
            ImportType::RenameApplyFacet => Self::RenameApplyFacet,
            ImportType::RenameApplyResult => Self::RenameApplyResult,
            ImportType::RenameIoFailed => Self::RenameIoFailed,
            ImportType::RenameMove => Self::RenameMove,
            ImportType::RenameStalePlan => Self::RenameStalePlan,
        }
    }
}

/// Machine-readable reason an import failed.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportErrorCodeValue {
    /// Source file was not found.
    FileNotFound,
    /// Target episode was not found.
    EpisodeNotFound,
    /// Episode lookup failed.
    EpisodeLookupFailed,
    /// The source job failed.
    SourceJobFailed,
    /// Import policy did not match.
    PolicyMismatch,
    /// File I/O failed.
    IoFailed,
    /// Permission denied.
    PermissionDenied,
    /// Storage is full.
    DiskFull,
    /// Failure has no more specific code.
    Unknown,
}

impl ImportErrorCodeValue {
    pub fn from_domain(value: ImportErrorCode) -> Self {
        match value {
            ImportErrorCode::FileNotFound => Self::FileNotFound,
            ImportErrorCode::EpisodeNotFound => Self::EpisodeNotFound,
            ImportErrorCode::EpisodeLookupFailed => Self::EpisodeLookupFailed,
            ImportErrorCode::SourceJobFailed => Self::SourceJobFailed,
            ImportErrorCode::PolicyMismatch => Self::PolicyMismatch,
            ImportErrorCode::IoFailed => Self::IoFailed,
            ImportErrorCode::PermissionDenied => Self::PermissionDenied,
            ImportErrorCode::DiskFull => Self::DiskFull,
            ImportErrorCode::Unknown => Self::Unknown,
        }
    }
}

/// State of a queued download deletion request.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueDeleteStatusValue {
    /// Deletion is queued.
    Queued,
    /// Deletion is running.
    Running,
    /// Deletion completed.
    Completed,
    /// Deletion failed.
    Failed,
}

impl DownloadQueueDeleteStatusValue {
    pub fn from_domain(value: scryer_domain::DownloadQueueDeleteStatus) -> Self {
        match value {
            scryer_domain::DownloadQueueDeleteStatus::Queued => Self::Queued,
            scryer_domain::DownloadQueueDeleteStatus::Running => Self::Running,
            scryer_domain::DownloadQueueDeleteStatus::Completed => Self::Completed,
            scryer_domain::DownloadQueueDeleteStatus::Failed => Self::Failed,
        }
    }
}

/// Outcome of an import decision.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportDecisionValue {
    /// File was imported.
    Imported,
    /// File was rejected.
    Rejected,
    /// File was skipped.
    Skipped,
    /// File conflicted with another candidate.
    Conflict,
    /// File could not be matched.
    Unmatched,
    /// Import failed.
    Failed,
}

impl ImportDecisionValue {
    pub fn from_domain(value: ImportDecision) -> Self {
        match value {
            ImportDecision::Imported => Self::Imported,
            ImportDecision::Rejected => Self::Rejected,
            ImportDecision::Skipped => Self::Skipped,
            ImportDecision::Conflict => Self::Conflict,
            ImportDecision::Unmatched => Self::Unmatched,
            ImportDecision::Failed => Self::Failed,
        }
    }
}

/// Reason an import candidate was skipped.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportSkipReasonValue {
    /// File was already imported.
    AlreadyImported,
    /// File duplicated another candidate.
    DuplicateFile,
    /// Post-download rules blocked the file.
    PostDownloadRuleBlocked,
    /// Import policy did not match.
    PolicyMismatch,
    /// Title identity could not be resolved.
    UnresolvedIdentity,
    /// Episode metadata could not be parsed.
    UnparseableEpisode,
    /// No video files were found.
    NoVideoFiles,
    /// Storage is full.
    DiskFull,
    /// Permission was denied.
    PermissionDenied,
    /// A password is required before retrying.
    PasswordRequired,
}

impl ImportSkipReasonValue {
    pub fn from_domain(value: ImportSkipReason) -> Self {
        match value {
            ImportSkipReason::AlreadyImported => Self::AlreadyImported,
            ImportSkipReason::DuplicateFile => Self::DuplicateFile,
            ImportSkipReason::PostDownloadRuleBlocked => Self::PostDownloadRuleBlocked,
            ImportSkipReason::PolicyMismatch => Self::PolicyMismatch,
            ImportSkipReason::UnresolvedIdentity => Self::UnresolvedIdentity,
            ImportSkipReason::UnparseableEpisode => Self::UnparseableEpisode,
            ImportSkipReason::NoVideoFiles => Self::NoVideoFiles,
            ImportSkipReason::DiskFull => Self::DiskFull,
            ImportSkipReason::PermissionDenied => Self::PermissionDenied,
            ImportSkipReason::PasswordRequired => Self::PasswordRequired,
        }
    }
}

/// Policy for filler episodes or scenes.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum FillerPolicyValue {
    /// Download filler content.
    DownloadAll,
    /// Skip filler content.
    SkipFiller,
}

impl FillerPolicyValue {
    // Stored as a settings string / structured title tag by the application.
    pub fn as_app_str(self) -> &'static str {
        match self {
            Self::DownloadAll => "download_all",
            Self::SkipFiller => "skip_filler",
        }
    }

    pub fn from_app_str(value: &str) -> Option<Self> {
        match value {
            "download_all" => Some(Self::DownloadAll),
            "skip_filler" => Some(Self::SkipFiller),
            _ => None,
        }
    }
}

/// Policy for recap episodes or scenes.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RecapPolicyValue {
    /// Download recap content.
    DownloadAll,
    /// Skip recap content.
    SkipRecap,
}

impl RecapPolicyValue {
    pub fn as_app_str(self) -> &'static str {
        match self {
            Self::DownloadAll => "download_all",
            Self::SkipRecap => "skip_recap",
        }
    }

    pub fn from_app_str(value: &str) -> Option<Self> {
        match value {
            "download_all" => Some(Self::DownloadAll),
            "skip_recap" => Some(Self::SkipRecap),
            _ => None,
        }
    }
}

/// Phase of transferring an imported file.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportTransferPhaseValue {
    /// File bytes are being copied.
    Copying,
    /// Transfer is finalizing metadata and filesystem state.
    Finalizing,
}

impl From<ImportTransferPhase> for ImportTransferPhaseValue {
    fn from(value: ImportTransferPhase) -> Self {
        match value {
            ImportTransferPhase::Copying => Self::Copying,
            ImportTransferPhase::Finalizing => Self::Finalizing,
        }
    }
}

/// Health state reported by the plugin catalog runtime.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum CatalogRefreshStateValue {
    /// Catalog is ready for use.
    Ready,
    /// Catalog is usable with degraded availability.
    Degraded,
}

impl CatalogRefreshStateValue {
    // The plugin-catalog runtime reports this as a string; parse at the API
    // boundary and fail safe to Ready.
    pub fn from_app_str(value: &str) -> Self {
        match value {
            "degraded" => Self::Degraded,
            _ => Self::Ready,
        }
    }
}

/// Stream identifier scope for event subscriptions.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum StreamKindValue {
    /// Events from the global stream.
    Global,
    /// Events for one title.
    Title,
    /// Events for one library scan.
    LibraryScan,
    /// Events for one job run.
    JobRun,
    /// Events for one download queue item.
    DownloadQueueItem,
}

/// File operation used during import.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportModeValue {
    /// Hardlink when possible, otherwise copy.
    HardlinkOrCopy,
    /// Move the source file.
    Move,
}

impl From<ImportMode> for ImportModeValue {
    fn from(value: ImportMode) -> Self {
        match value {
            ImportMode::HardlinkOrCopy => Self::HardlinkOrCopy,
            ImportMode::Move => Self::Move,
        }
    }
}

impl From<ImportModeValue> for ImportMode {
    fn from(value: ImportModeValue) -> Self {
        match value {
            ImportModeValue::HardlinkOrCopy => Self::HardlinkOrCopy,
            ImportModeValue::Move => Self::Move,
        }
    }
}

/// Action when a rename destination already exists.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RenameCollisionPolicyValue {
    /// Leave the existing destination and skip the rename.
    Skip,
    /// Treat an existing destination as an error.
    Error,
    /// Replace only when the incoming file is better.
    ReplaceIfBetter,
}

impl RenameCollisionPolicyValue {
    // The application/settings layer stores these as canonical strings; the
    // enum exists at the API boundary only.
    pub fn as_app_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Error => "error",
            Self::ReplaceIfBetter => "replace_if_better",
        }
    }

    pub fn from_app_str(value: &str) -> Option<Self> {
        // Tolerant like the application-layer parser (trim + case-insensitive):
        // the stored value is a raw settings string.
        match value.trim().to_ascii_lowercase().as_str() {
            "skip" => Some(Self::Skip),
            "error" => Some(Self::Error),
            "replace_if_better" => Some(Self::ReplaceIfBetter),
            _ => None,
        }
    }
}

/// Fallback when required metadata is missing during rename.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RenameMissingMetadataPolicyValue {
    /// Skip the rename.
    Skip,
    /// Use the title as fallback metadata.
    FallbackTitle,
}

impl RenameMissingMetadataPolicyValue {
    pub fn as_app_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::FallbackTitle => "fallback_title",
        }
    }

    pub fn from_app_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "skip" => Some(Self::Skip),
            "fallback_title" => Some(Self::FallbackTitle),
            _ => None,
        }
    }
}

/// Collection grouping type.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum CollectionTypeValue {
    /// A season grouping.
    Season,
    /// A movie grouping.
    Movie,
    /// An arc grouping.
    Arc,
    /// A specials grouping.
    Specials,
}

impl From<CollectionType> for CollectionTypeValue {
    fn from(value: CollectionType) -> Self {
        match value {
            CollectionType::Season => Self::Season,
            CollectionType::Movie => Self::Movie,
            CollectionType::Arc => Self::Arc,
            CollectionType::Specials => Self::Specials,
        }
    }
}

/// Episode classification.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EpisodeTypeValue {
    /// Standard episode.
    Standard,
    /// Special episode.
    Special,
    /// Official episode classification.
    Official,
    /// Original video animation episode.
    Ova,
    /// Original net animation episode.
    Ona,
    /// Alternate episode version.
    Alternate,
}

impl From<EpisodeType> for EpisodeTypeValue {
    fn from(value: EpisodeType) -> Self {
        match value {
            EpisodeType::Standard => Self::Standard,
            EpisodeType::Special => Self::Special,
            EpisodeType::Official => Self::Official,
            EpisodeType::Ova => Self::Ova,
            EpisodeType::Ona => Self::Ona,
            EpisodeType::Alternate => Self::Alternate,
        }
    }
}

/// Actor origin attached to an event.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActorKindValue {
    /// A registered user caused the event.
    User,
    /// An unauthenticated actor caused the event.
    Anonymous,
    /// The system caused the event.
    System,
}

impl From<DomainEventActorKind> for ActorKindValue {
    fn from(value: DomainEventActorKind) -> Self {
        match value {
            DomainEventActorKind::User => Self::User,
            DomainEventActorKind::Anonymous => Self::Anonymous,
            DomainEventActorKind::System => Self::System,
        }
    }
}

/// Execution strategy for a job or script.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionModeValue {
    /// Complete work before returning.
    Blocking,
    /// Start work and return without waiting for completion.
    FireAndForget,
}

impl From<ExecutionMode> for ExecutionModeValue {
    fn from(value: ExecutionMode) -> Self {
        match value {
            ExecutionMode::Blocking => Self::Blocking,
            ExecutionMode::FireAndForget => Self::FireAndForget,
        }
    }
}

impl From<ExecutionModeValue> for ExecutionMode {
    fn from(value: ExecutionModeValue) -> Self {
        match value {
            ExecutionModeValue::Blocking => Self::Blocking,
            ExecutionModeValue::FireAndForget => Self::FireAndForget,
        }
    }
}

/// Kind of activity event shown to clients.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivityKindValue {
    /// A setting was saved.
    SettingSaved,
    /// A movie was fetched.
    MovieFetched,
    /// A title was added.
    TitleAdded,
    /// A title was updated.
    TitleUpdated,
    /// Metadata hydration started.
    MetadataHydrationStarted,
    /// Metadata hydration completed.
    MetadataHydrationCompleted,
    /// Metadata hydration failed.
    MetadataHydrationFailed,
    /// A movie was downloaded.
    MovieDownloaded,
    /// A series episode was imported.
    SeriesEpisodeImported,
    /// An acquisition search completed.
    AcquisitionSearchCompleted,
    /// An acquisition candidate was accepted.
    AcquisitionCandidateAccepted,
    /// An acquisition candidate was rejected.
    AcquisitionCandidateRejected,
    /// An acquisition download failed.
    AcquisitionDownloadFailed,
    /// Post-processing completed.
    PostProcessingCompleted,
    /// A file was analyzed.
    FileAnalyzed,
    /// A file was upgraded.
    FileUpgraded,
    /// An import was rejected.
    ImportRejected,
    /// A subtitle was downloaded.
    SubtitleDownloaded,
    /// A subtitle search failed.
    SubtitleSearchFailed,
    /// A system notice was emitted.
    SystemNotice,
}

/// Severity of an activity event.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivitySeverityValue {
    /// Informational event.
    Info,
    /// Successful operation.
    Success,
    /// Operation needs attention.
    Warning,
    /// Operation failed.
    Error,
}

/// Delivery channel associated with an activity event.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivityChannelValue {
    /// Web application activity channel.
    WebUi,
    /// Toast notification channel.
    Toast,
}

/// Lifecycle state of a wanted acquisition target.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedStatusValue {
    /// Target is wanted and not yet grabbed.
    Wanted,
    /// A release was grabbed for the target.
    Grabbed,
    /// Target processing is paused.
    Paused,
    /// Target has completed acquisition.
    Completed,
}

impl WantedStatusValue {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wanted => "wanted",
            Self::Grabbed => "grabbed",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }
}

/// Media shape represented by a wanted target.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedMediaTypeValue {
    /// A movie target.
    Movie,
    /// An episode target.
    Episode,
    /// A movie belonging to a series.
    SeriesMovie,
}

impl WantedMediaTypeValue {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Episode => "episode",
            Self::SeriesMovie => "series_movie",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "episode" => Some(Self::Episode),
            "series_movie" => Some(Self::SeriesMovie),
            _ => None,
        }
    }
}

/// Search and RSS convergence state of an acquisition scope.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ConvergenceStateValue {
    /// No indexer has been searched under the current fingerprint and the cursor has not begun.
    Queued,
    /// Some, but not all, routed indexers are covered and the sweep is in progress.
    Searching,
    /// Every routed indexer is covered under the current fingerprint and RSS watches the scope.
    Converged,
    /// The scope is not converged and every uncovered indexer is currently unavailable.
    Deferred,
}

/// Recency lane used to prioritize acquisition convergence.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RecencyLaneValue {
    /// Prioritized for prompt convergence.
    Hot,
    /// Drained under backpressure.
    Cold,
}

/// Derived acquisition-target set represented by a wanted view.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Default)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedKindValue {
    #[default]
    /// Scope has no primary file; this is the default target set.
    Missing,
    /// Scope has a file below the effective profile cutoff.
    CutoffUpgrade,
}

impl WantedKindValue {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::CutoffUpgrade => "cutoff_upgrade",
        }
    }
}

/// Lifecycle state of a pending release candidate.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PendingReleaseStatusValue {
    /// Candidate is waiting for processing.
    Waiting,
    /// Candidate is held in standby.
    Standby,
    /// Candidate is being processed.
    Processing,
    /// Candidate was grabbed.
    Grabbed,
    /// Candidate was superseded.
    Superseded,
    /// Candidate expired before processing.
    Expired,
    /// Candidate was dismissed.
    Dismissed,
    /// Candidate needs manual review.
    NeedsReview,
}

/// Local username and password credentials with an optional TOTP code.
#[derive(InputObject)]
pub struct LoginInput {
    /// Local username to authenticate.
    pub username: String,
    /// Local password; never returned in a payload.
    pub password: String,
    /// Optional six-digit TOTP code, required only when password-login MFA is enabled.
    pub totp_code: Option<String>,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// External media provider used for account linking or login.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalAccountProviderValue {
    /// Plex provider.
    Plex,
    /// Jellyfin provider.
    Jellyfin,
    /// Emby provider.
    Emby,
}

/// Media-server provider configured by the application.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaServerProviderValue {
    /// Jellyfin server.
    Jellyfin,
    /// Plex server.
    Plex,
    /// Emby server.
    Emby,
}

/// Emby connection mode.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EmbyConnectionModeValue {
    /// Connect directly to a local or remote Emby server.
    Local,
    /// Resolve and connect through Emby Connect.
    Connect,
}

/// Credential method for local Emby setup.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EmbyLocalSetupMethodValue {
    /// Use an Emby API key.
    ApiKey,
    /// Use an Emby administrator username and password.
    AdminCredentials,
}

/// Reachability result for an Emby Connect address.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EmbyConnectAddressStatusValue {
    /// Address responded successfully.
    Reachable,
    /// Address could not be reached.
    Unreachable,
    /// Address is not a valid URL.
    InvalidUrl,
    /// Address responded for a different server ID.
    ServerIdMismatch,
}

/// Emby Connect user classification.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EmbyConnectUserTypeValue {
    /// User is linked to the server.
    LinkedUser,
    /// User is a guest.
    Guest,
    /// Provider did not classify the user.
    Unknown,
}

impl MediaServerProviderValue {
    pub fn into_domain(self) -> scryer_domain::MediaServerProvider {
        match self {
            Self::Jellyfin => scryer_domain::MediaServerProvider::Jellyfin,
            Self::Plex => scryer_domain::MediaServerProvider::Plex,
            Self::Emby => scryer_domain::MediaServerProvider::Emby,
        }
    }

    pub fn from_domain(provider: scryer_domain::MediaServerProvider) -> Self {
        match provider {
            scryer_domain::MediaServerProvider::Jellyfin => Self::Jellyfin,
            scryer_domain::MediaServerProvider::Plex => Self::Plex,
            scryer_domain::MediaServerProvider::Emby => Self::Emby,
        }
    }
}

impl ExternalAccountProviderValue {
    pub fn into_domain(self) -> scryer_domain::ExternalAccountProvider {
        match self {
            Self::Plex => scryer_domain::ExternalAccountProvider::Plex,
            Self::Jellyfin => scryer_domain::ExternalAccountProvider::Jellyfin,
            Self::Emby => scryer_domain::ExternalAccountProvider::Emby,
        }
    }

    pub fn from_domain(provider: scryer_domain::ExternalAccountProvider) -> Self {
        match provider {
            scryer_domain::ExternalAccountProvider::Plex => Self::Plex,
            scryer_domain::ExternalAccountProvider::Jellyfin => Self::Jellyfin,
            scryer_domain::ExternalAccountProvider::Emby => Self::Emby,
        }
    }
}

/// State of an external account link.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalAccountStatusValue {
    /// Invite or link is awaiting a claim.
    PendingClaim,
    /// Link is active.
    Active,
    /// Link has been disabled.
    Disabled,
}

impl ExternalAccountStatusValue {
    pub fn from_domain(status: scryer_domain::ExternalAccountStatus) -> Self {
        match status {
            scryer_domain::ExternalAccountStatus::PendingClaim => Self::PendingClaim,
            scryer_domain::ExternalAccountStatus::Active => Self::Active,
            scryer_domain::ExternalAccountStatus::Disabled => Self::Disabled,
        }
    }
}

/// Plex credentials and the configured connection target used for login.
#[derive(InputObject)]
pub struct LoginWithPlexInput {
    /// ID of the configured Plex connection.
    pub connection_id: ID,
    /// Plex authentication token used only for login.
    pub plex_auth_token: String,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// Jellyfin credentials and optional MFA data for login.
#[derive(InputObject)]
pub struct LoginWithJellyfinInput {
    /// ID of the configured Jellyfin connection.
    pub connection_id: ID,
    /// Jellyfin username.
    pub username: String,
    /// Jellyfin password; never returned in a payload.
    pub password: String,
    /// Optional TOTP code used when Jellyfin-login MFA is enabled.
    pub totp_code: Option<String>,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// Emby credentials, connection mode, and optional MFA data for login.
#[derive(InputObject)]
pub struct LoginWithEmbyInput {
    /// ID of the configured Emby connection.
    pub connection_id: ID,
    /// Whether to use local Emby access or Emby Connect.
    pub mode: EmbyConnectionModeValue,
    /// Emby username.
    pub username: String,
    /// Emby password; never returned in a payload.
    pub password: String,
    /// Optional TOTP code used when Emby-login MFA is enabled.
    pub totp_code: Option<String>,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// WebAuthn assertion response paired with a previously issued challenge.
#[derive(InputObject)]
pub struct WebauthnCompleteInput {
    /// ID of the WebAuthn challenge to complete.
    pub challenge_id: ID,
    /// Browser assertion JSON; credential material is consumed for verification and not echoed.
    pub response_json: Json<serde_json::Value>,
    /// Whether the returned session should persist; absent or null uses the request policy.
    pub persist_session: Option<bool>,
}

/// WebAuthn registration response paired with a previously issued challenge.
#[derive(InputObject)]
pub struct WebauthnRegisterCompleteInput {
    /// ID of the WebAuthn registration challenge.
    pub challenge_id: ID,
    /// Browser registration JSON; credential material is consumed for verification and not echoed.
    pub response_json: Json<serde_json::Value>,
    /// Optional display name for the new passkey; absent or null leaves it unset.
    pub friendly_name: Option<String>,
}

/// Session or MFA-enrollment result returned after authentication.
#[derive(SimpleObject, Clone)]
pub struct LoginPayload {
    /// Access token or short-lived MFA-enrollment token; clients must treat it as secret.
    pub token: String,
    /// Authenticated user summary without password or credential secrets.
    pub user: UserPayload,
    /// Token expiry as a UTC timestamp.
    pub expires_at: DateTime<Utc>,
    /// UTC time through which MFA verification remains fresh, or null when not verified.
    pub mfa_verified_until: Option<DateTime<Utc>>,
    /// True when the token can only complete MFA enrollment.
    pub mfa_enrollment_required: bool,
    /// Whether the session was requested to persist.
    pub persist_session: bool,
}

/// TOTP enrollment completion request.
#[derive(InputObject)]
pub struct TotpEnrollmentCompleteInput {
    /// ID of the pending TOTP enrollment challenge.
    pub challenge_id: ID,
    /// Current TOTP code used to verify the enrollment.
    pub code: String,
}

/// TOTP verification request.
#[derive(InputObject)]
pub struct TotpVerifyInput {
    /// Current TOTP code; never returned in a payload.
    pub code: String,
}

/// TOTP enrollment and usage status without exposing the shared secret.
#[derive(SimpleObject, Clone)]
pub struct TotpStatusPayload {
    /// Whether TOTP is enabled.
    pub enabled: bool,
    /// UTC time when TOTP was enrolled, or null when never enrolled.
    pub created_at: Option<DateTime<Utc>>,
    /// UTC time when TOTP was last used, or null when unused.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Number of unused recovery codes remaining.
    pub recovery_codes_remaining: i32,
}

/// One-time values returned when TOTP enrollment starts.
#[derive(SimpleObject, Clone)]
pub struct TotpEnrollmentStartPayload {
    /// ID of the short-lived enrollment challenge.
    pub challenge_id: ID,
    /// `otpauth` URI for authenticator setup; treat it as secret.
    pub otpauth_url: String,
    /// Base32 TOTP secret; returned only for enrollment and must be protected as a secret.
    pub secret_base32: String,
    /// UTC time when the enrollment challenge expires.
    pub expires_at: DateTime<Utc>,
}

/// Result of completing TOTP enrollment or regenerating recovery codes.
#[derive(SimpleObject, Clone)]
pub struct TotpEnrollmentCompletePayload {
    /// Updated TOTP status without the shared secret.
    pub status: TotpStatusPayload,
    /// Newly generated one-time recovery codes; they are not returned again.
    pub recovery_codes: Vec<String>,
}

/// Result of completing MFA enrollment during login.
#[derive(SimpleObject, Clone)]
pub struct LoginMfaEnrollmentCompletePayload {
    /// Updated TOTP status without the shared secret.
    pub status: TotpStatusPayload,
    /// Newly generated one-time recovery codes.
    pub recovery_codes: Vec<String>,
    /// Authenticated login payload issued after enrollment completes.
    pub login: LoginPayload,
}

/// WebAuthn registration or authentication challenge options.
#[derive(SimpleObject, Clone)]
pub struct WebauthnChallengePayload {
    /// ID of the short-lived WebAuthn challenge.
    pub challenge_id: ID,
    /// Browser options JSON; it contains challenge data and should not be persisted as a credential.
    pub options_json: Json<serde_json::Value>,
}

/// Non-secret summary of a registered passkey.
#[derive(SimpleObject, Clone)]
pub struct PasskeySummaryPayload {
    /// ID of the passkey.
    pub id: ID,
    /// Optional user-assigned passkey name; null when none was saved.
    pub friendly_name: Option<String>,
    /// UTC time when the passkey was created.
    pub created_at: DateTime<Utc>,
    /// UTC time when the passkey was last used, or null when unused.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Acknowledgement containing the ID of a deleted passkey.
#[derive(SimpleObject, Clone)]
pub struct DeleteMyPasskeyPayload {
    /// ID of the deleted passkey.
    pub id: ID,
}

/// Non-secret summary of an OAuth grant.
#[derive(SimpleObject, Clone)]
pub struct OAuthConnectedAppPayload {
    /// Grant ID used to revoke this authorization.
    pub grant_id: ID,
    /// OAuth client identifier.
    pub client_id: String,
    /// OAuth client display name.
    pub client_name: String,
    /// UTC time when authorization was granted.
    pub authorized_at: DateTime<Utc>,
    /// UTC time when the grant was last used, or null when unused.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Result of revoking an OAuth grant.
#[derive(SimpleObject, Clone)]
pub struct RevokeMyOauthAppPayload {
    /// Grant ID targeted by the revoke request.
    pub grant_id: ID,
    /// False when the grant was already revoked or was not owned by the caller.
    pub revoked: bool,
}

/// External identifier from a provider.
#[derive(SimpleObject, Clone)]
pub struct ExternalIdPayload {
    /// Provider or source name.
    pub source: String,
    /// Identifier assigned by that source.
    pub value: String,
}

/// User who submitted a media request.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestRequesterPayload {
    /// ID of the requesting user.
    pub user_id: ID,
    /// Requesting user's username.
    pub username: String,
    /// Avatar URL, or null when unavailable.
    pub avatar_url: Option<String>,
    /// UTC time when this user submitted the request.
    pub requested_at: DateTime<Utc>,
}

/// Media request with current status, title identity, and resolution metadata.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestPayload {
    /// ID of the media request.
    pub id: ID,
    /// ID of the library targeted by the request.
    pub library_id: ID,
    /// Media facet targeted by the request.
    pub facet: MediaFacetValue,
    /// Current request lifecycle status.
    pub status: MediaRequestStatusValue,
    /// Stable identity fingerprint used to deduplicate requests.
    pub identity_fingerprint: String,
    /// Display title at request time.
    pub title: String,
    /// Sort title, or null when unavailable.
    pub sort_title: Option<String>,
    /// Provider slug, or null when unavailable.
    pub slug: Option<String>,
    /// Poster URL, or null when unavailable.
    pub poster_url: Option<String>,
    /// Release year, or null when unknown.
    pub year: Option<i32>,
    /// Overview text, or null when unavailable.
    pub overview: Option<String>,
    /// Runtime in minutes, or null when unknown.
    pub runtime_minutes: Option<i32>,
    /// Original language code, or null when unknown.
    pub language: Option<String>,
    /// Provider content status, or null when unavailable.
    pub content_status: Option<String>,
    /// ID of the quality profile requested, or null when none was selected.
    pub requested_quality_profile_id: Option<ID>,
    /// Name of the requested quality profile, or null when none was selected.
    pub requested_quality_profile_name: Option<String>,
    /// Requested monitoring mode, or null when not specified.
    pub requested_monitor_type: Option<MonitorTypeValue>,
    /// ID of the user who resolved the request, or null while unresolved.
    pub resolved_by_user_id: Option<ID>,
    /// UTC time when the request was resolved, or null while unresolved.
    pub resolved_at: Option<DateTime<Utc>>,
    /// ID of the title created from the request, or null when not created.
    pub created_title_id: Option<ID>,
    /// ID of the approved quality profile, or null before approval.
    pub approved_quality_profile_id: Option<ID>,
    /// Name of the approved quality profile, or null before approval.
    pub approved_quality_profile_name: Option<String>,
    /// Provider identifiers associated with the request.
    pub external_ids: Vec<ExternalIdPayload>,
    /// Users who submitted or joined the request.
    pub requesters: Vec<MediaRequestRequesterPayload>,
    /// ID of the user who created the request.
    pub created_by_user_id: ID,
    /// UTC time when the request was created.
    pub created_at: DateTime<Utc>,
    /// UTC time when the request was last changed.
    pub updated_at: DateTime<Utc>,
}

/// Event payload identifying a changed media request.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestChangedPayload {
    /// ID of the event.
    pub event_id: ID,
    /// Domain event type that caused the notification.
    pub event_type: DomainEventTypeValue,
    /// ID of the changed media request.
    pub request_id: ID,
    /// ID of the library containing the request.
    pub library_id: ID,
}

/// Provider catalog family used when describing configurable providers.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ProviderCatalogFamilyValue {
    /// Subtitle provider.
    Subtitle,
    /// Notification provider.
    Notification,
    /// Indexer provider.
    Indexer,
    /// Download-client provider.
    DownloadClient,
    /// Archive-extractor provider.
    ArchiveExtractor,
}

/// Result containing the ID of the submitted or deduplicated request.
#[derive(SimpleObject, Clone)]
pub struct SubmitMediaRequestPayload {
    /// ID of the submitted or deduplicated media request.
    pub request_id: ID,
}

/// Audio stream metadata for a media file.
#[derive(SimpleObject, Clone)]
pub struct AudioStreamDetailPayload {
    /// Codec name, or null when unavailable.
    pub codec: Option<String>,
    /// Number of audio channels, or null when unavailable.
    pub channels: Option<i32>,
    /// Language code, or null when unavailable.
    pub language: Option<String>,
    /// Bitrate in kilobits per second, or null when unavailable.
    pub bitrate_kbps: Option<i32>,
}

/// Subtitle stream metadata for a media file.
#[derive(SimpleObject, Clone)]
pub struct SubtitleStreamDetailPayload {
    /// Codec name, or null when unavailable.
    pub codec: Option<String>,
    /// Language code, or null when unavailable.
    pub language: Option<String>,
    /// Stream name, or null when unavailable.
    pub name: Option<String>,
    /// Whether the subtitle is forced.
    pub forced: bool,
    /// Whether the subtitle is the default stream.
    pub default: bool,
}

/// Current service health and catalog counters.
#[derive(SimpleObject, Clone)]
pub struct SystemHealthPayload {
    /// Whether the service is ready to accept work.
    pub service_ready: bool,
    /// Configured database path.
    pub db_path: String,
    /// Datastore engine name.
    pub datastore_engine: String,
    /// Datastore migration key, or null when unavailable.
    pub datastore_migration_key: Option<String>,
    /// Runtime path syntax.
    pub runtime_path_style: RuntimePathStyleValue,
    /// Total title count across facets.
    pub total_titles: i32,
    /// Number of monitored titles.
    pub monitored_titles: i32,
    /// Total user count.
    pub total_users: i32,
    /// Movie title count.
    pub titles_movie: i32,
    /// Series title count.
    pub titles_series: i32,
    /// Anime title count.
    pub titles_anime: i32,
    /// Titles outside the named facets.
    pub titles_other: i32,
    /// Number of recent events included in the preview window.
    pub recent_events: i32,
    /// Recent event preview messages in source order.
    pub recent_event_preview: Vec<String>,
    /// Datastore migration version, or null when unavailable.
    pub db_migration_version: Option<String>,
    /// Query statistics for configured indexers.
    pub indexer_stats: Vec<IndexerQueryStatsPayload>,
}

/// Runtime path information used by clients interpreting filesystem values.
#[derive(SimpleObject, Clone, Debug)]
pub struct RuntimeInfoPayload {
    /// Path syntax used by the running service.
    pub runtime_path_style: RuntimePathStyleValue,
}

/// Compatibility result comparing the connected SMG version with requirements.
#[derive(SimpleObject, Clone)]
pub struct SmgVersionCompatibilityNoticePayload {
    /// Compatibility status string.
    pub status: String,
    /// Minimum supported SMG version.
    pub minimum_version: String,
    /// Connected SMG version.
    pub your_version: String,
    /// Human-readable compatibility message.
    pub message: String,
    /// Optional UTC deadline for upgrading.
    pub upgrade_deadline: Option<String>,
}

/// Available Scryer update information.
#[derive(SimpleObject, Clone)]
pub struct SmgScryerUpdateNoticePayload {
    /// Whether a newer version is available.
    pub available: bool,
    /// Currently running version.
    pub current_version: String,
    /// Latest available version.
    pub latest_version: String,
    /// Latest release tag.
    pub latest_tag: String,
    /// Release URL, or null when unavailable.
    pub release_url: Option<String>,
    /// UTC publication time, or null when unavailable.
    pub published_at: Option<DateTime<Utc>>,
    /// UTC time when the check completed.
    pub checked_at: DateTime<Utc>,
}

/// Query and quota counters for one indexer.
#[derive(SimpleObject, Clone)]
pub struct IndexerQueryStatsPayload {
    /// ID of the indexer.
    pub indexer_id: ID,
    /// Configured indexer name.
    pub indexer_name: String,
    /// Queries during the trailing 24 hours.
    pub queries_last_24h: i32,
    /// Successful queries during the trailing 24 hours.
    pub successful_last_24h: i32,
    /// Failed queries during the trailing 24 hours.
    pub failed_last_24h: i32,
    /// Releases grabbed through this indexer during the trailing 24 hours, counted by Scryer rather than reported by the provider.
    pub grabs_last_24h: i32,
    /// UTC time of the most recent query, or null when never queried.
    pub last_query_at: Option<DateTime<Utc>>,
    /// Current API request count, or null when not reported.
    pub api_current: Option<i32>,
    /// API request limit, or null when not reported.
    pub api_max: Option<i32>,
    /// Current grab count, or null when not reported.
    pub grab_current: Option<i32>,
    /// Grab limit, or null when not reported.
    pub grab_max: Option<i32>,
}

/// User summary with authorization and credential-presence flags, never credential values.
#[derive(SimpleObject, Clone)]
pub struct UserPayload {
    /// ID of the user.
    pub id: ID,
    /// Login username.
    pub username: String,
    /// Whether password or external login is enabled.
    pub login_enabled: bool,
    /// Whether this is the default administrator account.
    pub is_default_admin: bool,
    /// Whether a password is configured, without revealing it.
    pub has_password: bool,
    /// Whether MFA is configured, without revealing its secret.
    pub has_mfa: bool,
    /// Whether a passkey is configured.
    pub has_passkey: bool,
    /// Local or externally provisioned account origin.
    pub account_kind: UserAccountKindValue,
    /// Application-wide permissions granted to the user.
    pub app_permissions: Vec<AppPermissionValue>,
    /// Library-specific permission grants.
    pub library_permissions: Vec<UserLibraryPermissionGrantPayload>,
}

/// External account link summary without provider credentials.
#[derive(SimpleObject, Clone)]
pub struct LinkedAccountPayload {
    /// ID of the linked account.
    pub id: ID,
    /// ID of the linked local user.
    pub user_id: ID,
    /// External provider.
    pub provider: ExternalAccountProviderValue,
    /// ID of the configured media-server connection.
    pub connection_id: ID,
    /// Provider-specific user ID, or null when unavailable.
    pub external_user_id: Option<String>,
    /// Provider username.
    pub username: String,
    /// Provider display name, or null when unavailable.
    pub display_name: Option<String>,
    /// Provider avatar URL, or null when unavailable.
    pub avatar_url: Option<String>,
    /// Current link status.
    pub status: ExternalAccountStatusValue,
    /// UTC verification time, or null when not verified.
    pub verified_at: Option<DateTime<Utc>>,
    /// UTC time of the last successful login, or null when unused.
    pub last_login_at: Option<DateTime<Utc>>,
    /// UTC time when the link was created.
    pub created_at: DateTime<Utc>,
    /// UTC time when the link was last changed.
    pub updated_at: DateTime<Utc>,
}

/// Permissions granted to one library for one user.
#[derive(SimpleObject, Clone)]
pub struct UserLibraryPermissionGrantPayload {
    /// ID of the library receiving the grant.
    pub library_id: ID,
    /// Permissions granted within that library.
    pub permissions: Vec<LibraryPermissionValue>,
}

/// Audit event summary.
#[derive(SimpleObject, Clone)]
pub struct EventPayload {
    /// ID of the event.
    pub id: ID,
    /// Event name as stored by the event source.
    pub event_type: String,
    /// Origin of the actor that caused the event.
    pub actor_kind: ActorKindValue,
    /// ID of the user actor, or null for anonymous or system events.
    pub actor_user_id: Option<ID>,
    /// Display name of the actor.
    pub actor_display_name: String,
    /// ID of the affected title, or null when not title-scoped.
    pub title_id: Option<ID>,
    /// Human-readable event message.
    pub message: String,
    /// UTC time when the event occurred.
    pub occurred_at: DateTime<Utc>,
}

/// Activity notification with delivery channels and actor context.
#[derive(SimpleObject, Clone)]
pub struct ActivityEventPayload {
    /// ID of the activity event.
    pub id: ID,
    /// Activity kind.
    pub kind: ActivityKindValue,
    /// Activity severity.
    pub severity: ActivitySeverityValue,
    /// Channels associated with the activity.
    pub channels: Vec<ActivityChannelValue>,
    /// Origin of the actor that caused the activity.
    pub actor_kind: ActorKindValue,
    /// ID of the user actor, or null for anonymous or system activity.
    pub actor_user_id: Option<ID>,
    /// Display name of the actor.
    pub actor_display_name: String,
    /// ID of the affected title, or null when not title-scoped.
    pub title_id: Option<ID>,
    /// Media facet, or null when not facet-scoped.
    pub facet: Option<MediaFacetValue>,
    /// Human-readable activity message.
    pub message: String,
    /// UTC time when the activity occurred.
    pub occurred_at: DateTime<Utc>,
}

/// Ordered domain event envelope for stream subscriptions.
#[derive(SimpleObject, Clone)]
pub struct DomainEventEnvelopePayload {
    /// Monotonically increasing stream sequence.
    pub sequence: Long,
    /// ID of the event.
    pub event_id: ID,
    /// UTC time when the event occurred.
    pub occurred_at: DateTime<Utc>,
    /// Origin of the actor that caused the event.
    pub actor_kind: ActorKindValue,
    /// ID of the user actor, or null for anonymous or system events.
    pub actor_user_id: Option<ID>,
    /// Display name of the actor.
    pub actor_display_name: String,
    /// ID of the affected title, or null when not title-scoped.
    pub title_id: Option<ID>,
    /// Media facet, or null when not facet-scoped.
    pub facet: Option<MediaFacetValue>,
    /// Typed domain event name.
    pub event_type: DomainEventTypeValue,
    /// Stream category containing the event.
    pub stream_kind: StreamKindValue,
    /// ID of the stream target, or null for the global stream.
    pub stream_id: Option<ID>,
    /// Event-specific JSON payload.
    pub payload_json: Json<serde_json::Value>,
}

/// Stable key identifying a scheduled or manually triggered job.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobKeyValue {
    /// Movie library scan.
    LibraryScanMovies,
    /// Series library scan.
    LibraryScanSeries,
    /// Anime library scan.
    LibraryScanAnime,
    /// Background movie library refresh.
    BackgroundLibraryRefreshMovies,
    /// Background series library refresh.
    BackgroundLibraryRefreshSeries,
    /// Background anime library refresh.
    BackgroundLibraryRefreshAnime,
    /// RSS synchronization.
    RssSync,
    /// Subtitle search.
    SubtitleSearch,
    /// Plugin registry refresh.
    PluginRegistryRefresh,
    /// Housekeeping work.
    Housekeeping,
    /// Health checks.
    HealthChecks,
    /// Automatic backup.
    AutoBackup,
    /// Prowlarr synchronization.
    ProwlarrSync,
    /// Pending-release processing.
    PendingReleaseProcessing,
    /// Staged NZB pruning.
    StagedNzbPrune,
    /// Discovery synchronization.
    DiscoverySync,
    /// Title-image cache refresh.
    TitleImageCacheRefresh,
    /// Title deletion.
    TitleDeletion,
    /// Media-file deletion.
    MediaFileDeletion,
    /// Recycle-bin restore.
    RecycleBinRestore,
    /// Recycle-bin purge.
    RecycleBinPurge,
    /// Acquisition search.
    AcquisitionSearch,
}

/// Broad category assigned to a job.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobCategoryValue {
    /// Library work.
    Library,
    /// Acquisition work.
    Acquisition,
    /// Maintenance work.
    Maintenance,
    /// Subtitle work.
    Subtitles,
    /// System work.
    System,
}

/// Operational grouping assigned to a job.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobSectionValue {
    /// Primary operational jobs.
    Primary,
    /// Maintenance jobs.
    Maintenance,
}

/// Schedule rule used by a job definition.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobScheduleKindValue {
    /// Runs only when explicitly triggered.
    Manual,
    /// Repeats at a fixed interval.
    Interval,
    /// Runs at startup and then at a fixed interval.
    StartupAndInterval,
    /// Runs once per day at a configured local time.
    DailyAtTime,
}

/// Source that started a job run.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobTriggerSourceValue {
    /// Started by an explicit user or API action.
    Manual,
    /// Started by scheduled application startup.
    ScheduledStartup,
    /// Started by an interval schedule.
    ScheduledInterval,
    /// Started by a daily schedule.
    ScheduledDaily,
    /// Started internally by the system.
    SystemInternal,
}

/// Lifecycle state of a job run.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobRunStatusValue {
    /// Run accepted but not started.
    Queued,
    /// Run is discovering work.
    Discovering,
    /// Run is executing work.
    Running,
    /// Run completed successfully.
    Completed,
    /// Run completed with non-fatal issues.
    Warning,
    /// Run failed.
    Failed,
}

/// Mode used by a library scan.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryScanModeValue {
    /// Reconcile the full library.
    Full,
    /// Add newly discovered content without a full reconciliation.
    Additive,
}

/// Lifecycle state of a library scan.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryScanStatusValue {
    /// Scan is discovering files or titles.
    Discovering,
    /// Scan is processing discovered work.
    Running,
    /// Scan completed successfully.
    Completed,
    /// Scan was canceled.
    Canceled,
    /// Scan completed with non-fatal issues.
    Warning,
    /// Scan failed.
    Failed,
}

/// Counts for one phase of a library scan.
#[derive(SimpleObject, Clone)]
pub struct LibraryScanPhaseProgressPayload {
    /// Total units in this phase; zero is valid when no work was found.
    pub total: i32,
    /// Units completed so far.
    pub completed: i32,
    /// Units that failed in this phase.
    pub failed: i32,
}

/// Progress snapshot for one library scan.
#[derive(SimpleObject, Clone)]
pub struct LibraryScanProgressPayload {
    /// ID of the scan session.
    pub session_id: ID,
    /// Media facet being scanned.
    pub facet: MediaFacetValue,
    /// ID of the library being scanned, or null for all libraries in the facet.
    pub library_id: Option<ID>,
    /// Scan mode.
    pub mode: LibraryScanModeValue,
    /// Current scan status.
    pub status: LibraryScanStatusValue,
    /// UTC time when the scan started.
    pub started_at: DateTime<Utc>,
    /// UTC time when the progress snapshot was updated.
    pub updated_at: DateTime<Utc>,
    /// Number of titles found so far.
    pub found_titles: i32,
    /// Whether the total title-match count is known.
    pub title_match_total_known: bool,
    /// Title matching progress counts.
    pub title_match_progress: LibraryScanPhaseProgressPayload,
    /// Whether the total hydration count is known.
    pub hydration_total_known: bool,
    /// Metadata hydration progress counts.
    pub hydration_progress: LibraryScanPhaseProgressPayload,
    /// Whether the total media-analysis count is known.
    pub media_analysis_total_known: bool,
    /// Media-analysis progress counts.
    pub media_analysis_progress: LibraryScanPhaseProgressPayload,
    /// Final scan summary, or null until a summary is available.
    pub summary: Option<LibraryScanSummaryPayload>,
}

/// Schedule details for a job definition.
#[derive(SimpleObject, Clone)]
pub struct JobScheduleInfoPayload {
    /// Schedule kind.
    pub kind: JobScheduleKindValue,
    /// Human-readable schedule description.
    pub description: String,
    /// Repeat interval in seconds, or null for non-interval schedules.
    pub interval_seconds: Option<i32>,
    /// Initial delay in seconds, or null when not applicable.
    pub initial_delay_seconds: Option<i32>,
    /// Next scheduled run as a UTC timestamp, or null when no run is scheduled.
    pub next_run_at: Option<DateTime<Utc>>,
}

/// Metadata describing a configured job.
#[derive(SimpleObject, Clone)]
pub struct JobDefinitionPayload {
    /// Stable job key.
    pub key: JobKeyValue,
    /// Display name of the job.
    pub display_name: String,
    /// Human-readable job description.
    pub description: String,
    /// Broad job category.
    pub category: JobCategoryValue,
    /// Job section.
    pub section: JobSectionValue,
    /// Whether an explicit trigger is allowed.
    pub manual_trigger_allowed: bool,
    /// Whether the run reports library-scan progress.
    pub uses_library_scan_progress: bool,
    /// Configured schedule details.
    pub schedule: JobScheduleInfoPayload,
}

/// Current status and progress of one job run.
#[derive(SimpleObject, Clone)]
pub struct JobRunPayload {
    /// ID of the job run.
    pub id: ID,
    /// Stable key of the job being run.
    pub job_key: JobKeyValue,
    /// Display name of the job.
    pub display_name: String,
    /// Broad job category.
    pub category: JobCategoryValue,
    /// Job section.
    pub section: JobSectionValue,
    /// Current run status.
    pub status: JobRunStatusValue,
    /// Source that started the run.
    pub trigger_source: JobTriggerSourceValue,
    /// UTC time when the run started.
    pub started_at: DateTime<Utc>,
    /// UTC completion time, or null while the run is active.
    pub completed_at: Option<DateTime<Utc>>,
    /// Structured result summary, or null before completion or when unavailable.
    pub summary_json: Option<Json<serde_json::Value>>,
    /// Human-readable result summary, or null when unavailable.
    pub summary_text: Option<String>,
    /// Failure detail, or null when the run has not failed.
    pub error_text: Option<String>,
    /// Structured progress data, or null when the job does not expose it.
    pub progress_json: Option<Json<serde_json::Value>>,
    /// Library-scan progress, or null for jobs without scan progress.
    pub library_scan_progress: Option<LibraryScanProgressPayload>,
}

#[derive(SimpleObject, Clone)]
/// Discovery synchronization state and the number of context changes awaiting processing.
pub struct DiscoverySyncStatusPayload {
    /// Current synchronization lifecycle state and next eligible work times.
    pub state: DiscoverySyncStatePayload,
    /// Number of discovery context changes not yet incorporated into a completed snapshot.
    pub pending_context_change_count: Long,
}

#[derive(SimpleObject, Clone)]
/// Generation markers, completion timestamps, and eligibility timestamps for discovery synchronization.
pub struct DiscoverySyncStatePayload {
    /// Generation ID of the last successfully completed context snapshot, or null before the first success.
    pub last_success_generation_id: Option<ID>,
    /// Generation ID of the last completed public-feed build, or null when none has completed.
    pub last_public_feed_generation_id: Option<ID>,
    /// UTC completion time of the last context snapshot, or null when none has completed.
    pub last_context_snapshot_completed_at: Option<DateTime<Utc>>,
    /// UTC completion time of the last incremental reload, or null when none has completed.
    pub last_incremental_reload_completed_at: Option<DateTime<Utc>>,
    /// UTC completion time of the last public-feed build, or null when none has completed.
    pub last_public_feed_completed_at: Option<DateTime<Utc>>,
    /// UTC time when another context snapshot becomes eligible, or null when no schedule is set.
    pub next_context_snapshot_eligible_at: Option<DateTime<Utc>>,
    /// UTC time when another incremental reload becomes eligible, or null when no schedule is set.
    pub next_incremental_reload_eligible_at: Option<DateTime<Utc>>,
    /// UTC time when another public-feed build becomes eligible, or null when no schedule is set.
    pub next_public_feed_eligible_at: Option<DateTime<Utc>>,
    /// UTC time when this synchronization state was last updated.
    pub updated_at: DateTime<Utc>,
}

#[derive(InputObject, Clone, Default)]
/// Controls which discovery home surfaces are included and how many items each section may contain.
pub struct DiscoveryHomeInput {
    /// Include public discovery sections; defaults to true when omitted.
    pub include_public: Option<bool>,
    /// Include personalized sections when the caller is authorized; defaults to true when omitted.
    pub include_personalized: Option<bool>,
    /// Include unresolved external discovery items; defaults to false when omitted.
    pub include_unresolved: Option<bool>,
    /// Maximum items per section; defaults to 25 and must be between 1 and 100.
    pub limit_per_section: Option<i32>,
    /// Optional content, tag, studio, year, and rating filters.
    pub filters: Option<DiscoveryHomeFiltersInput>,
}

#[derive(InputObject, Clone, Default)]
/// Filters for discovery home items; omitted lists are treated as empty filters.
pub struct DiscoveryHomeFiltersInput {
    /// Restrict results to these media facets; omitted includes all facets.
    pub content_types: Option<Vec<MediaFacetValue>>,
    /// Canonical genre tag keys to include; blank keys are invalid.
    pub genre_tag_keys: Option<Vec<String>>,
    /// Canonical theme tag keys to include; blank keys are invalid.
    pub theme_tag_keys: Option<Vec<String>>,
    /// Studio slugs to include; omitted applies no studio restriction.
    pub studio_slugs: Option<Vec<String>>,
    /// Inclusive minimum release year, if supplied.
    pub minimum_year: Option<i32>,
    /// Inclusive maximum release year, if supplied.
    pub maximum_year: Option<i32>,
    /// Inclusive rating floor on a finite 0 through 10 scale.
    pub minimum_rating: Option<f64>,
}

#[derive(InputObject, Clone, Default)]
/// Selects public, personalized, and unresolved discovery sources for filter-option lookup.
pub struct DiscoveryHomeFilterOptionsInput {
    /// Include public filter options; defaults to true when omitted.
    pub include_public: Option<bool>,
    /// Include personalized filter options when authorized; defaults to true when omitted.
    pub include_personalized: Option<bool>,
    /// Include unresolved items in option generation; defaults to false when omitted.
    pub include_unresolved: Option<bool>,
}

#[derive(InputObject, Clone, Default)]
/// Filters and paginates the general discovery-item listing.
pub struct DiscoveryItemsInput {
    /// Free-text query applied to discovery titles and searchable metadata.
    pub query: Option<String>,
    /// Target-kind values to include; omitted uses no target-kind filter.
    pub target_kinds: Option<Vec<String>>,
    /// Source identifiers to include; omitted uses all available sources.
    pub sources: Option<Vec<String>>,
    /// Relation types to include; omitted applies no relation-type filter.
    pub relation_types: Option<Vec<String>>,
    /// Relation subtypes to include; omitted applies no relation-subtype filter.
    pub relation_subtypes: Option<Vec<String>>,
    /// Genre values to include; omitted applies no genre filter.
    pub genres: Option<Vec<String>>,
    /// Status tags to include; omitted applies no status-tag filter.
    pub status_tags: Option<Vec<String>>,
    /// Facet terms to include; omitted applies no facet-term filter.
    pub facet_terms: Option<Vec<String>>,
    /// Include items already owned in the caller's libraries; defaults to false.
    pub include_owned: Option<bool>,
    /// Include unresolved items; defaults to false.
    pub include_unresolved: Option<bool>,
    /// Include public-source items; defaults to false.
    pub include_public: Option<bool>,
    /// Page size; defaults to 50 and values below 1 become 1.
    pub limit: Option<i32>,
    /// Zero-based offset; defaults to 0 and negative values become 0.
    pub offset: Option<i32>,
}

#[derive(InputObject, Clone)]
/// Identifies one discovery item and controls whether unresolved records may be returned.
pub struct DiscoveryItemDetailInput {
    /// Stable discovery target key, not a local title ID.
    pub target_key: String,
    /// Include the unresolved record when no local title is resolved; defaults to true.
    pub include_unresolved: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Discovery home response with synchronization status, selected sections, facets, and authorization state.
pub struct DiscoveryHomePayload {
    /// Synchronization readiness and pending-context information used to interpret the result.
    pub status: DiscoverySyncStatusPayload,
    /// Selected hero item, or null when no eligible item exists.
    pub hero_item: Option<DiscoveryItemPayload>,
    /// Public sections; an empty list means no public section was selected.
    pub public_sections: Vec<DiscoverySectionPayload>,
    /// Personalized sections; empty when unavailable or no items qualify.
    pub personalized_sections: Vec<DiscoverySectionPayload>,
    /// Complete-collection section, or null when none is available.
    pub complete_collection: Option<DiscoverySectionPayload>,
    /// Facet summaries returned for the selected discovery scope.
    pub facets: Vec<DiscoveryFacetPayload>,
    /// Whether the caller may view personalized results.
    pub can_view_personalized: bool,
}

#[derive(SimpleObject, Clone)]
/// Card-oriented discovery home response with synchronization status and optional hero and sections.
pub struct DiscoveryHomeCardsPayload {
    /// Synchronization readiness and pending-context information.
    pub status: DiscoverySyncStatusPayload,
    /// Selected hero card, or null when no eligible card exists.
    pub hero_item: Option<DiscoveryHomeHeroPayload>,
    /// Public card sections; empty means no public section was selected.
    pub public_sections: Vec<DiscoveryHomeSectionPayload>,
    /// Personalized card sections; empty when unavailable or no items qualify.
    pub personalized_sections: Vec<DiscoveryHomeSectionPayload>,
    /// Complete-collection card section, or null when none is available.
    pub complete_collection: Option<DiscoveryHomeSectionPayload>,
    /// Whether the caller may view personalized cards.
    pub can_view_personalized: bool,
}

#[derive(SimpleObject, Clone)]
/// A titled discovery home section with its source surface, total count, and bounded items.
pub struct DiscoveryHomeSectionPayload {
    /// Stable section identifier.
    pub section_id: String,
    /// Section classification value.
    pub section_type: String,
    /// Human-readable section title.
    pub title: String,
    /// Whether this section is public, personalized, or mixed.
    pub surface: DiscoverySurfaceValue,
    /// Total matching items before the returned item page is applied.
    pub total_count: Long,
    /// Items returned for this section, possibly fewer than the total.
    pub items: Vec<DiscoveryHomeCardPayload>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Origin surface of a discovery home section.
pub enum DiscoverySurfaceValue {
    /// Publicly available discovery content.
    Public,
    /// Personalized content based on caller-visible context.
    Personalized,
    /// A section combining public and personalized content.
    Mixed,
}

#[derive(SimpleObject, Clone)]
/// Compact discovery card identity and display metadata.
pub struct DiscoveryHomeCardPayload {
    /// Stable local or discovery-card ID for this result.
    pub id: ID,
    /// Stable discovery target key used for detail lookup.
    pub target_key: String,
    /// Kind of target represented by the card.
    pub target_kind: MediaFacetValue,
    /// Display title selected for this card.
    pub display_title: String,
    /// Original source title when it differs from the display title, or null.
    pub original_title: Option<String>,
    /// Normalized sort title when available, or null.
    pub sort_title: Option<String>,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Poster URL, or null when no poster is available.
    pub poster_url: Option<String>,
    /// Content facet represented by the card.
    pub content_type: MediaFacetValue,
    /// Whether the source metadata marks the content as adult.
    pub is_adult: bool,
    /// Whether the target is already owned in the input scope.
    pub owned_in_input: bool,
}

#[derive(SimpleObject, Clone)]
/// Expanded discovery hero metadata, ratings, tags, and ownership state.
pub struct DiscoveryHomeHeroPayload {
    /// Stable local or discovery-card ID for this result.
    pub id: ID,
    /// Stable discovery target key used for detail lookup.
    pub target_key: String,
    /// Kind of target represented by the hero.
    pub target_kind: MediaFacetValue,
    /// Display title selected for the hero.
    pub display_title: String,
    /// Original source title when it differs from the display title, or null.
    pub original_title: Option<String>,
    /// Normalized sort title when available, or null.
    pub sort_title: Option<String>,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Poster URL, or null when no poster is available.
    pub poster_url: Option<String>,
    /// Background artwork URL, or null when unavailable.
    pub background_url: Option<String>,
    /// Overview text, or null when unavailable.
    pub overview: Option<String>,
    /// Content facet represented by the hero.
    pub content_type: MediaFacetValue,
    /// Whether the source metadata marks the content as adult.
    pub is_adult: bool,
    /// Primary rating on the source's normalized scale, or null when unavailable.
    pub rating: Option<f64>,
    /// Names of rating sources contributing to the primary rating.
    pub rating_sources: Vec<String>,
    /// External ratings with source-specific values and vote counts.
    pub external_ratings: Vec<DiscoveryExternalRatingPayload>,
    /// Canonical genre and theme tags associated with the hero.
    pub genre_tags: Vec<CanonicalMediaTagPayload>,
    /// Number of subject matches contributing to this hero selection.
    pub matched_subject_count: i32,
    /// Whether the target is already owned in the input scope.
    pub owned_in_input: bool,
}

#[derive(SimpleObject, Clone)]
/// Available discovery-home filter values for genres, themes, and studios.
pub struct DiscoveryHomeFilterOptionsPayload {
    /// Canonical genre options; empty means no matching genre option exists.
    pub genres: Vec<CanonicalTagFilterOptionPayload>,
    /// Canonical theme options; empty means no matching theme option exists.
    pub themes: Vec<CanonicalTagFilterOptionPayload>,
    /// Studio slugs available in the selected scope.
    pub studio_slugs: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// Canonical tag key and display name used by discovery filters.
pub struct CanonicalTagFilterOptionPayload {
    /// Stable canonical tag key accepted by discovery filters.
    pub key: String,
    /// Display name associated with the canonical key.
    pub name: String,
}

#[derive(SimpleObject, Clone)]
/// Paged discovery-item results with a total count and personalization capability.
pub struct DiscoveryItemsPayload {
    /// Items in the requested page; empty means no items matched.
    pub items: Vec<DiscoveryItemPayload>,
    /// Total number of matching items before pagination.
    pub total_count: Long,
    /// Whether the caller may view personalized results.
    pub can_view_personalized: bool,
}

#[derive(InputObject, Clone)]
/// Controls catalog discovery for one required media facet and optional library scope.
pub struct CatalogDiscoveryInput {
    /// Required media facet whose catalog is searched.
    pub facet: MediaFacetValue,
    /// Library IDs defining the owned scope; omitted or empty means no explicit library restriction.
    pub library_ids: Option<Vec<ID>>,
    /// Include unresolved discovery records; defaults to true.
    pub include_unresolved: Option<bool>,
    /// Maximum items per result group; defaults to 12 and values below 1 become 1.
    pub limit_per_group: Option<i32>,
    /// Maximum number of groups; defaults to 6 and values below 1 become 1.
    pub max_groups: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Catalog discovery response containing authorization state and grouped results.
pub struct CatalogDiscoveryPayload {
    /// Whether the caller may view personalized groups.
    pub can_view_personalized: bool,
    /// Discovery groups returned in selection order; empty means no group qualified.
    pub groups: Vec<CatalogDiscoveryGroupPayload>,
}

#[derive(SimpleObject, Clone)]
/// One catalog discovery group with surface, count, and bounded items.
pub struct CatalogDiscoveryGroupPayload {
    /// Stable group identifier.
    pub id: String,
    /// Reason this group was selected.
    pub kind: CatalogDiscoveryGroupKindValue,
    /// Public or personalized origin of the group.
    pub surface: CatalogDiscoverySurfaceValue,
    /// Optional label value, such as a genre or theme key.
    pub label_value: Option<String>,
    /// Total matching items before the returned item page is applied.
    pub total_count: Long,
    /// Items returned for this group.
    pub items: Vec<DiscoveryItemPayload>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Classification of a catalog discovery group.
pub enum CatalogDiscoveryGroupKindValue {
    /// Top public recommendations.
    PublicTop,
    /// A named public section.
    PublicSection,
    /// Personalized genre affinity.
    GenreAffinity,
    /// Personalized theme affinity.
    ThemeAffinity,
    /// Acclaimed content group.
    Acclaimed,
    /// Complete-collection group.
    CompleteCollection,
    /// Fallback group used when a more specific group is unavailable.
    Fallback,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Origin surface of a catalog discovery group.
pub enum CatalogDiscoverySurfaceValue {
    /// Publicly available group.
    Public,
    /// Personalized group based on caller-visible context.
    Personalized,
}

#[derive(SimpleObject, Clone)]
/// A discovery section containing a title, origin surface, count, and items.
pub struct DiscoverySectionPayload {
    /// Stable section identifier.
    pub section_id: String,
    /// Section classification value.
    pub section_type: String,
    /// Human-readable section title.
    pub title: String,
    /// Surface label for this section.
    pub surface: String,
    /// Total matching items before the returned item page is applied.
    pub total_count: Long,
    /// Items returned for this section.
    pub items: Vec<DiscoveryItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Rating from an external source, including normalized score and optional vote count.
pub struct DiscoveryExternalRatingPayload {
    /// External rating source name.
    pub source: String,
    /// Source-specific raw value, or null when unavailable.
    pub value: Option<f64>,
    /// Source-specific score, or null when unavailable.
    pub score: Option<f64>,
    /// Normalized score used for cross-source comparison.
    pub normalized: f64,
    /// Number of votes reported by the source, or null when unavailable.
    pub votes: Option<i32>,
    /// Source URL for the rating record.
    pub url: String,
}

#[derive(SimpleObject, Clone)]
/// External identifier associated with a discovery item.
pub struct DiscoveryExternalIdPayload {
    /// External source name.
    pub source: String,
    /// External identifier kind.
    pub kind: String,
    /// Provider-issued identifier value.
    pub id: String,
    /// Canonical composite key for the external identity.
    pub key: String,
}

#[derive(SimpleObject, Clone)]
/// Content certification for one country and release type.
pub struct DiscoveryContentCertificationPayload {
    /// Certification value, such as an age category.
    pub value: String,
    /// Authority or source that supplied the certification.
    pub source: String,
    /// Release-type code, or null when the source did not specify one.
    pub release_type: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Country-specific content ratings and their certifications.
pub struct DiscoveryContentRatingPayload {
    /// ISO-like country code supplied by the source.
    pub country: String,
    /// Certifications reported for this country.
    pub certifications: Vec<DiscoveryContentCertificationPayload>,
    /// Numeric age rating, or null when unavailable.
    pub age_rating: Option<i32>,
    /// Source of the numeric age rating, or null when unavailable.
    pub age_rating_source: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Canonical metadata tag with provenance and content-safety flags.
pub struct CanonicalMediaTagPayload {
    /// Stable canonical tag key.
    pub key: String,
    /// Canonical tag category.
    pub category: String,
    /// Display name for the tag.
    pub name: String,
    /// Confidence score from 0 through 1 when supplied by the source.
    pub confidence: Option<f64>,
    /// Source names contributing this tag.
    pub sources: Vec<String>,
    /// Original provider tag keys that mapped to this canonical tag.
    pub source_tag_keys: Vec<String>,
    /// Whether the tag is marked adult.
    pub is_adult: bool,
    /// Whether the tag is marked spoiler-sensitive.
    pub is_spoiler: bool,
}

#[derive(SimpleObject, Clone)]
/// Detailed discovery item with resolution, metadata, relationships, provenance, and ownership state.
pub struct DiscoveryItemPayload {
    /// Stable local or discovery-item ID.
    pub id: ID,
    /// Stable discovery target key used for detail lookup.
    pub target_key: String,
    /// Target kind string supplied by the discovery source.
    pub target_kind: String,
    /// Whether the target resolved to a local title.
    pub resolved: bool,
    /// Local title ID when resolved, or null when unresolved.
    pub resolved_title_id: Option<ID>,
    /// Display title selected from available metadata.
    pub display_title: String,
    /// Original source title when available.
    pub original_title: Option<String>,
    /// Normalized sort title when available.
    pub sort_title: Option<String>,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Poster URL, or null when unavailable.
    pub poster_url: Option<String>,
    /// Background artwork URL, or null when unavailable.
    pub background_url: Option<String>,
    /// Overview text, or null when unavailable.
    pub overview: Option<String>,
    /// Content facet string, or null when the source did not classify it.
    pub content_type: Option<String>,
    /// Canonical tags associated with the item.
    pub canonical_tags: Vec<CanonicalMediaTagPayload>,
    /// Whether the source metadata marks the content as adult.
    pub is_adult: bool,
    /// Country-specific content ratings.
    pub content_ratings: Vec<DiscoveryContentRatingPayload>,
    /// Primary normalized rating, or null when unavailable.
    pub rating: Option<f64>,
    /// Names of sources contributing to the primary rating.
    pub rating_sources: Vec<String>,
    /// External ratings from individual providers.
    pub external_ratings: Vec<DiscoveryExternalRatingPayload>,
    /// External IDs associated with the item.
    pub external_ids: Vec<DiscoveryExternalIdPayload>,
    /// Source-derived status tags.
    pub status_tags: Vec<String>,
    /// Source-derived tags not mapped to canonical tags.
    pub source_tags: Vec<String>,
    /// Source identifiers contributing this item.
    pub sources: Vec<String>,
    /// Highest-priority source, or null when no source is preferred.
    pub best_source: Option<String>,
    /// Relation types connecting this item to other discovery subjects.
    pub relation_types: Vec<String>,
    /// Relation subtypes connecting this item to other discovery subjects.
    pub relation_subtypes: Vec<String>,
    /// Number of contributing sources, or null when not computed.
    pub source_count: Option<i32>,
    /// Number of discovery edges, or null when not computed.
    pub edge_count: Option<i32>,
    /// Number of relations, or null when not computed.
    pub relation_count: Option<i32>,
    /// Number of matched source subjects, or null when not computed.
    pub source_subject_count: Option<i32>,
    /// Ranking score used for ordering, or null when not computed.
    pub rank_score: Option<f64>,
    /// Titles of matched subjects contributing to this item.
    pub matched_subject_titles: Vec<String>,
    /// Number of matched subjects contributing to this item.
    pub matched_subject_count: i32,
    /// TMDB collection ID, or null when unavailable.
    pub tmdb_collection_id: Option<String>,
    /// TMDB collection name, or null when unavailable.
    pub tmdb_collection_name: Option<String>,
    /// Whether the target is already owned in the input scope.
    pub owned_in_input: bool,
    /// Studio slug, or null when unavailable.
    pub studio_slug: Option<String>,
    /// Numeric person IDs associated with the item.
    pub person_ids: Vec<i32>,
    /// Facet terms extracted from the source.
    pub facet_terms: Vec<String>,
    /// Context terms extracted from the source.
    pub context_terms: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// Discovery facet counts for external and local catalog matches.
pub struct DiscoveryFacetPayload {
    /// Human-readable facet name.
    pub name: String,
    /// Stable facet value.
    pub value: String,
    /// External discovery count, or null when unavailable.
    pub smg_count: Option<Long>,
    /// Local owned-title count, or null when unavailable.
    pub local_count: Option<Long>,
}

#[derive(SimpleObject, Clone)]
/// One title release blocklist entry and the episodes it affects.
pub struct TitleReleaseBlocklistEntryPayload {
    /// Blocklist entry ID.
    pub id: ID,
    /// Download source locator used to identify the blocked release, or null when unavailable.
    pub source_hint: Option<String>,
    /// Release title recorded with the entry, or null when unavailable.
    pub source_title: Option<String>,
    /// Failure or blocklist reason, or null when unavailable.
    pub error_message: Option<String>,
    /// UTC time when the release was attempted.
    pub attempted_at: DateTime<Utc>,
    /// Episode IDs targeted by this blocklist entry.
    pub episode_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Indexer search result with parsed release metadata, scoring, and queue eligibility.
pub struct IndexerSearchResultPayload {
    /// Indexer source name.
    pub source: String,
    /// Release title shown by the indexer.
    pub title: String,
    /// Informational release link, or null when unavailable.
    pub link: Option<String>,
    /// Direct download URL, or null when unavailable.
    pub download_url: Option<String>,
    /// Source kind such as Usenet or torrent, or null when unknown.
    pub source_kind: Option<DownloadSourceKindValue>,
    /// Release size in bytes, or null when unknown.
    pub size_bytes: Option<Long>,
    /// Publication time in UTC, or null when unavailable.
    pub published_at: Option<DateTime<Utc>>,
    /// Positive vote count, or null when not reported.
    pub thumbs_up: Option<i32>,
    /// Negative vote count, or null when not reported.
    pub thumbs_down: Option<i32>,
    /// Parsed release fields, or null when parsing did not produce a result.
    pub parsed_release: Option<ParsedReleasePayload>,
    /// Quality-profile decision, or null when no profile was evaluated.
    pub quality_profile_decision: Option<QualityProfileDecisionPayload>,
    // Torrent-specific fields
    /// Torrent seeder count, or null for non-torrent results.
    pub seeders: Option<i32>,
    /// Torrent peer count, or null for non-torrent results.
    pub peers: Option<i32>,
    /// Torrent info hash, or null for non-torrent results.
    pub info_hash: Option<String>,
    /// Whether the torrent is marked freeleech, or null for non-torrent results.
    pub freeleech: Option<bool>,
    /// Torrent download volume factor, or null for non-torrent results.
    pub download_volume_factor: Option<f64>,
    /// Opaque token accepted when queuing this candidate, or null when unavailable.
    pub candidate_token: Option<String>,
    /// Acquisition scope targeted by this result, or null when no scope was inferred.
    pub queue_scope: Option<QueueDownloadScopePayload>,
    /// Whether automatic acquisition may select this result.
    pub auto_eligible: Option<bool>,
    /// Automatic acquisition decision code, or null when not evaluated.
    pub auto_decision_code: Option<String>,
    /// Human-readable automatic acquisition decision summary, or null when not evaluated.
    pub auto_decision_summary: Option<String>,
}

/// Acquisition scope targeted by a queued download.
#[derive(async_graphql::Union, Clone)]
pub enum QueueDownloadScopePayload {
    /// A single episode target.
    Episode(EpisodeScopePayload),
    /// A set of episode targets.
    EpisodeSet(EpisodeSetScopePayload),
    /// A series-movie link target.
    SeriesMovie(SeriesMovieScopePayload),
    /// A collection target.
    Collection(CollectionScopePayload),
    /// An entire-title target.
    Title(TitleScopePayload),
    /// A queued download with no known acquisition scope.
    Orphan(OrphanScopePayload),
}

impl QueueDownloadScopePayload {
    pub fn episode(episode_id: ID) -> Self {
        Self::Episode(EpisodeScopePayload { episode_id })
    }
}

#[derive(SimpleObject, Clone)]
/// Union member identifying one episode by ID.
pub struct EpisodeScopePayload {
    /// Target episode ID.
    pub episode_id: ID,
}

#[derive(SimpleObject, Clone)]
/// Union member identifying multiple episodes by ID.
pub struct EpisodeSetScopePayload {
    /// Target episode IDs; empty means no episode scope was supplied.
    pub episode_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Union member identifying a series-movie relationship by ID.
pub struct SeriesMovieScopePayload {
    /// Target series-movie link ID.
    pub series_movie_link_id: ID,
}

#[derive(SimpleObject, Clone)]
/// Union member identifying a collection by ID.
pub struct CollectionScopePayload {
    /// Target collection ID.
    pub collection_id: ID,
}

#[derive(SimpleObject, Clone)]
/// Union member indicating the entire title is the acquisition scope.
pub struct TitleScopePayload {
    /// Always true when this marker is emitted.
    pub whole_title: bool,
}

#[derive(SimpleObject, Clone)]
/// Union member indicating that no known acquisition scope is attached.
pub struct OrphanScopePayload {
    /// Always true when this marker is emitted.
    pub orphaned: bool,
}

#[derive(SimpleObject, Clone)]
/// Parsed episode numbering extracted from a release title.
pub struct ParsedEpisodePayload {
    /// Season number, or null when no season was parsed.
    pub season: Option<i32>,
    /// Parsed episode numbers; empty means none were detected.
    pub episode_numbers: Vec<i32>,
}

#[derive(SimpleObject, Clone)]
/// Parsed release title metadata and parser confidence.
pub struct ParsedReleasePayload {
    /// Original release title before normalization.
    pub raw_title: String,
    /// Normalized title used for matching.
    pub normalized_title: String,
    /// Release group, or null when not detected.
    pub release_group: Option<String>,
    /// Quality label, or null when not detected.
    pub quality: Option<String>,
    /// Source label, or null when not detected.
    pub source: Option<String>,
    /// Video codec, or null when not detected.
    pub video_codec: Option<String>,
    /// Video encoding, or null when not detected.
    pub video_encoding: Option<String>,
    /// Audio description, or null when not detected.
    pub audio: Option<String>,
    /// Whether dual audio was detected.
    pub is_dual_audio: bool,
    /// Whether Atmos audio was detected.
    pub is_atmos: bool,
    /// Whether Dolby Vision was detected.
    pub is_dolby_vision: bool,
    /// Whether HDR was detected.
    pub detected_hdr: bool,
    /// Whether the release is marked as a proper upload.
    pub is_proper_upload: bool,
    /// Whether the release is a remux.
    pub is_remux: bool,
    /// Whether the release is a Blu-ray disc image.
    pub is_bd_disk: bool,
    /// Whether an AI-enhanced marker was detected.
    pub is_ai_enhanced: bool,
    /// Parser confidence on the service's 0 through 1 scale.
    pub parse_confidence: f32,
    /// Parser hints retained for diagnostics.
    pub parse_hints: Vec<String>,
    /// Parsed episode numbering, or null for non-episodic releases.
    pub episode: Option<ParsedEpisodePayload>,
}

#[derive(SimpleObject, Clone)]
/// One scoring rule contribution used in a quality decision.
pub struct ScoringEntryPayload {
    /// Stable scoring rule code.
    pub code: String,
    /// Signed score delta contributed by the rule.
    pub delta: i32,
    /// Source of the scoring rule.
    pub source: String,
    /// Rule-set name, or null when not associated with a named set.
    pub rule_set_name: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Quality-profile acceptance and score breakdown for a release.
pub struct QualityProfileDecisionPayload {
    /// Whether the release passed the profile decision.
    pub allowed: bool,
    /// Blocking rule codes that prevented acceptance.
    pub block_codes: Vec<String>,
    /// Aggregate release score.
    pub release_score: i32,
    /// Aggregate preference score.
    pub preference_score: i32,
    /// Ordered scoring contributions used to compute the decision.
    pub scoring_log: Vec<ScoringEntryPayload>,
}

#[derive(SimpleObject, Clone)]
/// Provider configuration field metadata and its masked stored value.
pub struct ProviderConfigValuePayload {
    /// Stable provider configuration key.
    pub key: String,
    /// Human-readable field label, or null when the provider supplies none.
    pub label: Option<String>,
    /// Configuration field type, or null when unspecified.
    pub field_type: Option<PluginConfigFieldTypeValue>,
    /// Whether a value is required by the provider.
    pub required: bool,
    /// Provider default value, or null when no default exists.
    pub default_value: Option<String>,
    /// Source of the effective value, or null when no source is recorded.
    pub value_source: Option<PluginConfigValueSourceValue>,
    /// Semantic role of the field, or null when unspecified.
    pub role: Option<PluginConfigFieldRoleValue>,
    /// Host binding required by the field, or null when not applicable.
    pub host_binding: Option<String>,
    /// Allowed field options; empty means the field has no enumerated options.
    pub options: Vec<PluginConfigFieldOptionPayload>,
    /// Provider help text, or null when unavailable.
    pub help_text: Option<String>,
    /// The stored value as a typed union; null when the field is unset.
    pub value: Option<ProviderConfigFieldValue>,
}

/// Typed provider configuration value; secret variants expose presence but not plaintext.
#[derive(async_graphql::Union, Clone)]
pub enum ProviderConfigFieldValue {
    /// Stored string value.
    String(StringConfigValuePayload),
    /// Stored boolean value.
    Bool(BoolConfigValuePayload),
    /// Stored signed integer value.
    Int(IntConfigValuePayload),
    /// Stored floating-point value.
    Float(FloatConfigValuePayload),
    /// Secret presence marker without the secret contents.
    Secret(SecretConfigValuePayload),
}

#[derive(SimpleObject, Clone)]
/// Stored string provider configuration value.
pub struct StringConfigValuePayload {
    /// String value.
    pub value: String,
}

#[derive(SimpleObject, Clone)]
/// Stored boolean provider configuration value.
pub struct BoolConfigValuePayload {
    /// Boolean value.
    pub value: bool,
}

#[derive(SimpleObject, Clone)]
/// Stored integer provider configuration value.
pub struct IntConfigValuePayload {
    /// Signed integer value.
    pub value: i64,
}

#[derive(SimpleObject, Clone)]
/// Stored floating-point provider configuration value.
pub struct FloatConfigValuePayload {
    /// Floating-point value.
    pub value: f64,
}

#[derive(SimpleObject, Clone)]
/// Secret provider configuration value represented without plaintext.
pub struct SecretConfigValuePayload {
    /// True when a secret is stored; false means absent or cleared.
    pub stored: bool,
}

#[derive(InputObject, Clone)]
/// One provider configuration update, with typed value slots and explicit secret clearing.
pub struct ProviderConfigValueInput {
    /// Provider configuration key being written.
    pub key: String,
    /// String value slot; null means this slot is not selected.
    pub string_value: Option<String>,
    /// Boolean value slot; null means this slot is not selected.
    pub bool_value: Option<bool>,
    /// Signed integer value slot; null means this slot is not selected.
    pub int_value: Option<i64>,
    /// Floating-point value slot; null means this slot is not selected.
    pub float_value: Option<f64>,
    /// New secret value; it is accepted for writing but never returned in payloads.
    pub secret_value: Option<String>,
    /// When true, clear the stored secret instead of returning or preserving it.
    pub clear_secret: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Indexer configuration, health, capability, routing, and masked secret metadata.
pub struct IndexerConfigPayload {
    /// Indexer configuration ID.
    pub id: ID,
    /// User-facing indexer name.
    pub name: String,
    /// Stable provider implementation key for this indexer.
    pub provider_type: String,
    /// Indexer base URL.
    pub base_url: String,
    /// Optional proxy configuration ID used by this indexer.
    pub indexer_proxy_config_id: Option<ID>,
    /// Optional download-client ID associated with this indexer.
    pub download_client_id: Option<ID>,
    /// Whether an API key is configured without exposing it.
    pub has_api_key: bool,
    /// Whether this configuration is managed by a parent configuration.
    pub is_managed: bool,
    /// Parent managed configuration ID, or null when independent.
    pub managed_parent_config_id: Option<ID>,
    /// Whether managed child synchronization is supported.
    pub supports_managed_children_sync: bool,
    /// Names of stored secret fields, never their values.
    pub stored_secret_keys: Vec<String>,
    /// Minimum interval between requests in seconds, or null when unlimited.
    pub rate_limit_seconds: Option<i64>,
    /// Maximum burst size for rate limiting, or null when unspecified.
    pub rate_limit_burst: Option<i64>,
    /// UTC time until which the indexer is disabled, or null when not disabled.
    pub disabled_until: Option<DateTime<Utc>>,
    /// Whether the indexer is enabled.
    pub is_enabled: bool,
    /// Whether interactive searches are enabled.
    pub enable_interactive_search: bool,
    /// Whether automatic searches are enabled.
    pub enable_auto_search: bool,
    /// Most recent health status, or null before the first check.
    pub last_health_status: Option<String>,
    /// Most recent health error, or null when none is recorded.
    pub last_error_message: Option<String>,
    /// UTC time of the most recent health error, or null when none is recorded.
    pub last_error_at: Option<DateTime<Utc>>,
    /// UTC time of the most recent query, or null before the first query.
    pub last_query_at: Option<DateTime<Utc>>,
    /// Provider configuration fields with secret values masked.
    pub config: Vec<ProviderConfigValuePayload>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Available indexers, download clients, and provider compatibility mappings.
pub struct IndexerDownloadClientMappingCatalogPayload {
    /// Download clients available for mapping.
    pub clients: Vec<IndexerDownloadClientMappingClientPayload>,
    /// Indexers and their current mapping state.
    pub indexers: Vec<IndexerDownloadClientMappingIndexerPayload>,
    /// Provider-level compatibility information used when no concrete indexer exists.
    pub provider_compatibility: Vec<IndexerDownloadClientProviderCompatibilityPayload>,
}

#[derive(SimpleObject, Clone)]
/// Download client option used by the indexer mapping catalog.
pub struct IndexerDownloadClientMappingClientPayload {
    /// Download client ID.
    pub id: ID,
    /// Download client name.
    pub name: String,
    /// Download client provider type.
    pub client_type: String,
    /// Whether the client is enabled.
    pub is_enabled: bool,
    /// Current health status string.
    pub health_status: String,
}

#[derive(SimpleObject, Clone)]
/// Indexer mapping state and compatible download-client IDs.
pub struct IndexerDownloadClientMappingIndexerPayload {
    /// Indexer configuration ID.
    pub id: ID,
    /// Indexer name.
    pub name: String,
    /// Currently mapped download-client ID, or null when unmapped.
    pub download_client_id: Option<ID>,
    /// Protocol families supported by the indexer.
    pub protocol_families: Vec<String>,
    /// Whether this indexer supports explicit client mapping.
    pub supports_mapping: bool,
    /// Download-client IDs compatible with the indexer.
    pub compatible_client_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Provider-level protocol compatibility and compatible download clients.
pub struct IndexerDownloadClientProviderCompatibilityPayload {
    /// Stable indexer provider implementation key.
    pub provider_type: String,
    /// Protocol families supported by the provider.
    pub protocol_families: Vec<String>,
    /// Whether the provider supports explicit mapping.
    pub supports_mapping: bool,
    /// Compatible download-client IDs.
    pub compatible_client_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Indexer proxy configuration and latest health state.
pub struct IndexerProxyConfigPayload {
    /// Proxy configuration ID.
    pub id: ID,
    /// Proxy configuration name.
    pub name: String,
    /// Proxy provider type.
    pub provider_type: String,
    /// Proxy protocol.
    pub protocol: String,
    /// Proxy base URL.
    pub base_url: String,
    /// Request timeout in seconds.
    pub request_timeout_seconds: i32,
    /// Whether the proxy is enabled.
    pub is_enabled: bool,
    /// Most recent health status, or null before the first check.
    pub last_health_status: Option<String>,
    /// Most recent health error, or null when none is recorded.
    pub last_error_message: Option<String>,
    /// UTC time of the most recent health error, or null when none is recorded.
    pub last_error_at: Option<DateTime<Utc>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Result of testing an indexer proxy connection.
pub struct IndexerProxyTestResultPayload {
    /// Whether the connection test succeeded.
    pub ok: bool,
    /// Machine-readable test status.
    pub status: String,
    /// Optional diagnostic message.
    pub message: Option<String>,
    /// Test duration in milliseconds, or null when unavailable.
    pub duration_ms: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// IDs created, updated, and deleted while synchronizing managed indexer configurations.
pub struct IndexerConfigSyncPayload {
    /// Parent configuration ID used for synchronization.
    pub parent_config_id: ID,
    /// IDs created by synchronization.
    pub created_ids: Vec<ID>,
    /// IDs updated by synchronization.
    pub updated_ids: Vec<ID>,
    /// IDs deleted by synchronization.
    pub deleted_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Root-folder path and default marker.
pub struct RootFolderPayload {
    /// Filesystem path.
    pub path: String,
    /// Whether this is the default root folder.
    pub is_default: bool,
}

#[derive(SimpleObject, Clone)]
/// Library root-folder identity, path, and default marker.
pub struct LibraryRootPayload {
    /// Root-folder ID.
    pub id: ID,
    /// Filesystem path.
    pub path: String,
    /// Whether this is the default root folder.
    pub is_default: bool,
}

#[derive(SimpleObject, Clone)]
/// Effective library settings together with nullable per-library overrides.
pub struct LibrarySettingsPayload {
    /// Library override for required audio language codes; null means inherit.
    pub required_audio_languages_override: Option<Vec<String>>,
    /// Effective required audio language codes after inheritance.
    pub required_audio_languages: Vec<String>,
    /// Library override quality-profile ID; null means inherit.
    pub quality_profile_id_override: Option<ID>,
    /// Effective quality-profile ID.
    pub quality_profile_id: ID,
    /// Library override request quality-profile IDs; null means inherit.
    pub request_quality_profile_ids_override: Option<Vec<ID>>,
    /// Effective request quality-profile IDs; empty means no additional profiles.
    pub request_quality_profile_ids: Vec<ID>,
    /// Effective default request quality-profile ID.
    pub request_quality_profile_default_id: ID,
    /// Library override scoring persona; null means inherit.
    pub scoring_persona_override: Option<ScoringPersonaValue>,
    /// Effective scoring persona.
    pub scoring_persona: ScoringPersonaValue,
    /// Library override filler policy; null means inherit.
    pub filler_policy_override: Option<FillerPolicyValue>,
    /// Effective filler policy, or null when unset.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Library override recap policy; null means inherit.
    pub recap_policy_override: Option<RecapPolicyValue>,
    /// Effective recap policy, or null when unset.
    pub recap_policy: Option<RecapPolicyValue>,
    /// Library override for monitoring specials; null means inherit.
    pub monitor_specials_override: Option<bool>,
    /// Effective specials monitoring setting, or null when unset.
    pub monitor_specials: Option<bool>,
    /// Library override for inter-season movies; null means inherit.
    pub inter_season_movies_override: Option<bool>,
    /// Effective inter-season movie setting, or null when unset.
    pub inter_season_movies: Option<bool>,
    /// Library override for filler movies; null means inherit.
    pub monitor_filler_movies_override: Option<bool>,
    /// Effective filler-movie monitoring setting, or null when unset.
    pub monitor_filler_movies: Option<bool>,
    /// Library override for NFO writing; null means inherit.
    pub nfo_write_on_import_override: Option<bool>,
    /// Effective NFO-on-import setting.
    pub nfo_write_on_import: bool,
    /// Library override for Plex match writing; null means inherit.
    pub plexmatch_write_on_import_override: Option<bool>,
    /// Effective Plex match-on-import setting, or null when unset.
    pub plexmatch_write_on_import: Option<bool>,
    /// Library override import mode; null means inherit.
    pub import_mode_override: Option<ImportModeValue>,
    /// Effective import mode.
    pub import_mode: ImportModeValue,
    /// Library override for Linux permission updates; null means inherit.
    pub set_permissions_linux_override: Option<bool>,
    /// Effective Linux permission-update setting.
    pub set_permissions_linux: bool,
    /// Library override file chmod mode; null means inherit.
    pub file_chmod_override: Option<String>,
    /// Effective file chmod mode, or null when unset.
    pub file_chmod: Option<String>,
    /// Library override folder chmod mode; null means inherit.
    pub folder_chmod_override: Option<String>,
    /// Effective folder chmod mode, or null when unset.
    pub folder_chmod: Option<String>,
    /// Library override chown group; null means inherit.
    pub chown_group_override: Option<String>,
    /// Effective chown group, or null when unset.
    pub chown_group: Option<String>,
    /// Library override indexer routing entries; null means inherit.
    pub indexer_routing_override: Option<Vec<IndexerRoutingEntryPayload>>,
    /// Library override download-client routing entries; null means inherit.
    pub download_client_routing_override: Option<Vec<DownloadClientRoutingEntryPayload>>,
}

#[derive(SimpleObject, Clone)]
/// Download-client configuration with provider metadata and masked secret fields.
pub struct DownloadClientConfigPayload {
    /// Download-client configuration ID.
    pub id: ID,
    /// Download-client name.
    pub name: String,
    /// Download-client provider type.
    pub client_type: String,
    /// Base URL, or null for clients without a URL.
    pub base_url: Option<String>,
    /// Provider configuration fields with secrets masked.
    pub config: Vec<ProviderConfigValuePayload>,
    /// Names of stored secret fields, never their values.
    pub stored_secret_keys: Vec<String>,
    /// Whether the client is enabled.
    pub is_enabled: bool,
    /// Current client status.
    pub status: String,
    /// Most recent error, or null when none is recorded.
    pub last_error: Option<String>,
    /// UTC time the client was last observed, or null before the first observation.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Subtitle-provider configuration and health state without exposing secret values.
pub struct SubtitleProviderConfigPayload {
    /// Subtitle-provider configuration ID.
    pub id: ID,
    /// Provider configuration name.
    pub name: String,
    /// Subtitle-provider type identifier.
    pub provider_type: String,
    /// Whether provider configuration exists.
    pub has_config: bool,
    /// Names of stored secret fields, never their values.
    pub stored_secret_keys: Vec<String>,
    /// Media facets enabled for this provider.
    pub enabled_facets: Vec<MediaFacetValue>,
    /// Whether the provider is enabled.
    pub is_enabled: bool,
    /// Most recent health status, or null before the first check.
    pub last_health_status: Option<String>,
    /// Most recent error, or null when none is recorded.
    pub last_error: Option<String>,
    /// UTC time of the most recent error, or null when none is recorded.
    pub last_error_at: Option<DateTime<Utc>>,
    /// UTC time until which the provider is disabled, or null when not disabled.
    pub disabled_until: Option<DateTime<Utc>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Download-client option used by paged queue and history filters.
pub struct DownloadClientFilterOptionPayload {
    /// Download-client ID.
    pub client_id: ID,
    /// Download-client name.
    pub client_name: String,
    /// Download-client provider type.
    pub client_type: String,
}

#[derive(SimpleObject, Clone)]
/// Result of one import operation, including decision and source/destination paths.
pub struct ImportResultPayload {
    /// Import record ID.
    pub import_id: ID,
    /// Final import decision.
    pub decision: ImportDecisionValue,
    /// Skip reason when the decision skipped the import, or null otherwise.
    pub skip_reason: Option<ImportSkipReasonValue>,
    /// Imported title ID, or null when no title was bound.
    pub title_id: Option<ID>,
    /// Source path examined by the import.
    pub source_path: String,
    /// Destination path written by the import, or null when no destination was written.
    pub dest_path: Option<String>,
    /// Error message, or null when the operation completed without an error.
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Persisted import record with lifecycle status, decision, IDs, and UTC timestamps.
pub struct ImportRecordPayload {
    /// Import record ID.
    pub id: ID,
    /// External system that supplied the import.
    pub source_system: String,
    /// Source-system reference identifying the download or item.
    pub source_ref: String,
    /// Source title, or null when unavailable.
    pub source_title: Option<String>,
    /// Media facet, or null when not yet classified.
    pub facet: Option<MediaFacetValue>,
    /// Import operation type.
    pub import_type: ImportTypeValue,
    /// Current import lifecycle status.
    pub status: ImportStatusValue,
    /// Error message, or null when no error is recorded.
    pub error_message: Option<String>,
    /// Import decision, or null before a decision is made.
    pub decision: Option<ImportDecisionValue>,
    /// Skip reason, or null when not skipped.
    pub skip_reason: Option<ImportSkipReasonValue>,
    /// Bound title ID, or null when no title was selected.
    pub title_id: Option<ID>,
    /// Source path, or null when not recorded.
    pub source_path: Option<String>,
    /// Destination path, or null when no destination was produced.
    pub dest_path: Option<String>,
    /// UTC start time, or null before processing begins.
    pub started_at: Option<DateTime<Utc>>,
    /// UTC completion time, or null while incomplete.
    pub finished_at: Option<DateTime<Utc>>,
    /// UTC record creation time.
    pub created_at: DateTime<Utc>,
}

#[derive(InputObject)]
/// Requests retrying one import record, optionally supplying an archive password.
pub struct RetryImportInput {
    /// Import record ID to retry.
    pub import_id: ID,
    /// Optional password for an encrypted source archive.
    pub password: Option<String>,
}

#[derive(InputObject)]
/// Identifies a tracked download to ignore without deleting it from the download client.
pub struct IgnoreTrackedDownloadInput {
    /// Download-client ID, or null when the provider identity is sufficient.
    pub client_id: Option<ID>,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-issued download item ID.
    pub download_client_item_id: String,
}

#[derive(InputObject)]
/// Marks a tracked download failed and optionally prevents reacquisition.
pub struct MarkTrackedDownloadFailedInput {
    /// Download-client ID, or null when the provider identity is sufficient.
    pub client_id: Option<ID>,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-issued download item ID.
    pub download_client_item_id: String,
    /// When true, suppress reacquisition after marking failure.
    pub skip_reacquire: Option<bool>,
}

#[derive(OneofObject, Clone)]
/// Union input selecting the acquisition scope of a tracked download.
pub enum QueueDownloadScopeInput {
    /// One target episode ID.
    Episode(ID),
    /// Multiple target episode IDs.
    EpisodeSet(Vec<ID>),
    /// One series-movie link ID.
    SeriesMovie(ID),
    /// One collection ID.
    Collection(ID),
    /// Whole-title marker; the boolean must indicate the title scope.
    Title(bool),
}

#[derive(InputObject)]
/// Assigns a tracked download to a title and an explicit acquisition scope.
pub struct AssignTrackedDownloadTitleInput {
    /// Download-client ID, or null when the provider identity is sufficient.
    pub client_id: Option<ID>,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-issued download item ID.
    pub download_client_item_id: String,
    /// Target title ID.
    pub title_id: ID,
    /// Target scope within the title.
    pub scope: QueueDownloadScopeInput,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Lifecycle of title hydration while a tracked download is being associated.
pub enum AddTitleHydrationStateValue {
    /// Hydration has been requested but is not complete.
    Pending,
    /// Hydration completed successfully.
    Complete,
    /// No hydration was needed.
    NotRequired,
}

#[derive(InputObject)]
/// Requests scanning one library and optionally supplies an import warmup session.
pub struct ScanLibraryInput {
    /// Target library ID.
    pub library_id: ID,
    /// Optional import warmup session ID to use during the scan.
    pub import_warmup_session_id: Option<ID>,
}

#[derive(SimpleObject, Clone)]
/// Counts produced by a library scan.
pub struct LibraryScanSummaryPayload {
    /// Number of files or records scanned.
    pub scanned: i32,
    /// Number matched to known titles.
    pub matched: i32,
    /// Number imported successfully.
    pub imported: i32,
    /// Number skipped by scan or import policy.
    pub skipped: i32,
    /// Number left unmatched.
    pub unmatched: i32,
}

#[derive(SimpleObject, Clone)]
/// Pending-import counts grouped by media facet.
pub struct PendingImportCountsPayload {
    /// Pending movie count.
    pub movie: i32,
    /// Pending series count.
    pub series: i32,
    /// Pending anime count.
    pub anime: i32,
}

#[derive(SimpleObject, Clone)]
/// Title-history event counts for one dashboard activity window.
pub struct ActivityWindowCountsPayload {
    /// Releases grabbed during the window.
    pub grabbed: i32,
    /// Existing files replaced by a better release during the window.
    pub upgraded: i32,
    /// Imports that completed during the window.
    pub imported: i32,
    /// Imports rejected as failed during the window; skipped imports are excluded.
    pub import_failed: i32,
    /// Downloads that failed during the window.
    pub download_failed: i32,
}

#[derive(SimpleObject, Clone)]
/// A trailing activity window together with the window immediately before it,
/// so callers can render each count with its period-over-period delta.
pub struct DashboardActivityStatsPayload {
    /// Counts for the trailing window ending at the time of the request.
    pub current: ActivityWindowCountsPayload,
    /// Counts for the equally long window immediately before the current one.
    pub previous: ActivityWindowCountsPayload,
}

#[derive(SimpleObject, Clone)]
/// Filesystem usage of the volume backing one library root folder.
pub struct StorageRootUsagePayload {
    /// Configured root-folder path as stored on the library.
    pub path: String,
    /// ID of the library that owns this root folder.
    pub library_id: ID,
    /// Name of the library that owns this root folder.
    pub library_name: String,
    /// Media facet of the library that owns this root folder.
    pub facet: MediaFacetValue,
    /// Bytes in use on the backing filesystem; null when it cannot be inspected.
    pub used_bytes: Option<Long>,
    /// Total bytes on the backing filesystem; null when it cannot be inspected.
    pub total_bytes: Option<Long>,
}

#[derive(SimpleObject, Clone)]
/// Media-request counts grouped by media facet.
pub struct MediaRequestCountsPayload {
    /// Movie request count.
    pub movie: i32,
    /// Series request count.
    pub series: i32,
    /// Anime request count.
    pub anime: i32,
}

#[derive(SimpleObject, Clone)]
/// Authorization-filtered counts used for application navigation indicators.
pub struct NavigationBadgeCountsPayload {
    /// Pending imports visible to the caller.
    pub pending_import_counts: PendingImportCountsPayload,
    /// Pending media requests visible to the caller.
    pub pending_media_request_counts: MediaRequestCountsPayload,
    /// Count of import activity visible to the caller.
    pub activity_import_count: i32,
    /// Count of available plugin updates visible to the caller.
    pub plugin_update_count: i32,
}

#[derive(SimpleObject, Clone)]
/// One metadata search attempt made while resolving a pending import.
pub struct PendingImportSearchAttemptPayload {
    /// Search query submitted.
    pub query: String,
    /// Number of metadata results returned.
    pub result_count: i32,
    /// Top result titles retained for diagnostics.
    pub top_results: Vec<String>,
    /// Human-readable attempt summary.
    pub summary: String,
}

/// Coarse bucket for why a pending import is awaiting resolution.
///
/// The free-text `reason` field remains the authoritative scanner code; this
/// enum is the stable grouping the dashboard filters on, so a new scanner code
/// surfaces as `OTHER` instead of breaking clients.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum PendingImportReasonClassValue {
    /// Metadata lookup returned no candidates to choose from.
    Unmatched,
    /// Metadata lookup returned candidates but none could be accepted automatically.
    Ambiguous,
    /// The file's media metadata could not be read, so its quality is unknown.
    QualityUnknown,
    /// Any other scanner reason, including parse and folder-ownership problems.
    Other,
}

#[derive(SimpleObject, Clone)]
/// One unmatched library item awaiting import resolution.
pub struct PendingImportItemPayload {
    /// Pending-import item ID.
    pub id: ID,
    /// Library ID containing the item.
    pub library_id: ID,
    /// Media facet inferred for the item.
    pub facet: MediaFacetValue,
    /// Current pending-import status.
    pub status: PendingImportStatusValue,
    /// Bound title ID, or null before resolution.
    pub title_id: Option<ID>,
    /// Bound title name, or null before resolution.
    pub title_name: Option<String>,
    /// Bound title slug, or null before resolution.
    pub title_slug: Option<String>,
    /// Display name derived from the unmatched item.
    pub display_name: String,
    /// Full source path.
    pub path: String,
    /// Containing folder path, or null when not applicable.
    pub folder_path: Option<String>,
    /// Metadata query suggested for resolution.
    pub query: String,
    /// Parsed year hint, or null when unavailable.
    pub year_hint: Option<i32>,
    /// Explanation for the pending state.
    pub reason: String,
    /// Coarse bucket for `reason`, stable across scanner reason-code changes.
    pub reason_class: PendingImportReasonClassValue,
    /// Metadata search attempts made for this item.
    pub search_attempts: Vec<PendingImportSearchAttemptPayload>,
    /// Size of the pending file; null for folder items and unreadable files.
    pub size_bytes: Option<Long>,
    /// When the scanner first recorded this item.
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Offset-paginated pending-import connection.
pub struct PendingImportConnectionPayload {
    /// Items in the requested page; empty means no items matched.
    pub items: Vec<PendingImportItemPayload>,
    /// Total matching items before pagination.
    pub total_count: i32,
    /// Whether more matching items exist after this page.
    pub has_more: bool,
}

#[derive(InputObject)]
/// Resolves one pending import to a title and requested title metadata.
pub struct ResolvePendingImportInput {
    /// Pending-import item ID.
    pub pending_import_id: ID,
    /// Title metadata to associate with the item.
    pub title: AddTitleInput,
}

#[derive(SimpleObject, Clone)]
/// Parsed file details and suggested episode bindings for a pending import.
pub struct PendingImportBindingFilePreviewPayload {
    /// Full file path.
    pub file_path: String,
    /// File name component.
    pub file_name: String,
    /// File size in bytes.
    pub size_bytes: Long,
    /// Parsed season number, or null when unavailable.
    pub parsed_season: Option<i32>,
    /// Parsed episode numbers; empty means none were detected.
    pub parsed_episodes: Vec<i32>,
    /// Parsed absolute episode numbers; empty means none were detected.
    pub parsed_absolute_numbers: Vec<i32>,
    /// Suggested episode IDs; empty means no suggestions were found.
    pub suggested_episode_ids: Vec<ID>,
}

#[derive(InputObject)]
/// Binds one pending import to an optional collection and episode IDs.
pub struct BindPendingImportInput {
    /// Pending-import item ID.
    pub pending_import_id: ID,
    /// Collection ID, or null when binding episodes directly.
    pub collection_id: Option<ID>,
    /// Episode IDs to bind; empty means no explicit episode list.
    pub episode_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Result of ignoring one pending import.
pub struct IgnorePendingImportPayload {
    /// Pending-import item ID.
    pub id: ID,
    /// Status after the ignore operation.
    pub status: PendingImportStatusValue,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting cancellation for an acquisition search.
pub struct CancelAcquisitionSearchPayload {
    /// Acquisition-search job ID.
    pub id: ID,
    /// True when cancellation was accepted; false when the search was already terminal.
    pub accepted: bool,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting cancellation for a library scan.
pub struct CancelLibraryScanPayload {
    /// Library-scan session ID.
    pub session_id: ID,
    /// True when cancellation was accepted; false when the scan was already terminal.
    pub accepted: bool,
}

#[derive(SimpleObject, Clone)]
/// Read-only media deletion preview with confirmation and sample-path details.
pub struct DeletePreviewPayload {
    /// Stable preview fingerprint required to apply the same plan.
    pub fingerprint: String,
    /// Total files selected by the preview.
    pub total_file_count: i32,
    /// Selected media-file count.
    pub media_count: i32,
    /// Selected subtitle-file count.
    pub subtitle_count: i32,
    /// Selected image-file count.
    pub image_count: i32,
    /// Selected other-file count.
    pub other_count: i32,
    /// Selected directory count.
    pub directory_count: i32,
    /// Whether typed confirmation is required before applying the plan.
    pub requires_typed_confirmation: bool,
    /// Required confirmation text, or null when typed confirmation is not required.
    pub typed_confirmation_prompt: Option<String>,
    /// Human-readable preview target label.
    pub target_label: String,
    /// Sample paths from the selected files and directories.
    pub sample_paths: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// Per-title result within a multi-title deletion preview.
pub struct DeleteTitlePreviewResultPayload {
    /// Target title ID.
    pub title_id: ID,
    /// Deletion preview, or null when preview generation failed.
    pub preview: Option<DeletePreviewPayload>,
    /// Error message, or null when preview generation succeeded.
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Combined deletion preview for multiple titles.
pub struct DeleteTitlesPreviewPayload {
    /// Aggregate preview across all successful title targets.
    pub preview: DeletePreviewPayload,
    /// Per-title results.
    pub items: Vec<DeleteTitlePreviewResultPayload>,
    /// Number of title targets that failed preview generation.
    pub failed_count: i32,
}

#[derive(SimpleObject, Clone)]
/// Accepted title IDs and background job information for a deletion request.
pub struct DeleteTitlesPayload {
    /// Background job run tracking the deletion work.
    pub job_run: JobRunPayload,
    /// Title IDs accepted for processing.
    pub accepted_title_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// One media rename plan item with current and proposed paths and decision details.
pub struct MediaRenamePlanItemPayload {
    /// Collection ID, or null when not part of a collection.
    pub collection_id: Option<ID>,
    /// Series-movie link IDs associated with the item.
    pub series_movie_link_ids: Vec<ID>,
    /// Current filesystem path.
    pub current_path: String,
    /// Proposed filesystem path, or null when no rename is possible.
    pub proposed_path: Option<String>,
    /// Normalized filename, or null when metadata is insufficient.
    pub normalized_filename: Option<String>,
    /// Whether the proposed path collides with another path.
    pub collision: bool,
    /// Machine-readable reason for the plan decision.
    pub reason_code: String,
    /// Planned write action.
    pub write_action: String,
    /// Source file size in bytes, or null when unavailable.
    pub source_size_bytes: Option<Long>,
    /// Source modification time as Unix milliseconds, or null when unavailable.
    pub source_mtime_unix_ms: Option<Long>,
}

#[derive(SimpleObject, Clone)]
/// Read-only media rename plan and stable fingerprint for later application.
pub struct MediaRenamePlanPayload {
    /// Media facet covered by the plan.
    pub facet: MediaFacetValue,
    /// Target title ID, or null when the plan covers the whole facet.
    pub title_id: Option<ID>,
    /// Rename template used to generate proposed paths.
    pub template: String,
    /// Collision policy used by the plan.
    pub collision_policy: RenameCollisionPolicyValue,
    /// Missing-metadata policy used by the plan.
    pub missing_metadata_policy: RenameMissingMetadataPolicyValue,
    /// Stable fingerprint required to apply this exact plan.
    pub fingerprint: String,
    /// Total plan-item count.
    pub total: i32,
    /// Number of items eligible for rename.
    pub renamable: i32,
    /// Number of items already matching the desired path.
    pub noop: i32,
    /// Number of collision items.
    pub conflicts: i32,
    /// Number of items with planning errors.
    pub errors: i32,
    /// Plan items in deterministic service order.
    pub items: Vec<MediaRenamePlanItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Result for one applied media rename plan item.
pub struct MediaRenameApplyItemPayload {
    /// Collection ID, or null when not part of a collection.
    pub collection_id: Option<ID>,
    /// Series-movie link IDs associated with the item.
    pub series_movie_link_ids: Vec<ID>,
    /// Path before the apply operation.
    pub current_path: String,
    /// Planned path, or null when no rename was proposed.
    pub proposed_path: Option<String>,
    /// Final path after application, or null when not moved.
    pub final_path: Option<String>,
    /// Write action attempted.
    pub write_action: String,
    /// Apply status.
    pub status: String,
    /// Machine-readable reason for the outcome.
    pub reason_code: String,
    /// Error message, or null when the item succeeded or was skipped without error.
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Aggregate result of applying a previously generated rename plan.
pub struct MediaRenameApplyPayload {
    /// Fingerprint of the plan that was applied.
    pub plan_fingerprint: String,
    /// Total plan-item count.
    pub total: i32,
    /// Number of items applied.
    pub applied: i32,
    /// Number of items skipped.
    pub skipped: i32,
    /// Number of items that failed.
    pub failed: i32,
    /// Per-item apply results.
    pub items: Vec<MediaRenameApplyItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Subtitle language preference with hearing-impaired and forced flags.
pub struct SubtitleLanguagePreferencePayload {
    /// Language code.
    pub code: String,
    /// Whether hearing-impaired subtitles are preferred.
    pub hearing_impaired: bool,
    /// Whether forced subtitles are preferred.
    pub forced: bool,
}

#[derive(SimpleObject, Clone)]
/// Effective subtitle search, download, and synchronization settings.
pub struct SubtitleSettingsPayload {
    /// Whether subtitle processing is enabled.
    pub enabled: bool,
    /// Ordered subtitle language preferences.
    pub languages: Vec<SubtitleLanguagePreferencePayload>,
    /// Whether subtitles are downloaded automatically after import.
    pub auto_download_on_import: bool,
    /// Minimum subtitle score for series, on the provider score scale.
    pub minimum_score_series: i32,
    /// Minimum subtitle score for movies, on the provider score scale.
    pub minimum_score_movie: i32,
    /// Search interval in hours.
    pub search_interval_hours: i32,
    /// Whether AI-translated subtitles are eligible.
    pub include_ai_translated: bool,
    /// Whether machine-translated subtitles are eligible.
    pub include_machine_translated: bool,
    /// Whether subtitle synchronization is enabled.
    pub sync_enabled: bool,
    /// Synchronization threshold for series, on the provider score scale.
    pub sync_threshold_series: i32,
    /// Synchronization threshold for movies, on the provider score scale.
    pub sync_threshold_movie: i32,
    /// Maximum subtitle offset correction in seconds.
    pub sync_max_offset_seconds: i32,
}

#[derive(SimpleObject, Clone)]
/// Recycle-bin enablement setting.
pub struct RecycleBinSettingsPayload {
    /// Whether deleted media is moved to the recycle bin.
    pub enabled: bool,
}

#[derive(SimpleObject, Clone)]
/// Automatic official-plugin patch update setting.
pub struct PluginAutoUpdateSettingsPayload {
    /// Whether the scheduled plugin catalog refresh installs official patch updates automatically.
    pub enabled: bool,
}

#[derive(SimpleObject, Clone)]
/// Acquisition worker enablement and polling or convergence limits.
pub struct AcquisitionSettingsPayload {
    /// Whether automatic acquisition is enabled.
    pub enabled: bool,
    /// Upgrade cooldown in hours.
    pub upgrade_cooldown_hours: i32,
    /// Minimum score delta for same-tier upgrades.
    pub same_tier_min_delta: i32,
    /// Minimum score delta for cross-tier upgrades.
    pub cross_tier_min_delta: i32,
    /// Score delta that bypasses normal forced-upgrade thresholds.
    pub forced_upgrade_delta_bypass: i32,
    /// Acquisition polling interval in seconds.
    pub poll_interval_seconds: i32,
    /// Maximum long-tail scopes processed per cycle.
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    /// Number of days before long-tail scopes are reconverged.
    pub long_tail_reconverge_days: i32,
}

#[derive(SimpleObject, Clone)]
/// Trusted plugin HTTP certificate identified by SHA-256 fingerprint.
pub struct PluginHttpTrustedCertificatePayload {
    /// Lower-level certificate SHA-256 fingerprint.
    pub fingerprint_sha256: String,
    /// PEM-encoded certificate body.
    pub pem: String,
}

#[derive(SimpleObject, Clone)]
/// General service settings, including effective image-cache limits and trusted certificate data.
pub struct GeneralSettingsPayload {
    /// Whether import history is retained indefinitely.
    pub keep_history_forever: bool,
    /// History retention period in days when indefinite retention is false.
    pub history_retention_days: i32,
    /// Configured image-cache maximum in megabytes.
    pub image_cache_max_size_mb: i32,
    /// Effective image-cache maximum in bytes after environment overrides.
    pub effective_image_cache_max_size_bytes: Long,
    /// Effective image-cache maximum in megabytes after environment overrides.
    pub effective_image_cache_max_size_mb: f64,
    /// Whether an environment variable overrides the configured image-cache limit.
    pub image_cache_max_size_env_override_active: bool,
    /// PEM CA bundle used for plugin HTTP requests.
    pub plugin_http_ca_bundle_pem: String,
    /// Additional trusted plugin HTTP certificates.
    pub plugin_http_trusted_certificates: Vec<PluginHttpTrustedCertificatePayload>,
}

#[derive(SimpleObject, Clone)]
/// Automatic backup schedule and encryption-key readiness.
pub struct AutoBackupSettingsPayload {
    /// Whether automatic backups are enabled.
    pub enabled: bool,
    /// Daily local-time schedule in the service's configured time format.
    pub daily_time_local: String,
    /// Whether the automatic-backup encryption key is present.
    pub auto_backup_key_present: bool,
    /// Whether automatic backup is disabled because the key is missing.
    pub auto_backup_disabled_missing_key_notice: bool,
    /// Next scheduled run in UTC, or null when no run is scheduled.
    pub next_run_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Configured, default, and effective backup paths.
pub struct BackupSettingsPayload {
    /// Custom backup path, or null when the default path is used.
    pub custom_backup_path: Option<String>,
    /// Service default backup path.
    pub default_backup_path: String,
    /// Effective path selected after applying the custom override.
    pub effective_backup_path: String,
}

#[derive(SimpleObject, Clone)]
/// Security settings and effective environment overrides.
pub struct SecuritySettingsPayload {
    /// Whether form-based login is enabled by configuration.
    pub form_login_enabled: bool,
    /// Minimum accepted password length.
    pub password_min_length: i32,
    /// Whether local IPs may skip login.
    pub skip_login_for_local_ips: bool,
    /// Whether configuration changes require MFA step-up.
    pub mfa_require_config_step_up: bool,
    /// Whether password login is required alongside MFA.
    pub mfa_require_password_login: bool,
    /// Whether Jellyfin login requires TOTP.
    pub totp_require_jellyfin_login: bool,
    /// Whether Emby login requires TOTP.
    pub totp_require_emby_login: bool,
    /// Effective form-login state after environment overrides.
    pub effective_form_login_enabled: bool,
    /// Whether an environment override is active.
    pub env_override_active: bool,
    /// Description of the active override, or null when none is active.
    pub env_override_description: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Theme preference for the caller's settings.
pub enum UiThemeValue {
    /// Light theme.
    Light,
    /// Dark theme.
    Dark,
    /// Pride theme.
    Pride,
    /// Follow the system theme.
    System,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Date and time display format preference.
pub enum UiDateTimeFormatValue {
    /// Use locale-specific formatting.
    Locale,
    #[graphql(name = "ISO24H")]
    /// Use ISO date and 24-hour time formatting.
    Iso24h,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Density preference for settings and data presentation.
pub enum UiDensityValue {
    /// Compact density.
    Compact,
    /// Comfortable density.
    Comfortable,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Sidebar expansion preference.
pub enum UiSidebarModeValue {
    /// Keep the sidebar collapsed.
    Collapsed,
    /// Keep the sidebar expanded.
    Expanded,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Default landing content area.
pub enum UiDefaultLandingViewValue {
    /// Movies view.
    Movies,
    /// Series view.
    Series,
    /// Anime view.
    Anime,
    /// Activity view.
    Activity,
    /// Calendar view.
    Calendar,
    /// Wanted view.
    Wanted,
    /// History view.
    History,
    /// Settings view.
    Settings,
    /// System view.
    System,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Media facet used by table-column settings.
pub enum UiSettingsFacetValue {
    /// Movies facet.
    Movies,
    /// Series facet.
    Series,
    /// Anime facet.
    Anime,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Table presentation mode.
pub enum UiTableViewModeValue {
    /// Compact table mode.
    Compact,
    /// Poster table mode.
    PosterTable,
}

#[derive(SimpleObject, Clone)]
/// One persisted table-column preference for a facet and table mode.
pub struct UiTableColumnSettingPayload {
    /// Media facet to which the column setting applies.
    pub facet: UiSettingsFacetValue,
    /// Table mode to which the column setting applies.
    pub table_view_mode: UiTableViewModeValue,
    /// Stable column identifier.
    pub column_id: String,
    /// Zero-based display order.
    pub column_order: i32,
    /// Whether the column is visible.
    pub visible: bool,
}

#[derive(SimpleObject, Clone)]
/// Persisted caller presentation preferences and table-column settings.
pub struct UiSettingsPayload {
    /// Selected theme.
    pub theme: UiThemeValue,
    /// Selected date and time format.
    pub date_time_format: UiDateTimeFormatValue,
    /// Primary highlight color, or null when using the default.
    pub highlight_color: Option<String>,
    /// Secondary color, or null when using the default.
    pub secondary_color: Option<String>,
    /// Whether high-contrast presentation is enabled.
    pub high_contrast_mode: bool,
    /// Whether motion-reduction behavior is enabled.
    pub reduce_motion: bool,
    /// Whether the sponsor button is hidden.
    pub hide_sponsor_button: bool,
    /// Selected density.
    pub density: UiDensityValue,
    /// Selected sidebar mode.
    pub sidebar_mode: UiSidebarModeValue,
    /// Content area opened by default.
    pub default_landing_view: UiDefaultLandingViewValue,
    /// Persisted table-column settings.
    pub table_columns: Vec<UiTableColumnSettingPayload>,
}

#[derive(InputObject, Clone)]
/// Input form for one table-column preference.
pub struct UiTableColumnSettingInput {
    /// Media facet to which the setting applies.
    pub facet: UiSettingsFacetValue,
    /// Table mode to which the setting applies.
    pub table_view_mode: UiTableViewModeValue,
    /// Stable column identifier.
    pub column_id: String,
    /// Zero-based display order.
    pub column_order: i32,
    /// Whether the column should be visible.
    pub visible: bool,
}

#[derive(InputObject, Clone)]
/// Complete caller presentation-preferences update.
pub struct SetMyUiSettingsInput {
    /// Selected theme.
    pub theme: UiThemeValue,
    /// Optional date and time format; null preserves the existing value.
    pub date_time_format: Option<UiDateTimeFormatValue>,
    /// Optional primary highlight color; null preserves the existing value.
    pub highlight_color: Option<String>,
    /// Optional secondary color; null preserves the existing value.
    pub secondary_color: Option<String>,
    /// Whether high-contrast presentation should be enabled.
    pub high_contrast_mode: bool,
    /// Whether motion-reduction behavior should be enabled.
    pub reduce_motion: bool,
    /// Whether the sponsor button should be hidden.
    pub hide_sponsor_button: bool,
    /// Selected density.
    pub density: UiDensityValue,
    /// Selected sidebar mode.
    pub sidebar_mode: UiSidebarModeValue,
    /// Content area to open by default.
    pub default_landing_view: UiDefaultLandingViewValue,
    /// Complete table-column setting list; empty clears all saved column settings.
    pub table_columns: Vec<UiTableColumnSettingInput>,
}

#[derive(SimpleObject, Clone)]
/// Effective authentication runtime flags after configuration and environment overrides.
pub struct AuthRuntimeStatePayload {
    /// Effective form-login state.
    pub effective_form_login_enabled: bool,
    /// Whether local IPs may skip login.
    pub skip_login_for_local_ips: bool,
    /// Whether passkey authentication is enabled.
    pub passkey_enabled: bool,
    /// Whether this request's network provenance defaults sessions to persistent storage.
    pub default_persist_session: bool,
    /// Whether an environment override is active.
    pub env_override_active: bool,
    /// Whether password login is required alongside MFA.
    pub mfa_require_password_login: bool,
    /// Whether configuration changes require MFA step-up.
    pub mfa_require_config_step_up: bool,
    /// Whether Jellyfin login requires TOTP.
    pub totp_require_jellyfin_login: bool,
    /// Whether Emby login requires TOTP.
    pub totp_require_emby_login: bool,
}

#[derive(SimpleObject, Clone)]
/// Delay profile controlling protocol timing and acquisition bypass rules.
pub struct DelayProfilePayload {
    /// Delay-profile ID.
    pub id: ID,
    /// Delay-profile name.
    pub name: String,
    /// Usenet delay in minutes.
    pub usenet_delay_minutes: i32,
    /// Torrent delay in minutes.
    pub torrent_delay_minutes: i32,
    /// Preferred protocol after delay eligibility.
    pub preferred_protocol: DelayProfilePreferredProtocolValue,
    /// Minimum release age in minutes.
    pub min_age_minutes: i32,
    /// Score threshold that bypasses delay, or null when disabled.
    pub bypass_score_threshold: Option<i32>,
    /// Media facets to which the profile applies.
    pub applies_to_facets: Vec<MediaFacetValue>,
    /// Tags used to select this profile.
    pub tags: Vec<String>,
    /// Lower values run with higher priority according to service ordering.
    pub priority: i32,
    /// Whether the profile is enabled.
    pub enabled: bool,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after requesting delay-profile deletion.
pub struct DelayProfileDeletionPayload {
    /// Deleted delay-profile ID.
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
/// Optional boolean scoring overrides for quality evaluation.
pub struct ScoringOverridesPayload {
    /// Whether non-4K x265 releases are allowed.
    pub allow_x265_non4k: Option<bool>,
    /// Whether Dolby Vision without a fallback is blocked.
    pub block_dv_without_fallback: Option<bool>,
    /// Whether compact encodes are preferred.
    pub prefer_compact_encodes: Option<bool>,
    /// Whether lossless audio is preferred.
    pub prefer_lossless_audio: Option<bool>,
    /// Whether upscaled releases are blocked.
    pub block_upscaled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Quality acceptance criteria and scoring controls.
pub struct QualityProfileCriteriaPayload {
    /// Ordered quality tiers accepted by the profile.
    pub quality_tiers: Vec<String>,
    /// Archival quality label, or null when not configured.
    pub archival_quality: Option<String>,
    /// Whether unknown quality values are accepted.
    pub allow_unknown_quality: bool,
    /// Allowed source labels.
    pub source_allowlist: Vec<String>,
    /// Blocked source labels.
    pub source_blocklist: Vec<String>,
    /// Allowed video codec labels.
    pub video_codec_allowlist: Vec<String>,
    /// Blocked video codec labels.
    pub video_codec_blocklist: Vec<String>,
    /// Allowed audio codec labels.
    pub audio_codec_allowlist: Vec<String>,
    /// Blocked audio codec labels.
    pub audio_codec_blocklist: Vec<String>,
    /// Whether Dolby Vision is accepted.
    pub dolby_vision_allowed: bool,
    /// Whether detected HDR is accepted.
    pub detected_hdr_allowed: bool,
    /// Whether remux releases are preferred.
    pub prefer_remux: bool,
    /// Whether Blu-ray disc releases are accepted.
    pub allow_bd_disk: bool,
    /// Whether quality upgrades are allowed.
    pub allow_upgrades: bool,
    /// Optional boolean scoring overrides.
    pub scoring_overrides: ScoringOverridesPayload,
    /// Cutoff quality tier, or null when no cutoff is configured.
    pub cutoff_tier: Option<String>,
    /// Minimum score required to grab, or null when no threshold is configured.
    pub min_score_to_grab: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Named quality profile and its acceptance criteria.
pub struct QualityProfilePayload {
    /// Quality-profile ID.
    pub id: ID,
    /// Quality-profile name.
    pub name: String,
    /// Acceptance criteria and scoring controls.
    pub criteria: QualityProfileCriteriaPayload,
}

#[derive(SimpleObject, Clone)]
/// Effective quality-profile selection for one content scope.
pub struct QualityProfileSelectionPayload {
    /// Content scope to which the selection applies.
    pub scope: ContentScopeValue,
    /// Scope-specific profile override ID, or null when inherited.
    pub override_profile_id: Option<ID>,
    /// Quality-profile ID selected after applying inheritance and overrides.
    pub effective_profile_id: ID,
    /// Whether the effective profile comes from the global setting.
    pub inherits_global: bool,
}

#[derive(SimpleObject, Clone)]
/// Effective scoring-persona selection for one content scope.
pub struct FacetScoringPersonaSelectionPayload {
    /// Content scope to which the selection applies.
    pub scope: ContentScopeValue,
    /// Scope-specific persona override, or null when inherited.
    pub override_persona: Option<ScoringPersonaValue>,
    /// Effective scoring persona.
    pub effective_persona: ScoringPersonaValue,
    /// Whether the effective persona comes from the global setting.
    pub inherits_global: bool,
}

#[derive(SimpleObject, Clone)]
/// Quality profiles, global defaults, and per-content-scope selections.
pub struct QualityProfileSettingsPayload {
    /// Available quality profiles.
    pub profiles: Vec<QualityProfilePayload>,
    /// Global quality-profile ID.
    pub global_profile_id: ID,
    /// Scoring persona inherited by content scopes without an override.
    pub global_scoring_persona: ScoringPersonaValue,
    /// Effective profile selection by content scope.
    pub category_selections: Vec<QualityProfileSelectionPayload>,
    /// Effective scoring-persona selection by content scope.
    pub category_persona_selections: Vec<FacetScoringPersonaSelectionPayload>,
}

#[derive(SimpleObject, Clone)]
/// Download-client routing behavior for one client.
pub struct DownloadClientRoutingEntryPayload {
    /// Download-client ID.
    pub client_id: ID,
    /// Whether this route is enabled.
    pub enabled: bool,
    /// Optional category assigned to recent downloads.
    pub category: Option<String>,
    /// Priority for recent queue items, or null when unspecified.
    pub recent_queue_priority: Option<String>,
    /// Priority for older queue items, or null when unspecified.
    pub older_queue_priority: Option<String>,
    /// Whether completed items are removed from the client.
    pub remove_completed: bool,
    /// Whether failed items are removed from the client.
    pub remove_failed: bool,
}

#[derive(SimpleObject, Clone)]
/// Indexer routing behavior for one indexer.
pub struct IndexerRoutingEntryPayload {
    /// Indexer configuration ID.
    pub indexer_id: ID,
    /// Whether this route is enabled.
    pub enabled: bool,
    /// Categories accepted by this route.
    pub categories: Vec<String>,
    /// Relative routing priority.
    pub priority: i32,
}

#[derive(SimpleObject, Clone)]
/// Effective media path, naming, import, permission, and monitoring settings for one content scope.
pub struct MediaSettingsPayload {
    /// Content scope to which these settings apply.
    pub scope: ContentScopeValue,
    /// Primary library path.
    pub library_path: String,
    /// Configured root folders.
    pub root_folders: Vec<RootFolderPayload>,
    /// Effective required audio language codes.
    pub required_audio_languages: Vec<String>,
    /// Folder naming template.
    pub folder_template: String,
    /// Season-folder template, or null for facets without seasons.
    pub season_folder_template: Option<String>,
    /// Specials-folder template, or null when not configured.
    pub specials_folder_template: Option<String>,
    /// Whether automatic renaming is enabled.
    pub rename_enabled: bool,
    /// Rename filename template.
    pub rename_template: String,
    /// Collision policy for renames.
    pub rename_collision_policy: RenameCollisionPolicyValue,
    /// Missing-metadata policy for renames.
    pub rename_missing_metadata_policy: RenameMissingMetadataPolicyValue,
    /// Effective filler policy, or null when unset.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Effective recap policy, or null when unset.
    pub recap_policy: Option<RecapPolicyValue>,
    /// Whether specials are monitored, or null when unset.
    pub monitor_specials: Option<bool>,
    /// Whether inter-season movies are monitored, or null when unset.
    pub inter_season_movies: Option<bool>,
    /// Whether filler movies are monitored, or null when unset.
    pub monitor_filler_movies: Option<bool>,
    /// Whether NFO files are written on import.
    pub nfo_write_on_import: bool,
    /// Whether Plex match files are written on import, or null when unset.
    pub plexmatch_write_on_import: Option<bool>,
    /// Import mode applied to this scope.
    pub import_mode: ImportModeValue,
    /// Whether Linux permissions are updated after import.
    pub set_permissions_linux: bool,
    /// File chmod mode, or null when unset.
    pub file_chmod: Option<String>,
    /// Folder chmod mode, or null when unset.
    pub folder_chmod: Option<String>,
    /// Chown group, or null when unset.
    pub chown_group: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Filesystem paths used by the three supported media facets.
pub struct LibraryPathsPayload {
    /// Movie library path.
    pub movie_path: String,
    /// Series library path.
    pub series_path: String,
    /// Anime library path.
    pub anime_path: String,
}

#[derive(SimpleObject, Clone)]
/// TLS certificate and private-key paths used by the service.
pub struct ServiceSettingsPayload {
    /// Filesystem path to the TLS certificate.
    pub tls_cert_path: String,
    /// Filesystem path to the TLS private key.
    pub tls_key_path: String,
}

#[derive(InputObject, Clone)]
/// An external identifier supplied with a title or media request.
pub struct ExternalIdInput {
    /// Provider namespace for the identifier, such as TVDB or IMDb.
    pub source: String,
    /// Provider-issued identifier value.
    pub value: String,
}

#[derive(InputObject, Clone)]
/// Optional title settings used when creating or updating a title.
pub struct TitleOptionsInput {
    /// Quality profile identity; omission preserves the current value, null clears it, and a value replaces it.
    pub quality_profile_id: MaybeUndefined<ID>,
    /// Root-folder identity; omission preserves the current value, null clears it, and a value replaces it.
    pub root_folder_id: MaybeUndefined<ID>,
    /// Monitoring policy; omission preserves the current value, null clears it, and a value replaces it.
    pub monitor_type: MaybeUndefined<MonitorTypeValue>,
    /// Whether season folders are used; omission preserves the current value, null clears it, and a value replaces it.
    pub use_season_folders: MaybeUndefined<bool>,
    /// Whether specials are monitored; omission preserves the current value, null clears it, and a value replaces it.
    pub monitor_specials: MaybeUndefined<bool>,
    /// Whether inter-season movies are monitored; omission preserves the current value, null clears it, and a value replaces it.
    pub inter_season_movies: MaybeUndefined<bool>,
    /// Filler policy; omission preserves the current value, null clears it, and a value replaces it.
    pub filler_policy: MaybeUndefined<FillerPolicyValue>,
    /// Recap policy; omission preserves the current value, null clears it, and a value replaces it.
    pub recap_policy: MaybeUndefined<RecapPolicyValue>,
}

#[derive(InputObject, Clone)]
/// Metadata and acquisition settings for creating a title.
pub struct AddTitleInput {
    /// Display name of the title.
    pub name: String,
    /// Media facet, such as movie, series, or anime.
    pub facet: MediaFacetValue,
    /// Library identity receiving the title; null lets the server resolve the default behavior.
    pub library_id: Option<ID>,
    /// Whether the title starts monitored.
    pub monitored: bool,
    /// Tag values attached to the title.
    pub tags: Vec<String>,
    /// Optional title settings to apply at creation.
    pub options: Option<TitleOptionsInput>,
    /// External provider identifiers for the title.
    pub external_ids: Option<Vec<ExternalIdInput>>,
    /// Download source locator, such as an NZB URL or magnet URI, used when queuing the title.
    pub source_hint: Option<String>,
    /// Optional source category for the title.
    pub source_kind: Option<DownloadSourceKindValue>,
    /// Optional source release title.
    pub source_title: Option<String>,
    /// Optional minimum availability value used by acquisition logic.
    pub min_availability: Option<String>,
    // Non-artwork metadata fields supplied from the search result.
    // Poster and fanart URLs are sourced from server-side SMG metadata.
    /// Release year when known.
    pub year: Option<i32>,
    /// Plot summary when known.
    pub overview: Option<String>,
    /// Sort key for title ordering.
    pub sort_title: Option<String>,
    /// URL-safe title slug.
    pub slug: Option<String>,
    /// Runtime in minutes.
    pub runtime_minutes: Option<i32>,
    /// Metadata language code.
    pub language: Option<String>,
    /// Provider content-status label.
    pub content_status: Option<String>,
}

#[derive(InputObject, Clone)]
/// Metadata and preferences submitted with a media request.
pub struct SubmitMediaRequestInput {
    /// Library identity in which the requested title belongs.
    pub library_id: ID,
    /// Requested media facet.
    pub facet: MediaFacetValue,
    /// Requested title name.
    pub title: String,
    /// External provider identifiers for the request.
    pub external_ids: Vec<ExternalIdInput>,
    /// Release year when known.
    pub year: Option<i32>,
    /// Plot summary when known.
    pub overview: Option<String>,
    /// Sort key for title ordering.
    pub sort_title: Option<String>,
    /// URL-safe title slug.
    pub slug: Option<String>,
    /// Runtime in minutes.
    pub runtime_minutes: Option<i32>,
    /// Metadata language code.
    pub language: Option<String>,
    /// Provider content-status label.
    pub content_status: Option<String>,
    /// Quality profile identity requested for approval.
    pub requested_quality_profile_id: Option<ID>,
    /// Monitoring policy requested for approval.
    pub requested_monitor_type: Option<MonitorTypeValue>,
}

#[derive(InputObject, Clone)]
/// Approval choices for a media request.
pub struct ApproveMediaRequestInput {
    /// Media request identity to approve.
    pub request_id: ID,
    /// Quality profile identity to apply to the approved title.
    pub quality_profile_id: ID,
    /// Optional monitoring policy to apply to the approved title.
    pub monitor_type: Option<MonitorTypeValue>,
}

#[derive(InputObject, Clone)]
/// Replacement preferences for the caller's media request.
pub struct UpdateMediaRequestInput {
    /// Media request identity to update.
    pub request_id: ID,
    /// Quality profile identity requested for the title.
    pub requested_quality_profile_id: ID,
    /// Optional monitoring policy requested for the title.
    pub requested_monitor_type: Option<MonitorTypeValue>,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after a media-request action.
pub struct MediaRequestActionPayload {
    /// The media request the action applied to.
    pub request_id: ID,
}

#[derive(SimpleObject, Clone)]
/// Result of approving a media request.
pub struct ApproveMediaRequestPayload {
    /// Title identity created or updated by approval.
    pub title_id: ID,
    /// Search counts when approval queued acquisition work.
    pub wanted_search: Option<WantedSearchPayload>,
    /// Non-fatal search error when approval succeeded but search could not be queued.
    pub search_error: Option<String>,
}

#[derive(InputObject)]
/// Filters for an interactive release search.
pub struct SearchReleasesInput {
    /// Title identity whose releases are searched.
    pub title_id: ID,
    /// Optional series/movie link identity for an episodic movie target.
    pub series_movie_link_id: Option<ID>,
    /// Optional season label or number to search.
    pub season: Option<String>,
    /// Optional episode label or number to search.
    pub episode: Option<String>,
    /// Optional result limit; the resolver applies its own default and cap.
    pub limit: Option<i32>,
}

#[derive(InputObject)]
/// Signed-candidate submission choices for an existing title.
pub struct QueueDownloadInput {
    /// Title identity receiving the queued release.
    pub title_id: ID,
    /// Signed token identifying and authorizing the candidate release.
    pub candidate_token: String,
    /// Acquisition scope targeted by the submission.
    pub scope: QueueDownloadScopeInput,
    /// Whether an in-progress submission may be replaced; omission uses the resolver default.
    pub replace_in_progress: Option<bool>,
    /// Submission purpose; omission uses the normal download purpose.
    pub purpose: Option<QueueDownloadPurposeValue>,
}

#[derive(InputObject)]
/// Scope and replacement choices for selecting the best release.
pub struct QueueBestReleaseInput {
    /// Title identity whose best release is selected.
    pub title_id: ID,
    /// Acquisition scope targeted by the selection.
    pub scope: QueueDownloadScopeInput,
    /// Whether an in-progress submission may be replaced; omission uses the resolver default.
    pub replace_in_progress: Option<bool>,
}

#[derive(InputObject)]
/// Scope filters for a background acquisition search.
pub struct TriggerAcquisitionSearchInput {
    /// Wanted category to search, defaulting to missing items.
    pub wanted_kind: Option<WantedKindValue>,
    /// Optional facet restriction.
    pub facet: Option<MediaFacetValue>,
    /// Optional library identities to include.
    pub library_ids: Option<Vec<ID>>,
    /// Optional title identity to include.
    pub title_id: Option<ID>,
    /// Optional season number for the selected title.
    pub season_number: Option<i32>,
    /// Optional wanted-scope identity for searching exactly one scope.
    pub wanted_item_id: Option<ID>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Whether a download submission was accepted or conflicted with existing work.
pub enum QueueDownloadResultStatusValue {
    /// The release was accepted and queued.
    Queued,
    /// The release was not queued because the target scope conflicted.
    Conflict,
}

#[derive(SimpleObject, Clone)]
/// Existing download work that prevented a new submission.
pub struct QueueDownloadConflictPayload {
    /// Title identity involved in the conflict.
    pub title_id: ID,
    /// Current title name.
    pub title_name: String,
    /// Download-client identity, null when the conflict is not tied to a configured client.
    pub download_client_id: Option<ID>,
    /// Download-client provider type.
    pub download_client_type: String,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
    /// Release title already associated with the conflict, when known.
    pub source_title: Option<String>,
    /// Release source category, when known.
    pub source_kind: Option<DownloadSourceKindValue>,
    /// Acquisition scope occupied by the conflicting work.
    pub scope: QueueDownloadScopePayload,
    /// Current queue state, when available.
    pub state: Option<DownloadQueueStateValue>,
    /// Whether the conflicting work may be replaced.
    pub replaceable: bool,
}

#[derive(SimpleObject, Clone)]
/// Result of a release queue request.
pub struct QueueDownloadPayload {
    /// Whether the request was queued or conflicted.
    pub status: QueueDownloadResultStatusValue,
    /// Background job identity when the request was queued.
    pub job_id: Option<ID>,
    /// Title identity receiving the request.
    pub title_id: ID,
    /// Current title name.
    pub title_name: String,
    /// Queued release title, null when the request conflicted.
    pub source_title: Option<String>,
    /// Queued release source category, null when unavailable or conflicted.
    pub source_kind: Option<DownloadSourceKindValue>,
    /// Conflict details when the request was not queued.
    pub conflict: Option<QueueDownloadConflictPayload>,
}

#[derive(SimpleObject, Clone)]
/// Counts reported for a wanted search attempt.
pub struct WantedSearchPayload {
    /// Number of scopes queued for search.
    pub queued_count: i32,
    /// Number of scopes skipped because work was already in progress.
    pub skipped_in_progress_count: i32,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Action represented by a download queue mutation response.
pub enum DownloadQueueActionKindValue {
    /// A manual import was queued.
    QueuedManualImport,
    /// A tracked download was marked ignored.
    IgnoredTrackedDownload,
    /// A tracked download was marked failed.
    MarkedTrackedDownloadFailed,
    /// A title was assigned to a tracked download.
    AssignedTrackedDownloadTitle,
    /// A tracked download was paused.
    Paused,
    /// A tracked download was resumed.
    Resumed,
    /// A queued download deletion command was issued.
    DeleteQueued,
    /// A tracked download was deleted.
    Deleted,
}

#[derive(InputObject)]
/// Candidate mappings selected from a manual-import preview.
pub struct QueueManualImportInput {
    /// Manual-import selection identity.
    pub selection_id: ID,
    /// File mappings to enqueue for import.
    pub files: Vec<ManualImportCandidateMappingInput>,
}

#[derive(InputObject)]
/// Download identity used to build a manual-import preview.
pub struct BeginManualImportSelectionInput {
    /// Configured download-client identity.
    pub client_id: ID,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
    /// Title identity used to suggest import targets.
    pub title_id: ID,
}

#[derive(InputObject, Clone)]
/// Scope and mode for previewing media file renames.
pub struct MediaRenamePreviewInput {
    /// Facet whose paths are previewed.
    pub facet: MediaFacetValue,
    /// Optional title identity; null previews the full facet scope.
    pub title_id: Option<ID>,
    /// Whether to calculate changes without applying them; omission uses the resolver default.
    pub dry_run: Option<bool>,
    /// Whether to return only the items counted by `renamable`; counts and fingerprint still describe the whole plan.
    pub renamable_only: Option<bool>,
    /// Maximum number of `items` returned; counts and fingerprint still describe the whole plan.
    pub max_items: Option<i32>,
}

#[derive(InputObject, Clone)]
/// Scope and mode for previewing renames across several titles at once.
pub struct MediaRenamePreviewBulkInput {
    /// Facet shared by every requested title.
    pub facet: MediaFacetValue,
    /// Titles whose rename plans are returned, in the order supplied.
    pub title_ids: Vec<ID>,
    /// Whether to return only the items counted by `renamable`; counts and fingerprint still describe the whole plan.
    pub renamable_only: Option<bool>,
    /// Maximum number of `items` returned across all plans; counts and fingerprints still describe the whole plan.
    pub max_items: Option<i32>,
}

#[derive(InputObject, Clone)]
/// Idempotent request to apply a title's rename plan.
pub struct MediaRenameApplyInput {
    /// Facet containing the title.
    pub facet: MediaFacetValue,
    /// Title identity whose rename plan is applied.
    pub title_id: ID,
    /// Preview fingerprint required to ensure the plan is current.
    pub fingerprint: String,
    /// Optional caller key preventing duplicate application of the same request.
    pub idempotency_key: Option<String>,
}

#[derive(InputObject, Clone)]
/// Idempotent request to apply a bulk rename plan.
pub struct MediaRenameBulkApplyInput {
    /// Facet containing the titles.
    pub facet: MediaFacetValue,
    /// Preview fingerprint required to ensure the plan is current.
    pub fingerprint: String,
    /// Optional caller key preventing duplicate application of the same request.
    pub idempotency_key: Option<String>,
}

#[derive(InputObject, Clone)]
/// Subtitle language preference with optional hearing-impaired and forced flags.
pub struct SubtitleLanguagePreferenceInput {
    /// BCP 47 or provider language code.
    pub code: String,
    /// Whether hearing-impaired subtitles are preferred.
    pub hearing_impaired: Option<bool>,
    /// Whether forced subtitles are preferred.
    pub forced: Option<bool>,
}

#[derive(InputObject, Clone)]
/// Delay and routing policy for a download source.
pub struct DelayProfileInput {
    /// Existing delay-profile identity to update.
    pub id: ID,
    /// Display name of the delay profile.
    pub name: String,
    /// Usenet delay in minutes.
    pub usenet_delay_minutes: i32,
    /// Torrent delay in minutes.
    pub torrent_delay_minutes: i32,
    /// Preferred protocol when both delayed sources qualify.
    pub preferred_protocol: DelayProfilePreferredProtocolValue,
    /// Minimum release age in minutes.
    pub min_age_minutes: i32,
    /// Optional score threshold that bypasses the delay.
    pub bypass_score_threshold: Option<i32>,
    /// Facets to which the profile applies.
    pub applies_to_facets: Vec<MediaFacetValue>,
    /// Tags restricting the profile's scope.
    pub tags: Vec<String>,
    /// Relative priority among delay profiles.
    pub priority: i32,
    /// Whether this delay profile is active.
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
/// Library root path and default marker.
pub struct RootFolderInput {
    /// Absolute filesystem path for the library root.
    pub path: String,
    /// Whether this root is the library default.
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
/// Optional media-settings values applied to a content scope.
pub struct UpdateMediaSettingsInput {
    /// Content scope receiving the settings.
    pub scope: ContentScopeValue,
    /// Optional library path override.
    pub library_path: Option<String>,
    /// Optional replacement list of library roots; null leaves existing roots unchanged.
    pub root_folders: Option<Vec<RootFolderInput>>,
    /// Optional required audio-language codes.
    pub required_audio_languages: Option<Vec<String>>,
    /// Optional title folder template.
    pub folder_template: Option<String>,
    /// Optional season folder template.
    pub season_folder_template: Option<String>,
    /// Optional specials folder template.
    pub specials_folder_template: Option<String>,
    /// Whether automatic renaming is enabled.
    pub rename_enabled: Option<bool>,
    /// Optional file and folder rename template.
    pub rename_template: Option<String>,
    /// Collision policy for generated paths.
    pub rename_collision_policy: Option<RenameCollisionPolicyValue>,
    /// Behavior when metadata needed for renaming is missing.
    pub rename_missing_metadata_policy: Option<RenameMissingMetadataPolicyValue>,
    /// Filler monitoring policy.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Recap monitoring policy.
    pub recap_policy: Option<RecapPolicyValue>,
    /// Whether specials are monitored.
    pub monitor_specials: Option<bool>,
    /// Whether inter-season movies are monitored.
    pub inter_season_movies: Option<bool>,
    /// Whether filler movies are monitored.
    pub monitor_filler_movies: Option<bool>,
    /// Whether NFO metadata is written during import.
    pub nfo_write_on_import: Option<bool>,
    /// Whether Plex match metadata is written during import.
    pub plexmatch_write_on_import: Option<bool>,
    /// Import mode used for files in this scope.
    pub import_mode: Option<ImportModeValue>,
    /// Whether Linux ownership and mode changes are applied.
    pub set_permissions_linux: Option<bool>,
    /// File chmod mode in numeric or accepted symbolic notation.
    pub file_chmod: Option<String>,
    /// Folder chmod mode in numeric or accepted symbolic notation.
    pub folder_chmod: Option<String>,
    /// Optional Unix group name for imported paths.
    pub chown_group: Option<String>,
}

#[derive(InputObject, Clone)]
/// Root paths for movie, series, and optional anime libraries.
pub struct UpdateLibraryPathsInput {
    /// Movie library path.
    pub movie_path: String,
    /// Series library path.
    pub series_path: String,
    /// Anime library path, when configured.
    pub anime_path: Option<String>,
}

#[derive(InputObject, Clone)]
/// TLS certificate and private-key filesystem paths.
pub struct UpdateServiceSettingsInput {
    /// Absolute TLS certificate path.
    pub tls_cert_path: String,
    /// Absolute TLS private-key path.
    pub tls_key_path: String,
}

#[derive(InputObject, Clone)]
/// General retention, cache, and plugin trust settings.
pub struct UpdateGeneralSettingsInput {
    /// Whether history is retained without expiry.
    pub keep_history_forever: Option<bool>,
    /// History retention period in days when not retained forever.
    pub history_retention_days: Option<i32>,
    /// Maximum image-cache size in megabytes.
    pub image_cache_max_size_mb: Option<i32>,
    /// PEM bundle path or contents used to trust plugin HTTP certificates.
    pub plugin_http_ca_bundle_pem: Option<String>,
}

#[derive(InputObject, Clone)]
/// Automatic-backup schedule and key-management settings.
pub struct UpdateAutoBackupSettingsInput {
    /// Whether automatic backups are enabled.
    pub enabled: bool,
    /// Daily backup time in local time format.
    pub daily_time_local: String,
    /// New automatic-backup encryption key, when rotating or setting one.
    pub set_auto_backup_key: Option<String>,
    /// Whether the stored automatic-backup key should be cleared.
    pub clear_auto_backup_key: bool,
}

#[derive(InputObject, Clone)]
/// Optional custom backup destination.
pub struct UpdateBackupSettingsInput {
    /// Filesystem path used for custom backups.
    pub custom_backup_path: Option<String>,
}

#[derive(InputObject, Clone)]
/// Authentication and local-access security settings.
pub struct UpdateSecuritySettingsInput {
    /// Whether form login is enabled.
    pub form_login_enabled: bool,
    /// Minimum accepted password length.
    pub password_min_length: i32,
    /// Whether local IPs may skip login.
    pub skip_login_for_local_ips: bool,
    /// Whether sensitive configuration changes require MFA step-up.
    pub mfa_require_config_step_up: bool,
    /// Whether password login requires MFA.
    pub mfa_require_password_login: bool,
    /// Whether Jellyfin login requires TOTP.
    pub totp_require_jellyfin_login: bool,
    /// Whether Emby login requires TOTP. Omission preserves the saved setting.
    pub totp_require_emby_login: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Runtime state for one external authentication connection.
pub struct ExternalAuthRuntimeConnectionPayload {
    /// Connection identity.
    pub id: ID,
    /// External provider type.
    pub provider: ExternalAccountProviderValue,
    /// Display name of the connection.
    pub display_name: String,
    /// Whether login through this connection is enabled.
    pub login_enabled: bool,
    /// Whether account linking through this connection is enabled.
    pub linking_enabled: bool,
    /// Whether Emby Connect is enabled for this connection.
    pub emby_connect_enabled: bool,
}

#[derive(SimpleObject, Clone)]
/// Runtime authentication providers, linking providers, and connections.
pub struct ExternalAuthRuntimeSettingsPayload {
    /// Providers enabled for login.
    pub login_providers: Vec<ExternalAccountProviderValue>,
    /// Providers enabled for account linking.
    pub linking_providers: Vec<ExternalAccountProviderValue>,
    /// Configured external authentication connections.
    pub connections: Vec<ExternalAuthRuntimeConnectionPayload>,
}

#[derive(InputObject, Clone)]
/// Invitation linking a user to an external provider account.
pub struct CreateExternalAccountInviteInput {
    /// User identity receiving the invitation.
    pub user_id: ID,
    /// External authentication connection identity.
    pub connection_id: ID,
    /// Provider represented by the connection.
    pub provider: ExternalAccountProviderValue,
    /// Provider-side user identifier used to match the account.
    pub provider_user_identifier: String,
    /// Optional provider-native user id.
    pub provider_user_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Mapping from an external media-server path to a local path.
pub struct MediaServerPathMappingPayload {
    /// Path reported by the external server.
    pub source_path: String,
    /// Corresponding local filesystem path.
    pub destination_path: String,
}

#[derive(InputObject, Clone)]
/// Path mapping used when connecting an external media server.
pub struct MediaServerPathMappingInput {
    /// Path reported by the external server.
    pub source_path: String,
    /// Corresponding local filesystem path.
    pub destination_path: String,
}

#[derive(SimpleObject, Clone)]
/// Default library permission grant for a media-server connection.
pub struct MediaServerDefaultLibraryGrantPayload {
    /// Library identity receiving the grant.
    pub library_id: ID,
    /// Permissions granted by default.
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(InputObject, Clone)]
/// Default library permission grant to create with a media-server connection.
pub struct MediaServerDefaultLibraryGrantInput {
    /// Library identity receiving the grant.
    pub library_id: ID,
    /// Permissions granted by default.
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(SimpleObject, Clone)]
/// Configured media-server connection with secrets represented by presence flags.
pub struct MediaServerConnectionPayload {
    /// Connection identity.
    pub id: ID,
    /// Media-server provider type.
    pub provider: MediaServerProviderValue,
    /// Display name of the connection.
    pub display_name: String,
    /// Server base URL.
    pub base_url: String,
    /// Whether the connection is active.
    pub enabled: bool,
    /// Whether login through this server is enabled.
    pub login_enabled: bool,
    /// Whether account linking through this server is enabled.
    pub linking_enabled: bool,
    /// Whether automatic account addition is enabled.
    pub auto_add_enabled: bool,
    /// Default application permissions for auto-added users.
    pub default_app_permissions: Vec<AppPermissionValue>,
    /// Default library grants for auto-added users.
    pub default_library_grants: Vec<MediaServerDefaultLibraryGrantPayload>,
    /// Whether a machine identity is configured.
    pub machine_id_present: bool,
    /// Whether an API key is configured.
    pub api_key_present: bool,
    /// Whether an Emby server id is configured.
    pub emby_server_id_present: bool,
    /// Whether Emby Connect is enabled.
    pub emby_connect_enabled: bool,
    /// Configured external-to-local path mappings.
    pub path_mappings: Vec<MediaServerPathMappingPayload>,
    /// Creation timestamp in RFC 3339 format.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp in RFC 3339 format.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a media-server connection.
pub struct DeleteMediaServerConnectionPayload {
    /// Deleted connection identity.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// User discovered on a Jellyfin server.
pub struct JellyfinServerUserPayload {
    /// Server-side user identity.
    pub id: String,
    /// Username on the server.
    pub username: String,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Optional avatar URL.
    pub avatar_url: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Availability state for grouped media-server users.
pub enum MediaServerUserGroupStatusValue {
    /// User discovery completed successfully.
    Ready,
    /// Discovery needs credentials.
    MissingCredentials,
    /// Discovery failed.
    Error,
}

#[derive(SimpleObject, Clone)]
/// User returned from a media-server account discovery operation.
pub struct MediaServerUserPayload {
    /// Server-side user identity.
    pub id: String,
    /// Username on the server.
    pub username: String,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Optional avatar URL.
    pub avatar_url: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Grouped media-server users and discovery status.
pub struct MediaServerUserGroupPayload {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Connection display name.
    pub connection_name: String,
    /// External provider type.
    pub provider: ExternalAccountProviderValue,
    /// Discovery status.
    pub status: MediaServerUserGroupStatusValue,
    /// Error detail when discovery failed.
    pub error_message: Option<String>,
    /// Users discovered from the server.
    pub users: Vec<MediaServerUserPayload>,
}

#[derive(SimpleObject, Clone)]
/// A media server discovered through Plex.
pub struct PlexServerDiscoveryPayload {
    /// Discovered server identity.
    pub id: String,
    /// Discovered server name.
    pub name: String,
}

#[derive(SimpleObject, Clone)]
/// Emby Connect server and its reachable addresses.
pub struct EmbyConnectServerPayload {
    /// Emby Connect server identity.
    pub server_id: String,
    /// Server display name.
    pub name: String,
    /// Emby Connect user category.
    pub user_type: EmbyConnectUserTypeValue,
    /// Local server address, when advertised.
    pub local_address: Option<String>,
    /// Remote server address, when advertised.
    pub remote_address: Option<String>,
    /// Local API base URL, when reachable.
    pub local_api_base_url: Option<String>,
    /// Remote API base URL, when reachable.
    pub remote_api_base_url: Option<String>,
    /// Local address probe result.
    pub local_status: EmbyConnectAddressStatusValue,
    /// Remote address probe result.
    pub remote_status: EmbyConnectAddressStatusValue,
    /// Server address selected by connection logic, when one is available.
    pub suggested_base_url: Option<String>,
}

#[derive(InputObject, Clone)]
/// Credentials used to discover Emby Connect servers.
pub struct DiscoverEmbyConnectServersInput {
    /// Emby Connect username or email.
    pub username_or_email: String,
    /// Emby Connect password.
    pub password: String,
}

#[derive(InputObject, Clone)]
/// Credentials used to test an Emby Connect server.
pub struct TestEmbyConnectInput {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Emby Connect username or email.
    pub username_or_email: String,
    /// Emby Connect password.
    pub password: String,
}

#[derive(SimpleObject, Clone)]
/// Result of a media-server connection test.
pub struct MediaServerConnectionTestPayload {
    /// Machine-readable test status.
    pub status: String,
    /// Optional human-readable detail.
    pub message: Option<String>,
}

#[derive(InputObject, Clone)]
/// Configuration and credentials for a new media-server connection.
pub struct CreateMediaServerConnectionInput {
    /// Media-server provider type.
    pub provider: MediaServerProviderValue,
    /// Display name for the connection.
    pub display_name: String,
    /// Server base URL.
    pub base_url: String,
    /// Whether the connection is enabled, defaulting to true.
    pub enabled: Option<bool>,
    /// Whether login through the server is enabled, defaulting to false.
    pub login_enabled: Option<bool>,
    /// Whether account linking is enabled, defaulting to false.
    pub linking_enabled: Option<bool>,
    /// Whether automatic account addition is enabled, defaulting to false.
    pub auto_add_enabled: Option<bool>,
    /// Default application permissions for auto-added users.
    pub default_app_permissions: Option<Vec<AppPermissionValue>>,
    /// Default library grants for auto-added users.
    pub default_library_grants: Option<Vec<MediaServerDefaultLibraryGrantInput>>,
    /// Provider machine identity, when required.
    pub machine_id: Option<String>,
    /// Plex authentication token; stored as a secret and not returned.
    pub plex_auth_token: Option<String>,
    /// Plex server identity.
    pub plex_server_id: Option<String>,
    /// Provider API key; stored as a secret and not returned.
    pub api_key: Option<String>,
    /// Provider administrator username.
    pub admin_username: Option<String>,
    /// Provider administrator password; stored as a secret and not returned.
    pub admin_password: Option<String>,
    /// Address-selection mode used for the Emby connection.
    pub emby_connection_mode: Option<EmbyConnectionModeValue>,
    /// Credential flow used to configure a local Emby server.
    pub emby_local_setup_method: Option<EmbyLocalSetupMethodValue>,
    /// Whether Emby Connect is enabled.
    pub emby_connect_enabled: Option<bool>,
    /// Emby Connect account name used to authenticate the connection.
    pub emby_connect_username_or_email: Option<String>,
    /// Emby Connect password; stored as a secret and not returned.
    pub emby_connect_password: Option<String>,
    /// Emby Connect server identity.
    pub emby_connect_server_id: Option<String>,
    /// External-to-local filesystem path mappings.
    pub path_mappings: Option<Vec<MediaServerPathMappingInput>>,
}

#[derive(InputObject, Clone)]
/// Patch for an existing media-server connection.
pub struct UpdateMediaServerConnectionInput {
    /// Media-server connection identity to patch.
    pub id: ID,
    /// Replacement provider type; omission preserves the current value.
    pub provider: Option<MediaServerProviderValue>,
    /// Replacement display name; omission preserves the current value.
    pub display_name: Option<String>,
    /// Replacement base URL; omission preserves the current value.
    pub base_url: Option<String>,
    /// Replacement enabled state; omission preserves the current value.
    pub enabled: Option<bool>,
    /// Replacement login-enabled state; omission preserves the current value.
    pub login_enabled: Option<bool>,
    /// Replacement linking-enabled state; omission preserves the current value.
    pub linking_enabled: Option<bool>,
    /// Replacement auto-add state; omission preserves the current value.
    pub auto_add_enabled: Option<bool>,
    /// Replacement default application permissions; omission preserves the current list.
    pub default_app_permissions: Option<Vec<AppPermissionValue>>,
    /// Replacement default library grants; omission preserves the current list.
    pub default_library_grants: Option<Vec<MediaServerDefaultLibraryGrantInput>>,
    /// Replacement machine identity; omission preserves it.
    pub machine_id: Option<String>,
    /// Whether the stored machine identity should be cleared.
    pub clear_machine_id: Option<bool>,
    /// Replacement Plex authentication token; omission preserves it and the value is never returned.
    pub plex_auth_token: Option<String>,
    /// Replacement Plex server identity; omission preserves it.
    pub plex_server_id: Option<String>,
    /// Replacement provider API key; omission preserves it and the value is never returned.
    pub api_key: Option<String>,
    /// Whether the stored API key should be cleared.
    pub clear_api_key: Option<bool>,
    /// Replacement provider administrator username; omission preserves it.
    pub admin_username: Option<String>,
    /// Replacement provider administrator password; omission preserves it and the value is never returned.
    pub admin_password: Option<String>,
    /// Replacement Emby connection mode; omission preserves it.
    pub emby_connection_mode: Option<EmbyConnectionModeValue>,
    /// Replacement Emby local setup method; omission preserves it.
    pub emby_local_setup_method: Option<EmbyLocalSetupMethodValue>,
    /// Replacement Emby Connect enabled state; omission preserves it.
    pub emby_connect_enabled: Option<bool>,
    /// Replacement Emby Connect username or email; omission preserves it.
    pub emby_connect_username_or_email: Option<String>,
    /// Replacement Emby Connect password; omission preserves it and the value is never returned.
    pub emby_connect_password: Option<String>,
    /// Replacement Emby Connect server identity; omission preserves it.
    pub emby_connect_server_id: Option<String>,
    /// Replacement path mappings; omission preserves the current mappings.
    pub path_mappings: Option<Vec<MediaServerPathMappingInput>>,
}

#[derive(InputObject, Clone)]
/// Credentials used to test an existing media-server connection.
pub struct TestMediaServerConnectionInput {
    /// Media-server connection identity to test.
    pub id: ID,
    /// Optional Plex authentication token used for this test.
    pub plex_auth_token: Option<String>,
}

#[derive(InputObject, Clone)]
/// Plex credentials used to link an external account.
pub struct LinkPlexAccountInput {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Plex authentication token; used for linking and not returned.
    pub plex_auth_token: String,
}

#[derive(InputObject, Clone)]
/// Jellyfin credentials used to link an external account.
pub struct LinkJellyfinAccountInput {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Jellyfin username.
    pub username: String,
    /// Jellyfin password; used for linking and not returned.
    pub password: String,
}

#[derive(InputObject, Clone)]
/// Emby credentials used to link an external account.
pub struct LinkEmbyAccountInput {
    /// Media-server connection identity.
    pub connection_id: ID,
    /// Emby connection mode.
    pub mode: EmbyConnectionModeValue,
    /// Emby username.
    pub username: String,
    /// Emby password; used for linking and not returned.
    pub password: String,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after unlinking an external account.
pub struct UnlinkExternalAccountPayload {
    /// Linked-account identity that was removed.
    pub linked_account_id: ID,
}

#[derive(InputObject, Clone)]
/// Subtitle acquisition, translation, and synchronization settings.
pub struct UpdateSubtitleSettingsInput {
    /// Whether subtitle processing is enabled.
    pub enabled: bool,
    /// Ordered language preferences.
    pub languages: Vec<SubtitleLanguagePreferenceInput>,
    /// Whether subtitles are downloaded automatically after import.
    pub auto_download_on_import: bool,
    /// Minimum provider score for series subtitles.
    pub minimum_score_series: i32,
    /// Minimum provider score for movie subtitles.
    pub minimum_score_movie: i32,
    /// Search interval in hours.
    pub search_interval_hours: i32,
    /// Whether AI-translated subtitles are accepted.
    pub include_ai_translated: bool,
    /// Whether machine-translated subtitles are accepted.
    pub include_machine_translated: bool,
    /// Whether subtitle synchronization is enabled.
    pub sync_enabled: bool,
    /// Synchronization threshold for series subtitles.
    pub sync_threshold_series: i32,
    /// Synchronization threshold for movie subtitles.
    pub sync_threshold_movie: i32,
    /// Maximum synchronization offset in seconds.
    pub sync_max_offset_seconds: i32,
}

#[derive(InputObject, Clone)]
/// Recycle-bin enablement setting.
pub struct UpdateRecycleBinSettingsInput {
    /// Whether deleted media is retained in the recycle bin.
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
/// Automatic official-plugin patch update setting.
pub struct UpdatePluginAutoUpdateSettingsInput {
    /// Whether the scheduled plugin catalog refresh installs official patch updates automatically.
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
/// Acquisition scheduler timing and scoring thresholds.
pub struct UpdateAcquisitionSettingsInput {
    /// Whether acquisition scheduling is enabled.
    pub enabled: bool,
    /// Upgrade cooldown in hours.
    pub upgrade_cooldown_hours: i32,
    /// Minimum score improvement for a same-tier upgrade.
    pub same_tier_min_delta: i32,
    /// Minimum score improvement for a cross-tier upgrade.
    pub cross_tier_min_delta: i32,
    /// Score delta that bypasses the forced-upgrade guard.
    pub forced_upgrade_delta_bypass: i32,
    /// Scheduler poll interval in seconds.
    pub poll_interval_seconds: i32,
    /// Maximum long-tail scopes processed per cycle.
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    /// Days between long-tail reconvergence passes.
    pub long_tail_reconverge_days: i32,
}

#[derive(InputObject, Clone)]
/// Per-title scoring behavior overrides.
pub struct ScoringOverridesInput {
    /// Whether non-4K x265 releases are allowed.
    pub allow_x265_non4k: Option<bool>,
    /// Whether Dolby Vision without a fallback is blocked.
    pub block_dv_without_fallback: Option<bool>,
    /// Whether compact encodes are preferred.
    pub prefer_compact_encodes: Option<bool>,
    /// Whether lossless audio is preferred.
    pub prefer_lossless_audio: Option<bool>,
    /// Whether upscaled releases are blocked.
    pub block_upscaled: Option<bool>,
}

#[derive(InputObject, Clone)]
/// Quality constraints and scoring rules for release selection.
pub struct QualityProfileCriteriaInput {
    /// Ordered quality-tier identifiers.
    pub quality_tiers: Vec<String>,
    /// Optional archival quality tier.
    pub archival_quality: Option<String>,
    /// Whether releases with unknown quality are allowed.
    pub allow_unknown_quality: bool,
    /// Allowed release-source identifiers.
    pub source_allowlist: Vec<String>,
    /// Blocked release-source identifiers.
    pub source_blocklist: Vec<String>,
    /// Allowed video-codec identifiers.
    pub video_codec_allowlist: Vec<String>,
    /// Blocked video-codec identifiers.
    pub video_codec_blocklist: Vec<String>,
    /// Allowed audio-codec identifiers.
    pub audio_codec_allowlist: Vec<String>,
    /// Blocked audio-codec identifiers.
    pub audio_codec_blocklist: Vec<String>,
    /// Whether Dolby Vision is allowed.
    pub dolby_vision_allowed: bool,
    /// Whether detected HDR is allowed.
    pub detected_hdr_allowed: bool,
    /// Whether remux releases are preferred.
    pub prefer_remux: bool,
    /// Whether Blu-ray disk releases are allowed.
    pub allow_bd_disk: bool,
    /// Whether upgrades above the current file are allowed.
    pub allow_upgrades: bool,
    /// Additional scoring preferences.
    pub scoring_overrides: ScoringOverridesInput,
    /// Optional cutoff quality tier.
    pub cutoff_tier: Option<String>,
    /// Optional minimum score required to grab a release.
    pub min_score_to_grab: Option<i32>,
}

#[derive(InputObject, Clone)]
/// Named quality profile definition.
pub struct QualityProfileInput {
    /// Quality profile identity.
    pub id: ID,
    /// Display name of the profile.
    pub name: String,
    /// Constraints used to accept and score releases.
    pub criteria: QualityProfileCriteriaInput,
}

#[derive(InputObject, Clone)]
/// Quality profile selection for a content scope.
pub struct QualityProfileSelectionInput {
    /// Content scope receiving the selection.
    pub scope: ContentScopeValue,
    /// Quality profile identity; null uses inherited or global behavior.
    pub profile_id: Option<ID>,
    /// Whether the scope inherits the global profile.
    pub inherit_global: bool,
}

#[derive(InputObject, Clone)]
/// Scoring-persona selection for a content scope.
pub struct FacetScoringPersonaSelectionInput {
    /// Content scope receiving the selection.
    pub scope: ContentScopeValue,
    /// Optional scoring persona override.
    pub persona: Option<ScoringPersonaValue>,
    /// Whether the scope inherits the global persona.
    pub inherit_global: bool,
}

#[derive(InputObject, Clone)]
/// Complete quality-profile and scoring-persona settings replacement.
pub struct SaveQualityProfileSettingsInput {
    /// Quality profiles to store.
    pub profiles: Vec<QualityProfileInput>,
    /// Global quality profile identity, when configured.
    pub global_profile_id: Option<ID>,
    /// Global scoring persona, when configured.
    pub global_scoring_persona: Option<ScoringPersonaValue>,
    /// Per-category quality-profile selections.
    pub category_selections: Vec<QualityProfileSelectionInput>,
    /// Per-category scoring-persona selections.
    pub category_persona_selections: Vec<FacetScoringPersonaSelectionInput>,
    /// Whether existing stored profiles and selections are replaced.
    pub replace_existing: bool,
}

#[derive(InputObject, Clone)]
/// Routing settings for one download client.
pub struct DownloadClientRoutingEntryInput {
    /// Download-client identity.
    pub client_id: ID,
    /// Whether this client participates in routing.
    pub enabled: bool,
    /// Optional category sent to the client.
    pub category: Option<String>,
    /// Priority label for recent queue items.
    pub recent_queue_priority: Option<String>,
    /// Priority label for older queue items.
    pub older_queue_priority: Option<String>,
    /// Whether completed items are removed from the client.
    pub remove_completed: bool,
    /// Whether failed items are removed from the client.
    pub remove_failed: bool,
}

#[derive(InputObject, Clone)]
/// Download-client routing entries for a content scope.
pub struct UpdateDownloadClientRoutingInput {
    /// Content scope receiving the routing rules.
    pub scope: ContentScopeValue,
    /// Routing entries keyed by download-client identity.
    pub entries: Vec<DownloadClientRoutingEntryInput>,
}

#[derive(InputObject, Clone)]
/// Routing settings for one indexer.
pub struct IndexerRoutingEntryInput {
    /// Indexer identity.
    pub indexer_id: ID,
    /// Whether this indexer participates in routing.
    pub enabled: bool,
    /// Indexer categories to request.
    pub categories: Vec<String>,
    /// Relative indexer priority.
    pub priority: i32,
}

#[derive(InputObject, Clone)]
/// Indexer routing entries for a content scope.
pub struct UpdateIndexerRoutingInput {
    /// Content scope receiving the routing rules.
    pub scope: ContentScopeValue,
    /// Routing entries keyed by indexer identity.
    pub entries: Vec<IndexerRoutingEntryInput>,
}

#[derive(InputObject)]
/// Configuration for a new indexer provider.
pub struct CreateIndexerConfigInput {
    /// Display name of the indexer.
    pub name: String,
    /// Provider implementation identifier.
    pub provider_type: String,
    /// Optional proxy configuration identity.
    pub indexer_proxy_config_id: Option<ID>,
    /// Optional download-client identity used for routed grabs.
    pub download_client_id: Option<ID>,
    /// Rate-limit interval in seconds.
    pub rate_limit_seconds: Option<i64>,
    /// Maximum requests allowed in one rate-limit burst.
    pub rate_limit_burst: Option<i64>,
    /// Whether the indexer is enabled.
    pub is_enabled: Option<bool>,
    /// Whether interactive searches use this indexer.
    pub enable_interactive_search: Option<bool>,
    /// Whether automatic searches use this indexer.
    pub enable_auto_search: Option<bool>,
    /// Provider configuration values, including secret fields.
    pub config: Option<Vec<ProviderConfigValueInput>>,
}

#[derive(InputObject)]
/// Patch for an existing indexer provider.
pub struct UpdateIndexerConfigInput {
    /// Indexer configuration identity to patch.
    pub id: ID,
    /// Replacement display name; omission preserves the current value.
    pub name: Option<String>,
    /// Replacement provider implementation; omission preserves the current value.
    pub provider_type: Option<String>,
    /// Proxy identity: omission preserves it, null clears it, and a value replaces it.
    pub indexer_proxy_config_id: MaybeUndefined<ID>,
    /// Download-client identity: omission preserves it, null clears it, and a value replaces it.
    pub download_client_id: MaybeUndefined<ID>,
    /// Replacement rate-limit interval in seconds; omission preserves it.
    pub rate_limit_seconds: Option<i64>,
    /// Replacement rate-limit burst; omission preserves it.
    pub rate_limit_burst: Option<i64>,
    /// Replacement enabled state; omission preserves it.
    pub is_enabled: Option<bool>,
    /// Replacement interactive-search state; omission preserves it.
    pub enable_interactive_search: Option<bool>,
    /// Replacement automatic-search state; omission preserves it.
    pub enable_auto_search: Option<bool>,
    /// Replacement provider configuration; omitted secret fields retain stored secrets.
    pub config: Option<Vec<ProviderConfigValueInput>>,
}

#[derive(InputObject)]
/// Download-client mapping for one indexer.
pub struct SetIndexerDownloadClientMappingInput {
    /// Indexer identity to update.
    pub indexer_id: ID,
    /// Download-client identity to assign, or null to clear the mapping.
    pub download_client_id: Option<ID>,
}

#[derive(InputObject)]
/// Configuration for an indexer proxy provider.
pub struct CreateIndexerProxyConfigInput {
    /// Display name of the proxy.
    pub name: String,
    /// Proxy provider implementation identifier.
    pub provider_type: String,
    /// Proxy base URL.
    pub base_url: String,
    /// Request timeout in seconds.
    pub request_timeout_seconds: Option<i32>,
    /// Whether the proxy is enabled.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Patch for an indexer proxy configuration.
pub struct UpdateIndexerProxyConfigInput {
    /// Proxy configuration identity to patch.
    pub id: ID,
    /// Replacement display name; omission preserves it.
    pub name: Option<String>,
    /// Replacement base URL; omission preserves it.
    pub base_url: Option<String>,
    /// Replacement request timeout in seconds; omission preserves it.
    pub request_timeout_seconds: Option<i32>,
    /// Replacement enabled state; omission preserves it.
    pub is_enabled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting an indexer proxy.
pub struct DeleteIndexerProxyConfigPayload {
    /// Deleted proxy configuration identity.
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting an indexer configuration.
pub struct DeleteIndexerConfigPayload {
    /// Deleted indexer configuration identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Configuration for a new download client.
pub struct CreateDownloadClientConfigInput {
    /// Display name of the client.
    pub name: String,
    /// Download-client provider implementation identifier.
    pub client_type: String,
    /// Provider configuration values, including secret fields.
    pub config: Vec<ProviderConfigValueInput>,
    /// Whether the client is enabled.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Patch for an existing download client.
pub struct UpdateDownloadClientConfigInput {
    /// Download-client configuration identity to patch.
    pub id: ID,
    /// Replacement display name; omission preserves it.
    pub name: Option<String>,
    /// Replacement provider implementation; omission preserves it.
    pub client_type: Option<String>,
    /// Replacement provider configuration; omitted secret fields retain stored secrets.
    pub config: Option<Vec<ProviderConfigValueInput>>,
    /// Replacement enabled state; omission preserves it.
    pub is_enabled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Result of deleting a download-client configuration.
pub struct DeleteDownloadClientConfigPayload {
    /// Deleted download-client identity.
    pub id: async_graphql::ID,
    /// Number of indexer mappings cleared as a consequence.
    pub cleared_indexer_mapping_count: i32,
}

#[derive(InputObject)]
/// New ordering for download-client configurations.
pub struct ReorderDownloadClientConfigsInput {
    /// Download-client identities in desired order.
    pub ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Persisted download-client configuration order.
pub struct ReorderDownloadClientConfigsPayload {
    /// Download-client identities in stored order.
    pub ids: Vec<ID>,
}

#[derive(InputObject)]
/// Connection details used to test a download client.
pub struct TestDownloadClientConnectionInput {
    /// Existing client identity, when testing a stored configuration.
    pub id: Option<ID>,
    /// Provider implementation identifier for an unsaved configuration.
    pub client_type: String,
    /// Provider configuration values used for the test.
    pub config: Vec<ProviderConfigValueInput>,
}

#[derive(InputObject)]
/// Configuration for a new subtitle provider.
pub struct CreateSubtitleProviderConfigInput {
    /// Display name of the provider.
    pub name: String,
    /// Subtitle provider implementation identifier.
    pub provider_type: String,
    /// Provider configuration values, including secret fields.
    pub config: Vec<ProviderConfigValueInput>,
    /// Facets for which the provider is enabled.
    pub enabled_facets: Option<Vec<MediaFacetValue>>,
    /// Whether the provider is enabled.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Patch for an existing subtitle provider.
pub struct UpdateSubtitleProviderConfigInput {
    /// Subtitle-provider configuration identity to patch.
    pub id: ID,
    /// Replacement display name; omission preserves it.
    pub name: Option<String>,
    /// Replacement provider implementation; omission preserves it.
    pub provider_type: Option<String>,
    /// Replacement provider configuration; omitted secret fields retain stored secrets.
    pub config: Option<Vec<ProviderConfigValueInput>>,
    /// Replacement enabled facets; omission preserves them.
    pub enabled_facets: Option<Vec<MediaFacetValue>>,
    /// Replacement enabled state; omission preserves it.
    pub is_enabled: Option<bool>,
    /// Disable-until timestamp: omission preserves it, null clears it, and a value replaces it.
    pub disabled_until: MaybeUndefined<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a subtitle provider.
pub struct DeleteSubtitleProviderConfigPayload {
    /// Deleted subtitle-provider identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Connection details used to test a subtitle provider.
pub struct TestSubtitleProviderConnectionInput {
    /// Existing provider identity, when testing a stored configuration.
    pub id: Option<ID>,
    /// Provider implementation identifier for an unsaved configuration.
    pub provider_type: String,
    /// Provider configuration values used for the test.
    pub config: Vec<ProviderConfigValueInput>,
}

#[derive(InputObject)]
/// Connection details used to test an indexer provider.
pub struct TestIndexerConnectionInput {
    /// Provider implementation identifier.
    pub provider_type: String,
    /// Optional provider configuration values used for the test.
    pub config: Option<Vec<ProviderConfigValueInput>>,
    /// Existing indexer identity, when testing a stored configuration.
    pub indexer_id: Option<ID>,
    /// Proxy identity: omission preserves the stored association, null clears it, and a value replaces it.
    pub indexer_proxy_config_id: MaybeUndefined<ID>,
}

#[derive(InputObject)]
/// Destructive deletion request for one title.
pub struct DeleteTitleInput {
    /// Title identity to delete.
    pub title_id: ID,
    /// Whether associated media files are removed from disk.
    pub delete_files_on_disk: Option<bool>,
    /// Preview fingerprint required to confirm the current deletion target.
    pub preview_fingerprint: Option<String>,
    /// Required typed confirmation for destructive deletion.
    pub typed_confirmation: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a title.
pub struct DeleteTitlePayload {
    /// Deleted title identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Destructive deletion request for multiple titles.
pub struct DeleteTitlesInput {
    /// Title deletion items with per-title preview fingerprints.
    pub items: Vec<DeleteTitlesItemInput>,
    /// Whether associated media files are removed from disk.
    pub delete_files_on_disk: Option<bool>,
    /// Required typed confirmation for destructive deletion.
    pub typed_confirmation: Option<String>,
}

#[derive(InputObject)]
/// One title identity and its preview fingerprint for bulk deletion.
pub struct DeleteTitlesItemInput {
    /// Title identity to delete.
    pub title_id: ID,
    /// Preview fingerprint required to confirm this title's current deletion target.
    pub preview_fingerprint: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after clearing a title release blocklist entry.
pub struct ClearTitleReleaseBlocklistEntryPayload {
    /// Cleared release-blocklist entry identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Input for generating deletion previews for selected titles.
pub struct DeleteTitlesPreviewInput {
    /// Title identities to include in the preview.
    pub title_ids: Vec<ID>,
}

#[derive(InputObject)]
/// New user account and permission grants.
pub struct CreateUserInput {
    /// Login username.
    pub username: String,
    /// Initial password; stored securely and never returned.
    pub password: String,
    /// Application permissions granted to the user.
    pub app_permissions: Vec<AppPermissionValue>,
    /// Library permissions granted to the user.
    pub library_permissions: Vec<LibraryPermissionGrantInput>,
}

#[derive(InputObject)]
/// Enable or disable login for a user identity.
pub struct SetUserLoginEnabledInput {
    /// User identity to update.
    pub user_id: ID,
    /// Whether login is enabled.
    pub enabled: bool,
}

#[derive(InputObject)]
/// Password replacement for a user account.
pub struct SetUserPasswordInput {
    /// User identity whose password changes.
    pub user_id: ID,
    /// New password; stored securely and never returned.
    pub password: String,
    /// Current password when required by the authorization policy.
    pub current_password: Option<String>,
}

#[derive(InputObject)]
/// Monitoring state change for a title.
pub struct SetTitleMonitoredInput {
    /// Title identity to update.
    pub title_id: ID,
    /// Whether the title is monitored.
    pub monitored: bool,
}

#[derive(InputObject)]
/// Patch for title metadata and settings.
pub struct UpdateTitleInput {
    /// Title identity to patch.
    pub title_id: ID,
    /// Replacement title name; omission preserves it.
    pub name: Option<String>,
    /// Replacement facet; omission preserves it.
    pub facet: Option<MediaFacetValue>,
    /// Replacement tag list; omission preserves it and an empty list clears tags.
    pub tags: Option<Vec<String>>,
    /// Optional title settings patch.
    pub options: Option<TitleOptionsInput>,
}

#[derive(InputObject)]
/// Primary-file assignment for a movie title.
pub struct SetPrimaryMovieFileInput {
    /// Movie title identity.
    pub title_id: ID,
    /// Media-file identity to make primary.
    pub file_id: ID,
}

#[derive(InputObject)]
/// External metadata identity used to repair a title match.
pub struct FixTitleMatchInput {
    /// Title identity to rematch.
    pub title_id: ID,
    /// TVDB identity to associate with the title.
    pub tvdb_id: String,
}

#[derive(InputObject, Clone)]
/// Monitoring state change for a collection.
pub struct SetCollectionMonitoredInput {
    /// Collection identity to update.
    pub collection_id: ID,
    /// Whether the collection is monitored.
    pub monitored: bool,
}

#[derive(InputObject, Clone)]
/// Monitoring state change for an episode.
pub struct SetEpisodeMonitoredInput {
    /// Episode identity to update.
    pub episode_id: ID,
    /// Whether the episode is monitored.
    pub monitored: bool,
}

#[derive(InputObject, Clone)]
/// Monitoring state change for a series-movie link.
pub struct SetSeriesMovieMonitoredInput {
    /// Series-movie link identity to update.
    pub series_movie_link_id: ID,
    /// Whether the linked movie is monitored.
    pub monitored: bool,
}

#[derive(InputObject)]
/// Replacement application permissions for a user.
pub struct SetUserAppPermissionsInput {
    /// User identity to update.
    pub user_id: ID,
    /// Application permissions to store.
    pub permissions: Vec<AppPermissionValue>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a user.
pub struct DeleteUserPayload {
    /// Deleted user identity.
    pub id: ID,
}

#[derive(InputObject, Clone)]
/// Library permission grant for a user.
pub struct LibraryPermissionGrantInput {
    /// Library identity receiving the grant.
    pub library_id: ID,
    /// Library permissions to store.
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(InputObject, Clone)]
/// Replacement library permission grants for a user.
pub struct SetUserLibraryPermissionsInput {
    /// User identity to update.
    pub user_id: ID,
    /// Library grants to store.
    pub grants: Vec<LibraryPermissionGrantInput>,
}

#[derive(InputObject, Clone)]
/// New library root path and default marker.
pub struct CreateLibraryRootInput {
    /// Absolute filesystem path for the root.
    pub path: String,
    /// Whether this root is the library default.
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
/// Replacement library root path and default marker.
pub struct UpdateLibraryRootInput {
    /// Absolute filesystem path for the root.
    pub path: String,
    /// Whether this root is the library default.
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
/// New media library definition.
pub struct CreateLibraryInput {
    /// Media facet stored in the library.
    pub facet: MediaFacetValue,
    /// Library display name.
    pub name: String,
    /// Root paths owned by the library.
    pub roots: Vec<CreateLibraryRootInput>,
    /// Optional library acquisition and import settings.
    pub settings: Option<LibrarySettingsInput>,
}

#[derive(InputObject, Clone)]
/// Patch for an existing media library.
pub struct UpdateLibraryInput {
    /// Library identity to patch.
    pub library_id: ID,
    /// Replacement display name; omission preserves it.
    pub name: Option<String>,
    /// Replacement root list; omission preserves roots and an empty list clears them when valid.
    pub roots: Option<Vec<UpdateLibraryRootInput>>,
    /// Optional replacement library settings.
    pub settings: Option<LibrarySettingsInput>,
}

#[derive(InputObject, Clone)]
/// Acquisition, import, routing, and filesystem settings for a library.
pub struct LibrarySettingsInput {
    /// Required audio-language codes.
    pub required_audio_languages: Option<Vec<String>>,
    /// Default quality profile identity.
    pub quality_profile_id: Option<ID>,
    /// Quality profile identities allowed for requests.
    pub request_quality_profile_ids: Option<Vec<ID>>,
    /// Scoring persona applied by default.
    pub scoring_persona: Option<ScoringPersonaValue>,
    /// Filler monitoring policy.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Recap monitoring policy.
    pub recap_policy: Option<RecapPolicyValue>,
    /// Whether specials are monitored.
    pub monitor_specials: Option<bool>,
    /// Whether inter-season movies are monitored.
    pub inter_season_movies: Option<bool>,
    /// Whether filler movies are monitored.
    pub monitor_filler_movies: Option<bool>,
    /// Whether NFO metadata is written during import.
    pub nfo_write_on_import: Option<bool>,
    /// Whether Plex match metadata is written during import.
    pub plexmatch_write_on_import: Option<bool>,
    /// Import mode used for library files.
    pub import_mode: Option<ImportModeValue>,
    /// Whether Linux ownership and mode changes are applied.
    pub set_permissions_linux: Option<bool>,
    /// File chmod mode in numeric or accepted symbolic notation.
    pub file_chmod: Option<String>,
    /// Folder chmod mode in numeric or accepted symbolic notation.
    pub folder_chmod: Option<String>,
    /// Unix group name applied when permissions are set.
    pub chown_group: Option<String>,
    /// Indexer routing rules for this library.
    pub indexer_routing: Option<Vec<IndexerRoutingEntryInput>>,
    /// Download-client routing rules for this library.
    pub download_client_routing: Option<Vec<DownloadClientRoutingEntryInput>>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a library.
pub struct DeleteLibraryPayload {
    /// Deleted library identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Destructive deletion request for one media file.
pub struct DeleteMediaFileInput {
    /// Media-file identity to delete.
    pub file_id: ID,
    /// Whether the file is removed from disk.
    pub delete_from_disk: Option<bool>,
    /// Preview fingerprint required to confirm the current deletion target.
    pub preview_fingerprint: Option<String>,
    /// Required typed confirmation for destructive deletion.
    pub typed_confirmation: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Result of a media-file deletion request.
pub struct DeleteMediaFilePayload {
    /// Deleted media-file identity.
    pub id: async_graphql::ID,
    /// Background job accepted to complete deletion and related cleanup.
    pub job_run: JobRunPayload,
}

#[derive(InputObject)]
/// Identity of a download item to pause.
pub struct PauseDownloadInput {
    /// Download-client identity; null identifies the default or unscoped client behavior.
    pub client_id: Option<ID>,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
}

#[derive(InputObject)]
/// Identity of a download item to resume.
pub struct ResumeDownloadInput {
    /// Download-client identity; null identifies the default or unscoped client behavior.
    pub client_id: Option<ID>,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
}

#[derive(InputObject)]
/// Identity and history behavior for deleting a tracked download.
pub struct DeleteDownloadInput {
    /// Download-client identity, when the item is scoped to a configured client.
    pub client_id: Option<ID>,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
    /// Whether deletion should use the provider's history path.
    pub is_history: bool,
}

// --- Manual Import ---

#[derive(SimpleObject, Clone)]
/// Candidate file details used to preview a manual import selection.
pub struct ManualImportFilePreviewPayload {
    /// Candidate ID within the persisted manual-import selection; use it only with that selection.
    pub candidate_id: ID,
    /// Candidate file name.
    pub file_name: String,
    /// Candidate file size in bytes.
    pub size_bytes: Long,
    /// Parsed quality label, or null when unavailable.
    pub quality: Option<String>,
    /// Parsed season number, or null when unavailable.
    pub parsed_season: Option<i32>,
    /// Parsed episode numbers; empty means none were detected.
    pub parsed_episodes: Vec<i32>,
    /// Suggested episode ID, or null when no single suggestion is available.
    pub suggested_episode_id: Option<ID>,
    /// Label for the suggested episode, or null when no suggestion exists.
    pub suggested_episode_label: Option<String>,
    /// Suggested series-movie link, or null when this is not a grabbed series movie.
    pub suggested_series_movie_link_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Series-movie target candidate for a manual import.
pub struct ManualImportSeriesMovieTargetPayload {
    /// Series-movie link identity targeted by the candidate.
    pub series_movie_link_id: String,
    /// Movie title associated with the target.
    pub movie_title: String,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Runtime in minutes, or null when unavailable.
    pub runtime_minutes: Option<i32>,
}

#[derive(InputObject)]
/// Maps a persisted manual-import candidate to an episode, series-movie link, or title-level movie target.
pub struct ManualImportCandidateMappingInput {
    /// Candidate ID from the persisted manual-import selection.
    pub candidate_id: ID,
    /// Episode target ID for an episodic import; null for a series-movie or movie import.
    pub episode_id: Option<ID>,
    /// Series-movie link ID for a series-movie import; null for an episode or movie import.
    pub series_movie_link_id: Option<ID>,
}

// --- Wanted Items / Acquisition ---

#[derive(SimpleObject, Clone)]
/// Count of wanted items grouped by decision code.
pub struct DecisionCodeCountPayload {
    /// Decision code.
    pub code: String,
    /// Number of wanted items with this code.
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
/// Count of wanted items grouped by lifecycle status.
pub struct WantedStatusCountPayload {
    /// Wanted-item status.
    pub status: WantedStatusValue,
    /// Number of items with this status.
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
/// Count of pending releases grouped by lifecycle status.
pub struct PendingReleaseStatusCountPayload {
    /// Pending-release status.
    pub status: PendingReleaseStatusValue,
    /// Number of releases with this status.
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
/// One cutoff-unmet target with its current and target quality tiers and convergence state.
pub struct CutoffUnmetItemPayload {
    /// Target title ID.
    pub title_id: ID,
    /// Title display name.
    pub title_name: String,
    /// Title slug, or null when unavailable.
    pub title_slug: Option<String>,
    /// Media facet containing the title.
    pub title_facet: MediaFacetValue,
    /// Library ID containing the title.
    pub library_id: ID,
    /// Library name, or null when unavailable.
    pub library_name: Option<String>,
    /// Library slug, or null when unavailable.
    pub library_slug: Option<String>,
    /// Episode ID for episodic targets, or null for title-level targets.
    pub episode_id: Option<ID>,
    /// Season number as parsed text, or null when unavailable.
    pub season_number: Option<String>,
    /// Episode number as parsed text, or null when unavailable.
    pub episode_number: Option<String>,
    /// Current quality tier.
    pub current_tier: String,
    /// Required target quality tier.
    pub target_tier: String,
    /// Convergence state for this upgrade scope.
    pub convergence_state: ConvergenceStateValue,
    /// Number of indexers covering the target.
    pub indexers_covered: i32,
    /// Number of indexers selected by routing.
    pub indexers_routed: i32,
}

#[derive(SimpleObject, Clone)]
/// One page of cutoff-unmet targets plus the full matching count.
pub struct CutoffUnmetTitlesPagePayload {
    /// Items in the requested page.
    pub items: Vec<CutoffUnmetItemPayload>,
    /// Total matching targets before pagination.
    pub total_count: i64,
    /// Whether more matching targets exist after this page.
    pub has_more: bool,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after pausing a wanted state row or convergence scope.
pub struct PauseWantedItemPayload {
    /// State-row ID or convergence scope key that was paused.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after resuming a wanted state row or convergence scope.
pub struct ResumeWantedItemPayload {
    /// State-row ID or convergence scope key that was resumed.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Result of queuing title-mismatch recovery searches.
pub struct TriggerTitleMismatchRecoverySearchPayload {
    /// Title ID searched.
    pub title_id: ID,
    /// Number of recovery searches accepted for background processing.
    pub queued_count: i32,
}

/// Lifecycle state of a background acquisition search.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum AcquisitionSearchJobStateValue {
    /// Search is still running.
    Running,
    /// Search completed successfully.
    Completed,
    /// Search was canceled before completion.
    Cancelled,
    /// Search failed.
    Failed,
}

#[derive(SimpleObject, Clone)]
/// Progress snapshot for a background acquisition search.
pub struct AcquisitionSearchJobPayload {
    /// Acquisition-search job ID.
    pub id: ID,
    /// Current job lifecycle state.
    pub state: AcquisitionSearchJobStateValue,
    /// Total search targets.
    pub total: i32,
    /// Number of targets processed.
    pub processed: i32,
    /// Number of releases grabbed.
    pub grabbed_count: i32,
    /// Number of target searches that failed.
    pub failed_count: i32,
    /// Title currently being processed, or null when idle or complete.
    pub current_title: Option<String>,
    /// UTC job start time.
    pub started_at: DateTime<Utc>,
    /// UTC completion time, or null while running.
    pub finished_at: Option<DateTime<Utc>>,
}

/// Lifecycle state of a background interactive release search.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum InteractiveReleaseSearchStateValue {
    /// Search is still running.
    Running,
    /// Search completed and its snapshot is final.
    Completed,
    /// Search was canceled before completion.
    Cancelled,
}

/// Per-indexer progress within an interactive release search.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum InteractiveReleaseSearchIndexerStatusValue {
    /// Indexer has not started.
    Pending,
    /// Indexer is being queried.
    Searching,
    /// Indexer returned successfully.
    Completed,
    /// Indexer query failed.
    Failed,
    /// Indexer was excluded from this search.
    Skipped,
}

#[derive(SimpleObject, Clone)]
/// Per-indexer progress and result count for an interactive release search.
pub struct InteractiveReleaseSearchIndexerPayload {
    /// Indexer configuration ID.
    pub indexer_id: ID,
    /// Indexer name.
    pub name: String,
    /// Current indexer lifecycle state.
    pub status: InteractiveReleaseSearchIndexerStatusValue,
    /// The indexer's own result count (before cross-indexer dedup).
    pub result_count: i32,
    /// Failure reason, or null when the indexer did not fail.
    pub failure_reason: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Pollable snapshot of an interactive release-search job with partial results.
pub struct InteractiveReleaseSearchPayload {
    /// Interactive release-search job ID.
    pub id: ID,
    /// Current search lifecycle state.
    pub state: InteractiveReleaseSearchStateValue,
    /// Scored, cross-indexer-deduped snapshot of the merged results so far.
    pub results: Vec<IndexerSearchResultPayload>,
    /// Per-indexer progress states.
    pub indexers: Vec<InteractiveReleaseSearchIndexerPayload>,
    /// UTC search start time.
    pub started_at: DateTime<Utc>,
    /// UTC completion time, or null while running.
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting cancellation for an interactive release search.
pub struct CancelInteractiveReleaseSearchPayload {
    /// Interactive search job ID.
    pub id: ID,
    /// True when cancellation was accepted; false when the search was already terminal.
    pub accepted: bool,
}

// ── Rule Sets ──────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Rego rule set configuration and managed-pack metadata.
pub struct RuleSetPayload {
    /// Rule-set ID.
    pub id: ID,
    /// Rule-set name.
    pub name: String,
    /// Rule-set description.
    pub description: String,
    /// Rego source used for validation and evaluation.
    pub rego_source: String,
    /// Whether the rule set is enabled.
    pub enabled: bool,
    /// Evaluation priority.
    pub priority: i32,
    /// Media facets to which the rule set applies.
    pub applied_facets: Vec<String>,
    /// Whether the rule set is managed by a trusted pack.
    pub is_managed: bool,
    /// Managed-pack key, or null for user-authored rules.
    pub managed_key: Option<String>,
    /// Tags a managed pack is narrowed to. Null means it applies wherever its
    /// facts match. Always null for user-authored rule sets.
    pub managed_tag_filter: Option<Vec<String>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after deleting a rule set.
pub struct DeleteRuleSetPayload {
    /// Deleted rule-set ID.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Result of validating Rego source.
pub struct RuleValidationResultPayload {
    /// Whether the source is valid.
    pub valid: bool,
    /// Validation errors; empty when valid.
    pub errors: Vec<String>,
}

#[derive(InputObject)]
/// Creates a user-authored Rego rule set.
pub struct CreateRuleSetInput {
    /// Rule-set name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Complete Rego module evaluated for this rule set.
    pub rego_source: String,
    /// Optional media facets.
    pub applied_facets: Option<Vec<String>>,
    /// Optional evaluation priority.
    pub priority: Option<i32>,
    /// Optional enabled state.
    pub enabled: Option<bool>,
}

#[derive(InputObject)]
/// Patches a rule set while preserving omitted values.
pub struct UpdateRuleSetInput {
    /// Rule-set ID.
    pub id: ID,
    /// Replacement name, or null to preserve.
    pub name: Option<String>,
    /// Replacement description, or null to preserve.
    pub description: Option<String>,
    /// Replacement Rego source, or null to preserve.
    pub rego_source: Option<String>,
    /// Replacement facet list, or null to preserve.
    pub applied_facets: Option<Vec<String>>,
    /// Replacement priority, or null to preserve.
    pub priority: Option<i32>,
    /// Narrow a managed locale pack to titles carrying one of these tags. An
    /// empty list clears the filter so the pack applies wherever its facts
    /// match. Rejected for user-authored rule sets.
    pub managed_tag_filter: Option<Vec<String>>,
}

#[derive(InputObject)]
/// Enables or disables one rule set.
pub struct ToggleRuleSetInput {
    /// Rule-set ID.
    pub id: ID,
    /// Desired enabled state.
    pub enabled: bool,
}

#[derive(InputObject)]
/// Validates Rego source, optionally in the context of an existing rule set.
pub struct ValidateRuleSetInput {
    /// Rego source to validate.
    pub rego_source: String,
    /// Existing rule-set ID for context, or null for standalone validation.
    pub rule_set_id: Option<ID>,
}

#[derive(InputObject)]
/// Sets a title-level required-audio-language override.
pub struct SetTitleRequiredAudioInput {
    /// Target title ID.
    pub title_id: ID,
    /// The facet of the title: "movie", "series", or "anime"
    pub facet: MediaFacetValue,
    /// `null` removes the override and inherits from the facet.
    /// `[]` stores an explicit "no required languages" override for the title.
    pub languages: Option<Vec<String>>,
}

#[derive(SimpleObject, Clone)]
/// Result of setting a title's required-audio-language override.
pub struct SetTitleRequiredAudioPayload {
    /// Target title ID.
    pub title_id: ID,
    /// Title media facet.
    pub facet: MediaFacetValue,
    /// Effective override languages; null means inherited behavior.
    pub languages: Option<Vec<String>>,
    /// Whether the stored value changed.
    pub updated: bool,
}

#[derive(SimpleObject, Clone)]
/// Snapshot of the in-memory service log buffer.
pub struct ServiceLogsPayload {
    /// UTC time when the snapshot was generated.
    pub generated_at: DateTime<Utc>,
    /// Log lines returned, newest or oldest order as defined by the service buffer.
    pub lines: Vec<String>,
    /// Number of returned lines.
    pub count: i32,
}

// ── Metadata Gateway (proxied from SMG) ────────────────────────────────────

#[derive(InputObject, Clone)]
/// Metadata gateway movie lookup by provider ID and language.
pub struct MetadataMovieInput {
    /// TVDB movie ID.
    pub tvdb_id: String,
    /// Optional metadata language code.
    pub language: Option<String>,
}

#[derive(InputObject, Clone)]
/// Metadata gateway series lookup by provider ID and language.
pub struct MetadataSeriesInput {
    /// TVDB series ID.
    pub tvdb_id: String,
    /// Whether episode metadata should be included; omitted uses the service default.
    pub include_episodes: Option<bool>,
    /// Optional metadata language code.
    pub language: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Search result from the metadata gateway with nullable provider metadata.
pub struct MetadataSearchItemPayload {
    /// TVDB provider ID.
    pub tvdb_id: String,
    /// Metadata title.
    pub name: String,
    /// IMDb ID, or null when unavailable.
    pub imdb_id: Option<String>,
    /// Provider slug, or null when unavailable.
    pub slug: Option<String>,
    #[graphql(name = "type")]
    /// Provider content-type hint, or null when unavailable.
    pub type_hint: Option<String>,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Provider status, or null when unavailable.
    pub status: Option<String>,
    /// Overview text, or null when unavailable.
    pub overview: Option<String>,
    /// Popularity score, or null when unavailable.
    pub popularity: Option<f64>,
    /// Poster URL, or null when unavailable.
    pub poster_url: Option<String>,
    /// Metadata language, or null when unavailable.
    pub language: Option<String>,
    /// Runtime in minutes, or null when unavailable.
    pub runtime_minutes: Option<i32>,
    /// Normalized sort title, or null when unavailable.
    pub sort_title: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Metadata search results grouped by content facet.
pub struct MetadataSearchMultiPayload {
    /// Movie results; empty when no movies matched.
    pub movies: Vec<MetadataSearchItemPayload>,
    /// Series results; empty when no series matched.
    pub series: Vec<MetadataSearchItemPayload>,
    /// Anime results; empty when no anime matched.
    pub anime: Vec<MetadataSearchItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Full metadata gateway movie record.
pub struct MetadataMoviePayload {
    /// TVDB movie ID.
    pub tvdb_id: String,
    /// Movie title.
    pub name: String,
    /// Provider slug.
    pub slug: String,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Provider status.
    pub status: String,
    /// Overview text.
    pub overview: String,
    /// Metadata-provider URL for the poster image.
    pub poster_url: String,
    /// Metadata language code.
    pub language: String,
    /// Runtime in minutes.
    pub runtime_minutes: i32,
    /// Normalized sort title.
    pub sort_title: String,
    /// IMDb title identifier.
    pub imdb_id: String,
    /// Studio name.
    pub studio: String,
    /// TMDB release date, or null when unavailable.
    pub tmdb_release_date: Option<Date>,
}

#[derive(SimpleObject, Clone)]
/// Full metadata gateway series record with seasons and optional episodes.
pub struct MetadataSeriesPayload {
    /// TVDB series ID.
    pub tvdb_id: String,
    /// Series title.
    pub name: String,
    /// Normalized sort name.
    pub sort_name: String,
    /// Provider slug.
    pub slug: String,
    /// First release year, or null when unavailable.
    pub year: Option<i32>,
    /// Provider status.
    pub status: String,
    /// First-air date.
    pub first_aired: Date,
    /// Overview text.
    pub overview: String,
    /// Network name.
    pub network: String,
    /// Runtime in minutes.
    pub runtime_minutes: i32,
    /// Metadata-provider URL for the poster image.
    pub poster_url: String,
    /// Country code.
    pub country: String,
    /// Alternate titles.
    pub aliases: Vec<String>,
    /// Season metadata.
    pub seasons: Vec<MetadataSeasonPayload>,
    /// Episode metadata; empty when not requested or unavailable.
    pub episodes: Vec<MetadataEpisodePayload>,
}

#[derive(SimpleObject, Clone)]
/// Metadata gateway season record.
pub struct MetadataSeasonPayload {
    /// TVDB season ID.
    pub tvdb_id: String,
    /// Numeric season number assigned by the metadata provider.
    pub number: i32,
    /// Season label.
    pub label: String,
    /// Episode classification.
    pub episode_type: String,
}

#[derive(SimpleObject, Clone)]
/// Metadata gateway episode record.
pub struct MetadataEpisodePayload {
    /// TVDB episode ID.
    pub tvdb_id: String,
    /// Episode number within the season.
    pub episode_number: i32,
    /// Numeric season containing this episode.
    pub season_number: i32,
    /// Episode title.
    pub name: String,
    /// Original air date.
    pub aired: Date,
    /// Runtime in minutes.
    pub runtime_minutes: i32,
    /// Whether the episode is marked filler.
    pub is_filler: bool,
    /// Episode image URL.
    pub image_url: String,
}

#[derive(SimpleObject, Clone)]
/// Availability summary for an episode's primary media.
pub struct EpisodeMediaAvailabilityPayload {
    /// Current availability or scan state.
    pub state: EpisodeMediaAvailabilityStateValue,
    /// Quality label of the primary file, or null before a file is available.
    pub primary_quality_label: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// States used to describe an episode's media availability.
pub enum EpisodeMediaAvailabilityStateValue {
    /// A playable primary media file is available.
    Available,
    /// A library scan has not yet completed for the episode.
    PendingScan,
    /// The latest media scan failed.
    ScanFailed,
    /// No media file currently satisfies the episode requirements.
    Missing,
    /// The episode is not monitored and is excluded from acquisition.
    Unmonitored,
}

#[derive(SimpleObject, Clone)]
/// Calendar episode with title, library, monitoring, and air-date context.
pub struct CalendarEpisodePayload {
    /// Episode ID.
    pub id: ID,
    /// Parent title ID.
    pub title_id: ID,
    /// ID of the library containing the parent title.
    pub library_id: ID,
    /// Library name, or null when unavailable.
    pub library_name: Option<String>,
    /// Library slug, or null when unavailable.
    pub library_slug: Option<String>,
    /// Display name of the parent title.
    pub title_name: String,
    /// Title slug, or null when unavailable.
    pub title_slug: Option<String>,
    /// Title facet string.
    pub title_facet: String,
    /// Season number text, or null when unavailable.
    pub season_number: Option<String>,
    /// Episode number text, or null when unavailable.
    pub episode_number: Option<String>,
    /// Episode title, or null when unavailable.
    pub episode_title: Option<String>,
    /// Movie or episode overview, or null when unavailable.
    pub overview: Option<String>,
    /// Proxied movie poster or episode-still URL.
    pub image_url: Option<String>,
    /// Air date, or null when unavailable.
    pub air_date: Option<Date>,
    /// Whether both the episode and its parent title are monitored.
    pub monitored: bool,
    /// Compact availability derived from the episode's primary media file.
    pub media_availability: EpisodeMediaAvailabilityPayload,
}

// ── Plugins ────────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Plugin registry entry with trust, installation, update, and progress state.
pub struct RegistryPluginPayload {
    /// Registry plugin ID.
    pub id: ID,
    /// Plugin name.
    pub name: String,
    /// Plugin description.
    pub description: String,
    /// Registry version.
    pub version: String,
    /// Latest known version, or null when unavailable.
    pub latest_version: Option<String>,
    /// Registry classification of the plugin.
    pub plugin_type: String,
    /// Provider type exposed by the plugin.
    pub provider_type: String,
    /// Publisher or author name.
    pub author: String,
    /// Whether the plugin is official.
    pub official: bool,
    /// Publisher identity, or null when unavailable.
    pub publisher: Option<String>,
    /// Trust or support tier.
    pub support_tier: String,
    /// Current registry or installation status, or null when unavailable.
    pub status: Option<String>,
    /// Documentation URL, or null when unavailable.
    pub docs_url: Option<String>,
    /// Source repository URL, or null when unavailable.
    pub source_repo: Option<String>,
    /// Whether the plugin is built into the service.
    pub builtin: bool,
    /// Download source URL, or null when unavailable.
    pub source_url: Option<String>,
    /// Source kind, or null when unavailable.
    pub source_kind: Option<String>,
    /// Trust-block reason, or null when not blocked.
    pub blocked_reason: Option<String>,
    /// Artifact size in bytes, or null when unavailable.
    pub bytes: Option<Long>,
    /// Whether the plugin is installed.
    pub is_installed: bool,
    /// Whether the installed plugin is enabled.
    pub is_enabled: bool,
    /// Installed version, or null when not installed.
    pub installed_version: Option<String>,
    /// Whether a newer version is available.
    pub update_available: bool,
    /// Whether install or update work is currently running.
    pub install_in_progress: bool,
    /// Default provider base URL, or null when not applicable.
    pub default_base_url: Option<String>,
}

// ── Rule Packs ────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Rule-pack registry entry.
pub struct RulePackRegistryEntryPayload {
    /// Registry rule-pack ID.
    pub id: String,
    /// Rule-pack name.
    pub name: String,
    /// Rule-pack description.
    pub description: String,
    /// Rule-pack author.
    pub author: String,
    /// Rule-pack version.
    pub version: String,
}

#[derive(SimpleObject, Clone)]
/// Template supplied by a rule pack.
pub struct RulePackTemplatePayload {
    /// Template ID.
    pub id: String,
    /// Template title.
    pub title: String,
    /// Template description.
    pub description: String,
    /// Template category.
    pub category: String,
    /// Rego source for the template.
    pub rego_source: String,
    /// Facets to which the template applies.
    pub applied_facets: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// Installed plugin identity, trust metadata, artifact digests, and timestamps.
pub struct PluginInstallationPayload {
    /// Installation record ID.
    pub id: ID,
    /// Registry plugin ID.
    pub plugin_id: ID,
    /// Installed plugin name.
    pub name: String,
    /// Installed plugin description.
    pub description: String,
    /// Installed plugin version.
    pub version: String,
    /// Plugin SDK version.
    pub sdk_version: String,
    /// SDK compatibility constraint.
    pub sdk_constraint: String,
    /// Manifest classification of the installed plugin.
    pub plugin_type: String,
    /// Provider type exposed by the plugin.
    pub provider_type: String,
    /// Whether the plugin is enabled.
    pub is_enabled: bool,
    /// Whether the plugin is built in.
    pub is_builtin: bool,
    /// Installation source kind.
    pub source_kind: String,
    /// Installation source URL, or null when unavailable.
    pub source_url: Option<String>,
    /// Publisher identity, or null when unavailable.
    pub publisher: Option<String>,
    /// Trust or support tier.
    pub support_tier: String,
    /// Documentation URL, or null when unavailable.
    pub docs_url: Option<String>,
    /// Source repository URL, or null when unavailable.
    pub source_repo: Option<String>,
    /// Manifest URL, or null when unavailable.
    pub manifest_url: Option<String>,
    /// Verified WASM digest, or null when unavailable.
    pub wasm_digest: Option<String>,
    /// Verified artifact digest, or null when unavailable.
    pub artifact_digest: Option<String>,
    /// UTC installation time.
    pub installed_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject)]
/// Plugin catalog refresh state, trust warnings, and blocked actions.
pub struct PluginCatalogStatusPayload {
    /// Catalog refresh lifecycle state.
    pub refresh_state: CatalogRefreshStateValue,
    /// Whether the remote catalog source is reachable.
    pub github_available: bool,
    /// UTC time of the last catalog check, or null when never checked.
    pub last_checked_at: Option<DateTime<Utc>>,
    /// Current outage message, or null when no outage is reported.
    pub outage_message: Option<String>,
    /// Actions blocked by trust or catalog state.
    pub blocked_actions: Vec<String>,
    /// Restore warnings that require operator attention.
    pub restore_warnings: Vec<String>,
    /// Last catalog error, or null when none is recorded.
    pub last_error: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Background plugin installation operation kind.
pub enum PluginInstallOperationKindValue {
    /// New plugin installation.
    Install,
    /// Existing plugin upgrade.
    Upgrade,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Lifecycle state of a plugin installation operation.
pub enum PluginInstallStateValue {
    /// Artifact is downloading.
    Downloading,
    /// Artifact signatures or digests are being verified.
    Verifying,
    /// Artifact is being installed.
    Installing,
    /// Installation completed successfully.
    Succeeded,
    /// Installation failed.
    Failed,
}

#[derive(SimpleObject, Clone)]
/// Progress snapshot for a background plugin installation or upgrade.
pub struct PluginInstallProgressPayload {
    /// Plugin ID being installed or upgraded.
    pub plugin_id: ID,
    /// Installation operation kind.
    pub operation_kind: PluginInstallOperationKindValue,
    /// Current installation state.
    pub state: PluginInstallStateValue,
    /// Current progress label.
    pub label: String,
    /// Current step number, from 1 through the total step count.
    pub step_index: i32,
    /// Total step count.
    pub step_count: i32,
    /// Progress message, or null when none is available.
    pub message: Option<String>,
    /// Error message, or null when the operation has not failed.
    pub error: Option<String>,
}

#[derive(SimpleObject)]
/// Preview of a manually supplied plugin repository.
pub struct ManualPluginPreviewPayload {
    /// GitHub repository URL used for the preview.
    pub github_repo_url: String,
    /// Resolved registry metadata.
    pub plugin: RegistryPluginPayload,
}

#[derive(InputObject)]
/// GitHub repository source for a manual plugin preview or install.
pub struct ManualPluginRepoInput {
    /// GitHub repository URL.
    pub github_repo_url: String,
}

#[derive(InputObject)]
/// Uploaded plugin artifact and explicit risk acknowledgement.
pub struct ManualPluginUploadInput {
    /// Uploaded file name.
    pub file_name: String,
    /// Base64-encoded WASM artifact.
    pub wasm_base64: String,
    /// Must be true to acknowledge manual artifact risk.
    pub acknowledge_risk: bool,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after uninstalling a plugin.
pub struct UninstallPluginPayload {
    /// Uninstalled plugin ID.
    pub plugin_id: async_graphql::ID,
}

#[derive(InputObject)]
/// Enables or disables one installed plugin.
pub struct TogglePluginInput {
    /// ID of the installed plugin to enable or disable.
    pub plugin_id: ID,
    /// Desired enabled state.
    pub enabled: bool,
}

// ── Provider Type Config Schema ─────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// One selectable value for a plugin configuration field.
pub struct PluginConfigFieldOptionPayload {
    /// Machine-readable option value.
    pub value: String,
    /// Display label for the option.
    pub label: String,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Plugin configuration field rendering and validation type.
pub enum PluginConfigFieldTypeValue {
    /// Single-line string.
    String,
    /// Secret or password string.
    Password,
    /// Multiline text.
    Multiline,
    /// Boolean value.
    Bool,
    /// Enumerated selection.
    Select,
    /// Numeric value.
    Number,
    /// Filesystem path.
    Path,
    /// Tag value.
    Tag,
}

impl PluginConfigFieldTypeValue {
    pub fn from_domain(value: ConfigFieldType) -> Self {
        match value {
            ConfigFieldType::String => Self::String,
            ConfigFieldType::Password => Self::Password,
            ConfigFieldType::Multiline => Self::Multiline,
            ConfigFieldType::Bool => Self::Bool,
            ConfigFieldType::Select => Self::Select,
            ConfigFieldType::Number => Self::Number,
            ConfigFieldType::Path => Self::Path,
            ConfigFieldType::Tag => Self::Tag,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Source of a provider configuration value.
pub enum PluginConfigValueSourceValue {
    /// Value supplied by the user.
    User,
    /// Value supplied by a host binding.
    HostBinding,
}

impl PluginConfigValueSourceValue {
    pub fn from_domain(value: ConfigFieldValueSource) -> Self {
        match value {
            ConfigFieldValueSource::User => Self::User,
            ConfigFieldValueSource::HostBinding => Self::HostBinding,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Semantic role assigned to a provider configuration field.
pub enum PluginConfigFieldRoleValue {
    /// Field contains a connection URL.
    ConnectionUrl,
}

impl PluginConfigFieldRoleValue {
    pub fn from_domain(value: ConfigFieldRole) -> Self {
        match value {
            ConfigFieldRole::ConnectionUrl => Self::ConnectionUrl,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Provider configuration field schema.
pub struct PluginConfigFieldPayload {
    /// Stable field key.
    pub key: String,
    /// Human-readable field label.
    pub label: String,
    /// Value type that controls field validation and presentation.
    pub field_type: PluginConfigFieldTypeValue,
    /// Whether a value is required.
    pub required: bool,
    /// Default value, or null when none is defined.
    pub default_value: Option<String>,
    /// Source of the effective value.
    pub value_source: PluginConfigValueSourceValue,
    /// Optional semantic role.
    pub role: Option<PluginConfigFieldRoleValue>,
    /// Host binding name, or null when not applicable.
    pub host_binding: Option<String>,
    /// Enumerated options, empty for non-select fields.
    pub options: Vec<PluginConfigFieldOptionPayload>,
    /// Help text, or null when unavailable.
    pub help_text: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Provider type metadata and configuration schema.
pub struct ProviderTypePayload {
    /// Provider implementation identifier.
    pub provider_type: String,
    /// Provider display name.
    pub name: String,
    /// Configuration field schemas.
    pub config_fields: Vec<PluginConfigFieldPayload>,
    /// Default base URL, or null when not applicable.
    pub default_base_url: Option<String>,
    /// Host binding names accepted as configuration value sources.
    pub available_host_bindings: Vec<String>,
    /// Recommended media facets.
    pub recommended_facets: Vec<MediaFacetValue>,
    /// Supported notification events.
    pub supported_events: Vec<String>,
    /// Whether a connection test is supported.
    pub supports_test: bool,
}

#[derive(SimpleObject, Clone)]
/// Provider connection validation result.
pub struct ProviderValidationPayload {
    /// Machine-readable validation status.
    pub status: String,
    /// Diagnostic message, or null when unavailable.
    pub message: Option<String>,
    /// Retry delay in seconds, or null when no retry delay applies.
    pub retry_after_seconds: Option<i64>,
}

// ── Notification types ─────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Configured notification channel and its redacted provider settings.
pub struct NotificationChannelPayload {
    /// Notification channel ID.
    pub id: ID,
    /// Channel display name.
    pub name: String,
    /// Provider channel type.
    pub channel_type: String,
    /// Non-secret channel configuration values.
    pub config: Vec<ProviderConfigValuePayload>,
    /// Configuration keys whose secret values are stored but not returned.
    pub stored_secret_keys: Vec<String>,
    /// Media-server connection used by this channel, when applicable.
    pub media_server_connection_id: Option<ID>,
    /// Whether notifications are enabled for this channel.
    pub is_enabled: bool,
    /// Channel creation time in UTC.
    pub created_at: DateTime<Utc>,
    /// Time of the latest channel update in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Subscription connecting a notification channel to an event target and scope.
pub struct NotificationSubscriptionPayload {
    /// Subscription ID.
    pub id: ID,
    /// Notification channel ID, or null when the subscription has no channel.
    pub channel_id: Option<ID>,
    /// Target category for the subscription.
    pub target_kind: String,
    /// ID of the subscribed target.
    pub target_id: ID,
    /// Event type that triggers delivery.
    pub event_type: String,
    /// Scope category used to limit matching events.
    pub scope: String,
    /// Scope key, or null when the scope is global.
    pub scope_id: Option<String>,
    /// Whether this subscription is enabled.
    pub is_enabled: bool,
    /// Subscription creation time in UTC.
    pub created_at: DateTime<Utc>,
    /// Time of the latest subscription update in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Identifier of a deleted notification channel.
pub struct DeleteNotificationChannelPayload {
    /// Deleted notification channel ID.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Result of testing a notification channel configuration.
pub struct NotificationChannelTestPayload {
    /// Tested notification channel ID.
    pub id: async_graphql::ID,
    /// Test outcome status.
    pub status: String,
    /// Provider response or failure detail, when available.
    pub message: Option<String>,
    /// Suggested retry delay in seconds, when the provider requests one.
    pub retry_after_seconds: Option<i64>,
}

#[derive(SimpleObject, Clone)]
/// Identifier of a deleted notification subscription.
pub struct DeleteNotificationSubscriptionPayload {
    /// Deleted notification subscription ID.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Enabled notification target available for subscription.
pub struct NotificationTargetPayload {
    /// Target ID.
    pub id: ID,
    /// Target category.
    pub target_kind: String,
    /// Target display name.
    pub name: String,
    /// Provider type associated with the target.
    pub provider_type: String,
    /// Media-server provider, when the target is a media-server connection.
    pub media_server_provider: Option<MediaServerProviderValue>,
    /// Media-server connection ID, when applicable.
    pub media_server_connection_id: Option<ID>,
    /// Whether this target can receive notifications.
    pub is_enabled: bool,
}

#[derive(InputObject)]
/// Values required to create a notification channel.
pub struct CreateNotificationChannelInput {
    /// Channel display name.
    pub name: String,
    /// Provider channel type.
    pub channel_type: String,
    /// Channel configuration values; secret values are stored securely.
    pub config: Vec<ProviderConfigValueInput>,
    /// Media-server connection ID for channel providers that use one.
    pub media_server_connection_id: Option<ID>,
    /// Whether the new channel starts enabled; omitted uses the service default.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Values that may be changed on an existing notification channel.
pub struct UpdateNotificationChannelInput {
    /// Notification channel ID to update.
    pub id: ID,
    /// Replacement channel display name, or null to leave it unchanged.
    pub name: Option<String>,
    /// Replacement configuration, or null to leave it unchanged.
    pub config: Option<Vec<ProviderConfigValueInput>>,
    /// Replacement media-server connection; an explicit null clears it.
    pub media_server_connection_id: Option<Option<ID>>,
    /// Replacement enabled state, or null to leave it unchanged.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Values required to subscribe a notification channel to an event target.
pub struct CreateNotificationSubscriptionInput {
    /// Notification channel ID, or null for target-only event handling.
    pub channel_id: Option<ID>,
    /// Target category, or null when the event is not target-specific.
    pub target_kind: Option<String>,
    /// Target ID, or null when the event is not target-specific.
    pub target_id: Option<ID>,
    /// Event type that triggers delivery.
    pub event_type: String,
    /// Scope category used to limit matching events.
    pub scope: String,
    /// Scope key, or null for a global scope.
    pub scope_id: Option<String>,
    /// Whether the new subscription starts enabled; omitted uses the service default.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Values that may be changed on an existing notification subscription.
pub struct UpdateNotificationSubscriptionInput {
    /// Notification subscription ID to update.
    pub id: ID,
    /// Replacement target category, or null to leave it unchanged.
    pub target_kind: Option<String>,
    /// Replacement target ID, or null to leave it unchanged.
    pub target_id: Option<ID>,
    /// Replacement event type, or null to leave it unchanged.
    pub event_type: Option<String>,
    /// Replacement scope category, or null to leave it unchanged.
    pub scope: Option<String>,
    /// Replacement scope key, or null to leave it unchanged.
    pub scope_id: Option<String>,
    /// Replacement enabled state, or null to leave it unchanged.
    pub is_enabled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Notification provider type and the fields accepted in its configuration.
pub struct NotificationProviderTypePayload {
    /// Stable notification provider implementation key.
    pub provider_type: String,
    /// Provider display name.
    pub name: String,
    /// Configuration field definitions for this provider.
    pub config_fields: Vec<PluginConfigFieldPayload>,
}

#[derive(SimpleObject, Clone)]
/// Row count for one table in a backup.
pub struct BackupRowCountPayload {
    /// Table name.
    pub table: String,
    /// Number of rows copied from the table.
    pub row_count: Long,
}

#[derive(SimpleObject, Clone)]
/// Metadata and lifecycle result for one backup file.
pub struct BackupInfoPayload {
    /// Backup filename.
    pub filename: String,
    /// Backup size in bytes.
    pub size_bytes: Long,
    /// Backup creation time in UTC.
    pub created_at: DateTime<Utc>,
    /// Backup format version.
    pub format_version: String,
    /// Database engine that produced the backup.
    pub source_engine: String,
    /// Source migration key, when recorded.
    pub source_migration_key: Option<String>,
    /// Whether the backup is encrypted.
    pub encrypted: bool,
    /// Counts of rows included by table.
    pub row_counts: Vec<BackupRowCountPayload>,
    /// Operation that created the backup.
    pub trigger: String,
    /// Backup lifecycle status.
    pub status: String,
    /// Failure detail, or null when the operation succeeded.
    pub error_message: Option<String>,
}

#[derive(InputObject)]
/// Password used to encrypt a new backup.
pub struct CreateBackupInput {
    /// Encryption password; it is not returned in backup metadata.
    pub password: String,
}

#[derive(InputObject)]
/// Backup filename to prepare for download.
pub struct PrepareBackupDownloadInput {
    /// Existing backup filename.
    pub filename: String,
}

#[derive(InputObject)]
/// Backup filename to remove.
pub struct DeleteBackupInput {
    /// Existing backup filename.
    pub filename: String,
}

#[derive(SimpleObject, Clone)]
/// Result of attempting to delete a backup file.
pub struct DeleteBackupPayload {
    /// Filename targeted by the deletion.
    pub filename: String,
    /// False when no backup file with that name existed.
    pub deleted: bool,
}

#[derive(SimpleObject, Clone)]
/// Counts produced by one RSS synchronization pass.
pub struct RssSyncReportPayload {
    /// Number of releases fetched from feeds.
    pub releases_fetched: i32,
    /// Number of fetched releases matched to known titles.
    pub releases_matched: i32,
    /// Number of matched releases accepted for grabbing.
    pub releases_grabbed: i32,
    /// Number of matched releases held instead of grabbed.
    pub releases_held: i32,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting an immediate grab for a pending release.
pub struct ForceGrabPendingReleasePayload {
    /// Pending release ID.
    pub id: async_graphql::ID,
    /// Whether the release was accepted for grabbing.
    pub grabbed: bool,
}

#[derive(SimpleObject, Clone)]
/// Identifier of a dismissed pending release.
pub struct DismissPendingReleasePayload {
    /// Pending release ID.
    pub id: async_graphql::ID,
}

// ── Recycle Bin ────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// File moved to the recycle bin with its original library context.
pub struct RecycledItemPayload {
    /// Recycle-bin entry ID.
    pub id: async_graphql::ID,
    /// Original absolute or library-relative file path.
    pub original_path: String,
    /// Original file name.
    pub file_name: String,
    /// File size in bytes.
    pub size_bytes: Long,
    /// Associated title ID, or null when no title was matched.
    pub title_id: Option<async_graphql::ID>,
    /// Associated title name, or null when no title was matched.
    pub title_name: Option<String>,
    /// Reason the file entered the recycle bin.
    pub reason: String,
    /// Time the file entered the recycle bin in UTC.
    pub recycled_at: DateTime<Utc>,
    /// Media root containing the original file.
    pub media_root: String,
    /// Library ID containing the original file.
    pub library_id: async_graphql::ID,
    /// Library name containing the original file.
    pub library_name: String,
}

#[derive(SimpleObject, Clone)]
/// A page of recycle-bin entries and its total matching count.
pub struct RecycledItemsPayload {
    /// Recycle-bin entries in the requested page.
    pub items: Vec<RecycledItemPayload>,
    /// Total matching entries across all pages.
    pub total_count: i32,
}

#[derive(SimpleObject, Clone)]
/// Accepted request to restore one recycle-bin entry.
pub struct RestoreRecycledItemPayload {
    /// Recycle-bin entry ID.
    pub id: async_graphql::ID,
    /// Background job accepted for the restore operation.
    pub job_run: JobRunPayload,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Behavior when a restored file already occupies its destination path.
pub enum RecycleRestoreConflictPolicyValue {
    /// Preserve both files using a distinct destination name.
    KeepBoth,
    /// Replace the existing destination file.
    ReplaceExisting,
}

#[derive(InputObject)]
/// Recycle-bin entries and the preview version to restore.
pub struct RestoreRecycledItemsInput {
    /// Recycle-bin entry IDs to restore.
    pub ids: Vec<async_graphql::ID>,
    /// Conflict behavior for occupied destination paths.
    pub conflict_policy: RecycleRestoreConflictPolicyValue,
    /// Fingerprint returned by the current restore preview.
    pub preview_fingerprint: String,
}

#[derive(InputObject)]
/// Recycle-bin entries to delete permanently.
pub struct DeleteRecycledItemsInput {
    /// Recycle-bin entry IDs to delete.
    pub ids: Vec<async_graphql::ID>,
}

#[derive(SimpleObject, Clone)]
/// One destination collision found during restore preview.
pub struct RecycleRestorePreviewItemPayload {
    /// Recycle-bin entry ID.
    pub id: async_graphql::ID,
    /// Original path that would be restored.
    pub original_path: String,
    /// Whether the destination is currently occupied.
    pub destination_occupied: bool,
}

#[derive(SimpleObject, Clone)]
/// Restore preview fingerprint and destination collision details.
pub struct RecycleRestorePreviewPayload {
    /// Fingerprint required to confirm the current preview.
    pub fingerprint: String,
    /// Preview entries for the selected recycle-bin records.
    pub items: Vec<RecycleRestorePreviewItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Accepted background restore for multiple recycle-bin entries.
pub struct RestoreRecycledItemsPayload {
    /// Recycle-bin entry IDs accepted for restore.
    pub ids: Vec<async_graphql::ID>,
    /// Background job accepted for the restore operation.
    pub job_run: JobRunPayload,
}

#[derive(SimpleObject, Clone)]
/// Accepted background deletion for multiple recycle-bin entries.
pub struct DeleteRecycledItemsPayload {
    /// Recycle-bin entry IDs accepted for deletion.
    pub ids: Vec<async_graphql::ID>,
    /// Background job accepted for the deletion operation.
    pub job_run: JobRunPayload,
}

#[derive(SimpleObject, Clone)]
/// Result of deleting one recycle-bin entry.
pub struct DeleteRecycledItemPayload {
    /// Recycle-bin entry ID.
    pub id: async_graphql::ID,
    /// Whether the recycle-bin record and file were deleted.
    pub deleted: bool,
}

#[derive(SimpleObject, Clone)]
/// Count of files permanently purged from the recycle bin.
pub struct EmptyRecycleBinPayload {
    /// Number of recycle-bin entries purged.
    pub purged_count: i32,
}

#[derive(SimpleObject, Clone)]
/// Result of completing initial application setup.
pub struct CompleteSetupPayload {
    /// Whether setup is now complete.
    pub completed: bool,
}

#[derive(SimpleObject, Clone)]
/// Accepted request to clear cached title images.
pub struct ClearTitleImageCachePayload {
    /// When the cache-clear request was accepted.
    pub requested_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Current setup prerequisites and completion state.
pub struct SetupStatusPayload {
    /// Whether initial setup has been completed.
    pub setup_complete: bool,
    /// Whether at least one download client is configured.
    pub has_download_clients: bool,
    /// Whether at least one indexer is configured.
    pub has_indexers: bool,
}

#[derive(SimpleObject, Clone)]
/// Directory entry returned while browsing a configured path.
pub struct DirectoryEntryPayload {
    /// Entry name.
    pub name: String,
    /// Entry path.
    pub path: String,
    /// Whether the entry is a directory rather than a file.
    pub is_directory: bool,
}

// ── External Import (Sonarr/Radarr) ────────────────────────────────────────

#[derive(InputObject, Clone)]
/// Credentials for connecting to one external service instance.
pub struct ExternalImportConnectionInput {
    /// Base URL of the external service.
    pub base_url: String,
    /// API key used only for the connection operation and secret storage.
    pub api_key: String,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Supported external title-management service source.
pub enum ExternalArrSourceKind {
    /// Sonarr source.
    Sonarr,
    /// Radarr source.
    Radarr,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// External service kind accepted by connection validation and warmup.
pub enum ExternalImportConnectionKind {
    /// Sonarr service.
    Sonarr,
    /// Radarr service.
    Radarr,
    /// Prowlarr service.
    Prowlarr,
}

#[derive(InputObject)]
/// External service kind and credentials to validate.
pub struct ValidateExternalImportConnectionInput {
    /// External service kind.
    pub kind: ExternalImportConnectionKind,
    /// Base URL and API key to validate.
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
/// API key for one external service instance in an import setup draft.
pub struct ExternalImportSetupInstanceApiKeyInput {
    /// External instance ID used to identify the draft entry.
    pub instance_id: ID,
    /// External service kind.
    pub kind: ExternalImportConnectionKind,
    /// API key stored in the draft.
    pub api_key: String,
}

#[derive(InputObject)]
/// Secret values collected for an external import setup.
pub struct SaveExternalImportSetupSecretDraftInput {
    /// External instance API keys keyed by instance ID.
    pub instance_api_keys: Vec<ExternalImportSetupInstanceApiKeyInput>,
    /// Replacement API keys keyed by download-client deduplication key.
    pub download_client_api_key_overrides: Vec<DownloadClientApiKeyOverrideInput>,
    /// Replacement passwords keyed by download-client deduplication key.
    pub download_client_password_overrides: Vec<DownloadClientPasswordOverrideInput>,
    /// Replacement API keys keyed by indexer deduplication key.
    pub indexer_api_key_overrides: Vec<IndexerApiKeyOverrideInput>,
}

#[derive(SimpleObject, Clone)]
/// API key entry returned from an external import secret draft.
pub struct ExternalImportSetupInstanceApiKeyPayload {
    /// External instance ID.
    pub instance_id: ID,
    /// External service kind.
    pub kind: ExternalImportConnectionKind,
    /// Stored API key value for the draft.
    pub api_key: String,
}

#[derive(SimpleObject, Clone)]
/// Download-client or indexer API key stored in an import draft.
pub struct ExternalImportSetupApiKeyOverridePayload {
    /// Stable candidate identity used for deduplication.
    pub dedup_key: String,
    /// Stored API key value for the draft.
    pub api_key: String,
}

#[derive(SimpleObject, Clone)]
/// Download-client password stored in an import draft.
pub struct ExternalImportSetupPasswordOverridePayload {
    /// Stable candidate identity used for deduplication.
    pub dedup_key: String,
    /// Stored password value for the draft.
    pub password: String,
}

#[derive(SimpleObject, Clone)]
/// Secret values held in an external import setup draft.
pub struct ExternalImportSetupSecretDraftPayload {
    /// External instance API keys in the draft.
    pub instance_api_keys: Vec<ExternalImportSetupInstanceApiKeyPayload>,
    /// Download-client API key overrides in the draft.
    pub download_client_api_key_overrides: Vec<ExternalImportSetupApiKeyOverridePayload>,
    /// Download-client password overrides in the draft.
    pub download_client_password_overrides: Vec<ExternalImportSetupPasswordOverridePayload>,
    /// Indexer API key overrides in the draft.
    pub indexer_api_key_overrides: Vec<ExternalImportSetupApiKeyOverridePayload>,
    /// Last update time in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Ownership and existence status for the current user's import secret draft.
pub struct ExternalImportSetupSecretDraftStatusPayload {
    /// Whether a secret draft exists.
    pub has_draft: bool,
    /// Whether the draft belongs to the current user.
    pub owned_by_current_user: bool,
    /// Draft update time in UTC, or null when no draft exists.
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Result of saving an external import secret draft.
pub struct SaveExternalImportSetupSecretDraftPayload {
    /// Whether another user's existing draft was replaced.
    pub overwrote_another_user_draft: bool,
    /// Save time in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Result of clearing the current user's external import secret draft.
pub struct ClearExternalImportSetupSecretDraftPayload {
    /// False when there was no draft owned by the caller to clear.
    pub cleared: bool,
}

#[derive(InputObject)]
/// External source and credentials for an asynchronous warmup session.
pub struct StartExternalImportArrSourceWarmupInput {
    /// External source kind.
    pub kind: ExternalArrSourceKind,
    /// Connection URL and API key.
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
/// Prowlarr credentials for an asynchronous warmup session.
pub struct StartExternalImportProwlarrWarmupInput {
    /// Connection URL and API key.
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
/// Warmup session IDs whose aggregate progress should be calculated.
pub struct ExternalImportAggregateWarmupProgressInput {
    /// Warmup session IDs whose progress should be aggregated.
    pub source_warmup_session_ids: Vec<ID>,
}

#[derive(InputObject)]
/// Warmup sessions and optional Prowlarr connection used to build an import preview.
pub struct PreviewExternalImportInput {
    /// Source warmup session IDs to include.
    pub source_warmup_session_ids: Vec<ID>,
    /// Prowlarr warmup session ID, when one was started.
    pub prowlarr_warmup_session_id: Option<ID>,
    #[graphql(deprecation = "use prowlarrWarmupSessionId")]
    /// Deprecated direct Prowlarr connection retained for compatibility.
    pub prowlarr: Option<ExternalImportConnectionInput>,
}

#[derive(InputObject)]
/// API key override for a masked download-client candidate.
pub struct DownloadClientApiKeyOverrideInput {
    /// Stable candidate identity used to apply the override.
    pub dedup_key: String,
    /// Replacement API key.
    pub api_key: String,
}

#[derive(InputObject)]
/// Password override for a masked download-client candidate.
pub struct DownloadClientPasswordOverrideInput {
    /// Stable candidate identity used to apply the override.
    pub dedup_key: String,
    /// Replacement password.
    pub password: String,
}

#[derive(InputObject)]
/// API key override for a masked or conflicting indexer candidate.
pub struct IndexerApiKeyOverrideInput {
    /// Stable candidate identity used to apply the override.
    pub dedup_key: String,
    /// Replacement API key.
    pub api_key: String,
}

#[derive(InputObject)]
/// Candidate selections, mappings, and secret overrides for an external import.
pub struct ExecuteExternalImportInput {
    /// Source warmup session IDs whose data is being imported.
    pub source_warmup_session_ids: Vec<ID>,
    /// Optional Prowlarr connection used when no Prowlarr warmup session is available.
    pub prowlarr: Option<ExternalImportConnectionInput>,
    /// Deduplication keys of download clients to create.
    pub selected_download_client_dedup_keys: Vec<String>,
    /// Deduplication keys of indexers to create.
    pub selected_indexer_dedup_keys: Vec<String>,
    /// API key overrides keyed by download-client deduplication key.
    pub download_client_api_key_overrides: Vec<DownloadClientApiKeyOverrideInput>,
    /// Password overrides keyed by download-client deduplication key.
    pub download_client_password_overrides: Vec<DownloadClientPasswordOverrideInput>,
    /// API key overrides keyed by indexer deduplication key.
    pub indexer_api_key_overrides: Vec<IndexerApiKeyOverrideInput>,
}

#[derive(InputObject)]
/// Mapping from an external root folder to a Scryer library facet.
pub struct ExternalImportSourceLibraryMappingInput {
    /// Warmup session that surfaced this root; null identifies a manually added root without a monitored-status snapshot.
    pub source_warmup_session_id: Option<ID>,
    /// External source identity, required when a warmup session is supplied.
    pub source_key: Option<String>,
    /// External source kind, required when a warmup session is supplied.
    pub kind: Option<ExternalArrSourceKind>,
    /// Root path reported by the external source.
    pub arr_root_path: String,
    /// Destination root path in Scryer.
    pub scryer_root_path: String,
    /// Scryer library ID receiving the imported root.
    pub library_id: ID,
    /// Media facet receiving the imported root.
    pub facet: MediaFacetValue,
}

#[derive(InputObject)]
/// Warmup sessions and root mappings to finalize as an external import.
pub struct FinalizeExternalImportInput {
    /// Source warmup session IDs to finalize.
    pub source_warmup_session_ids: Vec<ID>,
    /// Root-to-library mappings selected for import.
    pub mappings: Vec<ExternalImportSourceLibraryMappingInput>,
}

#[derive(SimpleObject, Clone)]
/// Result of canceling an external import monitor warmup.
pub struct CancelExternalImportMonitorWarmupPayload {
    /// Warmup session ID.
    pub session_id: ID,
    /// Whether cancellation changed the session state.
    pub canceled: bool,
}

#[derive(SimpleObject, Clone)]
/// Connection probe result for an external service.
pub struct ExternalImportConnectionValidationPayload {
    /// Service kind that was validated.
    pub kind: ExternalImportConnectionKind,
    /// Normalized service base URL.
    pub base_url: String,
    /// Whether the service accepted the connection credentials.
    pub connected: bool,
    /// Detected service version, or null when unavailable.
    pub version: Option<String>,
    /// Connection failure detail, or null when connected.
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Aggregate title-fetch progress for external source warmups.
pub struct ExternalImportAggregateWarmupProgressPayload {
    /// Aggregate warmup status.
    pub status: ExternalImportMonitorWarmupStatusValue,
    /// Whether the total title count is known.
    pub titles_total_known: bool,
    /// Number of titles fetched.
    pub titles_fetched: i32,
    /// Total titles expected when known.
    pub titles_total: i32,
    /// Failure detail, or null when no error occurred.
    pub error_message: Option<String>,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// External setting inferred or applied during library import.
pub enum ExternalImportLibrarySettingKey {
    /// Whether title and file names are renamed.
    RenameEnabled,
    /// Whether NFO metadata is written during import.
    NfoWriteOnImport,
    /// Whether Plex match metadata is written during import.
    PlexmatchWriteOnImport,
    /// Whether Linux file permissions are set.
    SetPermissionsLinux,
    /// Folder permission mode.
    FolderChmod,
    /// Group assigned to created folders.
    ChownGroup,
    /// Quality profile ID.
    QualityProfileId,
    /// Quality profile IDs used for requests.
    RequestQualityProfileIds,
    /// Whether special episodes are monitored.
    MonitorSpecials,
    /// Title rename template.
    RenameTemplate,
    /// Folder naming template.
    FolderTemplate,
    /// Required audio language codes.
    RequiredAudioLanguages,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Confidence assigned to an inferred external library setting.
pub enum ExternalImportLibrarySettingConfidence {
    /// Strong evidence supports the setting.
    High,
    /// Some evidence supports the setting.
    Medium,
    /// Weak evidence supports the setting.
    Low,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// How an inferred external setting was handled.
pub enum ExternalImportLibrarySettingDisposition {
    /// Applied automatically.
    AutoApplied,
    /// Returned as a suggestion without automatic application.
    Suggested,
    /// Not applied because evidence or compatibility was insufficient.
    Skipped,
}

#[derive(SimpleObject, Clone)]
/// Typed value emitted for an inferred library setting.
pub struct ExternalImportLibrarySettingValuePayload {
    /// Boolean value, or null when the setting is not boolean.
    pub bool_value: Option<bool>,
    /// String value, or null when the setting is not scalar text.
    pub string_value: Option<String>,
    /// String-list value, or null when the setting is not a list.
    pub string_list_value: Option<Vec<String>>,
}

#[derive(SimpleObject, Clone)]
/// Evidence supporting one inferred external library setting.
pub struct ExternalImportLibrarySettingEvidencePayload {
    /// External source identity supplying the evidence.
    pub source_key: String,
    /// External source kind supplying the evidence.
    pub source_kind: ExternalArrSourceKind,
    /// Number of matching source records.
    pub matching_count: i32,
    /// Number of source records considered.
    pub total_count: i32,
    /// Human-readable evidence detail, when available.
    pub detail: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// One setting application decision for a Scryer library facet.
pub struct ExternalImportLibrarySettingApplicationPayload {
    /// Scryer library ID receiving the setting.
    pub library_id: ID,
    /// Media facet receiving the setting.
    pub facet: MediaFacetValue,
    /// Setting being evaluated.
    pub setting: ExternalImportLibrarySettingKey,
    /// Candidate value for the setting.
    pub value: ExternalImportLibrarySettingValuePayload,
    /// Confidence in the candidate value.
    pub confidence: ExternalImportLibrarySettingConfidence,
    /// Whether the value was applied, suggested, or skipped.
    pub disposition: ExternalImportLibrarySettingDisposition,
    /// Source evidence used for the decision.
    pub evidence: Vec<ExternalImportLibrarySettingEvidencePayload>,
    /// Additional reason, or null when no explanation was recorded.
    pub reason: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Accepted external import finalization and its monitor warmup session.
pub struct FinalizeExternalImportPayload {
    /// Monitor warmup session ID tracking accepted background work.
    pub monitor_warmup_session_id: ID,
}

#[derive(InputObject)]
/// Language used when clearing metadata for a full rehydration.
pub struct RehydrateAllMetadataInput {
    /// Metadata language code, such as `en` or `en-US`.
    pub language: String,
}

#[derive(SimpleObject, Clone)]
/// Result of clearing metadata before background rehydration.
pub struct RehydrateAllMetadataPayload {
    /// Language whose metadata was cleared.
    pub language: String,
    /// Number of titles cleared.
    pub titles_cleared: i64,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Lifecycle state of an external monitor warmup session.
pub enum ExternalImportMonitorWarmupStatusValue {
    /// Session accepted but not started.
    Queued,
    /// Session is actively fetching or building data.
    Running,
    /// Session completed successfully.
    Completed,
    /// Session was canceled before completion.
    Canceled,
    /// Session failed before completion.
    Failed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Current work phase of an external monitor warmup session.
pub enum ExternalImportMonitorWarmupPhaseValue {
    /// Loading indexer definitions.
    LoadingIndexers,
    /// Loading movie records.
    LoadingMovies,
    /// Loading series records.
    LoadingSeries,
    /// Loading episode records.
    LoadingEpisodes,
    /// Building the imported monitor snapshot.
    BuildingSnapshot,
    /// Warmup data is ready for use.
    Ready,
}

#[derive(SimpleObject, Clone)]
/// Progress and match counts for one external monitor warmup session.
pub struct ExternalImportMonitorWarmupProgressPayload {
    /// Warmup session ID.
    pub session_id: ID,
    /// Warmup lifecycle status.
    pub status: ExternalImportMonitorWarmupStatusValue,
    /// Current warmup phase.
    pub phase: ExternalImportMonitorWarmupPhaseValue,
    /// Session start time in UTC.
    pub started_at: DateTime<Utc>,
    /// Last progress update time in UTC.
    pub updated_at: DateTime<Utc>,
    /// Whether the overall total is known.
    pub overall_total_known: bool,
    /// Overall progress counters.
    pub overall_progress: LibraryScanPhaseProgressPayload,
    /// Whether the movie total is known.
    pub movies_total_known: bool,
    /// Movie loading progress counters.
    pub movies_progress: LibraryScanPhaseProgressPayload,
    /// Whether the series total is known.
    pub series_total_known: bool,
    /// Series loading progress counters.
    pub series_progress: LibraryScanPhaseProgressPayload,
    /// Whether the episode fetch total is known.
    pub episode_fetch_total_known: bool,
    /// Expected episode count when known.
    pub episode_fetch_expected_total: Option<i32>,
    /// Expected monitored episode count when known.
    pub episode_fetch_expected_monitored_total: Option<i32>,
    /// Episode loading progress counters.
    pub episode_fetch_progress: LibraryScanPhaseProgressPayload,
    /// Whether snapshot-build totals are known.
    pub snapshot_build_total_known: bool,
    /// Snapshot-build progress counters.
    pub snapshot_build_progress: LibraryScanPhaseProgressPayload,
    /// Number of matched movies.
    pub matched_movie_count: i32,
    /// Number of matched series.
    pub matched_series_count: i32,
    /// Number of unmatched movies.
    pub unmatched_movie_count: i32,
    /// Number of unmatched series.
    pub unmatched_series_count: i32,
    /// Number of ambiguous movies.
    pub ambiguous_movie_count: i32,
    /// Number of ambiguous series.
    pub ambiguous_series_count: i32,
    /// Failure detail, or null when no error occurred.
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// External import candidates and source warmup results.
pub struct ExternalImportPreviewPayload {
    /// Whether Prowlarr connected during preview construction.
    pub prowlarr_connected: bool,
    /// Prowlarr version, or null when unavailable.
    pub prowlarr_version: Option<String>,
    /// Prowlarr connection error, or null when connected.
    pub prowlarr_error: Option<String>,
    /// External Arr source sessions included in the preview.
    pub arr_sources: Vec<ExternalImportArrSourcePayload>,
    /// Root folders reported by the sources.
    pub root_folders: Vec<ExternalImportRootFolderPayload>,
    /// Download-client candidates grouped by deduplication identity.
    pub download_clients: Vec<ExternalImportDownloadClientPayload>,
    /// Indexer candidates grouped by deduplication identity.
    pub indexers: Vec<ExternalImportIndexerPayload>,
}

#[derive(SimpleObject, Clone)]
/// Connection and warmup result for one external Arr source.
pub struct ExternalImportArrSourcePayload {
    /// Warmup session ID.
    pub session_id: ID,
    /// Stable source identity.
    pub source_key: String,
    /// External source kind.
    pub kind: ExternalArrSourceKind,
    /// Source base URL.
    pub base_url: String,
    /// Whether the source connection succeeded.
    pub connected: bool,
    /// Source version, or null when unavailable.
    pub version: Option<String>,
    /// Warmup lifecycle status.
    pub status: ExternalImportMonitorWarmupStatusValue,
    /// Connection or warmup error, or null on success.
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Root folder reported by an external Arr source.
pub struct ExternalImportRootFolderPayload {
    /// Warmup session that reported the root.
    pub source_warmup_session_id: ID,
    /// Stable source identity.
    pub source_key: String,
    /// External source kind.
    pub kind: ExternalArrSourceKind,
    /// Root path reported by the source.
    pub arr_root_path: String,
}

#[derive(SimpleObject, Clone)]
/// Download-client candidate discovered during external import.
pub struct ExternalImportDownloadClientPayload {
    /// Source identities that reported this candidate.
    pub source_keys: Vec<String>,
    /// Download-client name.
    pub name: String,
    /// Source implementation name.
    pub implementation: String,
    /// Scryer client type, or null when unsupported.
    pub scryer_client_type: Option<String>,
    /// Hostname or address, when reported.
    pub host: Option<String>,
    /// Port as reported by the external source, which may not be numeric.
    pub port: Option<String>,
    /// Whether the source uses TLS.
    pub use_ssl: bool,
    /// URL base path, when reported.
    pub url_base: Option<String>,
    /// Username, when reported.
    pub username: Option<String>,
    /// Whether an API key was returned by the source.
    pub api_key_present: bool,
    /// Stable identity used to deduplicate this candidate.
    pub dedup_key: String,
    /// Whether Scryer can create this client type.
    pub supported: bool,
    /// Whether an explicit password override is required.
    pub requires_password_override: bool,
}

#[derive(SimpleObject, Clone)]
/// Indexer candidate discovered during external import.
pub struct ExternalImportIndexerPayload {
    /// Source identities that reported this candidate.
    pub source_keys: Vec<String>,
    /// Indexer name.
    pub name: String,
    /// Source implementation name.
    pub implementation: String,
    /// Scryer provider type, or null when unsupported.
    pub scryer_provider_type: Option<String>,
    /// Indexer base URL, when reported.
    pub base_url: Option<String>,
    /// Whether an API key was returned by the source.
    pub api_key_present: bool,
    /// Stable identity used to deduplicate this candidate.
    pub dedup_key: String,
    /// Whether Scryer can create this provider type.
    pub supported: bool,
    /// Number of grouped child indexers.
    pub child_count: i32,
    /// Names of grouped child indexers.
    pub child_names: Vec<String>,
    /// Whether an explicit API key override is required.
    pub requires_api_key_override: bool,
    /// Provider help URL for obtaining an API key, when available.
    pub api_key_help_url: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Counts and errors produced by external import execution.
pub struct ExternalImportResultPayload {
    /// Whether selected media paths were saved.
    pub media_paths_saved: bool,
    /// Number of download clients created.
    pub download_clients_created: i32,
    /// Number of indexers created.
    pub indexers_created: i32,
    /// Plugin identifiers installed as part of the import.
    pub plugins_installed: Vec<String>,
    /// Errors encountered during execution.
    pub errors: Vec<String>,
}

// ── Post-Processing Scripts ────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Configured post-processing script and execution policy.
pub struct PostProcessingScriptPayload {
    /// Post-processing script ID.
    pub id: ID,
    /// Script display name.
    pub name: String,
    /// Script description.
    pub description: String,
    /// Script source type.
    pub script_type: String,
    /// Source text executed by the post-processing runtime.
    pub script_content: String,
    /// Media facets to which the script applies.
    pub applied_facets: Vec<String>,
    /// Workflow phase in which the script runs.
    pub execution_mode: ExecutionModeValue,
    /// Maximum runtime in seconds.
    pub timeout_secs: i32,
    /// Relative execution priority.
    pub priority: i32,
    /// Whether execution is enabled.
    pub enabled: bool,
    /// Whether debug output is enabled.
    pub debug: bool,
    /// Creation time in UTC.
    pub created_at: DateTime<Utc>,
    /// Last update time in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Identifier of a deleted post-processing script.
pub struct DeletePostProcessingScriptPayload {
    /// Deleted script ID.
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
/// Result of one post-processing script execution.
pub struct PostProcessingScriptRunPayload {
    /// Script run ID.
    pub id: ID,
    /// ID of the configured script that produced this run.
    pub script_id: ID,
    /// Script name at execution time.
    pub script_name: String,
    /// Associated title ID, or null when not title-specific.
    pub title_id: Option<ID>,
    /// Associated title name, or null when not title-specific.
    pub title_name: Option<String>,
    /// Media facet processed, or null when not facet-specific.
    pub facet: Option<MediaFacetValue>,
    /// File path processed, or null when no file path applied.
    pub file_path: Option<String>,
    /// Run status.
    pub status: String,
    /// Process exit code, or null when the process did not exit normally.
    pub exit_code: Option<i32>,
    /// Tail of standard output, when captured.
    pub stdout_tail: Option<String>,
    /// Tail of standard error, when captured.
    pub stderr_tail: Option<String>,
    /// Runtime in milliseconds.
    pub duration_ms: Option<i32>,
    /// Start time in UTC.
    pub started_at: DateTime<Utc>,
    /// Completion time in UTC, or null while the run is active.
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(InputObject)]
/// Values required to create a post-processing script.
pub struct CreatePostProcessingScriptInput {
    /// Script display name.
    pub name: String,
    /// Script description, or null for no description.
    pub description: Option<String>,
    /// Script source type.
    pub script_type: String,
    /// Script content, or null when supplied by the selected script type.
    pub script_content: Option<String>,
    /// Explicit acknowledgement that inline shell executes with application privileges.
    pub inline_shell_acknowledged: Option<bool>,
    /// Media facets to process, or null for the service default.
    pub applied_facets: Option<Vec<String>>,
    /// Execution mode, or null for the service default.
    pub execution_mode: Option<ExecutionModeValue>,
    /// Maximum runtime in seconds, or null for the service default.
    pub timeout_secs: Option<i32>,
    /// Relative execution priority, or null for the service default.
    pub priority: Option<i32>,
    /// Whether debug output is enabled, or null for the service default.
    pub debug: Option<bool>,
}

#[derive(InputObject)]
/// Values that may be changed on an existing post-processing script.
pub struct UpdatePostProcessingScriptInput {
    /// Script ID to update.
    pub id: ID,
    /// Replacement script name, or null to leave unchanged.
    pub name: Option<String>,
    /// Replacement description, or null to leave unchanged.
    pub description: Option<String>,
    /// Replacement script source type, or null to leave unchanged.
    pub script_type: Option<String>,
    /// Replacement script content, or null to leave unchanged.
    pub script_content: Option<String>,
    /// Explicit acknowledgement that inline shell executes with application privileges.
    pub inline_shell_acknowledged: Option<bool>,
    /// Replacement media facets, or null to leave unchanged.
    pub applied_facets: Option<Vec<String>>,
    /// Replacement execution mode, or null to leave unchanged.
    pub execution_mode: Option<ExecutionModeValue>,
    /// Replacement maximum runtime in seconds, or null to leave unchanged.
    pub timeout_secs: Option<i32>,
    /// Replacement execution priority, or null to leave unchanged.
    pub priority: Option<i32>,
    /// Replacement enabled state, or null to leave unchanged.
    pub enabled: Option<bool>,
    /// Replacement debug state, or null to leave unchanged.
    pub debug: Option<bool>,
}

// ── Subtitle downloads ──────────────────────────────────────────────────────

#[derive(async_graphql::SimpleObject)]
/// Subtitle file downloaded for a media file.
pub struct ExternalSubtitlePayload {
    /// Subtitle record ID.
    pub id: ID,
    /// Media file ID containing the subtitle.
    pub media_file_id: ID,
    /// Title ID owning the media file.
    pub title_id: ID,
    /// Episode ID, or null for movie or series-level subtitles.
    pub episode_id: Option<ID>,
    /// Subtitle source category.
    pub source_kind: String,
    /// Subtitle language code.
    pub language: String,
    /// Subtitle provider, or null when provider metadata is unavailable.
    pub provider: Option<String>,
    /// Provider subtitle file ID, or null when unavailable.
    pub provider_file_id: Option<String>,
    /// Local subtitle file path.
    pub file_path: String,
    /// Provider score, when supplied.
    pub score: Option<i32>,
    /// Provider score as a percentage, when supplied.
    pub score_percent: Option<i32>,
    /// Whether the subtitle is marked for hearing-impaired viewers.
    pub hearing_impaired: bool,
    /// Whether the subtitle is forced.
    pub forced: bool,
    /// Whether the subtitle was translated by an AI system.
    pub ai_translated: bool,
    /// Whether the subtitle was translated by a machine process.
    pub machine_translated: bool,
    /// Provider uploader, or null when unavailable.
    pub uploader: Option<String>,
    /// Provider release information, or null when unavailable.
    pub release_info: Option<String>,
    /// Whether the subtitle timing is synchronized.
    pub synced: bool,
    /// Download time in UTC.
    pub downloaded_at: DateTime<Utc>,
}

#[derive(async_graphql::SimpleObject)]
/// Subtitle provider record blocked for a media file.
pub struct ExternalSubtitleBlocklistEntryPayload {
    /// Blocklist entry ID.
    pub id: ID,
    /// Media file ID affected by the blocklist entry.
    pub media_file_id: ID,
    /// Subtitle provider.
    pub provider: String,
    /// Provider subtitle file ID.
    pub provider_file_id: String,
    /// Blocked subtitle language code.
    pub language: String,
    /// Blocklist reason, or null when none was recorded.
    pub reason: Option<String>,
    /// Creation time in UTC.
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Title History
// ---------------------------------------------------------------------------

#[derive(SimpleObject, Clone)]
/// One event in a title's acquisition and file history.
pub struct TitleHistoryEventPayload {
    /// History event ID.
    pub id: ID,
    /// Title ID associated with the event.
    pub title_id: ID,
    /// Title name, or null when no name was available.
    pub title_name: Option<String>,
    /// Library owning the title, or null when the event cannot be attributed.
    pub library_id: Option<ID>,
    /// Media facet, or null when the event is not facet-specific.
    pub facet: Option<MediaFacetValue>,
    /// Bytes imported or upgraded; null for other event types and for events recorded before sizes were captured.
    pub size_bytes: Option<Long>,
    /// Episode ID, or null when not episode-specific.
    pub episode_id: Option<ID>,
    /// Episode IDs affected by the event.
    pub episode_ids: Vec<ID>,
    /// Collection ID, or null when not collection-specific.
    pub collection_id: Option<ID>,
    /// Stable domain event code represented by this history row.
    pub event_type: String,
    /// Kind of actor that caused the event, or null when unknown.
    pub actor_kind: Option<ActorKindValue>,
    /// Acting user ID, or null for non-user actors.
    pub actor_user_id: Option<ID>,
    /// Acting user display name, or null when unavailable.
    pub actor_display_name: Option<String>,
    /// Title as supplied by the source, or null when unavailable.
    pub source_title: Option<String>,
    /// Display title recorded for the event, or null when unavailable.
    pub display_title: Option<String>,
    /// Source system name, or null when unavailable.
    pub source_system: Option<String>,
    /// Source reference, or null when unavailable.
    pub source_ref: Option<String>,
    /// Source provider, or null when unavailable.
    pub source_provider: Option<String>,
    /// Download source locator associated with the event, or null when unavailable.
    pub source_hint: Option<String>,
    /// Quality label, or null when unavailable.
    pub quality: Option<String>,
    /// Download identity, or null when unavailable.
    pub download_id: Option<String>,
    /// Download client ID, or null when unavailable.
    pub client_id: Option<ID>,
    /// Download client name, or null when unavailable.
    pub client_name: Option<String>,
    /// Import job ID, or null when unavailable.
    pub import_id: Option<ID>,
    /// Skip reason, or null when the event was not skipped.
    pub skip_reason: Option<String>,
    /// Whether retrying requires a password.
    pub retry_requires_password: bool,
    /// Failure reason, or null when the event did not fail.
    pub failure_reason: Option<String>,
    /// Blocklist reason, or null when the event was not blocklisted.
    pub blocklist_reason: Option<String>,
    /// Source file path, or null when unavailable.
    pub source_path: Option<String>,
    /// Destination file path, or null when unavailable.
    pub dest_path: Option<String>,
    /// Additional event data as JSON, or null when absent.
    pub data_json: Option<Json<serde_json::Value>>,
    /// Time the event occurred in UTC.
    pub occurred_at: DateTime<Utc>,
    /// Time the event record was created in UTC.
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// A page of title history events and pagination state.
pub struct TitleHistoryPagePayload {
    /// Events in the requested page.
    pub items: Vec<TitleHistoryEventPayload>,
    /// Total matching events across all pages.
    pub total_count: i64,
    /// Whether more matching events exist after this page.
    pub has_more: bool,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Event types available to title history filters.
pub enum TitleHistoryEventTypeValue {
    /// Title request was created.
    Requested,
    /// Release was accepted for grabbing.
    Grabbed,
    /// Download failed.
    DownloadFailed,
    /// Release was blocklisted.
    Blocklisted,
    /// Download completed.
    DownloadCompleted,
    /// File was imported.
    Imported,
    /// Import failed.
    ImportFailed,
    /// Import was skipped.
    ImportSkipped,
    /// Existing file was upgraded.
    FileUpgraded,
    /// File was moved to the recycle bin.
    FileRecycled,
    /// File was deleted.
    FileDeleted,
    /// File was renamed.
    FileRenamed,
    /// Download was ignored.
    DownloadIgnored,
    /// Title or file was rematched.
    Rematched,
}

impl TitleHistoryEventTypeValue {
    pub fn into_domain(self) -> TitleHistoryEventType {
        match self {
            Self::Requested => TitleHistoryEventType::Requested,
            Self::Grabbed => TitleHistoryEventType::Grabbed,
            Self::DownloadFailed => TitleHistoryEventType::DownloadFailed,
            Self::Blocklisted => TitleHistoryEventType::Blocklisted,
            Self::DownloadCompleted => TitleHistoryEventType::DownloadCompleted,
            Self::Imported => TitleHistoryEventType::Imported,
            Self::ImportFailed => TitleHistoryEventType::ImportFailed,
            Self::ImportSkipped => TitleHistoryEventType::ImportSkipped,
            Self::FileUpgraded => TitleHistoryEventType::FileUpgraded,
            Self::FileRecycled => TitleHistoryEventType::FileRecycled,
            Self::FileDeleted => TitleHistoryEventType::FileDeleted,
            Self::FileRenamed => TitleHistoryEventType::FileRenamed,
            Self::DownloadIgnored => TitleHistoryEventType::DownloadIgnored,
            Self::Rematched => TitleHistoryEventType::Rematched,
        }
    }
}

#[derive(InputObject)]
/// Filters and pagination controls for title history.
pub struct TitleHistoryFilterInput {
    /// Event types to include, or null for all types.
    pub event_types: Option<Vec<TitleHistoryEventTypeValue>>,
    /// Title IDs to include, or null for all titles.
    pub title_ids: Option<Vec<ID>>,
    /// Library IDs to include, or null for all libraries.
    pub library_ids: Option<Vec<ID>>,
    /// Case-insensitive title search text, or null for no text filter.
    pub title_search: Option<String>,
    /// Download identity to include, or null for all downloads.
    pub download_id: Option<String>,
    /// Episode ID to include, or null for all episodes.
    pub episode_id: Option<ID>,
    /// Whether equivalent events should be grouped.
    pub group_by_event: Option<bool>,
    /// Maximum number of events to return.
    pub limit: Option<i32>,
    /// Number of matching events to skip.
    pub offset: Option<i32>,
}
