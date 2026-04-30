use scryer_application::{
    AppError, AppResult,
    subtitles::{ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry},
};
use scryer_domain::{ExternalSubtitleSourceKind, SubtitleBlocklistEntry, SubtitleDownload};
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Subtitle downloads ──────────────────────────────────────────────────────

pub async fn list_subtitle_downloads_for_title(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<SubtitleDownload>> {
    let rows = sqlx::query(
        "SELECT id, media_file_id, title_id, episode_id, source_kind, language, provider,
                provider_file_id, file_path, score, hearing_impaired, forced,
                ai_translated, machine_translated, uploader, release_info,
                synced, downloaded_at
         FROM subtitle_downloads
         WHERE title_id = ?
         ORDER BY downloaded_at DESC",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_subtitle_download(&row)?);
    }
    Ok(out)
}

pub async fn list_subtitle_downloads_for_media_file(
    pool: &SqlitePool,
    media_file_id: &str,
) -> AppResult<Vec<SubtitleDownload>> {
    let rows = sqlx::query(
        "SELECT id, media_file_id, title_id, episode_id, source_kind, language, provider,
                provider_file_id, file_path, score, hearing_impaired, forced,
                ai_translated, machine_translated, uploader, release_info,
                synced, downloaded_at
         FROM subtitle_downloads
         WHERE media_file_id = ?
         ORDER BY downloaded_at DESC",
    )
    .bind(media_file_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_subtitle_download(&row)?);
    }
    Ok(out)
}

pub async fn list_external_subtitle_probe_cache_for_media_file(
    pool: &SqlitePool,
    media_file_id: &str,
) -> AppResult<Vec<ExternalSubtitleProbeCacheEntry>> {
    let rows = sqlx::query(
        "SELECT media_file_id, file_path, size_bytes, modified_at, language,
                hearing_impaired, detection_source_language, detection_source_hi,
                probe_version, updated_at
         FROM external_subtitle_probe_cache
         WHERE media_file_id = ?
         ORDER BY file_path ASC",
    )
    .bind(media_file_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_external_subtitle_probe_cache_entry(&row)?);
    }
    Ok(out)
}

pub async fn get_subtitle_download(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<SubtitleDownload>> {
    let row = sqlx::query(
        "SELECT id, media_file_id, title_id, episode_id, source_kind, language, provider,
                provider_file_id, file_path, score, hearing_impaired, forced,
                ai_translated, machine_translated, uploader, release_info,
                synced, downloaded_at
         FROM subtitle_downloads
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    row.map(|row| row_to_subtitle_download(&row)).transpose()
}

pub async fn insert_subtitle_download(
    pool: &SqlitePool,
    download: &SubtitleDownload,
) -> AppResult<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO subtitle_downloads
         (id, media_file_id, title_id, episode_id, source_kind, language, provider,
          provider_file_id, file_path, score, hearing_impaired, forced,
          ai_translated, machine_translated, uploader, release_info,
          synced, downloaded_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&download.id)
    .bind(&download.media_file_id)
    .bind(&download.title_id)
    .bind(&download.episode_id)
    .bind(download.source_kind.as_str())
    .bind(&download.language)
    .bind(download.provider.as_deref().unwrap_or(""))
    .bind(&download.provider_file_id)
    .bind(&download.file_path)
    .bind(download.score)
    .bind(download.hearing_impaired as i32)
    .bind(download.forced as i32)
    .bind(download.ai_translated as i32)
    .bind(download.machine_translated as i32)
    .bind(&download.uploader)
    .bind(&download.release_info)
    .bind(download.synced as i32)
    .bind(&download.downloaded_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

pub async fn upsert_external_subtitle_probe_cache_entry(
    pool: &SqlitePool,
    entry: &ExternalSubtitleProbeCacheEntry,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO external_subtitle_probe_cache
         (media_file_id, file_path, size_bytes, modified_at, language,
          hearing_impaired, detection_source_language, detection_source_hi,
          probe_version, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(media_file_id, file_path) DO UPDATE SET
            size_bytes = excluded.size_bytes,
            modified_at = excluded.modified_at,
            language = excluded.language,
            hearing_impaired = excluded.hearing_impaired,
            detection_source_language = excluded.detection_source_language,
            detection_source_hi = excluded.detection_source_hi,
            probe_version = excluded.probe_version,
            updated_at = excluded.updated_at",
    )
    .bind(&entry.media_file_id)
    .bind(&entry.file_path)
    .bind(entry.size_bytes)
    .bind(&entry.modified_at)
    .bind(&entry.language)
    .bind(entry.hearing_impaired.map(|value| value as i32))
    .bind(entry.detection_source_language.as_str())
    .bind(entry.detection_source_hi.as_str())
    .bind(entry.probe_version)
    .bind(&entry.updated_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

pub async fn update_subtitle_download_synced(
    pool: &SqlitePool,
    id: &str,
    synced: bool,
) -> AppResult<()> {
    sqlx::query("UPDATE subtitle_downloads SET synced = ? WHERE id = ?")
        .bind(synced as i32)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

pub async fn delete_subtitle_download(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<SubtitleDownload>> {
    let row = sqlx::query(
        "SELECT id, media_file_id, title_id, episode_id, source_kind, language, provider,
                provider_file_id, file_path, score, hearing_impaired, forced,
                ai_translated, machine_translated, uploader, release_info,
                synced, downloaded_at
         FROM subtitle_downloads WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    let download = match row {
        Some(r) => row_to_subtitle_download(&r)?,
        None => return Ok(None),
    };

    sqlx::query("DELETE FROM subtitle_downloads WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(Some(download))
}

pub async fn delete_external_subtitle_probe_cache_entry(
    pool: &SqlitePool,
    media_file_id: &str,
    file_path: &str,
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM external_subtitle_probe_cache
         WHERE media_file_id = ? AND file_path = ?",
    )
    .bind(media_file_id)
    .bind(file_path)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

fn row_to_subtitle_download(row: &sqlx::sqlite::SqliteRow) -> AppResult<SubtitleDownload> {
    use sqlx::Row;
    Ok(SubtitleDownload {
        id: row
            .try_get("id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        media_file_id: row
            .try_get("media_file_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        title_id: row
            .try_get("title_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        episode_id: row.try_get("episode_id").unwrap_or(None),
        source_kind: ExternalSubtitleSourceKind::parse(
            &row.try_get::<String, _>("source_kind")
                .map_err(|e| AppError::Repository(e.to_string()))?,
        )
        .ok_or_else(|| AppError::Repository("invalid external subtitle source kind".into()))?,
        language: row
            .try_get("language")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        provider: row
            .try_get("provider")
            .map_err(|e| AppError::Repository(e.to_string()))
            .map(|value: String| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })?,
        provider_file_id: row.try_get("provider_file_id").unwrap_or(None),
        file_path: row
            .try_get("file_path")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        score: row.try_get("score").unwrap_or(None),
        hearing_impaired: row.try_get::<i32, _>("hearing_impaired").unwrap_or(0) != 0,
        forced: row.try_get::<i32, _>("forced").unwrap_or(0) != 0,
        ai_translated: row.try_get::<i32, _>("ai_translated").unwrap_or(0) != 0,
        machine_translated: row.try_get::<i32, _>("machine_translated").unwrap_or(0) != 0,
        uploader: row.try_get("uploader").unwrap_or(None),
        release_info: row.try_get("release_info").unwrap_or(None),
        synced: row.try_get::<i32, _>("synced").unwrap_or(0) != 0,
        downloaded_at: row
            .try_get("downloaded_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    })
}

fn row_to_external_subtitle_probe_cache_entry(
    row: &sqlx::sqlite::SqliteRow,
) -> AppResult<ExternalSubtitleProbeCacheEntry> {
    use sqlx::Row;

    let detection_source_language = ExternalSubtitleDetectionSource::parse(
        &row.try_get::<String, _>("detection_source_language")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    )
    .ok_or_else(|| {
        AppError::Repository("invalid subtitle probe language detection source".into())
    })?;
    let detection_source_hi = ExternalSubtitleDetectionSource::parse(
        &row.try_get::<String, _>("detection_source_hi")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    )
    .ok_or_else(|| AppError::Repository("invalid subtitle probe hi detection source".into()))?;

    Ok(ExternalSubtitleProbeCacheEntry {
        media_file_id: row
            .try_get("media_file_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        file_path: row
            .try_get("file_path")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        size_bytes: row
            .try_get("size_bytes")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        modified_at: row.try_get("modified_at").unwrap_or(None),
        language: row.try_get("language").unwrap_or(None),
        hearing_impaired: row
            .try_get::<Option<i32>, _>("hearing_impaired")
            .unwrap_or(None)
            .map(|value| value != 0),
        detection_source_language,
        detection_source_hi,
        probe_version: row
            .try_get::<i32, _>("probe_version")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    })
}

// ── Subtitle blocklist ──────────────────────────────────────────────────────

pub async fn is_blocklisted(
    pool: &SqlitePool,
    media_file_id: &str,
    provider: &str,
    provider_file_id: &str,
) -> AppResult<bool> {
    let count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subtitle_blocklist
         WHERE media_file_id = ? AND provider = ? AND provider_file_id = ?",
    )
    .bind(media_file_id)
    .bind(provider)
    .bind(provider_file_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(count > 0)
}

pub async fn insert_blocklist_entry(
    pool: &SqlitePool,
    media_file_id: &str,
    provider: &str,
    provider_file_id: &str,
    language: &str,
    reason: Option<&str>,
) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT OR IGNORE INTO subtitle_blocklist
         (id, media_file_id, provider, provider_file_id, language, reason)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(media_file_id)
    .bind(provider)
    .bind(provider_file_id)
    .bind(language)
    .bind(reason)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(id)
}

pub async fn list_blocklist_for_media_file(
    pool: &SqlitePool,
    media_file_id: &str,
) -> AppResult<Vec<SubtitleBlocklistEntry>> {
    let rows = sqlx::query(
        "SELECT id, media_file_id, provider, provider_file_id, language, reason, created_at
         FROM subtitle_blocklist
         WHERE media_file_id = ?
         ORDER BY created_at DESC",
    )
    .bind(media_file_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    use sqlx::Row;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(SubtitleBlocklistEntry {
            id: row
                .try_get("id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            media_file_id: row
                .try_get("media_file_id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            provider: row
                .try_get("provider")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            provider_file_id: row
                .try_get("provider_file_id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            language: row
                .try_get("language")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            reason: row.try_get("reason").unwrap_or(None),
            created_at: row
                .try_get("created_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
        });
    }
    Ok(out)
}
