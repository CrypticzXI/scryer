use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, TitleImageBlob, TitleImageKind, TitleImageReplacement,
    TitleImageRepository, TitleImageStorageMode, TitleImageSyncTask,
};
use scryer_domain::{DomainEvent, DomainEventStream, Id, MediaFacet, NewDomainEvent};
use serde_json::Value as JsonValue;
use sqlx::{Row, types::Json};

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};

const DOMAIN_EVENT_COLUMNS: &str = "sequence, event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id, schema_version, stream_kind, stream_id, payload_json";

#[derive(Clone)]
pub struct TitleImageStore {
    datastore: StoreDatastore,
}

impl TitleImageStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl TitleImageRepository for TitleImageStore {
    async fn list_titles_requiring_image_refresh(
        &self,
        kind: TitleImageKind,
        limit: usize,
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        let (sql, args) = build_refresh_sql(kind, limit);
        fetch_refresh_tasks(self.datastore.read_exec(), &sql, &args).await
    }

    async fn clear_title_image_cache(&self) -> AppResult<()> {
        SqlRuntime::run_in_transaction(&self.datastore, "clear_title_image_cache", move |tx| {
            Box::pin(async move { clear_title_image_cache_tx(tx).await })
        })
        .await
    }

    async fn replace_title_image(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
    ) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_title_image", move |tx| {
            let title_id = title_id.clone();
            let replacement = replacement.clone();
            Box::pin(async move { replace_title_image_tx(tx, &title_id, &replacement).await })
        })
        .await
    }

    async fn replace_title_image_and_append_event(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
        event: NewDomainEvent,
    ) -> AppResult<DomainEvent> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_title_image_and_append_event",
            move |tx| {
                let title_id = title_id.clone();
                let replacement = replacement.clone();
                let event = event.clone();
                Box::pin(async move {
                    replace_title_image_tx(tx, &title_id, &replacement).await?;
                    append_domain_event_tx(tx, &event).await
                })
            },
        )
        .await
    }

    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        fetch_title_image_blob(self.datastore.read_exec(), title_id, kind, variant_key).await
    }
}

fn build_refresh_sql(kind: TitleImageKind, limit: usize) -> (String, Vec<SqlArg>) {
    let source_col = match kind {
        TitleImageKind::Poster => "poster_url",
        TitleImageKind::Fanart => "background_url",
    };
    let required_variant = crate::title_images::required_persisted_variant_for_kind(kind);

    let mut sql = format!(
        "SELECT t.id AS title_id, t.{source_col} AS source_url, ti.source_url AS cached_source_url
         FROM titles t
         LEFT JOIN title_images ti
           ON ti.title_id = t.id
          AND ti.kind = {{}}",
    );
    if let Some(required_variant) = required_variant {
        sql.push_str(&format!(
            "
         LEFT JOIN title_image_variants pv
           ON pv.title_image_id = ti.id
          AND pv.variant_key = '{required_variant}'"
        ));
    }
    sql.push_str(&format!(
        "
         WHERE NULLIF(TRIM(t.{source_col}), '') IS NOT NULL
           AND TRIM(t.{source_col}) NOT LIKE {{}}
           AND (
                ti.id IS NULL
                OR ti.source_url <> t.{source_col}"
    ));
    if required_variant.is_some() {
        sql.push_str(
            "
                OR (
                    ti.storage_mode = {}
                    AND pv.id IS NULL
                )",
        );
    }
    sql.push_str(
        "
           )
         ORDER BY t.created_at ASC
         LIMIT {}",
    );

    let mut args = vec![
        SqlArg::Text(kind.as_str().to_string()),
        SqlArg::Text(local_title_image_route_pattern().to_string()),
    ];
    if required_variant.is_some() {
        args.push(SqlArg::Text(
            TitleImageStorageMode::AvifMaster.as_str().to_string(),
        ));
    }
    args.push(SqlArg::I64(limit as i64));
    (sql, args)
}

fn local_title_image_route_pattern() -> &'static str {
    "/images/titles/%"
}

async fn fetch_refresh_tasks(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<TitleImageSyncTask>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| {
            Ok(TitleImageSyncTask {
                title_id: row.text("title_id")?,
                source_url: row.text("source_url")?,
                cached_source_url: row.opt_text("cached_source_url")?,
            })
        })
        .collect()
}

async fn clear_title_image_cache_tx(tx: &mut SqlTx<'_>) -> AppResult<()> {
    for kind in [TitleImageKind::Poster, TitleImageKind::Fanart] {
        repair_local_title_image_source_tx(tx, kind).await?;
        clear_unrecoverable_local_title_image_source_tx(tx, kind).await?;
    }

    SqlRuntime::execute(SqlExec::Tx(tx), "DELETE FROM title_image_variants", &[]).await?;
    SqlRuntime::execute(SqlExec::Tx(tx), "DELETE FROM title_images", &[]).await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE titles
            SET poster_local_path = NULL,
                background_local_path = NULL
          WHERE poster_local_path IS NOT NULL
             OR background_local_path IS NOT NULL",
        &[],
    )
    .await?;

    Ok(())
}

fn title_image_source_columns(kind: TitleImageKind) -> (&'static str, &'static str) {
    match kind {
        TitleImageKind::Poster => ("poster_url", "poster_local_path"),
        TitleImageKind::Fanart => ("background_url", "background_local_path"),
    }
}

async fn repair_local_title_image_source_tx(
    tx: &mut SqlTx<'_>,
    kind: TitleImageKind,
) -> AppResult<()> {
    let (source_col, _) = title_image_source_columns(kind);
    let sql = format!(
        "UPDATE titles
            SET {source_col} = (
                SELECT ti.source_url
                  FROM title_images ti
                 WHERE ti.title_id = titles.id
                   AND ti.kind = {{}}
                   AND NULLIF(TRIM(ti.source_url), '') IS NOT NULL
                   AND TRIM(ti.source_url) NOT LIKE {{}}
                 LIMIT 1
            )
          WHERE TRIM(COALESCE({source_col}, '')) LIKE {{}}
            AND EXISTS (
                SELECT 1
                  FROM title_images ti
                 WHERE ti.title_id = titles.id
                   AND ti.kind = {{}}
                   AND NULLIF(TRIM(ti.source_url), '') IS NOT NULL
                   AND TRIM(ti.source_url) NOT LIKE {{}}
            )"
    );
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &sql,
        &[
            SqlArg::Text(kind.as_str().to_string()),
            SqlArg::Text(local_title_image_route_pattern().to_string()),
            SqlArg::Text(local_title_image_route_pattern().to_string()),
            SqlArg::Text(kind.as_str().to_string()),
            SqlArg::Text(local_title_image_route_pattern().to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn clear_unrecoverable_local_title_image_source_tx(
    tx: &mut SqlTx<'_>,
    kind: TitleImageKind,
) -> AppResult<()> {
    let (source_col, _) = title_image_source_columns(kind);
    let sql = format!(
        "UPDATE titles
            SET {source_col} = NULL,
                metadata_fetched_at = NULL,
                metadata_hydration_next_attempt_at = {{}},
                metadata_hydration_attempt_count = 0
          WHERE TRIM(COALESCE({source_col}, '')) LIKE {{}}"
    );
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &sql,
        &[
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Text(local_title_image_route_pattern().to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn replace_title_image_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    replacement: &TitleImageReplacement,
) -> AppResult<()> {
    let now = Utc::now();
    let image_id = match tx {
        SqlTx::Sqlite(_) => upsert_title_image_sqlite_tx(tx, title_id, replacement, now).await?,
        SqlTx::Postgres(_) => {
            upsert_title_image_postgres_tx(tx, title_id, replacement, now).await?
        }
    };

    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM title_image_variants WHERE title_image_id = {}",
        &[SqlArg::Text(image_id.clone())],
    )
    .await?;

    for variant in &replacement.variants {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO title_image_variants (
                id, title_image_id, variant_key, path, format, width, height, bytes, sha256,
                created_at, updated_at
             ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(image_id.clone()),
                SqlArg::Text(variant.variant_key.clone()),
                SqlArg::OptText(None),
                SqlArg::Text(variant.format.clone()),
                SqlArg::I32(variant.width),
                SqlArg::I32(variant.height),
                SqlArg::OptBytes(Some(variant.bytes.clone())),
                SqlArg::Text(variant.sha256.clone()),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
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
        TitleImageKind::Fanart => "background_local_path",
    };
    let update_title_sql = format!("UPDATE titles SET {local_path_column} = {{}} WHERE id = {{}}");
    let rows = SqlRuntime::execute(
        SqlExec::Tx(tx),
        &update_title_sql,
        &[SqlArg::Text(local_path), SqlArg::Text(title_id.to_string())],
    )
    .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("title {title_id}")));
    }

    Ok(())
}

async fn upsert_title_image_sqlite_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    replacement: &TitleImageReplacement,
    now: chrono::DateTime<Utc>,
) -> AppResult<String> {
    let existing = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id FROM title_images WHERE title_id = {} AND kind = {}",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(replacement.kind.as_str().to_string()),
        ],
    )
    .await?
    .map(|row| row.text("id"))
    .transpose()?;

    if let Some(image_id) = existing {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE title_images SET
                source_url = {},
                source_etag = {},
                source_last_modified = {},
                source_format = {},
                source_width = {},
                source_height = {},
                storage_mode = {},
                master_format = {},
                master_sha256 = {},
                master_width = {},
                master_height = {},
                bytes = {},
                updated_at = {}
             WHERE id = {}",
            &title_image_update_args(&image_id, replacement, now),
        )
        .await?;
        Ok(image_id)
    } else {
        let image_id = Id::new().0;
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO title_images (
                id, title_id, provider, provider_image_id, kind, source_url, source_etag,
                source_last_modified, source_format, source_width, source_height, storage_mode,
                master_path, master_format, master_sha256, master_width, master_height, bytes,
                created_at, updated_at
            ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            &title_image_insert_args(&image_id, title_id, replacement, now),
        )
        .await?;
        Ok(image_id)
    }
}

async fn upsert_title_image_postgres_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    replacement: &TitleImageReplacement,
    now: chrono::DateTime<Utc>,
) -> AppResult<String> {
    let image_id = Id::new().0;
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "INSERT INTO title_images (
            id, title_id, provider, provider_image_id, kind, source_url, source_etag,
            source_last_modified, source_format, source_width, source_height, storage_mode,
            master_path, master_format, master_sha256, master_width, master_height, bytes,
            created_at, updated_at
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT (title_id, kind) DO UPDATE SET
            source_url = excluded.source_url,
            source_etag = excluded.source_etag,
            source_last_modified = excluded.source_last_modified,
            source_format = excluded.source_format,
            source_width = excluded.source_width,
            source_height = excluded.source_height,
            storage_mode = excluded.storage_mode,
            master_format = excluded.master_format,
            master_sha256 = excluded.master_sha256,
            master_width = excluded.master_width,
            master_height = excluded.master_height,
            bytes = excluded.bytes,
            updated_at = excluded.updated_at
         RETURNING id",
        &title_image_insert_args(&image_id, title_id, replacement, now),
    )
    .await?
    .ok_or_else(|| AppError::Repository("failed to upsert title image".into()))?;
    row.text("id")
}

fn title_image_insert_args(
    image_id: &str,
    title_id: &str,
    replacement: &TitleImageReplacement,
    now: chrono::DateTime<Utc>,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(image_id.to_string()),
        SqlArg::Text(title_id.to_string()),
        SqlArg::Text("tvdb".to_string()),
        SqlArg::OptText(None),
        SqlArg::Text(replacement.kind.as_str().to_string()),
        SqlArg::Text(replacement.source_url.clone()),
        SqlArg::OptText(replacement.source_etag.clone()),
        SqlArg::OptText(replacement.source_last_modified.clone()),
        SqlArg::Text(replacement.source_format.clone()),
        SqlArg::I32(replacement.source_width),
        SqlArg::I32(replacement.source_height),
        SqlArg::Text(replacement.storage_mode.as_str().to_string()),
        SqlArg::OptText(None),
        SqlArg::Text(replacement.master_format.clone()),
        SqlArg::Text(replacement.master_sha256.clone()),
        SqlArg::I32(replacement.master_width),
        SqlArg::I32(replacement.master_height),
        SqlArg::OptBytes(Some(replacement.master_bytes.clone())),
        SqlArg::Timestamp(now),
        SqlArg::Timestamp(now),
    ]
}

fn title_image_update_args(
    image_id: &str,
    replacement: &TitleImageReplacement,
    now: chrono::DateTime<Utc>,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(replacement.source_url.clone()),
        SqlArg::OptText(replacement.source_etag.clone()),
        SqlArg::OptText(replacement.source_last_modified.clone()),
        SqlArg::Text(replacement.source_format.clone()),
        SqlArg::I32(replacement.source_width),
        SqlArg::I32(replacement.source_height),
        SqlArg::Text(replacement.storage_mode.as_str().to_string()),
        SqlArg::Text(replacement.master_format.clone()),
        SqlArg::Text(replacement.master_sha256.clone()),
        SqlArg::I32(replacement.master_width),
        SqlArg::I32(replacement.master_height),
        SqlArg::OptBytes(Some(replacement.master_bytes.clone())),
        SqlArg::Timestamp(now),
        SqlArg::Text(image_id.to_string()),
    ]
}

async fn fetch_title_image_blob(
    exec: SqlExec<'_, '_>,
    title_id: &str,
    kind: TitleImageKind,
    variant_key: &str,
) -> AppResult<Option<TitleImageBlob>> {
    if variant_key == "original"
        || (variant_key == "master" && matches!(kind, TitleImageKind::Fanart))
    {
        return fetch_optional_blob(
            exec,
            "SELECT master_format AS format, master_sha256 AS sha256, bytes
             FROM title_images
             WHERE title_id = {} AND kind = {}",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(kind.as_str().to_string()),
            ],
        )
        .await;
    }

    fetch_optional_blob(
        exec,
        "SELECT tiv.format, tiv.sha256, tiv.bytes
         FROM title_image_variants tiv
         INNER JOIN title_images ti ON ti.id = tiv.title_image_id
         WHERE ti.title_id = {} AND ti.kind = {} AND tiv.variant_key = {}",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(kind.as_str().to_string()),
            SqlArg::Text(variant_key.to_string()),
        ],
    )
    .await
}

async fn fetch_optional_blob(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<TitleImageBlob>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| {
            Ok(TitleImageBlob {
                content_type: crate::title_images::content_type_for_format(row.text("format")?),
                etag: row.text("sha256")?,
                bytes: row
                    .opt_bytes("bytes")?
                    .ok_or_else(|| AppError::Repository("title image blob missing bytes".into()))?,
            })
        })
        .transpose()
}

async fn append_domain_event_tx(
    tx: &mut SqlTx<'_>,
    event: &NewDomainEvent,
) -> AppResult<DomainEvent> {
    let payload = serde_json::to_value(&event.payload).map_err(repo_err)?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO domain_events (
            event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id,
            schema_version, stream_kind, stream_id, event_type, payload_json
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(event.event_id.clone()),
            SqlArg::Timestamp(event.occurred_at),
            SqlArg::OptText(event.actor_user_id.clone()),
            SqlArg::OptText(event.title_id.clone()),
            SqlArg::OptText(event.facet.as_ref().map(|facet| facet.as_str().to_string())),
            SqlArg::OptText(event.correlation_id.clone()),
            SqlArg::OptText(event.causation_id.clone()),
            SqlArg::I32(event.schema_version),
            SqlArg::Text(event.stream.kind().to_string()),
            SqlArg::OptText(event.stream.identifier().map(str::to_string)),
            SqlArg::Text(event.payload.event_type().as_str().to_string()),
            SqlArg::Json(payload),
        ],
    )
    .await?;
    fetch_domain_event_by_event_id(SqlExec::Tx(tx), &event.event_id)
        .await?
        .ok_or_else(|| AppError::Repository("failed to reload inserted domain event".into()))
}

async fn fetch_domain_event_by_event_id(
    exec: SqlExec<'_, '_>,
    event_id: &str,
) -> AppResult<Option<DomainEvent>> {
    SqlRuntime::fetch_optional(
        exec,
        &format!("SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events WHERE event_id = {{}}"),
        &[SqlArg::Text(event_id.to_string())],
    )
    .await?
    .map(|row| domain_event_from_row(&row))
    .transpose()
}

fn domain_event_from_row(row: &SqlRow) -> AppResult<DomainEvent> {
    let stream_kind = row.text("stream_kind")?;
    let payload = serde_json::from_value(json_from_row(row, "payload_json")?).map_err(repo_err)?;
    Ok(DomainEvent {
        sequence: row.i64("sequence")?,
        event_id: row.text("event_id")?,
        occurred_at: row.timestamp("occurred_at")?,
        actor_user_id: row.opt_text("actor_user_id")?,
        title_id: row.opt_text("title_id")?,
        facet: row
            .opt_text("facet")?
            .as_deref()
            .and_then(MediaFacet::parse),
        correlation_id: row.opt_text("correlation_id")?,
        causation_id: row.opt_text("causation_id")?,
        schema_version: row.i32("schema_version")?,
        stream: stream_from_parts(&stream_kind, row.opt_text("stream_id")?)?,
        payload,
    })
}

fn stream_from_parts(kind: &str, identifier: Option<String>) -> AppResult<DomainEventStream> {
    match kind {
        "global" => Ok(DomainEventStream::Global),
        "title" => identifier
            .map(|title_id| DomainEventStream::Title { title_id })
            .ok_or_else(|| AppError::Repository("domain event missing title stream id".into())),
        "library_scan" => identifier
            .map(|session_id| DomainEventStream::LibraryScan { session_id })
            .ok_or_else(|| {
                AppError::Repository("domain event missing library scan stream id".into())
            }),
        "job_run" => identifier
            .map(|run_id| DomainEventStream::JobRun { run_id })
            .ok_or_else(|| AppError::Repository("domain event missing job run stream id".into())),
        "download_queue_item" => identifier
            .map(|item_id| DomainEventStream::DownloadQueueItem { item_id })
            .ok_or_else(|| {
                AppError::Repository("domain event missing download queue item stream id".into())
            }),
        other => Err(AppError::Repository(format!(
            "unknown domain event stream kind: {other}"
        ))),
    }
}

fn json_from_row(row: &SqlRow, column: &str) -> AppResult<JsonValue> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: String = row.try_get(column).map_err(repo_err)?;
            serde_json::from_str(&raw).map_err(repo_err)
        }
        SqlRow::Postgres(row) => {
            let raw: Json<JsonValue> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.0)
        }
    }
}
