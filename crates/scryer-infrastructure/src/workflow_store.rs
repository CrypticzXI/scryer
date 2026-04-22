use async_trait::async_trait;
use scryer_application::{
    AcquisitionStateRepository, AppError, AppResult, DomainEventRepository,
    DownloadQueueCommandRecord, DownloadQueueCommandRepository, DownloadSubmission,
    DownloadSubmissionRepository, ExternalImportMonitorSnapshot,
    ExternalImportMonitorSnapshotRepository, ImportArtifact, ImportArtifactRepository,
    ImportRepository, JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource,
    SuccessfulGrabCommit, WorkflowOperationInfo, WorkflowOperationRepository,
};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DownloadQueueDeleteStatus, ImportRecord, ImportStatus,
    ImportType, NewDomainEvent,
};

use crate::SqliteServices;

#[derive(Clone)]
pub struct SqliteWorkflowStore {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
}

impl SqliteWorkflowStore {
    pub fn new(db: &SqliteServices) -> Self {
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
impl AcquisitionStateRepository for SqliteWorkflowStore {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        self.db.commit_successful_grab(commit).await
    }
}

#[async_trait]
impl DomainEventRepository for SqliteWorkflowStore {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        self.db.append_domain_event(&event).await
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        self.db.append_domain_events(events).await
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        crate::queries::domain_event::list_domain_events_query(&self.pool, filter).await
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

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        crate::queries::domain_event::get_event_subscriber_offset_query(&self.pool, subscriber)
            .await
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        self.db
            .set_event_subscriber_offset(subscriber, sequence)
            .await
    }
}

#[async_trait]
impl DownloadSubmissionRepository for SqliteWorkflowStore {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        self.db.record_download_submission(&submission).await
    }

    async fn find_by_client_item_id(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        crate::queries::workflow::find_download_submission_query(
            &self.pool,
            download_client_type,
            download_client_item_id,
        )
        .await
    }

    async fn list_for_client_items(
        &self,
        client_items: &[(String, String)],
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

    async fn delete_by_client_item_id(&self, download_client_item_id: &str) -> AppResult<()> {
        self.db
            .delete_download_submission_by_client_item_id(download_client_item_id)
            .await
    }

    async fn update_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
        tracked_state: &str,
    ) -> AppResult<()> {
        self.db
            .update_tracked_state(download_client_type, download_client_item_id, tracked_state)
            .await
    }

    async fn get_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<String>> {
        crate::queries::workflow::get_tracked_state_query(
            &self.pool,
            download_client_type,
            download_client_item_id,
        )
        .await
    }
}

#[async_trait]
impl ImportArtifactRepository for SqliteWorkflowStore {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
        self.db.insert_import_artifact(&artifact).await
    }

    async fn list_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Vec<ImportArtifact>> {
        crate::queries::workflow::list_import_artifacts_by_source_ref_query(
            &self.pool,
            source_system,
            source_ref,
        )
        .await
    }

    async fn count_by_result(
        &self,
        source_system: &str,
        source_ref: &str,
        result: &str,
    ) -> AppResult<u64> {
        crate::queries::workflow::count_import_artifacts_by_result_query(
            &self.pool,
            source_system,
            source_ref,
            result,
        )
        .await
    }
}

#[async_trait]
impl JobRunRepository for SqliteWorkflowStore {
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
}

#[async_trait]
impl ImportRepository for SqliteWorkflowStore {
    async fn queue_import_request(
        &self,
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        self.db
            .create_import_request(source_system, source_ref, import_type, payload_json)
            .await
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        crate::queries::workflow::get_import_by_id_query(&self.pool, id).await
    }

    async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>> {
        crate::queries::workflow::get_import_by_source_ref_query(
            &self.pool,
            source_system,
            source_ref,
        )
        .await
    }

    async fn get_import_by_source_ref_and_type(
        &self,
        source_system: &str,
        source_ref: &str,
        import_type: ImportType,
    ) -> AppResult<Option<ImportRecord>> {
        crate::queries::workflow::get_import_by_source_ref_and_type_query(
            &self.pool,
            source_system,
            source_ref,
            import_type,
        )
        .await
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

    async fn list_imports_for_sources(
        &self,
        sources: &[(String, String)],
    ) -> AppResult<Vec<ImportRecord>> {
        crate::queries::workflow::list_imports_for_sources_query(&self.pool, sources).await
    }

    async fn is_already_imported(&self, source_system: &str, source_ref: &str) -> AppResult<bool> {
        let rows_affected = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(1)
             FROM imports
             WHERE source_system = ?
               AND source_ref = ?
               AND status IN ('completed', 'skipped')",
        )
        .bind(source_system)
        .bind(source_ref)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

        Ok(rows_affected > 0)
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        crate::queries::workflow::list_imports_query(&self.pool, limit as i64).await
    }
}

#[async_trait]
impl ExternalImportMonitorSnapshotRepository for SqliteWorkflowStore {
    async fn upsert_external_import_monitor_snapshot(
        &self,
        snapshot: &ExternalImportMonitorSnapshot,
    ) -> AppResult<()> {
        self.db
            .upsert_external_import_monitor_snapshot(snapshot)
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
}

#[async_trait]
impl DownloadQueueCommandRepository for SqliteWorkflowStore {
    async fn queue_delete_command(
        &self,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        self.db
            .queue_delete_download_command(
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
        sources: &[(String, String, bool)],
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
}

#[async_trait]
impl WorkflowOperationRepository for SqliteWorkflowStore {
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
