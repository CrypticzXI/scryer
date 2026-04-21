use crate::queries::{
    blocklist as blocklist_queries, library_scan_unmatched as library_scan_unmatched_queries,
    title::*, title_history as th_queries,
};
use crate::{
    encryption::EncryptionKey,
    types::{SettingDefinitionSeed, SettingsValueRecord},
};
use scryer_application::{
    AppError, AppResult, CollectionUpdate, CreateTitleOutcome, DownloadClientConfigUpdate,
    DownloadQueueCommandRecord, EpisodeUpdate, ExternalImportMonitorSnapshot, ImportArtifact,
    IndexerConfigUpdate, InsertMediaFileInput, LibraryScanUnmatchedItem, MediaFileAnalysis,
    PendingRelease, PendingReleaseStatus, ReleaseDecision, ReleaseDownloadAttemptOutcome,
    SuccessfulGrabCommit, TitleImageReplacement, TitleMetadataUpdate, WantedItem,
    WorkflowOperationInfo,
};
use scryer_domain::{
    BlocklistEntry, Collection, DomainEvent, DownloadClientConfig, DownloadQueueDeleteStatus,
    Episode, ExternalId, ImportType, IndexerConfig, InterstitialMovieMetadata, MediaFacet,
    NewDomainEvent, NotificationChannelConfig, NotificationSubscription, PluginInstallation,
    PostProcessingScript, PostProcessingScriptRun, RuleSet, SubtitleDownload, Title,
    TitleHistoryRecord, User,
};
use sqlx::SqlitePool;
use std::future::Future;
use std::time::Duration;
use tokio::sync::mpsc;

use tokio::sync::oneshot::Sender;

const SQLITE_BUSY_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1000),
];
const SQLITE_BUSY_RETRY_HARD_CAP: Duration = Duration::from_secs(120);

pub(crate) fn is_transient_sqlite_busy(error: &AppError) -> bool {
    let AppError::Repository(message) = error else {
        return false;
    };

    let normalized = message.to_ascii_lowercase();
    normalized.contains("sqlite_code=5")
        || normalized.contains("sqlite_code=517")
        || normalized.contains("database is locked")
        || normalized.contains("database table is locked")
        || normalized.contains("database schema is locked")
        || normalized.contains("sqlite_busy")
        || normalized.contains("busy_snapshot")
        || normalized.contains("code: 5")
        || normalized.contains("code: 517")
}

pub(crate) async fn run_with_sqlite_busy_retries<T, Op, Fut>(
    operation_name: &str,
    mut operation: Op,
) -> AppResult<T>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    run_with_sqlite_busy_retries_with_deadline(
        operation_name,
        SQLITE_BUSY_RETRY_HARD_CAP,
        &mut operation,
    )
    .await
}

async fn run_with_sqlite_busy_retries_with_deadline<T, Op, Fut>(
    operation_name: &str,
    hard_cap: Duration,
    operation: &mut Op,
) -> AppResult<T>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    let started_at = tokio::time::Instant::now();
    let mut attempt = 0usize;

    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if is_transient_sqlite_busy(&error) => {
                let elapsed = started_at.elapsed();
                if elapsed >= hard_cap {
                    tracing::warn!(
                        attempts = attempt,
                        elapsed_ms = elapsed.as_millis(),
                        error = %error,
                        operation = operation_name,
                        "serialized db worker: sqlite busy retry deadline exhausted"
                    );
                    return Err(AppError::Repository(format!(
                        "serialized db worker: sqlite busy retry deadline exceeded for operation `{operation_name}` after {attempt} attempts over {}ms: {error}",
                        elapsed.as_millis()
                    )));
                }

                let scheduled_delay = SQLITE_BUSY_RETRY_DELAYS
                    [attempt.min(SQLITE_BUSY_RETRY_DELAYS.len().saturating_sub(1))];
                let remaining = hard_cap.saturating_sub(elapsed);
                let delay = scheduled_delay.min(remaining);
                tracing::debug!(
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    elapsed_ms = elapsed.as_millis(),
                    error = %error,
                    operation = operation_name,
                    "serialized db worker: retrying transient sqlite busy"
                );
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) enum DbCommand {
    UpsertLibraryScanUnmatchedItem {
        item: LibraryScanUnmatchedItem,
        reply: Sender<AppResult<String>>,
    },
    DeleteLibraryScanUnmatchedItem {
        facet: MediaFacet,
        item_path: String,
        reply: Sender<AppResult<()>>,
    },
    CreateOrGetExistingTitle {
        title: Title,
        reply: Sender<AppResult<CreateTitleOutcome>>,
    },
    ReplaceTitleImage {
        title_id: String,
        replacement: TitleImageReplacement,
        reply: Sender<AppResult<()>>,
    },
    SetTitleFolderPath {
        title_id: String,
        folder_path: String,
        reply: Sender<AppResult<()>>,
    },
    CreateCollection {
        collection: Collection,
        reply: Sender<AppResult<Collection>>,
    },
    InsertMediaFile {
        input: InsertMediaFileInput,
        reply: Sender<AppResult<String>>,
    },
    LinkFileToEpisode {
        file_id: String,
        episode_id: String,
        reply: Sender<AppResult<()>>,
    },
    UpdateMediaFileAnalysis {
        file_id: String,
        analysis: MediaFileAnalysis,
        reply: Sender<AppResult<()>>,
    },
    UpdateMediaFileSourceSignature {
        file_id: String,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    MarkScanFailed {
        file_id: String,
        error: String,
        reply: Sender<AppResult<()>>,
    },
    AppendDomainEvent {
        event: NewDomainEvent,
        reply: Sender<AppResult<DomainEvent>>,
    },
    SetEventSubscriberOffset {
        subscriber: String,
        sequence: i64,
        reply: Sender<AppResult<()>>,
    },
    UpdateTitleMetadata {
        id: String,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags_json: Option<String>,
        reply: Sender<AppResult<Title>>,
    },
    UpdateTitleHydratedMetadata {
        id: String,
        metadata: TitleMetadataUpdate,
        reply: Sender<AppResult<Title>>,
    },
    ReplaceTitleMatchState {
        id: String,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
        reply: Sender<AppResult<Title>>,
    },
    CreateEpisode {
        episode: Episode,
        reply: Sender<AppResult<Episode>>,
    },
    UpdateEpisode {
        episode_id: String,
        update: EpisodeUpdate,
        reply: Sender<AppResult<Episode>>,
    },
    DeleteEpisode {
        episode_id: String,
        reply: Sender<AppResult<()>>,
    },
    RecoverStaleRunningDeleteDownloadCommands {
        stale_seconds: i64,
        reply: Sender<AppResult<u64>>,
    },
    UpdateDeleteDownloadCommandStatus {
        id: String,
        status: DownloadQueueDeleteStatus,
        error_text: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    CreateTitle {
        title: Title,
        reply: Sender<AppResult<Title>>,
    },
    MarkTitleMetadataHydrationDueNow {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    ScheduleTitleMetadataHydrationRetry {
        id: String,
        next_attempt_at: String,
        attempt_count: i64,
        reply: Sender<AppResult<()>>,
    },
    ClearTitleMetadataHydrationRetryState {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    UpdateTitleMonitored {
        id: String,
        monitored: bool,
        reply: Sender<AppResult<Title>>,
    },
    DeleteTitle {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    ClearTitleFolderPath {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    ClearMetadataLanguageForAll {
        reply: Sender<AppResult<u64>>,
    },
    UpdateCollection {
        collection_id: String,
        update: CollectionUpdate,
        reply: Sender<AppResult<Collection>>,
    },
    UpdateCollectionInterstitialMovie {
        collection_id: String,
        interstitial_movie: InterstitialMovieMetadata,
        reply: Sender<AppResult<Collection>>,
    },
    UpdateCollectionSpecialsMovies {
        collection_id: String,
        specials_movies: Vec<InterstitialMovieMetadata>,
        reply: Sender<AppResult<Collection>>,
    },
    UpdateInterstitialSeasonEpisode {
        collection_id: String,
        season_episode: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    SetCollectionEpisodesMonitored {
        collection_id: String,
        monitored: bool,
        reply: Sender<AppResult<()>>,
    },
    DeleteCollection {
        collection_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteCollectionsForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteEpisodesForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    CreateUser {
        user: User,
        reply: Sender<AppResult<User>>,
    },
    UpdateUserEntitlements {
        id: String,
        entitlements_json: String,
        reply: Sender<AppResult<User>>,
    },
    UpdateUserPasswordHash {
        id: String,
        password_hash: String,
        reply: Sender<AppResult<User>>,
    },
    DeleteUser {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    CommitSuccessfulGrab {
        commit: SuccessfulGrabCommit,
        reply: Sender<AppResult<()>>,
    },
    AppendDomainEvents {
        events: Vec<NewDomainEvent>,
        reply: Sender<AppResult<Vec<DomainEvent>>>,
    },
    RecordDownloadSubmission {
        submission: scryer_application::DownloadSubmission,
        reply: Sender<AppResult<()>>,
    },
    DeleteDownloadSubmissionsForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteDownloadSubmissionByClientItemId {
        download_client_item_id: String,
        reply: Sender<AppResult<()>>,
    },
    UpdateTrackedState {
        download_client_type: String,
        download_client_item_id: String,
        tracked_state: String,
        reply: Sender<AppResult<()>>,
    },
    InsertImportArtifact {
        artifact: ImportArtifact,
        reply: Sender<AppResult<()>>,
    },
    CreateJobWorkflowOperation {
        operation_type: String,
        status: String,
        job_key: String,
        trigger_source: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        summary_json: Option<String>,
        summary_text: Option<String>,
        error_text: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
        reply: Sender<AppResult<crate::WorkflowOperationRecord>>,
    },
    UpdateJobWorkflowOperation {
        id: String,
        status: String,
        progress_json: Option<String>,
        summary_json: Option<String>,
        summary_text: Option<String>,
        error_text: Option<String>,
        completed_at: Option<String>,
        reply: Sender<AppResult<crate::WorkflowOperationRecord>>,
    },
    CreateImportRequest {
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
        reply: Sender<AppResult<String>>,
    },
    UpdateImportStatus {
        import_id: String,
        status: String,
        result_json: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    RecoverStaleProcessingImports {
        stale_seconds: i64,
        reply: Sender<AppResult<u64>>,
    },
    RecoverStaleProcessingImportsForType {
        import_type: ImportType,
        stale_seconds: i64,
        reply: Sender<AppResult<u64>>,
    },
    UpsertExternalImportMonitorSnapshot {
        snapshot: ExternalImportMonitorSnapshot,
        reply: Sender<AppResult<()>>,
    },
    DeleteExternalImportMonitorSnapshot {
        facet: MediaFacet,
        reply: Sender<AppResult<()>>,
    },
    QueueDeleteDownloadCommand {
        client_type: String,
        download_client_item_id: String,
        is_history: bool,
        requested_by_user_id: Option<String>,
        reply: Sender<AppResult<DownloadQueueCommandRecord>>,
    },
    PruneTerminalDeleteDownloadCommandsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    CreateWorkflowOperation {
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
        reply: Sender<AppResult<WorkflowOperationInfo>>,
    },
    UpsertLibraryProbeSignature {
        title_id: String,
        path: String,
        probe_signature_scheme: Option<String>,
        probe_signature_value: Option<String>,
        last_probed_at: Option<String>,
        last_changed_at: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    UpdateMediaFilePath {
        file_id: String,
        file_path: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteMediaFile {
        file_id: String,
        reply: Sender<AppResult<()>>,
    },
    CompleteWantedItemForTitle {
        title_id: String,
        episode_id: Option<String>,
        last_search_at: Option<String>,
        current_score: Option<i32>,
        reply: Sender<AppResult<bool>>,
    },
    DeleteReleaseDecisionsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteReleaseAttemptsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteDispatchedEventOutboxesOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteHistoryEventsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteDomainEventsOlderThanForTypes {
        days: i64,
        event_types: Vec<scryer_domain::DomainEventType>,
        reply: Sender<AppResult<u32>>,
    },
    DeleteTitleHistoryOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteDownloadImportArtifactsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteTerminalImportsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteTerminalDownloadQueueCommandsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteRuleSetHistoryOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteMediaFilesByIds {
        ids: Vec<String>,
        reply: Sender<AppResult<u32>>,
    },
    InsertSubtitleDownload {
        download: SubtitleDownload,
        reply: Sender<AppResult<()>>,
    },
    SetSubtitleDownloadSynced {
        id: String,
        synced: bool,
        reply: Sender<AppResult<()>>,
    },
    DeleteSubtitleDownload {
        id: String,
        reply: Sender<AppResult<Option<SubtitleDownload>>>,
    },
    BlacklistSubtitleDownload {
        media_file_id: String,
        provider: String,
        provider_file_id: String,
        language: String,
        reason: Option<String>,
        reply: Sender<AppResult<String>>,
    },
    CreateIndexerConfig {
        config: IndexerConfig,
        encryption_key: Option<EncryptionKey>,
        reply: Sender<AppResult<IndexerConfig>>,
    },
    TouchIndexerLastError {
        provider_type: String,
        reply: Sender<AppResult<()>>,
    },
    UpdateIndexerConfig {
        update: IndexerConfigUpdate,
        encryption_key: Option<EncryptionKey>,
        reply: Sender<AppResult<IndexerConfig>>,
    },
    DeleteIndexerConfig {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    CreateDownloadClientConfig {
        config: DownloadClientConfig,
        encryption_key: Option<EncryptionKey>,
        reply: Sender<AppResult<DownloadClientConfig>>,
    },
    UpdateDownloadClientConfig {
        update: DownloadClientConfigUpdate,
        encryption_key: Option<EncryptionKey>,
        reply: Sender<AppResult<DownloadClientConfig>>,
    },
    DeleteDownloadClientConfig {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    ReorderDownloadClientConfigs {
        ordered_ids: Vec<String>,
        reply: Sender<AppResult<()>>,
    },
    BatchEnsureSettingDefinitions {
        definitions: Vec<SettingDefinitionSeed>,
        reply: Sender<AppResult<()>>,
    },
    BatchUpsertSettingsIfNotOverridden {
        entries: Vec<(String, String, String, String)>,
        encryption_key: Option<EncryptionKey>,
        reply: Sender<AppResult<()>>,
    },
    UpsertSettingValue {
        scope: String,
        key_name: String,
        scope_id: Option<String>,
        value_json: String,
        source: String,
        updated_by_user_id: Option<String>,
        encryption_key: Option<EncryptionKey>,
        reply: Sender<AppResult<SettingsValueRecord>>,
    },
    DeleteSettingValue {
        scope: String,
        key_name: String,
        scope_id: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    ReplaceQualityProfiles {
        scope: String,
        scope_id: Option<String>,
        profiles: Vec<scryer_application::QualityProfile>,
        reply: Sender<AppResult<()>>,
    },
    UpsertQualityProfiles {
        scope: String,
        scope_id: Option<String>,
        profiles: Vec<scryer_application::QualityProfile>,
        reply: Sender<AppResult<()>>,
    },
    DeleteQualityProfile {
        profile_id: String,
        reply: Sender<AppResult<()>>,
    },
    VacuumInto {
        dest_path: String,
        reply: Sender<AppResult<()>>,
    },
    CreateRuleSet {
        rule_set: RuleSet,
        reply: Sender<AppResult<()>>,
    },
    UpdateRuleSet {
        rule_set: RuleSet,
        reply: Sender<AppResult<()>>,
    },
    DeleteRuleSet {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    RecordRuleSetHistory {
        id: String,
        rule_set_id: String,
        action: String,
        rego_source: Option<String>,
        actor_id: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    DeleteRuleSetByManagedKey {
        key: String,
        reply: Sender<AppResult<()>>,
    },
    CreatePostProcessingScript {
        script: PostProcessingScript,
        reply: Sender<AppResult<PostProcessingScript>>,
    },
    UpdatePostProcessingScript {
        script: PostProcessingScript,
        reply: Sender<AppResult<PostProcessingScript>>,
    },
    DeletePostProcessingScript {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    RecordPostProcessingScriptRun {
        run: PostProcessingScriptRun,
        reply: Sender<AppResult<()>>,
    },
    CreatePluginInstallation {
        installation: PluginInstallation,
        wasm_bytes: Option<Vec<u8>>,
        reply: Sender<AppResult<PluginInstallation>>,
    },
    UpdatePluginInstallation {
        installation: PluginInstallation,
        wasm_bytes: Option<Vec<u8>>,
        reply: Sender<AppResult<PluginInstallation>>,
    },
    DeletePluginInstallation {
        plugin_id: String,
        reply: Sender<AppResult<()>>,
    },
    SeedBuiltinPlugin {
        plugin_id: String,
        name: String,
        description: String,
        version: String,
        provider_type: String,
        reply: Sender<AppResult<()>>,
    },
    StorePluginRegistryCache {
        json: String,
        reply: Sender<AppResult<()>>,
    },
    CreateNotificationChannel {
        config: NotificationChannelConfig,
        encryption_key: Option<EncryptionKey>,
        reply: Sender<AppResult<NotificationChannelConfig>>,
    },
    UpdateNotificationChannel {
        config: NotificationChannelConfig,
        encryption_key: Option<EncryptionKey>,
        reply: Sender<AppResult<NotificationChannelConfig>>,
    },
    DeleteNotificationChannel {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    CreateNotificationSubscription {
        subscription: NotificationSubscription,
        reply: Sender<AppResult<NotificationSubscription>>,
    },
    UpdateNotificationSubscription {
        subscription: NotificationSubscription,
        reply: Sender<AppResult<NotificationSubscription>>,
    },
    DeleteNotificationSubscription {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    CreateReleaseDownloadAttempt {
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    ListEpisodesForTitle {
        title_id: String,
        reply: Sender<AppResult<Vec<Episode>>>,
    },
    FindEpisodeByTitleAndNumbers {
        title_id: String,
        season_number: String,
        episode_number: String,
        reply: Sender<AppResult<Option<Episode>>>,
    },
    FindEpisodeByTitleAndAbsoluteNumber {
        title_id: String,
        absolute_number: String,
        reply: Sender<AppResult<Option<Episode>>>,
    },
    UpsertWantedItem {
        item: WantedItem,
        reply: Sender<AppResult<String>>,
    },
    EnsureWantedItemSeeded {
        item: WantedItem,
        reply: Sender<AppResult<String>>,
    },
    UpdateWantedItemStatus {
        id: String,
        status: String,
        next_search_at: Option<String>,
        last_search_at: Option<String>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    GetWantedItemForTitle {
        title_id: String,
        episode_id: Option<String>,
        reply: Sender<AppResult<Option<WantedItem>>>,
    },
    DeleteWantedItemsForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteWantedItemsForCollection {
        collection_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteWantedItemsForEpisode {
        episode_id: String,
        reply: Sender<AppResult<()>>,
    },
    ResetFruitlessWantedItems {
        now: String,
        reply: Sender<AppResult<u64>>,
    },
    InsertReleaseDecision {
        decision: ReleaseDecision,
        reply: Sender<AppResult<String>>,
    },
    GetWantedItemById {
        id: String,
        reply: Sender<AppResult<Option<WantedItem>>>,
    },
    ListWantedItems {
        status: Option<String>,
        media_type: Option<String>,
        title_id: Option<String>,
        limit: i64,
        offset: i64,
        reply: Sender<AppResult<Vec<WantedItem>>>,
    },
    CountWantedItems {
        status: Option<String>,
        media_type: Option<String>,
        title_id: Option<String>,
        reply: Sender<AppResult<i64>>,
    },
    ListReleaseDecisionsForTitle {
        title_id: String,
        limit: i64,
        reply: Sender<AppResult<Vec<ReleaseDecision>>>,
    },
    ListReleaseDecisionsForWantedItem {
        wanted_item_id: String,
        limit: i64,
        reply: Sender<AppResult<Vec<ReleaseDecision>>>,
    },
    // ── Pending Releases ──────────────────────────────────────────────
    InsertPendingRelease {
        release: PendingRelease,
        reply: Sender<AppResult<String>>,
    },
    ListExpiredPendingReleases {
        now: String,
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    ListPendingReleasesForWantedItem {
        wanted_item_id: String,
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    UpdatePendingReleaseStatus {
        id: String,
        status: PendingReleaseStatus,
        grabbed_at: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    ListStandbyPendingReleasesForWantedItem {
        wanted_item_id: String,
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    DeleteStandbyPendingReleasesForWantedItem {
        wanted_item_id: String,
        reply: Sender<AppResult<()>>,
    },
    ListAllStandbyPendingReleases {
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    CompareAndSetPendingReleaseStatus {
        id: String,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<String>,
        reply: Sender<AppResult<bool>>,
    },
    SupersedePendingReleasesForWantedItem {
        wanted_item_id: String,
        except_id: String,
        reply: Sender<AppResult<()>>,
    },
    ListWaitingPendingReleases {
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    GetPendingRelease {
        id: String,
        reply: Sender<AppResult<Option<PendingRelease>>>,
    },
    DeletePendingReleasesForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    // ── Title History ─────────────────────────────────────────────────
    InsertTitleHistoryEvent {
        title_id: String,
        episode_id: Option<String>,
        collection_id: Option<String>,
        event_type: String,
        source_title: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        data_json: Option<String>,
        reply: Sender<AppResult<String>>,
    },
    ListTitleHistory {
        event_types: Option<Vec<String>>,
        title_ids: Option<Vec<String>>,
        download_id: Option<String>,
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<TitleHistoryRecord>, i64)>>,
    },
    ListTitleHistoryForTitle {
        title_id: String,
        event_types: Option<Vec<String>>,
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<TitleHistoryRecord>, i64)>>,
    },
    ListTitleHistoryForEpisode {
        episode_id: String,
        limit: usize,
        reply: Sender<AppResult<Vec<TitleHistoryRecord>>>,
    },
    FindTitleHistoryByDownloadId {
        download_id: String,
        reply: Sender<AppResult<Vec<TitleHistoryRecord>>>,
    },
    DeleteTitleHistoryForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    // ── Blocklist ─────────────────────────────────────────────────────
    InsertBlocklistEntry {
        title_id: String,
        source_title: Option<String>,
        source_hint: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        reason: Option<String>,
        data_json: Option<String>,
        reply: Sender<AppResult<String>>,
    },
    ListBlocklistForTitle {
        title_id: String,
        limit: usize,
        reply: Sender<AppResult<Vec<BlocklistEntry>>>,
    },
    ListBlocklistAll {
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<BlocklistEntry>, i64)>>,
    },
    DeleteBlocklistEntry {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    IsBlocklisted {
        title_id: String,
        source_title: String,
        reply: Sender<AppResult<bool>>,
    },
    DeleteBlocklistForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
}

pub(crate) fn spawn_db_command_worker(pool: SqlitePool) -> mpsc::Sender<DbCommand> {
    let (sender, mut receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            match command {
                DbCommand::UpsertLibraryScanUnmatchedItem { item, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("upsert_library_scan_unmatched_item", || {
                            library_scan_unmatched_queries::upsert_library_scan_unmatched_item_query(
                                &pool, &item,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteLibraryScanUnmatchedItem {
                    facet,
                    item_path,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_library_scan_unmatched_item", || {
                            library_scan_unmatched_queries::delete_library_scan_unmatched_item_query(
                                &pool, facet.clone(), &item_path,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreateOrGetExistingTitle { title, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_or_get_existing_title", || {
                            create_or_get_existing_title_query(&pool, &title)
                        })
                        .await,
                    );
                }
                DbCommand::ReplaceTitleImage {
                    title_id,
                    replacement,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("replace_title_image", || {
                            crate::title_images::replace_title_image_query(
                                &pool,
                                &title_id,
                                replacement.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::SetTitleFolderPath {
                    title_id,
                    folder_path,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("set_title_folder_path", || {
                            set_title_folder_path_query(&pool, &title_id, &folder_path)
                        })
                        .await,
                    );
                }
                DbCommand::CreateCollection { collection, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_collection", || {
                            create_collection_query(&pool, &collection)
                        })
                        .await,
                    );
                }
                DbCommand::InsertMediaFile { input, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("insert_media_file", || {
                            crate::queries::media_file::insert_media_file_query(&pool, &input)
                        })
                        .await,
                    );
                }
                DbCommand::LinkFileToEpisode {
                    file_id,
                    episode_id,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("link_file_to_episode", || {
                            crate::queries::media_file::link_file_to_episode_query(
                                &pool,
                                &file_id,
                                &episode_id,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateMediaFileAnalysis {
                    file_id,
                    analysis,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_media_file_analysis", || {
                            crate::queries::media_file::update_media_file_analysis_query(
                                &pool, &file_id, &analysis,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateMediaFileSourceSignature {
                    file_id,
                    size_bytes,
                    source_signature_scheme,
                    source_signature_value,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_media_file_source_signature", || {
                            crate::queries::media_file::update_media_file_source_signature_query(
                                &pool,
                                &file_id,
                                size_bytes,
                                source_signature_scheme.as_deref(),
                                source_signature_value.as_deref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::MarkScanFailed {
                    file_id,
                    error,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("mark_scan_failed", || {
                            crate::queries::media_file::mark_scan_failed_query(
                                &pool, &file_id, &error,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::AppendDomainEvent { event, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("append_domain_event", || {
                            crate::queries::domain_event::append_domain_event_query(&pool, &event)
                        })
                        .await,
                    );
                }
                DbCommand::SetEventSubscriberOffset {
                    subscriber,
                    sequence,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("set_event_subscriber_offset", || {
                            crate::queries::domain_event::set_event_subscriber_offset_query(
                                &pool,
                                &subscriber,
                                sequence,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateTitleMetadata {
                    id,
                    name,
                    facet,
                    tags_json,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_title_metadata", || {
                            update_title_metadata_query(
                                &pool,
                                &id,
                                name.clone(),
                                facet.clone(),
                                tags_json.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateTitleHydratedMetadata {
                    id,
                    metadata,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_title_hydrated_metadata", || {
                            update_title_hydrated_metadata_query(&pool, &id, metadata.clone())
                        })
                        .await,
                    );
                }
                DbCommand::ReplaceTitleMatchState {
                    id,
                    external_ids,
                    tags,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("replace_title_match_state", || {
                            replace_title_match_state_query(
                                &pool,
                                &id,
                                external_ids.clone(),
                                tags.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreateEpisode { episode, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_episode", || {
                            create_episode_query(&pool, &episode)
                        })
                        .await,
                    );
                }
                DbCommand::UpdateEpisode {
                    episode_id,
                    update,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_episode", || {
                            update_episode_query(&pool, &episode_id, update.clone())
                        })
                        .await,
                    );
                }
                DbCommand::DeleteEpisode { episode_id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_episode", || {
                            delete_episode_query(&pool, &episode_id)
                        })
                        .await,
                    );
                }
                DbCommand::RecoverStaleRunningDeleteDownloadCommands {
                    stale_seconds,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "recover_stale_running_delete_download_commands",
                            || {
                                crate::queries::workflow::recover_stale_running_delete_download_commands_query(
                                    &pool,
                                    stale_seconds,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::UpdateDeleteDownloadCommandStatus {
                    id,
                    status,
                    error_text,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_delete_download_command_status", || {
                            crate::queries::workflow::update_delete_download_command_status_query(
                                &pool,
                                &id,
                                status,
                                error_text.as_deref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreateTitle { title, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_title", || {
                            create_title_query(&pool, &title)
                        })
                        .await,
                    );
                }
                DbCommand::MarkTitleMetadataHydrationDueNow { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "mark_title_metadata_hydration_due_now",
                            || mark_title_metadata_hydration_due_now_query(&pool, &id),
                        )
                        .await,
                    );
                }
                DbCommand::ScheduleTitleMetadataHydrationRetry {
                    id,
                    next_attempt_at,
                    attempt_count,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "schedule_title_metadata_hydration_retry",
                            || {
                                schedule_title_metadata_hydration_retry_query(
                                    &pool,
                                    &id,
                                    &next_attempt_at,
                                    attempt_count,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::ClearTitleMetadataHydrationRetryState { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "clear_title_metadata_hydration_retry_state",
                            || clear_title_metadata_hydration_retry_state_query(&pool, &id),
                        )
                        .await,
                    );
                }
                DbCommand::UpdateTitleMonitored {
                    id,
                    monitored,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_title_monitored", || {
                            update_title_monitored_query(&pool, &id, monitored)
                        })
                        .await,
                    );
                }
                DbCommand::DeleteTitle { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_title", || {
                            delete_title_query(&pool, &id)
                        })
                        .await,
                    );
                }
                DbCommand::ClearTitleFolderPath { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("clear_title_folder_path", || {
                            clear_title_folder_path_query(&pool, &id)
                        })
                        .await,
                    );
                }
                DbCommand::ClearMetadataLanguageForAll { reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("clear_metadata_language_for_all", || {
                            clear_metadata_language_for_all_query(&pool)
                        })
                        .await,
                    );
                }
                DbCommand::UpdateCollection {
                    collection_id,
                    update,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_collection", || {
                            update_collection_query(&pool, &collection_id, update.clone())
                        })
                        .await,
                    );
                }
                DbCommand::UpdateCollectionInterstitialMovie {
                    collection_id,
                    interstitial_movie,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "update_collection_interstitial_movie",
                            || {
                                update_collection_interstitial_movie_query(
                                    &pool,
                                    &collection_id,
                                    &interstitial_movie,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::UpdateCollectionSpecialsMovies {
                    collection_id,
                    specials_movies,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_collection_specials_movies", || {
                            update_collection_specials_movies_query(
                                &pool,
                                &collection_id,
                                &specials_movies,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateInterstitialSeasonEpisode {
                    collection_id,
                    season_episode,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_interstitial_season_episode", || {
                            update_interstitial_season_episode_query(
                                &pool,
                                &collection_id,
                                season_episode.as_deref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::SetCollectionEpisodesMonitored {
                    collection_id,
                    monitored,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("set_collection_episodes_monitored", || {
                            set_collection_episodes_monitored_query(
                                &pool,
                                &collection_id,
                                monitored,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteCollection {
                    collection_id,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_collection", || {
                            delete_collection_query(&pool, &collection_id)
                        })
                        .await,
                    );
                }
                DbCommand::DeleteCollectionsForTitle { title_id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_collections_for_title", || {
                            delete_collections_for_title_query(&pool, &title_id)
                        })
                        .await,
                    );
                }
                DbCommand::DeleteEpisodesForTitle { title_id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_episodes_for_title", || {
                            delete_episodes_for_title_query(&pool, &title_id)
                        })
                        .await,
                    );
                }
                DbCommand::CreateUser { user, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_user", || {
                            crate::queries::user::create_user_query(&pool, &user)
                        })
                        .await,
                    );
                }
                DbCommand::UpdateUserEntitlements {
                    id,
                    entitlements_json,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_user_entitlements", || {
                            crate::queries::user::update_user_entitlements_query(
                                &pool,
                                &id,
                                &entitlements_json,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateUserPasswordHash {
                    id,
                    password_hash,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_user_password_hash", || {
                            crate::queries::user::update_user_password_query(
                                &pool,
                                &id,
                                &password_hash,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteUser { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_user", || {
                            crate::queries::user::delete_user_query(&pool, &id)
                        })
                        .await,
                    );
                }
                DbCommand::CommitSuccessfulGrab { commit, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("commit_successful_grab", || {
                            crate::queries::workflow::commit_successful_grab_query(&pool, &commit)
                        })
                        .await,
                    );
                }
                DbCommand::AppendDomainEvents { events, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("append_domain_events", || {
                            crate::queries::domain_event::append_domain_events_query(&pool, &events)
                        })
                        .await,
                    );
                }
                DbCommand::RecordDownloadSubmission { submission, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("record_download_submission", || {
                            crate::queries::workflow::record_download_submission_query(
                                &pool,
                                &submission,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteDownloadSubmissionsForTitle { title_id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_download_submissions_for_title", || {
                            crate::queries::workflow::delete_download_submissions_for_title_query(
                                &pool,
                                &title_id,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteDownloadSubmissionByClientItemId {
                    download_client_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "delete_download_submission_by_client_item_id",
                            || {
                                crate::queries::workflow::delete_download_submission_by_client_item_id_query(
                                    &pool,
                                    &download_client_item_id,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::UpdateTrackedState {
                    download_client_type,
                    download_client_item_id,
                    tracked_state,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_tracked_state", || {
                            crate::queries::workflow::update_tracked_state_query(
                                &pool,
                                &download_client_type,
                                &download_client_item_id,
                                &tracked_state,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::InsertImportArtifact { artifact, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("insert_import_artifact", || {
                            crate::queries::workflow::insert_import_artifact_query(&pool, &artifact)
                        })
                        .await,
                    );
                }
                DbCommand::CreateJobWorkflowOperation {
                    operation_type,
                    status,
                    job_key,
                    trigger_source,
                    actor_user_id,
                    progress_json,
                    summary_json,
                    summary_text,
                    error_text,
                    started_at,
                    completed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_job_workflow_operation", || {
                            crate::queries::workflow::create_job_workflow_operation_query(
                                &pool,
                                operation_type.clone(),
                                status.clone(),
                                job_key.clone(),
                                trigger_source.clone(),
                                actor_user_id.clone(),
                                progress_json.clone(),
                                summary_json.clone(),
                                summary_text.clone(),
                                error_text.clone(),
                                started_at.clone(),
                                completed_at.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateJobWorkflowOperation {
                    id,
                    status,
                    progress_json,
                    summary_json,
                    summary_text,
                    error_text,
                    completed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_job_workflow_operation", || {
                            crate::queries::workflow::update_job_workflow_operation_query(
                                &pool,
                                &id,
                                &status,
                                progress_json.clone(),
                                summary_json.clone(),
                                summary_text.clone(),
                                error_text.clone(),
                                completed_at.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreateImportRequest {
                    source_system,
                    source_ref,
                    import_type,
                    payload_json,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_import_request", || {
                            crate::queries::workflow::create_import_request_query(
                                &pool,
                                source_system.clone(),
                                source_ref.clone(),
                                import_type.clone(),
                                payload_json.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateImportStatus {
                    import_id,
                    status,
                    result_json,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_import_status", || {
                            crate::queries::workflow::update_import_status_query(
                                &pool,
                                &import_id,
                                &status,
                                result_json.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::RecoverStaleProcessingImports {
                    stale_seconds,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("recover_stale_processing_imports", || {
                            crate::queries::workflow::recover_stale_processing_imports_query(
                                &pool,
                                stale_seconds,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::RecoverStaleProcessingImportsForType {
                    import_type,
                    stale_seconds,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "recover_stale_processing_imports_for_type",
                            || {
                                crate::queries::workflow::recover_stale_processing_imports_for_type_query(
                                    &pool,
                                    import_type,
                                    stale_seconds,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::UpsertExternalImportMonitorSnapshot { snapshot, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "upsert_external_import_monitor_snapshot",
                            || {
                                crate::queries::workflow::upsert_external_import_monitor_snapshot_query(
                                    &pool,
                                    &snapshot,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::DeleteExternalImportMonitorSnapshot { facet, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "delete_external_import_monitor_snapshot",
                            || {
                                crate::queries::workflow::delete_external_import_monitor_snapshot_query(
                                    &pool,
                                    &facet,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::QueueDeleteDownloadCommand {
                    client_type,
                    download_client_item_id,
                    is_history,
                    requested_by_user_id,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("queue_delete_download_command", || {
                            crate::queries::workflow::queue_delete_download_command_query(
                                &pool,
                                &client_type,
                                &download_client_item_id,
                                is_history,
                                requested_by_user_id.as_deref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::PruneTerminalDeleteDownloadCommandsOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "prune_terminal_delete_download_commands_older_than",
                            || {
                                crate::queries::workflow::prune_terminal_delete_download_commands_query(
                                    &pool,
                                    days,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::CreateWorkflowOperation {
                    operation_type,
                    status,
                    actor_user_id,
                    progress_json,
                    started_at,
                    completed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_workflow_operation", || {
                            crate::queries::workflow::create_workflow_operation_query(
                                &pool,
                                operation_type.clone(),
                                status.clone(),
                                actor_user_id.clone(),
                                progress_json.clone(),
                                started_at.clone(),
                                completed_at.clone(),
                            )
                        })
                        .await
                        .map(|record| WorkflowOperationInfo {
                            id: record.id,
                            operation_type: record.operation_type,
                            status: record.status,
                            actor_user_id: record.actor_user_id,
                            progress_json: record.progress_json,
                            started_at: record.started_at,
                            completed_at: record.completed_at,
                            created_at: record.created_at,
                            updated_at: record.updated_at,
                        }),
                    );
                }
                DbCommand::UpsertLibraryProbeSignature {
                    title_id,
                    path,
                    probe_signature_scheme,
                    probe_signature_value,
                    last_probed_at,
                    last_changed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("upsert_library_probe_signature", || {
                            crate::queries::workflow::upsert_library_probe_signature_query(
                                &pool,
                                &title_id,
                                &path,
                                probe_signature_scheme.clone(),
                                probe_signature_value.clone(),
                                last_probed_at.clone(),
                                last_changed_at.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateMediaFilePath {
                    file_id,
                    file_path,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_media_file_path", || {
                            crate::queries::media_file::update_media_file_path_query(
                                &pool, &file_id, &file_path,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteMediaFile { file_id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_media_file", || {
                            crate::queries::media_file::delete_media_file_query(&pool, &file_id)
                        })
                        .await,
                    );
                }
                DbCommand::CompleteWantedItemForTitle {
                    title_id,
                    episode_id,
                    last_search_at,
                    current_score,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("complete_wanted_item_for_title", || {
                            crate::queries::wanted::complete_wanted_item_for_title_query(
                                &pool,
                                &title_id,
                                episode_id.as_deref(),
                                last_search_at.as_deref(),
                                current_score,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteReleaseDecisionsOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_release_decisions_older_than", || {
                            crate::queries::housekeeping::delete_release_decisions_older_than_query(
                                &pool, days,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteReleaseAttemptsOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_release_attempts_older_than", || {
                            crate::queries::housekeeping::delete_release_attempts_older_than_query(
                                &pool, days,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteDispatchedEventOutboxesOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "delete_dispatched_event_outboxes_older_than",
                            || {
                                crate::queries::housekeeping::delete_dispatched_event_outboxes_older_than_query(
                                    &pool,
                                    days,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::DeleteHistoryEventsOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_history_events_older_than", || {
                            crate::queries::housekeeping::delete_history_events_older_than_query(
                                &pool, days,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteDomainEventsOlderThanForTypes {
                    days,
                    event_types,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "delete_domain_events_older_than_for_types",
                            || {
                                crate::queries::housekeeping::delete_domain_events_older_than_for_types_query(
                                    &pool,
                                    days,
                                    &event_types,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::DeleteTitleHistoryOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_title_history_older_than", || {
                            crate::queries::housekeeping::delete_title_history_older_than_query(
                                &pool, days,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteDownloadImportArtifactsOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "delete_download_import_artifacts_older_than",
                            || {
                                crate::queries::housekeeping::delete_download_import_artifacts_older_than_query(
                                    &pool,
                                    days,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::DeleteTerminalImportsOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_terminal_imports_older_than", || {
                            crate::queries::housekeeping::delete_terminal_imports_older_than_query(
                                &pool, days,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteTerminalDownloadQueueCommandsOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "delete_terminal_download_queue_commands_older_than",
                            || {
                                crate::queries::housekeeping::delete_terminal_download_queue_commands_older_than_query(
                                    &pool,
                                    days,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::DeleteRuleSetHistoryOlderThan { days, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_rule_set_history_older_than", || {
                            crate::queries::housekeeping::delete_rule_set_history_older_than_query(
                                &pool, days,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteMediaFilesByIds { ids, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_media_files_by_ids", || {
                            crate::queries::housekeeping::delete_media_files_by_ids_query(
                                &pool, &ids,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::InsertSubtitleDownload { download, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("insert_subtitle_download", || {
                            crate::queries::subtitle::insert_subtitle_download(&pool, &download)
                        })
                        .await,
                    );
                }
                DbCommand::SetSubtitleDownloadSynced { id, synced, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("set_subtitle_download_synced", || {
                            crate::queries::subtitle::update_subtitle_download_synced(
                                &pool, &id, synced,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteSubtitleDownload { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_subtitle_download", || {
                            crate::queries::subtitle::delete_subtitle_download(&pool, &id)
                        })
                        .await,
                    );
                }
                DbCommand::BlacklistSubtitleDownload {
                    media_file_id,
                    provider,
                    provider_file_id,
                    language,
                    reason,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("blacklist_subtitle_download", || {
                            crate::queries::subtitle::insert_blacklist_entry(
                                &pool,
                                &media_file_id,
                                &provider,
                                &provider_file_id,
                                &language,
                                reason.as_deref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreateIndexerConfig {
                    config,
                    encryption_key,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_indexer_config", || {
                            crate::queries::indexer::create_indexer_config_query(
                                &pool,
                                &config,
                                encryption_key.as_ref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::TouchIndexerLastError {
                    provider_type,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("touch_indexer_last_error", || {
                            crate::queries::indexer::touch_indexer_last_error_query(
                                &pool,
                                &provider_type,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateIndexerConfig {
                    update,
                    encryption_key,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_indexer_config", || {
                            crate::queries::indexer::update_indexer_config_query(
                                &pool,
                                &update,
                                encryption_key.as_ref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteIndexerConfig { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_indexer_config", || {
                            crate::queries::indexer::delete_indexer_config_query(&pool, &id)
                        })
                        .await,
                    );
                }
                DbCommand::CreateDownloadClientConfig {
                    config,
                    encryption_key,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_download_client_config", || {
                            crate::queries::download_client::create_download_client_config_query(
                                &pool,
                                &config,
                                encryption_key.as_ref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateDownloadClientConfig {
                    update,
                    encryption_key,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_download_client_config", || {
                            crate::queries::download_client::update_download_client_config_query(
                                &pool,
                                &update,
                                encryption_key.as_ref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteDownloadClientConfig { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_download_client_config", || {
                            crate::queries::download_client::delete_download_client_config_query(
                                &pool, &id,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::ReorderDownloadClientConfigs { ordered_ids, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("reorder_download_client_configs", || {
                            crate::queries::download_client::reorder_download_client_configs_query(
                                &pool,
                                &ordered_ids,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::BatchEnsureSettingDefinitions { definitions, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("batch_ensure_setting_definitions", || {
                            crate::queries::settings::batch_ensure_setting_definitions_query(
                                &pool,
                                &definitions,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::BatchUpsertSettingsIfNotOverridden {
                    entries,
                    encryption_key,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "batch_upsert_settings_if_not_overridden",
                            || {
                                crate::queries::settings::batch_upsert_settings_if_not_overridden_query(
                                    &pool,
                                    &entries,
                                    encryption_key.as_ref(),
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::UpsertSettingValue {
                    scope,
                    key_name,
                    scope_id,
                    value_json,
                    source,
                    updated_by_user_id,
                    encryption_key,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("upsert_setting_value", || {
                            crate::queries::settings::upsert_setting_value_query(
                                &pool,
                                &scope,
                                &key_name,
                                scope_id.clone(),
                                &value_json,
                                &source,
                                updated_by_user_id.clone(),
                                encryption_key.as_ref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteSettingValue {
                    scope,
                    key_name,
                    scope_id,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_setting_value", || {
                            crate::queries::settings::delete_setting_value_query(
                                &pool,
                                &scope,
                                &key_name,
                                scope_id.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::ReplaceQualityProfiles {
                    scope,
                    scope_id,
                    profiles,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("replace_quality_profiles", || {
                            crate::queries::quality::replace_quality_profiles_query(
                                &pool,
                                &scope,
                                scope_id.clone(),
                                profiles.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpsertQualityProfiles {
                    scope,
                    scope_id,
                    profiles,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("upsert_quality_profiles", || {
                            crate::queries::quality::upsert_quality_profiles_query(
                                &pool,
                                &scope,
                                scope_id.clone(),
                                profiles.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteQualityProfile { profile_id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_quality_profile", || {
                            crate::queries::quality::delete_quality_profile_query(
                                &pool,
                                &profile_id,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::VacuumInto { dest_path, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("vacuum_into", || async {
                            sqlx::query("VACUUM INTO ?")
                                .bind(&dest_path)
                                .execute(&pool)
                                .await
                                .map_err(|err| AppError::Repository(err.to_string()))?;
                            Ok(())
                        })
                        .await,
                    );
                }
                DbCommand::CreateRuleSet { rule_set, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_rule_set", || {
                            crate::queries::rule_set::insert_rule_set_query(&pool, &rule_set)
                        })
                        .await,
                    );
                }
                DbCommand::UpdateRuleSet { rule_set, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_rule_set", || {
                            crate::queries::rule_set::update_rule_set_query(&pool, &rule_set)
                        })
                        .await,
                    );
                }
                DbCommand::DeleteRuleSet { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_rule_set", || {
                            crate::queries::rule_set::delete_rule_set_query(&pool, &id)
                        })
                        .await,
                    );
                }
                DbCommand::RecordRuleSetHistory {
                    id,
                    rule_set_id,
                    action,
                    rego_source,
                    actor_id,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("record_rule_set_history", || {
                            crate::queries::rule_set::insert_rule_set_history_query(
                                &pool,
                                &id,
                                &rule_set_id,
                                &action,
                                rego_source.as_deref(),
                                actor_id.as_deref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteRuleSetByManagedKey { key, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_rule_set_by_managed_key", || {
                            crate::queries::rule_set::delete_rule_set_by_managed_key_query(
                                &pool, &key,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreatePostProcessingScript { script, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_post_processing_script", || {
                            crate::queries::post_processing_script::insert_script_query(
                                &pool, &script,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdatePostProcessingScript { script, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_post_processing_script", || {
                            crate::queries::post_processing_script::update_script_query(
                                &pool, &script,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeletePostProcessingScript { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_post_processing_script", || {
                            crate::queries::post_processing_script::delete_script_query(&pool, &id)
                        })
                        .await,
                    );
                }
                DbCommand::RecordPostProcessingScriptRun { run, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("record_post_processing_script_run", || {
                            crate::queries::post_processing_script::record_run_query(&pool, &run)
                        })
                        .await,
                    );
                }
                DbCommand::CreatePluginInstallation {
                    installation,
                    wasm_bytes,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_plugin_installation", || {
                            crate::queries::plugin_installation::create_plugin_installation_query(
                                &pool,
                                &installation,
                                wasm_bytes.as_deref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdatePluginInstallation {
                    installation,
                    wasm_bytes,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_plugin_installation", || {
                            crate::queries::plugin_installation::update_plugin_installation_query(
                                &pool,
                                &installation,
                                wasm_bytes.as_deref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeletePluginInstallation { plugin_id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_plugin_installation", || {
                            crate::queries::plugin_installation::delete_plugin_installation_query(
                                &pool, &plugin_id,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::SeedBuiltinPlugin {
                    plugin_id,
                    name,
                    description,
                    version,
                    provider_type,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("seed_builtin_plugin", || {
                            crate::queries::plugin_installation::seed_builtin_query(
                                &pool,
                                &plugin_id,
                                &name,
                                &description,
                                &version,
                                &provider_type,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::StorePluginRegistryCache { json, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("store_plugin_registry_cache", || {
                            crate::queries::plugin_installation::store_registry_cache_query(
                                &pool, &json,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreateNotificationChannel {
                    config,
                    encryption_key,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_notification_channel", || {
                            crate::queries::notification_channel::create_notification_channel_query(
                                &pool,
                                &config,
                                encryption_key.as_ref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateNotificationChannel {
                    config,
                    encryption_key,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_notification_channel", || {
                            crate::queries::notification_channel::update_notification_channel_query(
                                &pool,
                                &config,
                                encryption_key.as_ref(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteNotificationChannel { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_notification_channel", || {
                            crate::queries::notification_channel::delete_notification_channel_query(
                                &pool, &id,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreateNotificationSubscription {
                    subscription,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_notification_subscription", || {
                            crate::queries::notification_subscription::create_notification_subscription_query(
                                &pool,
                                &subscription,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::UpdateNotificationSubscription {
                    subscription,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_notification_subscription", || {
                            crate::queries::notification_subscription::update_notification_subscription_query(
                                &pool,
                                &subscription,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::DeleteNotificationSubscription { id, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_notification_subscription", || {
                            crate::queries::notification_subscription::delete_notification_subscription_query(
                                &pool,
                                &id,
                            )
                        })
                        .await,
                    );
                }
                DbCommand::CreateReleaseDownloadAttempt {
                    title_id,
                    source_hint,
                    source_title,
                    outcome,
                    error_message,
                    source_password,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_release_download_attempt", || {
                            crate::queries::workflow::create_release_download_attempt_query(
                                &pool,
                                title_id.clone(),
                                source_hint.clone(),
                                source_title.clone(),
                                outcome.clone(),
                                error_message.clone(),
                                source_password.clone(),
                            )
                        })
                        .await,
                    );
                }
                DbCommand::ListEpisodesForTitle { title_id, reply } => {
                    let _ = reply.send(list_episodes_for_title_query(&pool, &title_id).await);
                }
                DbCommand::FindEpisodeByTitleAndNumbers {
                    title_id,
                    season_number,
                    episode_number,
                    reply,
                } => {
                    let _ = reply.send(
                        find_episode_by_title_and_numbers_query(
                            &pool,
                            &title_id,
                            &season_number,
                            &episode_number,
                        )
                        .await,
                    );
                }
                DbCommand::FindEpisodeByTitleAndAbsoluteNumber {
                    title_id,
                    absolute_number,
                    reply,
                } => {
                    let _ = reply.send(
                        find_episode_by_title_and_absolute_number_query(
                            &pool,
                            &title_id,
                            &absolute_number,
                        )
                        .await,
                    );
                }
                DbCommand::UpsertWantedItem { item, reply } => {
                    let _ = reply
                        .send(crate::queries::wanted::upsert_wanted_item_query(&pool, &item).await);
                }
                DbCommand::EnsureWantedItemSeeded { item, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::ensure_wanted_item_seeded_query(&pool, &item).await,
                    );
                }
                DbCommand::UpdateWantedItemStatus {
                    id,
                    status,
                    next_search_at,
                    last_search_at,
                    search_count,
                    current_score,
                    grabbed_release,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::update_wanted_item_status_query(
                            &pool,
                            &id,
                            &status,
                            next_search_at.as_deref(),
                            last_search_at.as_deref(),
                            search_count,
                            current_score,
                            grabbed_release.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::GetWantedItemForTitle {
                    title_id,
                    episode_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::get_wanted_item_for_title_query(
                            &pool,
                            &title_id,
                            episode_id.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::DeleteWantedItemsForTitle { title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::delete_wanted_items_for_title_query(
                            &pool, &title_id,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteWantedItemsForCollection {
                    collection_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::delete_wanted_items_for_collection_query(
                            &pool,
                            &collection_id,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteWantedItemsForEpisode { episode_id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::delete_wanted_items_for_episode_query(
                            &pool,
                            &episode_id,
                        )
                        .await,
                    );
                }
                DbCommand::ResetFruitlessWantedItems { now, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::reset_fruitless_wanted_items_query(&pool, &now)
                            .await,
                    );
                }
                DbCommand::InsertReleaseDecision { decision, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::insert_release_decision_query(&pool, &decision)
                            .await,
                    );
                }
                DbCommand::GetWantedItemById { id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::get_wanted_item_by_id_query(&pool, &id).await,
                    );
                }
                DbCommand::ListWantedItems {
                    status,
                    media_type,
                    title_id,
                    limit,
                    offset,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_wanted_items_query(
                            &pool,
                            status.as_deref(),
                            media_type.as_deref(),
                            title_id.as_deref(),
                            limit,
                            offset,
                        )
                        .await,
                    );
                }
                DbCommand::CountWantedItems {
                    status,
                    media_type,
                    title_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::count_wanted_items_query(
                            &pool,
                            status.as_deref(),
                            media_type.as_deref(),
                            title_id.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListReleaseDecisionsForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_release_decisions_for_title_query(
                            &pool, &title_id, limit,
                        )
                        .await,
                    );
                }
                DbCommand::ListReleaseDecisionsForWantedItem {
                    wanted_item_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_release_decisions_for_wanted_item_query(
                            &pool,
                            &wanted_item_id,
                            limit,
                        )
                        .await,
                    );
                }
                // ── Pending Releases ──────────────────────────────────────
                DbCommand::InsertPendingRelease { release, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::insert_pending_release_query(
                            &pool, &release,
                        )
                        .await,
                    );
                }
                DbCommand::ListExpiredPendingReleases { now, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_expired_pending_releases_query(
                            &pool, &now,
                        )
                        .await,
                    );
                }
                DbCommand::ListPendingReleasesForWantedItem {
                    wanted_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_pending_releases_for_wanted_item_query(
                            &pool, &wanted_item_id,
                        ).await,
                    );
                }
                DbCommand::UpdatePendingReleaseStatus {
                    id,
                    status,
                    grabbed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::update_pending_release_status_query(
                            &pool,
                            &id,
                            status,
                            grabbed_at.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListStandbyPendingReleasesForWantedItem {
                    wanted_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_standby_pending_releases_for_wanted_item_query(
                            &pool,
                            &wanted_item_id,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteStandbyPendingReleasesForWantedItem {
                    wanted_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::delete_standby_pending_releases_for_wanted_item_query(
                            &pool,
                            &wanted_item_id,
                        )
                        .await,
                    );
                }
                DbCommand::ListAllStandbyPendingReleases { reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_all_standby_pending_releases_query(
                            &pool,
                        )
                        .await,
                    );
                }
                DbCommand::CompareAndSetPendingReleaseStatus {
                    id,
                    current_status,
                    next_status,
                    grabbed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::compare_and_set_pending_release_status_query(
                            &pool,
                            &id,
                            current_status,
                            next_status,
                            grabbed_at.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::SupersedePendingReleasesForWantedItem {
                    wanted_item_id,
                    except_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::supersede_pending_releases_for_wanted_item_query(
                            &pool, &wanted_item_id, &except_id,
                        ).await,
                    );
                }
                DbCommand::ListWaitingPendingReleases { reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_waiting_pending_releases_query(
                            &pool,
                        )
                        .await,
                    );
                }
                DbCommand::GetPendingRelease { id, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::get_pending_release_query(&pool, &id)
                            .await,
                    );
                }
                DbCommand::DeletePendingReleasesForTitle { title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::delete_pending_releases_for_title_query(
                            &pool, &title_id,
                        )
                        .await,
                    );
                }
                // ── Title History ─────────────────────────────────────────
                DbCommand::InsertTitleHistoryEvent {
                    title_id,
                    episode_id,
                    collection_id,
                    event_type,
                    source_title,
                    quality,
                    download_id,
                    data_json,
                    reply,
                } => {
                    let _ = reply.send(
                        th_queries::insert_title_history_event_query(
                            &pool,
                            &title_id,
                            episode_id.as_deref(),
                            collection_id.as_deref(),
                            &event_type,
                            source_title.as_deref(),
                            quality.as_deref(),
                            download_id.as_deref(),
                            data_json.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListTitleHistory {
                    event_types,
                    title_ids,
                    download_id,
                    limit,
                    offset,
                    reply,
                } => {
                    let type_strs: Option<Vec<&str>> = event_types
                        .as_ref()
                        .map(|v| v.iter().map(|s| s.as_str()).collect());
                    let res = th_queries::list_title_history_query(
                        &pool,
                        type_strs.as_deref(),
                        title_ids.as_deref(),
                        download_id.as_deref(),
                        limit,
                        offset,
                    )
                    .await
                    .map(|(rows, total)| {
                        let records = rows.into_iter().map(th_row_to_record).collect();
                        (records, total)
                    });
                    let _ = reply.send(res);
                }
                DbCommand::ListTitleHistoryForTitle {
                    title_id,
                    event_types,
                    limit,
                    offset,
                    reply,
                } => {
                    let type_strs: Option<Vec<&str>> = event_types
                        .as_ref()
                        .map(|v| v.iter().map(|s| s.as_str()).collect());
                    let res = th_queries::list_title_history_for_title_query(
                        &pool,
                        &title_id,
                        type_strs.as_deref(),
                        limit,
                        offset,
                    )
                    .await
                    .map(|(rows, total)| {
                        let records = rows.into_iter().map(th_row_to_record).collect();
                        (records, total)
                    });
                    let _ = reply.send(res);
                }
                DbCommand::ListTitleHistoryForEpisode {
                    episode_id,
                    limit,
                    reply,
                } => {
                    let res =
                        th_queries::list_title_history_for_episode_query(&pool, &episode_id, limit)
                            .await
                            .map(|rows| rows.into_iter().map(th_row_to_record).collect());
                    let _ = reply.send(res);
                }
                DbCommand::FindTitleHistoryByDownloadId { download_id, reply } => {
                    let res =
                        th_queries::find_title_history_by_download_id_query(&pool, &download_id)
                            .await
                            .map(|rows| rows.into_iter().map(th_row_to_record).collect());
                    let _ = reply.send(res);
                }
                DbCommand::DeleteTitleHistoryForTitle { title_id, reply } => {
                    let _ = reply.send(
                        th_queries::delete_title_history_for_title_query(&pool, &title_id).await,
                    );
                }
                // ── Blocklist ─────────────────────────────────────────────
                DbCommand::InsertBlocklistEntry {
                    title_id,
                    source_title,
                    source_hint,
                    quality,
                    download_id,
                    reason,
                    data_json,
                    reply,
                } => {
                    let _ = reply.send(
                        blocklist_queries::insert_blocklist_entry_query(
                            &pool,
                            &title_id,
                            source_title.as_deref(),
                            source_hint.as_deref(),
                            quality.as_deref(),
                            download_id.as_deref(),
                            reason.as_deref(),
                            data_json.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListBlocklistForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let res =
                        blocklist_queries::list_blocklist_for_title_query(&pool, &title_id, limit)
                            .await
                            .map(|rows| rows.into_iter().map(bl_row_to_entry).collect());
                    let _ = reply.send(res);
                }
                DbCommand::ListBlocklistAll {
                    limit,
                    offset,
                    reply,
                } => {
                    let res = blocklist_queries::list_blocklist_all_query(&pool, limit, offset)
                        .await
                        .map(|(rows, total)| {
                            let entries = rows.into_iter().map(bl_row_to_entry).collect();
                            (entries, total)
                        });
                    let _ = reply.send(res);
                }
                DbCommand::DeleteBlocklistEntry { id, reply } => {
                    let _ = reply
                        .send(blocklist_queries::delete_blocklist_entry_query(&pool, &id).await);
                }
                DbCommand::IsBlocklisted {
                    title_id,
                    source_title,
                    reply,
                } => {
                    let _ = reply.send(
                        blocklist_queries::is_blocklisted_query(&pool, &title_id, &source_title)
                            .await,
                    );
                }
                DbCommand::DeleteBlocklistForTitle { title_id, reply } => {
                    let _ = reply.send(
                        blocklist_queries::delete_blocklist_for_title_query(&pool, &title_id).await,
                    );
                }
            }
        }
    });

    sender
}

fn th_row_to_record(row: th_queries::TitleHistoryRow) -> TitleHistoryRecord {
    TitleHistoryRecord {
        id: row.id,
        title_id: row.title_id,
        episode_id: row.episode_id,
        collection_id: row.collection_id,
        event_type: scryer_domain::TitleHistoryEventType::parse(&row.event_type)
            .unwrap_or(scryer_domain::TitleHistoryEventType::Grabbed),
        source_title: row.source_title,
        quality: row.quality,
        download_id: row.download_id,
        data_json: row.data_json,
        occurred_at: row.occurred_at,
        created_at: row.created_at,
    }
}

fn bl_row_to_entry(row: blocklist_queries::BlocklistRow) -> BlocklistEntry {
    BlocklistEntry {
        id: row.id,
        title_id: row.title_id,
        source_title: row.source_title,
        source_hint: row.source_hint,
        quality: row.quality,
        download_id: row.download_id,
        reason: row.reason,
        data_json: row.data_json,
        created_at: row.created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn sqlite_busy_retries_eventually_succeed_before_deadline() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let result = {
            let mut operation = {
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                            Err(AppError::Repository("sqlite_code=5".to_string()))
                        } else {
                            Ok("ok")
                        }
                    }
                }
            };
            run_with_sqlite_busy_retries_with_deadline(
                "test_operation",
                Duration::from_millis(250),
                &mut operation,
            )
            .await
            .expect("operation should eventually succeed")
        };

        assert_eq!(result, "ok");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn sqlite_busy_retries_fail_after_two_minute_deadline() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let error = {
            let mut operation = {
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        Err::<(), _>(AppError::Repository("database is locked".to_string()))
                    }
                }
            };
            run_with_sqlite_busy_retries_with_deadline(
                "test_operation",
                Duration::from_millis(5),
                &mut operation,
            )
            .await
            .expect_err("persistent busy should fail once the deadline is exhausted")
        };

        let AppError::Repository(message) = error else {
            panic!("expected repository error");
        };
        assert!(message.contains("test_operation"));
        assert!(message.contains("deadline exceeded"));
        assert!(attempts.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn sqlite_busy_retries_do_not_retry_non_busy_errors() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let error = {
            let mut operation = {
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        Err::<(), _>(AppError::Repository("constraint violation".to_string()))
                    }
                }
            };
            run_with_sqlite_busy_retries_with_deadline(
                "test_operation",
                Duration::from_millis(50),
                &mut operation,
            )
            .await
            .expect_err("non-busy errors should fail immediately")
        };

        let AppError::Repository(message) = error else {
            panic!("expected repository error");
        };
        assert_eq!(message, "constraint violation");
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
}
