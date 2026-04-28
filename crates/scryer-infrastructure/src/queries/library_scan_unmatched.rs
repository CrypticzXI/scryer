use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use scryer_application::{
    AppError, AppResult, LibraryScanUnmatchedItem, LibraryScanUnmatchedSearchAttempt,
    PendingImportStatus,
};
use scryer_domain::MediaFacet;

use super::common::repository_error_from_sqlx;

fn row_to_library_scan_unmatched_item(row: &SqliteRow) -> AppResult<LibraryScanUnmatchedItem> {
    let facet_raw: String = row
        .try_get("facet")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let facet = MediaFacet::parse(&facet_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "library scan unmatched item has invalid facet '{facet_raw}'"
        ))
    })?;
    let search_attempts_json: String = row
        .try_get("search_attempts_json")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let status_raw: String = row
        .try_get("status")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let search_attempts =
        serde_json::from_str::<Vec<LibraryScanUnmatchedSearchAttempt>>(&search_attempts_json)
            .map_err(|err| AppError::Repository(err.to_string()))?;
    let status = PendingImportStatus::parse(&status_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "library scan unmatched item has invalid status '{status_raw}'"
        ))
    })?;

    Ok(LibraryScanUnmatchedItem {
        id: row
            .try_get("id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        facet,
        status,
        title_id: row.try_get("title_id").unwrap_or(None),
        scan_session_id: row
            .try_get("scan_session_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        scan_root: row
            .try_get("scan_root")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        item_path: row
            .try_get("item_path")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        display_name: row
            .try_get("display_name")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        query: row
            .try_get("query")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        year_hint: row.try_get("year_hint").unwrap_or(None),
        reason_code: row
            .try_get("reason_code")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        error_message: row.try_get("error_message").unwrap_or(None),
        search_attempts,
        created_at: row
            .try_get("created_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|err| AppError::Repository(err.to_string()))?,
    })
}

pub(crate) async fn upsert_library_scan_unmatched_item_query(
    pool: &SqlitePool,
    item: &LibraryScanUnmatchedItem,
) -> AppResult<String> {
    let search_attempts_json = serde_json::to_string(&item.search_attempts)
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query(
        "INSERT INTO library_scan_unmatched_items
         (id, facet, title_id, scan_session_id, scan_root, item_path, display_name, query,
          year_hint, reason_code, error_message, search_attempts_json, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            facet = excluded.facet,
            title_id = excluded.title_id,
            scan_session_id = excluded.scan_session_id,
            scan_root = excluded.scan_root,
            item_path = excluded.item_path,
            display_name = excluded.display_name,
            query = excluded.query,
            year_hint = excluded.year_hint,
            reason_code = excluded.reason_code,
            error_message = excluded.error_message,
            search_attempts_json = excluded.search_attempts_json,
            status = CASE
                WHEN library_scan_unmatched_items.status = 'ignored' AND excluded.status = 'pending'
                    THEN library_scan_unmatched_items.status
                ELSE excluded.status
            END,
            updated_at = excluded.updated_at",
    )
    .bind(&item.id)
    .bind(item.facet.as_str())
    .bind(&item.title_id)
    .bind(&item.scan_session_id)
    .bind(&item.scan_root)
    .bind(&item.item_path)
    .bind(&item.display_name)
    .bind(&item.query)
    .bind(item.year_hint)
    .bind(&item.reason_code)
    .bind(&item.error_message)
    .bind(search_attempts_json)
    .bind(item.status.as_str())
    .bind(&item.created_at)
    .bind(&item.updated_at)
    .execute(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(item.id.clone())
}

pub(crate) async fn delete_library_scan_unmatched_item_query(
    pool: &SqlitePool,
    facet: MediaFacet,
    item_path: &str,
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM library_scan_unmatched_items
         WHERE facet = ? AND item_path = ?",
    )
    .bind(facet.as_str())
    .bind(item_path)
    .execute(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(())
}

pub(crate) async fn get_library_scan_unmatched_item_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<LibraryScanUnmatchedItem>> {
    let row = sqlx::query(
        "SELECT id, facet, title_id, scan_session_id, scan_root, item_path, display_name, query,
                year_hint, reason_code, error_message, search_attempts_json, status, created_at, updated_at
         FROM library_scan_unmatched_items
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.as_ref()
        .map(row_to_library_scan_unmatched_item)
        .transpose()
}

pub(crate) async fn list_library_scan_unmatched_items_query(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
    scan_root: Option<&str>,
    status: Option<PendingImportStatus>,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
    let mut sql = String::from(
        "SELECT id, facet, title_id, scan_session_id, scan_root, item_path, display_name, query,
                year_hint, reason_code, error_message, search_attempts_json, status, created_at, updated_at
         FROM library_scan_unmatched_items
         WHERE 1=1",
    );
    let mut binds = Vec::new();

    if let Some(facet) = facet {
        sql.push_str(" AND facet = ?");
        binds.push(facet.as_str().to_string());
    }

    if let Some(scan_root) = scan_root {
        sql.push_str(" AND scan_root = ?");
        binds.push(scan_root.to_string());
    }

    if let Some(status) = status {
        sql.push_str(" AND status = ?");
        binds.push(status.as_str().to_string());
    }

    sql.push_str(" ORDER BY updated_at DESC, item_path ASC LIMIT ? OFFSET ?");

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }
    query = query.bind(limit).bind(offset);

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    rows.iter()
        .map(row_to_library_scan_unmatched_item)
        .collect()
}

pub(crate) async fn count_library_scan_unmatched_items_query(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
    scan_root: Option<&str>,
    status: Option<PendingImportStatus>,
) -> AppResult<i64> {
    let mut sql = String::from(
        "SELECT COUNT(*) AS count
         FROM library_scan_unmatched_items
         WHERE 1=1",
    );
    let mut binds = Vec::new();

    if let Some(facet) = facet {
        sql.push_str(" AND facet = ?");
        binds.push(facet.as_str().to_string());
    }

    if let Some(scan_root) = scan_root {
        sql.push_str(" AND scan_root = ?");
        binds.push(scan_root.to_string());
    }

    if let Some(status) = status {
        sql.push_str(" AND status = ?");
        binds.push(status.as_str().to_string());
    }

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let row = query
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    row.try_get("count")
        .map_err(|err| AppError::Repository(err.to_string()))
}
