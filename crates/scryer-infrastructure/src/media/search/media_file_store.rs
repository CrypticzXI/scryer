use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, CutoffUnmetQualitySummary, EpisodeScopedMediaFile, InsertMediaFileInput,
    MediaFileAnalysis, MediaFileRepository, TitleEpisodeProgressSummary, TitleMediaFile,
    TitleMediaSizeSummary, TitleQualitySummary,
};
use scryer_domain::Id;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;

use crate::queries::common::parse_utc_datetime;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore, repo_err};
use crate::storage::sql::json::{canonical_json_text, json_text_or};

const RECYCLE_BIN_PATH_SEGMENT: &str = "/.scryer-recycle/";

#[derive(Clone)]
pub struct MediaFileStore {
    datastore: StoreDatastore,
}

#[derive(Clone, Copy)]
enum SqlDialect {
    Sqlite,
    Postgres,
}

impl MediaFileStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl MediaFileRepository for MediaFileStore {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        let id = Id::new().0;
        let now = Utc::now();
        execute_write(
            &self.datastore,
            "insert_media_file",
            "INSERT INTO media_files
             (id, title_id, file_path, size_bytes, quality_id, scan_status, created_at,
              source_signature_scheme, source_signature_value,
              scene_name, release_group, source_type, resolution,
              video_codec_parsed, audio_codec_parsed, audio_channels_parsed,
              acquisition_score, scoring_log,
              indexer_source, grabbed_release_title, grabbed_at,
              edition, original_file_path, release_hash)
             VALUES ({}, {}, {}, {}, {}, 'imported', {},
                     {}, {},
                     {}, {}, {}, {},
                     {}, {}, {},
                     {}, {},
                     {}, {}, {},
                     {}, {}, {})
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
            vec![
                SqlArg::Text(id.clone()),
                SqlArg::Text(input.title_id.clone()),
                SqlArg::Text(input.file_path.clone()),
                SqlArg::I64(input.size_bytes),
                SqlArg::OptText(input.quality_label.clone()),
                SqlArg::Timestamp(now),
                SqlArg::OptText(input.source_signature_scheme.clone()),
                SqlArg::OptText(input.source_signature_value.clone()),
                SqlArg::OptText(input.scene_name.clone()),
                SqlArg::OptText(input.release_group.clone()),
                SqlArg::OptText(input.source_type.clone()),
                SqlArg::OptText(input.resolution.clone()),
                SqlArg::OptText(input.video_codec_parsed.as_ref().map(ToString::to_string)),
                SqlArg::OptText(input.audio_codec_parsed.clone()),
                SqlArg::OptText(input.audio_channels_parsed.clone()),
                SqlArg::OptI32(input.acquisition_score),
                SqlArg::OptText(input.scoring_log.clone()),
                SqlArg::OptText(input.indexer_source.clone()),
                SqlArg::OptText(input.grabbed_release_title.clone()),
                opt_timestamp_arg_for_datastore(&self.datastore, input.grabbed_at.as_deref())?,
                SqlArg::OptText(input.edition.clone()),
                SqlArg::OptText(input.original_file_path.clone()),
                SqlArg::OptText(input.release_hash.clone()),
            ],
        )
        .await?;
        Ok(id)
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "link_file_to_episode",
            "INSERT INTO file_episode_map (file_id, episode_id)
             VALUES ({}, {})
             ON CONFLICT(file_id, episode_id) DO NOTHING",
            vec![
                SqlArg::Text(file_id.to_string()),
                SqlArg::Text(episode_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        let dialect = dialect_for_datastore(&self.datastore);
        let sql = format!(
            "SELECT {}
             FROM media_files mf
             LEFT JOIN file_episode_map fem ON fem.file_id = mf.id
             WHERE mf.title_id = {{}}
               AND {}
             ORDER BY mf.created_at DESC",
            media_file_select_columns("fem.episode_id"),
            live_media_file_predicate(dialect, "mf")
        );
        fetch_media_files(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        if episode_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(episode_ids.len());
        let sql = format!(
            "SELECT {},
                    {} AS episode_ids_json
             FROM media_files mf
             INNER JOIN file_episode_map fem_target ON fem_target.file_id = mf.id
             LEFT JOIN file_episode_map fem_all ON fem_all.file_id = mf.id
             WHERE mf.title_id = {{}}
               AND {}
               AND fem_target.episode_id IN ({placeholders})
             GROUP BY mf.id
             ORDER BY mf.created_at DESC",
            media_file_select_columns("NULL"),
            episode_ids_aggregate(dialect),
            live_media_file_predicate(dialect, "mf")
        );
        let mut args = vec![SqlArg::Text(title_id.to_string())];
        args.extend(episode_ids.iter().cloned().map(SqlArg::Text));
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_episode_scoped_media_file)
            .collect()
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let total_size_expression = total_size_bytes_sum_expression(dialect, "matched.size_bytes");
        let sql = format!(
            "SELECT matched.title_id,
                    {total_size_expression} AS total_size_bytes
               FROM (
                    SELECT DISTINCT mf.id,
                           mf.title_id,
                           CASE
                               WHEN mf.size_bytes > 0 THEN mf.size_bytes
                               ELSE 0
                           END AS size_bytes
                      FROM media_files mf
                 LEFT JOIN file_episode_map fem
                        ON fem.file_id = mf.id
                 LEFT JOIN collections c
                        ON c.title_id = mf.title_id
                       AND c.ordered_path = mf.file_path
                     WHERE mf.title_id IN ({placeholders})
                       AND {}
                       AND (fem.file_id IS NOT NULL OR c.id IS NOT NULL)
               ) matched
              GROUP BY matched.title_id",
            live_media_file_predicate(dialect, "mf")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(TitleMediaSizeSummary {
                    title_id: row.text("title_id")?,
                    total_size_bytes: row.i64("total_size_bytes")?,
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

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
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
                   AND {normalized_quality} IS NOT NULL
             ) ranked
             WHERE quality_row = 1
               AND quality_tier IS NOT NULL",
            live_media_file_predicate(dialect, "media_files")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(TitleQualitySummary {
                    title_id: row.text("title_id")?,
                    quality_tier: row.text("quality_tier")?,
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

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let normalized_quality = normalized_quality_expression("media_files");
        let quality_rank = quality_rank_expression("media_files");
        let sql = format!(
            "SELECT title_id, episode_id, season_number, episode_number, quality_tier
             FROM (
                SELECT media_files.title_id AS title_id,
                       fem.episode_id AS episode_id,
                       e.season_number AS season_number,
                       e.episode_number AS episode_number,
                       {normalized_quality} AS quality_tier,
                       ROW_NUMBER() OVER (
                          PARTITION BY CASE
                              WHEN fem.episode_id IS NOT NULL THEN fem.episode_id
                              ELSE {}
                          END
                          ORDER BY {quality_rank} DESC,
                                   media_files.created_at DESC,
                                   media_files.id DESC
                       ) AS quality_row
                  FROM media_files
                  LEFT JOIN file_episode_map fem ON fem.file_id = media_files.id
                  LEFT JOIN episodes e ON e.id = fem.episode_id
                 WHERE media_files.title_id IN ({placeholders})
                   AND {}
                   AND {normalized_quality} IS NOT NULL
                   AND (fem.episode_id IS NULL OR {})
             ) ranked
             WHERE quality_row = 1
               AND quality_tier IS NOT NULL",
            title_partition_fallback(dialect, "media_files.title_id"),
            live_media_file_predicate(dialect, "media_files"),
            bool_column_is_true(dialect, "e.monitored")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(CutoffUnmetQualitySummary {
                    title_id: row.text("title_id")?,
                    episode_id: row.opt_text("episode_id")?,
                    season_number: row.opt_text("season_number")?,
                    episode_number: row.opt_text("episode_number")?,
                    quality_tier: row.text("quality_tier")?,
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

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let sql = format!(
            "SELECT e.title_id,
                    COUNT(DISTINCT e.id) AS total_episodes,
                    COUNT(DISTINCT CASE WHEN {} THEN e.id END) AS monitored_episodes,
                    COUNT(DISTINCT CASE WHEN mf.id IS NOT NULL THEN e.id END) AS owned_episodes
             FROM episodes e
             INNER JOIN collections c ON c.id = e.collection_id
             LEFT JOIN file_episode_map fem ON fem.episode_id = e.id
             LEFT JOIN media_files mf ON mf.id = fem.file_id AND {}
             WHERE e.title_id IN ({placeholders})
               AND c.collection_type <> 'specials'
               AND c.collection_index <> '0'
               AND trim(COALESCE(e.title, '')) <> ''
               AND upper(trim(e.title)) NOT IN ('TBA', 'TBD')
               AND trim(COALESCE(e.air_date, '')) <> ''
             GROUP BY e.title_id",
            bool_column_is_true(dialect, "e.monitored"),
            live_media_file_predicate(dialect, "mf")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(TitleEpisodeProgressSummary {
                    title_id: row.text("title_id")?,
                    owned_episodes: row.i64("owned_episodes")?,
                    monitored_episodes: row.i64("monitored_episodes")?,
                    total_episodes: row.i64("total_episodes")?,
                })
            })
            .collect()
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        let analysis_json = serialized_media_analysis(&analysis)?;
        execute_write(
            &self.datastore,
            "update_media_file_analysis",
            "UPDATE media_files SET
                video_codec = {},
                video_width = {},
                video_height = {},
                video_bitrate_kbps = {},
                video_bit_depth = {},
                video_hdr_format = {},
                video_frame_rate = {},
                video_profile = {},
                audio_codec = {},
                audio_profile = {},
                audio_channels = {},
                audio_bitrate_kbps = {},
                duration_seconds = {},
                num_chapters = {},
                container_format = {},
                analysis_json = {},
                has_multiaudio = {},
                scan_status = 'scanned'
             WHERE id = {}",
            vec![
                SqlArg::OptText(analysis.video_codec.as_ref().map(ToString::to_string)),
                SqlArg::OptI32(analysis.video_width),
                SqlArg::OptI32(analysis.video_height),
                SqlArg::OptI32(analysis.video_bitrate_kbps),
                SqlArg::OptI32(analysis.video_bit_depth),
                SqlArg::OptText(analysis.video_hdr_format),
                SqlArg::OptText(analysis.video_frame_rate),
                SqlArg::OptText(analysis.video_profile),
                SqlArg::OptText(analysis.audio_codec),
                SqlArg::OptText(analysis.audio_profile),
                SqlArg::OptI32(analysis.audio_channels),
                SqlArg::OptI32(analysis.audio_bitrate_kbps),
                SqlArg::OptI32(analysis.duration_seconds),
                SqlArg::OptI32(analysis.num_chapters),
                SqlArg::OptText(analysis.container_format),
                SqlArg::Text(analysis_json),
                SqlArg::Bool(analysis.has_multiaudio),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "update_media_file_source_signature",
            "UPDATE media_files SET
                size_bytes = {},
                source_signature_scheme = {},
                source_signature_value = {}
             WHERE id = {}",
            vec![
                SqlArg::I64(size_bytes),
                SqlArg::OptText(source_signature_scheme),
                SqlArg::OptText(source_signature_value),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "update_media_file_path",
            "UPDATE media_files SET file_path = {} WHERE id = {}",
            vec![
                SqlArg::Text(file_path.to_string()),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn replace_media_file_for_upgrade(
        &self,
        old_file_id: &str,
        replacement_file_id: &str,
        replacement_file_path: &str,
    ) -> AppResult<()> {
        let old_file_id = old_file_id.to_string();
        let replacement_file_id = replacement_file_id.to_string();
        let replacement_file_path = replacement_file_path.to_string();

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_media_file_for_upgrade",
            move |tx| {
                let old_file_id = old_file_id.clone();
                let replacement_file_id = replacement_file_id.clone();
                let replacement_file_path = replacement_file_path.clone();
                Box::pin(async move {
                    let deleted = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM media_files WHERE id = {}",
                        &[SqlArg::Text(old_file_id.clone())],
                    )
                    .await?;
                    if deleted != 1 {
                        return Err(AppError::Repository(format!(
                            "expected to delete one old media file during upgrade replacement, deleted {deleted}: {old_file_id}"
                        )));
                    }

                    let updated = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE media_files SET file_path = {} WHERE id = {}",
                        &[
                            SqlArg::Text(replacement_file_path),
                            SqlArg::Text(replacement_file_id.clone()),
                        ],
                    )
                    .await?;
                    if updated != 1 {
                        return Err(AppError::Repository(format!(
                            "expected to update one replacement media file during upgrade replacement, updated {updated}: {replacement_file_id}"
                        )));
                    }

                    Ok(())
                })
            },
        )
        .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "mark_scan_failed",
            "UPDATE media_files SET scan_status = 'scan_failed', scan_error = {} WHERE id = {}",
            vec![
                SqlArg::Text(error.to_string()),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        let sql = format!(
            "SELECT {}
             FROM media_files mf
             WHERE mf.id = {{}}",
            media_file_select_columns("NULL")
        );
        fetch_optional_media_file(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(file_id.to_string())],
        )
        .await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        let sql = format!(
            "SELECT {}
             FROM media_files mf
             WHERE mf.file_path = {{}}
             LIMIT 1",
            media_file_select_columns("NULL")
        );
        fetch_optional_media_file(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(file_path.to_string())],
        )
        .await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_media_file",
            "DELETE FROM media_files WHERE id = {}",
            vec![SqlArg::Text(file_id.to_string())],
        )
        .await?;
        Ok(())
    }
}

fn dialect_for_datastore(datastore: &StoreDatastore) -> SqlDialect {
    match datastore {
        StoreDatastore::Sqlite { .. } => SqlDialect::Sqlite,
        StoreDatastore::Postgres { .. } => SqlDialect::Postgres,
    }
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn live_media_file_predicate(dialect: SqlDialect, alias: &str) -> String {
    match dialect {
        SqlDialect::Sqlite => format!("instr({alias}.file_path, '{RECYCLE_BIN_PATH_SEGMENT}') = 0"),
        SqlDialect::Postgres => {
            format!("POSITION('{RECYCLE_BIN_PATH_SEGMENT}' IN {alias}.file_path) = 0")
        }
    }
}

fn total_size_bytes_sum_expression(dialect: SqlDialect, expr: &str) -> String {
    match dialect {
        SqlDialect::Sqlite => format!("COALESCE(SUM({expr}), 0)"),
        SqlDialect::Postgres => format!("COALESCE(SUM({expr}), 0)::BIGINT"),
    }
}

fn normalized_quality_expression(alias: &str) -> String {
    format!(
        "CASE
            WHEN {alias}.video_width >= 7680 OR {alias}.video_height >= 4200 THEN '4320P'
            WHEN {alias}.video_width >= 3840 OR {alias}.video_height >= 2100 THEN '2160P'
            WHEN {alias}.video_height >= 1300 THEN '1440P'
            WHEN {alias}.video_width >= 1920 OR {alias}.video_height >= 1000 THEN '1080P'
            WHEN {alias}.video_width >= 1280 OR {alias}.video_height >= 700 THEN '720P'
            WHEN {alias}.video_width >= 854 OR {alias}.video_height >= 480 THEN '480P'
            WHEN {alias}.video_height >= 300 THEN '360P'
            WHEN trim(COALESCE({alias}.quality_id, '')) = '' THEN NULL
            ELSE upper(trim({alias}.quality_id))
         END"
    )
}

fn quality_rank_expression(alias: &str) -> String {
    format!(
        "CASE
            WHEN {alias}.video_width >= 7680 OR {alias}.video_height >= 4200 THEN 0
            WHEN {alias}.video_width >= 3840 OR {alias}.video_height >= 2100 THEN 1
            WHEN {alias}.video_height >= 1300 THEN 2
            WHEN {alias}.video_width >= 1920 OR {alias}.video_height >= 1000 THEN 3
            WHEN {alias}.video_width >= 1280 OR {alias}.video_height >= 700 THEN 5
            WHEN {alias}.video_width >= 854 OR {alias}.video_height >= 480 THEN 6
            WHEN {alias}.video_height >= 300 THEN 7
            ELSE CASE upper(trim(COALESCE({alias}.quality_id, '')))
                WHEN '4320P' THEN 0
                WHEN '2160P' THEN 1
                WHEN '1440P' THEN 2
                WHEN '1080P' THEN 3
                WHEN '1080I' THEN 4
                WHEN '720P' THEN 5
                WHEN '480P' THEN 6
                WHEN '360P' THEN 7
                ELSE 999
            END
         END"
    )
}

fn serialized_media_analysis(analysis: &MediaFileAnalysis) -> AppResult<String> {
    canonical_json_text(analysis)
}

fn media_file_select_columns(episode_expr: &str) -> String {
    format!(
        "mf.id, mf.title_id, {episode_expr} AS episode_id, mf.file_path,
            mf.size_bytes, mf.source_signature_scheme, mf.source_signature_value,
            mf.quality_id, mf.scan_status, mf.created_at,
            mf.video_codec, mf.video_width, mf.video_height,
            mf.video_bitrate_kbps, mf.video_bit_depth,
            mf.video_hdr_format, mf.video_frame_rate, mf.video_profile,
            mf.audio_codec, mf.audio_profile, mf.audio_channels, mf.audio_bitrate_kbps,
            mf.duration_seconds, mf.num_chapters, mf.container_format, mf.analysis_json,
            mf.has_multiaudio,
            mf.scene_name, mf.release_group, mf.source_type, mf.resolution,
            mf.video_codec_parsed, mf.audio_codec_parsed, mf.audio_channels_parsed,
            mf.acquisition_score, mf.scoring_log,
            mf.indexer_source, mf.grabbed_release_title, mf.grabbed_at,
            mf.edition, mf.original_file_path, mf.release_hash",
    )
}

fn episode_ids_aggregate(dialect: SqlDialect) -> &'static str {
    match dialect {
        SqlDialect::Sqlite => "COALESCE(json_group_array(DISTINCT fem_all.episode_id), '[]')",
        SqlDialect::Postgres => {
            "COALESCE(
                jsonb_agg(DISTINCT fem_all.episode_id)
                    FILTER (WHERE fem_all.episode_id IS NOT NULL),
                '[]'::jsonb
             )::text"
        }
    }
}

fn title_partition_fallback(dialect: SqlDialect, title_id_expr: &str) -> String {
    match dialect {
        SqlDialect::Sqlite => format!("printf('title:%s', {title_id_expr})"),
        SqlDialect::Postgres => format!("('title:' || {title_id_expr})"),
    }
}

fn bool_column_is_true(dialect: SqlDialect, column: &str) -> String {
    match dialect {
        SqlDialect::Sqlite => format!("{column} = 1"),
        SqlDialect::Postgres => column.to_string(),
    }
}

async fn fetch_media_files(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<TitleMediaFile>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(row_to_title_media_file)
        .collect()
}

async fn fetch_optional_media_file(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<TitleMediaFile>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .as_ref()
        .map(row_to_title_media_file)
        .transpose()
}

fn row_to_title_media_file(row: &SqlRow) -> AppResult<TitleMediaFile> {
    let analysis = analysis_json_from_row(row);
    let mut audio_streams: Vec<scryer_application::AudioStreamDetail> =
        analysis_array_field(&analysis, "audio_streams");
    let mut subtitle_streams: Vec<scryer_application::SubtitleStreamDetail> =
        analysis_array_field(&analysis, "subtitle_streams");

    let audio_language_values: Vec<String> = analysis_array_field(&analysis, "audio_languages");
    let audio_languages = scryer_application::normalize_detected_audio_languages(
        audio_language_values.iter().map(String::as_str),
    );
    for stream in &mut audio_streams {
        stream.language = stream
            .language
            .as_deref()
            .and_then(scryer_application::normalize_detected_audio_language_code);
    }
    let subtitle_language_values: Vec<String> =
        analysis_array_field(&analysis, "subtitle_languages");
    let subtitle_languages = scryer_application::normalize_detected_subtitle_languages(
        subtitle_language_values.iter().map(String::as_str),
    );
    for stream in &mut subtitle_streams {
        stream.language = stream
            .language
            .as_deref()
            .and_then(scryer_application::normalize_detected_subtitle_language_code);
    }

    Ok(TitleMediaFile {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        episode_id: row.opt_text("episode_id")?,
        file_path: row.text("file_path")?,
        size_bytes: row.i64("size_bytes")?,
        source_signature_scheme: row.opt_text("source_signature_scheme")?,
        source_signature_value: row.opt_text("source_signature_value")?,
        quality_label: row.opt_text("quality_id")?,
        scan_status: row.text("scan_status")?,
        created_at: timestamp_text(row, "created_at")?,
        video_codec: parse_stored_video_codec(row.opt_text("video_codec")?)?,
        video_width: row.opt_i32("video_width")?,
        video_height: row.opt_i32("video_height")?,
        video_bitrate_kbps: row.opt_i32("video_bitrate_kbps")?,
        video_bit_depth: row.opt_i32("video_bit_depth")?,
        video_hdr_format: row.opt_text("video_hdr_format")?,
        video_frame_rate: row.opt_text("video_frame_rate")?,
        video_profile: row.opt_text("video_profile")?,
        audio_codec: row.opt_text("audio_codec")?,
        audio_profile: row.opt_text("audio_profile")?,
        audio_channels: row.opt_i32("audio_channels")?,
        audio_bitrate_kbps: row.opt_i32("audio_bitrate_kbps")?,
        audio_languages,
        audio_streams,
        subtitle_languages,
        subtitle_codecs: analysis_array_field(&analysis, "subtitle_codecs"),
        subtitle_streams,
        has_multiaudio: row.opt_bool("has_multiaudio")?.unwrap_or(false),
        duration_seconds: row.opt_i32("duration_seconds")?,
        num_chapters: row.opt_i32("num_chapters")?,
        container_format: row.opt_text("container_format")?,
        scene_name: row.opt_text("scene_name")?,
        release_group: row.opt_text("release_group")?,
        source_type: row.opt_text("source_type")?,
        resolution: row.opt_text("resolution")?,
        video_codec_parsed: parse_stored_video_codec(row.opt_text("video_codec_parsed")?)?,
        audio_codec_parsed: row.opt_text("audio_codec_parsed")?,
        audio_channels_parsed: row.opt_text("audio_channels_parsed")?,
        acquisition_score: row.opt_i32("acquisition_score")?,
        scoring_log: row.opt_text("scoring_log")?,
        indexer_source: row.opt_text("indexer_source")?,
        grabbed_release_title: row.opt_text("grabbed_release_title")?,
        grabbed_at: opt_timestamp_text(row, "grabbed_at")?,
        edition: row.opt_text("edition")?,
        original_file_path: row.opt_text("original_file_path")?,
        release_hash: row.opt_text("release_hash")?,
    })
}

fn parse_stored_video_codec(
    raw: Option<String>,
) -> AppResult<Option<scryer_application::VideoCodec>> {
    raw.map(|value| {
        scryer_application::VideoCodec::parse(value.as_str())
            .ok_or_else(|| repo_err(format!("invalid stored video codec {value:?}")))
    })
    .transpose()
}

fn row_to_episode_scoped_media_file(row: &SqlRow) -> AppResult<EpisodeScopedMediaFile> {
    let media_file = row_to_title_media_file(row)?;
    let mut episode_ids: Vec<String> = match row.opt_text("episode_ids_json") {
        Ok(Some(json)) => match serde_json::from_str::<Vec<String>>(&json) {
            Ok(episode_ids) => episode_ids,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    file_id = %media_file.id,
                    "failed to parse media_files.episode_ids_json; treating row as unlinked"
                );
                Vec::new()
            }
        },
        Ok(None) | Err(_) => Vec::new(),
    };
    episode_ids.sort();
    episode_ids.dedup();

    Ok(EpisodeScopedMediaFile {
        media_file,
        episode_ids,
    })
}

fn analysis_json_from_row(row: &SqlRow) -> JsonValue {
    json_text_or(row, "analysis_json", "{}")
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or(JsonValue::Null)
}

fn analysis_array_field<T: DeserializeOwned>(analysis: &JsonValue, field: &str) -> Vec<T> {
    analysis
        .get(field)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn opt_timestamp_arg_for_datastore(
    datastore: &StoreDatastore,
    value: Option<&str>,
) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::OptText(value.map(str::to_string))),
        StoreDatastore::Postgres { .. } => value
            .map(parse_utc_datetime)
            .transpose()
            .map(SqlArg::OptTimestamp),
    }
}

fn timestamp_text(row: &SqlRow, column: &str) -> AppResult<String> {
    match row {
        SqlRow::Sqlite(_) => row.text(column),
        SqlRow::Postgres(_) => row.timestamp(column).map(|value| value.to_rfc3339()),
    }
}

fn opt_timestamp_text(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(_) => row.opt_text(column),
        SqlRow::Postgres(_) => row
            .opt_timestamp(column)
            .map(|value| value.map(|value| value.to_rfc3339())),
    }
}

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: impl Into<String>,
    args: Vec<SqlArg>,
) -> AppResult<u64> {
    let sql = sql.into();
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let sql = sql.clone();
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MediaFileStore, ShowStore, SqliteServices, TitleStore};
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
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2026),
            overview: Some("overview".to_string()),
            poster_url: None,
            poster_source_url: None,
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

    fn title_store(services: &SqliteServices) -> TitleStore {
        TitleStore::new(services.datastore())
    }

    fn show_store(services: &SqliteServices) -> ShowStore {
        ShowStore::new(crate::queries::sql_runtime::StoreDatastore::Sqlite {
            pool: services.pool().clone(),
            writer_gate: services.writer_gate(),
        })
    }

    fn media_file_store(services: &SqliteServices) -> MediaFileStore {
        MediaFileStore::new(services.datastore())
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
        let titles = title_store(&services);
        let shows = show_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-live-query");
        titles
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
        ShowRepository::create_collection(&shows, collection.clone())
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
            air_date: Some("2026-04-01".to_string()),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
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
            air_date: Some("2026-04-08".to_string()),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_episode(&shows, episode_one.clone())
            .await
            .expect("episode one should insert");
        ShowRepository::create_episode(&shows, episode_two.clone())
            .await
            .expect("episode two should insert");

        let live_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                ..Default::default()
            })
            .await
            .expect("live media file should insert");
        media_files
            .link_file_to_episode(&live_file_id, &episode_one.id)
            .await
            .expect("live file should link");

        let recycled_file_id = media_files
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
        media_files
            .link_file_to_episode(&recycled_file_id, &episode_two.id)
            .await
            .expect("recycled file should link");

        let live_files = media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files should succeed");
        assert_eq!(live_files.len(), 1);
        assert_eq!(live_files[0].id, live_file_id);
        assert_eq!(
            live_files[0].file_path,
            "/library/Show/Season 01/Show - S01E01.mkv"
        );

        let size_summaries = media_files
            .list_title_media_size_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("size summaries should succeed");
        assert_eq!(size_summaries.len(), 1);
        assert_eq!(size_summaries[0].title_id, title.id);
        assert_eq!(size_summaries[0].total_size_bytes, 1_000);

        let episode_progress = media_files
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
        let titles = title_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-quality-summary");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("2160p".to_string()),
                ..Default::default()
            })
            .await
            .expect("high quality file should insert");

        media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E02.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("720p".to_string()),
                ..Default::default()
            })
            .await
            .expect("lower quality file should insert");

        media_files
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

        let quality_summaries = media_files
            .list_title_quality_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("quality summaries should succeed");
        assert_eq!(quality_summaries.len(), 1);
        assert_eq!(quality_summaries[0].title_id, title.id);
        assert_eq!(quality_summaries[0].quality_tier, "720P");

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn cutoff_unmet_quality_summaries_expand_season_pack_links() {
        let db = std::env::temp_dir().join(format!(
            "scryer_cutoff_unmet_quality_summary_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let media_files = media_file_store(&services);
        let shows = show_store(&services);

        let title = make_test_series_title("title-cutoff-summary");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        let collection = Collection {
            id: "collection-cutoff-summary".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_collection(&shows, collection.clone())
            .await
            .expect("collection should insert");

        let monitored_episode_one = Episode {
            id: "episode-cutoff-summary-1".to_string(),
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
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let monitored_episode_two = Episode {
            id: "episode-cutoff-summary-2".to_string(),
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
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let unmonitored_episode_three = Episode {
            id: "episode-cutoff-summary-3".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Episode 3".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: false,
            created_at: Utc::now(),
        };
        for episode in [
            &monitored_episode_one,
            &monitored_episode_two,
            &unmonitored_episode_three,
        ] {
            ShowRepository::create_episode(&shows, episode.clone())
                .await
                .expect("episode should insert");
        }

        let pack_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01 pack.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("720p".to_string()),
                ..Default::default()
            })
            .await
            .expect("season pack should insert");

        for episode_id in [
            &monitored_episode_one.id,
            &monitored_episode_two.id,
            &unmonitored_episode_three.id,
        ] {
            media_files
                .link_file_to_episode(&pack_file_id, episode_id)
                .await
                .expect("season pack should link");
        }
        media_files
            .update_media_file_analysis(
                &pack_file_id,
                MediaFileAnalysis {
                    video_codec: None,
                    video_width: Some(1920),
                    video_height: Some(800),
                    video_bitrate_kbps: None,
                    video_bit_depth: None,
                    video_hdr_format: None,
                    video_frame_rate: None,
                    video_profile: None,
                    audio_codec: None,
                    audio_profile: None,
                    audio_channels: None,
                    audio_bitrate_kbps: None,
                    audio_languages: vec![],
                    audio_streams: vec![],
                    subtitle_languages: vec![],
                    subtitle_codecs: vec![],
                    subtitle_streams: vec![],
                    has_multiaudio: false,
                    duration_seconds: None,
                    num_chapters: None,
                    container_format: None,
                },
            )
            .await
            .expect("season pack analysis should update");

        let summaries = media_files
            .list_cutoff_unmet_quality_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("cutoff summaries should succeed");

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].title_id, title.id);
        assert_eq!(summaries[0].quality_tier, "1080P");
        assert_eq!(summaries[0].season_number.as_deref(), Some("1"));
        assert_eq!(summaries[0].episode_number.as_deref(), Some("1"));
        assert_eq!(summaries[1].quality_tier, "1080P");
        assert_eq!(summaries[1].season_number.as_deref(), Some("1"));
        assert_eq!(summaries[1].episode_number.as_deref(), Some("2"));

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn episode_scoped_media_file_query_dedupes_file_ids_and_returns_full_episode_set() {
        let db = std::env::temp_dir().join(format!(
            "scryer_episode_scoped_media_files_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let shows = show_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-episode-scope");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        let collection = Collection {
            id: "collection-episode-scope".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_collection(&shows, collection.clone())
            .await
            .expect("collection should insert");

        let episode_one = Episode {
            id: "episode-episode-scope-1".to_string(),
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
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let episode_two = Episode {
            id: "episode-episode-scope-2".to_string(),
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
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let episode_three = Episode {
            id: "episode-episode-scope-3".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Episode 3".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        for episode in [&episode_one, &episode_two, &episode_three] {
            ShowRepository::create_episode(&shows, episode.clone())
                .await
                .expect("episode should insert");
        }

        let pack_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01-E02.mkv".to_string(),
                size_bytes: 2_000,
                ..Default::default()
            })
            .await
            .expect("pack file should insert");
        media_files
            .link_file_to_episode(&pack_file_id, &episode_one.id)
            .await
            .expect("pack should link episode one");
        media_files
            .link_file_to_episode(&pack_file_id, &episode_two.id)
            .await
            .expect("pack should link episode two");

        let single_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E03.mkv".to_string(),
                size_bytes: 1_000,
                ..Default::default()
            })
            .await
            .expect("single file should insert");
        media_files
            .link_file_to_episode(&single_file_id, &episode_three.id)
            .await
            .expect("single should link episode three");

        let recycled_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path:
                    "/library/Show/.scryer-recycle/20260404_000000_deadbeef/Show - S01E01.mkv"
                        .to_string(),
                size_bytes: 999,
                ..Default::default()
            })
            .await
            .expect("recycled file should insert");
        media_files
            .link_file_to_episode(&recycled_file_id, &episode_one.id)
            .await
            .expect("recycled should link episode one");

        let scoped = media_files
            .list_live_media_files_for_episode_ids(
                &title.id,
                &[episode_one.id.clone(), episode_two.id.clone()],
            )
            .await
            .expect("episode scoped query should succeed");

        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].media_file.id, pack_file_id);
        assert_eq!(
            scoped[0].episode_ids,
            vec![episode_one.id.clone(), episode_two.id.clone()]
        );

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
        let titles = title_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-audio-profile");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        let file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("720p".to_string()),
                resolution: Some("720p".to_string()),
                source_type: Some("WEB-DL".to_string()),
                audio_channels_parsed: Some("7.1".to_string()),
                ..Default::default()
            })
            .await
            .expect("media file should insert");

        media_files
            .update_media_file_analysis(
                &file_id,
                MediaFileAnalysis {
                    video_codec: Some(
                        scryer_application::VideoCodec::parse("hevc").expect("parse codec"),
                    ),
                    video_width: Some(1920),
                    video_height: Some(800),
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

        let files = media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files should succeed");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].quality_label.as_deref(), Some("720p"));
        assert_eq!(files[0].resolution.as_deref(), Some("720p"));
        assert_eq!(files[0].source_type.as_deref(), Some("WEB-DL"));
        assert_eq!(
            files[0].audio_profile.as_deref(),
            Some("DTS-HD MA + DTS:X IMAX")
        );
        assert_eq!(files[0].audio_channels_parsed.as_deref(), Some("7.1"));
        assert_eq!(
            files[0].audio_streams[0].profile.as_deref(),
            Some("DTS-HD MA + DTS:X IMAX")
        );
        let quality_summaries = media_files
            .list_title_quality_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("quality summaries should succeed");
        assert_eq!(quality_summaries.len(), 1);
        assert_eq!(quality_summaries[0].quality_tier, "1080P");

        let _ = std::fs::remove_file(db);
    }
}
