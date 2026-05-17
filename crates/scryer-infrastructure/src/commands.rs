use crate::queries::{
    blocklist as blocklist_queries, library_scan_unmatched as library_scan_unmatched_queries,
    sql_runtime::run_with_sqlite_busy_retries,
};
use crate::{
    encryption::EncryptionKey,
    types::{SettingDefinitionSeed, SettingsValueRecord},
};
use scryer_application::{
    AppError, AppResult, DownloadQueueCommandRecord, DownloadSourceIdentity,
    ExternalImportMonitorSnapshot, ImportArtifact, InsertMediaFileInput, LibraryScanUnmatchedItem,
    MediaFileAnalysis, PendingRelease, PendingReleaseStatus, ReleaseDecision,
    ReleaseDownloadAttemptOutcome, SuccessfulGrabCommit, TitleImageReplacement, WantedItem,
    WantedItemsQuery, WorkflowOperationInfo,
};
use scryer_domain::{
    BlocklistEntry, DomainEvent, DownloadQueueDeleteStatus, ImportType, MediaFacet, NewDomainEvent,
    SubtitleDownload,
};
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use tokio::sync::oneshot::Sender;

pub(crate) enum DbCommand {
    UpsertLibraryScanUnmatchedItem {
        item: LibraryScanUnmatchedItem,
        reply: Sender<AppResult<String>>,
    },
    DeleteLibraryScanUnmatchedItem {
        library_id: String,
        facet: MediaFacet,
        item_path: String,
        reply: Sender<AppResult<()>>,
    },
    ReplaceTitleImage {
        title_id: String,
        replacement: TitleImageReplacement,
        reply: Sender<AppResult<()>>,
    },
    ReplaceTitleImageAndAppendEvent {
        title_id: String,
        replacement: TitleImageReplacement,
        event: NewDomainEvent,
        reply: Sender<AppResult<DomainEvent>>,
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
        identity: DownloadSourceIdentity,
        reply: Sender<AppResult<()>>,
    },
    UpdateTrackedState {
        identity: DownloadSourceIdentity,
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
        source_identity: DownloadSourceIdentity,
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
        client_id: Option<String>,
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
    BlocklistSubtitleDownload {
        media_file_id: String,
        provider: String,
        provider_file_id: String,
        language: String,
        reason: Option<String>,
        reply: Sender<AppResult<String>>,
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
    CreateReleaseDownloadAttempt {
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
        reply: Sender<AppResult<()>>,
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
        query: WantedItemsQuery,
        reply: Sender<AppResult<Vec<WantedItem>>>,
    },
    CountWantedItems {
        query: WantedItemsQuery,
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
    HasRecordedDownloadFailure {
        title_id: String,
        source_title: Option<String>,
        reply: Sender<AppResult<bool>>,
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
                    library_id,
                    facet,
                    item_path,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("delete_library_scan_unmatched_item", || {
                            library_scan_unmatched_queries::delete_library_scan_unmatched_item_query(
                                &pool, &library_id, facet.clone(), &item_path,
                            )
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
                DbCommand::ReplaceTitleImageAndAppendEvent {
                    title_id,
                    replacement,
                    event,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "replace_title_image_and_append_event",
                            || {
                                crate::title_images::replace_title_image_and_append_event_query(
                                    &pool,
                                    &title_id,
                                    replacement.clone(),
                                    event.clone(),
                                )
                            },
                        )
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
                DbCommand::DeleteDownloadSubmissionByClientItemId { identity, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries(
                            "delete_download_submission_by_client_item_id",
                            || {
                                crate::queries::workflow::delete_download_submission_by_client_item_id_query(
                                    &pool,
                                    &identity,
                                )
                            },
                        )
                        .await,
                    );
                }
                DbCommand::UpdateTrackedState {
                    identity,
                    tracked_state,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("update_tracked_state", || {
                            crate::queries::workflow::update_tracked_state_query(
                                &pool,
                                &identity,
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
                    source_identity,
                    import_type,
                    payload_json,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("create_import_request", || {
                            crate::queries::workflow::create_import_request_query(
                                &pool,
                                source_identity.clone(),
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
                    client_id,
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
                                client_id.as_deref(),
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
                DbCommand::BlocklistSubtitleDownload {
                    media_file_id,
                    provider,
                    provider_file_id,
                    language,
                    reason,
                    reply,
                } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("blocklist_subtitle_download", || {
                            crate::queries::subtitle::insert_blocklist_entry(
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
                DbCommand::UpsertWantedItem { item, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("upsert_wanted_item", || {
                            crate::queries::wanted::upsert_wanted_item_query(&pool, &item)
                        })
                        .await,
                    );
                }
                DbCommand::EnsureWantedItemSeeded { item, reply } => {
                    let _ = reply.send(
                        run_with_sqlite_busy_retries("ensure_wanted_item_seeded", || {
                            crate::queries::wanted::ensure_wanted_item_seeded_query(&pool, &item)
                        })
                        .await,
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
                DbCommand::ListWantedItems { query, reply } => {
                    let _ = reply
                        .send(crate::queries::wanted::list_wanted_items_query(&pool, &query).await);
                }
                DbCommand::CountWantedItems { query, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::count_wanted_items_query(&pool, &query).await,
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
                DbCommand::HasRecordedDownloadFailure {
                    title_id,
                    source_title,
                    reply,
                } => {
                    let _ = reply.send(
                        blocklist_queries::has_recorded_download_failure_query(
                            &pool,
                            &title_id,
                            source_title.as_deref(),
                        )
                        .await,
                    );
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
    use crate::queries::sql_runtime::run_with_sqlite_busy_retries_with_deadline;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

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
                Duration::ZERO,
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
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
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
