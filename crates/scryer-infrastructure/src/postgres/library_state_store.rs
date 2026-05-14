use async_trait::async_trait;
use chrono::Utc;
use scryer_application::*;
use scryer_domain::*;
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder, Row};

use crate::library_state_store::LibraryStateStore;
use crate::postgres::PostgresServices;
use crate::postgres::timestamp::{parse_optional_rfc3339_timestamp, parse_rfc3339_timestamp};

pub type PostgresLibraryStateStore = LibraryStateStore<PostgresLibraryStateSql>;

#[derive(Clone)]
pub struct PostgresLibraryStateSql {
    pool: sqlx::PgPool,
}

impl PostgresLibraryStateStore {
    pub fn new(db: &PostgresServices) -> Self {
        Self::from_sql(PostgresLibraryStateSql::new(db.pool().clone()))
    }
}

impl PostgresLibraryStateSql {
    fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn bool_from_pg(row: &sqlx::postgres::PgRow, column: &str) -> bool {
    row.try_get::<Option<bool>, _>(column)
        .ok()
        .flatten()
        .unwrap_or(false)
}

fn text_from_pg(row: &sqlx::postgres::PgRow, column: &str) -> Option<String> {
    row.try_get::<Option<String>, _>(column).ok().flatten()
}

fn i32_from_pg(row: &sqlx::postgres::PgRow, column: &str) -> Option<i32> {
    row.try_get::<Option<i32>, _>(column)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<Option<i64>, _>(column)
                .ok()
                .flatten()
                .and_then(|value| i32::try_from(value).ok())
        })
}

fn i64_from_pg(row: &sqlx::postgres::PgRow, column: &str) -> Option<i64> {
    row.try_get::<Option<i64>, _>(column)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<Option<i32>, _>(column)
                .ok()
                .flatten()
                .map(i64::from)
        })
}

fn timestamp_from_pg(row: &sqlx::postgres::PgRow, column: &str) -> Option<String> {
    row.try_get::<Option<chrono::DateTime<Utc>>, _>(column)
        .ok()
        .flatten()
        .map(|value| value.to_rfc3339())
        .or_else(|| row.try_get::<Option<String>, _>(column).ok().flatten())
}

fn required_timestamp_from_pg(row: &sqlx::postgres::PgRow, column: &str) -> AppResult<String> {
    timestamp_from_pg(row, column)
        .ok_or_else(|| AppError::Repository(format!("missing timestamp column {column}")))
}

fn json_string_from_pg(row: &sqlx::postgres::PgRow, column: &str) -> Option<String> {
    row.try_get::<Option<Value>, _>(column)
        .ok()
        .flatten()
        .map(|value| value.to_string())
        .or_else(|| row.try_get::<Option<String>, _>(column).ok().flatten())
}

fn content_type_for_format(format: String) -> String {
    match format.trim().to_ascii_lowercase().as_str() {
        "avif" => "image/avif",
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        other => other,
    }
    .to_string()
}

fn row_to_release_decision_pg(row: &sqlx::postgres::PgRow) -> AppResult<ReleaseDecision> {
    Ok(ReleaseDecision {
        id: row.try_get("id").map_err(repo_err)?,
        wanted_item_id: row.try_get("wanted_item_id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        release_title: row.try_get("release_title").map_err(repo_err)?,
        release_url: text_from_pg(row, "release_url"),
        release_size_bytes: i64_from_pg(row, "release_size_bytes"),
        decision_code: row.try_get("decision_code").map_err(repo_err)?,
        candidate_score: i32_from_pg(row, "candidate_score").unwrap_or_default(),
        current_score: i32_from_pg(row, "current_score"),
        score_delta: i32_from_pg(row, "score_delta"),
        explanation_json: json_string_from_pg(row, "explanation_json"),
        created_at: required_timestamp_from_pg(row, "created_at")?,
    })
}

fn row_to_wanted_item_pg(row: &sqlx::postgres::PgRow) -> AppResult<WantedItem> {
    let latest_release_decision = match row.try_get::<Option<String>, _>("latest_decision_id") {
        Ok(Some(id)) => Some(ReleaseDecision {
            id,
            wanted_item_id: row
                .try_get("latest_decision_wanted_item_id")
                .map_err(repo_err)?,
            title_id: row.try_get("latest_decision_title_id").map_err(repo_err)?,
            release_title: row
                .try_get("latest_decision_release_title")
                .map_err(repo_err)?,
            release_url: text_from_pg(row, "latest_decision_release_url"),
            release_size_bytes: i64_from_pg(row, "latest_decision_release_size_bytes"),
            decision_code: row
                .try_get("latest_decision_decision_code")
                .map_err(repo_err)?,
            candidate_score: i32_from_pg(row, "latest_decision_candidate_score")
                .unwrap_or_default(),
            current_score: i32_from_pg(row, "latest_decision_current_score"),
            score_delta: i32_from_pg(row, "latest_decision_score_delta"),
            explanation_json: json_string_from_pg(row, "latest_decision_explanation_json"),
            created_at: required_timestamp_from_pg(row, "latest_decision_created_at")?,
        }),
        _ => None,
    };

    let status_raw: String = row.try_get("status").map_err(repo_err)?;
    Ok(WantedItem {
        id: row.try_get("id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        title_name: text_from_pg(row, "title_name"),
        title_slug: text_from_pg(row, "title_slug"),
        title_facet: text_from_pg(row, "title_facet"),
        library_id: text_from_pg(row, "library_id"),
        library_name: text_from_pg(row, "library_name"),
        library_slug: text_from_pg(row, "library_slug"),
        episode_id: text_from_pg(row, "episode_id"),
        collection_id: text_from_pg(row, "collection_id"),
        season_number: text_from_pg(row, "season_number"),
        episode_number: text_from_pg(row, "episode_number"),
        media_type: row.try_get("media_type").map_err(repo_err)?,
        search_phase: row.try_get("search_phase").map_err(repo_err)?,
        next_search_at: timestamp_from_pg(row, "next_search_at"),
        last_search_at: timestamp_from_pg(row, "last_search_at"),
        search_count: i64_from_pg(row, "search_count").unwrap_or_default(),
        baseline_date: timestamp_from_pg(row, "baseline_date"),
        status: WantedStatus::parse(&status_raw).unwrap_or_default(),
        grabbed_release: text_from_pg(row, "grabbed_release"),
        current_score: i32_from_pg(row, "current_score"),
        latest_release_decision,
        mismatch_recovery_eligible: bool_from_pg(row, "mismatch_recovery_eligible"),
        created_at: required_timestamp_from_pg(row, "created_at")?,
        updated_at: required_timestamp_from_pg(row, "updated_at")?,
    })
}

fn wanted_item_select_sql() -> &'static str {
    "SELECT w.id, w.title_id, t.name AS title_name, t.slug AS title_slug,
            t.facet AS title_facet, t.library_id AS library_id,
            libraries.name AS library_name, libraries.slug AS library_slug,
            w.episode_id, w.collection_id,
            e.season_number, e.episode_number, w.media_type, w.search_phase, w.next_search_at,
            w.last_search_at, w.search_count, w.baseline_date, w.status, w.grabbed_release,
            w.current_score,
            latest_decision.id AS latest_decision_id,
            latest_decision.wanted_item_id AS latest_decision_wanted_item_id,
            latest_decision.title_id AS latest_decision_title_id,
            latest_decision.release_title AS latest_decision_release_title,
            latest_decision.release_url AS latest_decision_release_url,
            latest_decision.release_size_bytes AS latest_decision_release_size_bytes,
            latest_decision.decision_code AS latest_decision_decision_code,
            latest_decision.candidate_score AS latest_decision_candidate_score,
            latest_decision.current_score AS latest_decision_current_score,
            latest_decision.score_delta AS latest_decision_score_delta,
            latest_decision.explanation_json AS latest_decision_explanation_json,
            latest_decision.created_at AS latest_decision_created_at,
            CASE
                WHEN w.status = 'wanted'
                 AND EXISTS (
                     SELECT 1 FROM release_decisions mismatch_any
                     WHERE mismatch_any.wanted_item_id = w.id
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM release_decisions mismatch_other
                     WHERE mismatch_other.wanted_item_id = w.id
                       AND mismatch_other.decision_code <> 'title_mismatch'
                 )
                THEN TRUE
                ELSE FALSE
            END AS mismatch_recovery_eligible,
            w.created_at, w.updated_at
       FROM wanted_items w
       LEFT JOIN titles t ON t.id = w.title_id
       LEFT JOIN libraries ON libraries.id = t.library_id
       LEFT JOIN episodes e ON e.id = w.episode_id
       LEFT JOIN LATERAL (
           SELECT *
           FROM release_decisions rd
           WHERE rd.wanted_item_id = w.id
           ORDER BY rd.created_at DESC
           LIMIT 1
       ) latest_decision ON TRUE"
}

fn row_to_pending_release_pg(row: &sqlx::postgres::PgRow) -> AppResult<PendingRelease> {
    let status_raw: String = row.try_get("status").map_err(repo_err)?;
    Ok(PendingRelease {
        id: row.try_get("id").map_err(repo_err)?,
        wanted_item_id: row.try_get("wanted_item_id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        release_title: row.try_get("release_title").map_err(repo_err)?,
        release_url: text_from_pg(row, "release_url"),
        source_kind: text_from_pg(row, "source_kind")
            .and_then(|value| DownloadSourceKind::parse(&value)),
        release_size_bytes: i64_from_pg(row, "release_size_bytes"),
        release_score: i32_from_pg(row, "release_score").unwrap_or_default(),
        scoring_log_json: json_string_from_pg(row, "scoring_log_json"),
        indexer_source: text_from_pg(row, "indexer_source"),
        release_guid: text_from_pg(row, "release_guid"),
        added_at: required_timestamp_from_pg(row, "added_at")?,
        delay_until: required_timestamp_from_pg(row, "delay_until")?,
        status: PendingReleaseStatus::parse(&status_raw).ok_or_else(|| {
            AppError::Repository(format!("invalid pending release status {status_raw}"))
        })?,
        grabbed_at: timestamp_from_pg(row, "grabbed_at"),
        source_password: text_from_pg(row, "source_password"),
        published_at: timestamp_from_pg(row, "published_at"),
        info_hash: text_from_pg(row, "info_hash"),
    })
}

fn pending_release_select_sql() -> &'static str {
    "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
            source_kind, release_score, scoring_log_json, indexer_source, release_guid,
            added_at, delay_until, status, grabbed_at,
            source_password, published_at, info_hash
       FROM pending_releases"
}

fn push_wanted_query_filters(builder: &mut QueryBuilder<'_, Postgres>, query: &WantedItemsQuery) {
    if !query.statuses.is_empty() {
        builder.push(" AND w.status = ANY(");
        builder.push_bind(query.statuses.clone());
        builder.push(")");
    }
    if !query.media_types.is_empty() {
        builder.push(" AND w.media_type = ANY(");
        builder.push_bind(query.media_types.clone());
        builder.push(")");
    }
    if let Some(title_id) = query.title_id.as_deref() {
        builder.push(" AND w.title_id = ");
        builder.push_bind(title_id.to_string());
    }
    if !query.library_ids.is_empty() {
        builder.push(" AND t.library_id = ANY(");
        builder.push_bind(query.library_ids.clone());
        builder.push(")");
    }
    if let Some(search) = query
        .title_search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let pattern = format!("%{}%", search.to_ascii_lowercase());
        builder.push(" AND LOWER(COALESCE(t.name, '')) LIKE ");
        builder.push_bind(pattern);
    }
    if !query.latest_decision_codes.is_empty() {
        builder.push(" AND latest_decision.decision_code = ANY(");
        builder.push_bind(query.latest_decision_codes.clone());
        builder.push(")");
    }
}

fn row_to_blocklist_entry_pg(row: &sqlx::postgres::PgRow) -> BlocklistEntry {
    BlocklistEntry {
        id: row.try_get("id").unwrap_or_default(),
        title_id: row.try_get("title_id").unwrap_or_default(),
        source_title: text_from_pg(row, "source_title"),
        source_hint: text_from_pg(row, "source_hint"),
        quality: text_from_pg(row, "quality"),
        download_id: text_from_pg(row, "download_id"),
        reason: text_from_pg(row, "reason"),
        data_json: json_string_from_pg(row, "data_json"),
        created_at: timestamp_from_pg(row, "created_at").unwrap_or_else(now_rfc3339),
    }
}

async fn delete_older_than(
    pool: &sqlx::PgPool,
    table: &'static str,
    timestamp_column: &'static str,
    days: i64,
) -> AppResult<u32> {
    let sql = format!(
        "DELETE FROM {table}
          WHERE {timestamp_column} < NOW() - ($1::TEXT || ' days')::interval"
    );
    let result = sqlx::query(&sql)
        .bind(days.to_string())
        .execute(pool)
        .await
        .map_err(repo_err)?;
    Ok(result.rows_affected() as u32)
}

async fn delete_for_title_ids(
    pool: &sqlx::PgPool,
    table: &'static str,
    title_ids: &[String],
) -> AppResult<u32> {
    if title_ids.is_empty() {
        return Ok(0);
    }
    let sql = format!("DELETE FROM {table} WHERE title_id = ANY($1)");
    let result = sqlx::query(&sql)
        .bind(title_ids)
        .execute(pool)
        .await
        .map_err(repo_err)?;
    Ok(result.rows_affected() as u32)
}

fn row_to_library_scan_unmatched_pg(
    row: &sqlx::postgres::PgRow,
) -> AppResult<LibraryScanUnmatchedItem> {
    let facet_raw: String = row.try_get("facet").map_err(repo_err)?;
    let facet = MediaFacet::parse(&facet_raw)
        .ok_or_else(|| AppError::Repository(format!("invalid unmatched item facet {facet_raw}")))?;
    let status_raw: String = row.try_get("status").map_err(repo_err)?;
    let status = PendingImportStatus::parse(&status_raw).ok_or_else(|| {
        AppError::Repository(format!("invalid unmatched item status {status_raw}"))
    })?;
    let attempts_value = row
        .try_get::<Option<Value>, _>("search_attempts_json")
        .ok()
        .flatten()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let search_attempts =
        serde_json::from_value::<Vec<LibraryScanUnmatchedSearchAttempt>>(attempts_value)
            .map_err(repo_err)?;
    Ok(LibraryScanUnmatchedItem {
        id: row.try_get("id").map_err(repo_err)?,
        library_id: text_from_pg(row, "library_id")
            .unwrap_or_else(|| scryer_domain::default_library_id_for_facet(&facet)),
        facet,
        status,
        title_id: text_from_pg(row, "title_id"),
        scan_session_id: row.try_get("scan_session_id").map_err(repo_err)?,
        scan_root: row.try_get("scan_root").map_err(repo_err)?,
        item_path: row.try_get("item_path").map_err(repo_err)?,
        display_name: row.try_get("display_name").map_err(repo_err)?,
        query: row.try_get("query").map_err(repo_err)?,
        year_hint: i32_from_pg(row, "year_hint"),
        reason_code: row.try_get("reason_code").map_err(repo_err)?,
        error_message: text_from_pg(row, "error_message"),
        search_attempts,
        created_at: required_timestamp_from_pg(row, "created_at")?,
        updated_at: required_timestamp_from_pg(row, "updated_at")?,
    })
}

async fn replace_title_image_pg_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    title_id: &str,
    replacement: &TitleImageReplacement,
) -> AppResult<()> {
    let image_id: String = sqlx::query(
        "INSERT INTO title_images (
            id, title_id, provider, provider_image_id, kind, source_url, source_etag,
            source_last_modified, source_format, source_width, source_height, storage_mode,
            master_path, master_format, master_sha256, master_width, master_height, bytes,
            created_at, updated_at
         ) VALUES ($1, $2, 'tvdb', NULL, $3, $4, $5, $6, $7, $8, $9, $10,
                   NULL, $11, $12, $13, $14, $15, NOW(), NOW())
         ON CONFLICT (title_id, kind) DO UPDATE SET
            source_url = EXCLUDED.source_url,
            source_etag = EXCLUDED.source_etag,
            source_last_modified = EXCLUDED.source_last_modified,
            source_format = EXCLUDED.source_format,
            source_width = EXCLUDED.source_width,
            source_height = EXCLUDED.source_height,
            storage_mode = EXCLUDED.storage_mode,
            master_format = EXCLUDED.master_format,
            master_sha256 = EXCLUDED.master_sha256,
            master_width = EXCLUDED.master_width,
            master_height = EXCLUDED.master_height,
            bytes = EXCLUDED.bytes,
            updated_at = NOW()
         RETURNING id",
    )
    .bind(Id::new().0)
    .bind(title_id)
    .bind(replacement.kind.as_str())
    .bind(&replacement.source_url)
    .bind(&replacement.source_etag)
    .bind(&replacement.source_last_modified)
    .bind(&replacement.source_format)
    .bind(replacement.source_width)
    .bind(replacement.source_height)
    .bind(replacement.storage_mode.as_str())
    .bind(&replacement.master_format)
    .bind(&replacement.master_sha256)
    .bind(replacement.master_width)
    .bind(replacement.master_height)
    .bind(&replacement.master_bytes)
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?
    .try_get("id")
    .map_err(repo_err)?;

    sqlx::query("DELETE FROM title_image_variants WHERE title_image_id = $1")
        .bind(&image_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;

    for variant in &replacement.variants {
        sqlx::query(
            "INSERT INTO title_image_variants
             (id, title_image_id, variant_key, path, format, width, height, bytes, sha256, created_at, updated_at)
             VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, NOW(), NOW())",
        )
        .bind(Id::new().0)
        .bind(&image_id)
        .bind(&variant.variant_key)
        .bind(&variant.format)
        .bind(variant.width)
        .bind(variant.height)
        .bind(&variant.bytes)
        .bind(&variant.sha256)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }

    let local_path = crate::title_images::materialize_local_title_image_path(
        title_id,
        replacement.kind,
        replacement.storage_mode,
        &replacement.master_sha256,
        &replacement.variants,
    );
    let local_path_column = match replacement.kind {
        TitleImageKind::Poster => "poster_local_path",
        TitleImageKind::Banner => "banner_local_path",
        TitleImageKind::Fanart => "background_local_path",
    };
    let update_title_sql = format!("UPDATE titles SET {local_path_column} = $1 WHERE id = $2");
    let result = sqlx::query(&update_title_sql)
        .bind(&local_path)
        .bind(title_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {title_id}")));
    }
    Ok(())
}

async fn append_domain_event_pg_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    event: &NewDomainEvent,
) -> AppResult<DomainEvent> {
    let payload = serde_json::to_value(&event.payload).map_err(repo_err)?;
    let sequence: i64 = sqlx::query(
        "INSERT INTO domain_events (
            event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id,
            schema_version, stream_kind, stream_id, event_type, payload_json
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::jsonb)
         RETURNING sequence",
    )
    .bind(&event.event_id)
    .bind(event.occurred_at)
    .bind(&event.actor_user_id)
    .bind(&event.title_id)
    .bind(event.facet.as_ref().map(MediaFacet::as_str))
    .bind(&event.correlation_id)
    .bind(&event.causation_id)
    .bind(event.schema_version)
    .bind(event.stream.kind())
    .bind(event.stream.identifier())
    .bind(event.payload.event_type().as_str())
    .bind(payload)
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?
    .try_get("sequence")
    .map_err(repo_err)?;

    Ok(DomainEvent {
        sequence,
        event_id: event.event_id.clone(),
        occurred_at: event.occurred_at,
        actor_user_id: event.actor_user_id.clone(),
        title_id: event.title_id.clone(),
        facet: event.facet.clone(),
        correlation_id: event.correlation_id.clone(),
        causation_id: event.causation_id.clone(),
        schema_version: event.schema_version,
        stream: event.stream.clone(),
        payload: event.payload.clone(),
    })
}

fn row_to_subtitle_download_pg(row: &sqlx::postgres::PgRow) -> AppResult<SubtitleDownload> {
    let source_kind_raw: String = row.try_get("source_kind").map_err(repo_err)?;
    Ok(SubtitleDownload {
        id: row.try_get("id").map_err(repo_err)?,
        media_file_id: row.try_get("media_file_id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        episode_id: text_from_pg(row, "episode_id"),
        source_kind: ExternalSubtitleSourceKind::parse(&source_kind_raw)
            .ok_or_else(|| AppError::Repository("invalid external subtitle source kind".into()))?,
        language: row.try_get("language").map_err(repo_err)?,
        provider: text_from_pg(row, "provider").and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }),
        provider_file_id: text_from_pg(row, "provider_file_id"),
        file_path: row.try_get("file_path").map_err(repo_err)?,
        score: i32_from_pg(row, "score"),
        hearing_impaired: bool_from_pg(row, "hearing_impaired"),
        forced: bool_from_pg(row, "forced"),
        ai_translated: bool_from_pg(row, "ai_translated"),
        machine_translated: bool_from_pg(row, "machine_translated"),
        uploader: text_from_pg(row, "uploader"),
        release_info: text_from_pg(row, "release_info"),
        synced: bool_from_pg(row, "synced"),
        downloaded_at: required_timestamp_from_pg(row, "downloaded_at")?,
    })
}

fn row_to_external_subtitle_probe_cache_entry_pg(
    row: &sqlx::postgres::PgRow,
) -> AppResult<scryer_application::subtitles::ExternalSubtitleProbeCacheEntry> {
    let detection_source_language =
        scryer_application::subtitles::ExternalSubtitleDetectionSource::parse(
            &row.try_get::<String, _>("detection_source_language")
                .map_err(repo_err)?,
        )
        .ok_or_else(|| {
            AppError::Repository("invalid subtitle probe language detection source".into())
        })?;
    let detection_source_hi =
        scryer_application::subtitles::ExternalSubtitleDetectionSource::parse(
            &row.try_get::<String, _>("detection_source_hi")
                .map_err(repo_err)?,
        )
        .ok_or_else(|| AppError::Repository("invalid subtitle probe hi detection source".into()))?;

    Ok(
        scryer_application::subtitles::ExternalSubtitleProbeCacheEntry {
            media_file_id: row.try_get("media_file_id").map_err(repo_err)?,
            file_path: row.try_get("file_path").map_err(repo_err)?,
            size_bytes: i64_from_pg(row, "size_bytes").unwrap_or_default(),
            modified_at: timestamp_from_pg(row, "modified_at"),
            language: text_from_pg(row, "language"),
            hearing_impaired: row
                .try_get::<Option<bool>, _>("hearing_impaired")
                .ok()
                .flatten(),
            detection_source_language,
            detection_source_hi,
            probe_version: i32_from_pg(row, "probe_version").unwrap_or_default(),
            updated_at: required_timestamp_from_pg(row, "updated_at")?,
        },
    )
}

fn row_to_subtitle_blocklist_entry_pg(
    row: &sqlx::postgres::PgRow,
) -> AppResult<SubtitleBlocklistEntry> {
    Ok(SubtitleBlocklistEntry {
        id: row.try_get("id").map_err(repo_err)?,
        media_file_id: row.try_get("media_file_id").map_err(repo_err)?,
        provider: row.try_get("provider").map_err(repo_err)?,
        provider_file_id: row.try_get("provider_file_id").map_err(repo_err)?,
        language: row.try_get("language").map_err(repo_err)?,
        reason: text_from_pg(row, "reason"),
        created_at: required_timestamp_from_pg(row, "created_at")?,
    })
}

fn row_to_title_media_file(row: &sqlx::postgres::PgRow) -> AppResult<TitleMediaFile> {
    Ok(TitleMediaFile {
        id: row.try_get("id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        episode_id: text_from_pg(row, "episode_id"),
        file_path: row.try_get("file_path").map_err(repo_err)?,
        size_bytes: row
            .try_get::<Option<i64>, _>("size_bytes")
            .map_err(repo_err)?
            .unwrap_or(0),
        source_signature_scheme: text_from_pg(row, "source_signature_scheme"),
        source_signature_value: text_from_pg(row, "source_signature_value"),
        quality_label: text_from_pg(row, "quality_id"),
        scan_status: row
            .try_get::<Option<String>, _>("scan_status")
            .map_err(repo_err)?
            .unwrap_or_else(|| "pending".to_string()),
        created_at: row
            .try_get::<Option<chrono::DateTime<Utc>>, _>("created_at")
            .ok()
            .flatten()
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(now_rfc3339),
        video_codec: text_from_pg(row, "video_codec"),
        video_width: i32_from_pg(row, "video_width"),
        video_height: i32_from_pg(row, "video_height"),
        video_bitrate_kbps: i32_from_pg(row, "video_bitrate_kbps"),
        video_bit_depth: i32_from_pg(row, "video_bit_depth"),
        video_hdr_format: text_from_pg(row, "video_hdr_format"),
        video_frame_rate: text_from_pg(row, "video_frame_rate"),
        video_profile: text_from_pg(row, "video_profile"),
        audio_codec: text_from_pg(row, "audio_codec"),
        audio_profile: text_from_pg(row, "audio_profile"),
        audio_channels: i32_from_pg(row, "audio_channels"),
        audio_bitrate_kbps: i32_from_pg(row, "audio_bitrate_kbps"),
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: bool_from_pg(row, "has_multiaudio"),
        duration_seconds: i32_from_pg(row, "duration_seconds"),
        num_chapters: i32_from_pg(row, "num_chapters"),
        container_format: text_from_pg(row, "container_format"),
        scene_name: text_from_pg(row, "scene_name"),
        release_group: text_from_pg(row, "release_group"),
        source_type: text_from_pg(row, "source_type"),
        resolution: text_from_pg(row, "resolution"),
        video_codec_parsed: text_from_pg(row, "video_codec_parsed"),
        audio_codec_parsed: text_from_pg(row, "audio_codec_parsed"),
        audio_channels_parsed: text_from_pg(row, "audio_channels_parsed"),
        acquisition_score: i32_from_pg(row, "acquisition_score"),
        scoring_log: text_from_pg(row, "scoring_log"),
        indexer_source: text_from_pg(row, "indexer_source"),
        grabbed_release_title: text_from_pg(row, "grabbed_release_title"),
        grabbed_at: text_from_pg(row, "grabbed_at"),
        edition: text_from_pg(row, "edition"),
        original_file_path: text_from_pg(row, "original_file_path"),
        release_hash: text_from_pg(row, "release_hash"),
    })
}

#[async_trait]
impl MediaFileRepository for PostgresLibraryStateSql {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        let id = Id::new().0;
        let grabbed_at = parse_optional_rfc3339_timestamp(
            input.grabbed_at.as_deref(),
            "media_files.grabbed_at",
        )?;
        sqlx::query(
            "INSERT INTO media_files (
                id, title_id, file_path, size_bytes, quality_id, scan_status, created_at,
                source_signature_scheme, source_signature_value, scene_name, release_group,
                source_type, resolution, video_codec_parsed, audio_codec_parsed,
                audio_channels_parsed, acquisition_score, scoring_log, indexer_source,
                grabbed_release_title, grabbed_at, edition, original_file_path, release_hash
             )
             VALUES ($1, $2, $3, $4, $5, 'pending', NOW(), $6, $7, $8, $9, $10, $11,
                     $12, $13, $14, $15, $16, $17, $18, $19::timestamptz, $20, $21, $22)
             ON CONFLICT (file_path) DO UPDATE SET
                title_id = EXCLUDED.title_id,
                size_bytes = EXCLUDED.size_bytes,
                quality_id = EXCLUDED.quality_id,
                source_signature_scheme = EXCLUDED.source_signature_scheme,
                source_signature_value = EXCLUDED.source_signature_value
             RETURNING id",
        )
        .bind(&id)
        .bind(&input.title_id)
        .bind(&input.file_path)
        .bind(input.size_bytes)
        .bind(&input.quality_label)
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
        .bind(grabbed_at)
        .bind(&input.edition)
        .bind(&input.original_file_path)
        .bind(&input.release_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?
        .try_get("id")
        .map_err(repo_err)
    }
    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO file_episode_map (file_id, episode_id, created_at)
             VALUES ($1, $2, NOW())
             ON CONFLICT (file_id, episode_id) DO NOTHING",
        )
        .bind(file_id)
        .bind(episode_id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        sqlx::query("UPDATE media_files SET episode_id = COALESCE(episode_id, $2) WHERE id = $1")
            .bind(file_id)
            .bind(episode_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        let rows =
            sqlx::query("SELECT * FROM media_files WHERE title_id = $1 ORDER BY file_path, id")
                .bind(title_id)
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
        rows.iter().map(row_to_title_media_file).collect()
    }
    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        if episode_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT mf.*,
                    ARRAY_REMOVE(ARRAY_AGG(DISTINCT fem_all.episode_id), NULL) AS episode_ids
               FROM media_files mf
               JOIN file_episode_map fem_target ON fem_target.file_id = mf.id
               LEFT JOIN file_episode_map fem_all ON fem_all.file_id = mf.id
              WHERE mf.title_id = $1
                AND fem_target.episode_id = ANY($2)
                AND COALESCE(mf.scan_status, '') <> 'recycled'
              GROUP BY mf.id
              ORDER BY mf.created_at DESC",
        )
        .bind(title_id)
        .bind(episode_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(EpisodeScopedMediaFile {
                    media_file: row_to_title_media_file(row)?,
                    episode_ids: row
                        .try_get::<Vec<String>, _>("episode_ids")
                        .unwrap_or_default(),
                })
            })
            .collect()
    }
    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT title_id, COALESCE(SUM(GREATEST(size_bytes, 0)), 0)::BIGINT AS total_size_bytes
               FROM media_files
              WHERE title_id = ANY($1)
                AND COALESCE(scan_status, '') <> 'recycled'
              GROUP BY title_id",
        )
        .bind(title_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(TitleMediaSizeSummary {
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    total_size_bytes: row.try_get("total_size_bytes").map_err(repo_err)?,
                })
            })
            .collect()
    }
    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT DISTINCT ON (title_id) title_id, quality_id AS quality_tier
               FROM media_files
              WHERE title_id = ANY($1)
                AND COALESCE(scan_status, '') <> 'recycled'
                AND BTRIM(COALESCE(quality_id, '')) <> ''
              ORDER BY title_id, created_at DESC, id DESC",
        )
        .bind(title_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(TitleQualitySummary {
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    quality_tier: row.try_get("quality_tier").map_err(repo_err)?,
                })
            })
            .collect()
    }
    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT DISTINCT ON (COALESCE(fem.episode_id, media_files.title_id))
                    media_files.title_id, fem.episode_id, e.season_number, e.episode_number,
                    media_files.quality_id AS quality_tier
               FROM media_files
               LEFT JOIN file_episode_map fem ON fem.file_id = media_files.id
               LEFT JOIN episodes e ON e.id = fem.episode_id
              WHERE media_files.title_id = ANY($1)
                AND COALESCE(media_files.scan_status, '') <> 'recycled'
                AND BTRIM(COALESCE(media_files.quality_id, '')) <> ''
                AND (fem.episode_id IS NULL OR e.monitored = TRUE)
              ORDER BY COALESCE(fem.episode_id, media_files.title_id),
                       media_files.created_at DESC, media_files.id DESC",
        )
        .bind(title_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(CutoffUnmetQualitySummary {
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    episode_id: text_from_pg(row, "episode_id"),
                    season_number: text_from_pg(row, "season_number"),
                    episode_number: text_from_pg(row, "episode_number"),
                    quality_tier: row.try_get("quality_tier").map_err(repo_err)?,
                })
            })
            .collect()
    }
    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT e.title_id,
                    COUNT(DISTINCT e.id)::BIGINT AS total_episodes,
                    COUNT(DISTINCT CASE WHEN e.monitored THEN e.id END)::BIGINT AS monitored_episodes,
                    COUNT(DISTINCT CASE WHEN mf.id IS NOT NULL THEN e.id END)::BIGINT AS owned_episodes
               FROM episodes e
               JOIN collections c ON c.id = e.collection_id
               LEFT JOIN file_episode_map fem ON fem.episode_id = e.id
               LEFT JOIN media_files mf ON mf.id = fem.file_id
                    AND COALESCE(mf.scan_status, '') <> 'recycled'
              WHERE e.title_id = ANY($1)
                AND c.collection_type <> 'specials'
                AND c.collection_index <> '0'
                AND BTRIM(COALESCE(e.title, '')) <> ''
                AND UPPER(BTRIM(e.title)) NOT IN ('TBA', 'TBD')
                AND BTRIM(COALESCE(e.air_date, '')) <> ''
              GROUP BY e.title_id",
        )
        .bind(title_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(TitleEpisodeProgressSummary {
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    owned_episodes: row.try_get("owned_episodes").map_err(repo_err)?,
                    monitored_episodes: row.try_get("monitored_episodes").map_err(repo_err)?,
                    total_episodes: row.try_get("total_episodes").map_err(repo_err)?,
                })
            })
            .collect()
    }
    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        let analysis_json = serde_json::json!({
            "audio_languages": analysis.audio_languages,
            "audio_streams": analysis.audio_streams,
            "subtitle_languages": analysis.subtitle_languages,
            "subtitle_codecs": analysis.subtitle_codecs,
            "subtitle_streams": analysis.subtitle_streams,
        });
        sqlx::query(
            "UPDATE media_files SET
                scan_status = 'scanned',
                video_codec = $2,
                video_width = $3,
                video_height = $4,
                video_bitrate_kbps = $5,
                video_bit_depth = $6,
                video_hdr_format = $7,
                video_frame_rate = $8,
                video_profile = $9,
                audio_codec = $10,
                audio_profile = $11,
                audio_channels = $12,
                audio_bitrate_kbps = $13,
                has_multiaudio = $14,
                duration_seconds = $15,
                num_chapters = $16,
                container_format = $17,
                analysis_json = $18::jsonb
              WHERE id = $1",
        )
        .bind(file_id)
        .bind(analysis.video_codec)
        .bind(analysis.video_width)
        .bind(analysis.video_height)
        .bind(analysis.video_bitrate_kbps)
        .bind(analysis.video_bit_depth)
        .bind(analysis.video_hdr_format)
        .bind(analysis.video_frame_rate)
        .bind(analysis.video_profile)
        .bind(analysis.audio_codec)
        .bind(analysis.audio_profile)
        .bind(analysis.audio_channels)
        .bind(analysis.audio_bitrate_kbps)
        .bind(analysis.has_multiaudio)
        .bind(analysis.duration_seconds)
        .bind(analysis.num_chapters)
        .bind(analysis.container_format)
        .bind(analysis_json)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE media_files
                SET size_bytes = $2,
                    source_signature_scheme = $3,
                    source_signature_value = $4
              WHERE id = $1",
        )
        .bind(file_id)
        .bind(size_bytes)
        .bind(source_signature_scheme)
        .bind(source_signature_value)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        sqlx::query("UPDATE media_files SET file_path = $2 WHERE id = $1")
            .bind(file_id)
            .bind(file_path)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        sqlx::query("UPDATE media_files SET scan_status = 'failed', scan_error = $2 WHERE id = $1")
            .bind(file_id)
            .bind(error)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM media_files WHERE id = $1")
            .bind(file_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        let row = sqlx::query("SELECT * FROM media_files WHERE id = $1")
            .bind(file_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(row_to_title_media_file).transpose()
    }
    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        let row = sqlx::query("SELECT * FROM media_files WHERE file_path = $1")
            .bind(file_path)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(row_to_title_media_file).transpose()
    }
}

#[async_trait]
impl WantedItemRepository for PostgresLibraryStateSql {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        let conflict_target = if item.collection_id.is_some() {
            "(collection_id) WHERE collection_id IS NOT NULL"
        } else if item.episode_id.is_some() {
            "(title_id, episode_id) WHERE episode_id IS NOT NULL"
        } else {
            "(title_id) WHERE episode_id IS NULL AND collection_id IS NULL"
        };
        let next_search_at = parse_optional_rfc3339_timestamp(
            item.next_search_at.as_deref(),
            "wanted_items.next_search_at",
        )?;
        let last_search_at = parse_optional_rfc3339_timestamp(
            item.last_search_at.as_deref(),
            "wanted_items.last_search_at",
        )?;
        let baseline_date = parse_optional_rfc3339_timestamp(
            item.baseline_date.as_deref(),
            "wanted_items.baseline_date",
        )?;
        let sql = format!(
            "INSERT INTO wanted_items
             (id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
              last_search_at, search_count, baseline_date, status, grabbed_release, current_score,
              created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz, $8::timestamptz, $9,
                     $10::timestamptz, $11, $12, $13, NOW(), NOW())
             ON CONFLICT {conflict_target} DO UPDATE SET
                search_phase = EXCLUDED.search_phase,
                next_search_at = CASE
                    WHEN EXCLUDED.next_search_at IS NULL THEN NULL
                    WHEN wanted_items.search_count > 0 AND wanted_items.next_search_at IS NOT NULL
                    THEN wanted_items.next_search_at
                    WHEN wanted_items.status IN ('paused', 'completed')
                    THEN wanted_items.next_search_at
                    ELSE EXCLUDED.next_search_at
                END,
                baseline_date = EXCLUDED.baseline_date,
                status = CASE
                    WHEN wanted_items.status IN ('completed', 'paused') AND EXCLUDED.status = 'wanted'
                    THEN wanted_items.status
                    ELSE EXCLUDED.status
                END,
                updated_at = NOW()"
        );
        sqlx::query(&sql)
            .bind(&item.id)
            .bind(&item.title_id)
            .bind(&item.episode_id)
            .bind(&item.collection_id)
            .bind(&item.media_type)
            .bind(&item.search_phase)
            .bind(next_search_at)
            .bind(last_search_at)
            .bind(item.search_count)
            .bind(baseline_date)
            .bind(item.status.as_str())
            .bind(&item.grabbed_release)
            .bind(item.current_score)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(item.id.clone())
    }
    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<WantedItem>> {
        let mut builder = QueryBuilder::<Postgres>::new(
            "SELECT w.id, w.title_id, w.episode_id, w.collection_id, e.season_number,
                    NULL::TEXT AS title_name, NULL::TEXT AS title_slug, t.facet AS title_facet,
                    t.library_id AS library_id, NULL::TEXT AS library_name, NULL::TEXT AS library_slug,
                    e.episode_number, w.media_type, w.search_phase, w.next_search_at,
                    w.last_search_at, w.search_count, w.baseline_date, w.status, w.grabbed_release,
                    w.current_score, NULL::TEXT AS latest_decision_id,
                    NULL::TEXT AS latest_decision_wanted_item_id, NULL::TEXT AS latest_decision_title_id,
                    NULL::TEXT AS latest_decision_release_title, NULL::TEXT AS latest_decision_release_url,
                    NULL::BIGINT AS latest_decision_release_size_bytes, NULL::TEXT AS latest_decision_decision_code,
                    NULL::BIGINT AS latest_decision_candidate_score, NULL::BIGINT AS latest_decision_current_score,
                    NULL::BIGINT AS latest_decision_score_delta, NULL::JSONB AS latest_decision_explanation_json,
                    NULL::TIMESTAMPTZ AS latest_decision_created_at, FALSE AS mismatch_recovery_eligible,
                    w.created_at, w.updated_at
               FROM wanted_items w
               JOIN titles t ON t.id = w.title_id
               LEFT JOIN episodes e ON e.id = w.episode_id
              WHERE w.status = 'wanted'
                AND w.next_search_at IS NOT NULL
                AND w.next_search_at <= ",
        );
        builder.push_bind(now);
        builder
            .push("::timestamptz AND (w.media_type != 'episode' OR w.baseline_date IS NOT NULL)");
        if !excluded_facets.is_empty() {
            builder.push(" AND t.facet != ALL(");
            let values = excluded_facets
                .iter()
                .map(MediaFacet::as_str)
                .collect::<Vec<_>>();
            builder.push_bind(values);
            builder.push(")");
        }
        builder.push(" ORDER BY w.next_search_at ASC LIMIT ");
        builder.push_bind(batch_limit);
        let rows = builder
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_wanted_item_pg).collect()
    }
    async fn update_wanted_item_status(
        &self,
        id: &str,
        status: &str,
        next_search_at: Option<&str>,
        last_search_at: Option<&str>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        let next_search_at =
            parse_optional_rfc3339_timestamp(next_search_at, "wanted_items.next_search_at")?;
        let last_search_at =
            parse_optional_rfc3339_timestamp(last_search_at, "wanted_items.last_search_at")?;
        sqlx::query(
            "UPDATE wanted_items
                SET status = $1, next_search_at = $2::timestamptz, last_search_at = $3::timestamptz,
                    search_count = $4, current_score = $5, grabbed_release = $6, updated_at = NOW()
              WHERE id = $7",
        )
        .bind(status)
        .bind(next_search_at)
        .bind(last_search_at)
        .bind(search_count)
        .bind(current_score)
        .bind(grabbed_release)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        let condition = if episode_id.is_some() {
            " WHERE w.title_id = $1 AND w.episode_id = $2"
        } else {
            " WHERE w.title_id = $1 AND w.episode_id IS NULL AND w.collection_id IS NULL"
        };
        let sql = format!("{}{}", wanted_item_select_sql(), condition);
        let mut query = sqlx::query(&sql).bind(title_id);
        if let Some(episode_id) = episode_id {
            query = query.bind(episode_id);
        }
        let row = query.fetch_optional(&self.pool).await.map_err(repo_err)?;
        row.as_ref().map(row_to_wanted_item_pg).transpose()
    }
    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM wanted_items WHERE title_id = $1")
            .bind(title_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM wanted_items
              WHERE collection_id = $1
                 OR episode_id IN (SELECT id FROM episodes WHERE collection_id = $1)",
        )
        .bind(collection_id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM wanted_items WHERE episode_id = $1")
            .bind(episode_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        let next_search_at = parse_rfc3339_timestamp(now, "wanted_items.next_search_at")?;
        let result = sqlx::query(
            "UPDATE wanted_items
                SET next_search_at = $1::timestamptz, updated_at = $1::timestamptz
              WHERE status = 'wanted'
                AND search_count > 0
                AND current_score IS NULL
                AND (media_type != 'episode' OR baseline_date IS NOT NULL)",
        )
        .bind(next_search_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected())
    }
    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        let created_at =
            parse_rfc3339_timestamp(&decision.created_at, "release_decisions.created_at")?;
        sqlx::query(
            "INSERT INTO release_decisions
             (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
              decision_code, candidate_score, current_score, score_delta, explanation_json, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::jsonb, $12::timestamptz)",
        )
        .bind(&decision.id)
        .bind(&decision.wanted_item_id)
        .bind(&decision.title_id)
        .bind(&decision.release_title)
        .bind(&decision.release_url)
        .bind(decision.release_size_bytes)
        .bind(&decision.decision_code)
        .bind(decision.candidate_score)
        .bind(decision.current_score)
        .bind(decision.score_delta)
        .bind(&decision.explanation_json)
        .bind(created_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(decision.id.clone())
    }
    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        let sql = format!("{} WHERE w.id = $1", wanted_item_select_sql());
        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(row_to_wanted_item_pg).transpose()
    }
    async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>> {
        let mut builder = QueryBuilder::<Postgres>::new(wanted_item_select_sql());
        builder.push(" WHERE TRUE");
        push_wanted_query_filters(&mut builder, &query);
        builder.push(" ORDER BY w.updated_at DESC LIMIT ");
        builder.push_bind(query.limit);
        builder.push(" OFFSET ");
        builder.push_bind(query.offset);
        let rows = builder
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_wanted_item_pg).collect()
    }
    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        let mut builder = QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*) AS cnt
               FROM wanted_items w
               LEFT JOIN titles t ON t.id = w.title_id
               LEFT JOIN LATERAL (
                   SELECT *
                   FROM release_decisions rd
                   WHERE rd.wanted_item_id = w.id
                   ORDER BY rd.created_at DESC
                   LIMIT 1
               ) latest_decision ON TRUE
              WHERE TRUE",
        );
        push_wanted_query_filters(&mut builder, &query);
        let row = builder
            .build()
            .fetch_one(&self.pool)
            .await
            .map_err(repo_err)?;
        row.try_get("cnt").map_err(repo_err)
    }
    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        let rows = sqlx::query(
            "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                    decision_code, candidate_score, current_score, score_delta, explanation_json, created_at
               FROM release_decisions
              WHERE title_id = $1
              ORDER BY created_at DESC
              LIMIT $2",
        )
        .bind(title_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_release_decision_pg).collect()
    }
    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        let rows = sqlx::query(
            "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                    decision_code, candidate_score, current_score, score_delta, explanation_json, created_at
               FROM release_decisions
              WHERE wanted_item_id = $1
              ORDER BY created_at DESC
              LIMIT $2",
        )
        .bind(wanted_item_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_release_decision_pg).collect()
    }
}

#[async_trait]
impl PendingReleaseRepository for PostgresLibraryStateSql {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        let added_at = parse_rfc3339_timestamp(&release.added_at, "pending_releases.added_at")?;
        let delay_until =
            parse_rfc3339_timestamp(&release.delay_until, "pending_releases.delay_until")?;
        let grabbed_at = parse_optional_rfc3339_timestamp(
            release.grabbed_at.as_deref(),
            "pending_releases.grabbed_at",
        )?;
        let published_at = parse_optional_rfc3339_timestamp(
            release.published_at.as_deref(),
            "pending_releases.published_at",
        )?;
        sqlx::query(
            "INSERT INTO pending_releases
             (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
              source_kind, release_score, scoring_log_json, indexer_source, release_guid,
              added_at, delay_until, status, grabbed_at,
              source_password, published_at, info_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, $10, $11,
                     $12::timestamptz, $13::timestamptz, $14, $15::timestamptz,
                     $16, $17::timestamptz, $18)",
        )
        .bind(&release.id)
        .bind(&release.wanted_item_id)
        .bind(&release.title_id)
        .bind(&release.release_title)
        .bind(&release.release_url)
        .bind(release.release_size_bytes)
        .bind(release.source_kind.map(|value| value.as_str().to_string()))
        .bind(release.release_score)
        .bind(&release.scoring_log_json)
        .bind(&release.indexer_source)
        .bind(&release.release_guid)
        .bind(added_at)
        .bind(delay_until)
        .bind(release.status.as_str())
        .bind(grabbed_at)
        .bind(&release.source_password)
        .bind(published_at)
        .bind(&release.info_hash)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(release.id.clone())
    }
    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "{} WHERE status = 'waiting' AND delay_until <= $1::timestamptz ORDER BY delay_until ASC",
            pending_release_select_sql()
        );
        let rows = sqlx::query(&sql)
            .bind(now)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_pending_release_pg).collect()
    }
    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "{} WHERE status = 'waiting' ORDER BY delay_until ASC",
            pending_release_select_sql()
        );
        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_pending_release_pg).collect()
    }
    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        let sql = format!("{} WHERE id = $1", pending_release_select_sql());
        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(row_to_pending_release_pg).transpose()
    }
    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "{} WHERE wanted_item_id = $1 AND status = 'waiting' ORDER BY release_score DESC",
            pending_release_select_sql()
        );
        let rows = sqlx::query(&sql)
            .bind(wanted_item_id)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_pending_release_pg).collect()
    }
    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "{} WHERE title_id = $1 ORDER BY added_at DESC",
            pending_release_select_sql()
        );
        let rows = sqlx::query(&sql)
            .bind(title_id)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_pending_release_pg).collect()
    }
    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        let grabbed_at =
            parse_optional_rfc3339_timestamp(grabbed_at, "pending_releases.grabbed_at")?;
        sqlx::query(
            "UPDATE pending_releases SET status = $2, grabbed_at = $3::timestamptz WHERE id = $1",
        )
        .bind(id)
        .bind(status.as_str())
        .bind(grabbed_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "{} WHERE wanted_item_id = $1 AND status = 'standby' ORDER BY release_score DESC, added_at ASC",
            pending_release_select_sql()
        );
        let rows = sqlx::query(&sql)
            .bind(wanted_item_id)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_pending_release_pg).collect()
    }
    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM pending_releases WHERE wanted_item_id = $1 AND status = 'standby'",
        )
        .bind(wanted_item_id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "{} WHERE status = 'standby' ORDER BY wanted_item_id ASC, release_score DESC, added_at ASC",
            pending_release_select_sql()
        );
        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_pending_release_pg).collect()
    }
    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        let grabbed_at =
            parse_optional_rfc3339_timestamp(grabbed_at, "pending_releases.grabbed_at")?;
        let result = sqlx::query(
            "UPDATE pending_releases
                SET status = $2, grabbed_at = $3::timestamptz
              WHERE id = $1 AND status = $4",
        )
        .bind(id)
        .bind(next_status.as_str())
        .bind(grabbed_at)
        .bind(current_status.as_str())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() > 0)
    }
    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE pending_releases
                SET status = 'superseded'
              WHERE wanted_item_id = $1 AND id <> $2 AND status = 'waiting'",
        )
        .bind(wanted_item_id)
        .bind(except_id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM pending_releases WHERE title_id = $1")
            .bind(title_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl BlocklistRepository for PostgresLibraryStateSql {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        let id = Id::new().0;
        let data_json = serde_json::to_value(&entry.data)
            .map_err(repo_err)
            .ok()
            .filter(|value| !value.is_null());
        sqlx::query(
            "INSERT INTO blocklist
             (id, title_id, source_title, source_hint, quality, download_id, reason, data_json, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, NOW())",
        )
        .bind(&id)
        .bind(&entry.title_id)
        .bind(&entry.source_title)
        .bind(&entry.source_hint)
        .bind(&entry.quality)
        .bind(&entry.download_id)
        .bind(&entry.reason)
        .bind(data_json)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(id)
    }
    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        let rows = sqlx::query(
            "SELECT id, title_id, source_title, source_hint, quality, download_id, reason, data_json, created_at
               FROM blocklist
              WHERE title_id = $1
              ORDER BY created_at DESC
              LIMIT $2",
        )
        .bind(title_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.iter().map(row_to_blocklist_entry_pg).collect())
    }
    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blocklist")
            .fetch_one(&self.pool)
            .await
            .map_err(repo_err)?;
        let rows = sqlx::query(
            "SELECT id, title_id, source_title, source_hint, quality, download_id, reason, data_json, created_at
               FROM blocklist
              ORDER BY created_at DESC
              LIMIT $1 OFFSET $2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok((rows.iter().map(row_to_blocklist_entry_pg).collect(), total))
    }
    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool> {
        let Some(source_title) = source_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
        else {
            return Ok(false);
        };
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                 SELECT 1 FROM blocklist
                  WHERE title_id = $1
                    AND LOWER(BTRIM(COALESCE(source_title, ''))) = $2
             )",
        )
        .bind(title_id)
        .bind(source_title)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)
    }
    async fn remove(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM blocklist WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                 SELECT 1 FROM blocklist
                  WHERE title_id = $1
                    AND LOWER(BTRIM(COALESCE(source_title, ''))) = LOWER(BTRIM($2))
             )",
        )
        .bind(title_id)
        .bind(source_title)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)
    }
    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM blocklist WHERE title_id = $1")
            .bind(title_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl HousekeepingRepository for PostgresLibraryStateSql {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        delete_older_than(&self.pool, "release_decisions", "created_at", days).await
    }
    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        delete_older_than(
            &self.pool,
            "release_download_attempts",
            "attempted_at",
            days,
        )
        .await
    }
    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        let result = sqlx::query(
            "DELETE FROM event_outboxes
              WHERE status = 'dispatched'
                AND COALESCE(dispatched_at, created_at) < NOW() - ($1::TEXT || ' days')::interval",
        )
        .bind(days.to_string())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }
    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        delete_older_than(&self.pool, "history_events", "occurred_at", days).await
    }
    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        if event_types.is_empty() {
            return Ok(0);
        }
        let values = event_types
            .iter()
            .map(|event_type| event_type.as_str())
            .collect::<Vec<_>>();
        let result = sqlx::query(
            "DELETE FROM domain_events
              WHERE occurred_at < NOW() - ($1::TEXT || ' days')::interval
                AND event_type = ANY($2)",
        )
        .bind(days.to_string())
        .bind(values)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }
    async fn delete_title_history_older_than(&self, days: i64) -> AppResult<u32> {
        let result = sqlx::query(
            "DELETE FROM domain_events
              WHERE title_id IS NOT NULL
                AND occurred_at < NOW() - ($1::TEXT || ' days')::interval",
        )
        .bind(days.to_string())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }
    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        delete_older_than(&self.pool, "download_import_artifacts", "created_at", days).await
    }
    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        let result = sqlx::query(
            "DELETE FROM imports
              WHERE status IN ('completed', 'failed', 'skipped')
                AND COALESCE(completed_at, updated_at, created_at) < NOW() - ($1::TEXT || ' days')::interval",
        )
        .bind(days.to_string())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }
    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        let result = sqlx::query(
            "DELETE FROM download_queue_commands
              WHERE status IN ('completed', 'failed')
                AND COALESCE(finished_at, updated_at, created_at) < NOW() - ($1::TEXT || ' days')::interval",
        )
        .bind(days.to_string())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }
    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        delete_older_than(&self.pool, "rule_set_history", "created_at", days).await
    }
    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        delete_for_title_ids(&self.pool, "history_events", title_ids).await
    }
    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        delete_for_title_ids(&self.pool, "download_import_artifacts", title_ids).await
    }
    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        delete_for_title_ids(&self.pool, "release_download_attempts", title_ids).await
    }
    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT id, file_path FROM media_files ORDER BY id")
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok((
                    row.try_get("id").map_err(repo_err)?,
                    row.try_get("file_path").map_err(repo_err)?,
                ))
            })
            .collect()
    }
    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        if ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query("DELETE FROM media_files WHERE id = ANY($1)")
            .bind(ids)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }
}

#[async_trait]
impl LibraryProbeRepository for PostgresLibraryStateSql {
    async fn get_probe_signature(
        &self,
        title_id: &str,
    ) -> AppResult<Option<LibraryProbeSignature>> {
        let row = sqlx::query(
            "SELECT title_id, path, probe_signature_scheme, probe_signature_value,
                    last_probed_at, last_changed_at
               FROM library_probe_signatures
              WHERE title_id = $1",
        )
        .bind(title_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(|row| {
            Ok(LibraryProbeSignature {
                title_id: row.try_get("title_id").map_err(repo_err)?,
                path: row.try_get("path").map_err(repo_err)?,
                probe_signature_scheme: text_from_pg(&row, "probe_signature_scheme"),
                probe_signature_value: text_from_pg(&row, "probe_signature_value"),
                last_probed_at: row
                    .try_get::<Option<chrono::DateTime<Utc>>, _>("last_probed_at")
                    .map_err(repo_err)?,
                last_changed_at: row
                    .try_get::<Option<chrono::DateTime<Utc>>, _>("last_changed_at")
                    .map_err(repo_err)?,
            })
        })
        .transpose()
    }
    async fn upsert_probe_signature(&self, probe: &LibraryProbeSignature) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO library_probe_signatures
             (title_id, path, probe_signature_scheme, probe_signature_value,
              last_probed_at, last_changed_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
             ON CONFLICT (title_id) DO UPDATE SET
                path = EXCLUDED.path,
                probe_signature_scheme = EXCLUDED.probe_signature_scheme,
                probe_signature_value = EXCLUDED.probe_signature_value,
                last_probed_at = EXCLUDED.last_probed_at,
                last_changed_at = EXCLUDED.last_changed_at,
                updated_at = NOW()",
        )
        .bind(&probe.title_id)
        .bind(&probe.path)
        .bind(&probe.probe_signature_scheme)
        .bind(&probe.probe_signature_value)
        .bind(probe.last_probed_at)
        .bind(probe.last_changed_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_probe_signatures_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if title_ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query("DELETE FROM library_probe_signatures WHERE title_id = ANY($1)")
            .bind(title_ids)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }
}

#[async_trait]
impl LibraryScanUnmatchedItemRepository for PostgresLibraryStateSql {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        let attempts = serde_json::to_value(&item.search_attempts).map_err(repo_err)?;
        let created_at =
            parse_rfc3339_timestamp(&item.created_at, "library_scan_unmatched_items.created_at")?;
        let updated_at =
            parse_rfc3339_timestamp(&item.updated_at, "library_scan_unmatched_items.updated_at")?;
        sqlx::query(
            "INSERT INTO library_scan_unmatched_items
             (id, library_id, facet, title_id, scan_session_id, scan_root, item_path, display_name,
              query, year_hint, reason_code, error_message, search_attempts_json, status,
              created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::jsonb,
                     $14, $15::timestamptz, $16::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
                library_id = EXCLUDED.library_id,
                facet = EXCLUDED.facet,
                title_id = EXCLUDED.title_id,
                scan_session_id = EXCLUDED.scan_session_id,
                scan_root = EXCLUDED.scan_root,
                item_path = EXCLUDED.item_path,
                display_name = EXCLUDED.display_name,
                query = EXCLUDED.query,
                year_hint = EXCLUDED.year_hint,
                reason_code = EXCLUDED.reason_code,
                error_message = EXCLUDED.error_message,
                search_attempts_json = EXCLUDED.search_attempts_json,
                status = CASE
                    WHEN library_scan_unmatched_items.status = 'ignored' AND EXCLUDED.status = 'pending'
                    THEN library_scan_unmatched_items.status
                    ELSE EXCLUDED.status
                END,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&item.id)
        .bind(&item.library_id)
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
        .bind(attempts)
        .bind(item.status.as_str())
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(item.id.clone())
    }
    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        let row = sqlx::query(
            "SELECT id, library_id, facet, title_id, scan_session_id, scan_root, item_path,
                    display_name, query, year_hint, reason_code, error_message,
                    search_attempts_json, status, created_at, updated_at
               FROM library_scan_unmatched_items
              WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref()
            .map(row_to_library_scan_unmatched_pg)
            .transpose()
    }
    async fn delete_library_scan_unmatched_item(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM library_scan_unmatched_items
              WHERE library_id = $1 AND facet = $2 AND item_path = $3",
        )
        .bind(library_id)
        .bind(facet.as_str())
        .bind(item_path)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32> {
        let result = sqlx::query("DELETE FROM library_scan_unmatched_items WHERE library_id = $1")
            .bind(library_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(result.rows_affected() as u32)
    }
    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        let mut builder = QueryBuilder::<Postgres>::new(
            "SELECT id, library_id, facet, title_id, scan_session_id, scan_root, item_path,
                    display_name, query, year_hint, reason_code, error_message,
                    search_attempts_json, status, created_at, updated_at
               FROM library_scan_unmatched_items
              WHERE TRUE",
        );
        if let Some(facet) = facet {
            builder.push(" AND facet = ");
            builder.push_bind(facet.as_str());
        }
        if let Some(scan_root) = scan_root {
            builder.push(" AND scan_root = ");
            builder.push_bind(scan_root);
        }
        if let Some(status) = status {
            builder.push(" AND status = ");
            builder.push_bind(status.as_str());
        }
        builder.push(" ORDER BY updated_at DESC, item_path ASC LIMIT ");
        builder.push_bind(limit);
        builder.push(" OFFSET ");
        builder.push_bind(offset);
        let rows = builder
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_library_scan_unmatched_pg).collect()
    }
    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        let mut builder = QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*) AS count FROM library_scan_unmatched_items WHERE TRUE",
        );
        if let Some(facet) = facet {
            builder.push(" AND facet = ");
            builder.push_bind(facet.as_str());
        }
        if let Some(scan_root) = scan_root {
            builder.push(" AND scan_root = ");
            builder.push_bind(scan_root);
        }
        if let Some(status) = status {
            builder.push(" AND status = ");
            builder.push_bind(status.as_str());
        }
        let row = builder
            .build()
            .fetch_one(&self.pool)
            .await
            .map_err(repo_err)?;
        row.try_get("count").map_err(repo_err)
    }
}

#[async_trait]
impl TitleImageRepository for PostgresLibraryStateSql {
    async fn list_titles_requiring_image_refresh(
        &self,
        kind: TitleImageKind,
        limit: usize,
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        let source_key = match kind {
            TitleImageKind::Poster => "poster_url",
            TitleImageKind::Banner => "banner_url",
            TitleImageKind::Fanart => "background_url",
        };
        let source_expr = format!("t.record_json ->> '{source_key}'");
        let sql = format!(
            "SELECT t.id AS title_id, {source_expr} AS source_url, ti.source_url AS cached_source_url
               FROM titles t
               LEFT JOIN title_images ti ON ti.title_id = t.id AND ti.kind = $1
              WHERE NULLIF(BTRIM({source_expr}), '') IS NOT NULL
                AND (ti.id IS NULL OR ti.source_url IS DISTINCT FROM {source_expr})
              ORDER BY t.updated_at ASC NULLS FIRST, t.id ASC
              LIMIT $2"
        );
        let rows = sqlx::query(&sql)
            .bind(kind.as_str())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(TitleImageSyncTask {
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    source_url: row.try_get("source_url").map_err(repo_err)?,
                    cached_source_url: text_from_pg(row, "cached_source_url"),
                })
            })
            .collect()
    }
    async fn replace_title_image(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        replace_title_image_pg_tx(&mut tx, title_id, &replacement).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }
    async fn replace_title_image_and_append_event(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
        event: NewDomainEvent,
    ) -> AppResult<DomainEvent> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        replace_title_image_pg_tx(&mut tx, title_id, &replacement).await?;
        let stored = append_domain_event_pg_tx(&mut tx, &event).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(stored)
    }
    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        if variant_key == "original"
            || (variant_key == "master"
                && matches!(kind, TitleImageKind::Banner | TitleImageKind::Fanart))
        {
            let row = sqlx::query(
                "SELECT master_format, master_sha256, bytes
                   FROM title_images
                  WHERE title_id = $1 AND kind = $2",
            )
            .bind(title_id)
            .bind(kind.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
            return row
                .map(|row| {
                    Ok(TitleImageBlob {
                        content_type: content_type_for_format(
                            row.try_get("master_format").map_err(repo_err)?,
                        ),
                        etag: row.try_get("master_sha256").map_err(repo_err)?,
                        bytes: row.try_get("bytes").map_err(repo_err)?,
                    })
                })
                .transpose();
        }

        let row = sqlx::query(
            "SELECT tiv.format, tiv.sha256, tiv.bytes
               FROM title_image_variants tiv
               JOIN title_images ti ON ti.id = tiv.title_image_id
              WHERE ti.title_id = $1 AND ti.kind = $2 AND tiv.variant_key = $3",
        )
        .bind(title_id)
        .bind(kind.as_str())
        .bind(variant_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(|row| {
            Ok(TitleImageBlob {
                content_type: content_type_for_format(row.try_get("format").map_err(repo_err)?),
                etag: row.try_get("sha256").map_err(repo_err)?,
                bytes: row.try_get("bytes").map_err(repo_err)?,
            })
        })
        .transpose()
    }
}

#[async_trait]
impl SubtitleDownloadRepository for PostgresLibraryStateSql {
    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        let rows = sqlx::query(
            "SELECT id, media_file_id, title_id, episode_id, source_kind, language, provider,
                    provider_file_id, file_path, score, hearing_impaired, forced,
                    ai_translated, machine_translated, uploader, release_info,
                    synced, downloaded_at
               FROM subtitle_downloads
              WHERE title_id = $1
              ORDER BY downloaded_at DESC",
        )
        .bind(title_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_subtitle_download_pg).collect()
    }
    async fn get(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let row = sqlx::query(
            "SELECT id, media_file_id, title_id, episode_id, source_kind, language, provider,
                    provider_file_id, file_path, score, hearing_impaired, forced,
                    ai_translated, machine_translated, uploader, release_info,
                    synced, downloaded_at
               FROM subtitle_downloads
              WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(row_to_subtitle_download_pg).transpose()
    }
    async fn list_for_media_file(&self, media_file_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        let rows = sqlx::query(
            "SELECT id, media_file_id, title_id, episode_id, source_kind, language, provider,
                    provider_file_id, file_path, score, hearing_impaired, forced,
                    ai_translated, machine_translated, uploader, release_info,
                    synced, downloaded_at
               FROM subtitle_downloads
              WHERE media_file_id = $1
              ORDER BY downloaded_at DESC",
        )
        .bind(media_file_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_subtitle_download_pg).collect()
    }
    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_application::subtitles::ExternalSubtitleProbeCacheEntry>> {
        let rows = sqlx::query(
            "SELECT media_file_id, file_path, size_bytes, modified_at, language,
                    hearing_impaired, detection_source_language, detection_source_hi,
                    probe_version, updated_at
               FROM external_subtitle_probe_cache
              WHERE media_file_id = $1
              ORDER BY file_path ASC",
        )
        .bind(media_file_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(row_to_external_subtitle_probe_cache_entry_pg)
            .collect()
    }
    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<SubtitleBlocklistEntry>> {
        let rows = sqlx::query(
            "SELECT id, media_file_id, provider, provider_file_id, language, reason, created_at
               FROM subtitle_blocklist
              WHERE media_file_id = $1
              ORDER BY created_at DESC",
        )
        .bind(media_file_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(row_to_subtitle_blocklist_entry_pg)
            .collect()
    }
    async fn insert(&self, download: &SubtitleDownload) -> AppResult<()> {
        let downloaded_at =
            parse_rfc3339_timestamp(&download.downloaded_at, "subtitle_downloads.downloaded_at")?;
        sqlx::query(
            "INSERT INTO subtitle_downloads
             (id, media_file_id, title_id, episode_id, source_kind, language, provider,
              provider_file_id, file_path, score, hearing_impaired, forced,
              ai_translated, machine_translated, uploader, release_info,
              synced, downloaded_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
                media_file_id = EXCLUDED.media_file_id,
                title_id = EXCLUDED.title_id,
                episode_id = EXCLUDED.episode_id,
                source_kind = EXCLUDED.source_kind,
                language = EXCLUDED.language,
                provider = EXCLUDED.provider,
                provider_file_id = EXCLUDED.provider_file_id,
                file_path = EXCLUDED.file_path,
                score = EXCLUDED.score,
                hearing_impaired = EXCLUDED.hearing_impaired,
                forced = EXCLUDED.forced,
                ai_translated = EXCLUDED.ai_translated,
                machine_translated = EXCLUDED.machine_translated,
                uploader = EXCLUDED.uploader,
                release_info = EXCLUDED.release_info,
                synced = EXCLUDED.synced,
                downloaded_at = EXCLUDED.downloaded_at",
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
        .bind(download.hearing_impaired)
        .bind(download.forced)
        .bind(download.ai_translated)
        .bind(download.machine_translated)
        .bind(&download.uploader)
        .bind(&download.release_info)
        .bind(download.synced)
        .bind(downloaded_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn upsert_probe_cache_entry(
        &self,
        entry: &scryer_application::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        let modified_at = parse_optional_rfc3339_timestamp(
            entry.modified_at.as_deref(),
            "external_subtitle_probe_cache.modified_at",
        )?;
        let updated_at = parse_rfc3339_timestamp(
            &entry.updated_at,
            "external_subtitle_probe_cache.updated_at",
        )?;
        sqlx::query(
            "INSERT INTO external_subtitle_probe_cache
             (media_file_id, file_path, size_bytes, modified_at, language,
              hearing_impaired, detection_source_language, detection_source_hi,
              probe_version, updated_at)
             VALUES ($1, $2, $3, $4::timestamptz, $5, $6, $7, $8, $9, $10::timestamptz)
             ON CONFLICT (media_file_id, file_path) DO UPDATE SET
                size_bytes = EXCLUDED.size_bytes,
                modified_at = EXCLUDED.modified_at,
                language = EXCLUDED.language,
                hearing_impaired = EXCLUDED.hearing_impaired,
                detection_source_language = EXCLUDED.detection_source_language,
                detection_source_hi = EXCLUDED.detection_source_hi,
                probe_version = EXCLUDED.probe_version,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&entry.media_file_id)
        .bind(&entry.file_path)
        .bind(entry.size_bytes)
        .bind(modified_at)
        .bind(&entry.language)
        .bind(entry.hearing_impaired)
        .bind(entry.detection_source_language.as_str())
        .bind(entry.detection_source_hi.as_str())
        .bind(entry.probe_version)
        .bind(updated_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        sqlx::query("UPDATE subtitle_downloads SET synced = $2 WHERE id = $1")
            .bind(id)
            .bind(synced)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn delete(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let existing = self.get(id).await?;
        if existing.is_some() {
            sqlx::query("DELETE FROM subtitle_downloads WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(repo_err)?;
        }
        Ok(existing)
    }
    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM external_subtitle_probe_cache
              WHERE media_file_id = $1 AND file_path = $2",
        )
        .bind(media_file_id)
        .bind(file_path)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                 SELECT 1 FROM subtitle_blocklist
                  WHERE media_file_id = $1 AND provider = $2 AND provider_file_id = $3
             )",
        )
        .bind(media_file_id)
        .bind(provider)
        .bind(provider_file_id)
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)
    }
    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO subtitle_blocklist
             (id, media_file_id, provider, provider_file_id, language, reason, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())
             ON CONFLICT (media_file_id, provider, provider_file_id) DO NOTHING",
        )
        .bind(Id::new().0)
        .bind(media_file_id)
        .bind(provider)
        .bind(provider_file_id)
        .bind(language)
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
}
