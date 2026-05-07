use chrono::Utc;
use scryer_application::{AppError, AppResult};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventStream, DomainEventType,
    ImportStatus, MediaFacet, NewDomainEvent, TitleHistoryEventType,
};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool, Transaction};

use super::common::parse_utc_datetime;

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

fn row_to_domain_event(row: &sqlx::sqlite::SqliteRow) -> AppResult<DomainEvent> {
    let sequence: i64 = row
        .try_get("sequence")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let event_id: String = row
        .try_get("event_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let occurred_at_raw: String = row
        .try_get("occurred_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let actor_user_id: Option<String> = row
        .try_get("actor_user_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title_id: Option<String> = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let facet_raw: Option<String> = row
        .try_get("facet")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let correlation_id: Option<String> = row
        .try_get("correlation_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let causation_id: Option<String> = row
        .try_get("causation_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let schema_version: i32 = row
        .try_get("schema_version")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let stream_kind: String = row
        .try_get("stream_kind")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let stream_id: Option<String> = row
        .try_get("stream_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let payload = serde_json::from_str::<DomainEventPayload>(&payload_json)
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(DomainEvent {
        sequence,
        event_id,
        occurred_at: parse_utc_datetime(&occurred_at_raw)?,
        actor_user_id,
        title_id,
        facet: facet_raw.as_deref().and_then(MediaFacet::parse),
        correlation_id,
        causation_id,
        schema_version,
        stream: stream_from_parts(&stream_kind, stream_id)?,
        payload,
    })
}

const TITLE_HISTORY_PAGE_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::TitleRematched,
    DomainEventType::ReleaseGrabbed,
    DomainEventType::ImportCompleted,
    DomainEventType::ImportRejected,
    DomainEventType::DownloadFailed,
    DomainEventType::ReleaseBlocklisted,
    DomainEventType::MediaFileDeleted,
    DomainEventType::MediaFileRenamed,
];

fn push_title_history_page_filters<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    event_types: Option<&[TitleHistoryEventType]>,
    title_ids: Option<&'a [String]>,
    download_id: Option<&'a str>,
) {
    let mut has_where = false;
    let mut push_where = |builder: &mut QueryBuilder<'_, Sqlite>| {
        if has_where {
            builder.push(" AND ");
        } else {
            builder.push(" WHERE ");
            has_where = true;
        }
    };

    push_where(builder);
    builder.push("title_id IS NOT NULL");

    match event_types {
        None => {
            push_where(builder);
            builder.push("event_type IN (");
            let mut separated = builder.separated(", ");
            for event_type in TITLE_HISTORY_PAGE_DOMAIN_EVENT_TYPES {
                separated.push_bind(event_type.as_str());
            }
            separated.push_unseparated(")");
        }
        Some([]) => {
            push_where(builder);
            builder.push("0");
        }
        Some(event_types) => {
            push_where(builder);
            builder.push("(");
            for (index, event_type) in event_types.iter().enumerate() {
                if index > 0 {
                    builder.push(" OR ");
                }
                match event_type {
                    TitleHistoryEventType::Grabbed => {
                        builder
                            .push("event_type = ")
                            .push_bind(DomainEventType::ReleaseGrabbed.as_str());
                    }
                    TitleHistoryEventType::DownloadFailed => {
                        builder
                            .push("event_type = ")
                            .push_bind(DomainEventType::DownloadFailed.as_str());
                    }
                    TitleHistoryEventType::Blocklisted => {
                        builder
                            .push("event_type = ")
                            .push_bind(DomainEventType::ReleaseBlocklisted.as_str());
                    }
                    TitleHistoryEventType::Imported => {
                        builder
                            .push("event_type = ")
                            .push_bind(DomainEventType::ImportCompleted.as_str());
                    }
                    TitleHistoryEventType::ImportFailed => {
                        builder
                            .push("(event_type = ")
                            .push_bind(DomainEventType::ImportRejected.as_str())
                            .push(" AND json_extract(payload_json, '$.data.status') = ")
                            .push_bind(ImportStatus::Failed.as_str())
                            .push(")");
                    }
                    TitleHistoryEventType::ImportSkipped => {
                        builder
                            .push("(event_type = ")
                            .push_bind(DomainEventType::ImportRejected.as_str())
                            .push(" AND json_extract(payload_json, '$.data.status') = ")
                            .push_bind(ImportStatus::Skipped.as_str())
                            .push(")");
                    }
                    TitleHistoryEventType::FileDeleted => {
                        builder
                            .push("event_type = ")
                            .push_bind(DomainEventType::MediaFileDeleted.as_str());
                    }
                    TitleHistoryEventType::FileRenamed => {
                        builder
                            .push("event_type = ")
                            .push_bind(DomainEventType::MediaFileRenamed.as_str());
                    }
                    TitleHistoryEventType::Rematched => {
                        builder
                            .push("event_type = ")
                            .push_bind(DomainEventType::TitleRematched.as_str());
                    }
                    TitleHistoryEventType::DownloadCompleted
                    | TitleHistoryEventType::DownloadIgnored => {
                        builder.push("0");
                    }
                }
            }
            builder.push(")");
        }
    }

    if let Some(title_ids) = title_ids {
        if title_ids.is_empty() {
            push_where(builder);
            builder.push("0");
        } else if title_ids.len() == 1 {
            push_where(builder);
            builder.push("title_id = ").push_bind(&title_ids[0]);
        } else {
            push_where(builder);
            builder.push("title_id IN (");
            let mut separated = builder.separated(", ");
            for title_id in title_ids {
                separated.push_bind(title_id);
            }
            separated.push_unseparated(")");
        }
    }

    if let Some(download_id) = download_id {
        push_where(builder);
        builder
            .push("json_extract(payload_json, '$.data.download_id') = ")
            .push_bind(download_id);
    }
}

pub(crate) async fn count_title_history_page_events_query(
    pool: &SqlitePool,
    event_types: Option<&[TitleHistoryEventType]>,
    title_ids: Option<&[String]>,
    download_id: Option<&str>,
) -> AppResult<i64> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM domain_events");
    push_title_history_page_filters(&mut builder, event_types, title_ids, download_id);

    builder
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))
}

pub(crate) async fn list_title_history_page_events_query(
    pool: &SqlitePool,
    event_types: Option<&[TitleHistoryEventType]>,
    title_ids: Option<&[String]>,
    download_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> AppResult<Vec<DomainEvent>> {
    let page_size = if limit == 0 { 50 } else { limit.min(500) };
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT sequence, event_id, occurred_at, actor_user_id, title_id, facet, correlation_id,
                causation_id, schema_version, stream_kind, stream_id, payload_json
         FROM domain_events",
    );
    push_title_history_page_filters(&mut builder, event_types, title_ids, download_id);
    builder.push(" ORDER BY sequence DESC");
    builder.push(" LIMIT ").push_bind(page_size as i64);
    builder.push(" OFFSET ").push_bind(offset as i64);

    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    rows.iter().map(row_to_domain_event).collect()
}

pub(crate) async fn get_domain_event_by_sequence_tx(
    tx: &mut Transaction<'_, Sqlite>,
    sequence: i64,
) -> AppResult<Option<DomainEvent>> {
    let row = sqlx::query(
        "SELECT sequence, event_id, occurred_at, actor_user_id, title_id, facet, correlation_id,
                causation_id, schema_version, stream_kind, stream_id, payload_json
         FROM domain_events
         WHERE sequence = ?",
    )
    .bind(sequence)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.as_ref().map(row_to_domain_event).transpose()
}

pub(crate) async fn append_domain_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &NewDomainEvent,
) -> AppResult<DomainEvent> {
    let payload_json = serde_json::to_string(&event.payload)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let result = sqlx::query(
        "INSERT INTO domain_events (
            event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id,
            schema_version, stream_kind, stream_id, event_type, payload_json
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&event.event_id)
    .bind(event.occurred_at.to_rfc3339())
    .bind(&event.actor_user_id)
    .bind(&event.title_id)
    .bind(event.facet.as_ref().map(MediaFacet::as_str))
    .bind(&event.correlation_id)
    .bind(&event.causation_id)
    .bind(event.schema_version)
    .bind(event.stream.kind())
    .bind(event.stream.identifier())
    .bind(event.payload.event_type().as_str())
    .bind(payload_json)
    .execute(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let sequence = result.last_insert_rowid();
    get_domain_event_by_sequence_tx(tx, sequence)
        .await?
        .ok_or_else(|| AppError::Repository("failed to reload inserted domain event".into()))
}

pub(crate) async fn append_domain_event_query(
    pool: &SqlitePool,
    event: &NewDomainEvent,
) -> AppResult<DomainEvent> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let stored = append_domain_event_tx(&mut tx, event).await?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(stored)
}

pub(crate) async fn append_domain_events_query(
    pool: &SqlitePool,
    events: &[NewDomainEvent],
) -> AppResult<Vec<DomainEvent>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let mut sequences = Vec::with_capacity(events.len());

    for event in events {
        let payload_json = serde_json::to_string(&event.payload)
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let result = sqlx::query(
            "INSERT INTO domain_events (
                event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id,
                schema_version, stream_kind, stream_id, event_type, payload_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&event.event_id)
        .bind(event.occurred_at.to_rfc3339())
        .bind(&event.actor_user_id)
        .bind(&event.title_id)
        .bind(event.facet.as_ref().map(MediaFacet::as_str))
        .bind(&event.correlation_id)
        .bind(&event.causation_id)
        .bind(event.schema_version)
        .bind(event.stream.kind())
        .bind(event.stream.identifier())
        .bind(event.payload.event_type().as_str())
        .bind(payload_json)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
        sequences.push(result.last_insert_rowid());
    }

    let mut stored = Vec::with_capacity(sequences.len());
    for sequence in sequences {
        stored.push(
            get_domain_event_by_sequence_tx(&mut tx, sequence)
                .await?
                .ok_or_else(|| {
                    AppError::Repository("failed to reload inserted domain event batch".into())
                })?,
        );
    }
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(stored)
}

pub(crate) async fn list_domain_events_query(
    pool: &SqlitePool,
    filter: &DomainEventFilter,
) -> AppResult<Vec<DomainEvent>> {
    let limit = if filter.limit == 0 {
        100
    } else {
        filter.limit.min(500)
    };
    let mut builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT sequence, event_id, occurred_at, actor_user_id, title_id, facet, correlation_id,
                causation_id, schema_version, stream_kind, stream_id, payload_json
         FROM domain_events",
    );

    let mut has_where = false;
    let mut push_where = |builder: &mut QueryBuilder<'_, Sqlite>| {
        if has_where {
            builder.push(" AND ");
        } else {
            builder.push(" WHERE ");
            has_where = true;
        }
    };

    if let Some(event_types) = filter.event_types.as_ref()
        && !event_types.is_empty()
    {
        push_where(&mut builder);
        builder.push("event_type IN (");
        let mut separated = builder.separated(", ");
        for event_type in event_types {
            separated.push_bind(event_type.as_str());
        }
        separated.push_unseparated(")");
    }

    if let Some(title_id) = filter.title_id.as_ref() {
        push_where(&mut builder);
        builder.push("title_id = ").push_bind(title_id);
    }

    if let Some(facet) = filter.facet.as_ref() {
        push_where(&mut builder);
        builder.push("facet = ").push_bind(facet.as_str());
    }

    if let Some(after_sequence) = filter.after_sequence {
        push_where(&mut builder);
        builder.push("sequence > ").push_bind(after_sequence);
    }

    if let Some(before_sequence) = filter.before_sequence {
        push_where(&mut builder);
        builder.push("sequence < ").push_bind(before_sequence);
    }

    if filter.after_sequence.is_some() && filter.before_sequence.is_none() {
        builder.push(" ORDER BY sequence ASC");
    } else {
        builder.push(" ORDER BY sequence DESC");
    }
    builder.push(" LIMIT ").push_bind(limit as i64);

    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    rows.iter().map(row_to_domain_event).collect()
}

pub(crate) async fn list_domain_events_after_sequence_query(
    pool: &SqlitePool,
    after_sequence: i64,
    limit: usize,
) -> AppResult<Vec<DomainEvent>> {
    let filter = DomainEventFilter {
        after_sequence: Some(after_sequence),
        limit,
        ..DomainEventFilter::default()
    };
    list_domain_events_query(pool, &filter).await
}

pub(crate) async fn delete_domain_events_for_title_ids_query(
    pool: &SqlitePool,
    title_ids: &[String],
) -> AppResult<u32> {
    if title_ids.is_empty() {
        return Ok(0);
    }

    let mut builder = QueryBuilder::<Sqlite>::new("DELETE FROM domain_events WHERE title_id IN (");
    let mut separated = builder.separated(", ");
    for title_id in title_ids {
        separated.push_bind(title_id);
    }
    separated.push_unseparated(")");

    let result = builder
        .build()
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn get_event_subscriber_offset_query(
    pool: &SqlitePool,
    subscriber: &str,
) -> AppResult<i64> {
    let offset = sqlx::query_scalar::<_, i64>(
        "SELECT sequence FROM event_subscriber_offsets WHERE subscriber_name = ?",
    )
    .bind(subscriber)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(offset.unwrap_or(0))
}

pub(crate) async fn set_event_subscriber_offset_query(
    pool: &SqlitePool,
    subscriber: &str,
    sequence: i64,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO event_subscriber_offsets (subscriber_name, sequence, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(subscriber_name) DO UPDATE SET
            sequence = excluded.sequence,
            updated_at = excluded.updated_at",
    )
    .bind(subscriber)
    .bind(sequence)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}
