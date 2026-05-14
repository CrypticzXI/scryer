use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, DownloadQueueCommandRecord, DownloadSourceIdentity, DownloadSubmission,
    ExternalImportMonitorSnapshot, ImportArtifact, JobKey, JobRunRecord, JobRunStatus,
    JobTriggerSource, SubmissionScope, SuccessfulGrabCommit, WorkflowOperationInfo,
};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventStream, DomainEventType,
    DownloadQueueCommandAction, DownloadQueueDeleteStatus, Id, ImportRecord, ImportStatus,
    ImportType, MediaFacet, NewDomainEvent, TitleHistoryEventType,
};
use sqlx::{QueryBuilder, Row};

use crate::postgres::PostgresServices;
use crate::postgres::timestamp::{parse_optional_rfc3339_timestamp, parse_rfc3339_or_now};
use crate::workflow_store::{WorkflowSql, WorkflowStore};

pub type PostgresWorkflowStore = WorkflowStore<PostgresWorkflowSql>;

#[derive(Clone)]
pub struct PostgresWorkflowSql {
    pool: sqlx::PgPool,
}

impl PostgresWorkflowStore {
    pub fn new(db: &PostgresServices) -> Self {
        Self::from_sql(PostgresWorkflowSql::new(db.pool().clone()))
    }
}

impl PostgresWorkflowSql {
    fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkflowSql for PostgresWorkflowSql {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        self.record_submission(commit.download_submission.clone())
            .await?;
        let last_search_at = parse_optional_rfc3339_timestamp(
            commit.last_search_at.as_deref(),
            "wanted_items.last_search_at",
        )?;
        let grabbed_at = parse_optional_rfc3339_timestamp(
            commit.grabbed_at.as_deref(),
            "pending_releases.grabbed_at",
        )?;

        let mut wanted_ids = Vec::with_capacity(commit.covered_wanted_item_ids.len() + 1);
        wanted_ids.push(commit.wanted_item_id.clone());
        wanted_ids.extend(commit.covered_wanted_item_ids.iter().cloned());
        wanted_ids.sort();
        wanted_ids.dedup();

        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        sqlx::query(
            "UPDATE wanted_items
                SET status = 'grabbed',
                    next_search_at = NULL,
                    last_search_at = $2::timestamptz,
                    search_count = $3,
                    current_score = $4,
                    grabbed_release = $5,
                    updated_at = NOW()
              WHERE id = ANY($1)",
        )
        .bind(&wanted_ids)
        .bind(last_search_at)
        .bind(commit.search_count)
        .bind(commit.current_score)
        .bind(&commit.grabbed_release)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;

        if let Some(pending_id) = commit.grabbed_pending_release_id.as_deref() {
            sqlx::query(
                "UPDATE pending_releases
                    SET status = 'grabbed',
                        grabbed_at = COALESCE($2::timestamptz, grabbed_at)
                  WHERE id = $1",
            )
            .bind(pending_id)
            .bind(grabbed_at)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;

            sqlx::query(
                "UPDATE pending_releases
                    SET status = 'superseded'
                  WHERE wanted_item_id = $1
                    AND id <> $2
                    AND status IN ('waiting', 'standby')",
            )
            .bind(&commit.wanted_item_id)
            .bind(pending_id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }

        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        let payload = serde_json::to_value(&event.payload).map_err(repo_err)?;
        let stream_kind = event.stream.kind();
        let stream_id = event.stream.identifier();
        let event_type = event.payload.event_type().as_str();
        let row = sqlx::query(
            "INSERT INTO domain_events
             (event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id,
              schema_version, stream_kind, stream_id, event_type, payload_json)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::jsonb)
             RETURNING sequence",
        )
        .bind(&event.event_id)
        .bind(event.occurred_at)
        .bind(&event.actor_user_id)
        .bind(&event.title_id)
        .bind(event.facet.as_ref().map(MediaFacet::as_str))
        .bind(&event.correlation_id)
        .bind(&event.causation_id)
        .bind(i64::from(event.schema_version))
        .bind(stream_kind)
        .bind(stream_id)
        .bind(event_type)
        .bind(payload)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(DomainEvent {
            sequence: row.try_get("sequence").map_err(repo_err)?,
            event_id: event.event_id,
            occurred_at: event.occurred_at,
            actor_user_id: event.actor_user_id,
            title_id: event.title_id,
            facet: event.facet,
            correlation_id: event.correlation_id,
            causation_id: event.causation_id,
            schema_version: event.schema_version,
            stream: event.stream,
            payload: event.payload,
        })
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        let mut out = Vec::with_capacity(events.len());
        for event in events {
            out.push(self.append(event).await?);
        }
        Ok(out)
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        let mut builder = QueryBuilder::<sqlx::Postgres>::new(
            "SELECT sequence, event_id, occurred_at, actor_user_id, title_id, facet,
                    correlation_id, causation_id, schema_version, stream_kind, stream_id,
                    payload_json
             FROM domain_events WHERE TRUE",
        );
        if let Some(types) = filter.event_types.as_ref() {
            builder.push(" AND event_type = ANY(");
            builder.push_bind(types.iter().map(|value| value.as_str()).collect::<Vec<_>>());
            builder.push(")");
        }
        if let Some(title_id) = filter.title_id.as_ref() {
            builder.push(" AND title_id = ");
            builder.push_bind(title_id);
        }
        if let Some(facet) = filter.facet.as_ref() {
            builder.push(" AND facet = ");
            builder.push_bind(facet.as_str());
        }
        if let Some(after) = filter.after_sequence {
            builder.push(" AND sequence > ");
            builder.push_bind(after);
        }
        if let Some(before) = filter.before_sequence {
            builder.push(" AND sequence < ");
            builder.push_bind(before);
        }
        builder.push(" ORDER BY sequence ASC LIMIT ");
        builder.push_bind(filter.limit.max(1) as i64);
        let rows = builder
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(domain_event_from_row).collect()
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        let mut builder =
            QueryBuilder::<sqlx::Postgres>::new("SELECT COUNT(*) FROM domain_events WHERE TRUE");
        push_title_history_filters(&mut builder, event_types, title_ids, download_id);
        sqlx_count(builder, &self.pool).await
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let mut builder = QueryBuilder::<sqlx::Postgres>::new(
            "SELECT sequence, event_id, occurred_at, actor_user_id, title_id, facet,
                    correlation_id, causation_id, schema_version, stream_kind, stream_id,
                    payload_json
             FROM domain_events WHERE TRUE",
        );
        push_title_history_filters(&mut builder, event_types, title_ids, download_id);
        builder.push(" ORDER BY occurred_at DESC, sequence DESC LIMIT ");
        builder.push_bind(limit as i64);
        builder.push(" OFFSET ");
        builder.push_bind(offset as i64);
        let rows = builder
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(domain_event_from_row).collect()
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let rows = sqlx::query(
            "SELECT sequence, event_id, occurred_at, actor_user_id, title_id, facet,
                    correlation_id, causation_id, schema_version, stream_kind, stream_id,
                    payload_json
             FROM domain_events
             WHERE sequence > $1
             ORDER BY sequence ASC
             LIMIT $2",
        )
        .bind(after_sequence)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(domain_event_from_row).collect()
    }

    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if title_ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query("DELETE FROM domain_events WHERE title_id = ANY($1)")
            .bind(title_ids)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT sequence FROM event_subscriber_offsets WHERE subscriber_name = $1",
        )
        .bind(subscriber)
        .fetch_optional(&self.pool)
        .await
        .map(|value| value.unwrap_or(0))
        .map_err(repo_err)
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO event_subscriber_offsets (subscriber_name, sequence, updated_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (subscriber_name) DO UPDATE SET
                sequence = EXCLUDED.sequence,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(subscriber)
        .bind(sequence)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        let (episode_id, collection_id) = persisted_submission_scope(&submission.scope);
        let episode_set_ids = persisted_episode_set_ids(&submission.scope).join("\u{1f}");
        sqlx::query(
            "INSERT INTO download_submissions
             (id, title_id, facet, download_client_id, download_client_type,
              download_client_item_id, source_title, submitted_at, collection_id, tracked_state,
              tracked_state_at, source_hint, source_kind, request_signature, episode_id,
              episode_set_ids)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), $8, NULL, NULL, $9, $10, $11, $12, $13)
             ON CONFLICT (download_client_id, download_client_type, download_client_item_id)
             DO UPDATE SET
                title_id = EXCLUDED.title_id,
                facet = EXCLUDED.facet,
                source_title = EXCLUDED.source_title,
                collection_id = EXCLUDED.collection_id,
                source_hint = EXCLUDED.source_hint,
                source_kind = EXCLUDED.source_kind,
                request_signature = EXCLUDED.request_signature,
                episode_id = EXCLUDED.episode_id,
                episode_set_ids = EXCLUDED.episode_set_ids",
        )
        .bind(Id::new().0)
        .bind(&submission.title_id)
        .bind(&submission.facet)
        .bind(normalize_download_client_id(
            submission.download_client_id.as_deref(),
        ))
        .bind(&submission.download_client_type)
        .bind(&submission.download_client_item_id)
        .bind(&submission.source_title)
        .bind(collection_id)
        .bind(&submission.source_hint)
        .bind(submission.source_kind.as_ref().map(|kind| kind.as_str()))
        .bind(&submission.request_signature)
        .bind(episode_id)
        .bind(if episode_set_ids.is_empty() {
            None::<String>
        } else {
            Some(episode_set_ids)
        })
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn find_by_client_item_id(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        let sql = download_submission_select_where(
            "COALESCE(download_client_id, '') = $1 AND download_client_type = $2 AND download_client_item_id = $3",
        );
        let row = sqlx::query(&sql)
            .bind(normalize_download_client_id(identity.client_id.as_deref()))
            .bind(&identity.client_type)
            .bind(&identity.item_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(download_submission_from_row).transpose()
    }

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        let mut out = Vec::new();
        for identity in client_items {
            if let Some(submission) = self.find_by_client_item_id(identity).await? {
                out.push(submission);
            }
        }
        Ok(out)
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        let sql = download_submission_select_where("title_id = $1");
        let rows = sqlx::query(&sql)
            .bind(title_id)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(download_submission_from_row).collect()
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        let sql = download_submission_select_where("title_id = $1 AND request_signature = $2");
        let row = sqlx::query(&sql)
            .bind(title_id)
            .bind(request_signature)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(download_submission_from_row).transpose()
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM download_submissions WHERE title_id = $1")
            .bind(title_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM download_submissions
             WHERE COALESCE(download_client_id, '') = $1
               AND download_client_type = $2
               AND download_client_item_id = $3",
        )
        .bind(normalize_download_client_id(identity.client_id.as_deref()))
        .bind(&identity.client_type)
        .bind(&identity.item_id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE download_submissions
             SET tracked_state = $4, tracked_state_at = NOW()
             WHERE COALESCE(download_client_id, '') = $1
               AND download_client_type = $2
               AND download_client_item_id = $3",
        )
        .bind(normalize_download_client_id(identity.client_id.as_deref()))
        .bind(&identity.client_type)
        .bind(&identity.item_id)
        .bind(tracked_state)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>> {
        sqlx::query_scalar(
            "SELECT tracked_state
             FROM download_submissions
             WHERE COALESCE(download_client_id, '') = $1
               AND download_client_type = $2
               AND download_client_item_id = $3",
        )
        .bind(normalize_download_client_id(identity.client_id.as_deref()))
        .bind(&identity.client_type)
        .bind(&identity.item_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)
    }
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO download_import_artifacts
             (id, source_system, source_ref, import_id, relative_path, normalized_file_name,
              media_kind, title_id, episode_id, season_number, episode_number, result,
              reason_code, imported_media_file_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (id) DO UPDATE SET
                result = EXCLUDED.result,
                reason_code = EXCLUDED.reason_code,
                imported_media_file_id = EXCLUDED.imported_media_file_id",
        )
        .bind(&artifact.id)
        .bind(&artifact.source_system)
        .bind(&artifact.source_ref)
        .bind(&artifact.import_id)
        .bind(&artifact.relative_path)
        .bind(&artifact.normalized_file_name)
        .bind(&artifact.media_kind)
        .bind(&artifact.title_id)
        .bind(&artifact.episode_id)
        .bind(artifact.season_number)
        .bind(artifact.episode_number)
        .bind(&artifact.result)
        .bind(&artifact.reason_code)
        .bind(&artifact.imported_media_file_id)
        .bind(artifact.created_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn list_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Vec<ImportArtifact>> {
        let rows = sqlx::query(
            "SELECT * FROM download_import_artifacts
             WHERE source_system = $1 AND source_ref = $2
             ORDER BY created_at ASC, id ASC",
        )
        .bind(source_system)
        .bind(source_ref)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(import_artifact_from_row).collect()
    }

    async fn count_by_result(
        &self,
        source_system: &str,
        source_ref: &str,
        result: &str,
    ) -> AppResult<u64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_import_artifacts
             WHERE source_system = $1 AND source_ref = $2 AND result = $3",
        )
        .bind(source_system)
        .bind(source_ref)
        .bind(result)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(count as u64)
    }
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        upsert_job_run(&self.pool, run).await?;
        Ok(run.clone())
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        upsert_job_run(&self.pool, run).await?;
        Ok(run.clone())
    }

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>> {
        let row = sqlx::query("SELECT * FROM download_jobs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(job_run_from_row).transpose()
    }

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        let rows = if let Some(job_key) = job_key {
            sqlx::query(
                "SELECT * FROM download_jobs WHERE job_key = $1 ORDER BY created_at DESC LIMIT $2",
            )
            .bind(job_key.as_str())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query("SELECT * FROM download_jobs ORDER BY created_at DESC LIMIT $1")
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
        }
        .map_err(repo_err)?;
        rows.iter().map(job_run_from_row).collect()
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        let rows = sqlx::query(
            "SELECT * FROM download_jobs WHERE status IN ('queued', 'running') ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(job_run_from_row).collect()
    }
    async fn queue_import_request(
        &self,
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        let id = Id::new().0;
        let now = Utc::now();
        let payload: serde_json::Value = serde_json::from_str(&payload_json).map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO imports
             (id, source_system, source_ref, import_type, status, payload_json, created_at, updated_at)
             VALUES ($1, $2, $3, $4, 'queued', $5::jsonb, $6, $7)",
        )
        .bind(&id)
        .bind(source_system)
        .bind(source_ref)
        .bind(import_type)
        .bind(payload)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(id)
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        let row = sqlx::query("SELECT * FROM imports WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(import_record_from_row).transpose()
    }

    async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>> {
        let row = sqlx::query(
            "SELECT * FROM imports WHERE source_system = $1 AND source_ref = $2 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(source_system)
        .bind(source_ref)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(import_record_from_row).transpose()
    }

    async fn get_import_by_source_ref_and_type(
        &self,
        source_system: &str,
        source_ref: &str,
        import_type: ImportType,
    ) -> AppResult<Option<ImportRecord>> {
        let row = sqlx::query(
            "SELECT * FROM imports
             WHERE source_system = $1 AND source_ref = $2 AND import_type = $3
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(source_system)
        .bind(source_ref)
        .bind(import_type.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(import_record_from_row).transpose()
    }

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        let result_json = result_json
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .map_err(repo_err)?;
        sqlx::query(
            "UPDATE imports
             SET status = $2,
                 result_json = $3::jsonb,
                 started_at = CASE WHEN $2 = 'processing' AND started_at IS NULL THEN NOW() ELSE started_at END,
                 finished_at = CASE WHEN $2 IN ('completed', 'failed', 'skipped') THEN NOW() ELSE finished_at END,
                 updated_at = NOW()
             WHERE id = $1",
        )
        .bind(import_id)
        .bind(status.as_str())
        .bind(result_json)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        recover_stale_imports(&self.pool, None, stale_seconds).await
    }

    async fn recover_stale_processing_imports_for_type(
        &self,
        import_type: ImportType,
        stale_seconds: i64,
    ) -> AppResult<u64> {
        recover_stale_imports(&self.pool, Some(import_type), stale_seconds).await
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        list_imports_where(
            &self.pool,
            "status IN ('queued', 'processing')",
            vec![],
            500,
        )
        .await
    }

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>> {
        let rows = sqlx::query(
            "SELECT * FROM imports
             WHERE status IN ('queued', 'processing') AND import_type = $1
             ORDER BY created_at ASC",
        )
        .bind(import_type.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(import_record_from_row).collect()
    }

    async fn list_imports_for_sources(
        &self,
        sources: &[(String, String)],
    ) -> AppResult<Vec<ImportRecord>> {
        let mut out = Vec::new();
        for (source_system, source_ref) in sources {
            if let Some(record) = self
                .get_import_by_source_ref(source_system, source_ref)
                .await?
            {
                out.push(record);
            }
        }
        Ok(out)
    }

    async fn is_already_imported(&self, source_system: &str, source_ref: &str) -> AppResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM imports
             WHERE source_system = $1 AND source_ref = $2 AND status IN ('completed', 'skipped')",
        )
        .bind(source_system)
        .bind(source_ref)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(count > 0)
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        let rows = sqlx::query("SELECT * FROM imports ORDER BY created_at DESC LIMIT $1")
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(import_record_from_row).collect()
    }
    async fn upsert_external_import_monitor_snapshot(
        &self,
        snapshot: &ExternalImportMonitorSnapshot,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO external_import_monitor_snapshots (facet, payload_json, created_at)
             VALUES ($1, $2::jsonb, $3)
             ON CONFLICT (facet) DO UPDATE SET
                payload_json = EXCLUDED.payload_json,
                created_at = EXCLUDED.created_at",
        )
        .bind(snapshot.facet.as_str())
        .bind(serde_json::to_value(&snapshot.payload).map_err(repo_err)?)
        .bind(parse_rfc3339_or_now(&snapshot.created_at))
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn get_external_import_monitor_snapshot(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Option<ExternalImportMonitorSnapshot>> {
        let row = sqlx::query(
            "SELECT facet, payload_json, created_at
             FROM external_import_monitor_snapshots
             WHERE facet = $1",
        )
        .bind(facet.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(snapshot_from_row).transpose()
    }

    async fn delete_external_import_monitor_snapshot(&self, facet: &MediaFacet) -> AppResult<()> {
        sqlx::query("DELETE FROM external_import_monitor_snapshots WHERE facet = $1")
            .bind(facet.as_str())
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        let id = Id::new().0;
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO download_queue_commands
             (id, action, client_id, client_type, download_client_item_id, is_history, status,
              requested_by_user_id, created_at, updated_at)
             VALUES ($1, 'delete', $2, $3, $4, $5, 'queued', $6, $7, $8)
             ON CONFLICT (action, client_id, client_type, download_client_item_id, is_history)
             WHERE status IN ('queued', 'running')
             DO NOTHING",
        )
        .bind(&id)
        .bind(normalize_download_client_id(client_id))
        .bind(client_type)
        .bind(download_client_item_id)
        .bind(is_history)
        .bind(requested_by_user_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;

        let row = sqlx::query(
            "SELECT * FROM download_queue_commands
             WHERE action = 'delete'
               AND COALESCE(client_id, '') = $1
               AND client_type = $2
               AND download_client_item_id = $3
               AND is_history = $4
               AND status IN ('queued', 'running')
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
        )
        .bind(normalize_download_client_id(client_id))
        .bind(client_type)
        .bind(download_client_item_id)
        .bind(is_history)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        download_queue_command_from_row(&row)
    }

    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64> {
        let result = sqlx::query(
            "UPDATE download_queue_commands
             SET status = 'queued',
                 error_text = NULL,
                 started_at = NULL,
                 finished_at = NULL,
                 updated_at = NOW()
             WHERE action = 'delete'
               AND status = 'running'
               AND updated_at <= NOW() - ($1::text || ' seconds')::interval",
        )
        .bind(stale_seconds)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected())
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        let rows = sqlx::query(
            "SELECT * FROM download_queue_commands
             WHERE action = 'delete' AND status = 'queued'
             ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(download_queue_command_from_row).collect()
    }

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()> {
        update_delete_command_status(&self.pool, id, DownloadQueueDeleteStatus::Running, None).await
    }

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()> {
        update_delete_command_status(&self.pool, id, DownloadQueueDeleteStatus::Completed, None)
            .await
    }

    async fn mark_delete_command_failed(
        &self,
        id: &str,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        update_delete_command_status(
            &self.pool,
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
        let mut out = Vec::new();
        for (client_id, client_type, item_id, is_history) in sources {
            let row = sqlx::query(
                "SELECT * FROM download_queue_commands
                 WHERE action = 'delete'
                   AND COALESCE(client_id, '') = $1
                   AND client_type = $2
                   AND download_client_item_id = $3
                   AND is_history = $4
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1",
            )
            .bind(normalize_download_client_id(client_id.as_deref()))
            .bind(client_type)
            .bind(item_id)
            .bind(*is_history)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
            if let Some(row) = row {
                out.push(download_queue_command_from_row(&row)?);
            }
        }
        Ok(out)
    }

    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32> {
        let result = sqlx::query(
            "DELETE FROM download_queue_commands
             WHERE action = 'delete'
               AND status IN ('completed', 'failed')
               AND updated_at < NOW() - ($1::text || ' days')::interval",
        )
        .bind(days)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
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
        let id = Id::new().0;
        let now = Utc::now();
        let progress_json = progress_json
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .map_err(repo_err)?;
        let row = sqlx::query(
            "INSERT INTO workflow_operations
             (id, operation_type, status, actor_user_id, progress_json, started_at, completed_at,
              created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7, $8, $9)
             RETURNING *",
        )
        .bind(&id)
        .bind(operation_type)
        .bind(status)
        .bind(actor_user_id)
        .bind(progress_json)
        .bind(started_at.as_deref().map(parse_rfc3339_or_now))
        .bind(completed_at.as_deref().map(parse_rfc3339_or_now))
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        workflow_operation_from_row(&row)
    }
}

fn persisted_submission_scope(scope: &SubmissionScope) -> (Option<&str>, Option<&str>) {
    (
        scope.persisted_episode_id(),
        scope.persisted_collection_id(),
    )
}

fn persisted_episode_set_ids(scope: &SubmissionScope) -> &[String] {
    match scope {
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids.as_slice(),
        _ => &[],
    }
}

fn normalize_download_client_id(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

fn download_submission_select_where(where_clause: &str) -> String {
    format!(
        "SELECT title_id, facet, download_client_id, download_client_type, download_client_item_id,
                source_hint, source_kind, source_title, request_signature, episode_id,
                collection_id, episode_set_ids
         FROM download_submissions WHERE {where_clause}"
    )
}

fn download_submission_from_row(row: &sqlx::postgres::PgRow) -> AppResult<DownloadSubmission> {
    let title_id: String = row.try_get("title_id").map_err(repo_err)?;
    let episode_id = row.try_get("episode_id").unwrap_or(None);
    let collection_id = row.try_get("collection_id").unwrap_or(None);
    let episode_set_ids = row
        .try_get::<Option<String>, _>("episode_set_ids")
        .ok()
        .flatten()
        .map(|raw| {
            raw.split('\u{1f}')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
    Ok(DownloadSubmission {
        scope: SubmissionScope::from_persisted(
            &title_id,
            episode_id,
            collection_id,
            episode_set_ids,
        ),
        title_id,
        facet: row.try_get("facet").map_err(repo_err)?,
        download_client_id: row
            .try_get::<Option<String>, _>("download_client_id")
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty()),
        download_client_type: row.try_get("download_client_type").map_err(repo_err)?,
        download_client_item_id: row.try_get("download_client_item_id").map_err(repo_err)?,
        source_hint: row.try_get("source_hint").map_err(repo_err)?,
        source_kind: row
            .try_get::<Option<String>, _>("source_kind")
            .map_err(repo_err)?
            .as_deref()
            .and_then(scryer_application::DownloadSourceKind::parse),
        source_title: row.try_get("source_title").map_err(repo_err)?,
        request_signature: row.try_get("request_signature").map_err(repo_err)?,
    })
}

fn domain_event_from_row(row: &sqlx::postgres::PgRow) -> AppResult<DomainEvent> {
    let facet_raw: Option<String> = row.try_get("facet").map_err(repo_err)?;
    let facet = match facet_raw.as_deref() {
        Some(raw) => Some(
            MediaFacet::parse(raw)
                .ok_or_else(|| AppError::Repository("invalid domain event facet".to_string()))?,
        ),
        None => None,
    };
    let payload_value: serde_json::Value = row.try_get("payload_json").map_err(repo_err)?;
    let payload: DomainEventPayload = serde_json::from_value(payload_value).map_err(repo_err)?;
    let stream_kind: String = row.try_get("stream_kind").map_err(repo_err)?;
    let stream_id: Option<String> = row.try_get("stream_id").map_err(repo_err)?;
    Ok(DomainEvent {
        sequence: row.try_get("sequence").map_err(repo_err)?,
        event_id: row.try_get("event_id").map_err(repo_err)?,
        occurred_at: row.try_get("occurred_at").map_err(repo_err)?,
        actor_user_id: row.try_get("actor_user_id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        facet,
        correlation_id: row.try_get("correlation_id").map_err(repo_err)?,
        causation_id: row.try_get("causation_id").map_err(repo_err)?,
        schema_version: row.try_get::<i64, _>("schema_version").map_err(repo_err)? as i32,
        stream: domain_event_stream(&stream_kind, stream_id),
        payload,
    })
}

fn domain_event_stream(kind: &str, id: Option<String>) -> DomainEventStream {
    match (kind, id) {
        ("title", Some(title_id)) => DomainEventStream::Title { title_id },
        ("library_scan", Some(session_id)) => DomainEventStream::LibraryScan { session_id },
        ("job_run", Some(run_id)) => DomainEventStream::JobRun { run_id },
        ("download_queue_item", Some(item_id)) => DomainEventStream::DownloadQueueItem { item_id },
        _ => DomainEventStream::Global,
    }
}

fn push_title_history_filters<'a>(
    builder: &mut QueryBuilder<'a, sqlx::Postgres>,
    event_types: Option<&[TitleHistoryEventType]>,
    title_ids: Option<&'a [String]>,
    download_id: Option<&'a str>,
) {
    if let Some(event_types) = event_types {
        let event_types = event_types
            .iter()
            .filter_map(title_history_domain_event_type)
            .map(DomainEventType::as_str)
            .collect::<Vec<_>>();
        builder.push(" AND event_type = ANY(");
        builder.push_bind(event_types);
        builder.push(")");
    }
    if let Some(title_ids) = title_ids
        && !title_ids.is_empty()
    {
        builder.push(" AND title_id = ANY(");
        builder.push_bind(title_ids);
        builder.push(")");
    }
    if let Some(download_id) = download_id {
        builder.push(" AND payload_json::text ILIKE ");
        builder.push_bind(format!("%{download_id}%"));
    }
}

async fn sqlx_count(
    mut builder: QueryBuilder<'_, sqlx::Postgres>,
    pool: &sqlx::PgPool,
) -> AppResult<i64> {
    let row = builder.build().fetch_one(pool).await.map_err(repo_err)?;
    row.try_get::<i64, _>(0).map_err(repo_err)
}

fn title_history_domain_event_type(event_type: &TitleHistoryEventType) -> Option<DomainEventType> {
    match event_type {
        TitleHistoryEventType::Grabbed => Some(DomainEventType::ReleaseGrabbed),
        TitleHistoryEventType::DownloadFailed => Some(DomainEventType::DownloadFailed),
        TitleHistoryEventType::Blocklisted => Some(DomainEventType::ReleaseBlocklisted),
        TitleHistoryEventType::Imported => Some(DomainEventType::ImportCompleted),
        TitleHistoryEventType::ImportFailed | TitleHistoryEventType::ImportSkipped => {
            Some(DomainEventType::ImportRejected)
        }
        TitleHistoryEventType::FileDeleted => Some(DomainEventType::MediaFileDeleted),
        TitleHistoryEventType::FileRenamed => Some(DomainEventType::MediaFileRenamed),
        TitleHistoryEventType::Rematched => Some(DomainEventType::TitleRematched),
        TitleHistoryEventType::DownloadCompleted | TitleHistoryEventType::DownloadIgnored => None,
    }
}

fn import_record_from_row(row: &sqlx::postgres::PgRow) -> AppResult<ImportRecord> {
    let import_type_raw: String = row.try_get("import_type").map_err(repo_err)?;
    let status_raw: String = row.try_get("status").map_err(repo_err)?;
    Ok(ImportRecord {
        id: row.try_get("id").map_err(repo_err)?,
        source_system: row.try_get("source_system").map_err(repo_err)?,
        source_ref: row.try_get("source_ref").map_err(repo_err)?,
        import_type: ImportType::parse(&import_type_raw).ok_or_else(|| {
            AppError::Repository(format!("unknown import_type: {import_type_raw}"))
        })?,
        status: ImportStatus::parse(&status_raw).unwrap_or_default(),
        payload_json: json_value_as_string(row.try_get("payload_json").map_err(repo_err)?),
        result_json: row
            .try_get::<Option<serde_json::Value>, _>("result_json")
            .map_err(repo_err)?
            .map(json_value_as_string),
        started_at: row
            .try_get::<Option<DateTime<Utc>>, _>("started_at")
            .map_err(repo_err)?
            .map(|value| value.to_rfc3339()),
        finished_at: row
            .try_get::<Option<DateTime<Utc>>, _>("finished_at")
            .map_err(repo_err)?
            .map(|value| value.to_rfc3339()),
        created_at: row
            .try_get::<DateTime<Utc>, _>("created_at")
            .map_err(repo_err)?
            .to_rfc3339(),
        updated_at: row
            .try_get::<DateTime<Utc>, _>("updated_at")
            .map_err(repo_err)?
            .to_rfc3339(),
    })
}

fn import_artifact_from_row(row: &sqlx::postgres::PgRow) -> AppResult<ImportArtifact> {
    Ok(ImportArtifact {
        id: row.try_get("id").map_err(repo_err)?,
        source_system: row.try_get("source_system").map_err(repo_err)?,
        source_ref: row.try_get("source_ref").map_err(repo_err)?,
        import_id: row.try_get("import_id").map_err(repo_err)?,
        relative_path: row.try_get("relative_path").map_err(repo_err)?,
        normalized_file_name: row.try_get("normalized_file_name").map_err(repo_err)?,
        media_kind: row.try_get("media_kind").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        episode_id: row.try_get("episode_id").map_err(repo_err)?,
        season_number: row.try_get("season_number").map_err(repo_err)?,
        episode_number: row.try_get("episode_number").map_err(repo_err)?,
        result: row.try_get("result").map_err(repo_err)?,
        reason_code: row.try_get("reason_code").map_err(repo_err)?,
        imported_media_file_id: row.try_get("imported_media_file_id").map_err(repo_err)?,
        created_at: row.try_get("created_at").map_err(repo_err)?,
    })
}

fn snapshot_from_row(row: &sqlx::postgres::PgRow) -> AppResult<ExternalImportMonitorSnapshot> {
    let facet_raw: String = row.try_get("facet").map_err(repo_err)?;
    let facet = MediaFacet::parse(&facet_raw)
        .ok_or_else(|| AppError::Repository(format!("invalid snapshot facet: {facet_raw}")))?;
    Ok(ExternalImportMonitorSnapshot {
        facet,
        payload: serde_json::from_value(row.try_get("payload_json").map_err(repo_err)?)
            .map_err(repo_err)?,
        created_at: row
            .try_get::<DateTime<Utc>, _>("created_at")
            .map_err(repo_err)?
            .to_rfc3339(),
    })
}

fn download_queue_command_from_row(
    row: &sqlx::postgres::PgRow,
) -> AppResult<DownloadQueueCommandRecord> {
    let action_raw: String = row.try_get("action").map_err(repo_err)?;
    let status_raw: String = row.try_get("status").map_err(repo_err)?;
    Ok(DownloadQueueCommandRecord {
        id: row.try_get("id").map_err(repo_err)?,
        action: DownloadQueueCommandAction::parse(&action_raw)
            .ok_or_else(|| AppError::Repository(format!("unknown action: {action_raw}")))?,
        client_id: row
            .try_get::<Option<String>, _>("client_id")
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty()),
        client_type: row.try_get("client_type").map_err(repo_err)?,
        download_client_item_id: row.try_get("download_client_item_id").map_err(repo_err)?,
        is_history: row.try_get("is_history").map_err(repo_err)?,
        status: DownloadQueueDeleteStatus::parse(&status_raw)
            .ok_or_else(|| AppError::Repository(format!("unknown status: {status_raw}")))?,
        error_text: row.try_get("error_text").map_err(repo_err)?,
        requested_by_user_id: row.try_get("requested_by_user_id").map_err(repo_err)?,
        started_at: row
            .try_get::<Option<DateTime<Utc>>, _>("started_at")
            .map_err(repo_err)?
            .map(|value| value.to_rfc3339()),
        finished_at: row
            .try_get::<Option<DateTime<Utc>>, _>("finished_at")
            .map_err(repo_err)?
            .map(|value| value.to_rfc3339()),
        created_at: row
            .try_get::<DateTime<Utc>, _>("created_at")
            .map_err(repo_err)?
            .to_rfc3339(),
        updated_at: row
            .try_get::<DateTime<Utc>, _>("updated_at")
            .map_err(repo_err)?
            .to_rfc3339(),
    })
}

async fn update_delete_command_status(
    pool: &sqlx::PgPool,
    id: &str,
    status: DownloadQueueDeleteStatus,
    error_text: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE download_queue_commands
         SET status = $2,
             error_text = $3,
             started_at = CASE WHEN $2 = 'running' THEN NOW() ELSE started_at END,
             finished_at = CASE WHEN $2 IN ('completed', 'failed') THEN NOW() ELSE finished_at END,
             updated_at = NOW()
         WHERE id = $1",
    )
    .bind(id)
    .bind(status.as_str())
    .bind(error_text)
    .execute(pool)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn upsert_job_run(pool: &sqlx::PgPool, run: &JobRunRecord) -> AppResult<()> {
    let progress_json = run
        .progress_json
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .map_err(repo_err)?;
    let summary_json = run
        .summary_json
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .map_err(repo_err)?;
    sqlx::query(
        "INSERT INTO download_jobs
         (id, job_key, status, started_at, completed_at, progress_json, summary_json,
          error_text, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8, $9, $10)
         ON CONFLICT (id) DO UPDATE SET
            status = EXCLUDED.status,
            completed_at = EXCLUDED.completed_at,
            progress_json = EXCLUDED.progress_json,
            summary_json = EXCLUDED.summary_json,
            error_text = EXCLUDED.error_text,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(&run.id)
    .bind(run.job_key.as_str())
    .bind(run.status.as_str())
    .bind(run.started_at)
    .bind(run.completed_at)
    .bind(progress_json)
    .bind(summary_json)
    .bind(&run.error_text)
    .bind(run.created_at)
    .bind(run.updated_at)
    .execute(pool)
    .await
    .map_err(repo_err)?;
    Ok(())
}

fn job_run_from_row(row: &sqlx::postgres::PgRow) -> AppResult<JobRunRecord> {
    let job_key_raw: String = row.try_get("job_key").map_err(repo_err)?;
    let status_raw: String = row.try_get("status").map_err(repo_err)?;
    Ok(JobRunRecord {
        id: row.try_get("id").map_err(repo_err)?,
        job_key: JobKey::parse(&job_key_raw)
            .ok_or_else(|| AppError::Repository(format!("invalid job_key: {job_key_raw}")))?,
        operation_type: job_key_raw.clone(),
        status: JobRunStatus::parse(&status_raw)
            .ok_or_else(|| AppError::Repository(format!("invalid job status: {status_raw}")))?,
        trigger_source: JobTriggerSource::SystemInternal,
        actor_user_id: None,
        progress_json: row
            .try_get::<Option<serde_json::Value>, _>("progress_json")
            .map_err(repo_err)?
            .map(json_value_as_string),
        summary_json: row
            .try_get::<Option<serde_json::Value>, _>("summary_json")
            .map_err(repo_err)?
            .map(json_value_as_string),
        summary_text: None,
        error_text: row.try_get("error_text").map_err(repo_err)?,
        started_at: row.try_get("started_at").map_err(repo_err)?,
        completed_at: row.try_get("completed_at").map_err(repo_err)?,
        created_at: row.try_get("created_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn workflow_operation_from_row(row: &sqlx::postgres::PgRow) -> AppResult<WorkflowOperationInfo> {
    Ok(WorkflowOperationInfo {
        id: row.try_get("id").map_err(repo_err)?,
        operation_type: row.try_get("operation_type").map_err(repo_err)?,
        status: row.try_get("status").map_err(repo_err)?,
        actor_user_id: row.try_get("actor_user_id").map_err(repo_err)?,
        progress_json: row
            .try_get::<Option<serde_json::Value>, _>("progress_json")
            .map_err(repo_err)?
            .map(json_value_as_string),
        started_at: row
            .try_get::<Option<DateTime<Utc>>, _>("started_at")
            .map_err(repo_err)?
            .map(|value| value.to_rfc3339()),
        completed_at: row
            .try_get::<Option<DateTime<Utc>>, _>("completed_at")
            .map_err(repo_err)?
            .map(|value| value.to_rfc3339()),
        created_at: row
            .try_get::<DateTime<Utc>, _>("created_at")
            .map_err(repo_err)?
            .to_rfc3339(),
        updated_at: row
            .try_get::<DateTime<Utc>, _>("updated_at")
            .map_err(repo_err)?
            .to_rfc3339(),
    })
}

async fn recover_stale_imports(
    pool: &sqlx::PgPool,
    import_type: Option<ImportType>,
    stale_seconds: i64,
) -> AppResult<u64> {
    let mut builder = QueryBuilder::<sqlx::Postgres>::new(
        "UPDATE imports
         SET status = 'queued', updated_at = NOW()
         WHERE status = 'processing'
           AND updated_at <= NOW() - (",
    );
    builder.push_bind(stale_seconds.to_string());
    builder.push(" || ' seconds')::interval");
    if let Some(import_type) = import_type {
        builder.push(" AND import_type = ");
        builder.push_bind(import_type.as_str());
    }
    let result = builder.build().execute(pool).await.map_err(repo_err)?;
    Ok(result.rows_affected())
}

async fn list_imports_where(
    pool: &sqlx::PgPool,
    where_clause: &str,
    _binds: Vec<String>,
    limit: usize,
) -> AppResult<Vec<ImportRecord>> {
    let sql =
        format!("SELECT * FROM imports WHERE {where_clause} ORDER BY created_at ASC LIMIT $1");
    let rows = sqlx::query(&sql)
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .map_err(repo_err)?;
    rows.iter().map(import_record_from_row).collect()
}

fn json_value_as_string(value: serde_json::Value) -> String {
    value.to_string()
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}
