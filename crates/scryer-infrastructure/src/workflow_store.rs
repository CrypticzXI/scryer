use async_trait::async_trait;
use scryer_application::{
    AcquisitionStateRepository, AppError, AppResult, DomainEventRepository,
    DownloadQueueCommandRecord, DownloadQueueCommandRepository, DownloadSourceIdentity,
    DownloadSubmission, DownloadSubmissionRepository, ExternalImportMonitorSnapshot,
    ExternalImportMonitorSnapshotChunk, ExternalImportMonitorSnapshotChunkScopeKind,
    ExternalImportMonitorSnapshotEntryKind, ExternalImportMonitorSnapshotRepository,
    ImportArtifact, ImportArtifactRepository, ImportRepository, JobKey, JobRunRecord,
    JobRunRepository, JobRunStatus, JobTriggerSource, SuccessfulGrabCommit, WorkflowOperationInfo,
    WorkflowOperationRepository,
};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DownloadQueueDeleteStatus, ImportRecord, ImportStatus,
    ImportType, NewDomainEvent, TitleHistoryEventType,
};

use crate::SqliteServices;

#[async_trait]
pub trait WorkflowSql: Clone + Send + Sync + 'static {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()>;
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
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord>;
    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord>;
    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>>;
    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>>;
    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>>;
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
    async fn upsert_external_import_monitor_snapshot(
        &self,
        snapshot: &ExternalImportMonitorSnapshot,
    ) -> AppResult<()>;
    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()>;
    async fn list_external_import_monitor_snapshot_chunks(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>>;
    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>>;
    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
    ) -> AppResult<()>;
    async fn get_external_import_monitor_snapshot(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Option<ExternalImportMonitorSnapshot>>;
    async fn delete_external_import_monitor_snapshot(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<()>;
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord>;
    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64>;
    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>>;
    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()>;
    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()>;
    async fn mark_delete_command_failed(&self, id: &str, error_text: Option<&str>)
    -> AppResult<()>;
    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(Option<String>, String, String, bool)],
    ) -> AppResult<Vec<DownloadQueueCommandRecord>>;
    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32>;
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

#[derive(Clone)]
pub struct WorkflowStore<S> {
    sql: S,
}

impl<S> WorkflowStore<S> {
    pub(crate) fn from_sql(sql: S) -> Self {
        Self { sql }
    }
}

#[async_trait]
impl<S: WorkflowSql> AcquisitionStateRepository for WorkflowStore<S> {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        self.sql.commit_successful_grab(commit).await
    }
}

#[async_trait]
impl<S: WorkflowSql> DomainEventRepository for WorkflowStore<S> {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        self.sql.append(event).await
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        self.sql.append_many(events).await
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        self.sql.list(filter).await
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        self.sql
            .count_title_history_page_events(event_types, title_ids, download_id)
            .await
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        self.sql
            .list_title_history_page_events(event_types, title_ids, download_id, limit, offset)
            .await
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        self.sql.list_after_sequence(after_sequence, limit).await
    }

    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.sql.delete_for_title_ids(title_ids).await
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        self.sql.get_subscriber_offset(subscriber).await
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        self.sql.set_subscriber_offset(subscriber, sequence).await
    }
}

#[async_trait]
impl<S: WorkflowSql> DownloadSubmissionRepository for WorkflowStore<S> {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        self.sql.record_submission(submission).await
    }

    async fn find_by_client_item_id(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        self.sql.find_by_client_item_id(identity).await
    }

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        self.sql.list_for_client_items(client_items).await
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        self.sql.list_for_title(title_id).await
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        self.sql
            .find_by_title_and_request_signature(title_id, request_signature)
            .await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.sql.delete_for_title(title_id).await
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        self.sql.delete_by_client_item_id(identity).await
    }

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()> {
        self.sql.update_tracked_state(identity, tracked_state).await
    }

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>> {
        self.sql.get_tracked_state(identity).await
    }
}

#[async_trait]
impl<S: WorkflowSql> ImportArtifactRepository for WorkflowStore<S> {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
        self.sql.insert_artifact(artifact).await
    }

    async fn list_by_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Vec<ImportArtifact>> {
        self.sql.list_by_source_identity(identity).await
    }

    async fn count_by_result_for_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
        result: &str,
    ) -> AppResult<u64> {
        self.sql
            .count_by_result_for_source_identity(identity, result)
            .await
    }
}

#[async_trait]
impl<S: WorkflowSql> JobRunRepository for WorkflowStore<S> {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        self.sql.create_job_run(run).await
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        self.sql.update_job_run(run).await
    }

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>> {
        self.sql.get_job_run(run_id).await
    }

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        self.sql.list_job_runs(job_key, limit).await
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        self.sql.list_active_job_runs().await
    }
}

#[async_trait]
impl<S: WorkflowSql> ImportRepository for WorkflowStore<S> {
    async fn queue_import_request(
        &self,
        source_identity: DownloadSourceIdentity,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        self.sql
            .queue_import_request(source_identity, import_type, payload_json)
            .await
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        self.sql.get_import_by_id(id).await
    }

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.sql
            .update_import_status(import_id, status, result_json)
            .await
    }

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        self.sql
            .recover_stale_processing_imports(stale_seconds)
            .await
    }

    async fn recover_stale_processing_imports_for_type(
        &self,
        import_type: ImportType,
        stale_seconds: i64,
    ) -> AppResult<u64> {
        self.sql
            .recover_stale_processing_imports_for_type(import_type, stale_seconds)
            .await
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        self.sql.list_pending_imports().await
    }

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>> {
        self.sql.list_pending_imports_for_type(import_type).await
    }

    async fn list_imports_for_identities(
        &self,
        identities: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<ImportRecord>> {
        self.sql.list_imports_for_identities(identities).await
    }

    async fn is_already_imported(&self, identity: &DownloadSourceIdentity) -> AppResult<bool> {
        self.sql.is_already_imported(identity).await
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        self.sql.list_imports(limit).await
    }
}

#[async_trait]
impl<S: WorkflowSql> ExternalImportMonitorSnapshotRepository for WorkflowStore<S> {
    async fn upsert_external_import_monitor_snapshot(
        &self,
        snapshot: &ExternalImportMonitorSnapshot,
    ) -> AppResult<()> {
        self.sql
            .upsert_external_import_monitor_snapshot(snapshot)
            .await
    }

    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        self.sql
            .append_external_import_monitor_snapshot_chunk(chunk)
            .await
    }

    async fn list_external_import_monitor_snapshot_chunks(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        self.sql
            .list_external_import_monitor_snapshot_chunks(scope_kind, scope_key, entry_kind)
            .await
    }

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        self.sql
            .list_external_import_monitor_snapshot_chunk_batch(
                scope_kind,
                scope_key,
                entry_kind,
                after_chunk_index,
                limit,
            )
            .await
    }

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
    ) -> AppResult<()> {
        self.sql
            .delete_external_import_monitor_snapshot_chunks(scope_kind, scope_key)
            .await
    }

    async fn get_external_import_monitor_snapshot(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Option<ExternalImportMonitorSnapshot>> {
        self.sql.get_external_import_monitor_snapshot(facet).await
    }

    async fn delete_external_import_monitor_snapshot(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<()> {
        self.sql
            .delete_external_import_monitor_snapshot(facet)
            .await
    }
}

#[async_trait]
impl<S: WorkflowSql> DownloadQueueCommandRepository for WorkflowStore<S> {
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        self.sql
            .queue_delete_command(
                client_id,
                client_type,
                download_client_item_id,
                is_history,
                requested_by_user_id,
            )
            .await
    }

    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64> {
        self.sql
            .recover_stale_running_delete_commands(stale_seconds)
            .await
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        self.sql.list_pending_delete_commands().await
    }

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()> {
        self.sql.mark_delete_command_running(id).await
    }

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()> {
        self.sql.mark_delete_command_completed(id).await
    }

    async fn mark_delete_command_failed(
        &self,
        id: &str,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        self.sql.mark_delete_command_failed(id, error_text).await
    }

    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(Option<String>, String, String, bool)],
    ) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        self.sql
            .list_latest_delete_commands_for_sources(sources)
            .await
    }

    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql
            .prune_terminal_delete_commands_older_than(days)
            .await
    }
}

#[async_trait]
impl<S: WorkflowSql> WorkflowOperationRepository for WorkflowStore<S> {
    async fn create_workflow_operation(
        &self,
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<WorkflowOperationInfo> {
        self.sql
            .create_workflow_operation(
                operation_type,
                status,
                actor_user_id,
                progress_json,
                started_at,
                completed_at,
            )
            .await
    }
}

pub type SqliteWorkflowStore = WorkflowStore<SqliteWorkflowSql>;

#[derive(Clone)]
pub struct SqliteWorkflowSql {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
}

impl SqliteWorkflowStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self::from_sql(SqliteWorkflowSql::new(db))
    }
}

impl SqliteWorkflowSql {
    fn new(db: &SqliteServices) -> Self {
        Self {
            db: db.clone(),
            pool: db.pool().clone(),
        }
    }
}

fn parse_rfc3339_or_now(value: Option<String>) -> chrono::DateTime<chrono::Utc> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now)
}

fn job_run_record_from_workflow(record: crate::WorkflowOperationRecord) -> AppResult<JobRunRecord> {
    let job_key = record
        .job_key
        .as_deref()
        .and_then(JobKey::parse)
        .ok_or_else(|| AppError::Repository("workflow operation missing valid job_key".into()))?;
    let trigger_source = record
        .trigger_source
        .as_deref()
        .and_then(JobTriggerSource::parse)
        .ok_or_else(|| {
            AppError::Repository("workflow operation missing valid trigger_source".into())
        })?;
    let status = JobRunStatus::parse(&record.status)
        .ok_or_else(|| AppError::Repository("workflow operation missing valid status".into()))?;

    Ok(JobRunRecord {
        id: record.id,
        job_key,
        operation_type: record.operation_type,
        status,
        trigger_source,
        actor_user_id: record.actor_user_id,
        progress_json: record.progress_json,
        summary_json: record.summary_json,
        summary_text: record.summary_text,
        error_text: record.error_text,
        started_at: parse_rfc3339_or_now(record.started_at),
        completed_at: record
            .completed_at
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
            .map(|value| value.with_timezone(&chrono::Utc)),
        created_at: parse_rfc3339_or_now(Some(record.created_at)),
        updated_at: parse_rfc3339_or_now(Some(record.updated_at)),
    })
}

#[async_trait]
impl WorkflowSql for SqliteWorkflowSql {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        self.db.commit_successful_grab(commit).await
    }
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        self.db.append_domain_event(&event).await
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        self.db.append_domain_events(events).await
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        crate::queries::domain_event::list_domain_events_query(&self.pool, filter).await
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        crate::queries::domain_event::count_title_history_page_events_query(
            &self.pool,
            event_types,
            title_ids,
            download_id,
        )
        .await
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        crate::queries::domain_event::list_title_history_page_events_query(
            &self.pool,
            event_types,
            title_ids,
            download_id,
            limit,
            offset,
        )
        .await
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        crate::queries::domain_event::list_domain_events_after_sequence_query(
            &self.pool,
            after_sequence,
            limit,
        )
        .await
    }

    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        crate::queries::domain_event::delete_domain_events_for_title_ids_query(
            &self.pool, title_ids,
        )
        .await
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        crate::queries::domain_event::get_event_subscriber_offset_query(&self.pool, subscriber)
            .await
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        self.db
            .set_event_subscriber_offset(subscriber, sequence)
            .await
    }
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        self.db.record_download_submission(&submission).await
    }

    async fn find_by_client_item_id(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        crate::queries::workflow::find_download_submission_query(&self.pool, identity).await
    }

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        crate::queries::workflow::list_download_submissions_for_client_items_query(
            &self.pool,
            client_items,
        )
        .await
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        crate::queries::workflow::list_download_submissions_for_title_query(&self.pool, title_id)
            .await
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        crate::queries::workflow::find_download_submission_by_title_and_request_signature_query(
            &self.pool,
            title_id,
            request_signature,
        )
        .await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.db
            .delete_download_submissions_for_title(title_id)
            .await
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        self.db
            .delete_download_submission_by_client_item_id(identity)
            .await
    }

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()> {
        self.db.update_tracked_state(identity, tracked_state).await
    }

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>> {
        crate::queries::workflow::get_tracked_state_query(&self.pool, identity).await
    }
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
        self.db.insert_import_artifact(&artifact).await
    }

    async fn list_by_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Vec<ImportArtifact>> {
        crate::queries::workflow::list_import_artifacts_by_source_identity_query(
            &self.pool, identity,
        )
        .await
    }

    async fn count_by_result_for_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
        result: &str,
    ) -> AppResult<u64> {
        crate::queries::workflow::count_import_artifacts_by_result_for_source_identity_query(
            &self.pool, identity, result,
        )
        .await
    }
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let record = self
            .db
            .create_job_workflow_operation(
                run.operation_type.clone(),
                run.status.as_str().to_string(),
                run.job_key.as_str().to_string(),
                run.trigger_source.as_str().to_string(),
                run.actor_user_id.clone(),
                run.progress_json.clone(),
                run.summary_json.clone(),
                run.summary_text.clone(),
                run.error_text.clone(),
                Some(run.started_at.to_rfc3339()),
                run.completed_at.map(|value| value.to_rfc3339()),
            )
            .await?;

        job_run_record_from_workflow(record)
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let record = self
            .db
            .update_job_workflow_operation(
                &run.id,
                run.status.as_str(),
                run.progress_json.clone(),
                run.summary_json.clone(),
                run.summary_text.clone(),
                run.error_text.clone(),
                run.completed_at.map(|value| value.to_rfc3339()),
            )
            .await?;

        job_run_record_from_workflow(record)
    }

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>> {
        crate::queries::workflow::get_workflow_operation_by_id_query(&self.pool, run_id)
            .await?
            .map(job_run_record_from_workflow)
            .transpose()
    }

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        crate::queries::workflow::list_job_workflow_operations_query(
            &self.pool,
            job_key.map(JobKey::as_str),
            limit as i64,
        )
        .await?
        .into_iter()
        .map(job_run_record_from_workflow)
        .collect()
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        crate::queries::workflow::list_active_job_workflow_operations_query(&self.pool)
            .await?
            .into_iter()
            .map(job_run_record_from_workflow)
            .collect()
    }
    async fn queue_import_request(
        &self,
        source_identity: DownloadSourceIdentity,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        self.db
            .create_import_request(source_identity, import_type, payload_json)
            .await
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        crate::queries::workflow::get_import_by_id_query(&self.pool, id).await
    }

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.db
            .update_import_status(import_id, status, result_json)
            .await
    }

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        self.db
            .recover_stale_processing_imports(stale_seconds)
            .await
    }

    async fn recover_stale_processing_imports_for_type(
        &self,
        import_type: ImportType,
        stale_seconds: i64,
    ) -> AppResult<u64> {
        self.db
            .recover_stale_processing_imports_for_type(import_type, stale_seconds)
            .await
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        crate::queries::workflow::list_pending_imports_query(&self.pool).await
    }

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>> {
        crate::queries::workflow::list_pending_imports_for_type_query(&self.pool, import_type).await
    }

    async fn list_imports_for_identities(
        &self,
        identities: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<ImportRecord>> {
        crate::queries::workflow::list_imports_for_identities_query(&self.pool, identities).await
    }

    async fn is_already_imported(&self, identity: &DownloadSourceIdentity) -> AppResult<bool> {
        let rows_affected = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(1)
             FROM imports
             WHERE COALESCE(source_client_id, '') = ?
               AND source_system = ?
               AND source_ref = ?
               AND status IN ('completed', 'skipped')",
        )
        .bind(identity.client_id_or_empty())
        .bind(&identity.client_type)
        .bind(&identity.item_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

        Ok(rows_affected > 0)
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        crate::queries::workflow::list_imports_query(&self.pool, limit as i64).await
    }
    async fn upsert_external_import_monitor_snapshot(
        &self,
        snapshot: &ExternalImportMonitorSnapshot,
    ) -> AppResult<()> {
        self.db
            .upsert_external_import_monitor_snapshot(snapshot)
            .await
    }

    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        crate::queries::workflow::append_external_import_monitor_snapshot_chunk_query(
            &self.pool, chunk,
        )
        .await
    }

    async fn list_external_import_monitor_snapshot_chunks(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        crate::queries::workflow::list_external_import_monitor_snapshot_chunks_query(
            &self.pool, scope_kind, scope_key, entry_kind,
        )
        .await
    }

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        crate::queries::workflow::list_external_import_monitor_snapshot_chunk_batch_query(
            &self.pool,
            scope_kind,
            scope_key,
            entry_kind,
            after_chunk_index,
            limit,
        )
        .await
    }

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
    ) -> AppResult<()> {
        crate::queries::workflow::delete_external_import_monitor_snapshot_chunks_query(
            &self.pool, scope_kind, scope_key,
        )
        .await
    }

    async fn get_external_import_monitor_snapshot(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Option<ExternalImportMonitorSnapshot>> {
        crate::queries::workflow::get_external_import_monitor_snapshot_query(&self.pool, facet)
            .await
    }

    async fn delete_external_import_monitor_snapshot(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<()> {
        self.db
            .delete_external_import_monitor_snapshot(facet.clone())
            .await
    }
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        self.db
            .queue_delete_download_command(
                client_id,
                client_type,
                download_client_item_id,
                is_history,
                requested_by_user_id,
            )
            .await
    }

    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64> {
        self.db
            .recover_stale_running_delete_download_commands(stale_seconds)
            .await
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        crate::queries::workflow::list_pending_delete_download_commands_query(&self.pool).await
    }

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()> {
        self.db
            .update_delete_download_command_status(id, DownloadQueueDeleteStatus::Running, None)
            .await
    }

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()> {
        self.db
            .update_delete_download_command_status(id, DownloadQueueDeleteStatus::Completed, None)
            .await
    }

    async fn mark_delete_command_failed(
        &self,
        id: &str,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        self.db
            .update_delete_download_command_status(
                id,
                DownloadQueueDeleteStatus::Failed,
                error_text,
            )
            .await
    }

    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(Option<String>, String, String, bool)],
    ) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        crate::queries::workflow::list_latest_delete_download_commands_for_sources_query(
            &self.pool, sources,
        )
        .await
    }

    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32> {
        self.db
            .prune_terminal_delete_download_commands_older_than(days)
            .await
    }
    async fn create_workflow_operation(
        &self,
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<WorkflowOperationInfo> {
        self.db
            .create_workflow_operation(
                operation_type,
                status,
                actor_user_id,
                progress_json,
                started_at,
                completed_at,
            )
            .await
    }
}
