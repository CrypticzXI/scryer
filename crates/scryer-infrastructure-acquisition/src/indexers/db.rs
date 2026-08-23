use scryer_application::AppResult;

use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

/// Upsert quota snapshot for an indexer after a search response.
pub async fn upsert_indexer_quota(
    datastore: &StoreDatastore,
    indexer_id: &str,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
    query_delta: u32,
) -> AppResult<()> {
    SqlRuntime::execute_write(
        datastore,
        "upsert_indexer_quota",
        "INSERT INTO indexer_api_quotas (indexer_id, api_current, api_max, grab_current, grab_max, queries_today, last_query_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))
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
        vec![
            SqlArg::Text(indexer_id.to_string()),
            SqlArg::OptI64(api_current.map(i64::from)),
            SqlArg::OptI64(api_max.map(i64::from)),
            SqlArg::OptI64(grab_current.map(i64::from)),
            SqlArg::OptI64(grab_max.map(i64::from)),
            SqlArg::I64(i64::from(query_delta)),
        ],
    )
    .await?;
    Ok(())
}
