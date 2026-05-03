use chrono::Utc;
use scryer_application::{AppError, AppResult};
use scryer_domain::Id;
use sqlx::SqlitePool;

#[derive(sqlx::FromRow)]
pub(crate) struct BlocklistRow {
    pub id: String,
    pub title_id: String,
    pub source_title: Option<String>,
    pub source_hint: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub reason: Option<String>,
    pub data_json: Option<String>,
    pub created_at: String,
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn insert_blocklist_entry_query(
    pool: &SqlitePool,
    title_id: &str,
    source_title: Option<&str>,
    source_hint: Option<&str>,
    quality: Option<&str>,
    download_id: Option<&str>,
    reason: Option<&str>,
    data_json: Option<&str>,
) -> AppResult<String> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO blocklist
         (id, title_id, source_title, source_hint, quality, download_id, reason, data_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(title_id)
    .bind(source_title)
    .bind(source_hint)
    .bind(quality)
    .bind(download_id)
    .bind(reason)
    .bind(data_json)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(id)
}

pub(crate) async fn list_blocklist_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
    limit: usize,
) -> AppResult<Vec<BlocklistRow>> {
    let rows = sqlx::query_as::<_, BlocklistRow>(
        "SELECT
             b.id,
             b.title_id,
             COALESCE(
                 (
                     SELECT rda.source_title
                     FROM release_download_attempts rda
                     WHERE rda.title_id = b.title_id
                       AND b.source_hint IS NOT NULL
                       AND rda.source_hint = b.source_hint
                       AND rda.source_title IS NOT NULL
                       AND TRIM(rda.source_title) <> ''
                     ORDER BY LENGTH(rda.source_title) DESC, rda.attempted_at DESC
                     LIMIT 1
                 ),
                 b.source_title
             ) AS source_title,
             b.source_hint,
             b.quality,
             b.download_id,
             b.reason,
             b.data_json,
             b.created_at
         FROM blocklist b
         WHERE b.title_id = ?
         ORDER BY b.created_at DESC
         LIMIT ?",
    )
    .bind(title_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(rows)
}

pub(crate) async fn list_blocklist_all_query(
    pool: &SqlitePool,
    limit: usize,
    offset: usize,
) -> AppResult<(Vec<BlocklistRow>, i64)> {
    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blocklist")
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let rows = sqlx::query_as::<_, BlocklistRow>(
        "SELECT
             b.id,
             b.title_id,
             COALESCE(
                 (
                     SELECT rda.source_title
                     FROM release_download_attempts rda
                     WHERE rda.title_id = b.title_id
                       AND b.source_hint IS NOT NULL
                       AND rda.source_hint = b.source_hint
                       AND rda.source_title IS NOT NULL
                       AND TRIM(rda.source_title) <> ''
                     ORDER BY LENGTH(rda.source_title) DESC, rda.attempted_at DESC
                     LIMIT 1
                 ),
                 b.source_title
             ) AS source_title,
             b.source_hint,
             b.quality,
             b.download_id,
             b.reason,
             b.data_json,
             b.created_at
         FROM blocklist b
         ORDER BY b.created_at DESC
         LIMIT ? OFFSET ?",
    )
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok((rows, total))
}

pub(crate) async fn delete_blocklist_entry_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM blocklist WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn is_blocklisted_query(
    pool: &SqlitePool,
    title_id: &str,
    source_title: &str,
) -> AppResult<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM blocklist
             WHERE title_id = ? AND LOWER(source_title) = LOWER(?)
         )",
    )
    .bind(title_id)
    .bind(source_title)
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(exists)
}

pub(crate) async fn has_recorded_download_failure_query(
    pool: &SqlitePool,
    title_id: &str,
    download_id: Option<&str>,
    source_title: Option<&str>,
    source_hint: Option<&str>,
) -> AppResult<bool> {
    if let Some(download_id) = download_id.map(str::trim).filter(|value| !value.is_empty()) {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                 SELECT 1 FROM blocklist
                 WHERE title_id = ? AND download_id = ?
             )",
        )
        .bind(title_id)
        .bind(download_id)
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

        return Ok(exists);
    }

    let Some(source_title) = source_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(false);
    };
    let Some(source_hint) = source_hint.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(false);
    };

    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM blocklist
             WHERE title_id = ?
               AND LOWER(TRIM(COALESCE(source_title, ''))) = ?
               AND TRIM(COALESCE(source_hint, '')) = ?
         )",
    )
    .bind(title_id)
    .bind(source_title)
    .bind(source_hint)
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(exists)
}

pub(crate) async fn delete_blocklist_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM blocklist WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}
