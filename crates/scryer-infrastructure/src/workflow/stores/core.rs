use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AcquisitionStateRepository, AppError, AppResult, DomainEventRepository,
    DownloadQueueCommandRecord, DownloadQueueCommandRepository, DownloadSourceIdentity,
    DownloadSubmission, DownloadSubmissionRepository, ExternalImportMonitorSnapshotChunk,
    ExternalImportMonitorSnapshotEntryKind,
    ExternalImportMonitorSnapshotRepository, ImportArtifact, ImportArtifactRepository,
    ImportRepository, JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource,
    PendingReleaseStatus, SubmissionScope, SuccessfulGrabCommit, WantedStatus,
    WorkflowOperationInfo, WorkflowOperationRepository,
};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventStream, DomainEventType, DownloadQueueCommandAction,
    DownloadQueueDeleteStatus, Id, ImportRecord, ImportStatus, ImportType, MediaFacet,
    NewDomainEvent, TitleHistoryEventType,
};
use sqlx::{Row, types::Json};

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use crate::sqlite_services::SqliteServices;
use crate::types::WorkflowOperationRecord;

const DOMAIN_EVENT_COLUMNS: &str = "sequence, event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id, schema_version, stream_kind, stream_id, payload_json";
const DOWNLOAD_SUBMISSION_COLUMNS: &str = "title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id";
const IMPORT_COLUMNS: &str = "id, source_client_id, source_system, source_ref, import_type, status, payload_json, result_json, started_at, finished_at, created_at, updated_at";
const DOWNLOAD_QUEUE_COMMAND_COLUMNS: &str = "id, action, client_id, client_type, download_client_item_id, is_history, status, error_text, requested_by_user_id, started_at, finished_at, created_at, updated_at";

#[derive(Clone)]
pub struct DomainEventStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct AcquisitionStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct DownloadSubmissionStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct ImportStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct ExternalImportMonitorStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct DownloadQueueCommandStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct WorkflowOperationStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
struct NewWorkflowOperation {
    operation_type: String,
    status: String,
    job_key: Option<String>,
    trigger_source: Option<String>,
    actor_user_id: Option<String>,
    progress_json: Option<String>,
    summary_json: Option<String>,
    summary_text: Option<String>,
    error_text: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
}

macro_rules! impl_store_new {
    ($store:ident) => {
        impl $store {
            pub(crate) fn new(datastore: StoreDatastore) -> Self {
                Self { datastore }
            }

            pub fn from_sqlite_services(db: &SqliteServices) -> Self {
                Self::new(StoreDatastore::Sqlite {
                    pool: db.pool().clone(),
                    writer_gate: db.writer_gate(),
                })
            }

            pub fn from_postgres_services(db: &crate::postgres::PostgresServices) -> Self {
                Self::new(StoreDatastore::Postgres {
                    pool: db.pool().clone(),
                })
            }
        }
    };
}

impl_store_new!(DomainEventStore);
impl_store_new!(AcquisitionStore);
impl_store_new!(DownloadSubmissionStore);
impl_store_new!(ImportStore);
impl_store_new!(ExternalImportMonitorStore);
impl_store_new!(DownloadQueueCommandStore);
impl_store_new!(WorkflowOperationStore);

#[async_trait]
impl DomainEventRepository for DomainEventStore {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        append_domain_events(&self.datastore, vec![event])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Repository("failed to append domain event".into()))
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        append_domain_events(&self.datastore, events).await
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        let (sql, args) = build_domain_event_list_sql(filter);
        fetch_domain_events(self.datastore.read_exec(), &sql, &args).await
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        let (where_sql, args) =
            build_title_history_filter_sql(&self.datastore, event_types, title_ids, download_id);
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!("SELECT COUNT(*) AS count FROM domain_events{where_sql}"),
            &args,
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing domain event count".into()))?;
        row.i64("count")
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let page_size = if limit == 0 { 50 } else { limit.min(500) };
        let (where_sql, mut args) =
            build_title_history_filter_sql(&self.datastore, event_types, title_ids, download_id);
        args.push(SqlArg::I64(page_size as i64));
        args.push(SqlArg::I64(offset as i64));
        fetch_domain_events(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events{where_sql} ORDER BY sequence DESC LIMIT {{}} OFFSET {{}}"
            ),
            &args,
        )
        .await
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let filter = DomainEventFilter {
            after_sequence: Some(after_sequence),
            limit,
            ..DomainEventFilter::default()
        };
        self.list(&filter).await
    }

    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if title_ids.is_empty() {
            return Ok(0);
        }
        let mut args = Vec::with_capacity(title_ids.len());
        args.extend(title_ids.iter().cloned().map(SqlArg::Text));
        let rows = execute_write(
            &self.datastore,
            "delete_domain_events_for_title_ids",
            format!(
                "DELETE FROM domain_events WHERE title_id IN ({})",
                placeholders(title_ids.len())
            ),
            args,
        )
        .await?;
        Ok(rows as u32)
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT sequence FROM event_subscriber_offsets WHERE subscriber_name = {}",
            &[SqlArg::Text(subscriber.to_string())],
        )
        .await?;
        Ok(row.map(|row| row.i64("sequence")).transpose()?.unwrap_or(0))
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        let subscriber = subscriber.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "set_event_subscriber_offset", move |tx| {
            let subscriber = subscriber.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO event_subscriber_offsets (subscriber_name, sequence, updated_at)
                         VALUES ({}, {}, {})
                         ON CONFLICT(subscriber_name) DO UPDATE SET
                            sequence = excluded.sequence,
                            updated_at = excluded.updated_at",
                    &[
                        SqlArg::Text(subscriber),
                        SqlArg::I64(sequence),
                        SqlArg::Timestamp(Utc::now()),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }
}

#[async_trait]
impl AcquisitionStateRepository for AcquisitionStore {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        let commit = commit.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "commit_successful_grab", move |tx| {
            let commit = commit.clone();
            Box::pin(async move { commit_successful_grab_tx(tx, &commit).await })
        })
        .await
    }
}

#[async_trait]
impl DownloadSubmissionRepository for DownloadSubmissionStore {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        SqlRuntime::run_in_transaction(&self.datastore, "record_download_submission", move |tx| {
            let submission = submission.clone();
            Box::pin(async move { record_download_submission_tx(tx, &submission).await })
        })
        .await
    }

    async fn find_by_client_item_id(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE download_client_type = {} AND download_client_item_id = {} AND download_client_id = {} LIMIT 1",
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| download_submission_from_row(&row))
            .transpose()
    }

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        let chunks = chunk_download_submission_client_items(client_items);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for chunk in chunks {
            let mut args = Vec::with_capacity(chunk.len() * 3);
            let clauses = chunk
                .iter()
                .map(|identity| {
                    args.push(SqlArg::Text(identity.client_type.clone()));
                    args.push(SqlArg::Text(identity.item_id.clone()));
                    args.push(SqlArg::Text(normalize_download_client_id(
                        identity.client_id.as_deref(),
                    )));
                    "(download_client_type = {} AND download_client_item_id = {} AND download_client_id = {})"
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = download_submission_select_sql(&self.datastore, &format!("WHERE {clauses}"));
            out.extend(fetch_download_submissions(self.datastore.read_exec(), &sql, &args).await?);
        }
        Ok(out)
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        let sql = download_submission_select_sql(&self.datastore, "WHERE title_id = {}");
        fetch_download_submissions(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        let recent_cutoff = Utc::now() - chrono::Duration::seconds(30);
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE title_id = {} AND request_signature = {} AND COALESCE(tracked_state, '') = '' AND submitted_at >= {} ORDER BY submitted_at DESC, id DESC LIMIT 1",
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(request_signature.to_string()),
                SqlArg::Timestamp(recent_cutoff),
            ],
        )
        .await?;
        row.map(|row| download_submission_from_row(&row))
            .transpose()
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_download_submissions_for_title",
            move |tx| {
                let title_id = title_id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submission_episode_links
                         WHERE EXISTS (
                             SELECT 1
                               FROM download_submissions
                              WHERE download_submissions.download_client_id = download_submission_episode_links.download_client_id
                                AND download_submissions.download_client_type = download_submission_episode_links.download_client_type
                                AND download_submissions.download_client_item_id = download_submission_episode_links.download_client_item_id
                                AND download_submissions.title_id = {}
                         )",
                        &[SqlArg::Text(title_id.clone())],
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submissions WHERE title_id = {}",
                        &[SqlArg::Text(title_id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        let identity = identity.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_download_submission_by_client_item_id",
            move |tx| {
                let identity = identity.clone();
                Box::pin(async move {
                    let normalized_client_id =
                        normalize_download_client_id(identity.client_id.as_deref());
                    let args = [
                        SqlArg::Text(normalized_client_id.clone()),
                        SqlArg::Text(identity.client_type.clone()),
                        SqlArg::Text(identity.item_id.clone()),
                    ];
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submission_episode_links
                         WHERE download_client_id = {}
                           AND download_client_type = {}
                           AND download_client_item_id = {}",
                        &args,
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submissions
                         WHERE download_client_id = {}
                           AND download_client_type = {}
                           AND download_client_item_id = {}",
                        &args,
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()> {
        let identity = identity.clone();
        let tracked_state = tracked_state.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_tracked_state", move |tx| {
            let identity = identity.clone();
            let tracked_state = tracked_state.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO download_submissions
                     (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id, tracked_state, tracked_state_at)
                     VALUES ({}, '', '', {}, {}, {}, NULL, NULL, NULL, NULL, NULL, NULL, {}, {})
                     ON CONFLICT(download_client_id, download_client_type, download_client_item_id) DO UPDATE
                     SET tracked_state = excluded.tracked_state,
                         tracked_state_at = excluded.tracked_state_at",
                    &[
                        SqlArg::Text(Id::new().0),
                        SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                        SqlArg::Text(identity.client_type),
                        SqlArg::Text(identity.item_id),
                        SqlArg::Text(tracked_state),
                        SqlArg::Timestamp(Utc::now()),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT tracked_state FROM download_submissions
             WHERE download_client_type = {}
               AND download_client_item_id = {}
               AND download_client_id = {}
             LIMIT 1",
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| row.opt_text("tracked_state"))
            .transpose()
            .map(Option::flatten)
    }
}

#[async_trait]
impl ImportRepository for ImportStore {
    async fn queue_import_request(
        &self,
        source_identity: DownloadSourceIdentity,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        queue_import_request(&self.datastore, source_identity, import_type, payload_json).await
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!("SELECT {IMPORT_COLUMNS} FROM imports WHERE id = {{}} LIMIT 1"),
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.map(|row| import_record_from_row(&row)).transpose()
    }

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        let import_id = import_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_import_status", move |tx| {
            let import_id = import_id.clone();
            let result_json = result_json.clone();
            Box::pin(async move {
                let now = Utc::now();
                let is_terminal = status.is_terminal();
                let result_arg = json_arg_for_tx(tx, result_json.as_deref())?;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE imports
                     SET status = {},
                         result_json = {},
                         started_at = CASE WHEN started_at IS NULL THEN {} ELSE started_at END,
                         finished_at = CASE WHEN {} THEN {} ELSE finished_at END,
                         updated_at = {}
                     WHERE id = {}",
                    &[
                        SqlArg::Text(status.as_str().to_string()),
                        result_arg,
                        SqlArg::Timestamp(now),
                        SqlArg::Bool(is_terminal),
                        SqlArg::Timestamp(now),
                        SqlArg::Timestamp(now),
                        SqlArg::Text(import_id),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        recover_stale_processing_imports(&self.datastore, None, stale_seconds).await
    }

    async fn recover_stale_processing_imports_for_type(
        &self,
        import_type: ImportType,
        stale_seconds: i64,
    ) -> AppResult<u64> {
        recover_stale_processing_imports(&self.datastore, Some(import_type), stale_seconds).await
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        fetch_imports(
            self.datastore.read_exec(),
            &format!(
                "SELECT {IMPORT_COLUMNS} FROM imports
                 WHERE status IN ('queued', 'pending', 'running', 'processing')
                 ORDER BY created_at ASC"
            ),
            &[],
        )
        .await
    }

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>> {
        fetch_imports(
            self.datastore.read_exec(),
            &format!(
                "SELECT {IMPORT_COLUMNS} FROM imports
                 WHERE import_type = {{}}
                   AND status IN ('queued', 'pending', 'running', 'processing')
                 ORDER BY created_at ASC"
            ),
            &[SqlArg::Text(import_type.as_str().to_string())],
        )
        .await
    }

    async fn list_imports_for_identities(
        &self,
        identities: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<ImportRecord>> {
        let identities = dedupe_identities(identities);
        if identities.is_empty() {
            return Ok(Vec::new());
        }
        let mut args = Vec::with_capacity(identities.len() * 3);
        let clauses = identities
            .iter()
            .map(|identity| {
                args.push(SqlArg::Text(normalize_download_client_id(
                    identity.client_id.as_deref(),
                )));
                args.push(SqlArg::Text(identity.client_type.clone()));
                args.push(SqlArg::Text(identity.item_id.clone()));
                "(COALESCE(source_client_id, '') = {} AND source_system = {} AND source_ref = {})"
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        fetch_imports(
            self.datastore.read_exec(),
            &format!(
                "SELECT {IMPORT_COLUMNS} FROM imports WHERE {clauses} ORDER BY updated_at DESC"
            ),
            &args,
        )
        .await
    }

    async fn is_already_imported(&self, identity: &DownloadSourceIdentity) -> AppResult<bool> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(1) AS count
             FROM imports
             WHERE COALESCE(source_client_id, '') = {}
               AND source_system = {}
               AND source_ref = {}
               AND status IN ('completed', 'skipped')",
            &[
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
            ],
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing import count".into()))?;
        Ok(row.i64("count")? > 0)
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        fetch_imports(
            self.datastore.read_exec(),
            &format!("SELECT {IMPORT_COLUMNS} FROM imports ORDER BY created_at DESC LIMIT {{}}"),
            &[SqlArg::I64((limit as i64).clamp(1, 500))],
        )
        .await
    }
}

#[async_trait]
impl ImportArtifactRepository for ImportStore {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
        SqlRuntime::run_in_transaction(&self.datastore, "insert_import_artifact", move |tx| {
            let artifact = artifact.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO download_import_artifacts
                     (id, source_client_id, source_system, source_ref, import_id, relative_path, normalized_file_name,
                      media_kind, title_id, episode_id, season_number, episode_number,
                      result, reason_code, imported_media_file_id, created_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &[
                        SqlArg::Text(artifact.id),
                        SqlArg::OptText(artifact.source_client_id),
                        SqlArg::Text(artifact.source_system),
                        SqlArg::Text(artifact.source_ref),
                        SqlArg::OptText(artifact.import_id),
                        SqlArg::OptText(artifact.relative_path),
                        SqlArg::Text(artifact.normalized_file_name),
                        SqlArg::Text(artifact.media_kind),
                        SqlArg::OptText(artifact.title_id),
                        SqlArg::OptText(artifact.episode_id),
                        SqlArg::OptI32(artifact.season_number),
                        SqlArg::OptI32(artifact.episode_number),
                        SqlArg::Text(artifact.result),
                        SqlArg::OptText(artifact.reason_code),
                        SqlArg::OptText(artifact.imported_media_file_id),
                        SqlArg::Timestamp(artifact.created_at),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn list_by_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Vec<ImportArtifact>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, source_client_id, source_system, source_ref, import_id, relative_path,
                    normalized_file_name, media_kind, title_id, episode_id,
                    season_number, episode_number, result, reason_code,
                    imported_media_file_id, created_at
             FROM download_import_artifacts
             WHERE COALESCE(source_client_id, '') = {} AND source_system = {} AND source_ref = {}
             ORDER BY created_at",
            &[
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
            ],
        )
        .await?
        .into_iter()
        .map(|row| import_artifact_from_row(&row))
        .collect()
    }

    async fn count_by_result_for_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
        result: &str,
    ) -> AppResult<u64> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM download_import_artifacts
             WHERE COALESCE(source_client_id, '') = {} AND source_system = {} AND source_ref = {} AND result = {}",
            &[
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(result.to_string()),
            ],
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing import artifact count".into()))?;
        Ok(row.i64("count")? as u64)
    }
}

#[async_trait]
impl ExternalImportMonitorSnapshotRepository for ExternalImportMonitorStore {
    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        let chunk = chunk.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "append_external_import_monitor_snapshot_chunk",
            move |tx| {
                let chunk = chunk.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO external_import_monitor_snapshot_chunks
                         (facet, entry_kind, chunk_index, payload_ndjson, created_at)
                         VALUES ({}, {}, {}, {}, {})
                         ON CONFLICT(facet, entry_kind, chunk_index) DO UPDATE SET
                             payload_ndjson = excluded.payload_ndjson,
                             created_at = excluded.created_at",
                        &[
                            SqlArg::Text(chunk.facet.as_str().to_string()),
                            SqlArg::Text(chunk.entry_kind.as_str().to_string()),
                            SqlArg::I32(chunk.chunk_index),
                            SqlArg::Text(chunk.payload_ndjson),
                            SqlArg::Timestamp(parse_datetime_or_now(Some(&chunk.created_at))),
                        ],
                    )
                    .await
                    .map_err(map_snapshot_chunk_error)?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        fetch_snapshot_chunks(
            self.datastore.read_exec(),
            "SELECT facet, entry_kind, chunk_index, payload_ndjson, created_at
             FROM external_import_monitor_snapshot_chunks
             WHERE facet = {} AND entry_kind = {} AND ({} IS NULL OR chunk_index > {})
             ORDER BY chunk_index ASC
             LIMIT {}",
            &[
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(entry_kind.as_str().to_string()),
                SqlArg::OptI32(after_chunk_index),
                SqlArg::OptI32(after_chunk_index),
                SqlArg::I32(limit),
            ],
        )
        .await
    }

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        facet: MediaFacet,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_external_import_monitor_snapshot_chunks",
            "DELETE FROM external_import_monitor_snapshot_chunks WHERE facet = {}".to_string(),
            vec![SqlArg::Text(facet.as_str().to_string())],
        )
        .await
        .map_err(map_snapshot_chunk_error)?;
        Ok(())
    }
}

#[async_trait]
impl DownloadQueueCommandRepository for DownloadQueueCommandStore {
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        let client_id = client_id.map(str::to_string);
        let client_type = client_type.to_string();
        let download_client_item_id = download_client_item_id.to_string();
        let requested_by_user_id = requested_by_user_id.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "queue_delete_download_command",
            move |tx| {
                let client_id = client_id.clone();
                let client_type = client_type.clone();
                let download_client_item_id = download_client_item_id.clone();
                let requested_by_user_id = requested_by_user_id.clone();
                Box::pin(async move {
                    let id = Id::new().0;
                    let now = Utc::now();
                    let normalized_client_id = normalize_download_client_id(client_id.as_deref());
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_queue_commands
                         (id, action, client_id, client_type, download_client_item_id, is_history, status, error_text, requested_by_user_id, started_at, finished_at, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, NULL, {}, NULL, NULL, {}, {})
                         ON CONFLICT DO NOTHING",
                        &[
                            SqlArg::Text(id),
                            SqlArg::Text(DownloadQueueCommandAction::Delete.as_str().to_string()),
                            SqlArg::Text(normalized_client_id.clone()),
                            SqlArg::Text(client_type.clone()),
                            SqlArg::Text(download_client_item_id.clone()),
                            SqlArg::Bool(is_history),
                            SqlArg::Text(DownloadQueueDeleteStatus::Queued.as_str().to_string()),
                            SqlArg::OptText(requested_by_user_id),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    fetch_optional_delete_command(
                        SqlExec::Tx(tx),
                        "WHERE action = {}
                           AND COALESCE(client_id, '') = {}
                           AND client_type = {}
                           AND download_client_item_id = {}
                           AND is_history = {}
                           AND status IN ('queued', 'running')
                         ORDER BY created_at DESC, id DESC
                         LIMIT 1",
                        &[
                            SqlArg::Text(DownloadQueueCommandAction::Delete.as_str().to_string()),
                            SqlArg::Text(normalized_client_id),
                            SqlArg::Text(client_type),
                            SqlArg::Text(download_client_item_id),
                            SqlArg::Bool(is_history),
                        ],
                    )
                    .await?
                    .ok_or_else(|| AppError::Repository("failed to load queued delete command".into()))
                })
            },
        )
        .await
    }

    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64> {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(stale_seconds);
        let rows = execute_write(
            &self.datastore,
            "recover_stale_running_delete_download_commands",
            "UPDATE download_queue_commands
             SET status = 'queued',
                 error_text = NULL,
                 started_at = NULL,
                 finished_at = NULL,
                 updated_at = {}
             WHERE action = 'delete'
               AND status = 'running'
               AND updated_at <= {}"
                .to_string(),
            vec![SqlArg::Timestamp(now), SqlArg::Timestamp(cutoff)],
        )
        .await?;
        Ok(rows)
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        fetch_delete_commands(
            self.datastore.read_exec(),
            "WHERE action = 'delete' AND status = 'queued' ORDER BY created_at ASC, id ASC",
            &[],
        )
        .await
    }

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
            id,
            DownloadQueueDeleteStatus::Running,
            None,
        )
        .await
    }

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
            id,
            DownloadQueueDeleteStatus::Completed,
            None,
        )
        .await
    }

    async fn mark_delete_command_failed(
        &self,
        id: &str,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
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
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        let mut args = Vec::new();
        let mut clauses = Vec::with_capacity(sources.len());
        for (client_id, client_type, download_client_item_id, is_history) in sources {
            let normalized_client_id = normalize_download_client_id(client_id.as_deref());
            let client_clause = if normalized_client_id.is_empty() {
                "COALESCE(client_id, '') = ''".to_string()
            } else {
                args.push(SqlArg::Text(normalized_client_id));
                "(COALESCE(client_id, '') = {} OR COALESCE(client_id, '') = '')".to_string()
            };
            args.push(SqlArg::Text(client_type.clone()));
            args.push(SqlArg::Text(download_client_item_id.clone()));
            args.push(SqlArg::Bool(*is_history));
            clauses.push(format!(
                "({client_clause} AND client_type = {{}} AND download_client_item_id = {{}} AND is_history = {{}})"
            ));
        }
        let rows = fetch_delete_commands(
            self.datastore.read_exec(),
            &format!(
                "WHERE action = 'delete' AND ({}) ORDER BY created_at DESC, id DESC",
                clauses.join(" OR ")
            ),
            &args,
        )
        .await?;
        let mut latest = HashMap::new();
        for record in rows {
            let key = (
                record.client_id.clone().unwrap_or_default(),
                record.client_type.clone(),
                record.download_client_item_id.clone(),
                record.is_history,
            );
            latest.entry(key).or_insert(record);
        }
        Ok(latest.into_values().collect())
    }

    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let rows = execute_write(
            &self.datastore,
            "prune_terminal_delete_download_commands_older_than",
            "DELETE FROM download_queue_commands
             WHERE action = 'delete'
               AND status IN ('completed', 'failed')
               AND updated_at < {}"
                .to_string(),
            vec![SqlArg::Timestamp(cutoff)],
        )
        .await?;
        Ok(rows as u32)
    }
}

#[async_trait]
impl WorkflowOperationRepository for WorkflowOperationStore {
    async fn create_workflow_operation(
        &self,
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<WorkflowOperationInfo> {
        let record = create_workflow_operation(
            &self.datastore,
            NewWorkflowOperation {
                operation_type,
                status,
                job_key: None,
                trigger_source: None,
                actor_user_id,
                progress_json,
                summary_json: None,
                summary_text: None,
                error_text: None,
                started_at,
                completed_at,
            },
        )
        .await?;
        Ok(workflow_operation_info(record))
    }
}

#[async_trait]
impl JobRunRepository for WorkflowOperationStore {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let record = create_workflow_operation(
            &self.datastore,
            NewWorkflowOperation {
                operation_type: run.operation_type.clone(),
                status: run.status.as_str().to_string(),
                job_key: Some(run.job_key.as_str().to_string()),
                trigger_source: Some(run.trigger_source.as_str().to_string()),
                actor_user_id: run.actor_user_id.clone(),
                progress_json: run.progress_json.clone(),
                summary_json: run.summary_json.clone(),
                summary_text: run.summary_text.clone(),
                error_text: run.error_text.clone(),
                started_at: Some(run.started_at.to_rfc3339()),
                completed_at: run.completed_at.map(|value| value.to_rfc3339()),
            },
        )
        .await?;
        job_run_record_from_workflow(record)
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let id = run.id.clone();
        let status = run.status.as_str().to_string();
        let progress_json = run.progress_json.clone();
        let summary_json = run.summary_json.clone();
        let summary_text = run.summary_text.clone();
        let error_text = run.error_text.clone();
        let completed_at = run.completed_at.map(|value| value.to_rfc3339());
        let record = SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_job_workflow_operation",
            move |tx| {
                let id = id.clone();
                let status = status.clone();
                let progress_json = progress_json.clone();
                let summary_json = summary_json.clone();
                let summary_text = summary_text.clone();
                let error_text = error_text.clone();
                let completed_at = completed_at.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    let progress_arg = json_arg_for_tx(tx, progress_json.as_deref())?;
                    let summary_arg = json_arg_for_tx(tx, summary_json.as_deref())?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE workflow_operations
                         SET status = {},
                             progress_json = {},
                             summary_json = {},
                             summary_text = {},
                             error_text = {},
                             completed_at = {},
                             updated_at = {}
                         WHERE id = {}",
                        &[
                            SqlArg::Text(status),
                            progress_arg,
                            summary_arg,
                            SqlArg::OptText(summary_text),
                            SqlArg::OptText(error_text),
                            opt_timestamp_arg(completed_at.as_deref()),
                            SqlArg::Timestamp(now),
                            SqlArg::Text(id.clone()),
                        ],
                    )
                    .await?;
                    fetch_optional_workflow_operation(SqlExec::Tx(tx), &id)
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("workflow operation {id}")))
                })
            },
        )
        .await?;
        job_run_record_from_workflow(record)
    }

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>> {
        fetch_optional_workflow_operation(self.datastore.read_exec(), run_id)
            .await?
            .map(job_run_record_from_workflow)
            .transpose()
    }

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        let limit = limit as i64;
        let (sql, args) = if let Some(job_key) = job_key {
            (
                "SELECT * FROM workflow_operations WHERE job_key = {} ORDER BY COALESCE(started_at, created_at) DESC LIMIT {}",
                vec![
                    SqlArg::Text(job_key.as_str().to_string()),
                    SqlArg::I64(limit),
                ],
            )
        } else {
            (
                "SELECT * FROM workflow_operations WHERE job_key IS NOT NULL ORDER BY COALESCE(started_at, created_at) DESC LIMIT {}",
                vec![SqlArg::I64(limit)],
            )
        };
        SqlRuntime::fetch_all(self.datastore.read_exec(), sql, &args)
            .await?
            .into_iter()
            .map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow))
            .collect()
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT * FROM workflow_operations
             WHERE job_key IS NOT NULL
               AND status IN ('queued', 'running', 'discovering')
             ORDER BY COALESCE(started_at, created_at) ASC",
            &[],
        )
        .await?
        .into_iter()
        .map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow))
        .collect()
    }
}

async fn append_domain_events(
    datastore: &StoreDatastore,
    events: Vec<NewDomainEvent>,
) -> AppResult<Vec<DomainEvent>> {
    SqlRuntime::run_in_transaction(datastore, "append_domain_events", move |tx| {
        let events = events.clone();
        Box::pin(async move {
            let mut out = Vec::with_capacity(events.len());
            for event in events {
                let payload = serde_json::to_value(&event.payload).map_err(repo_err)?;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO domain_events (
                        event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id,
                        schema_version, stream_kind, stream_id, event_type, payload_json
                     ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &[
                        SqlArg::Text(event.event_id.clone()),
                        SqlArg::Timestamp(event.occurred_at),
                        SqlArg::OptText(event.actor_user_id.clone()),
                        SqlArg::OptText(event.title_id.clone()),
                        SqlArg::OptText(event.facet.as_ref().map(|facet| facet.as_str().to_string())),
                        SqlArg::OptText(event.correlation_id.clone()),
                        SqlArg::OptText(event.causation_id.clone()),
                        SqlArg::I32(event.schema_version),
                        SqlArg::Text(event.stream.kind().to_string()),
                        SqlArg::OptText(event.stream.identifier().map(str::to_string)),
                        SqlArg::Text(event.payload.event_type().as_str().to_string()),
                        SqlArg::Json(payload),
                    ],
                )
                .await?;
                let stored = fetch_domain_event_by_event_id(SqlExec::Tx(tx), &event.event_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::Repository("failed to reload inserted domain event".into())
                    })?;
                out.push(stored);
            }
            Ok(out)
        })
    })
    .await
}

async fn commit_successful_grab_tx(
    tx: &mut SqlTx<'_>,
    commit: &SuccessfulGrabCommit,
) -> AppResult<()> {
    record_download_submission_tx(tx, &commit.download_submission).await?;
    let mut wanted_item_ids = commit.covered_wanted_item_ids.clone();
    if !wanted_item_ids
        .iter()
        .any(|id| id == &commit.wanted_item_id)
    {
        wanted_item_ids.push(commit.wanted_item_id.clone());
    }
    wanted_item_ids.sort();
    wanted_item_ids.dedup();

    for wanted_item_id in &wanted_item_ids {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE wanted_items
             SET status = {}, next_search_at = {}, last_search_at = {},
                 search_count = {}, current_score = {}, grabbed_release = {}, updated_at = {}
             WHERE id = {}",
            &[
                SqlArg::Text(WantedStatus::Grabbed.as_str().to_string()),
                SqlArg::OptTimestamp(None),
                opt_timestamp_arg(commit.last_search_at.as_deref()),
                SqlArg::I64(commit.search_count),
                SqlArg::OptI32(commit.current_score),
                SqlArg::Text(commit.grabbed_release.clone()),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(wanted_item_id.clone()),
            ],
        )
        .await?;
    }

    if let Some(pending_release_id) = commit.grabbed_pending_release_id.as_deref() {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE pending_releases SET status = {}, grabbed_at = {} WHERE id = {}",
            &[
                SqlArg::Text(PendingReleaseStatus::Grabbed.as_str().to_string()),
                opt_timestamp_arg(commit.grabbed_at.as_deref()),
                SqlArg::Text(pending_release_id.to_string()),
            ],
        )
        .await?;
    }

    for wanted_item_id in &wanted_item_ids {
        match commit.grabbed_pending_release_id.as_deref() {
            Some(except_id) => {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE pending_releases
                     SET status = 'superseded'
                     WHERE wanted_item_id = {}
                       AND id != {}
                       AND status IN ('waiting', 'standby')",
                    &[
                        SqlArg::Text(wanted_item_id.clone()),
                        SqlArg::Text(except_id.to_string()),
                    ],
                )
                .await?;
            }
            None => {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE pending_releases
                     SET status = 'superseded'
                     WHERE wanted_item_id = {}
                       AND status IN ('waiting', 'standby')",
                    &[SqlArg::Text(wanted_item_id.clone())],
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn record_download_submission_tx(
    tx: &mut SqlTx<'_>,
    submission: &DownloadSubmission,
) -> AppResult<()> {
    let (episode_id, collection_id) = persisted_submission_scope(&submission.scope);
    let download_client_id = normalize_download_client_id(submission.download_client_id.as_deref());
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(download_client_id, download_client_type, download_client_item_id) DO UPDATE
         SET title_id = excluded.title_id,
             facet = excluded.facet,
             source_hint = excluded.source_hint,
             source_kind = excluded.source_kind,
             source_title = excluded.source_title,
             request_signature = excluded.request_signature,
             episode_id = excluded.episode_id,
             collection_id = excluded.collection_id",
        &[
            SqlArg::Text(Id::new().0),
            SqlArg::Text(submission.title_id.clone()),
            SqlArg::Text(submission.facet.clone()),
            SqlArg::Text(download_client_id.clone()),
            SqlArg::Text(submission.download_client_type.clone()),
            SqlArg::Text(submission.download_client_item_id.clone()),
            SqlArg::OptText(submission.source_hint.clone()),
            SqlArg::OptText(submission.source_kind.map(|value| value.as_str().to_string())),
            SqlArg::OptText(submission.source_title.clone()),
            SqlArg::OptText(submission.request_signature.clone()),
            SqlArg::OptText(episode_id.map(str::to_string)),
            SqlArg::OptText(collection_id.map(str::to_string)),
        ],
    )
    .await?;
    replace_download_submission_episode_links_tx(
        tx,
        &download_client_id,
        &submission.download_client_type,
        &submission.download_client_item_id,
        persisted_episode_set_ids(&submission.scope),
    )
    .await
}

async fn replace_download_submission_episode_links_tx(
    tx: &mut SqlTx<'_>,
    download_client_id: &str,
    download_client_type: &str,
    download_client_item_id: &str,
    episode_ids: &[String],
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM download_submission_episode_links
         WHERE download_client_id = {}
           AND download_client_type = {}
           AND download_client_item_id = {}",
        &[
            SqlArg::Text(download_client_id.to_string()),
            SqlArg::Text(download_client_type.to_string()),
            SqlArg::Text(download_client_item_id.to_string()),
        ],
    )
    .await?;
    for episode_id in episode_ids {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO download_submission_episode_links
             (download_client_id, download_client_type, download_client_item_id, episode_id)
             VALUES ({}, {}, {}, {})
             ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(download_client_id.to_string()),
                SqlArg::Text(download_client_type.to_string()),
                SqlArg::Text(download_client_item_id.to_string()),
                SqlArg::Text(episode_id.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn queue_import_request(
    datastore: &StoreDatastore,
    source_identity: DownloadSourceIdentity,
    import_type: String,
    payload_json: String,
) -> AppResult<String> {
    let normalized_client_id = source_identity
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let lookup_client_id = normalized_client_id.as_deref().unwrap_or("").to_string();
    let is_rename = ImportType::parse(&import_type).is_some_and(|kind| kind.is_rename());
    let rename_plan_json = is_rename.then_some(payload_json.clone());
    let payload_arg = json_arg_for_datastore(datastore, Some(&payload_json))?;
    let rename_arg = json_arg_for_datastore(datastore, rename_plan_json.as_deref())?;
    let result_arg = json_arg_for_datastore(datastore, None::<&str>)?;
    let id = Id::new().0;
    let now = Utc::now();

    SqlRuntime::run_in_transaction(datastore, "create_import_request", move |tx| {
        let source_identity = source_identity.clone();
        let import_type = import_type.clone();
        let normalized_client_id = normalized_client_id.clone();
        let lookup_client_id = lookup_client_id.clone();
        let payload_arg = payload_arg.clone();
        let rename_arg = rename_arg.clone();
        let result_arg = result_arg.clone();
        let id = id.clone();
        Box::pin(async move {
            let source_system = source_identity.client_type.clone();
            let source_ref = source_identity.item_id.clone();
            let import_type_key = import_type.clone();
            let upsert_sql = import_request_upsert_sql(tx);
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                &upsert_sql,
                &[
                    SqlArg::Text(id.clone()),
                    SqlArg::OptText(normalized_client_id),
                    SqlArg::Text(source_system.clone()),
                    SqlArg::Text(source_ref.clone()),
                    SqlArg::Text(import_type),
                    SqlArg::Text(ImportStatus::Pending.as_str().to_string()),
                    payload_arg,
                    rename_arg,
                    result_arg,
                    SqlArg::OptTimestamp(None),
                    SqlArg::OptTimestamp(None),
                    SqlArg::Timestamp(now),
                    SqlArg::Timestamp(now),
                ],
            )
            .await?;

            let row = SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT id FROM imports
                 WHERE COALESCE(source_client_id, '') = {}
                   AND source_system = {}
                   AND source_ref = {}
                   AND import_type = {}
                 LIMIT 1",
                &[
                    SqlArg::Text(lookup_client_id),
                    SqlArg::Text(source_system),
                    SqlArg::Text(source_ref),
                    SqlArg::Text(import_type_key),
                ],
            )
            .await?
            .ok_or_else(|| AppError::Repository("failed to reload persisted import".into()))?;
            row.text("id")
        })
    })
    .await
}

fn import_request_upsert_sql(tx: &SqlTx<'_>) -> String {
    let conflict_clause = match tx {
        SqlTx::Sqlite(_) => "ON CONFLICT DO UPDATE",
        SqlTx::Postgres(_) => {
            "ON CONFLICT ((COALESCE(source_client_id, '')), source_system, source_ref, import_type) DO UPDATE"
        }
    };

    format!(
        "INSERT INTO imports
         (id, source_client_id, source_system, source_ref, import_type, status, payload_json, rename_plan_json, result_json, started_at, finished_at, created_at, updated_at)
         VALUES ({{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}})
         {conflict_clause} SET
            source_client_id = excluded.source_client_id,
            status = excluded.status,
            payload_json = excluded.payload_json,
            rename_plan_json = excluded.rename_plan_json,
            result_json = NULL,
            started_at = NULL,
            finished_at = NULL,
            updated_at = excluded.updated_at"
    )
}

async fn recover_stale_processing_imports(
    datastore: &StoreDatastore,
    import_type: Option<ImportType>,
    stale_seconds: i64,
) -> AppResult<u64> {
    let now = Utc::now();
    let cutoff = now - chrono::Duration::seconds(stale_seconds);
    let mut args = vec![
        json_arg_for_datastore(datastore, Some("{\"error\":\"stale processing recovery\"}"))?,
        SqlArg::Timestamp(now),
        SqlArg::Timestamp(now),
    ];
    let type_filter = if let Some(import_type) = import_type {
        args.push(SqlArg::Text(import_type.as_str().to_string()));
        "AND import_type = {}"
    } else {
        "AND import_type != 'manual_import'"
    };
    args.push(SqlArg::Timestamp(cutoff));
    let rows = execute_write(
        datastore,
        "recover_stale_processing_imports",
        format!(
            "UPDATE imports
             SET status = 'failed',
                 result_json = {{}},
                 finished_at = {{}},
                 updated_at = {{}}
             WHERE status = 'processing'
               {type_filter}
               AND updated_at < {{}}"
        ),
        args,
    )
    .await?;
    Ok(rows)
}

async fn create_workflow_operation(
    datastore: &StoreDatastore,
    operation: NewWorkflowOperation,
) -> AppResult<WorkflowOperationRecord> {
    let progress_arg = json_arg_for_datastore(datastore, operation.progress_json.as_deref())?;
    let summary_arg = json_arg_for_datastore(datastore, operation.summary_json.as_deref())?;
    let id = Id::new().0;
    let now = Utc::now();
    SqlRuntime::run_in_transaction(datastore, "create_workflow_operation", move |tx| {
        let id = id.clone();
        let operation = operation.clone();
        let progress_arg = progress_arg.clone();
        let summary_arg = summary_arg.clone();
        Box::pin(async move {
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "INSERT INTO workflow_operations
                 (id, operation_type, status, job_key, trigger_source, actor_user_id, progress_json, summary_json, summary_text, error_text, started_at, completed_at, created_at, updated_at)
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                &[
                    SqlArg::Text(id.clone()),
                    SqlArg::Text(operation.operation_type.clone()),
                    SqlArg::Text(operation.status.clone()),
                    SqlArg::OptText(operation.job_key.clone()),
                    SqlArg::OptText(operation.trigger_source.clone()),
                    SqlArg::OptText(operation.actor_user_id.clone()),
                    progress_arg,
                    summary_arg,
                    SqlArg::OptText(operation.summary_text.clone()),
                    SqlArg::OptText(operation.error_text.clone()),
                    opt_timestamp_arg(operation.started_at.as_deref()).or_timestamp(now),
                    opt_timestamp_arg(operation.completed_at.as_deref()),
                    SqlArg::Timestamp(now),
                    SqlArg::Timestamp(now),
                ],
            )
            .await?;
            fetch_optional_workflow_operation(SqlExec::Tx(tx), &id)
                .await?
                .ok_or_else(|| AppError::Repository("failed to reload workflow operation".into()))
        })
    })
    .await
}

trait SqlArgExt {
    fn or_timestamp(self, fallback: DateTime<Utc>) -> SqlArg;
}

impl SqlArgExt for SqlArg {
    fn or_timestamp(self, fallback: DateTime<Utc>) -> SqlArg {
        match self {
            SqlArg::OptTimestamp(None) => SqlArg::Timestamp(fallback),
            other => other,
        }
    }
}

fn build_domain_event_list_sql(filter: &DomainEventFilter) -> (String, Vec<SqlArg>) {
    let limit = if filter.limit == 0 {
        100
    } else {
        filter.limit.min(500)
    };
    let mut where_clauses = Vec::new();
    let mut args = Vec::new();
    if let Some(event_types) = filter.event_types.as_ref()
        && !event_types.is_empty()
    {
        where_clauses.push(format!(
            "event_type IN ({})",
            placeholders(event_types.len())
        ));
        args.extend(
            event_types
                .iter()
                .map(|event_type| SqlArg::Text(event_type.as_str().to_string())),
        );
    }
    if let Some(title_id) = filter.title_id.as_ref() {
        where_clauses.push("title_id = {}".to_string());
        args.push(SqlArg::Text(title_id.clone()));
    }
    if let Some(facet) = filter.facet.as_ref() {
        where_clauses.push("facet = {}".to_string());
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }
    if let Some(after_sequence) = filter.after_sequence {
        where_clauses.push("sequence > {}".to_string());
        args.push(SqlArg::I64(after_sequence));
    }
    if let Some(before_sequence) = filter.before_sequence {
        where_clauses.push("sequence < {}".to_string());
        args.push(SqlArg::I64(before_sequence));
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clauses.join(" AND "))
    };
    let order = if filter.after_sequence.is_some() && filter.before_sequence.is_none() {
        "ASC"
    } else {
        "DESC"
    };
    args.push(SqlArg::I64(limit as i64));
    (
        format!(
            "SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events{where_sql} ORDER BY sequence {order} LIMIT {{}}"
        ),
        args,
    )
}

fn build_title_history_filter_sql(
    datastore: &StoreDatastore,
    event_types: Option<&[TitleHistoryEventType]>,
    title_ids: Option<&[String]>,
    download_id: Option<&str>,
) -> (String, Vec<SqlArg>) {
    let mut clauses = vec!["title_id IS NOT NULL".to_string()];
    let mut args = Vec::new();
    match event_types {
        None => {
            clauses.push(format!(
                "event_type IN ({})",
                placeholders(TITLE_HISTORY_PAGE_DOMAIN_EVENT_TYPES.len())
            ));
            args.extend(
                TITLE_HISTORY_PAGE_DOMAIN_EVENT_TYPES
                    .iter()
                    .map(|event_type| SqlArg::Text(event_type.as_str().to_string())),
            );
        }
        Some([]) => clauses.push("0".to_string()),
        Some(event_types) => {
            let mut parts = Vec::new();
            for event_type in event_types {
                match event_type {
                    TitleHistoryEventType::Grabbed => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::ReleaseGrabbed.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::DownloadFailed => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::DownloadFailed.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::Blocklisted => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::ReleaseBlocklisted.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::Imported => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::ImportCompleted.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::ImportFailed => {
                        parts.push(format!(
                            "(event_type = {{}} AND {} = {{}})",
                            json_extract(datastore, "payload_json", "data", "status")
                        ));
                        args.push(SqlArg::Text(
                            DomainEventType::ImportRejected.as_str().into(),
                        ));
                        args.push(SqlArg::Text(ImportStatus::Failed.as_str().into()));
                    }
                    TitleHistoryEventType::ImportSkipped => {
                        parts.push(format!(
                            "(event_type = {{}} AND {} = {{}})",
                            json_extract(datastore, "payload_json", "data", "status")
                        ));
                        args.push(SqlArg::Text(
                            DomainEventType::ImportRejected.as_str().into(),
                        ));
                        args.push(SqlArg::Text(ImportStatus::Skipped.as_str().into()));
                    }
                    TitleHistoryEventType::FileDeleted => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::MediaFileDeleted.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::FileRenamed => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::MediaFileRenamed.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::Rematched => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::TitleRematched.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::DownloadCompleted
                    | TitleHistoryEventType::DownloadIgnored => parts.push("0".to_string()),
                }
            }
            clauses.push(format!("({})", parts.join(" OR ")));
        }
    }
    if let Some(title_ids) = title_ids {
        if title_ids.is_empty() {
            clauses.push("0".to_string());
        } else if title_ids.len() == 1 {
            clauses.push("title_id = {}".to_string());
            args.push(SqlArg::Text(title_ids[0].clone()));
        } else {
            clauses.push(format!("title_id IN ({})", placeholders(title_ids.len())));
            args.extend(title_ids.iter().cloned().map(SqlArg::Text));
        }
    }
    if let Some(download_id) = download_id {
        clauses.push(format!(
            "{} = {{}}",
            json_extract(datastore, "payload_json", "data", "download_id")
        ));
        args.push(SqlArg::Text(download_id.to_string()));
    }
    (format!(" WHERE {}", clauses.join(" AND ")), args)
}

const TITLE_HISTORY_PAGE_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::TitleRematched,
    DomainEventType::ReleaseGrabbed,
    DomainEventType::ImportCompleted,
    DomainEventType::ImportRejected,
    DomainEventType::DownloadFailed,
    DomainEventType::ReleaseBlocklisted,
    DomainEventType::MediaFileDeleted,
    DomainEventType::MediaFileRenamed,
];

fn json_extract(datastore: &StoreDatastore, column: &str, first: &str, second: &str) -> String {
    match datastore {
        StoreDatastore::Sqlite { .. } => format!("json_extract({column}, '$.{first}.{second}')"),
        StoreDatastore::Postgres { .. } => format!("{column} #>> '{{{first},{second}}}'"),
    }
}

fn download_submission_select_sql(datastore: &StoreDatastore, suffix: &str) -> String {
    let episode_set = match datastore {
        StoreDatastore::Sqlite { .. } => {
            "(SELECT group_concat(link.episode_id, char(31))
                FROM download_submission_episode_links link
               WHERE link.download_client_id = download_submissions.download_client_id
                 AND link.download_client_type = download_submissions.download_client_type
                 AND link.download_client_item_id = download_submissions.download_client_item_id)"
        }
        StoreDatastore::Postgres { .. } => {
            "(SELECT string_agg(link.episode_id, chr(31))
                FROM download_submission_episode_links link
               WHERE link.download_client_id = download_submissions.download_client_id
                 AND link.download_client_type = download_submissions.download_client_type
                 AND link.download_client_item_id = download_submissions.download_client_item_id)"
        }
    };
    format!(
        "SELECT {DOWNLOAD_SUBMISSION_COLUMNS}, {episode_set} AS episode_set_ids FROM download_submissions {suffix}"
    )
}

async fn fetch_domain_events(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DomainEvent>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| domain_event_from_row(&row))
        .collect()
}

async fn fetch_domain_event_by_event_id(
    exec: SqlExec<'_, '_>,
    event_id: &str,
) -> AppResult<Option<DomainEvent>> {
    SqlRuntime::fetch_optional(
        exec,
        &format!("SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events WHERE event_id = {{}}"),
        &[SqlArg::Text(event_id.to_string())],
    )
    .await?
    .map(|row| domain_event_from_row(&row))
    .transpose()
}

async fn fetch_download_submissions(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DownloadSubmission>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| download_submission_from_row(&row))
        .collect()
}

async fn fetch_imports(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<ImportRecord>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| import_record_from_row(&row))
        .collect()
}

async fn fetch_snapshot_chunks(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await
        .map_err(map_snapshot_chunk_error)?
        .into_iter()
        .map(|row| snapshot_chunk_from_row(&row))
        .collect()
}

async fn fetch_delete_commands(
    exec: SqlExec<'_, '_>,
    suffix: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DownloadQueueCommandRecord>> {
    SqlRuntime::fetch_all(
        exec,
        &format!("SELECT {DOWNLOAD_QUEUE_COMMAND_COLUMNS} FROM download_queue_commands {suffix}"),
        args,
    )
    .await?
    .into_iter()
    .map(|row| download_queue_command_from_row(&row))
    .collect()
}

async fn fetch_optional_delete_command(
    exec: SqlExec<'_, '_>,
    suffix: &str,
    args: &[SqlArg],
) -> AppResult<Option<DownloadQueueCommandRecord>> {
    SqlRuntime::fetch_optional(
        exec,
        &format!("SELECT {DOWNLOAD_QUEUE_COMMAND_COLUMNS} FROM download_queue_commands {suffix}"),
        args,
    )
    .await?
    .map(|row| download_queue_command_from_row(&row))
    .transpose()
}

async fn fetch_optional_workflow_operation(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<WorkflowOperationRecord>> {
    SqlRuntime::fetch_optional(
        exec,
        "SELECT * FROM workflow_operations WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?
    .map(|row| workflow_operation_from_row(&row))
    .transpose()
}

fn domain_event_from_row(row: &SqlRow) -> AppResult<DomainEvent> {
    let stream_kind = row.text("stream_kind")?;
    let payload = serde_json::from_value(json_from_row(row, "payload_json")?).map_err(repo_err)?;
    Ok(DomainEvent {
        sequence: row.i64("sequence")?,
        event_id: row.text("event_id")?,
        occurred_at: row.timestamp("occurred_at")?,
        actor_user_id: row.opt_text("actor_user_id")?,
        title_id: row.opt_text("title_id")?,
        facet: row
            .opt_text("facet")?
            .as_deref()
            .and_then(MediaFacet::parse),
        correlation_id: row.opt_text("correlation_id")?,
        causation_id: row.opt_text("causation_id")?,
        schema_version: row.i32("schema_version")?,
        stream: stream_from_parts(&stream_kind, row.opt_text("stream_id")?)?,
        payload,
    })
}

fn stream_from_parts(kind: &str, identifier: Option<String>) -> AppResult<DomainEventStream> {
    match kind {
        "global" => Ok(DomainEventStream::Global),
        "title" => identifier
            .map(|title_id| DomainEventStream::Title { title_id })
            .ok_or_else(|| AppError::Repository("domain event missing title stream id".into())),
        "library_scan" => identifier
            .map(|session_id| DomainEventStream::LibraryScan { session_id })
            .ok_or_else(|| {
                AppError::Repository("domain event missing library scan stream id".into())
            }),
        "job_run" => identifier
            .map(|run_id| DomainEventStream::JobRun { run_id })
            .ok_or_else(|| AppError::Repository("domain event missing job run stream id".into())),
        "download_queue_item" => identifier
            .map(|item_id| DomainEventStream::DownloadQueueItem { item_id })
            .ok_or_else(|| {
                AppError::Repository("domain event missing download queue item stream id".into())
            }),
        other => Err(AppError::Repository(format!(
            "unknown domain event stream kind: {other}"
        ))),
    }
}

fn download_submission_from_row(row: &SqlRow) -> AppResult<DownloadSubmission> {
    let title_id = row.text("title_id")?;
    let episode_id = opt_text_lenient(row, "episode_id")?;
    let collection_id = opt_text_lenient(row, "collection_id")?;
    let episode_set_ids = opt_text_lenient(row, "episode_set_ids")?.map(|raw| {
        raw.split('\u{1f}')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let source_kind = row
        .opt_text("source_kind")?
        .as_deref()
        .and_then(scryer_application::DownloadSourceKind::parse);
    Ok(DownloadSubmission {
        scope: SubmissionScope::from_persisted(
            &title_id,
            episode_id,
            collection_id,
            episode_set_ids,
        ),
        title_id,
        facet: row.text("facet")?,
        download_client_id: row
            .opt_text("download_client_id")?
            .filter(|value| !value.trim().is_empty()),
        download_client_type: row.text("download_client_type")?,
        download_client_item_id: row.text("download_client_item_id")?,
        source_hint: row.opt_text("source_hint")?,
        source_kind,
        source_title: row.opt_text("source_title")?,
        request_signature: row.opt_text("request_signature")?,
    })
}

fn import_record_from_row(row: &SqlRow) -> AppResult<ImportRecord> {
    let import_type_raw = row.text("import_type")?;
    let status_raw = row.text("status")?;
    Ok(ImportRecord {
        id: row.text("id")?,
        source_client_id: row
            .opt_text("source_client_id")?
            .filter(|value| !value.trim().is_empty()),
        source_system: row.text("source_system")?,
        source_ref: row.text("source_ref")?,
        import_type: ImportType::parse(&import_type_raw).ok_or_else(|| {
            AppError::Repository(format!("unknown import_type: {import_type_raw}"))
        })?,
        status: ImportStatus::parse(&status_raw).unwrap_or_default(),
        payload_json: json_text_from_row(row, "payload_json")?.unwrap_or_default(),
        result_json: json_text_from_row(row, "result_json")?,
        started_at: opt_timestamp_string(row, "started_at")?,
        finished_at: opt_timestamp_string(row, "finished_at")?,
        created_at: timestamp_string(row, "created_at")?,
        updated_at: timestamp_string(row, "updated_at")?,
    })
}

fn import_artifact_from_row(row: &SqlRow) -> AppResult<ImportArtifact> {
    Ok(ImportArtifact {
        id: row.text("id")?,
        source_client_id: row
            .opt_text("source_client_id")?
            .filter(|value| !value.trim().is_empty()),
        source_system: row.text("source_system")?,
        source_ref: row.text("source_ref")?,
        import_id: row.opt_text("import_id")?,
        relative_path: row.opt_text("relative_path")?,
        normalized_file_name: row.text("normalized_file_name")?,
        media_kind: row.text("media_kind")?,
        title_id: row.opt_text("title_id")?,
        episode_id: row.opt_text("episode_id")?,
        season_number: row.opt_i32("season_number")?,
        episode_number: row.opt_i32("episode_number")?,
        result: row.text("result")?,
        reason_code: row.opt_text("reason_code")?,
        imported_media_file_id: row.opt_text("imported_media_file_id")?,
        created_at: row.timestamp("created_at")?,
    })
}

fn snapshot_chunk_from_row(row: &SqlRow) -> AppResult<ExternalImportMonitorSnapshotChunk> {
    let facet_raw = row.text("facet")?;
    let entry_kind_raw = row.text("entry_kind")?;
    Ok(ExternalImportMonitorSnapshotChunk {
        facet: MediaFacet::parse(&facet_raw).ok_or_else(|| {
            AppError::Repository(format!("invalid monitor snapshot chunk facet: {facet_raw}"))
        })?,
        entry_kind: ExternalImportMonitorSnapshotEntryKind::parse(&entry_kind_raw).ok_or_else(
            || {
                AppError::Repository(format!(
                    "invalid monitor snapshot chunk entry kind: {entry_kind_raw}"
                ))
            },
        )?,
        chunk_index: row.i32("chunk_index")?,
        payload_ndjson: row.text("payload_ndjson")?,
        created_at: timestamp_string(row, "created_at")?,
    })
}

fn download_queue_command_from_row(row: &SqlRow) -> AppResult<DownloadQueueCommandRecord> {
    let action = row.text("action")?;
    let status = row.text("status")?;
    Ok(DownloadQueueCommandRecord {
        id: row.text("id")?,
        action: DownloadQueueCommandAction::parse(&action).ok_or_else(|| {
            AppError::Repository(format!("unknown download queue action: {action}"))
        })?,
        client_id: row
            .opt_text("client_id")?
            .filter(|value| !value.trim().is_empty()),
        client_type: row.text("client_type")?,
        download_client_item_id: row.text("download_client_item_id")?,
        is_history: row.bool("is_history")?,
        status: DownloadQueueDeleteStatus::parse(&status).ok_or_else(|| {
            AppError::Repository(format!("unknown download queue command status: {status}"))
        })?,
        error_text: row.opt_text("error_text")?,
        requested_by_user_id: row.opt_text("requested_by_user_id")?,
        started_at: opt_timestamp_string(row, "started_at")?,
        finished_at: opt_timestamp_string(row, "finished_at")?,
        created_at: timestamp_string(row, "created_at")?,
        updated_at: timestamp_string(row, "updated_at")?,
    })
}

fn workflow_operation_from_row(row: &SqlRow) -> AppResult<WorkflowOperationRecord> {
    Ok(WorkflowOperationRecord {
        id: row.text("id")?,
        operation_type: row.text("operation_type")?,
        status: row.text("status")?,
        job_key: row.opt_text("job_key")?,
        trigger_source: row.opt_text("trigger_source")?,
        actor_user_id: row.opt_text("actor_user_id")?,
        title_id: row.opt_text("title_id")?,
        collection_id: row.opt_text("collection_id")?,
        episode_id: row.opt_text("episode_id")?,
        release_id: row.opt_text("release_id")?,
        media_file_id: row.opt_text("media_file_id")?,
        external_reference: row.opt_text("external_reference")?,
        progress_json: json_text_from_row(row, "progress_json")?,
        summary_json: json_text_from_row(row, "summary_json")?,
        summary_text: row.opt_text("summary_text")?,
        error_text: row.opt_text("error_text")?,
        started_at: opt_timestamp_string(row, "started_at")?,
        completed_at: opt_timestamp_string(row, "completed_at")?,
        created_at: timestamp_string(row, "created_at")?,
        updated_at: timestamp_string(row, "updated_at")?,
    })
}

fn job_run_record_from_workflow(record: WorkflowOperationRecord) -> AppResult<JobRunRecord> {
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
        started_at: parse_datetime_or_now(record.started_at.as_deref()),
        completed_at: record
            .completed_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc)),
        created_at: parse_datetime_or_now(Some(&record.created_at)),
        updated_at: parse_datetime_or_now(Some(&record.updated_at)),
    })
}

fn workflow_operation_info(record: WorkflowOperationRecord) -> WorkflowOperationInfo {
    WorkflowOperationInfo {
        id: record.id,
        operation_type: record.operation_type,
        status: record.status,
        actor_user_id: record.actor_user_id,
        progress_json: record.progress_json,
        started_at: record.started_at,
        completed_at: record.completed_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: String,
    args: Vec<SqlArg>,
) -> AppResult<u64> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let sql = sql.clone();
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
    })
    .await
}

async fn update_delete_command_status(
    datastore: &StoreDatastore,
    id: &str,
    status: DownloadQueueDeleteStatus,
    error_text: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now();
    let started_at = match status {
        DownloadQueueDeleteStatus::Running => Some(now),
        _ => None,
    };
    let finished_at = match status {
        DownloadQueueDeleteStatus::Completed | DownloadQueueDeleteStatus::Failed => Some(now),
        _ => None,
    };
    execute_write(
        datastore,
        "update_delete_download_command_status",
        "UPDATE download_queue_commands
         SET status = {},
             error_text = {},
             started_at = COALESCE({}, started_at),
             finished_at = {},
             updated_at = {}
         WHERE id = {}"
            .to_string(),
        vec![
            SqlArg::Text(status.as_str().to_string()),
            SqlArg::OptText(error_text.map(str::to_string)),
            SqlArg::OptTimestamp(started_at),
            SqlArg::OptTimestamp(finished_at),
            SqlArg::Timestamp(now),
            SqlArg::Text(id.to_string()),
        ],
    )
    .await?;
    Ok(())
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

const DOWNLOAD_SUBMISSION_BATCH_LOOKUP_CHUNK_SIZE: usize = 400;

pub(crate) fn chunk_download_submission_client_items(
    client_items: &[DownloadSourceIdentity],
) -> Vec<Vec<DownloadSourceIdentity>> {
    let deduped = dedupe_identities(client_items);
    deduped
        .chunks(DOWNLOAD_SUBMISSION_BATCH_LOOKUP_CHUNK_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

fn dedupe_identities(identities: &[DownloadSourceIdentity]) -> Vec<DownloadSourceIdentity> {
    let mut seen = HashSet::with_capacity(identities.len());
    let mut deduped = Vec::with_capacity(identities.len());
    for identity in identities {
        if seen.insert((
            normalize_download_client_id(identity.client_id.as_deref()),
            identity.client_type.clone(),
            identity.item_id.clone(),
        )) {
            deduped.push(identity.clone());
        }
    }
    deduped
}

fn normalize_download_client_id(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn json_arg_for_datastore(datastore: &StoreDatastore, value: Option<&str>) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::OptText(value.map(str::to_string))),
        StoreDatastore::Postgres { .. } => value
            .map(postgres_json_value)
            .transpose()
            .map(SqlArg::OptJson),
    }
}

fn json_arg_for_tx(tx: &SqlTx<'_>, value: Option<&str>) -> AppResult<SqlArg> {
    match tx {
        SqlTx::Sqlite(_) => Ok(SqlArg::OptText(value.map(str::to_string))),
        SqlTx::Postgres(_) => value
            .map(postgres_json_value)
            .transpose()
            .map(SqlArg::OptJson),
    }
}

fn postgres_json_value(value: &str) -> AppResult<JsonValue> {
    Ok(serde_json::from_str(value).unwrap_or_else(|_| JsonValue::String(value.to_string())))
}

fn json_from_row(row: &SqlRow, column: &str) -> AppResult<JsonValue> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: String = row.try_get(column).map_err(repo_err)?;
            serde_json::from_str(&raw).map_err(repo_err)
        }
        SqlRow::Postgres(row) => {
            let raw: Json<JsonValue> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.0)
        }
    }
}

fn json_text_from_row(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
        SqlRow::Postgres(row) => {
            let raw: Option<Json<JsonValue>> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.map(|value| json_value_as_string(value.0)))
        }
    }
}

fn json_value_as_string(value: JsonValue) -> String {
    match value {
        JsonValue::String(value) => value,
        value => value.to_string(),
    }
}

fn opt_text_lenient(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(row) => Ok(row.try_get::<Option<String>, _>(column).ok().flatten()),
        SqlRow::Postgres(row) => Ok(row.try_get::<Option<String>, _>(column).ok().flatten()),
    }
}

fn timestamp_string(row: &SqlRow, column: &str) -> AppResult<String> {
    match row {
        SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
        SqlRow::Postgres(row) => {
            let value: DateTime<Utc> = row.try_get(column).map_err(repo_err)?;
            Ok(value.to_rfc3339())
        }
    }
}

fn opt_timestamp_string(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
        SqlRow::Postgres(row) => {
            let value: Option<DateTime<Utc>> = row.try_get(column).map_err(repo_err)?;
            Ok(value.map(|value| value.to_rfc3339()))
        }
    }
}

fn opt_timestamp_arg(value: Option<&str>) -> SqlArg {
    SqlArg::OptTimestamp(
        value
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc)),
    )
}

fn parse_datetime_or_now(value: Option<&str>) -> DateTime<Utc> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

fn map_snapshot_chunk_error(error: AppError) -> AppError {
    let message = error.to_string();
    if message.contains("no such table: external_import_monitor_snapshot_chunks") {
        return AppError::Repository(
            "database is missing external_import_monitor_snapshot_chunks; restart with a build that includes migration 0117_external_import_monitor_snapshot_chunks and let migrations complete".into(),
        );
    }
    error
}
