use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppResult, DiscoveryContextIncrementalCommit, DiscoveryContextSnapshotCommit,
    DiscoveryFacetRecord, DiscoveryItemRecord, DiscoveryPendingContextChangeRecord,
    DiscoveryPruneReport, DiscoveryPublicFeedCommit, DiscoveryRawPageRecord, DiscoveryRepository,
    DiscoverySectionRecord, DiscoverySubmittedSubjectRecord, DiscoverySyncRunRecord,
    DiscoverySyncStateRecord,
};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};

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
    transient_failure_count,
    startup_jitter_seconds, context_jitter_seconds, incremental_reload_jitter_seconds,
    public_feed_jitter_seconds, last_seen_domain_event_sequence,
    inflight_context_snapshot_run_id, inflight_subject_fingerprint,
    inflight_domain_event_sequence, lease_owner_id, lease_expires_at, updated_at";

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
    "library_provenance_json",
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

    async fn try_acquire_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute(
            self.datastore.read_exec(),
            "INSERT INTO discovery_sync_state
                (scope_key, lease_owner_id, lease_expires_at, updated_at)
             VALUES ({}, {}, {}, {})
             ON CONFLICT(scope_key) DO UPDATE SET
                lease_owner_id = excluded.lease_owner_id,
                lease_expires_at = excluded.lease_expires_at,
                updated_at = excluded.updated_at
             WHERE discovery_sync_state.lease_owner_id IS NULL
                OR discovery_sync_state.lease_expires_at IS NULL
                OR discovery_sync_state.lease_expires_at <= {}
                OR discovery_sync_state.lease_owner_id = {}",
            &[
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(owner_id.to_string()),
                SqlArg::Timestamp(lease_expires_at),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
                SqlArg::Text(owner_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn renew_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute(
            self.datastore.read_exec(),
            "UPDATE discovery_sync_state
             SET lease_expires_at = {}, updated_at = {}
             WHERE scope_key = {} AND lease_owner_id = {}",
            &[
                SqlArg::Timestamp(lease_expires_at),
                SqlArg::Timestamp(now),
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(owner_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn release_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "UPDATE discovery_sync_state
             SET lease_owner_id = NULL, lease_expires_at = NULL, updated_at = {}
             WHERE scope_key = {} AND lease_owner_id = {}",
            &[
                SqlArg::Timestamp(now),
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(owner_id.to_string()),
            ],
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

    async fn list_recent_discovery_sync_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DISCOVERY_SYNC_RUN_COLUMNS}
                 FROM discovery_sync_runs
                 ORDER BY COALESCE(completed_at, started_at, updated_at, created_at) DESC,
                          created_at DESC
                 LIMIT {{}}"
            ),
            &[SqlArg::I64(limit.clamp(1, 100))],
        )
        .await?;
        rows.iter().map(sync_run_from_row).collect()
    }

    async fn list_unacked_discovery_context_snapshot_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DISCOVERY_SYNC_RUN_COLUMNS}
                 FROM discovery_sync_runs
                 WHERE kind = 'context_snapshot'
                   AND status IN ('complete', 'warning')
                   AND smg_request_id IS NOT NULL
                   AND raw_ack_json IS NULL
                 ORDER BY COALESCE(completed_at, updated_at, created_at) ASC,
                          created_at ASC
                 LIMIT {{}}"
            ),
            &[SqlArg::I64(limit.clamp(1, 100))],
        )
        .await?;
        rows.iter().map(sync_run_from_row).collect()
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

    async fn commit_discovery_context_snapshot(
        &self,
        commit: &DiscoveryContextSnapshotCommit,
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let commit = commit.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "commit_discovery_context_snapshot",
            move |tx| {
                let datastore = datastore.clone();
                let commit = commit.clone();
                Box::pin(async move {
                    upsert_sync_run_tx(tx, &datastore, &commit.run).await?;
                    upsert_sync_state_tx(tx, &commit.state).await?;
                    delete_for_run_tx(tx, "discovery_raw_pages", &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_submitted_subjects", &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_items", &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_facets", &commit.run.id).await?;

                    for page in &commit.raw_pages {
                        insert_raw_page_tx(tx, page).await?;
                    }
                    for subject in &commit.submitted_subjects {
                        insert_submitted_subject_tx(tx, &datastore, subject).await?;
                    }
                    for item in &commit.items {
                        insert_item_tx(tx, &datastore, item).await?;
                    }
                    for facet in &commit.facets {
                        insert_facet_tx(tx, &datastore, facet).await?;
                    }
                    if let Some(sequence) = commit.clear_pending_through_sequence {
                        clear_pending_discovery_context_changes_tx(
                            tx,
                            &commit.state.scope_key,
                            sequence,
                        )
                        .await?;
                    }

                    Ok(())
                })
            },
        )
        .await
    }

    async fn commit_discovery_context_incremental(
        &self,
        commit: &DiscoveryContextIncrementalCommit,
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let commit = commit.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "commit_discovery_context_incremental",
            move |tx| {
                let datastore = datastore.clone();
                let commit = commit.clone();
                Box::pin(async move {
                    upsert_sync_run_tx(tx, &datastore, &commit.run).await?;
                    upsert_sync_state_tx(tx, &commit.state).await?;
                    delete_for_run_tx(tx, "discovery_raw_pages", &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_items", &commit.run.id).await?;
                    insert_raw_page_tx(tx, &commit.raw_changes).await?;
                    tombstone_discovery_items_tx(
                        tx,
                        commit.run.base_generation_id.as_deref(),
                        &commit.tombstone_target_keys,
                        &commit.run.id,
                        commit.run.completed_at.unwrap_or(commit.run.updated_at),
                    )
                    .await?;
                    for item in &commit.items {
                        insert_item_tx(tx, &datastore, item).await?;
                    }
                    if let Some(sequence) = commit.clear_pending_through_sequence {
                        clear_pending_discovery_context_changes_tx(
                            tx,
                            &commit.state.scope_key,
                            sequence,
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn commit_discovery_public_feed(
        &self,
        commit: &DiscoveryPublicFeedCommit,
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let commit = commit.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "commit_discovery_public_feed", move |tx| {
            let datastore = datastore.clone();
            let commit = commit.clone();
            Box::pin(async move {
                upsert_sync_run_tx(tx, &datastore, &commit.run).await?;
                upsert_sync_state_tx(tx, &commit.state).await?;
                delete_for_run_tx(tx, "discovery_raw_pages", &commit.run.id).await?;
                delete_for_run_tx(tx, "discovery_sections", &commit.run.id).await?;
                delete_for_run_tx(tx, "discovery_items", &commit.run.id).await?;
                insert_raw_page_tx(tx, &commit.raw_feed).await?;
                for section in &commit.sections {
                    insert_section_tx(tx, &datastore, section).await?;
                }
                for item in &commit.items {
                    insert_item_tx(tx, &datastore, item).await?;
                }
                Ok(())
            })
        })
        .await
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

    async fn list_discovery_submitted_subjects(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT run_id, subject_key, title_id, library_id, library_facet, title_kind, display_title,
                    external_ids_json, raw_subject_json
             FROM discovery_submitted_subjects
             WHERE run_id = {}
             ORDER BY subject_key ASC, library_id ASC, title_id ASC",
            &[SqlArg::Text(run_id.to_string())],
        )
        .await?;
        rows.iter().map(submitted_subject_from_row).collect()
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

    async fn get_pending_discovery_context_change(
        &self,
        id: &str,
    ) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {PENDING_CONTEXT_CHANGE_COLUMNS}
                 FROM discovery_pending_context_changes
                 WHERE id = {{}}"
            ),
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.as_ref()
            .map(pending_context_change_from_row)
            .transpose()
    }

    async fn delete_pending_discovery_context_change(&self, id: &str) -> AppResult<u64> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "DELETE FROM discovery_pending_context_changes WHERE id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn list_all_pending_discovery_context_changes(
        &self,
        scope_key: &str,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {PENDING_CONTEXT_CHANGE_COLUMNS}
                 FROM discovery_pending_context_changes
                 WHERE scope_key = {{}}
                 ORDER BY last_seen_at ASC, id ASC"
            ),
            &[SqlArg::Text(scope_key.to_string())],
        )
        .await?;
        rows.iter().map(pending_context_change_from_row).collect()
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

    async fn count_pending_discovery_context_changes(&self, scope_key: &str) -> AppResult<i64> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS pending_count FROM discovery_pending_context_changes WHERE scope_key = {}",
            &[SqlArg::Text(scope_key.to_string())],
        )
        .await?;

        row.as_ref().map_or(Ok(0), |row| row.i64("pending_count"))
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

    async fn list_discovery_sections(
        &self,
        run_id: &str,
        surface: Option<&str>,
    ) -> AppResult<Vec<DiscoverySectionRecord>> {
        let mut args = vec![SqlArg::Text(run_id.to_string())];
        let surface_clause = if let Some(surface) = surface {
            args.push(SqlArg::Text(surface.to_string()));
            " AND surface = {}"
        } else {
            ""
        };
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {}
                 FROM discovery_sections
                 WHERE run_id = {{}}{surface_clause}
                 ORDER BY sort_index ASC, section_id ASC",
                SECTION_COLUMNS.join(", ")
            ),
            &args,
        )
        .await?;
        rows.iter().map(section_from_row).collect()
    }

    async fn list_discovery_items_for_generation(
        &self,
        base_generation_id: &str,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {}
                 FROM discovery_items
                 WHERE base_generation_id = {{}}
                   AND tombstoned_at IS NULL
                 ORDER BY COALESCE(section_id, '') ASC,
                          id ASC",
                ITEM_COLUMNS.join(", ")
            ),
            &[SqlArg::Text(base_generation_id.to_string())],
        )
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    async fn list_discovery_facets(&self, run_id: &str) -> AppResult<Vec<DiscoveryFacetRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT run_id, facet_name, facet_value, smg_count, local_count, raw_json
             FROM discovery_facets
             WHERE run_id = {}
             ORDER BY facet_name ASC, facet_value ASC",
            &[SqlArg::Text(run_id.to_string())],
        )
        .await?;
        rows.iter().map(facet_from_row).collect()
    }

    async fn prune_discovery_history(
        &self,
        scope_key: &str,
        retain_successful_per_kind: usize,
        diagnostic_cutoff: DateTime<Utc>,
    ) -> AppResult<DiscoveryPruneReport> {
        let state = self.get_discovery_sync_state(scope_key).await?;
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, kind, status, base_generation_id, updated_at
             FROM discovery_sync_runs
             ORDER BY updated_at DESC, id DESC",
            &[],
        )
        .await?;
        let candidates = rows
            .iter()
            .map(discovery_run_prune_candidate_from_row)
            .collect::<AppResult<Vec<_>>>()?;

        let mut keep_ids = HashSet::new();
        let active_context_generation_id = state
            .as_ref()
            .and_then(|state| state.last_success_generation_id.clone());
        if let Some(state) = &state {
            keep_optional_id(&mut keep_ids, state.last_success_generation_id.as_deref());
            keep_optional_id(
                &mut keep_ids,
                state.last_public_feed_generation_id.as_deref(),
            );
            keep_optional_id(
                &mut keep_ids,
                state.inflight_context_snapshot_run_id.as_deref(),
            );
        }

        if let Some(active_context_generation_id) = active_context_generation_id.as_deref() {
            for candidate in &candidates {
                if candidate.kind == "context_incremental"
                    && candidate.status == "complete"
                    && candidate.base_generation_id.as_deref() == Some(active_context_generation_id)
                {
                    keep_ids.insert(candidate.id.clone());
                }
            }
        }

        let mut retained_successful_by_kind = HashMap::<String, usize>::new();
        for candidate in &candidates {
            if discovery_run_status_is_successful(&candidate.status) {
                let retained = retained_successful_by_kind
                    .entry(candidate.kind.clone())
                    .or_default();
                if *retained < retain_successful_per_kind {
                    keep_ids.insert(candidate.id.clone());
                    *retained += 1;
                }
            }

            if discovery_run_status_is_diagnostic(&candidate.status)
                && candidate.updated_at >= diagnostic_cutoff
            {
                keep_ids.insert(candidate.id.clone());
            }

            if candidate.status == "running" {
                keep_ids.insert(candidate.id.clone());
            }
        }

        let mut runs_deleted = 0u64;
        for candidate in &candidates {
            if keep_ids.contains(&candidate.id) {
                continue;
            }
            runs_deleted += SqlRuntime::execute(
                self.datastore.read_exec(),
                "DELETE FROM discovery_sync_runs WHERE id = {}",
                &[SqlArg::Text(candidate.id.clone())],
            )
            .await?;
        }

        Ok(DiscoveryPruneReport { runs_deleted })
    }
}

struct DiscoveryRunPruneCandidate {
    id: String,
    kind: String,
    status: String,
    base_generation_id: Option<String>,
    updated_at: DateTime<Utc>,
}

fn discovery_run_prune_candidate_from_row(row: &SqlRow) -> AppResult<DiscoveryRunPruneCandidate> {
    Ok(DiscoveryRunPruneCandidate {
        id: row.text("id")?,
        kind: row.text("kind")?,
        status: row.text("status")?,
        base_generation_id: row.opt_text("base_generation_id")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn keep_optional_id(keep_ids: &mut HashSet<String>, id: Option<&str>) {
    if let Some(id) = id {
        keep_ids.insert(id.to_string());
    }
}

fn discovery_run_status_is_successful(status: &str) -> bool {
    status == "complete" || status == "warning"
}

fn discovery_run_status_is_diagnostic(status: &str) -> bool {
    status == "warning" || status == "failed" || status == "deferred"
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

async fn upsert_sync_state_tx(
    tx: &mut SqlTx<'_>,
    state: &DiscoverySyncStateRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &upsert_sql(
            "discovery_sync_state",
            &split_columns(DISCOVERY_SYNC_STATE_COLUMNS),
            &["scope_key"],
        ),
        &sync_state_args(state),
    )
    .await?;
    Ok(())
}

async fn upsert_sync_run_tx(
    tx: &mut SqlTx<'_>,
    datastore: &StoreDatastore,
    run: &DiscoverySyncRunRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &upsert_sql(
            "discovery_sync_runs",
            &split_columns(DISCOVERY_SYNC_RUN_COLUMNS),
            &["id"],
        ),
        &sync_run_args(datastore, run)?,
    )
    .await?;
    Ok(())
}

async fn insert_raw_page_tx(tx: &mut SqlTx<'_>, page: &DiscoveryRawPageRecord) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
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

async fn tombstone_discovery_items_tx(
    tx: &mut SqlTx<'_>,
    base_generation_id: Option<&str>,
    target_keys: &[String],
    tombstone_run_id: &str,
    tombstoned_at: chrono::DateTime<chrono::Utc>,
) -> AppResult<()> {
    let Some(base_generation_id) = base_generation_id else {
        return Ok(());
    };
    for target_key in target_keys {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE discovery_items
             SET tombstoned_by_run_id = {}, tombstoned_at = {}, updated_at = {}
             WHERE base_generation_id = {}
               AND target_key = {}
               AND tombstoned_at IS NULL",
            &[
                SqlArg::Text(tombstone_run_id.to_string()),
                SqlArg::Timestamp(tombstoned_at),
                SqlArg::Timestamp(tombstoned_at),
                SqlArg::Text(base_generation_id.to_string()),
                SqlArg::Text(target_key.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn clear_pending_discovery_context_changes_tx(
    tx: &mut SqlTx<'_>,
    scope_key: &str,
    last_seen_sequence: i64,
) -> AppResult<u64> {
    let deleted = SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM discovery_pending_context_changes
         WHERE scope_key = {}
           AND last_seen_sequence IS NOT NULL
           AND last_seen_sequence <= {}",
        &[
            SqlArg::Text(scope_key.to_string()),
            SqlArg::I64(last_seen_sequence),
        ],
    )
    .await?;
    Ok(deleted)
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
        SqlArg::I64(state.transient_failure_count),
        SqlArg::I64(state.startup_jitter_seconds),
        SqlArg::I64(state.context_jitter_seconds),
        SqlArg::I64(state.incremental_reload_jitter_seconds),
        SqlArg::I64(state.public_feed_jitter_seconds),
        SqlArg::OptI64(state.last_seen_domain_event_sequence),
        SqlArg::OptText(state.inflight_context_snapshot_run_id.clone()),
        SqlArg::OptText(state.inflight_subject_fingerprint.clone()),
        SqlArg::OptI64(state.inflight_domain_event_sequence),
        SqlArg::OptText(state.lease_owner_id.clone()),
        SqlArg::OptTimestamp(state.lease_expires_at),
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
        transient_failure_count: row.i64("transient_failure_count")?,
        startup_jitter_seconds: row.i64("startup_jitter_seconds")?,
        context_jitter_seconds: row.i64("context_jitter_seconds")?,
        incremental_reload_jitter_seconds: row.i64("incremental_reload_jitter_seconds")?,
        public_feed_jitter_seconds: row.i64("public_feed_jitter_seconds")?,
        last_seen_domain_event_sequence: row.opt_i64("last_seen_domain_event_sequence")?,
        inflight_context_snapshot_run_id: row.opt_text("inflight_context_snapshot_run_id")?,
        inflight_subject_fingerprint: row.opt_text("inflight_subject_fingerprint")?,
        inflight_domain_event_sequence: row.opt_i64("inflight_domain_event_sequence")?,
        lease_owner_id: row.opt_text("lease_owner_id")?,
        lease_expires_at: row.opt_timestamp("lease_expires_at")?,
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

fn section_from_row(row: &SqlRow) -> AppResult<DiscoverySectionRecord> {
    Ok(DiscoverySectionRecord {
        id: row.text("id")?,
        run_id: row.text("run_id")?,
        section_id: row.text("section_id")?,
        section_type: row.text("section_type")?,
        surface: row.text("surface")?,
        title: row.text("title")?,
        source_signals_json: json_text(row, "source_signals_json")?,
        facets_json: json_text(row, "facets_json")?,
        sort_index: row.i32("sort_index")?,
        raw_json: json_text(row, "raw_json")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn item_from_row(row: &SqlRow) -> AppResult<DiscoveryItemRecord> {
    Ok(DiscoveryItemRecord {
        id: row.text("id")?,
        run_id: row.text("run_id")?,
        base_generation_id: row.opt_text("base_generation_id")?,
        source_run_kind: row.text("source_run_kind")?,
        section_id: row.opt_text("section_id")?,
        target_key: row.text("target_key")?,
        target_kind: row.text("target_kind")?,
        resolved: row.bool("resolved")?,
        resolved_title_id: row.opt_text("resolved_title_id")?,
        display_title: row.text("display_title")?,
        original_title: row.opt_text("original_title")?,
        sort_title: row.opt_text("sort_title")?,
        year: row.opt_i32("year")?,
        poster_path: row.opt_text("poster_path")?,
        poster_url: row.opt_text("poster_url")?,
        background_url: row.opt_text("background_url")?,
        overview: row.opt_text("overview")?,
        content_type: row.opt_text("content_type")?,
        genres_json: json_text(row, "genres_json")?,
        rating: row.opt_f64("rating")?,
        rating_sources_json: json_text(row, "rating_sources_json")?,
        status_tags_json: json_text(row, "status_tags_json")?,
        source_tags_json: json_text(row, "source_tags_json")?,
        sources_json: json_text(row, "sources_json")?,
        best_source: row.opt_text("best_source")?,
        relation_types_json: json_text(row, "relation_types_json")?,
        relation_subtypes_json: json_text(row, "relation_subtypes_json")?,
        chart_signals_json: json_text(row, "chart_signals_json")?,
        provider_signals_json: json_text(row, "provider_signals_json")?,
        rank_components_json: json_text(row, "rank_components_json")?,
        source_count: row.opt_i32("source_count")?,
        edge_count: row.opt_i32("edge_count")?,
        relation_count: row.opt_i32("relation_count")?,
        source_subject_count: row.opt_i32("source_subject_count")?,
        rank_score: row.opt_f64("rank_score")?,
        matched_subject_keys_json: json_text(row, "matched_subject_keys_json")?,
        matched_subject_titles_json: json_text(row, "matched_subject_titles_json")?,
        matched_subject_count: row.i32("matched_subject_count")?,
        library_provenance_json: json_text(row, "library_provenance_json")?,
        tmdb_collection_id: row.opt_text("tmdb_collection_id")?,
        tmdb_collection_name: row.opt_text("tmdb_collection_name")?,
        owned_in_input: row.bool("owned_in_input")?,
        facet_terms_json: json_text(row, "facet_terms_json")?,
        context_terms_json: json_text(row, "context_terms_json")?,
        change_subject_keys_json: json_text(row, "change_subject_keys_json")?,
        removed_subject_keys_json: json_text(row, "removed_subject_keys_json")?,
        tombstoned_by_run_id: row.opt_text("tombstoned_by_run_id")?,
        tombstoned_at: row.opt_timestamp("tombstoned_at")?,
        raw_json: json_text(row, "raw_json")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn facet_from_row(row: &SqlRow) -> AppResult<DiscoveryFacetRecord> {
    Ok(DiscoveryFacetRecord {
        run_id: row.text("run_id")?,
        facet_name: row.text("facet_name")?,
        facet_value: row.text("facet_value")?,
        smg_count: row.opt_i64("smg_count")?,
        local_count: row.opt_i64("local_count")?,
        raw_json: json_text(row, "raw_json")?,
    })
}

fn submitted_subject_from_row(row: &SqlRow) -> AppResult<DiscoverySubmittedSubjectRecord> {
    Ok(DiscoverySubmittedSubjectRecord {
        run_id: row.text("run_id")?,
        subject_key: row.text("subject_key")?,
        title_id: row.opt_text("title_id")?,
        library_id: row.opt_text("library_id")?,
        library_facet: row.opt_text("library_facet")?,
        title_kind: row.opt_text("title_kind")?,
        display_title: row.opt_text("display_title")?,
        external_ids_json: json_text(row, "external_ids_json")?,
        raw_subject_json: json_text(row, "raw_subject_json")?,
    })
}

fn json_text(row: &SqlRow, column: &str) -> AppResult<String> {
    row.opt_json(column)?
        .map(|value| serde_json::to_string(&value).map_err(repo_err))
        .transpose()
        .map(|value| value.unwrap_or_else(|| JsonValue::Null.to_string()))
}

async fn insert_submitted_subject_tx(
    tx: &mut SqlTx<'_>,
    datastore: &StoreDatastore,
    subject: &DiscoverySubmittedSubjectRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_submitted_subjects
         (run_id, subject_key, title_id, library_id, library_facet, title_kind, display_title,
          external_ids_json, raw_subject_json)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(subject.run_id.clone()),
            SqlArg::Text(subject.subject_key.clone()),
            SqlArg::OptText(subject.title_id.clone()),
            SqlArg::OptText(subject.library_id.clone()),
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
        json_arg(datastore, &item.library_provenance_json)?,
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
    async fn sqlite_store_round_trips_inflight_snapshot_and_discovery_lease() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_lease_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let lease_expires_at = now + chrono::Duration::minutes(30);

        let state = DiscoverySyncStateRecord {
            inflight_context_snapshot_run_id: Some("run-inflight".to_string()),
            lease_owner_id: Some("owner-a".to_string()),
            lease_expires_at: Some(lease_expires_at),
            transient_failure_count: 2,
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
        assert_eq!(
            loaded_state.inflight_context_snapshot_run_id.as_deref(),
            Some("run-inflight")
        );
        assert_eq!(loaded_state.lease_owner_id.as_deref(), Some("owner-a"));
        assert!(loaded_state.lease_expires_at.is_some());
        assert_eq!(loaded_state.transient_failure_count, 2);

        assert!(
            !store
                .try_acquire_discovery_sync_lease(
                    DISCOVERY_DEFAULT_SCOPE_KEY,
                    "owner-b",
                    now + chrono::Duration::minutes(31),
                    now + chrono::Duration::minutes(1),
                )
                .await
                .expect("live lease should be checked"),
            "a different owner must not steal a live lease"
        );
        assert!(
            store
                .renew_discovery_sync_lease(
                    DISCOVERY_DEFAULT_SCOPE_KEY,
                    "owner-a",
                    now + chrono::Duration::minutes(45),
                    now + chrono::Duration::minutes(2),
                )
                .await
                .expect("lease should renew"),
            "current owner should renew the lease"
        );
        assert!(
            store
                .try_acquire_discovery_sync_lease(
                    DISCOVERY_DEFAULT_SCOPE_KEY,
                    "owner-b",
                    now + chrono::Duration::minutes(90),
                    now + chrono::Duration::minutes(60),
                )
                .await
                .expect("expired lease should be checked"),
            "expired leases can be stolen"
        );
        let stolen_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load after steal")
            .expect("state should exist after steal");
        assert_eq!(stolen_state.lease_owner_id.as_deref(), Some("owner-b"));

        store
            .release_discovery_sync_lease(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                "owner-a",
                now + chrono::Duration::minutes(61),
            )
            .await
            .expect("wrong owner release should be harmless");
        let still_leased_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load after wrong release")
            .expect("state should exist after wrong release");
        assert_eq!(
            still_leased_state.lease_owner_id.as_deref(),
            Some("owner-b")
        );

        store
            .release_discovery_sync_lease(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                "owner-b",
                now + chrono::Duration::minutes(62),
            )
            .await
            .expect("lease should release");
        let released_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load after release")
            .expect("state should exist after release");
        assert!(released_state.lease_owner_id.is_none());
        assert!(released_state.lease_expires_at.is_none());
        assert_eq!(
            released_state.inflight_context_snapshot_run_id.as_deref(),
            Some("run-inflight")
        );
    }

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
        let recent_runs = store
            .list_recent_discovery_sync_runs(5)
            .await
            .expect("recent runs should list");
        assert_eq!(recent_runs.len(), 1);
        assert_eq!(recent_runs[0].id, "run-1");

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
                &[
                    DiscoverySubmittedSubjectRecord {
                        run_id: "run-1".to_string(),
                        subject_key: "tvdb:series:1".to_string(),
                        title_id: None,
                        library_id: Some("series-library-a".to_string()),
                        library_facet: Some("series".to_string()),
                        title_kind: Some("series".to_string()),
                        display_title: Some("Example Series A".to_string()),
                        external_ids_json: json!([{"source": "tvdb", "value": "1"}]).to_string(),
                        raw_subject_json: json!({"key": "tvdb:series:1"}).to_string(),
                    },
                    DiscoverySubmittedSubjectRecord {
                        run_id: "run-1".to_string(),
                        subject_key: "tvdb:series:1".to_string(),
                        title_id: None,
                        library_id: Some("series-library-b".to_string()),
                        library_facet: Some("series".to_string()),
                        title_kind: Some("series".to_string()),
                        display_title: Some("Example Series B".to_string()),
                        external_ids_json: json!([{"source": "tvdb", "value": "1"}]).to_string(),
                        raw_subject_json: json!({"key": "tvdb:series:1"}).to_string(),
                    },
                ],
            )
            .await
            .expect("submitted subjects should replace");
        let read_subjects = store
            .list_discovery_submitted_subjects("run-1")
            .await
            .expect("submitted subjects should list");
        assert_eq!(read_subjects.len(), 2);
        assert_eq!(read_subjects[0].subject_key, "tvdb:series:1");
        assert_eq!(
            read_subjects[0].library_id.as_deref(),
            Some("series-library-a")
        );
        assert_eq!(
            read_subjects[1].library_id.as_deref(),
            Some("series-library-b")
        );
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
                    library_provenance_json: json!([{
                        "subjectKey": "tvdb:series:1",
                        "titleId": null,
                        "libraryId": "series-library-a"
                    }])
                    .to_string(),
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
        let read_sections = store
            .list_discovery_sections("run-1", Some("personalized"))
            .await
            .expect("sections should list");
        assert_eq!(read_sections.len(), 1);
        assert_eq!(read_sections[0].section_id, "for_you");
        let read_items = store
            .list_discovery_items_for_generation("run-1")
            .await
            .expect("items should list");
        assert_eq!(read_items.len(), 1);
        assert_eq!(read_items[0].target_key, "tmdb:movie:10");
        assert_eq!(
            read_items[0].library_provenance_json,
            json!([{
                "subjectKey": "tvdb:series:1",
                "titleId": null,
                "libraryId": "series-library-a"
            }])
            .to_string()
        );
        let read_facets = store
            .list_discovery_facets("run-1")
            .await
            .expect("facets should list");
        assert_eq!(read_facets.len(), 1);
        assert_eq!(read_facets[0].facet_value, "Drama");

        store
            .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
                id: "snapshot-change-1".to_string(),
                scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
                subject_key: Some("tmdb:movie:603".to_string()),
                previous_subject_key: None,
                change_type: "updated".to_string(),
                title_id: None,
                previous_title_id: None,
                library_facet: Some("movie".to_string()),
                raw_subject_json: Some(json!({"tmdbId": 603}).to_string()),
                raw_previous_subject_json: None,
                first_seen_sequence: Some(4),
                last_seen_sequence: Some(4),
                first_seen_at: now,
                last_seen_at: now,
            })
            .await
            .expect("snapshot pending change should upsert");
        assert_eq!(
            store
                .count_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
                .await
                .expect("pending changes should count"),
            1
        );

        let committed_state = DiscoverySyncStateRecord {
            last_success_generation_id: Some("run-2".to_string()),
            last_subject_fingerprint: Some("fingerprint-run-2".to_string()),
            last_context_snapshot_completed_at: Some(now),
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        let committed_run = DiscoverySyncRunRecord {
            id: "run-2".to_string(),
            kind: "context_snapshot".to_string(),
            status: "complete".to_string(),
            trigger_source: "scheduled_interval".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 1,
            subject_fingerprint: Some("fingerprint-run-2".to_string()),
            previous_subject_fingerprint: Some("fingerprint-current".to_string()),
            base_generation_id: None,
            changed_subject_count: 0,
            affected_target_count: 0,
            smg_request_id: Some("request-2".to_string()),
            smg_status: Some("COMPLETE".to_string()),
            discovery_index_watermark: Some("watermark-2".to_string()),
            page_count: Some(1),
            item_count: Some(0),
            facet_count: Some(0),
            raw_submit_json: Some(json!({"status": "ACCEPTED"}).to_string()),
            raw_changes_json: None,
            raw_final_status_json: Some(json!({"status": "COMPLETE"}).to_string()),
            raw_ack_json: None,
            error_text: None,
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        store
            .commit_discovery_context_snapshot(&DiscoveryContextSnapshotCommit {
                state: committed_state,
                run: committed_run,
                raw_pages: vec![DiscoveryRawPageRecord {
                    run_id: "run-2".to_string(),
                    payload_kind: "snapshot_page".to_string(),
                    page_number: 1,
                    compression: "none".to_string(),
                    raw_payload: json!({"items": []}).to_string(),
                    created_at: now,
                }],
                submitted_subjects: vec![DiscoverySubmittedSubjectRecord {
                    run_id: "run-2".to_string(),
                    subject_key: "tmdb:movie:603".to_string(),
                    title_id: None,
                    library_id: Some("movie-library".to_string()),
                    library_facet: Some("movie".to_string()),
                    title_kind: Some("movie".to_string()),
                    display_title: Some("Example Movie".to_string()),
                    external_ids_json: json!([{"source": "tmdb", "value": "603"}]).to_string(),
                    raw_subject_json: json!({"tmdbId": 603}).to_string(),
                }],
                items: Vec::new(),
                facets: Vec::new(),
                clear_pending_through_sequence: Some(4),
            })
            .await
            .expect("snapshot commit should be transactional");

        let active_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("committed state should load")
            .expect("committed state should exist");
        assert_eq!(
            active_state.last_success_generation_id.as_deref(),
            Some("run-2")
        );
        let committed_run = store
            .get_discovery_sync_run("run-2")
            .await
            .expect("committed run should load")
            .expect("committed run should exist");
        assert_eq!(committed_run.kind, "context_snapshot");
        assert_eq!(committed_run.smg_request_id.as_deref(), Some("request-2"));
        let unacked_runs = store
            .list_unacked_discovery_context_snapshot_runs(10)
            .await
            .expect("unacked context snapshot runs should list");
        assert_eq!(unacked_runs.len(), 1);
        assert_eq!(unacked_runs[0].id, "run-2");
        assert!(
            store
                .list_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY, 10)
                .await
                .expect("pending changes should list after snapshot")
                .is_empty()
        );
        assert_eq!(
            store
                .count_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
                .await
                .expect("pending changes should count after snapshot"),
            0
        );

        let incremental_state = DiscoverySyncStateRecord {
            last_success_generation_id: Some("run-2".to_string()),
            last_subject_fingerprint: Some("fingerprint-incremental".to_string()),
            last_context_snapshot_completed_at: Some(now),
            last_incremental_reload_completed_at: Some(now),
            last_seen_domain_event_sequence: Some(12),
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        let incremental_run = DiscoverySyncRunRecord {
            id: "run-3".to_string(),
            kind: "context_incremental".to_string(),
            status: "complete".to_string(),
            trigger_source: "scheduled_incremental".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 1,
            subject_fingerprint: Some("fingerprint-incremental".to_string()),
            previous_subject_fingerprint: Some("fingerprint-run-2".to_string()),
            base_generation_id: Some("run-2".to_string()),
            changed_subject_count: 1,
            affected_target_count: 1,
            smg_request_id: None,
            smg_status: Some("COMPLETE".to_string()),
            discovery_index_watermark: Some("watermark-3".to_string()),
            page_count: None,
            item_count: Some(0),
            facet_count: None,
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
            .commit_discovery_context_incremental(&DiscoveryContextIncrementalCommit {
                state: incremental_state,
                run: incremental_run,
                raw_changes: DiscoveryRawPageRecord {
                    run_id: "run-3".to_string(),
                    payload_kind: "context_changes".to_string(),
                    page_number: 0,
                    compression: "none".to_string(),
                    raw_payload: json!({"items": []}).to_string(),
                    created_at: now,
                },
                items: Vec::new(),
                tombstone_target_keys: vec!["tmdb:movie:10".to_string()],
                clear_pending_through_sequence: Some(12),
            })
            .await
            .expect("incremental commit should be transactional");
        let incremental_run = store
            .get_discovery_sync_run("run-3")
            .await
            .expect("incremental run should load")
            .expect("incremental run should exist");
        assert_eq!(incremental_run.kind, "context_incremental");
        let incremental_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("incremental state should load")
            .expect("incremental state should exist");
        assert_eq!(
            incremental_state.last_incremental_reload_completed_at,
            Some(now)
        );

        let public_state = DiscoverySyncStateRecord {
            last_success_generation_id: Some("run-2".to_string()),
            last_public_feed_generation_id: Some("run-4".to_string()),
            last_public_feed_completed_at: Some(now),
            updated_at: now,
            ..incremental_state.clone()
        };
        let public_run = DiscoverySyncRunRecord {
            id: "run-4".to_string(),
            kind: "public_feed".to_string(),
            status: "complete".to_string(),
            trigger_source: "scheduled_interval".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 0,
            subject_fingerprint: None,
            previous_subject_fingerprint: None,
            base_generation_id: None,
            changed_subject_count: 0,
            affected_target_count: 0,
            smg_request_id: None,
            smg_status: Some("COMPLETE".to_string()),
            discovery_index_watermark: None,
            page_count: Some(1),
            item_count: Some(0),
            facet_count: Some(0),
            raw_submit_json: Some(json!({"region": "US"}).to_string()),
            raw_changes_json: None,
            raw_final_status_json: Some(json!({"sections": []}).to_string()),
            raw_ack_json: None,
            error_text: None,
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        store
            .commit_discovery_public_feed(&DiscoveryPublicFeedCommit {
                state: public_state,
                run: public_run,
                raw_feed: DiscoveryRawPageRecord {
                    run_id: "run-4".to_string(),
                    payload_kind: "public_feed".to_string(),
                    page_number: 0,
                    compression: "none".to_string(),
                    raw_payload: json!({"sections": []}).to_string(),
                    created_at: now,
                },
                sections: Vec::new(),
                items: Vec::new(),
            })
            .await
            .expect("public feed commit should be transactional");
        let public_run = store
            .get_discovery_sync_run("run-4")
            .await
            .expect("public feed run should load")
            .expect("public feed run should exist");
        assert_eq!(public_run.kind, "public_feed");
        let public_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("public feed state should load")
            .expect("public feed state should exist");
        assert_eq!(
            public_state.last_public_feed_generation_id.as_deref(),
            Some("run-4")
        );

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

    #[tokio::test]
    async fn sqlite_store_get_delete_and_list_all_pending_changes() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_pending_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let change = DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(json!({"tmdbId": 603}).to_string()),
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
        let loaded = store
            .get_pending_discovery_context_change("change-1")
            .await
            .expect("pending change should load")
            .expect("pending change should exist");
        assert_eq!(loaded.subject_key.as_deref(), Some("tmdb:movie:603"));
        assert_eq!(loaded.first_seen_sequence, Some(10));

        let all = store
            .list_all_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("all pending changes should list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "change-1");

        assert_eq!(
            store
                .delete_pending_discovery_context_change("change-1")
                .await
                .expect("pending change should delete"),
            1
        );
        assert!(
            store
                .get_pending_discovery_context_change("change-1")
                .await
                .expect("pending change lookup should succeed")
                .is_none()
        );
        assert_eq!(
            store
                .delete_pending_discovery_context_change("change-1")
                .await
                .expect("missing pending change delete should be harmless"),
            0
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_store_prunes_discovery_history_with_retention_guards() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_prune_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let active_snapshot_at = now - chrono::Duration::days(10);
        let old_at = now - chrono::Duration::days(60);

        for run in [
            discovery_prune_run(
                "snapshot-active",
                "context_snapshot",
                "complete",
                active_snapshot_at,
            ),
            discovery_prune_run("snapshot-newer", "context_snapshot", "complete", now),
            discovery_prune_run("snapshot-pruned", "context_snapshot", "complete", old_at),
            discovery_prune_run("public-active", "public_feed", "complete", old_at),
            discovery_prune_run(
                "incremental-attached",
                "context_incremental",
                "complete",
                old_at,
            ),
            discovery_prune_run("deferred-old", "context_incremental", "deferred", old_at),
            discovery_prune_run("failed-recent", "context_incremental", "failed", now),
            discovery_prune_run("running-old", "context_snapshot", "running", old_at),
        ] {
            let mut run = run;
            if run.id == "incremental-attached" {
                run.base_generation_id = Some("snapshot-active".to_string());
            }
            store
                .upsert_discovery_sync_run(&run)
                .await
                .expect("run should upsert");
        }
        store
            .upsert_discovery_sync_state(&DiscoverySyncStateRecord {
                last_success_generation_id: Some("snapshot-active".to_string()),
                last_public_feed_generation_id: Some("public-active".to_string()),
                updated_at: now,
                ..DiscoverySyncStateRecord::default()
            })
            .await
            .expect("state should upsert");
        store
            .insert_discovery_raw_page(&DiscoveryRawPageRecord {
                run_id: "snapshot-pruned".to_string(),
                payload_kind: "snapshot_page".to_string(),
                page_number: 1,
                compression: "none".to_string(),
                raw_payload: json!({"items": []}).to_string(),
                created_at: old_at,
            })
            .await
            .expect("raw page should insert for pruned run");
        store
            .replace_discovery_items(
                "snapshot-pruned",
                &[discovery_prune_item("snapshot-pruned", old_at)],
            )
            .await
            .expect("item should insert for pruned run");

        let report = store
            .prune_discovery_history(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                2,
                now - chrono::Duration::days(30),
            )
            .await
            .expect("discovery history should prune");
        assert_eq!(report.runs_deleted, 2);

        for id in [
            "snapshot-active",
            "snapshot-newer",
            "public-active",
            "incremental-attached",
            "failed-recent",
            "running-old",
        ] {
            assert!(
                store
                    .get_discovery_sync_run(id)
                    .await
                    .expect("run lookup should succeed")
                    .is_some(),
                "{id} should be retained"
            );
        }
        for id in ["snapshot-pruned", "deferred-old"] {
            assert!(
                store
                    .get_discovery_sync_run(id)
                    .await
                    .expect("run lookup should succeed")
                    .is_none(),
                "{id} should be pruned"
            );
        }
        let raw_page_count = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM discovery_raw_pages WHERE run_id = {}",
            &[SqlArg::Text("snapshot-pruned".to_string())],
        )
        .await
        .expect("raw page count should query")
        .expect("raw page count should return")
        .i64("count")
        .expect("raw page count should parse");
        assert_eq!(raw_page_count, 0);
        let item_count = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM discovery_items WHERE run_id = {}",
            &[SqlArg::Text("snapshot-pruned".to_string())],
        )
        .await
        .expect("item count should query")
        .expect("item count should return")
        .i64("count")
        .expect("item count should parse");
        assert_eq!(item_count, 0);

        let _ = std::fs::remove_file(db);
    }

    fn discovery_prune_run(
        id: &str,
        kind: &str,
        status: &str,
        observed_at: chrono::DateTime<Utc>,
    ) -> DiscoverySyncRunRecord {
        DiscoverySyncRunRecord {
            id: id.to_string(),
            kind: kind.to_string(),
            status: status.to_string(),
            trigger_source: "scheduled_interval".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 1,
            subject_fingerprint: Some(format!("{id}-fingerprint")),
            previous_subject_fingerprint: None,
            base_generation_id: None,
            changed_subject_count: 0,
            affected_target_count: 0,
            smg_request_id: None,
            smg_status: Some(status.to_string()),
            discovery_index_watermark: None,
            page_count: None,
            item_count: Some(0),
            facet_count: Some(0),
            raw_submit_json: None,
            raw_changes_json: None,
            raw_final_status_json: None,
            raw_ack_json: None,
            error_text: None,
            started_at: Some(observed_at),
            completed_at: if status == "running" {
                None
            } else {
                Some(observed_at)
            },
            created_at: observed_at,
            updated_at: observed_at,
        }
    }

    fn discovery_prune_item(
        run_id: &str,
        observed_at: chrono::DateTime<Utc>,
    ) -> DiscoveryItemRecord {
        DiscoveryItemRecord {
            id: format!("{run_id}:item:tmdb:movie:604"),
            run_id: run_id.to_string(),
            base_generation_id: Some(run_id.to_string()),
            source_run_kind: "context_snapshot".to_string(),
            section_id: None,
            target_key: "tmdb:movie:604".to_string(),
            target_kind: "movie".to_string(),
            resolved: false,
            resolved_title_id: None,
            display_title: "Pruned Movie".to_string(),
            original_title: None,
            sort_title: Some("Pruned Movie".to_string()),
            year: Some(2026),
            poster_path: None,
            poster_url: None,
            background_url: None,
            overview: None,
            content_type: Some("movie".to_string()),
            genres_json: "[]".to_string(),
            rating: None,
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
            edge_count: Some(0),
            relation_count: Some(0),
            source_subject_count: Some(0),
            rank_score: Some(0.1),
            matched_subject_keys_json: "[]".to_string(),
            matched_subject_titles_json: "[]".to_string(),
            matched_subject_count: 0,
            library_provenance_json: "[]".to_string(),
            tmdb_collection_id: None,
            tmdb_collection_name: None,
            owned_in_input: false,
            facet_terms_json: "[]".to_string(),
            context_terms_json: "[]".to_string(),
            change_subject_keys_json: "[]".to_string(),
            removed_subject_keys_json: "[]".to_string(),
            tombstoned_by_run_id: None,
            tombstoned_at: None,
            raw_json: json!({"targetKey": "tmdb:movie:604"}).to_string(),
            created_at: observed_at,
            updated_at: observed_at,
        }
    }
}
