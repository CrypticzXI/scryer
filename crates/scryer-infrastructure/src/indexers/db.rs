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
    query_delta: u32,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO indexer_api_quotas (indexer_id, api_current, api_max, grab_current, grab_max, queries_today, last_query_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
         ON CONFLICT(indexer_id) DO UPDATE SET
           api_current = COALESCE(excluded.api_current, indexer_api_quotas.api_current),
           api_max = COALESCE(excluded.api_max, indexer_api_quotas.api_max),
           grab_current = COALESCE(excluded.grab_current, indexer_api_quotas.grab_current),
           grab_max = COALESCE(excluded.grab_max, indexer_api_quotas.grab_max),
           queries_today = CASE
             WHEN julianday('now') - julianday(indexer_api_quotas.last_reset_at) >= 1.0
             THEN excluded.queries_today
             ELSE indexer_api_quotas.queries_today + excluded.queries_today
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
    .bind(query_delta as i64)
    .execute(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(())
}
