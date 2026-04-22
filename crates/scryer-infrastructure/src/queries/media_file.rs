use chrono::Utc;
use scryer_application::{
    AppError, AppResult, InsertMediaFileInput, MediaFileAnalysis, TitleEpisodeProgressSummary,
    TitleMediaFile, TitleMediaSizeSummary, TitleQualitySummary,
};
use scryer_domain::Id;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use super::common::repository_error_from_sqlx;

const RECYCLE_BIN_PATH_SEGMENT: &str = "/.scryer-recycle/";

fn live_media_file_predicate(alias: &str) -> String {
    format!("instr({alias}.file_path, '{RECYCLE_BIN_PATH_SEGMENT}') = 0")
}

fn normalized_quality_expression(alias: &str) -> String {
    format!(
        "CASE
            WHEN trim(COALESCE({alias}.quality_id, '')) = '' THEN NULL
            ELSE upper(trim({alias}.quality_id))
         END"
    )
}

fn quality_rank_expression(alias: &str) -> String {
    format!(
        "CASE upper(trim(COALESCE({alias}.quality_id, '')))
            WHEN '4320P' THEN 0
            WHEN '2160P' THEN 1
            WHEN '1440P' THEN 2
            WHEN '1080P' THEN 3
            WHEN '1080I' THEN 4
            WHEN '720P' THEN 5
            WHEN '480P' THEN 6
            WHEN '360P' THEN 7
            ELSE 999
         END"
    )
}

fn serialized_media_analysis(analysis: &MediaFileAnalysis) -> String {
    serde_json::to_string(analysis).unwrap_or_else(|_| "{}".to_string())
}

pub(crate) async fn insert_media_file_query(
    pool: &SqlitePool,
    input: &InsertMediaFileInput,
) -> AppResult<String> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO media_files
         (id, title_id, file_path, size_bytes, quality_id, scan_status, created_at,
          source_signature_scheme, source_signature_value,
          scene_name, release_group, source_type, resolution,
          video_codec_parsed, audio_codec_parsed, audio_channels_parsed,
          acquisition_score, scoring_log,
          indexer_source, grabbed_release_title, grabbed_at,
          edition, original_file_path, release_hash)
         VALUES (?, ?, ?, ?, ?, 'imported', ?,
                 ?, ?,
                 ?, ?, ?, ?,
                 ?, ?, ?,
                 ?, ?,
                 ?, ?, ?,
                 ?, ?, ?)
         ON CONFLICT(file_path) DO UPDATE SET
            title_id = excluded.title_id,
            size_bytes = excluded.size_bytes,
            quality_id = excluded.quality_id,
            scan_status = excluded.scan_status,
            source_signature_scheme = excluded.source_signature_scheme,
            source_signature_value = excluded.source_signature_value,
            scene_name = excluded.scene_name,
            release_group = excluded.release_group,
            source_type = excluded.source_type,
            resolution = excluded.resolution,
            video_codec_parsed = excluded.video_codec_parsed,
            audio_codec_parsed = excluded.audio_codec_parsed,
            audio_channels_parsed = excluded.audio_channels_parsed,
            acquisition_score = excluded.acquisition_score,
            scoring_log = excluded.scoring_log,
            indexer_source = excluded.indexer_source,
            grabbed_release_title = excluded.grabbed_release_title,
            grabbed_at = excluded.grabbed_at,
            edition = excluded.edition,
            original_file_path = excluded.original_file_path,
            release_hash = excluded.release_hash",
    )
    .bind(&id)
    .bind(&input.title_id)
    .bind(&input.file_path)
    .bind(input.size_bytes)
    .bind(&input.quality_label)
    .bind(&now)
    .bind(&input.source_signature_scheme)
    .bind(&input.source_signature_value)
    .bind(&input.scene_name)
    .bind(&input.release_group)
    .bind(&input.source_type)
    .bind(&input.resolution)
    .bind(&input.video_codec_parsed)
    .bind(&input.audio_codec_parsed)
    .bind(&input.audio_channels_parsed)
    .bind(input.acquisition_score)
    .bind(&input.scoring_log)
    .bind(&input.indexer_source)
    .bind(&input.grabbed_release_title)
    .bind(&input.grabbed_at)
    .bind(&input.edition)
    .bind(&input.original_file_path)
    .bind(&input.release_hash)
    .execute(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(id)
}

pub(crate) async fn link_file_to_episode_query(
    pool: &SqlitePool,
    file_id: &str,
    episode_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO file_episode_map (file_id, episode_id)
         VALUES (?, ?)
         ON CONFLICT(file_id, episode_id) DO NOTHING",
    )
    .bind(file_id)
    .bind(episode_id)
    .execute(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(())
}

pub(crate) async fn list_media_files_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<TitleMediaFile>> {
    let sql = format!(
        "SELECT mf.id, mf.title_id, fem.episode_id, mf.file_path,
                mf.size_bytes, mf.source_signature_scheme, mf.source_signature_value,
                mf.quality_id, mf.scan_status, mf.created_at,
                mf.video_codec, mf.video_width, mf.video_height,
                mf.video_bitrate_kbps, mf.video_bit_depth,
                mf.video_hdr_format, mf.video_frame_rate, mf.video_profile,
                mf.audio_codec, mf.audio_profile, mf.audio_channels, mf.audio_bitrate_kbps,
                mf.duration_seconds, mf.num_chapters, mf.container_format,
                COALESCE(json_extract(mf.analysis_json, '$.audio_languages'), '[]') AS audio_languages_json,
                COALESCE(json_extract(mf.analysis_json, '$.audio_streams'), '[]') AS audio_streams_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_languages'), '[]') AS subtitle_languages_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_codecs'), '[]') AS subtitle_codecs_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_streams'), '[]') AS subtitle_streams_json,
                mf.has_multiaudio,
                mf.scene_name, mf.release_group, mf.source_type, mf.resolution,
                mf.video_codec_parsed, mf.audio_codec_parsed, mf.audio_channels_parsed,
                mf.acquisition_score, mf.scoring_log,
                mf.indexer_source, mf.grabbed_release_title, mf.grabbed_at,
                mf.edition, mf.original_file_path, mf.release_hash
         FROM media_files mf
         LEFT JOIN file_episode_map fem ON fem.file_id = mf.id
         WHERE mf.title_id = ?
           AND {}
         ORDER BY mf.created_at DESC",
        live_media_file_predicate("mf")
    );
    let rows: Vec<SqliteRow> = sqlx::query(&sql)
        .bind(title_id)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_title_media_file(row)?);
    }
    Ok(out)
}

pub(crate) async fn list_title_media_size_summaries_query(
    pool: &SqlitePool,
    title_ids: &[String],
) -> AppResult<Vec<TitleMediaSizeSummary>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = title_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT title_id, COALESCE(SUM(CASE WHEN size_bytes > 0 THEN size_bytes ELSE 0 END), 0) AS total_size_bytes
         FROM media_files
         WHERE title_id IN ({placeholders})
           AND {}
         GROUP BY title_id"
        ,
        live_media_file_predicate("media_files")
    );

    let mut query = sqlx::query(&sql);
    for title_id in title_ids {
        query = query.bind(title_id);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(TitleMediaSizeSummary {
            title_id: row
                .try_get("title_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            total_size_bytes: row
                .try_get("total_size_bytes")
                .map_err(|err| AppError::Repository(err.to_string()))?,
        });
    }

    Ok(out)
}

pub(crate) async fn list_title_quality_summaries_query(
    pool: &SqlitePool,
    title_ids: &[String],
) -> AppResult<Vec<TitleQualitySummary>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = title_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let normalized_quality = normalized_quality_expression("media_files");
    let quality_rank = quality_rank_expression("media_files");
    let sql = format!(
        "SELECT title_id, quality_tier
         FROM (
            SELECT media_files.title_id AS title_id,
                   {normalized_quality} AS quality_tier,
                   ROW_NUMBER() OVER (
                      PARTITION BY media_files.title_id
                      ORDER BY {quality_rank} DESC,
                               media_files.created_at DESC,
                               media_files.id DESC
                   ) AS quality_row
              FROM media_files
             WHERE media_files.title_id IN ({placeholders})
               AND {}
               AND trim(COALESCE(media_files.quality_id, '')) <> ''
         ) ranked
         WHERE quality_row = 1
           AND quality_tier IS NOT NULL",
        live_media_file_predicate("media_files"),
    );

    let mut query = sqlx::query(&sql);
    for title_id in title_ids {
        query = query.bind(title_id);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(TitleQualitySummary {
            title_id: row
                .try_get("title_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            quality_tier: row
                .try_get("quality_tier")
                .map_err(|err| AppError::Repository(err.to_string()))?,
        });
    }

    Ok(out)
}

pub(crate) async fn list_title_episode_progress_summaries_query(
    pool: &SqlitePool,
    title_ids: &[String],
) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = title_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT e.title_id,
                COUNT(DISTINCT e.id) AS total_episodes,
                COUNT(DISTINCT CASE WHEN e.monitored = 1 THEN e.id END) AS monitored_episodes,
                COUNT(DISTINCT CASE WHEN mf.id IS NOT NULL THEN e.id END) AS owned_episodes
         FROM episodes e
         INNER JOIN collections c ON c.id = e.collection_id
         LEFT JOIN file_episode_map fem ON fem.episode_id = e.id
         LEFT JOIN media_files mf ON mf.id = fem.file_id AND {}
         WHERE e.title_id IN ({placeholders})
           AND c.collection_type <> 'specials'
           AND c.collection_index <> '0'
         GROUP BY e.title_id",
        live_media_file_predicate("mf")
    );

    let mut query = sqlx::query(&sql);
    for title_id in title_ids {
        query = query.bind(title_id);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(TitleEpisodeProgressSummary {
            title_id: row
                .try_get("title_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            owned_episodes: row
                .try_get("owned_episodes")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            monitored_episodes: row
                .try_get("monitored_episodes")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            total_episodes: row
                .try_get("total_episodes")
                .map_err(|err| AppError::Repository(err.to_string()))?,
        });
    }

    Ok(out)
}

fn row_to_title_media_file(row: &SqliteRow) -> AppResult<TitleMediaFile> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title_id: String = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_id: Option<String> = row.try_get("episode_id").unwrap_or(None);
    let file_path: String = row
        .try_get("file_path")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let size_bytes: i64 = row
        .try_get("size_bytes")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let source_signature_scheme: Option<String> =
        row.try_get("source_signature_scheme").unwrap_or(None);
    let source_signature_value: Option<String> =
        row.try_get("source_signature_value").unwrap_or(None);
    let quality_label: Option<String> = row.try_get("quality_id").unwrap_or(None);
    let scan_status: String = row
        .try_get("scan_status")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let video_codec: Option<String> = row.try_get("video_codec").unwrap_or(None);
    let video_width: Option<i32> = row.try_get("video_width").unwrap_or(None);
    let video_height: Option<i32> = row.try_get("video_height").unwrap_or(None);
    let video_bitrate_kbps: Option<i32> = row.try_get("video_bitrate_kbps").unwrap_or(None);
    let video_bit_depth: Option<i32> = row.try_get("video_bit_depth").unwrap_or(None);
    let video_hdr_format: Option<String> = row.try_get("video_hdr_format").unwrap_or(None);
    let video_frame_rate: Option<String> = row.try_get("video_frame_rate").unwrap_or(None);
    let video_profile: Option<String> = row.try_get("video_profile").unwrap_or(None);
    let audio_codec: Option<String> = row.try_get("audio_codec").unwrap_or(None);
    let audio_profile: Option<String> = row.try_get("audio_profile").unwrap_or(None);
    let audio_channels: Option<i32> = row.try_get("audio_channels").unwrap_or(None);
    let audio_bitrate_kbps: Option<i32> = row.try_get("audio_bitrate_kbps").unwrap_or(None);
    let duration_seconds: Option<i32> = row.try_get("duration_seconds").unwrap_or(None);
    let num_chapters: Option<i32> = row.try_get("num_chapters").unwrap_or(None);
    let container_format: Option<String> = row.try_get("container_format").unwrap_or(None);
    let has_multiaudio: i64 = row.try_get("has_multiaudio").unwrap_or(0i64);

    let audio_languages: Vec<String> = row
        .try_get::<Option<String>, _>("audio_languages_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let mut audio_streams: Vec<scryer_application::AudioStreamDetail> = row
        .try_get::<Option<String>, _>("audio_streams_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let subtitle_languages: Vec<String> = row
        .try_get::<Option<String>, _>("subtitle_languages_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let subtitle_codecs: Vec<String> = row
        .try_get::<Option<String>, _>("subtitle_codecs_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let mut subtitle_streams: Vec<scryer_application::SubtitleStreamDetail> = row
        .try_get::<Option<String>, _>("subtitle_streams_json")
        .unwrap_or(None)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    let audio_languages = scryer_application::normalize_detected_audio_languages(
        audio_languages.iter().map(String::as_str),
    );
    for stream in &mut audio_streams {
        stream.language = stream
            .language
            .as_deref()
            .and_then(scryer_application::normalize_detected_audio_language_code);
    }
    let subtitle_languages = scryer_application::normalize_detected_subtitle_languages(
        subtitle_languages.iter().map(String::as_str),
    );
    for stream in &mut subtitle_streams {
        stream.language = stream
            .language
            .as_deref()
            .and_then(scryer_application::normalize_detected_subtitle_language_code);
    }

    // Rich schema fields (added by migration 0037)
    let scene_name: Option<String> = row.try_get("scene_name").unwrap_or(None);
    let release_group: Option<String> = row.try_get("release_group").unwrap_or(None);
    let source_type: Option<String> = row.try_get("source_type").unwrap_or(None);
    let resolution: Option<String> = row.try_get("resolution").unwrap_or(None);
    let video_codec_parsed: Option<String> = row.try_get("video_codec_parsed").unwrap_or(None);
    let audio_codec_parsed: Option<String> = row.try_get("audio_codec_parsed").unwrap_or(None);
    let audio_channels_parsed: Option<String> =
        row.try_get("audio_channels_parsed").unwrap_or(None);
    let acquisition_score: Option<i32> = row.try_get("acquisition_score").unwrap_or(None);
    let scoring_log: Option<String> = row.try_get("scoring_log").unwrap_or(None);
    let indexer_source: Option<String> = row.try_get("indexer_source").unwrap_or(None);
    let grabbed_release_title: Option<String> =
        row.try_get("grabbed_release_title").unwrap_or(None);
    let grabbed_at: Option<String> = row.try_get("grabbed_at").unwrap_or(None);
    let edition: Option<String> = row.try_get("edition").unwrap_or(None);
    let original_file_path: Option<String> = row.try_get("original_file_path").unwrap_or(None);
    let release_hash: Option<String> = row.try_get("release_hash").unwrap_or(None);

    Ok(TitleMediaFile {
        id,
        title_id,
        episode_id,
        file_path,
        size_bytes,
        source_signature_scheme,
        source_signature_value,
        quality_label,
        scan_status,
        created_at,
        video_codec,
        video_width,
        video_height,
        video_bitrate_kbps,
        video_bit_depth,
        video_hdr_format,
        video_frame_rate,
        video_profile,
        audio_codec,
        audio_profile,
        audio_channels,
        audio_bitrate_kbps,
        audio_languages,
        audio_streams,
        subtitle_languages,
        subtitle_codecs,
        subtitle_streams,
        has_multiaudio: has_multiaudio != 0,
        duration_seconds,
        num_chapters,
        container_format,
        scene_name,
        release_group,
        source_type,
        resolution,
        video_codec_parsed,
        audio_codec_parsed,
        audio_channels_parsed,
        acquisition_score,
        scoring_log,
        indexer_source,
        grabbed_release_title,
        grabbed_at,
        edition,
        original_file_path,
        release_hash,
    })
}

pub(crate) async fn update_media_file_analysis_query(
    pool: &SqlitePool,
    file_id: &str,
    analysis: &MediaFileAnalysis,
) -> AppResult<()> {
    let analysis_json = serialized_media_analysis(analysis);

    sqlx::query(
        "UPDATE media_files SET
            video_codec = ?,
            video_width = ?,
            video_height = ?,
            video_bitrate_kbps = ?,
            video_bit_depth = ?,
            video_hdr_format = ?,
            video_frame_rate = ?,
            video_profile = ?,
            audio_codec = ?,
            audio_profile = ?,
            audio_channels = ?,
            audio_bitrate_kbps = ?,
            duration_seconds = ?,
            num_chapters = ?,
            container_format = ?,
            analysis_json = ?,
            has_multiaudio = ?,
            scan_status = 'scanned'
         WHERE id = ?",
    )
    .bind(&analysis.video_codec)
    .bind(analysis.video_width)
    .bind(analysis.video_height)
    .bind(analysis.video_bitrate_kbps)
    .bind(analysis.video_bit_depth)
    .bind(&analysis.video_hdr_format)
    .bind(&analysis.video_frame_rate)
    .bind(&analysis.video_profile)
    .bind(&analysis.audio_codec)
    .bind(&analysis.audio_profile)
    .bind(analysis.audio_channels)
    .bind(analysis.audio_bitrate_kbps)
    .bind(analysis.duration_seconds)
    .bind(analysis.num_chapters)
    .bind(&analysis.container_format)
    .bind(&analysis_json)
    .bind(if analysis.has_multiaudio { 1i64 } else { 0i64 })
    .bind(file_id)
    .execute(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(())
}

pub(crate) async fn update_media_file_source_signature_query(
    pool: &SqlitePool,
    file_id: &str,
    size_bytes: i64,
    source_signature_scheme: Option<&str>,
    source_signature_value: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE media_files SET
            size_bytes = ?,
            source_signature_scheme = ?,
            source_signature_value = ?
         WHERE id = ?",
    )
    .bind(size_bytes)
    .bind(source_signature_scheme)
    .bind(source_signature_value)
    .bind(file_id)
    .execute(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(())
}

pub(crate) async fn update_media_file_path_query(
    pool: &SqlitePool,
    file_id: &str,
    file_path: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE media_files SET file_path = ? WHERE id = ?")
        .bind(file_path)
        .bind(file_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn mark_scan_failed_query(
    pool: &SqlitePool,
    file_id: &str,
    error: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE media_files SET scan_status = 'scan_failed', scan_error = ? WHERE id = ?")
        .bind(error)
        .bind(file_id)
        .execute(pool)
        .await
        .map_err(repository_error_from_sqlx)?;

    Ok(())
}

pub(crate) async fn get_media_file_by_id_query(
    pool: &SqlitePool,
    file_id: &str,
) -> AppResult<Option<TitleMediaFile>> {
    let row: Option<SqliteRow> = sqlx::query(
        "SELECT mf.id, mf.title_id, NULL AS episode_id, mf.file_path,
                mf.size_bytes, mf.source_signature_scheme, mf.source_signature_value,
                mf.quality_id, mf.scan_status, mf.created_at,
                mf.video_codec, mf.video_width, mf.video_height,
                mf.video_bitrate_kbps, mf.video_bit_depth,
                mf.video_hdr_format, mf.video_frame_rate, mf.video_profile,
                mf.audio_codec, mf.audio_profile, mf.audio_channels, mf.audio_bitrate_kbps,
                mf.duration_seconds, mf.num_chapters, mf.container_format,
                COALESCE(json_extract(mf.analysis_json, '$.audio_languages'), '[]') AS audio_languages_json,
                COALESCE(json_extract(mf.analysis_json, '$.audio_streams'), '[]') AS audio_streams_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_languages'), '[]') AS subtitle_languages_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_codecs'), '[]') AS subtitle_codecs_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_streams'), '[]') AS subtitle_streams_json,
                mf.has_multiaudio,
                mf.scene_name, mf.release_group, mf.source_type, mf.resolution,
                mf.video_codec_parsed, mf.audio_codec_parsed, mf.audio_channels_parsed,
                mf.acquisition_score, mf.scoring_log,
                mf.indexer_source, mf.grabbed_release_title, mf.grabbed_at,
                mf.edition, mf.original_file_path, mf.release_hash
         FROM media_files mf
         WHERE mf.id = ?",
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(ref r) => Ok(Some(row_to_title_media_file(r)?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_media_file_by_path_query(
    pool: &SqlitePool,
    file_path: &str,
) -> AppResult<Option<TitleMediaFile>> {
    let row: Option<SqliteRow> = sqlx::query(
        "SELECT mf.id, mf.title_id, NULL AS episode_id, mf.file_path,
                mf.size_bytes, mf.source_signature_scheme, mf.source_signature_value,
                mf.quality_id, mf.scan_status, mf.created_at,
                mf.video_codec, mf.video_width, mf.video_height,
                mf.video_bitrate_kbps, mf.video_bit_depth,
                mf.video_hdr_format, mf.video_frame_rate, mf.video_profile,
                mf.audio_codec, mf.audio_profile, mf.audio_channels, mf.audio_bitrate_kbps,
                mf.duration_seconds, mf.num_chapters, mf.container_format,
                COALESCE(json_extract(mf.analysis_json, '$.audio_languages'), '[]') AS audio_languages_json,
                COALESCE(json_extract(mf.analysis_json, '$.audio_streams'), '[]') AS audio_streams_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_languages'), '[]') AS subtitle_languages_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_codecs'), '[]') AS subtitle_codecs_json,
                COALESCE(json_extract(mf.analysis_json, '$.subtitle_streams'), '[]') AS subtitle_streams_json,
                mf.has_multiaudio,
                mf.scene_name, mf.release_group, mf.source_type, mf.resolution,
                mf.video_codec_parsed, mf.audio_codec_parsed, mf.audio_channels_parsed,
                mf.acquisition_score, mf.scoring_log,
                mf.indexer_source, mf.grabbed_release_title, mf.grabbed_at,
                mf.edition, mf.original_file_path, mf.release_hash
         FROM media_files mf
         WHERE mf.file_path = ?
         LIMIT 1",
    )
    .bind(file_path)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(ref r) => Ok(Some(row_to_title_media_file(r)?)),
        None => Ok(None),
    }
}

pub(crate) async fn delete_media_file_query(pool: &SqlitePool, file_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM media_files WHERE id = ?")
        .bind(file_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SqliteCatalogStore, SqliteLibraryStateStore, SqliteServices};
    use chrono::Utc;
    use scryer_application::{
        AudioStreamDetail, MediaFileAnalysis, MediaFileRepository, ShowRepository, TitleRepository,
    };
    use scryer_domain::{Collection, CollectionType, Episode, MediaFacet, Title};

    fn make_test_series_title(id: &str) -> Title {
        Title {
            id: id.to_string(),
            name: "Live Query Test".to_string(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2026),
            overview: Some("overview".to_string()),
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn catalog_store(services: &SqliteServices) -> SqliteCatalogStore {
        SqliteCatalogStore::new(services)
    }

    fn library_state_store(services: &SqliteServices) -> SqliteLibraryStateStore {
        SqliteLibraryStateStore::new(services)
    }

    #[tokio::test]
    async fn recycled_media_files_are_excluded_from_live_title_queries() {
        let db = std::env::temp_dir().join(format!(
            "scryer_media_file_live_query_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let catalog = catalog_store(&services);
        let library_state = library_state_store(&services);

        let title = make_test_series_title("title-live-query");
        catalog
            .create(title.clone())
            .await
            .expect("title should insert");

        let collection = Collection {
            id: "collection-live-query".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        };
        catalog
            .create_collection(collection.clone())
            .await
            .expect("collection should insert");

        let episode_one = Episode {
            id: "episode-live-query-1".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode 1".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let episode_two = Episode {
            id: "episode-live-query-2".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode 2".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        };
        catalog
            .create_episode(episode_one.clone())
            .await
            .expect("episode one should insert");
        catalog
            .create_episode(episode_two.clone())
            .await
            .expect("episode two should insert");

        let live_file_id = library_state
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                ..Default::default()
            })
            .await
            .expect("live media file should insert");
        library_state
            .link_file_to_episode(&live_file_id, &episode_one.id)
            .await
            .expect("live file should link");

        let recycled_file_id = library_state
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path:
                    "/library/Show/.scryer-recycle/20260404_000000_deadbeef/Show - S01E02.mkv"
                        .to_string(),
                size_bytes: 9_999,
                ..Default::default()
            })
            .await
            .expect("recycled media file should insert");
        library_state
            .link_file_to_episode(&recycled_file_id, &episode_two.id)
            .await
            .expect("recycled file should link");

        let live_files = library_state
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files should succeed");
        assert_eq!(live_files.len(), 1);
        assert_eq!(live_files[0].id, live_file_id);
        assert_eq!(
            live_files[0].file_path,
            "/library/Show/Season 01/Show - S01E01.mkv"
        );

        let size_summaries = library_state
            .list_title_media_size_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("size summaries should succeed");
        assert_eq!(size_summaries.len(), 1);
        assert_eq!(size_summaries[0].title_id, title.id);
        assert_eq!(size_summaries[0].total_size_bytes, 1_000);

        let episode_progress = library_state
            .list_title_episode_progress_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("episode progress summaries should succeed");
        assert_eq!(episode_progress.len(), 1);
        assert_eq!(episode_progress[0].title_id, title.id);
        assert_eq!(episode_progress[0].total_episodes, 2);
        assert_eq!(episode_progress[0].monitored_episodes, 2);
        assert_eq!(episode_progress[0].owned_episodes, 1);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn title_quality_summaries_use_lowest_live_quality_and_ignore_recycled_files() {
        let db = std::env::temp_dir().join(format!(
            "scryer_title_quality_summary_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let catalog = catalog_store(&services);
        let library_state = library_state_store(&services);

        let title = make_test_series_title("title-quality-summary");
        catalog
            .create(title.clone())
            .await
            .expect("title should insert");

        library_state
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("2160p".to_string()),
                ..Default::default()
            })
            .await
            .expect("high quality file should insert");

        library_state
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E02.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("720p".to_string()),
                ..Default::default()
            })
            .await
            .expect("lower quality file should insert");

        library_state
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path:
                    "/library/Show/.scryer-recycle/20260404_000000_deadbeef/Show - S01E03.mkv"
                        .to_string(),
                size_bytes: 1_000,
                quality_label: Some("360p".to_string()),
                ..Default::default()
            })
            .await
            .expect("recycled file should insert");

        let quality_summaries = library_state
            .list_title_quality_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("quality summaries should succeed");
        assert_eq!(quality_summaries.len(), 1);
        assert_eq!(quality_summaries[0].title_id, title.id);
        assert_eq!(quality_summaries[0].quality_tier, "720P");

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn media_file_roundtrip_persists_audio_profile_and_parsed_channel_backup() {
        let db = std::env::temp_dir().join(format!(
            "scryer_media_file_audio_profile_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let catalog = catalog_store(&services);
        let library_state = library_state_store(&services);

        let title = make_test_series_title("title-audio-profile");
        catalog
            .create(title.clone())
            .await
            .expect("title should insert");

        let file_id = library_state
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                audio_channels_parsed: Some("7.1".to_string()),
                ..Default::default()
            })
            .await
            .expect("media file should insert");

        library_state
            .update_media_file_analysis(
                &file_id,
                MediaFileAnalysis {
                    video_codec: Some("hevc".to_string()),
                    video_width: Some(3840),
                    video_height: Some(2160),
                    video_bitrate_kbps: None,
                    video_bit_depth: Some(10),
                    video_hdr_format: Some("HDR10".to_string()),
                    video_frame_rate: Some("23.976".to_string()),
                    video_profile: Some("Main 10".to_string()),
                    audio_codec: Some("dts".to_string()),
                    audio_profile: Some("DTS-HD MA + DTS:X IMAX".to_string()),
                    audio_channels: Some(8),
                    audio_bitrate_kbps: Some(4_000),
                    audio_languages: vec!["eng".to_string()],
                    audio_streams: vec![AudioStreamDetail {
                        codec: Some("dts".to_string()),
                        profile: Some("DTS-HD MA + DTS:X IMAX".to_string()),
                        channels: Some(8),
                        language: Some("eng".to_string()),
                        bitrate_kbps: Some(4_000),
                    }],
                    subtitle_languages: vec![],
                    subtitle_codecs: vec![],
                    subtitle_streams: vec![],
                    has_multiaudio: false,
                    duration_seconds: Some(1800),
                    num_chapters: Some(4),
                    container_format: Some("matroska".to_string()),
                },
            )
            .await
            .expect("analysis should update");

        let files = library_state
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files should succeed");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].audio_profile.as_deref(),
            Some("DTS-HD MA + DTS:X IMAX")
        );
        assert_eq!(files[0].audio_channels_parsed.as_deref(), Some("7.1"));
        assert_eq!(
            files[0].audio_streams[0].profile.as_deref(),
            Some("DTS-HD MA + DTS:X IMAX")
        );

        let _ = std::fs::remove_file(db);
    }
}
