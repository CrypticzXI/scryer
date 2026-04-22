use scryer_application::{AppError, AppResult};
use scryer_domain::DomainEventType;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

pub(crate) async fn delete_release_decisions_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query("DELETE FROM release_decisions WHERE created_at < datetime('now', ?)")
        .bind(&modifier)
        .execute(pool)
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "housekeeping: release_decisions cleanup failed: {e}"
            ))
        })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_release_attempts_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query(
        "DELETE FROM release_download_attempts WHERE attempted_at < datetime('now', ?) AND outcome != 'pending'",
    )
    .bind(&modifier)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(format!("housekeeping: release_attempts cleanup failed: {e}")))?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_dispatched_event_outboxes_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query(
        "DELETE FROM event_outboxes WHERE status = 'dispatched' AND created_at < datetime('now', ?)",
    )
    .bind(&modifier)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(format!("housekeeping: event_outboxes cleanup failed: {e}")))?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_history_events_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query("DELETE FROM history_events WHERE occurred_at < datetime('now', ?)")
        .bind(&modifier)
        .execute(pool)
        .await
        .map_err(|e| {
            AppError::Repository(format!("housekeeping: history_events cleanup failed: {e}"))
        })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_domain_events_older_than_for_types_query(
    pool: &SqlitePool,
    days: i64,
    event_types: &[DomainEventType],
) -> AppResult<u32> {
    if event_types.is_empty() {
        return Ok(0);
    }

    let modifier = format!("-{days} days");
    let mut builder = QueryBuilder::<Sqlite>::new(
        "DELETE FROM domain_events WHERE occurred_at < datetime('now', ",
    );
    builder.push_bind(&modifier);
    builder.push(") AND event_type IN (");
    let mut separated = builder.separated(", ");
    for event_type in event_types {
        separated.push_bind(event_type.as_str());
    }
    separated.push_unseparated(")");

    let result = builder.build().execute(pool).await.map_err(|e| {
        AppError::Repository(format!("housekeeping: domain_events cleanup failed: {e}"))
    })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_download_import_artifacts_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query(
        "DELETE FROM download_import_artifacts
         WHERE created_at < datetime('now', ?)
           AND (
                import_id IS NULL
                OR NOT EXISTS (
                    SELECT 1
                    FROM imports
                    WHERE imports.id = download_import_artifacts.import_id
                )
                OR EXISTS (
                    SELECT 1
                    FROM imports
                    WHERE imports.id = download_import_artifacts.import_id
                      AND imports.status IN ('completed', 'failed', 'skipped')
                )
           )",
    )
    .bind(&modifier)
    .execute(pool)
    .await
    .map_err(|e| {
        AppError::Repository(format!(
            "housekeeping: download_import_artifacts cleanup failed: {e}"
        ))
    })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_terminal_imports_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query(
        "DELETE FROM imports
         WHERE status IN ('completed', 'failed', 'skipped')
           AND updated_at < datetime('now', ?)",
    )
    .bind(&modifier)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(format!("housekeeping: imports cleanup failed: {e}")))?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_terminal_download_queue_commands_older_than_query(
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
    .map_err(|e| {
        AppError::Repository(format!(
            "housekeeping: download_queue_commands cleanup failed: {e}"
        ))
    })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_rule_set_history_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query("DELETE FROM rule_set_history WHERE created_at < datetime('now', ?)")
        .bind(&modifier)
        .execute(pool)
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "housekeeping: rule_set_history cleanup failed: {e}"
            ))
        })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn list_all_media_file_paths_query(
    pool: &SqlitePool,
) -> AppResult<Vec<(String, String)>> {
    let rows = sqlx::query("SELECT id, file_path FROM media_files")
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Repository(format!("housekeeping: list media files failed: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: String = row.get("id");
        let file_path: String = row.get("file_path");
        out.push((id, file_path));
    }
    Ok(out)
}

pub(crate) async fn delete_media_files_by_ids_query(
    pool: &SqlitePool,
    ids: &[String],
) -> AppResult<u32> {
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
    let sql = format!(
        "DELETE FROM media_files WHERE id IN ({})",
        placeholders.join(", ")
    );

    let mut query = sqlx::query(&sql);
    for id in ids {
        query = query.bind(id);
    }

    let result = query.execute(pool).await.map_err(|e| {
        AppError::Repository(format!("housekeeping: delete media files failed: {e}"))
    })?;

    Ok(result.rows_affected() as u32)
}
