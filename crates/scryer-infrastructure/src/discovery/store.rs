use async_trait::async_trait;
use scryer_application::{
    AppResult, DiscoveryFacetRecord, DiscoveryItemRecord, DiscoveryPendingContextChangeRecord,
    DiscoveryRawPageRecord, DiscoveryRepository, DiscoverySectionRecord,
    DiscoverySubmittedSubjectRecord, DiscoverySyncRunRecord, DiscoverySyncStateRecord,
};
use serde_json::Value as JsonValue;

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use crate::storage::sql::json::opt_json_text;

const DISCOVERY_SYNC_STATE_COLUMNS: &str = "scope_key, last_success_generation_id,
    last_public_feed_generation_id, last_subject_fingerprint,
    last_context_snapshot_completed_at, last_incremental_reload_completed_at,
    last_public_feed_completed_at, dirty_since, dirty_reason_mask, bootstrap_started_at,
    bootstrap_quiet_until, next_context_snapshot_eligible_at,
    next_incremental_reload_eligible_at, next_public_feed_eligible_at, backoff_until,
    startup_jitter_seconds, context_jitter_seconds, incremental_reload_jitter_seconds,
    public_feed_jitter_seconds, last_seen_domain_event_sequence,
    inflight_subject_fingerprint, inflight_domain_event_sequence, updated_at";

const DISCOVERY_SYNC_RUN_COLUMNS: &str = "id, kind, status, trigger_source, region, language,
    subject_count, subject_fingerprint, previous_subject_fingerprint, base_generation_id,
    changed_subject_count, affected_target_count, smg_request_id, smg_status,
    discovery_index_watermark, page_count, item_count, facet_count, raw_submit_json,
    raw_changes_json, raw_final_status_json, raw_ack_json, error_text, started_at, completed_at,
    created_at, updated_at";

const PENDING_CONTEXT_CHANGE_COLUMNS: &str = "id, scope_key, subject_key, previous_subject_key,
    change_type, title_id, previous_title_id, library_facet, raw_subject_json,
    raw_previous_subject_json, first_seen_sequence, last_seen_sequence, first_seen_at,
    last_seen_at";

const SECTION_COLUMNS: &[&str] = &[
    "id",
    "run_id",
    "section_id",
    "section_type",
    "surface",
    "title",
    "source_signals_json",
    "facets_json",
    "sort_index",
    "raw_json",
    "created_at",
    "updated_at",
];

const ITEM_COLUMNS: &[&str] = &[
    "id",
    "run_id",
    "base_generation_id",
    "source_run_kind",
    "section_id",
    "target_key",
    "target_kind",
    "resolved",
    "resolved_title_id",
    "display_title",
    "original_title",
    "sort_title",
    "year",
    "poster_path",
    "poster_url",
    "background_url",
    "overview",
    "content_type",
    "genres_json",
    "rating",
    "rating_sources_json",
    "status_tags_json",
    "source_tags_json",
    "sources_json",
    "best_source",
    "relation_types_json",
    "relation_subtypes_json",
    "chart_signals_json",
    "provider_signals_json",
    "rank_components_json",
    "source_count",
    "edge_count",
    "relation_count",
    "source_subject_count",
    "rank_score",
    "matched_subject_keys_json",
    "matched_subject_titles_json",
    "matched_subject_count",
    "tmdb_collection_id",
    "tmdb_collection_name",
    "owned_in_input",
    "facet_terms_json",
    "context_terms_json",
    "change_subject_keys_json",
    "removed_subject_keys_json",
    "tombstoned_by_run_id",
    "tombstoned_at",
    "raw_json",
    "created_at",
    "updated_at",
];

const FACET_COLUMNS: &[&str] = &[
    "run_id",
    "facet_name",
    "facet_value",
    "smg_count",
    "local_count",
    "raw_json",
];

#[derive(Clone)]
pub struct DiscoveryStore {
    datastore: StoreDatastore,
}

impl DiscoveryStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl DiscoveryRepository for DiscoveryStore {
    async fn get_discovery_sync_state(
        &self,
        scope_key: &str,
    ) -> AppResult<Option<DiscoverySyncStateRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!("SELECT {DISCOVERY_SYNC_STATE_COLUMNS} FROM discovery_sync_state WHERE scope_key = {{}}"),
            &[SqlArg::Text(scope_key.to_string())],
        )
        .await?;
        row.as_ref().map(sync_state_from_row).transpose()
    }

    async fn upsert_discovery_sync_state(&self, state: &DiscoverySyncStateRecord) -> AppResult<()> {
        let args = sync_state_args(state);
        SqlRuntime::execute(
            self.datastore.read_exec(),
            &upsert_sql(
                "discovery_sync_state",
                &split_columns(DISCOVERY_SYNC_STATE_COLUMNS),
                &["scope_key"],
            ),
            &args,
        )
        .await?;
        Ok(())
    }

    async fn get_discovery_sync_run(&self, id: &str) -> AppResult<Option<DiscoverySyncRunRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DISCOVERY_SYNC_RUN_COLUMNS} FROM discovery_sync_runs WHERE id = {{}}"
            ),
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.as_ref().map(sync_run_from_row).transpose()
    }

    async fn upsert_discovery_sync_run(&self, run: &DiscoverySyncRunRecord) -> AppResult<()> {
        let columns = split_columns(DISCOVERY_SYNC_RUN_COLUMNS);
        SqlRuntime::execute(
            self.datastore.read_exec(),
            &upsert_sql("discovery_sync_runs", &columns, &["id"]),
            &sync_run_args(&self.datastore, run)?,
        )
        .await?;
        Ok(())
    }

    async fn insert_discovery_raw_page(&self, page: &DiscoveryRawPageRecord) -> AppResult<()> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "INSERT INTO discovery_raw_pages
             (run_id, payload_kind, page_number, compression, raw_payload, created_at)
             VALUES ({}, {}, {}, {}, {}, {})
             ON CONFLICT(run_id, payload_kind, page_number) DO UPDATE SET
                compression = excluded.compression,
                raw_payload = excluded.raw_payload,
                created_at = excluded.created_at",
            &[
                SqlArg::Text(page.run_id.clone()),
                SqlArg::Text(page.payload_kind.clone()),
                SqlArg::I32(page.page_number),
                SqlArg::Text(page.compression.clone()),
                SqlArg::Text(page.raw_payload.clone()),
                SqlArg::Timestamp(page.created_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn replace_discovery_submitted_subjects(
        &self,
        run_id: &str,
        subjects: &[DiscoverySubmittedSubjectRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let run_id = run_id.to_string();
        let subjects = subjects.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_subjects", move |tx| {
            let datastore = datastore.clone();
            let run_id = run_id.clone();
            let subjects = subjects.clone();
            Box::pin(async move {
                delete_for_run_tx(tx, "discovery_submitted_subjects", &run_id).await?;
                for subject in &subjects {
                    insert_submitted_subject_tx(tx, &datastore, subject).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn upsert_pending_discovery_context_change(
        &self,
        change: &DiscoveryPendingContextChangeRecord,
    ) -> AppResult<()> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            &upsert_sql(
                "discovery_pending_context_changes",
                &split_columns(PENDING_CONTEXT_CHANGE_COLUMNS),
                &["id"],
            ),
            &pending_context_change_args(&self.datastore, change)?,
        )
        .await?;
        Ok(())
    }

    async fn list_pending_discovery_context_changes(
        &self,
        scope_key: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {PENDING_CONTEXT_CHANGE_COLUMNS}
                 FROM discovery_pending_context_changes
                 WHERE scope_key = {{}}
                 ORDER BY last_seen_at ASC, id ASC
                 LIMIT {{}}"
            ),
            &[SqlArg::Text(scope_key.to_string()), SqlArg::I64(limit)],
        )
        .await?;
        rows.iter().map(pending_context_change_from_row).collect()
    }

    async fn clear_pending_discovery_context_changes_through_sequence(
        &self,
        scope_key: &str,
        last_seen_sequence: i64,
    ) -> AppResult<u64> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "DELETE FROM discovery_pending_context_changes
             WHERE scope_key = {}
               AND last_seen_sequence IS NOT NULL
               AND last_seen_sequence <= {}",
            &[
                SqlArg::Text(scope_key.to_string()),
                SqlArg::I64(last_seen_sequence),
            ],
        )
        .await
    }

    async fn replace_discovery_sections(
        &self,
        run_id: &str,
        sections: &[DiscoverySectionRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let run_id = run_id.to_string();
        let sections = sections.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_sections", move |tx| {
            let datastore = datastore.clone();
            let run_id = run_id.clone();
            let sections = sections.clone();
            Box::pin(async move {
                delete_for_run_tx(tx, "discovery_sections", &run_id).await?;
                for section in &sections {
                    insert_section_tx(tx, &datastore, section).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn replace_discovery_items(
        &self,
        run_id: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let run_id = run_id.to_string();
        let items = items.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_items", move |tx| {
            let datastore = datastore.clone();
            let run_id = run_id.clone();
            let items = items.clone();
            Box::pin(async move {
                delete_for_run_tx(tx, "discovery_items", &run_id).await?;
                for item in &items {
                    insert_item_tx(tx, &datastore, item).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn replace_discovery_facets(
        &self,
        run_id: &str,
        facets: &[DiscoveryFacetRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let run_id = run_id.to_string();
        let facets = facets.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_facets", move |tx| {
            let datastore = datastore.clone();
            let run_id = run_id.clone();
            let facets = facets.clone();
            Box::pin(async move {
                delete_for_run_tx(tx, "discovery_facets", &run_id).await?;
                for facet in &facets {
                    insert_facet_tx(tx, &datastore, facet).await?;
                }
                Ok(())
            })
        })
        .await
    }
}

fn split_columns(columns: &str) -> Vec<&str> {
    columns
        .split(',')
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .collect()
}

fn placeholders(count: usize) -> String {
    (0..count).map(|_| "{}").collect::<Vec<_>>().join(", ")
}

fn upsert_sql(table: &str, columns: &[&str], conflict_columns: &[&str]) -> String {
    let updates = columns
        .iter()
        .filter(|column| !conflict_columns.contains(column))
        .map(|column| format!("{column} = excluded.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {table} ({}) VALUES ({})
         ON CONFLICT({}) DO UPDATE SET {updates}",
        columns.join(", "),
        placeholders(columns.len()),
        conflict_columns.join(", ")
    )
}

async fn delete_for_run_tx(tx: &mut SqlTx<'_>, table: &'static str, run_id: &str) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &format!("DELETE FROM {table} WHERE run_id = {{}}"),
        &[SqlArg::Text(run_id.to_string())],
    )
    .await?;
    Ok(())
}

fn sync_state_args(state: &DiscoverySyncStateRecord) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(state.scope_key.clone()),
        SqlArg::OptText(state.last_success_generation_id.clone()),
        SqlArg::OptText(state.last_public_feed_generation_id.clone()),
        SqlArg::OptText(state.last_subject_fingerprint.clone()),
        SqlArg::OptTimestamp(state.last_context_snapshot_completed_at),
        SqlArg::OptTimestamp(state.last_incremental_reload_completed_at),
        SqlArg::OptTimestamp(state.last_public_feed_completed_at),
        SqlArg::OptTimestamp(state.dirty_since),
        SqlArg::I64(state.dirty_reason_mask),
        SqlArg::OptTimestamp(state.bootstrap_started_at),
        SqlArg::OptTimestamp(state.bootstrap_quiet_until),
        SqlArg::OptTimestamp(state.next_context_snapshot_eligible_at),
        SqlArg::OptTimestamp(state.next_incremental_reload_eligible_at),
        SqlArg::OptTimestamp(state.next_public_feed_eligible_at),
        SqlArg::OptTimestamp(state.backoff_until),
        SqlArg::I64(state.startup_jitter_seconds),
        SqlArg::I64(state.context_jitter_seconds),
        SqlArg::I64(state.incremental_reload_jitter_seconds),
        SqlArg::I64(state.public_feed_jitter_seconds),
        SqlArg::OptI64(state.last_seen_domain_event_sequence),
        SqlArg::OptText(state.inflight_subject_fingerprint.clone()),
        SqlArg::OptI64(state.inflight_domain_event_sequence),
        SqlArg::Timestamp(state.updated_at),
    ]
}

fn sync_state_from_row(row: &SqlRow) -> AppResult<DiscoverySyncStateRecord> {
    Ok(DiscoverySyncStateRecord {
        scope_key: row.text("scope_key")?,
        last_success_generation_id: row.opt_text("last_success_generation_id")?,
        last_public_feed_generation_id: row.opt_text("last_public_feed_generation_id")?,
        last_subject_fingerprint: row.opt_text("last_subject_fingerprint")?,
        last_context_snapshot_completed_at: row
            .opt_timestamp("last_context_snapshot_completed_at")?,
        last_incremental_reload_completed_at: row
            .opt_timestamp("last_incremental_reload_completed_at")?,
        last_public_feed_completed_at: row.opt_timestamp("last_public_feed_completed_at")?,
        dirty_since: row.opt_timestamp("dirty_since")?,
        dirty_reason_mask: row.i64("dirty_reason_mask")?,
        bootstrap_started_at: row.opt_timestamp("bootstrap_started_at")?,
        bootstrap_quiet_until: row.opt_timestamp("bootstrap_quiet_until")?,
        next_context_snapshot_eligible_at: row
            .opt_timestamp("next_context_snapshot_eligible_at")?,
        next_incremental_reload_eligible_at: row
            .opt_timestamp("next_incremental_reload_eligible_at")?,
        next_public_feed_eligible_at: row.opt_timestamp("next_public_feed_eligible_at")?,
        backoff_until: row.opt_timestamp("backoff_until")?,
        startup_jitter_seconds: row.i64("startup_jitter_seconds")?,
        context_jitter_seconds: row.i64("context_jitter_seconds")?,
        incremental_reload_jitter_seconds: row.i64("incremental_reload_jitter_seconds")?,
        public_feed_jitter_seconds: row.i64("public_feed_jitter_seconds")?,
        last_seen_domain_event_sequence: row.opt_i64("last_seen_domain_event_sequence")?,
        inflight_subject_fingerprint: row.opt_text("inflight_subject_fingerprint")?,
        inflight_domain_event_sequence: row.opt_i64("inflight_domain_event_sequence")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn sync_run_args(
    datastore: &StoreDatastore,
    run: &DiscoverySyncRunRecord,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(run.id.clone()),
        SqlArg::Text(run.kind.clone()),
        SqlArg::Text(run.status.clone()),
        SqlArg::Text(run.trigger_source.clone()),
        SqlArg::Text(run.region.clone()),
        SqlArg::Text(run.language.clone()),
        SqlArg::I64(run.subject_count),
        SqlArg::OptText(run.subject_fingerprint.clone()),
        SqlArg::OptText(run.previous_subject_fingerprint.clone()),
        SqlArg::OptText(run.base_generation_id.clone()),
        SqlArg::I64(run.changed_subject_count),
        SqlArg::I64(run.affected_target_count),
        SqlArg::OptText(run.smg_request_id.clone()),
        SqlArg::OptText(run.smg_status.clone()),
        SqlArg::OptText(run.discovery_index_watermark.clone()),
        SqlArg::OptI32(run.page_count),
        SqlArg::OptI64(run.item_count),
        SqlArg::OptI64(run.facet_count),
        opt_json_arg(datastore, run.raw_submit_json.as_deref())?,
        opt_json_arg(datastore, run.raw_changes_json.as_deref())?,
        opt_json_arg(datastore, run.raw_final_status_json.as_deref())?,
        opt_json_arg(datastore, run.raw_ack_json.as_deref())?,
        SqlArg::OptText(run.error_text.clone()),
        SqlArg::OptTimestamp(run.started_at),
        SqlArg::OptTimestamp(run.completed_at),
        SqlArg::Timestamp(run.created_at),
        SqlArg::Timestamp(run.updated_at),
    ])
}

fn sync_run_from_row(row: &SqlRow) -> AppResult<DiscoverySyncRunRecord> {
    Ok(DiscoverySyncRunRecord {
        id: row.text("id")?,
        kind: row.text("kind")?,
        status: row.text("status")?,
        trigger_source: row.text("trigger_source")?,
        region: row.text("region")?,
        language: row.text("language")?,
        subject_count: row.i64("subject_count")?,
        subject_fingerprint: row.opt_text("subject_fingerprint")?,
        previous_subject_fingerprint: row.opt_text("previous_subject_fingerprint")?,
        base_generation_id: row.opt_text("base_generation_id")?,
        changed_subject_count: row.i64("changed_subject_count")?,
        affected_target_count: row.i64("affected_target_count")?,
        smg_request_id: row.opt_text("smg_request_id")?,
        smg_status: row.opt_text("smg_status")?,
        discovery_index_watermark: row.opt_text("discovery_index_watermark")?,
        page_count: row.opt_i32("page_count")?,
        item_count: row.opt_i64("item_count")?,
        facet_count: row.opt_i64("facet_count")?,
        raw_submit_json: opt_json_text(row, "raw_submit_json")?,
        raw_changes_json: opt_json_text(row, "raw_changes_json")?,
        raw_final_status_json: opt_json_text(row, "raw_final_status_json")?,
        raw_ack_json: opt_json_text(row, "raw_ack_json")?,
        error_text: row.opt_text("error_text")?,
        started_at: row.opt_timestamp("started_at")?,
        completed_at: row.opt_timestamp("completed_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn pending_context_change_args(
    datastore: &StoreDatastore,
    change: &DiscoveryPendingContextChangeRecord,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(change.id.clone()),
        SqlArg::Text(change.scope_key.clone()),
        SqlArg::OptText(change.subject_key.clone()),
        SqlArg::OptText(change.previous_subject_key.clone()),
        SqlArg::Text(change.change_type.clone()),
        SqlArg::OptText(change.title_id.clone()),
        SqlArg::OptText(change.previous_title_id.clone()),
        SqlArg::OptText(change.library_facet.clone()),
        opt_json_arg(datastore, change.raw_subject_json.as_deref())?,
        opt_json_arg(datastore, change.raw_previous_subject_json.as_deref())?,
        SqlArg::OptI64(change.first_seen_sequence),
        SqlArg::OptI64(change.last_seen_sequence),
        SqlArg::Timestamp(change.first_seen_at),
        SqlArg::Timestamp(change.last_seen_at),
    ])
}

fn pending_context_change_from_row(row: &SqlRow) -> AppResult<DiscoveryPendingContextChangeRecord> {
    Ok(DiscoveryPendingContextChangeRecord {
        id: row.text("id")?,
        scope_key: row.text("scope_key")?,
        subject_key: row.opt_text("subject_key")?,
        previous_subject_key: row.opt_text("previous_subject_key")?,
        change_type: row.text("change_type")?,
        title_id: row.opt_text("title_id")?,
        previous_title_id: row.opt_text("previous_title_id")?,
        library_facet: row.opt_text("library_facet")?,
        raw_subject_json: opt_json_text(row, "raw_subject_json")?,
        raw_previous_subject_json: opt_json_text(row, "raw_previous_subject_json")?,
        first_seen_sequence: row.opt_i64("first_seen_sequence")?,
        last_seen_sequence: row.opt_i64("last_seen_sequence")?,
        first_seen_at: row.timestamp("first_seen_at")?,
        last_seen_at: row.timestamp("last_seen_at")?,
    })
}

async fn insert_submitted_subject_tx(
    tx: &mut SqlTx<'_>,
    datastore: &StoreDatastore,
    subject: &DiscoverySubmittedSubjectRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_submitted_subjects
         (run_id, subject_key, title_id, library_facet, title_kind, display_title,
          external_ids_json, raw_subject_json)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(subject.run_id.clone()),
            SqlArg::Text(subject.subject_key.clone()),
            SqlArg::OptText(subject.title_id.clone()),
            SqlArg::OptText(subject.library_facet.clone()),
            SqlArg::OptText(subject.title_kind.clone()),
            SqlArg::OptText(subject.display_title.clone()),
            json_arg(datastore, &subject.external_ids_json)?,
            json_arg(datastore, &subject.raw_subject_json)?,
        ],
    )
    .await?;
    Ok(())
}

async fn insert_section_tx(
    tx: &mut SqlTx<'_>,
    datastore: &StoreDatastore,
    section: &DiscoverySectionRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &insert_sql("discovery_sections", SECTION_COLUMNS),
        &[
            SqlArg::Text(section.id.clone()),
            SqlArg::Text(section.run_id.clone()),
            SqlArg::Text(section.section_id.clone()),
            SqlArg::Text(section.section_type.clone()),
            SqlArg::Text(section.surface.clone()),
            SqlArg::Text(section.title.clone()),
            json_arg(datastore, &section.source_signals_json)?,
            json_arg(datastore, &section.facets_json)?,
            SqlArg::I32(section.sort_index),
            json_arg(datastore, &section.raw_json)?,
            SqlArg::Timestamp(section.created_at),
            SqlArg::Timestamp(section.updated_at),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_item_tx(
    tx: &mut SqlTx<'_>,
    datastore: &StoreDatastore,
    item: &DiscoveryItemRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &insert_sql("discovery_items", ITEM_COLUMNS),
        &item_args(datastore, item)?,
    )
    .await?;
    Ok(())
}

async fn insert_facet_tx(
    tx: &mut SqlTx<'_>,
    datastore: &StoreDatastore,
    facet: &DiscoveryFacetRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &insert_sql("discovery_facets", FACET_COLUMNS),
        &[
            SqlArg::Text(facet.run_id.clone()),
            SqlArg::Text(facet.facet_name.clone()),
            SqlArg::Text(facet.facet_value.clone()),
            SqlArg::OptI64(facet.smg_count),
            SqlArg::OptI64(facet.local_count),
            json_arg(datastore, &facet.raw_json)?,
        ],
    )
    .await?;
    Ok(())
}

fn insert_sql(table: &str, columns: &[&str]) -> String {
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        columns.join(", "),
        placeholders(columns.len())
    )
}

fn item_args(datastore: &StoreDatastore, item: &DiscoveryItemRecord) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(item.id.clone()),
        SqlArg::Text(item.run_id.clone()),
        SqlArg::OptText(item.base_generation_id.clone()),
        SqlArg::Text(item.source_run_kind.clone()),
        SqlArg::OptText(item.section_id.clone()),
        SqlArg::Text(item.target_key.clone()),
        SqlArg::Text(item.target_kind.clone()),
        SqlArg::Bool(item.resolved),
        SqlArg::OptText(item.resolved_title_id.clone()),
        SqlArg::Text(item.display_title.clone()),
        SqlArg::OptText(item.original_title.clone()),
        SqlArg::OptText(item.sort_title.clone()),
        SqlArg::OptI32(item.year),
        SqlArg::OptText(item.poster_path.clone()),
        SqlArg::OptText(item.poster_url.clone()),
        SqlArg::OptText(item.background_url.clone()),
        SqlArg::OptText(item.overview.clone()),
        SqlArg::OptText(item.content_type.clone()),
        json_arg(datastore, &item.genres_json)?,
        SqlArg::OptF64(item.rating),
        json_arg(datastore, &item.rating_sources_json)?,
        json_arg(datastore, &item.status_tags_json)?,
        json_arg(datastore, &item.source_tags_json)?,
        json_arg(datastore, &item.sources_json)?,
        SqlArg::OptText(item.best_source.clone()),
        json_arg(datastore, &item.relation_types_json)?,
        json_arg(datastore, &item.relation_subtypes_json)?,
        json_arg(datastore, &item.chart_signals_json)?,
        json_arg(datastore, &item.provider_signals_json)?,
        json_arg(datastore, &item.rank_components_json)?,
        SqlArg::OptI32(item.source_count),
        SqlArg::OptI32(item.edge_count),
        SqlArg::OptI32(item.relation_count),
        SqlArg::OptI32(item.source_subject_count),
        SqlArg::OptF64(item.rank_score),
        json_arg(datastore, &item.matched_subject_keys_json)?,
        json_arg(datastore, &item.matched_subject_titles_json)?,
        SqlArg::I32(item.matched_subject_count),
        SqlArg::OptText(item.tmdb_collection_id.clone()),
        SqlArg::OptText(item.tmdb_collection_name.clone()),
        SqlArg::Bool(item.owned_in_input),
        json_arg(datastore, &item.facet_terms_json)?,
        json_arg(datastore, &item.context_terms_json)?,
        json_arg(datastore, &item.change_subject_keys_json)?,
        json_arg(datastore, &item.removed_subject_keys_json)?,
        SqlArg::OptText(item.tombstoned_by_run_id.clone()),
        SqlArg::OptTimestamp(item.tombstoned_at),
        json_arg(datastore, &item.raw_json)?,
        SqlArg::Timestamp(item.created_at),
        SqlArg::Timestamp(item.updated_at),
    ])
}

fn json_arg(datastore: &StoreDatastore, raw: &str) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::Text(raw.to_string())),
        StoreDatastore::Postgres { .. } => serde_json::from_str::<JsonValue>(raw)
            .map(SqlArg::Json)
            .map_err(repo_err),
    }
}

fn opt_json_arg(datastore: &StoreDatastore, raw: Option<&str>) -> AppResult<SqlArg> {
    match (datastore, raw) {
        (StoreDatastore::Sqlite { .. }, Some(raw)) => Ok(SqlArg::OptText(Some(raw.to_string()))),
        (StoreDatastore::Sqlite { .. }, None) => Ok(SqlArg::OptText(None)),
        (StoreDatastore::Postgres { .. }, Some(raw)) => serde_json::from_str::<JsonValue>(raw)
            .map(|value| SqlArg::OptJson(Some(value)))
            .map_err(repo_err),
        (StoreDatastore::Postgres { .. }, None) => Ok(SqlArg::OptJson(None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_application::DISCOVERY_DEFAULT_SCOPE_KEY;
    use serde_json::json;

    use crate::storage::sqlite::services::SqliteServices;

    #[tokio::test]
    async fn sqlite_store_round_trips_state_run_and_pending_change() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();

        let state = DiscoverySyncStateRecord {
            dirty_since: Some(now),
            next_incremental_reload_eligible_at: Some(now),
            incremental_reload_jitter_seconds: 731,
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        store
            .upsert_discovery_sync_state(&state)
            .await
            .expect("state should upsert");

        let loaded_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load")
            .expect("state should exist");
        assert_eq!(loaded_state.scope_key, DISCOVERY_DEFAULT_SCOPE_KEY);
        assert_eq!(loaded_state.incremental_reload_jitter_seconds, 731);
        assert!(loaded_state.next_incremental_reload_eligible_at.is_some());

        let run = DiscoverySyncRunRecord {
            id: "run-1".to_string(),
            kind: "context_incremental".to_string(),
            status: "complete".to_string(),
            trigger_source: "scheduled_incremental".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 1,
            subject_fingerprint: Some("fingerprint-current".to_string()),
            previous_subject_fingerprint: Some("fingerprint-previous".to_string()),
            base_generation_id: None,
            changed_subject_count: 1,
            affected_target_count: 1,
            smg_request_id: None,
            smg_status: Some("COMPLETE".to_string()),
            discovery_index_watermark: Some("watermark".to_string()),
            page_count: None,
            item_count: Some(0),
            facet_count: Some(0),
            raw_submit_json: None,
            raw_changes_json: Some(json!({"status": "COMPLETE"}).to_string()),
            raw_final_status_json: None,
            raw_ack_json: None,
            error_text: None,
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        store
            .upsert_discovery_sync_run(&run)
            .await
            .expect("run should upsert");

        let loaded_run = store
            .get_discovery_sync_run("run-1")
            .await
            .expect("run should load")
            .expect("run should exist");
        assert_eq!(loaded_run.kind, "context_incremental");
        assert_eq!(loaded_run.smg_status.as_deref(), Some("COMPLETE"));
        assert_eq!(
            loaded_run.raw_changes_json.as_deref(),
            Some(r#"{"status":"COMPLETE"}"#)
        );

        store
            .insert_discovery_raw_page(&DiscoveryRawPageRecord {
                run_id: "run-1".to_string(),
                payload_kind: "context_changes".to_string(),
                page_number: 0,
                compression: "none".to_string(),
                raw_payload: json!({"items": []}).to_string(),
                created_at: now,
            })
            .await
            .expect("raw page should insert");
        store
            .replace_discovery_submitted_subjects(
                "run-1",
                &[DiscoverySubmittedSubjectRecord {
                    run_id: "run-1".to_string(),
                    subject_key: "tvdb:series:1".to_string(),
                    title_id: None,
                    library_facet: Some("series".to_string()),
                    title_kind: Some("series".to_string()),
                    display_title: Some("Example Series".to_string()),
                    external_ids_json: json!([{"source": "tvdb", "value": "1"}]).to_string(),
                    raw_subject_json: json!({"key": "tvdb:series:1"}).to_string(),
                }],
            )
            .await
            .expect("submitted subjects should replace");
        store
            .replace_discovery_sections(
                "run-1",
                &[DiscoverySectionRecord {
                    id: "section-row-1".to_string(),
                    run_id: "run-1".to_string(),
                    section_id: "for_you".to_string(),
                    section_type: "FOR_YOU".to_string(),
                    surface: "personalized".to_string(),
                    title: "For You".to_string(),
                    source_signals_json: "[]".to_string(),
                    facets_json: "[]".to_string(),
                    sort_index: 0,
                    raw_json: json!({"sectionId": "for_you"}).to_string(),
                    created_at: now,
                    updated_at: now,
                }],
            )
            .await
            .expect("sections should replace");
        store
            .replace_discovery_items(
                "run-1",
                &[DiscoveryItemRecord {
                    id: "item-row-1".to_string(),
                    run_id: "run-1".to_string(),
                    base_generation_id: Some("run-1".to_string()),
                    source_run_kind: "context_incremental".to_string(),
                    section_id: Some("for_you".to_string()),
                    target_key: "tmdb:movie:10".to_string(),
                    target_kind: "movie".to_string(),
                    resolved: true,
                    resolved_title_id: None,
                    display_title: "Example Movie".to_string(),
                    original_title: None,
                    sort_title: Some("Example Movie".to_string()),
                    year: Some(2026),
                    poster_path: None,
                    poster_url: None,
                    background_url: None,
                    overview: None,
                    content_type: Some("movie".to_string()),
                    genres_json: "[]".to_string(),
                    rating: Some(7.5),
                    rating_sources_json: "[]".to_string(),
                    status_tags_json: "[]".to_string(),
                    source_tags_json: "[]".to_string(),
                    sources_json: "[]".to_string(),
                    best_source: None,
                    relation_types_json: "[]".to_string(),
                    relation_subtypes_json: "[]".to_string(),
                    chart_signals_json: "[]".to_string(),
                    provider_signals_json: "[]".to_string(),
                    rank_components_json: "[]".to_string(),
                    source_count: Some(1),
                    edge_count: Some(1),
                    relation_count: Some(0),
                    source_subject_count: Some(1),
                    rank_score: Some(0.42),
                    matched_subject_keys_json: json!(["tvdb:series:1"]).to_string(),
                    matched_subject_titles_json: json!(["Example Series"]).to_string(),
                    matched_subject_count: 1,
                    tmdb_collection_id: None,
                    tmdb_collection_name: None,
                    owned_in_input: false,
                    facet_terms_json: "[]".to_string(),
                    context_terms_json: "[]".to_string(),
                    change_subject_keys_json: json!(["tvdb:series:1"]).to_string(),
                    removed_subject_keys_json: "[]".to_string(),
                    tombstoned_by_run_id: None,
                    tombstoned_at: None,
                    raw_json: json!({"targetKey": "tmdb:movie:10"}).to_string(),
                    created_at: now,
                    updated_at: now,
                }],
            )
            .await
            .expect("items should replace");
        store
            .replace_discovery_facets(
                "run-1",
                &[DiscoveryFacetRecord {
                    run_id: "run-1".to_string(),
                    facet_name: "genre".to_string(),
                    facet_value: "Drama".to_string(),
                    smg_count: Some(1),
                    local_count: Some(1),
                    raw_json: json!({"value": "Drama", "count": 1}).to_string(),
                }],
            )
            .await
            .expect("facets should replace");

        let change = DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tvdb:series:1".to_string()),
            previous_subject_key: None,
            change_type: "added".to_string(),
            title_id: None,
            previous_title_id: None,
            library_facet: Some("series".to_string()),
            raw_subject_json: Some(json!({"key": "tvdb:series:1"}).to_string()),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: now,
            last_seen_at: now,
        };
        store
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending change should upsert");

        let pending = store
            .list_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY, 10)
            .await
            .expect("pending changes should list");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].change_type, "added");
        assert_eq!(
            pending[0].raw_subject_json.as_deref(),
            Some(r#"{"key":"tvdb:series:1"}"#)
        );

        let deleted = store
            .clear_pending_discovery_context_changes_through_sequence(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                12,
            )
            .await
            .expect("pending changes should clear");
        assert_eq!(deleted, 1);

        let _ = std::fs::remove_file(db);
    }
}
