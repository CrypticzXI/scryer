use scryer_application::{AppError, AppResult};
use sqlx::SqlitePool;

/// Upsert quota snapshot for an indexer after a search response.
pub(crate) async fn upsert_indexer_quota(
    pool: &SqlitePool,
    indexer_id: &str,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO indexer_api_quotas (indexer_id, api_current, api_max, grab_current, grab_max, queries_today, last_query_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 1, datetime('now'), datetime('now'))
         ON CONFLICT(indexer_id) DO UPDATE SET
           api_current = excluded.api_current,
           api_max = excluded.api_max,
           grab_current = excluded.grab_current,
           grab_max = excluded.grab_max,
           queries_today = CASE
             WHEN julianday('now') - julianday(indexer_api_quotas.last_reset_at) >= 1.0
             THEN 1
             ELSE indexer_api_quotas.queries_today + 1
           END,
           last_reset_at = CASE
             WHEN julianday('now') - julianday(indexer_api_quotas.last_reset_at) >= 1.0
             THEN datetime('now')
             ELSE indexer_api_quotas.last_reset_at
           END,
           last_query_at = datetime('now'),
           updated_at = datetime('now')",
    )
    .bind(indexer_id)
    .bind(api_current.map(|value| value as i64))
    .bind(api_max.map(|value| value as i64))
    .bind(grab_current.map(|value| value as i64))
    .bind(grab_max.map(|value| value as i64))
    .execute(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(())
}
