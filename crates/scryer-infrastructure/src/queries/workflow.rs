use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadQueueCommandRecord as AppDownloadQueueCommandRecord,
    DownloadSourceIdentity, DownloadSubmission, ExternalImportMonitorSnapshot,
    PendingReleaseStatus, ReleaseDownloadAttemptOutcome, SubmissionScope, SuccessfulGrabCommit,
    WantedStatus,
};
use scryer_domain::{
    DownloadQueueCommandAction, DownloadQueueDeleteStatus, Id, ImportRecord, ImportStatus,
    ImportType, MediaFacet,
};
use sqlx::Row;
use sqlx::{QueryBuilder, Sqlite, SqlitePool, Transaction};
use std::collections::HashSet;

use crate::types::{
    LibraryProbeSignatureRecord, ReleaseDownloadFailureSignatureRecord,
    TitleReleaseBlocklistRecord, WorkflowOperationRecord,
};

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

fn normalize_download_client_id(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn chunk_download_submission_client_items(
    client_items: &[DownloadSourceIdentity],
) -> Vec<Vec<DownloadSourceIdentity>> {
    let mut seen = HashSet::with_capacity(client_items.len());
    let mut deduped = Vec::with_capacity(client_items.len());
    for identity in client_items {
        let key = (
            normalize_download_client_id(identity.client_id.as_deref()),
            identity.client_type.clone(),
            identity.item_id.clone(),
        );
        if seen.insert(key) {
            deduped.push(identity.clone());
        }
    }

    deduped
        .chunks(DOWNLOAD_SUBMISSION_BATCH_LOOKUP_CHUNK_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

fn download_submission_from_row(row: &sqlx::sqlite::SqliteRow) -> AppResult<DownloadSubmission> {
    let title_id: String = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let facet: String = row
        .try_get("facet")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let download_client_type: String = row
        .try_get("download_client_type")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let download_client_id = row
        .try_get::<Option<String>, _>("download_client_id")
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty());
    let download_client_item_id: String = row
        .try_get("download_client_item_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let source_hint = row
        .try_get("source_hint")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let source_kind = row
        .try_get::<Option<String>, _>("source_kind")
        .map_err(|err| AppError::Repository(err.to_string()))?
        .as_deref()
        .and_then(scryer_application::DownloadSourceKind::parse);
    let source_title = row
        .try_get("source_title")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let request_signature = row
        .try_get("request_signature")
        .map_err(|err| AppError::Repository(err.to_string()))?;
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
        facet,
        download_client_id,
        download_client_type,
        download_client_item_id,
        source_hint,
        source_kind,
        source_title,
        request_signature,
    })
}

fn workflow_operation_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> AppResult<WorkflowOperationRecord> {
    Ok(WorkflowOperationRecord {
        id: row
            .try_get("id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        operation_type: row
            .try_get("operation_type")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        status: row
            .try_get("status")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        job_key: row
            .try_get("job_key")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        trigger_source: row
            .try_get("trigger_source")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        actor_user_id: row
            .try_get("actor_user_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        title_id: row
            .try_get("title_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        collection_id: row
            .try_get("collection_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        episode_id: row
            .try_get("episode_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        release_id: row
            .try_get("release_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        media_file_id: row
            .try_get("media_file_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        external_reference: row
            .try_get("external_reference")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        progress_json: row
            .try_get("progress_json")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        summary_json: row
            .try_get("summary_json")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        summary_text: row
            .try_get("summary_text")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        error_text: row
            .try_get("error_text")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        started_at: row
            .try_get("started_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        completed_at: row
            .try_get("completed_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
    })
}

fn import_record_from_row(row: &sqlx::sqlite::SqliteRow) -> AppResult<ImportRecord> {
    Ok(ImportRecord {
        id: row
            .try_get("id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        source_system: row
            .try_get("source_system")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        source_ref: row
            .try_get("source_ref")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        import_type: {
            let s: String = row
                .try_get("import_type")
                .map_err(|e| AppError::Repository(e.to_string()))?;
            ImportType::parse(&s)
                .ok_or_else(|| AppError::Repository(format!("unknown import_type: {s}")))?
        },
        status: {
            let s: String = row
                .try_get("status")
                .map_err(|e| AppError::Repository(e.to_string()))?;
            ImportStatus::parse(&s).unwrap_or_default()
        },
        payload_json: row
            .try_get("payload_json")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        result_json: row
            .try_get("result_json")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        started_at: row
            .try_get("started_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        finished_at: row
            .try_get("finished_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    })
}

fn download_queue_command_record_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> AppResult<AppDownloadQueueCommandRecord> {
    let action: String = row
        .try_get("action")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let status: String = row
        .try_get("status")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(AppDownloadQueueCommandRecord {
        id: row
            .try_get("id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        action: DownloadQueueCommandAction::parse(&action).ok_or_else(|| {
            AppError::Repository(format!("unknown download queue action: {action}"))
        })?,
        client_id: row
            .try_get::<Option<String>, _>("client_id")
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty()),
        client_type: row
            .try_get("client_type")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        download_client_item_id: row
            .try_get("download_client_item_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        is_history: row
            .try_get::<i64, _>("is_history")
            .map_err(|err| AppError::Repository(err.to_string()))?
            != 0,
        status: DownloadQueueDeleteStatus::parse(&status).ok_or_else(|| {
            AppError::Repository(format!("unknown download queue command status: {status}"))
        })?,
        error_text: row
            .try_get("error_text")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        requested_by_user_id: row
            .try_get("requested_by_user_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        started_at: row
            .try_get("started_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        finished_at: row
            .try_get("finished_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
    })
}

pub(crate) async fn upsert_external_import_monitor_snapshot_query(
    pool: &SqlitePool,
    snapshot: &ExternalImportMonitorSnapshot,
) -> AppResult<()> {
    let payload_json = serde_json::to_string(&snapshot.payload)
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query(
        "INSERT INTO external_import_monitor_snapshots (facet, payload_json, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT(facet) DO UPDATE SET
             payload_json = excluded.payload_json,
             created_at = excluded.created_at",
    )
    .bind(snapshot.facet.as_str())
    .bind(payload_json)
    .bind(&snapshot.created_at)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn get_external_import_monitor_snapshot_query(
    pool: &SqlitePool,
    facet: &MediaFacet,
) -> AppResult<Option<ExternalImportMonitorSnapshot>> {
    let row = sqlx::query(
        "SELECT facet, payload_json, created_at
         FROM external_import_monitor_snapshots
         WHERE facet = ?",
    )
    .bind(facet.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let facet_value: String = row
        .try_get("facet")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let facet = MediaFacet::parse(&facet_value)
        .ok_or_else(|| AppError::Repository(format!("invalid snapshot facet: {facet_value}")))?;
    let payload =
        serde_json::from_str(&payload_json).map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(Some(ExternalImportMonitorSnapshot {
        facet,
        payload,
        created_at,
    }))
}

pub(crate) async fn delete_external_import_monitor_snapshot_query(
    pool: &SqlitePool,
    facet: &MediaFacet,
) -> AppResult<()> {
    sqlx::query("DELETE FROM external_import_monitor_snapshots WHERE facet = ?")
        .bind(facet.as_str())
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn queue_delete_download_command_query(
    pool: &SqlitePool,
    client_id: Option<&str>,
    client_type: &str,
    download_client_item_id: &str,
    is_history: bool,
    requested_by_user_id: Option<&str>,
) -> AppResult<AppDownloadQueueCommandRecord> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    let normalized_client_id = normalize_download_client_id(client_id);

    sqlx::query(
        "INSERT OR IGNORE INTO download_queue_commands
         (id, action, client_id, client_type, download_client_item_id, is_history, status, error_text, requested_by_user_id, started_at, finished_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, NULL, NULL, ?, ?)",
    )
    .bind(&id)
    .bind(DownloadQueueCommandAction::Delete.as_str())
    .bind(&normalized_client_id)
    .bind(client_type)
    .bind(download_client_item_id)
    .bind(if is_history { 1 } else { 0 })
    .bind(DownloadQueueDeleteStatus::Queued.as_str())
    .bind(requested_by_user_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let row = sqlx::query(
        "SELECT id, action, client_id, client_type, download_client_item_id, is_history, status, error_text,
                requested_by_user_id, started_at, finished_at, created_at, updated_at
         FROM download_queue_commands
         WHERE action = ?
           AND COALESCE(client_id, '') = ?
           AND client_type = ?
           AND download_client_item_id = ?
           AND is_history = ?
           AND status IN ('queued', 'running')
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )
    .bind(DownloadQueueCommandAction::Delete.as_str())
    .bind(&normalized_client_id)
    .bind(client_type)
    .bind(download_client_item_id)
    .bind(if is_history { 1 } else { 0 })
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    download_queue_command_record_from_row(&row)
}

pub(crate) async fn prune_terminal_delete_download_commands_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query(
        "DELETE FROM download_queue_commands
         WHERE action = 'delete'
           AND status IN ('completed', 'failed')
           AND updated_at < datetime('now', ?)",
    )
    .bind(&modifier)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn recover_stale_running_delete_download_commands_query(
    pool: &SqlitePool,
    stale_seconds: i64,
) -> AppResult<u64> {
    let cutoff = (Utc::now() - chrono::Duration::seconds(stale_seconds)).to_rfc3339();
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE download_queue_commands
         SET status = 'queued',
             error_text = NULL,
             started_at = NULL,
             finished_at = NULL,
             updated_at = ?
         WHERE action = 'delete'
           AND status = 'running'
           AND updated_at <= ?",
    )
    .bind(&now)
    .bind(&cutoff)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(result.rows_affected())
}

pub(crate) async fn list_pending_delete_download_commands_query(
    pool: &SqlitePool,
) -> AppResult<Vec<AppDownloadQueueCommandRecord>> {
    let rows = sqlx::query(
        "SELECT id, action, client_id, client_type, download_client_item_id, is_history, status, error_text,
                requested_by_user_id, started_at, finished_at, created_at, updated_at
         FROM download_queue_commands
         WHERE action = 'delete'
           AND status = 'queued'
         ORDER BY created_at ASC, id ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    rows.iter()
        .map(download_queue_command_record_from_row)
        .collect()
}

pub(crate) async fn update_delete_download_command_status_query(
    pool: &SqlitePool,
    id: &str,
    status: DownloadQueueDeleteStatus,
    error_text: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let started_at = match status {
        DownloadQueueDeleteStatus::Running => Some(now.clone()),
        _ => None,
    };
    let finished_at = match status {
        DownloadQueueDeleteStatus::Completed | DownloadQueueDeleteStatus::Failed => {
            Some(now.clone())
        }
        _ => None,
    };

    sqlx::query(
        "UPDATE download_queue_commands
         SET status = ?,
             error_text = ?,
             started_at = COALESCE(?, started_at),
             finished_at = ?,
             updated_at = ?
         WHERE id = ?",
    )
    .bind(status.as_str())
    .bind(error_text)
    .bind(started_at)
    .bind(finished_at)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn list_latest_delete_download_commands_for_sources_query(
    pool: &SqlitePool,
    sources: &[(Option<String>, String, String, bool)],
) -> AppResult<Vec<AppDownloadQueueCommandRecord>> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, action, client_id, client_type, download_client_item_id, is_history, status, error_text,
                requested_by_user_id, started_at, finished_at, created_at, updated_at
         FROM download_queue_commands
         WHERE action = 'delete' AND ",
    );

    query.push("(");
    for (idx, (client_id, client_type, download_client_item_id, is_history)) in
        sources.iter().enumerate()
    {
        if idx > 0 {
            query.push(" OR ");
        }
        let normalized_client_id = normalize_download_client_id(client_id.as_deref());
        query.push("(");
        if normalized_client_id.is_empty() {
            query.push("COALESCE(client_id, '') = ''");
        } else {
            query
                .push("(COALESCE(client_id, '') = ")
                .push_bind(normalized_client_id)
                .push(" OR COALESCE(client_id, '') = '')");
        }
        query
            .push(" AND client_type = ")
            .push_bind(client_type)
            .push(" AND download_client_item_id = ")
            .push_bind(download_client_item_id)
            .push(" AND is_history = ")
            .push_bind(if *is_history { 1 } else { 0 })
            .push(")");
    }
    query.push(") ORDER BY created_at DESC, id DESC");

    let rows = query
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut latest = std::collections::HashMap::new();
    for row in rows {
        let record = download_queue_command_record_from_row(&row)?;
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

pub(crate) async fn create_workflow_operation_query(
    pool: &SqlitePool,
    operation_type: String,
    status: String,
    actor_user_id: Option<String>,
    progress_json: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
) -> AppResult<WorkflowOperationRecord> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    let started_at = started_at.unwrap_or_else(|| now.clone());

    sqlx::query(
        "INSERT INTO workflow_operations
         (id, operation_type, status, actor_user_id, progress_json, job_key, trigger_source, summary_json, summary_text, error_text, started_at, completed_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL, NULL, NULL, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&operation_type)
    .bind(&status)
    .bind(&actor_user_id)
    .bind(&progress_json)
    .bind(&started_at)
    .bind(&completed_at)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(WorkflowOperationRecord {
        id,
        operation_type,
        status,
        job_key: None,
        trigger_source: None,
        actor_user_id,
        title_id: None,
        collection_id: None,
        episode_id: None,
        release_id: None,
        media_file_id: None,
        external_reference: None,
        progress_json,
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at: Some(started_at),
        completed_at,
        created_at: now.clone(),
        updated_at: now,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_job_workflow_operation_query(
    pool: &SqlitePool,
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
) -> AppResult<WorkflowOperationRecord> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    let started_at = started_at.unwrap_or_else(|| now.clone());

    sqlx::query(
        "INSERT INTO workflow_operations
         (id, operation_type, status, job_key, trigger_source, actor_user_id, progress_json, summary_json, summary_text, error_text, started_at, completed_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&operation_type)
    .bind(&status)
    .bind(&job_key)
    .bind(&trigger_source)
    .bind(&actor_user_id)
    .bind(&progress_json)
    .bind(&summary_json)
    .bind(&summary_text)
    .bind(&error_text)
    .bind(&started_at)
    .bind(&completed_at)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(WorkflowOperationRecord {
        id,
        operation_type,
        status,
        job_key: Some(job_key),
        trigger_source: Some(trigger_source),
        actor_user_id,
        title_id: None,
        collection_id: None,
        episode_id: None,
        release_id: None,
        media_file_id: None,
        external_reference: None,
        progress_json,
        summary_json,
        summary_text,
        error_text,
        started_at: Some(started_at),
        completed_at,
        created_at: now.clone(),
        updated_at: now,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_job_workflow_operation_query(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    progress_json: Option<String>,
    summary_json: Option<String>,
    summary_text: Option<String>,
    error_text: Option<String>,
    completed_at: Option<String>,
) -> AppResult<WorkflowOperationRecord> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    sqlx::query(
        "UPDATE workflow_operations
         SET status = ?,
             progress_json = ?,
             summary_json = ?,
             summary_text = ?,
             error_text = ?,
             completed_at = ?,
             updated_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(&progress_json)
    .bind(&summary_json)
    .bind(&summary_text)
    .bind(&error_text)
    .bind(&completed_at)
    .bind(&now)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let operation = get_workflow_operation_by_id_tx(&mut tx, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workflow operation {id}")))?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(operation)
}

pub(crate) async fn get_workflow_operation_by_id_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<WorkflowOperationRecord>> {
    let row = sqlx::query("SELECT * FROM workflow_operations WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    row.as_ref().map(workflow_operation_from_row).transpose()
}

async fn get_workflow_operation_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> AppResult<Option<WorkflowOperationRecord>> {
    let row = sqlx::query("SELECT * FROM workflow_operations WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    row.as_ref().map(workflow_operation_from_row).transpose()
}

pub(crate) async fn list_job_workflow_operations_query(
    pool: &SqlitePool,
    job_key: Option<&str>,
    limit: i64,
) -> AppResult<Vec<WorkflowOperationRecord>> {
    let rows = if let Some(job_key) = job_key {
        sqlx::query(
            "SELECT *
             FROM workflow_operations
             WHERE job_key = ?
             ORDER BY COALESCE(started_at, created_at) DESC
             LIMIT ?",
        )
        .bind(job_key)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?
    } else {
        sqlx::query(
            "SELECT *
             FROM workflow_operations
             WHERE job_key IS NOT NULL
             ORDER BY COALESCE(started_at, created_at) DESC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?
    };

    rows.iter().map(workflow_operation_from_row).collect()
}

pub(crate) async fn list_active_job_workflow_operations_query(
    pool: &SqlitePool,
) -> AppResult<Vec<WorkflowOperationRecord>> {
    let rows = sqlx::query(
        "SELECT *
         FROM workflow_operations
         WHERE job_key IS NOT NULL
           AND status IN ('queued', 'running', 'discovering')
         ORDER BY COALESCE(started_at, created_at) ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    rows.iter().map(workflow_operation_from_row).collect()
}

pub(crate) async fn upsert_library_probe_signature_query(
    pool: &SqlitePool,
    title_id: &str,
    path: &str,
    probe_signature_scheme: Option<String>,
    probe_signature_value: Option<String>,
    last_probed_at: Option<String>,
    last_changed_at: Option<String>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO library_probe_signatures
         (title_id, path, probe_signature_scheme, probe_signature_value, last_probed_at, last_changed_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(title_id) DO UPDATE SET
            path = excluded.path,
            probe_signature_scheme = excluded.probe_signature_scheme,
            probe_signature_value = excluded.probe_signature_value,
            last_probed_at = excluded.last_probed_at,
            last_changed_at = excluded.last_changed_at,
            updated_at = excluded.updated_at",
    )
    .bind(title_id)
    .bind(path)
    .bind(probe_signature_scheme)
    .bind(probe_signature_value)
    .bind(last_probed_at)
    .bind(last_changed_at)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn get_library_probe_signature_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Option<LibraryProbeSignatureRecord>> {
    let row = sqlx::query("SELECT * FROM library_probe_signatures WHERE title_id = ?")
        .bind(title_id)
        .fetch_optional(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    row.map(|row| {
        Ok(LibraryProbeSignatureRecord {
            title_id: row
                .try_get("title_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            path: row
                .try_get("path")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            probe_signature_scheme: row
                .try_get("probe_signature_scheme")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            probe_signature_value: row
                .try_get("probe_signature_value")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            last_probed_at: row
                .try_get("last_probed_at")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            last_changed_at: row
                .try_get("last_changed_at")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|err| AppError::Repository(err.to_string()))?,
        })
    })
    .transpose()
}

pub(crate) async fn delete_library_probe_signatures_for_title_ids_query(
    pool: &SqlitePool,
    title_ids: &[String],
) -> AppResult<u32> {
    if title_ids.is_empty() {
        return Ok(0);
    }

    let mut builder =
        QueryBuilder::<Sqlite>::new("DELETE FROM library_probe_signatures WHERE title_id IN (");
    let mut separated = builder.separated(", ");
    for title_id in title_ids {
        separated.push_bind(title_id);
    }
    separated.push_unseparated(")");

    let result = builder
        .build()
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn create_release_download_attempt_query(
    pool: &SqlitePool,
    title_id: Option<String>,
    source_hint: Option<String>,
    source_title: Option<String>,
    outcome: ReleaseDownloadAttemptOutcome,
    error_message: Option<String>,
    source_password: Option<String>,
) -> AppResult<()> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, error_message, source_password, attempted_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&title_id)
    .bind(&source_hint)
    .bind(&source_title)
    .bind(outcome.as_str())
    .bind(&error_message)
    .bind(&source_password)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn get_latest_source_password_query(
    pool: &SqlitePool,
    title_id: Option<&str>,
    source_hint: Option<&str>,
    source_title: Option<&str>,
) -> AppResult<Option<String>> {
    let mut sql = String::from(
        "SELECT source_password
         FROM release_download_attempts
         WHERE source_password IS NOT NULL",
    );

    let mut filters = Vec::new();
    if title_id.is_some() {
        filters.push("title_id = ?");
    }
    if source_hint.is_some() {
        filters.push("source_hint = ?");
    }
    if source_title.is_some() {
        filters.push("source_title = ?");
    }

    if !filters.is_empty() {
        sql.push(' ');
        sql.push_str("AND ");
        sql.push_str(&filters.join(" AND "));
    }

    sql.push_str(" ORDER BY attempted_at DESC LIMIT 1");

    let mut query = sqlx::query(&sql);
    if let Some(title_id) = title_id {
        query = query.bind(title_id);
    }
    if let Some(source_hint) = source_hint {
        query = query.bind(source_hint);
    }
    if let Some(source_title) = source_title {
        query = query.bind(source_title);
    }

    let row = query
        .fetch_optional(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(row
            .try_get::<Option<String>, _>("source_password")
            .map_err(|err| AppError::Repository(err.to_string()))?),
        None => Ok(None),
    }
}

pub(crate) async fn create_import_request_query(
    pool: &SqlitePool,
    source_system: String,
    source_ref: String,
    import_type: String,
    payload_json: String,
) -> AppResult<String> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    let is_rename = ImportType::parse(&import_type).is_some_and(|t| t.is_rename());
    let rename_plan_json = if is_rename {
        Some(payload_json.clone())
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO imports
         (id, source_system, source_ref, import_type, status, payload_json, rename_plan_json, result_json, started_at, finished_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(source_system, source_ref, import_type) DO UPDATE SET
            status = excluded.status,
            payload_json = excluded.payload_json,
            rename_plan_json = excluded.rename_plan_json,
            result_json = NULL,
            started_at = NULL,
            finished_at = NULL,
            updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&source_system)
    .bind(&source_ref)
    .bind(&import_type)
    .bind(ImportStatus::Pending.as_str())
    .bind(&payload_json)
    .bind(&rename_plan_json)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let row = sqlx::query(
        "SELECT id
         FROM imports
         WHERE source_system = ?
           AND source_ref = ?
           AND import_type = ?",
    )
    .bind(&source_system)
    .bind(&source_ref)
    .bind(&import_type)
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let persisted_id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(persisted_id)
}

pub(crate) async fn get_import_by_id_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<ImportRecord>> {
    let row = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE id = ?
         LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.as_ref().map(import_record_from_row).transpose()
}

pub(crate) async fn get_import_by_source_ref_query(
    pool: &SqlitePool,
    source_system: &str,
    source_ref: &str,
) -> AppResult<Option<ImportRecord>> {
    let row = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE source_system = ? AND source_ref = ?
         ORDER BY updated_at DESC
         LIMIT 1",
    )
    .bind(source_system)
    .bind(source_ref)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.as_ref().map(import_record_from_row).transpose()
}

pub(crate) async fn get_import_by_source_ref_and_type_query(
    pool: &SqlitePool,
    source_system: &str,
    source_ref: &str,
    import_type: ImportType,
) -> AppResult<Option<ImportRecord>> {
    let row = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE source_system = ? AND source_ref = ? AND import_type = ?
         ORDER BY updated_at DESC
         LIMIT 1",
    )
    .bind(source_system)
    .bind(source_ref)
    .bind(import_type.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.as_ref().map(import_record_from_row).transpose()
}

pub(crate) async fn update_import_status_query(
    pool: &SqlitePool,
    import_id: &str,
    status: &str,
    result_json: Option<String>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let is_terminal = ImportStatus::parse(status).is_some_and(|s| s.is_terminal());

    sqlx::query(
        "UPDATE imports
         SET status = ?,
             result_json = ?,
             started_at = CASE WHEN started_at IS NULL THEN ? ELSE started_at END,
             finished_at = CASE WHEN ? THEN ? ELSE finished_at END,
             updated_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(&result_json)
    .bind(&now)
    .bind(is_terminal)
    .bind(&now)
    .bind(&now)
    .bind(import_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn recover_stale_processing_imports_query(
    pool: &SqlitePool,
    stale_seconds: i64,
) -> AppResult<u64> {
    let now = Utc::now();
    let cutoff = (now - chrono::Duration::seconds(stale_seconds)).to_rfc3339();
    let now_str = now.to_rfc3339();

    let result = sqlx::query(
        "UPDATE imports
         SET status = 'failed',
             result_json = '{\"error\":\"stale processing recovery\"}',
             finished_at = ?,
             updated_at = ?
         WHERE status = 'processing'
           AND import_type != 'manual_import'
           AND updated_at < ?",
    )
    .bind(&now_str)
    .bind(&now_str)
    .bind(&cutoff)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(result.rows_affected())
}

pub(crate) async fn recover_stale_processing_imports_for_type_query(
    pool: &SqlitePool,
    import_type: ImportType,
    stale_seconds: i64,
) -> AppResult<u64> {
    let now = Utc::now();
    let cutoff = (now - chrono::Duration::seconds(stale_seconds)).to_rfc3339();
    let now_str = now.to_rfc3339();

    let result = sqlx::query(
        "UPDATE imports
         SET status = 'failed',
             result_json = '{\"error\":\"stale processing recovery\"}',
             finished_at = ?,
             updated_at = ?
         WHERE status = 'processing'
           AND import_type = ?
           AND updated_at < ?",
    )
    .bind(&now_str)
    .bind(&now_str)
    .bind(import_type.as_str())
    .bind(&cutoff)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(result.rows_affected())
}

pub(crate) async fn list_pending_imports_query(pool: &SqlitePool) -> AppResult<Vec<ImportRecord>> {
    let rows = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE status IN ('queued', 'pending', 'running', 'processing')
         ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(import_record_from_row(&row)?);
    }

    Ok(out)
}

pub(crate) async fn list_pending_imports_for_type_query(
    pool: &SqlitePool,
    import_type: ImportType,
) -> AppResult<Vec<ImportRecord>> {
    let rows = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE import_type = ?
           AND status IN ('queued', 'pending', 'running', 'processing')
         ORDER BY created_at ASC",
    )
    .bind(import_type.as_str())
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(import_record_from_row(&row)?);
    }

    Ok(out)
}

pub(crate) async fn list_imports_for_sources_query(
    pool: &SqlitePool,
    sources: &[(String, String)],
) -> AppResult<Vec<ImportRecord>> {
    if sources.is_empty() {
        return Ok(vec![]);
    }

    let mut builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE ",
    );

    for (index, (source_system, source_ref)) in sources.iter().enumerate() {
        if index > 0 {
            builder.push(" OR ");
        }

        builder
            .push("(source_system = ")
            .push_bind(source_system)
            .push(" AND source_ref = ")
            .push_bind(source_ref)
            .push(")");
    }
    builder.push(" ORDER BY updated_at DESC");

    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(import_record_from_row(&row)?);
    }

    Ok(out)
}

pub(crate) async fn list_imports_query(
    pool: &SqlitePool,
    limit: i64,
) -> AppResult<Vec<ImportRecord>> {
    let limit = limit.clamp(1, 500);
    let rows = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(import_record_from_row(&row)?);
    }

    Ok(out)
}

pub(crate) async fn list_failed_release_download_attempts_query(
    pool: &SqlitePool,
    limit: i64,
) -> AppResult<Vec<ReleaseDownloadFailureSignatureRecord>> {
    let limit = limit.clamp(1, 20_000);
    let rows = sqlx::query(
        "SELECT source_hint, source_title
         FROM (
                     SELECT source_hint,
                                    source_title,
                                    attempted_at AS last_attempted_at,
                                    ROW_NUMBER() OVER (
                                        PARTITION BY LOWER(TRIM(source_title))
                                        ORDER BY attempted_at DESC
                                    ) AS row_number
           FROM release_download_attempts
           WHERE outcome = 'failed'
                         AND COALESCE(TRIM(source_title), '') <> ''
         )
                 WHERE row_number = 1
         ORDER BY last_attempted_at DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let source_hint: Option<String> = row
            .try_get("source_hint")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let source_title: Option<String> = row
            .try_get("source_title")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        out.push(ReleaseDownloadFailureSignatureRecord {
            source_hint,
            source_title,
        });
    }

    Ok(out)
}

pub(crate) async fn list_failed_release_download_attempts_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
    limit: i64,
) -> AppResult<Vec<TitleReleaseBlocklistRecord>> {
    let limit = limit.clamp(1, 1_000);
    let rows = sqlx::query(
        "SELECT source_hint, source_title, error_message, attempted_at
         FROM (
           SELECT source_hint,
                  source_title,
                  error_message,
                  attempted_at,
                  ROW_NUMBER() OVER (
                                        PARTITION BY LOWER(TRIM(source_title))
                    ORDER BY attempted_at DESC
                  ) AS row_number
           FROM release_download_attempts
           WHERE outcome = 'failed'
             AND title_id = ?
                         AND COALESCE(TRIM(source_title), '') <> ''
         )
         WHERE row_number = 1
         ORDER BY attempted_at DESC
         LIMIT ?",
    )
    .bind(title_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let source_hint: Option<String> = row
            .try_get("source_hint")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let source_title: Option<String> = row
            .try_get("source_title")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let error_message: Option<String> = row
            .try_get("error_message")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let attempted_at: String = row
            .try_get("attempted_at")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        out.push(TitleReleaseBlocklistRecord {
            source_hint,
            source_title,
            error_message,
            attempted_at,
        });
    }

    Ok(out)
}

pub(crate) async fn record_download_submission_query(
    pool: &SqlitePool,
    submission: &DownloadSubmission,
) -> AppResult<()> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    record_download_submission_tx(&mut tx, submission).await?;

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))
}

pub(crate) async fn commit_successful_grab_query(
    pool: &SqlitePool,
    commit: &SuccessfulGrabCommit,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    record_download_submission_tx(&mut tx, &commit.download_submission).await?;

    let mut covered_wanted_item_ids = commit.covered_wanted_item_ids.clone();
    if !covered_wanted_item_ids
        .iter()
        .any(|id| id == &commit.wanted_item_id)
    {
        covered_wanted_item_ids.push(commit.wanted_item_id.clone());
    }
    covered_wanted_item_ids.sort();
    covered_wanted_item_ids.dedup();

    for wanted_item_id in &covered_wanted_item_ids {
        sqlx::query(
            "UPDATE wanted_items
             SET status = ?, next_search_at = ?, last_search_at = ?,
                 search_count = ?, current_score = ?, grabbed_release = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(WantedStatus::Grabbed.as_str())
        .bind(Option::<String>::None)
        .bind(commit.last_search_at.as_deref())
        .bind(commit.search_count)
        .bind(commit.current_score)
        .bind(&commit.grabbed_release)
        .bind(&now)
        .bind(wanted_item_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    if let Some(pending_release_id) = commit.grabbed_pending_release_id.as_deref() {
        sqlx::query(
            "UPDATE pending_releases
             SET status = ?, grabbed_at = ?
             WHERE id = ?",
        )
        .bind(PendingReleaseStatus::Grabbed.as_str())
        .bind(commit.grabbed_at.as_deref())
        .bind(pending_release_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    for wanted_item_id in &covered_wanted_item_ids {
        supersede_pending_release_siblings_tx(
            &mut tx,
            wanted_item_id,
            commit.grabbed_pending_release_id.as_deref(),
        )
        .await?;
    }

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))
}

async fn record_download_submission_tx(
    tx: &mut Transaction<'_, Sqlite>,
    submission: &DownloadSubmission,
) -> AppResult<()> {
    let id = Id::new().0;
    let (episode_id, collection_id) = persisted_submission_scope(&submission.scope);
    let download_client_id = normalize_download_client_id(submission.download_client_id.as_deref());

    sqlx::query(
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(download_client_id, download_client_type, download_client_item_id) DO UPDATE
         SET title_id = excluded.title_id,
             facet = excluded.facet,
             source_hint = excluded.source_hint,
             source_kind = excluded.source_kind,
             source_title = excluded.source_title,
             request_signature = excluded.request_signature,
             episode_id = excluded.episode_id,
             collection_id = excluded.collection_id",
    )
    .bind(&id)
    .bind(&submission.title_id)
    .bind(&submission.facet)
    .bind(&download_client_id)
    .bind(&submission.download_client_type)
    .bind(&submission.download_client_item_id)
    .bind(submission.source_hint.as_deref())
    .bind(submission.source_kind.map(|value| value.as_str()))
    .bind(submission.source_title.as_deref())
    .bind(submission.request_signature.as_deref())
    .bind(episode_id)
    .bind(collection_id)
    .execute(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    replace_download_submission_episode_links_tx(
        tx,
        &download_client_id,
        &submission.download_client_type,
        &submission.download_client_item_id,
        persisted_episode_set_ids(&submission.scope),
    )
    .await?;

    Ok(())
}

async fn replace_download_submission_episode_links_tx(
    tx: &mut Transaction<'_, Sqlite>,
    download_client_id: &str,
    download_client_type: &str,
    download_client_item_id: &str,
    episode_ids: &[String],
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM download_submission_episode_links
         WHERE download_client_id = ?
           AND download_client_type = ?
           AND download_client_item_id = ?",
    )
    .bind(download_client_id)
    .bind(download_client_type)
    .bind(download_client_item_id)
    .execute(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    for episode_id in episode_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO download_submission_episode_links
             (download_client_id, download_client_type, download_client_item_id, episode_id)
             VALUES (?, ?, ?, ?)",
        )
        .bind(download_client_id)
        .bind(download_client_type)
        .bind(download_client_item_id)
        .bind(episode_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

async fn supersede_pending_release_siblings_tx(
    tx: &mut Transaction<'_, Sqlite>,
    wanted_item_id: &str,
    except_id: Option<&str>,
) -> AppResult<()> {
    match except_id {
        Some(except_id) => {
            sqlx::query(
                "UPDATE pending_releases
                 SET status = 'superseded'
                 WHERE wanted_item_id = ?
                   AND id != ?
                   AND status IN ('waiting', 'standby')",
            )
            .bind(wanted_item_id)
            .bind(except_id)
            .execute(&mut **tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        }
        None => {
            sqlx::query(
                "UPDATE pending_releases
                 SET status = 'superseded'
                 WHERE wanted_item_id = ?
                   AND status IN ('waiting', 'standby')",
            )
            .bind(wanted_item_id)
            .execute(&mut **tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        }
    }

    Ok(())
}

pub(crate) async fn find_download_submission_query(
    pool: &SqlitePool,
    identity: &DownloadSourceIdentity,
) -> AppResult<Option<DownloadSubmission>> {
    let row = sqlx::query(
        "SELECT title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id,
                (SELECT group_concat(link.episode_id, char(31))
                   FROM download_submission_episode_links link
                  WHERE link.download_client_id = download_submissions.download_client_id
                    AND link.download_client_type = download_submissions.download_client_type
                    AND link.download_client_item_id = download_submissions.download_client_item_id) AS episode_set_ids
         FROM download_submissions
         WHERE download_client_type = ?
           AND download_client_item_id = ?
           AND download_client_id = ?
         LIMIT 1",
    )
    .bind(&identity.client_type)
    .bind(&identity.item_id)
    .bind(normalize_download_client_id(identity.client_id.as_deref()))
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(download_submission_from_row(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn list_download_submissions_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<DownloadSubmission>> {
    let rows = sqlx::query(
        "SELECT title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id,
                (SELECT group_concat(link.episode_id, char(31))
                   FROM download_submission_episode_links link
                  WHERE link.download_client_id = download_submissions.download_client_id
                    AND link.download_client_type = download_submissions.download_client_type
                    AND link.download_client_item_id = download_submissions.download_client_item_id) AS episode_set_ids
         FROM download_submissions
         WHERE title_id = ?",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(download_submission_from_row(&row)?);
    }

    Ok(out)
}

pub(crate) async fn list_download_submissions_for_client_items_query(
    pool: &SqlitePool,
    client_items: &[DownloadSourceIdentity],
) -> AppResult<Vec<DownloadSubmission>> {
    let client_item_chunks = chunk_download_submission_client_items(client_items);
    if client_item_chunks.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for client_items in client_item_chunks {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id,
                    (SELECT group_concat(link.episode_id, char(31))
                       FROM download_submission_episode_links link
                      WHERE link.download_client_id = download_submissions.download_client_id
                        AND link.download_client_type = download_submissions.download_client_type
                        AND link.download_client_item_id = download_submissions.download_client_item_id) AS episode_set_ids
             FROM download_submissions
             WHERE ",
        );
        for (idx, identity) in client_items.iter().enumerate() {
            if idx > 0 {
                query.push(" OR ");
            }
            query.push("(download_client_type = ");
            query.push_bind(&identity.client_type);
            query.push(" AND download_client_item_id = ");
            query.push_bind(&identity.item_id);
            query.push(" AND download_client_id = ");
            query.push_bind(normalize_download_client_id(identity.client_id.as_deref()));
            query.push(")");
        }

        let rows = query
            .build()
            .fetch_all(pool)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        out.reserve(rows.len());
        for row in rows {
            out.push(download_submission_from_row(&row)?);
        }
    }

    Ok(out)
}

pub(crate) async fn find_download_submission_by_title_and_request_signature_query(
    pool: &SqlitePool,
    title_id: &str,
    request_signature: &str,
) -> AppResult<Option<DownloadSubmission>> {
    let row = sqlx::query(
        "SELECT title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id,
                (SELECT group_concat(link.episode_id, char(31))
                   FROM download_submission_episode_links link
                  WHERE link.download_client_id = download_submissions.download_client_id
                    AND link.download_client_type = download_submissions.download_client_type
                    AND link.download_client_item_id = download_submissions.download_client_item_id) AS episode_set_ids
         FROM download_submissions
         WHERE title_id = ? AND request_signature = ?
           AND COALESCE(tracked_state, '') = ''
           AND submitted_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-30 seconds')
         ORDER BY submitted_at DESC, id DESC
         LIMIT 1",
    )
    .bind(title_id)
    .bind(request_signature)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(download_submission_from_row(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn delete_download_submission_by_client_item_id_query(
    pool: &SqlitePool,
    identity: &DownloadSourceIdentity,
) -> AppResult<()> {
    let normalized_client_id = normalize_download_client_id(identity.client_id.as_deref());
    sqlx::query(
        "DELETE FROM download_submission_episode_links
         WHERE download_client_id = ?
           AND download_client_type = ?
           AND download_client_item_id = ?",
    )
    .bind(&normalized_client_id)
    .bind(&identity.client_type)
    .bind(&identity.item_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    sqlx::query(
        "DELETE FROM download_submissions
         WHERE download_client_id = ?
           AND download_client_type = ?
           AND download_client_item_id = ?",
    )
    .bind(&normalized_client_id)
    .bind(&identity.client_type)
    .bind(&identity.item_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn delete_download_submissions_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM download_submission_episode_links
         WHERE EXISTS (
             SELECT 1
               FROM download_submissions
              WHERE download_submissions.download_client_id = download_submission_episode_links.download_client_id
                AND download_submissions.download_client_type = download_submission_episode_links.download_client_type
                AND download_submissions.download_client_item_id = download_submission_episode_links.download_client_item_id
                AND download_submissions.title_id = ?
         )",
    )
    .bind(title_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM download_submissions WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

// ── TrackedDownloads (plan 055) ──────────────────────────────────────────────

pub(crate) async fn update_tracked_state_query(
    pool: &SqlitePool,
    identity: &DownloadSourceIdentity,
    tracked_state: &str,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let id = Id::new().0;
    let normalized_client_id = normalize_download_client_id(identity.client_id.as_deref());
    sqlx::query(
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, episode_id, collection_id, tracked_state, tracked_state_at)
         VALUES (?, '', '', ?, ?, ?, NULL, NULL, NULL, NULL, NULL, NULL, ?, ?)
         ON CONFLICT(download_client_id, download_client_type, download_client_item_id) DO UPDATE
         SET tracked_state = excluded.tracked_state,
             tracked_state_at = excluded.tracked_state_at",
    )
    .bind(&id)
    .bind(&normalized_client_id)
    .bind(&identity.client_type)
    .bind(&identity.item_id)
    .bind(tracked_state)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn get_tracked_state_query(
    pool: &SqlitePool,
    identity: &DownloadSourceIdentity,
) -> AppResult<Option<String>> {
    let row = sqlx::query(
        "SELECT tracked_state FROM download_submissions
         WHERE download_client_type = ?
           AND download_client_item_id = ?
           AND download_client_id = ?
         LIMIT 1",
    )
    .bind(&identity.client_type)
    .bind(&identity.item_id)
    .bind(normalize_download_client_id(identity.client_id.as_deref()))
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => {
            let state: Option<String> = row
                .try_get("tracked_state")
                .map_err(|err| AppError::Repository(err.to_string()))?;
            Ok(state)
        }
        None => Ok(None),
    }
}

pub(crate) async fn insert_import_artifact_query(
    pool: &SqlitePool,
    artifact: &scryer_application::ImportArtifact,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO download_import_artifacts
         (id, source_system, source_ref, import_id, relative_path, normalized_file_name,
          media_kind, title_id, episode_id, season_number, episode_number,
          result, reason_code, imported_media_file_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(artifact.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn list_import_artifacts_by_source_ref_query(
    pool: &SqlitePool,
    source_system: &str,
    source_ref: &str,
) -> AppResult<Vec<scryer_application::ImportArtifact>> {
    let rows = sqlx::query(
        "SELECT id, source_system, source_ref, import_id, relative_path,
                normalized_file_name, media_kind, title_id, episode_id,
                season_number, episode_number, result, reason_code,
                imported_media_file_id, created_at
         FROM download_import_artifacts
         WHERE source_system = ? AND source_ref = ?
         ORDER BY created_at",
    )
    .bind(source_system)
    .bind(source_ref)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(scryer_application::ImportArtifact {
            id: row
                .try_get("id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_system: row
                .try_get("source_system")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_ref: row
                .try_get("source_ref")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            import_id: row
                .try_get("import_id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            relative_path: row
                .try_get("relative_path")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            normalized_file_name: row
                .try_get("normalized_file_name")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            media_kind: row
                .try_get("media_kind")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            title_id: row
                .try_get("title_id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            episode_id: row
                .try_get("episode_id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            season_number: row
                .try_get("season_number")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            episode_number: row
                .try_get("episode_number")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            result: row
                .try_get("result")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            reason_code: row
                .try_get("reason_code")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            imported_media_file_id: row
                .try_get("imported_media_file_id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            created_at: {
                let s: String = row
                    .try_get("created_at")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now())
            },
        });
    }
    Ok(out)
}

pub(crate) async fn count_import_artifacts_by_result_query(
    pool: &SqlitePool,
    source_system: &str,
    source_ref: &str,
    result: &str,
) -> AppResult<u64> {
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM download_import_artifacts
         WHERE source_system = ? AND source_ref = ? AND result = ?",
    )
    .bind(source_system)
    .bind(source_ref)
    .bind(result)
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let count: i64 = row
        .try_get("cnt")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(count as u64)
}
