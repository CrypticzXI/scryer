use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppResult, CatalogDiscoveryCandidatesRecord, CatalogDiscoverySectionCandidatesRecord,
    DiscoveryContextIncrementalCommit, DiscoveryContextSnapshotCommit, DiscoveryExternalIdRecord,
    DiscoveryFacetRecord, DiscoveryItemLibraryProvenanceRecord, DiscoveryItemRecord,
    DiscoveryItemsPageRecord, DiscoveryItemsStorageQuery, DiscoveryPendingContextChangeRecord,
    DiscoveryPruneReport, DiscoveryPublicFeedCommit, DiscoveryRankComponentRecord,
    DiscoveryRawPageRecord, DiscoveryRepository, DiscoverySectionItemsRecord,
    DiscoverySectionRecord, DiscoverySourceTagRecord, DiscoverySubmittedSubjectRecord,
    DiscoverySyncRunRecord, DiscoverySyncStateRecord, TitleExternalRating,
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
    "sort_index",
    "created_at",
    "updated_at",
];

const ITEM_COLUMNS: &[&str] = &[
    "id",
    "run_id",
    "base_generation_id",
    "source_run_kind",
    "section_id",
    "sort_index",
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
    "rating",
    "best_source",
    "source_count",
    "edge_count",
    "relation_count",
    "source_subject_count",
    "rank_score",
    "matched_subject_count",
    "tmdb_collection_id",
    "tmdb_collection_name",
    "owned_in_input",
    "tombstoned_by_run_id",
    "tombstoned_at",
    "created_at",
    "updated_at",
];

const TITLE_COLUMNS: &[&str] = &[
    "id",
    "target_key",
    "target_key_norm",
    "language",
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
    "rating",
    "tmdb_collection_id",
    "tmdb_collection_name",
    "created_at",
    "updated_at",
];

const OCCURRENCE_COLUMNS: &[&str] = &[
    "id",
    "run_id",
    "base_generation_id",
    "discovery_title_id",
    "source_run_kind",
    "section_id",
    "sort_index",
    "best_source",
    "source_count",
    "edge_count",
    "relation_count",
    "source_subject_count",
    "rank_score",
    "matched_subject_count",
    "owned_in_input",
    "tombstoned_by_run_id",
    "tombstoned_at",
    "created_at",
    "updated_at",
];

const FACET_COLUMNS: &[&str] = &[
    "run_id",
    "facet_name",
    "facet_value",
    "smg_count",
    "local_count",
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
                    delete_item_children_for_run_tx(tx, &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_items", &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_facets", &commit.run.id).await?;

                    for page in &commit.raw_pages {
                        insert_raw_page_tx(tx, page).await?;
                    }
                    for subject in &commit.submitted_subjects {
                        insert_submitted_subject_tx(tx, &datastore, subject).await?;
                    }
                    for item in &commit.items {
                        insert_item_tx(tx, &datastore, item, &commit.run.language).await?;
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
                    delete_item_children_for_run_tx(tx, &commit.run.id).await?;
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
                        insert_item_tx(tx, &datastore, item, &commit.run.language).await?;
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
                delete_for_run_tx(tx, "discovery_section_items", &commit.run.id).await?;
                delete_for_run_tx(tx, "discovery_sections", &commit.run.id).await?;
                delete_item_children_for_run_tx(tx, &commit.run.id).await?;
                delete_for_run_tx(tx, "discovery_items", &commit.run.id).await?;
                insert_raw_page_tx(tx, &commit.raw_feed).await?;
                for section in &commit.sections {
                    insert_section_tx(tx, &datastore, section).await?;
                }
                for item in &commit.items {
                    insert_item_tx(tx, &datastore, item, &commit.run.language).await?;
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
                delete_for_run_tx(tx, "discovery_section_items", &run_id).await?;
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
        let language = discovery_run_language(&self.datastore, run_id)
            .await?
            .unwrap_or_else(|| "eng".to_string());
        let run_id = run_id.to_string();
        let items = items.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_items", move |tx| {
            let datastore = datastore.clone();
            let language = language.clone();
            let run_id = run_id.clone();
            let items = items.clone();
            Box::pin(async move {
                delete_item_children_for_run_tx(tx, &run_id).await?;
                delete_for_run_tx(tx, "discovery_items", &run_id).await?;
                for item in &items {
                    insert_item_tx(tx, &datastore, item, &language).await?;
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

    async fn list_public_discovery_section_items(
        &self,
        run_id: &str,
        include_unresolved: bool,
        limit_per_section: i64,
    ) -> AppResult<Vec<DiscoverySectionItemsRecord>> {
        let sections = self.list_discovery_sections(run_id, Some("public")).await?;
        if sections.is_empty() {
            return Ok(Vec::new());
        }
        let rows = fetch_public_section_item_rows(
            &self.datastore,
            run_id,
            include_unresolved,
            limit_per_section.clamp(1, 100),
        )
        .await?;
        section_items_from_rows(&self.datastore, sections, rows).await
    }

    async fn list_personalized_discovery_home_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        fetch_personalized_items(
            &self.datastore,
            run_id,
            readable_library_ids,
            include_unresolved,
            None,
            limit.clamp(1, 5_000),
        )
        .await
    }

    async fn list_personalized_complete_collection_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        fetch_personalized_items(
            &self.datastore,
            run_id,
            readable_library_ids,
            include_unresolved,
            Some(PersonalizedItemSubset::CompleteCollection),
            limit.clamp(1, 2_000),
        )
        .await
    }

    async fn list_personalized_discovery_facets(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        include_unresolved: bool,
    ) -> AppResult<Vec<DiscoveryFacetRecord>> {
        fetch_personalized_facets(
            &self.datastore,
            run_id,
            readable_library_ids,
            include_unresolved,
        )
        .await
    }

    async fn list_catalog_public_discovery_items(
        &self,
        run_id: &str,
        owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord> {
        fetch_catalog_public_items(
            &self.datastore,
            run_id,
            owned_library_ids,
            excluded_identity_keys,
            media_kind,
            include_unresolved,
            limit.clamp(1, 1_000),
        )
        .await
    }

    async fn list_catalog_public_discovery_sections(
        &self,
        run_id: &str,
        owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit_per_section: i64,
    ) -> AppResult<Vec<CatalogDiscoverySectionCandidatesRecord>> {
        fetch_catalog_public_sections(
            &self.datastore,
            run_id,
            owned_library_ids,
            excluded_identity_keys,
            media_kind,
            include_unresolved,
            limit_per_section.clamp(1, 1_000),
        )
        .await
    }

    async fn list_catalog_personalized_discovery_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord> {
        fetch_catalog_personalized_items(
            &self.datastore,
            run_id,
            readable_library_ids,
            media_kind,
            include_unresolved,
            limit.clamp(1, 1_000),
        )
        .await
    }

    async fn query_discovery_items(
        &self,
        query: &DiscoveryItemsStorageQuery,
    ) -> AppResult<DiscoveryItemsPageRecord> {
        query_discovery_items_page(&self.datastore, query).await
    }

    async fn replace_title_more_like_this_items(
        &self,
        title_id: &str,
        language: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let title_id = title_id.to_string();
        let language = normalize_discovery_language(language);
        let items = items.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_title_more_like_this_items",
            move |tx| {
                let datastore = datastore.clone();
                let title_id = title_id.clone();
                let language = language.clone();
                let items = items.clone();
                Box::pin(async move {
                    delete_title_more_like_this_items_tx(tx, &title_id).await?;
                    for item in &items {
                        insert_title_more_like_this_item_tx(
                            tx, &datastore, &title_id, item, &language,
                        )
                        .await?;
                    }
                    delete_unreferenced_discovery_titles_tx(tx).await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_title_more_like_this_items(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {}
                 FROM title_more_like_this_items
                 JOIN discovery_titles t
                   ON t.id = title_more_like_this_items.discovery_title_id
                 WHERE source_title_id = {{}}
                 ORDER BY sort_index ASC,
                          COALESCE(rank_score, 0) DESC,
                          discovery_title_id ASC
                 LIMIT {{}}",
                title_more_like_this_projection()
            ),
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I64(limit.clamp(0, 100)),
            ],
        )
        .await?;
        let mut items = rows
            .iter()
            .map(item_from_row)
            .collect::<AppResult<Vec<_>>>()?;
        let title_ids = rows
            .iter()
            .map(|row| row.text("discovery_title_id"))
            .collect::<AppResult<Vec<_>>>()?;
        hydrate_discovery_title_children(&self.datastore, &mut items, &title_ids).await?;
        Ok(items)
    }

    async fn list_discovery_items_for_generation(
        &self,
        base_generation_id: &str,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {}
                 FROM discovery_items i
                 JOIN discovery_titles t
                   ON t.id = i.discovery_title_id
                 WHERE i.base_generation_id = {{}}
                   AND i.tombstoned_at IS NULL
                 ORDER BY COALESCE(i.section_id, '') ASC,
                          i.sort_index ASC,
                          i.id ASC",
                discovery_item_projection("i", "t")
            ),
            &[SqlArg::Text(base_generation_id.to_string())],
        )
        .await?;
        let mut items = rows
            .iter()
            .map(item_from_row)
            .collect::<AppResult<Vec<_>>>()?;
        let title_ids = discovery_title_ids_from_rows(&rows)?;
        hydrate_discovery_items(&self.datastore, &mut items, &title_ids).await?;
        Ok(items)
    }

    async fn list_discovery_facets(&self, run_id: &str) -> AppResult<Vec<DiscoveryFacetRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT run_id, facet_name, facet_value, smg_count, local_count
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
        delete_unreferenced_discovery_titles(&self.datastore).await?;

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

fn discovery_item_projection(item_alias: &str, title_alias: &str) -> String {
    [
        format!("{item_alias}.id AS id"),
        format!("{item_alias}.run_id AS run_id"),
        format!("{item_alias}.base_generation_id AS base_generation_id"),
        format!("{item_alias}.source_run_kind AS source_run_kind"),
        format!("{item_alias}.section_id AS section_id"),
        format!("{item_alias}.sort_index AS sort_index"),
        format!("{title_alias}.target_key AS target_key"),
        format!("{title_alias}.target_kind AS target_kind"),
        format!("{title_alias}.resolved AS resolved"),
        format!("{title_alias}.resolved_title_id AS resolved_title_id"),
        format!("{title_alias}.display_title AS display_title"),
        format!("{title_alias}.original_title AS original_title"),
        format!("{title_alias}.sort_title AS sort_title"),
        format!("{title_alias}.year AS year"),
        format!("{title_alias}.poster_path AS poster_path"),
        format!("{title_alias}.poster_url AS poster_url"),
        format!("{title_alias}.background_url AS background_url"),
        format!("{title_alias}.overview AS overview"),
        format!("{title_alias}.content_type AS content_type"),
        format!("{title_alias}.rating AS rating"),
        format!("{item_alias}.best_source AS best_source"),
        format!("{item_alias}.source_count AS source_count"),
        format!("{item_alias}.edge_count AS edge_count"),
        format!("{item_alias}.relation_count AS relation_count"),
        format!("{item_alias}.source_subject_count AS source_subject_count"),
        format!("{item_alias}.rank_score AS rank_score"),
        format!("{item_alias}.matched_subject_count AS matched_subject_count"),
        format!("{title_alias}.tmdb_collection_id AS tmdb_collection_id"),
        format!("{title_alias}.tmdb_collection_name AS tmdb_collection_name"),
        format!("{item_alias}.owned_in_input AS owned_in_input"),
        format!("{item_alias}.tombstoned_by_run_id AS tombstoned_by_run_id"),
        format!("{item_alias}.tombstoned_at AS tombstoned_at"),
        format!("{item_alias}.created_at AS created_at"),
        format!("{item_alias}.updated_at AS updated_at"),
        format!("{item_alias}.discovery_title_id AS discovery_title_id"),
    ]
    .join(", ")
}

fn discovery_item_row_columns() -> String {
    format!("{}, discovery_title_id", ITEM_COLUMNS.join(", "))
}

fn title_more_like_this_projection() -> String {
    [
        "source_title_id || ':more-like-this:' || discovery_title_id AS id".to_string(),
        "source_title_id AS run_id".to_string(),
        "NULL AS base_generation_id".to_string(),
        "'title_more_like_this' AS source_run_kind".to_string(),
        "NULL AS section_id".to_string(),
        "title_more_like_this_items.sort_index AS sort_index".to_string(),
        "t.target_key AS target_key".to_string(),
        "t.target_kind AS target_kind".to_string(),
        "t.resolved AS resolved".to_string(),
        "t.resolved_title_id AS resolved_title_id".to_string(),
        "t.display_title AS display_title".to_string(),
        "t.original_title AS original_title".to_string(),
        "t.sort_title AS sort_title".to_string(),
        "t.year AS year".to_string(),
        "t.poster_path AS poster_path".to_string(),
        "t.poster_url AS poster_url".to_string(),
        "t.background_url AS background_url".to_string(),
        "t.overview AS overview".to_string(),
        "t.content_type AS content_type".to_string(),
        "t.rating AS rating".to_string(),
        "title_more_like_this_items.best_source AS best_source".to_string(),
        "title_more_like_this_items.source_count AS source_count".to_string(),
        "title_more_like_this_items.edge_count AS edge_count".to_string(),
        "title_more_like_this_items.relation_count AS relation_count".to_string(),
        "title_more_like_this_items.source_subject_count AS source_subject_count".to_string(),
        "title_more_like_this_items.rank_score AS rank_score".to_string(),
        "0 AS matched_subject_count".to_string(),
        "t.tmdb_collection_id AS tmdb_collection_id".to_string(),
        "t.tmdb_collection_name AS tmdb_collection_name".to_string(),
        "FALSE AS owned_in_input".to_string(),
        "NULL AS tombstoned_by_run_id".to_string(),
        "NULL AS tombstoned_at".to_string(),
        "title_more_like_this_items.created_at AS created_at".to_string(),
        "title_more_like_this_items.updated_at AS updated_at".to_string(),
        "t.id AS discovery_title_id".to_string(),
    ]
    .join(", ")
}

fn storage_text(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn normalize_discovery_language(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        "und".to_string()
    } else {
        value
    }
}

fn discovery_title_target_key_norm(item: &DiscoveryItemRecord) -> String {
    let target_key = item.target_key.trim().to_ascii_lowercase();
    if target_key.is_empty() {
        format!(
            "__scryer_occurrence:{}",
            item.id.trim().to_ascii_lowercase()
        )
    } else {
        target_key
    }
}

fn discovery_title_id_for(target_key_norm: &str, language: &str) -> String {
    let digest = blake3::hash(format!("{language}\0{target_key_norm}").as_bytes());
    format!("discovery-title:{}", digest.to_hex())
}

fn empty_to_none(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn discovery_item_authoritative_media_kind(item: &DiscoveryItemRecord) -> Option<String> {
    normalized_discovery_media_kind(item.content_type.as_deref())
        .or_else(|| normalized_discovery_media_kind(Some(item.target_kind.as_str())))
        .map(str::to_string)
}

fn normalized_discovery_media_kind(value: Option<&str>) -> Option<&'static str> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "anime" => Some("anime"),
        "movie" => Some("movie"),
        "series" => Some("series"),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum PersonalizedItemSubset {
    CompleteCollection,
}

struct DiscoveryItemsSql {
    cte_sql: String,
    args: Vec<SqlArg>,
}

async fn fetch_public_section_item_rows(
    datastore: &StoreDatastore,
    run_id: &str,
    include_unresolved: bool,
    limit_per_section: i64,
) -> AppResult<Vec<SqlRow>> {
    let resolved_clause = if include_unresolved {
        ""
    } else {
        " AND t.resolved = TRUE"
    };
    SqlRuntime::fetch_all(
        datastore.read_exec(),
        &format!(
            "WITH candidates AS (
                SELECT {}, si.section_id AS result_section_id, si.sort_index AS section_sort_index,
                       ROW_NUMBER() OVER (
                           PARTITION BY si.section_id,
                                        CASE WHEN TRIM(t.target_key) = '' THEN i.id ELSE t.target_key END
                           ORDER BY si.sort_index ASC, i.id ASC
                       ) AS identity_rank
                FROM discovery_section_items si
                JOIN discovery_sections s
                  ON s.run_id = si.run_id
                 AND s.section_id = si.section_id
                JOIN discovery_items i
                  ON i.id = si.item_id
                JOIN discovery_titles t
                  ON t.id = i.discovery_title_id
                WHERE si.run_id = {{}}
                  AND i.base_generation_id = {{}}
                  AND i.tombstoned_at IS NULL
                  AND i.owned_in_input = FALSE
                  AND s.surface = 'public'
                  AND UPPER(TRIM(s.section_type)) <> 'COMPLETE_THE_COLLECTION'
                  {resolved_clause}
             ),
             deduped AS (
                SELECT * FROM candidates WHERE identity_rank = 1
             ),
             ranked AS (
                SELECT *,
                       ROW_NUMBER() OVER (
                           PARTITION BY result_section_id
                           ORDER BY section_sort_index ASC, id ASC
                       ) AS section_rank,
                       COUNT(*) OVER (PARTITION BY result_section_id) AS section_total_count
                FROM deduped
             )
             SELECT {}, result_section_id, section_total_count
             FROM ranked
             WHERE section_rank <= {{}}
            ORDER BY result_section_id ASC, section_rank ASC",
            discovery_item_projection("i", "t"),
            discovery_item_row_columns()
        ),
        &[
            SqlArg::Text(run_id.to_string()),
            SqlArg::Text(run_id.to_string()),
            SqlArg::I64(limit_per_section),
        ],
    )
    .await
}

async fn section_items_from_rows(
    datastore: &StoreDatastore,
    sections: Vec<DiscoverySectionRecord>,
    rows: Vec<SqlRow>,
) -> AppResult<Vec<DiscoverySectionItemsRecord>> {
    let mut item_metadata = Vec::new();
    let mut items = Vec::new();
    for row in &rows {
        item_metadata.push((
            row.text("result_section_id")?,
            row.i64("section_total_count")?,
        ));
        items.push(item_from_row(row)?);
    }
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_items(datastore, &mut items, &title_ids).await?;

    let mut items_by_section = HashMap::<String, Vec<DiscoveryItemRecord>>::new();
    let mut totals_by_section = HashMap::<String, i64>::new();
    for (item, (section_id, total_count)) in items.into_iter().zip(item_metadata) {
        totals_by_section
            .entry(section_id.clone())
            .or_insert(total_count);
        items_by_section.entry(section_id).or_default().push(item);
    }

    Ok(sections
        .into_iter()
        .filter(|section| !discovery_section_type_is_complete(&section.section_type))
        .filter_map(|section| {
            let items = items_by_section.remove(&section.section_id)?;
            Some(DiscoverySectionItemsRecord {
                total_count: totals_by_section
                    .remove(&section.section_id)
                    .unwrap_or(items.len() as i64),
                section,
                items,
            })
        })
        .collect())
}

fn discovery_section_type_is_complete(section_type: &str) -> bool {
    section_type
        .trim()
        .eq_ignore_ascii_case("COMPLETE_THE_COLLECTION")
}

async fn fetch_personalized_items(
    datastore: &StoreDatastore,
    run_id: &str,
    readable_library_ids: &[String],
    include_unresolved: bool,
    subset: Option<PersonalizedItemSubset>,
    limit: i64,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    if readable_library_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut args = vec![SqlArg::Text(run_id.to_string())];
    let mut clauses = vec![
        "i.base_generation_id = {}".to_string(),
        "i.tombstoned_at IS NULL".to_string(),
        "i.owned_in_input = FALSE".to_string(),
    ];
    if !include_unresolved {
        clauses.push("t.resolved = TRUE".to_string());
    }
    clauses.push(library_provenance_exists_clause(
        "i",
        readable_library_ids,
        &mut args,
    ));
    if matches!(subset, Some(PersonalizedItemSubset::CompleteCollection)) {
        clauses.push(authoritative_media_kind_clause("t", "movie"));
        clauses.push(collection_signal_clause("i", "t"));
    }
    args.push(SqlArg::I64(limit));

    let sql = format!(
        "SELECT {}
         FROM discovery_items i
         JOIN discovery_titles t
           ON t.id = i.discovery_title_id
         WHERE {}
         ORDER BY COALESCE(i.rank_score, -999999999.0) DESC,
                  COALESCE(t.sort_title, t.display_title) ASC,
                  t.target_key ASC
         LIMIT {{}}",
        discovery_item_projection("i", "t"),
        clauses.join(" AND ")
    );
    fetch_items_with_sql(datastore, &sql, &args).await
}

async fn fetch_catalog_public_items(
    datastore: &StoreDatastore,
    run_id: &str,
    owned_library_ids: &[String],
    excluded_identity_keys: &[String],
    media_kind: &str,
    include_unresolved: bool,
    limit: i64,
) -> AppResult<CatalogDiscoveryCandidatesRecord> {
    let resolved_clause = if include_unresolved {
        ""
    } else {
        " AND t.resolved = TRUE"
    };
    let mut args = vec![
        SqlArg::Text(run_id.to_string()),
        SqlArg::Text(run_id.to_string()),
    ];
    let owned_clause = if owned_library_ids.is_empty() {
        String::new()
    } else {
        let placeholders = placeholders(owned_library_ids.len());
        args.extend(owned_library_ids.iter().cloned().map(SqlArg::Text));
        format!(
            " AND NOT EXISTS (
                SELECT 1
                FROM titles owned
                WHERE owned.id = t.resolved_title_id
                  AND owned.library_id IN ({placeholders})
             )"
        )
    };
    let excluded_identity_clause = if excluded_identity_keys.is_empty() {
        String::new()
    } else {
        let placeholders = placeholders(excluded_identity_keys.len());
        args.extend(excluded_identity_keys.iter().cloned().map(SqlArg::Text));
        format!(
            " AND CASE WHEN TRIM(t.target_key) = '' THEN LOWER(i.id) ELSE LOWER(TRIM(t.target_key)) END NOT IN ({placeholders})"
        )
    };
    let sql = format!(
        "WITH candidates AS (
            SELECT {}, s.sort_index AS section_sort_index, si.sort_index AS section_item_sort_index,
                   ROW_NUMBER() OVER (
                       PARTITION BY CASE WHEN TRIM(t.target_key) = '' THEN i.id ELSE t.target_key END
                       ORDER BY s.sort_index ASC, si.sort_index ASC, i.id ASC
                   ) AS identity_rank
            FROM discovery_section_items si
            JOIN discovery_sections s
              ON s.run_id = si.run_id
             AND s.section_id = si.section_id
            JOIN discovery_items i
              ON i.id = si.item_id
            JOIN discovery_titles t
              ON t.id = i.discovery_title_id
            WHERE si.run_id = {{}}
              AND i.base_generation_id = {{}}
              AND i.tombstoned_at IS NULL
              AND i.owned_in_input = FALSE
              AND s.surface = 'public'
              AND UPPER(TRIM(s.section_type)) <> 'COMPLETE_THE_COLLECTION'
              AND {}
              {owned_clause}
              {excluded_identity_clause}
              {resolved_clause}
         ),
         deduped AS (
            SELECT * FROM candidates WHERE identity_rank = 1
         ),
         ranked AS (
            SELECT *,
                   COUNT(*) OVER () AS total_count
            FROM deduped
         )
         SELECT {}, total_count
         FROM ranked
        ORDER BY section_sort_index ASC, section_item_sort_index ASC, id ASC
        LIMIT {{}}",
        discovery_item_projection("i", "t"),
        authoritative_media_kind_clause("t", media_kind),
        discovery_item_row_columns()
    );
    args.push(SqlArg::I64(limit));
    fetch_catalog_candidates_with_sql(datastore, &sql, &args).await
}

async fn fetch_catalog_public_sections(
    datastore: &StoreDatastore,
    run_id: &str,
    owned_library_ids: &[String],
    excluded_identity_keys: &[String],
    media_kind: &str,
    include_unresolved: bool,
    limit_per_section: i64,
) -> AppResult<Vec<CatalogDiscoverySectionCandidatesRecord>> {
    let resolved_clause = if include_unresolved {
        ""
    } else {
        " AND t.resolved = TRUE"
    };
    let mut args = vec![
        SqlArg::Text(run_id.to_string()),
        SqlArg::Text(run_id.to_string()),
    ];
    let owned_clause = if owned_library_ids.is_empty() {
        String::new()
    } else {
        let placeholders = placeholders(owned_library_ids.len());
        args.extend(owned_library_ids.iter().cloned().map(SqlArg::Text));
        format!(
            " AND NOT EXISTS (
                SELECT 1
                FROM titles owned
                WHERE owned.id = t.resolved_title_id
                  AND owned.library_id IN ({placeholders})
             )"
        )
    };
    let excluded_identity_clause = if excluded_identity_keys.is_empty() {
        String::new()
    } else {
        let placeholders = placeholders(excluded_identity_keys.len());
        args.extend(excluded_identity_keys.iter().cloned().map(SqlArg::Text));
        format!(
            " AND CASE WHEN TRIM(t.target_key) = '' THEN LOWER(i.id) ELSE LOWER(TRIM(t.target_key)) END NOT IN ({placeholders})"
        )
    };
    let sql = format!(
        "WITH candidates AS (
            SELECT {}, s.section_id AS result_section_id,
                   s.section_type AS result_section_type,
                   s.title AS result_section_title,
                   s.sort_index AS section_sort_index,
                   si.sort_index AS section_item_sort_index,
                   ROW_NUMBER() OVER (
                       PARTITION BY s.section_id,
                                    CASE WHEN TRIM(t.target_key) = '' THEN i.id ELSE t.target_key END
                       ORDER BY si.sort_index ASC, i.id ASC
                   ) AS identity_rank
            FROM discovery_section_items si
            JOIN discovery_sections s
              ON s.run_id = si.run_id
             AND s.section_id = si.section_id
            JOIN discovery_items i
              ON i.id = si.item_id
            JOIN discovery_titles t
              ON t.id = i.discovery_title_id
            WHERE si.run_id = {{}}
              AND i.base_generation_id = {{}}
              AND i.tombstoned_at IS NULL
              AND i.owned_in_input = FALSE
              AND s.surface = 'public'
              AND UPPER(TRIM(s.section_type)) <> 'COMPLETE_THE_COLLECTION'
              AND {}
              {owned_clause}
              {excluded_identity_clause}
              {resolved_clause}
         ),
         deduped AS (
            SELECT * FROM candidates WHERE identity_rank = 1
         ),
         ranked AS (
            SELECT *,
                   ROW_NUMBER() OVER (
                       PARTITION BY result_section_id
                       ORDER BY section_sort_index ASC, section_item_sort_index ASC, id ASC
                   ) AS section_rank,
                   COUNT(*) OVER (PARTITION BY result_section_id) AS section_total_count
            FROM deduped
         )
         SELECT {}, result_section_id, result_section_type, result_section_title, section_total_count
         FROM ranked
         WHERE section_rank <= {{}}
        ORDER BY section_sort_index ASC, section_rank ASC, id ASC",
        discovery_item_projection("i", "t"),
        authoritative_media_kind_clause("t", media_kind),
        discovery_item_row_columns()
    );
    args.push(SqlArg::I64(limit_per_section));
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await?;
    let mut item_metadata = Vec::new();
    let mut items = Vec::new();
    for row in &rows {
        item_metadata.push((
            row.text("result_section_id")?,
            row.text("result_section_type")?,
            row.opt_text("result_section_title")?,
            row.i64("section_total_count")?,
        ));
        items.push(item_from_row(row)?);
    }
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_items(datastore, &mut items, &title_ids).await?;

    let mut sections = Vec::<CatalogDiscoverySectionCandidatesRecord>::new();
    for (item, (section_id, section_type, title, total_count)) in
        items.into_iter().zip(item_metadata)
    {
        if let Some(section) = sections
            .last_mut()
            .filter(|section| section.section_id == section_id)
        {
            section.items.push(item);
        } else {
            sections.push(CatalogDiscoverySectionCandidatesRecord {
                section_id,
                section_type,
                title,
                total_count,
                items: vec![item],
            });
        }
    }
    Ok(sections)
}

async fn fetch_catalog_personalized_items(
    datastore: &StoreDatastore,
    run_id: &str,
    readable_library_ids: &[String],
    media_kind: &str,
    include_unresolved: bool,
    limit: i64,
) -> AppResult<CatalogDiscoveryCandidatesRecord> {
    if readable_library_ids.is_empty() {
        return Ok(CatalogDiscoveryCandidatesRecord {
            total_count: 0,
            items: Vec::new(),
        });
    }

    let mut args = vec![SqlArg::Text(run_id.to_string())];
    let mut clauses = vec![
        "i.base_generation_id = {}".to_string(),
        "i.tombstoned_at IS NULL".to_string(),
        "i.owned_in_input = FALSE".to_string(),
        authoritative_media_kind_clause("t", media_kind),
    ];
    if !include_unresolved {
        clauses.push("t.resolved = TRUE".to_string());
    }
    clauses.push(library_provenance_exists_clause(
        "i",
        readable_library_ids,
        &mut args,
    ));
    args.push(SqlArg::I64(limit));

    let sql = format!(
        "WITH candidates AS (
            SELECT {},
                   ROW_NUMBER() OVER (
                       PARTITION BY CASE WHEN TRIM(t.target_key) = '' THEN i.id ELSE t.target_key END
                       ORDER BY COALESCE(i.rank_score, -999999999.0) DESC,
                                COALESCE(t.sort_title, t.display_title) ASC,
                                t.target_key ASC
                   ) AS identity_rank
            FROM discovery_items i
            JOIN discovery_titles t
              ON t.id = i.discovery_title_id
            WHERE {}
         ),
         deduped AS (
            SELECT * FROM candidates WHERE identity_rank = 1
         ),
         ranked AS (
            SELECT *,
                   COUNT(*) OVER () AS total_count
            FROM deduped
         )
         SELECT {}, total_count
         FROM ranked
         ORDER BY COALESCE(rank_score, -999999999.0) DESC,
                  COALESCE(sort_title, display_title) ASC,
                  target_key ASC
        LIMIT {{}}",
        discovery_item_projection("i", "t"),
        clauses.join(" AND "),
        discovery_item_row_columns()
    );
    fetch_catalog_candidates_with_sql(datastore, &sql, &args).await
}

async fn fetch_catalog_candidates_with_sql(
    datastore: &StoreDatastore,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<CatalogDiscoveryCandidatesRecord> {
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), sql, args).await?;
    let total_count = rows
        .first()
        .map(|row| row.i64("total_count"))
        .transpose()?
        .unwrap_or_default();
    let mut items = rows
        .iter()
        .map(item_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_items(datastore, &mut items, &title_ids).await?;
    Ok(CatalogDiscoveryCandidatesRecord { total_count, items })
}

async fn fetch_personalized_facets(
    datastore: &StoreDatastore,
    run_id: &str,
    readable_library_ids: &[String],
    include_unresolved: bool,
) -> AppResult<Vec<DiscoveryFacetRecord>> {
    if readable_library_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut args = vec![SqlArg::Text(run_id.to_string())];
    let mut clauses = vec![
        "i.base_generation_id = {}".to_string(),
        "i.tombstoned_at IS NULL".to_string(),
        "i.owned_in_input = FALSE".to_string(),
        "t.term_kind = 'facet_term'".to_string(),
        "(LOWER(t.term_value) LIKE 'canonical:genre:%'
          OR LOWER(t.term_value) LIKE 'canonical:theme:%')"
            .to_string(),
    ];
    if !include_unresolved {
        clauses.push("dt.resolved = TRUE".to_string());
    }
    clauses.push(library_provenance_exists_clause(
        "i",
        readable_library_ids,
        &mut args,
    ));

    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        &format!(
            "SELECT t.term_value AS facet_term,
                    COUNT(DISTINCT i.id) AS local_count
             FROM discovery_items i
             JOIN discovery_titles dt
               ON dt.id = i.discovery_title_id
             JOIN discovery_title_terms t
               ON t.discovery_title_id = dt.id
             WHERE {}
             GROUP BY t.term_value
             HAVING COUNT(DISTINCT i.id) > 0
             ORDER BY t.term_value ASC",
            clauses.join(" AND ")
        ),
        &args,
    )
    .await?;
    rows.iter()
        .filter_map(|row| canonical_facet_from_row(run_id, row).transpose())
        .collect()
}

async fn query_discovery_items_page(
    datastore: &StoreDatastore,
    query: &DiscoveryItemsStorageQuery,
) -> AppResult<DiscoveryItemsPageRecord> {
    let Some(sql) = build_discovery_items_sql(query) else {
        return Ok(DiscoveryItemsPageRecord {
            items: Vec::new(),
            total_count: 0,
        });
    };

    let count_sql = format!(
        "{}
         SELECT COUNT(*) AS total_count
         FROM deduped
         WHERE identity_rank = 1",
        sql.cte_sql
    );
    let total_count = SqlRuntime::fetch_optional(datastore.read_exec(), &count_sql, &sql.args)
        .await?
        .map(|row| row.i64("total_count"))
        .transpose()?
        .unwrap_or_default();
    if total_count == 0 {
        return Ok(DiscoveryItemsPageRecord {
            items: Vec::new(),
            total_count,
        });
    }

    let mut page_args = sql.args;
    page_args.push(SqlArg::I64(query.limit as i64));
    page_args.push(SqlArg::I64(query.offset as i64));
    let page_sql = format!(
        "{}
         SELECT {}
         FROM deduped
         WHERE identity_rank = 1
         ORDER BY COALESCE(rank_score, -999999999.0) DESC,
                  COALESCE(sort_title, display_title) ASC,
                  target_key ASC
         LIMIT {{}}
         OFFSET {{}}",
        sql.cte_sql,
        discovery_item_row_columns()
    );
    let items = fetch_items_with_sql(datastore, &page_sql, &page_args).await?;
    Ok(DiscoveryItemsPageRecord { items, total_count })
}

fn build_discovery_items_sql(query: &DiscoveryItemsStorageQuery) -> Option<DiscoveryItemsSql> {
    let mut args = Vec::new();
    let mut sources = Vec::new();
    if let Some(context_run_id) = query.context_run_id.as_deref()
        && !query.readable_library_ids.is_empty()
    {
        let mut source_args = vec![SqlArg::Text(context_run_id.to_string())];
        let provenance_clause =
            library_provenance_exists_clause("i", &query.readable_library_ids, &mut source_args);
        args.extend(source_args);
        sources.push(format!(
            "SELECT {}, 0 AS source_priority
             FROM discovery_items i
             JOIN discovery_titles t
               ON t.id = i.discovery_title_id
             WHERE i.base_generation_id = {{}}
               AND i.tombstoned_at IS NULL
               AND {provenance_clause}",
            discovery_item_projection("i", "t")
        ));
    }
    if let Some(public_run_id) = query.public_run_id.as_deref() {
        args.push(SqlArg::Text(public_run_id.to_string()));
        sources.push(format!(
            "SELECT {}, 1 AS source_priority
             FROM discovery_items i
             JOIN discovery_titles t
               ON t.id = i.discovery_title_id
             WHERE i.base_generation_id = {{}}
               AND i.tombstoned_at IS NULL",
            discovery_item_projection("i", "t")
        ));
    }
    if sources.is_empty() {
        return None;
    }

    let mut clauses = Vec::new();
    append_discovery_items_filters(&mut clauses, &mut args, &query.filters);
    let where_clause = if clauses.is_empty() {
        "1 = 1".to_string()
    } else {
        clauses.join(" AND ")
    };

    Some(DiscoveryItemsSql {
        cte_sql: format!(
            "WITH visible AS (
                {}
             ),
             filtered AS (
                SELECT *
                FROM visible i
                WHERE {where_clause}
             ),
             deduped AS (
                SELECT *,
                       ROW_NUMBER() OVER (
                           PARTITION BY CASE WHEN TRIM(target_key) = '' THEN id ELSE target_key END
                           ORDER BY source_priority ASC,
                                    COALESCE(rank_score, -999999999.0) DESC,
                                    COALESCE(sort_title, display_title) ASC,
                                    target_key ASC
                       ) AS identity_rank
                FROM filtered
             )",
            sources.join(" UNION ALL ")
        ),
        args,
    })
}

fn append_discovery_items_filters(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    query: &scryer_application::DiscoveryItemsQuery,
) {
    if !query.include_owned {
        clauses.push("i.owned_in_input = FALSE".to_string());
    }
    if !query.include_unresolved {
        clauses.push("i.resolved = TRUE".to_string());
    }
    if let Some(query_text) = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let pattern = escaped_like_pattern(query_text);
        let text_columns = [
            "i.display_title",
            "i.original_title",
            "i.sort_title",
            "i.overview",
            "i.tmdb_collection_name",
        ];
        clauses.push(format!(
            "({})",
            text_columns
                .iter()
                .map(|column| {
                    args.push(SqlArg::Text(pattern.clone()));
                    format!("LOWER(COALESCE({column}, '')) LIKE {{}} ESCAPE '\\'")
                })
                .collect::<Vec<_>>()
                .join(" OR ")
        ));
    }
    let target_keys = normalized_filter_values(&query.target_keys);
    if !target_keys.is_empty() {
        let placeholders = placeholders(target_keys.len());
        args.extend(target_keys.into_iter().map(SqlArg::Text));
        clauses.push(format!("LOWER(i.target_key) IN ({placeholders})"));
    }
    append_term_filter(clauses, args, "media_kind", &query.target_kinds);
    append_source_filter(clauses, args, &query.sources);
    append_term_filter(clauses, args, "relation_type", &query.relation_types);
    append_term_filter(clauses, args, "relation_subtype", &query.relation_subtypes);
    append_canonical_facet_filter(clauses, args, "genre", &query.genres);
    append_term_filter(clauses, args, "status_tag", &query.status_tags);
    append_term_filter(clauses, args, "facet_term", &query.facet_terms);
}

fn append_term_filter(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    term_kind: &str,
    filters: &[String],
) {
    let values = normalized_filter_values(filters);
    if values.is_empty() {
        return;
    }
    let placeholders = placeholders(values.len());
    args.extend(values.into_iter().map(SqlArg::Text));
    clauses.push(format!(
        "EXISTS (
            SELECT 1
            FROM discovery_title_terms t
            WHERE t.discovery_title_id = i.discovery_title_id
              AND t.term_kind = '{term_kind}'
              AND LOWER(t.term_value) IN ({placeholders})
         )"
    ));
}

fn append_canonical_facet_filter(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    kind: &str,
    filters: &[String],
) {
    let values = canonical_facet_filter_values(kind, filters);
    if values.is_empty() {
        return;
    }
    let placeholders = placeholders(values.len());
    args.extend(values.into_iter().map(SqlArg::Text));
    clauses.push(format!(
        "EXISTS (
            SELECT 1
            FROM discovery_title_terms t
            WHERE t.discovery_title_id = i.discovery_title_id
              AND t.term_kind = 'facet_term'
              AND LOWER(t.term_value) IN ({placeholders})
         )"
    ));
}

fn canonical_facet_filter_values(kind: &str, filters: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for filter in filters {
        let normalized = filter.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if normalized.starts_with(&format!("canonical:{kind}:")) {
            if seen.insert(normalized.clone()) {
                values.push(normalized);
            }
            continue;
        }
        let parts = normalized
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        for separator in ["_", "-", " "] {
            let value = format!("canonical:{kind}:{}", parts.join(separator));
            if seen.insert(value.clone()) {
                values.push(value);
            }
        }
    }
    values
}

fn append_source_filter(clauses: &mut Vec<String>, args: &mut Vec<SqlArg>, filters: &[String]) {
    let values = normalized_filter_values(filters);
    if values.is_empty() {
        return;
    }
    let best_source_placeholders = placeholders(values.len());
    args.extend(values.iter().cloned().map(SqlArg::Text));
    let source_term_placeholders = placeholders(values.len());
    args.extend(values.into_iter().map(SqlArg::Text));
    clauses.push(format!(
        "(LOWER(COALESCE(i.best_source, '')) IN ({best_source_placeholders})
          OR EXISTS (
            SELECT 1
            FROM discovery_title_terms t
            WHERE t.discovery_title_id = i.discovery_title_id
              AND t.term_kind = 'source'
              AND LOWER(t.term_value) IN ({source_term_placeholders})
          ))"
    ));
}

fn normalized_filter_values(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

fn escaped_like_pattern(value: &str) -> String {
    let mut pattern = String::from("%");
    for character in value.trim().to_ascii_lowercase().chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

fn library_provenance_exists_clause(
    item_alias: &str,
    readable_library_ids: &[String],
    args: &mut Vec<SqlArg>,
) -> String {
    let placeholders = placeholders(readable_library_ids.len());
    args.extend(readable_library_ids.iter().cloned().map(SqlArg::Text));
    format!(
        "EXISTS (
            SELECT 1
            FROM discovery_item_library_provenance p
            WHERE p.item_id = {item_alias}.id
              AND p.library_id IN ({placeholders})
         )"
    )
}

fn authoritative_media_kind_clause(item_alias: &str, media_kind: &str) -> String {
    format!(
        "LOWER(COALESCE(NULLIF(TRIM({item_alias}.content_type), ''), {item_alias}.target_kind)) = '{media_kind}'"
    )
}

fn collection_signal_clause(item_alias: &str, title_alias: &str) -> String {
    format!(
        "({title_alias}.tmdb_collection_id IS NOT NULL
          OR TRIM(COALESCE({title_alias}.tmdb_collection_name, '')) <> ''
          OR EXISTS (
            SELECT 1
            FROM discovery_title_terms t
            WHERE t.discovery_title_id = {item_alias}.discovery_title_id
              AND t.term_kind IN ('relation_type', 'relation_subtype')
              AND (
                LOWER(t.term_value) = 'tmdb.collection'
                OR LOWER(t.term_value) LIKE '%collection%'
                OR LOWER(t.term_value) LIKE '%franchise%'
              )
          ))"
    )
}

async fn fetch_items_with_sql(
    datastore: &StoreDatastore,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), sql, args).await?;
    let mut items = rows
        .iter()
        .map(item_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_items(datastore, &mut items, &title_ids).await?;
    Ok(items)
}

async fn discovery_run_language(
    datastore: &StoreDatastore,
    run_id: &str,
) -> AppResult<Option<String>> {
    SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT language FROM discovery_sync_runs WHERE id = {}",
        &[SqlArg::Text(run_id.to_string())],
    )
    .await?
    .map(|row| row.text("language"))
    .transpose()
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

fn upsert_discovery_title_sql() -> String {
    format!(
        "INSERT INTO discovery_titles ({}) VALUES ({})
         ON CONFLICT(target_key_norm, language) DO UPDATE SET
            target_key = COALESCE(NULLIF(excluded.target_key, ''), discovery_titles.target_key),
            target_kind = COALESCE(NULLIF(excluded.target_kind, ''), discovery_titles.target_kind),
            resolved = CASE
                WHEN excluded.resolved THEN excluded.resolved
                ELSE discovery_titles.resolved
            END,
            resolved_title_id = COALESCE(
                NULLIF(excluded.resolved_title_id, ''),
                discovery_titles.resolved_title_id
            ),
            display_title = COALESCE(
                NULLIF(excluded.display_title, ''),
                discovery_titles.display_title
            ),
            original_title = COALESCE(
                NULLIF(excluded.original_title, ''),
                discovery_titles.original_title
            ),
            sort_title = COALESCE(NULLIF(excluded.sort_title, ''), discovery_titles.sort_title),
            year = COALESCE(excluded.year, discovery_titles.year),
            poster_path = COALESCE(NULLIF(excluded.poster_path, ''), discovery_titles.poster_path),
            poster_url = COALESCE(NULLIF(excluded.poster_url, ''), discovery_titles.poster_url),
            background_url = COALESCE(
                NULLIF(excluded.background_url, ''),
                discovery_titles.background_url
            ),
            overview = COALESCE(NULLIF(excluded.overview, ''), discovery_titles.overview),
            content_type = COALESCE(
                NULLIF(excluded.content_type, ''),
                discovery_titles.content_type
            ),
            rating = COALESCE(excluded.rating, discovery_titles.rating),
            tmdb_collection_id = COALESCE(
                NULLIF(excluded.tmdb_collection_id, ''),
                discovery_titles.tmdb_collection_id
            ),
            tmdb_collection_name = COALESCE(
                NULLIF(excluded.tmdb_collection_name, ''),
                discovery_titles.tmdb_collection_name
            ),
            updated_at = excluded.updated_at",
        TITLE_COLUMNS.join(", "),
        placeholders(TITLE_COLUMNS.len())
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

async fn delete_item_children_for_run_tx(tx: &mut SqlTx<'_>, run_id: &str) -> AppResult<()> {
    for table in [
        "discovery_section_items",
        "discovery_item_rank_components",
        "discovery_item_subject_links",
        "discovery_item_library_provenance",
    ] {
        delete_for_run_tx(tx, table, run_id).await?;
    }
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
               AND discovery_title_id IN (
                    SELECT id
                    FROM discovery_titles
                    WHERE target_key = {}
               )
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
        sort_index: row.i32("sort_index")?,
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
        sort_index: row.i32("sort_index")?,
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
        genres: Vec::new(),
        rating: row.opt_f64("rating")?,
        rating_sources: Vec::new(),
        external_ratings: Vec::new(),
        external_ids: Vec::new(),
        status_tags: Vec::new(),
        source_tags: Vec::new(),
        sources: Vec::new(),
        best_source: row.opt_text("best_source")?,
        relation_types: Vec::new(),
        relation_subtypes: Vec::new(),
        chart_signals: Vec::new(),
        provider_signals: Vec::new(),
        rank_components: Vec::new(),
        source_count: row.opt_i32("source_count")?,
        edge_count: row.opt_i32("edge_count")?,
        relation_count: row.opt_i32("relation_count")?,
        source_subject_count: row.opt_i32("source_subject_count")?,
        rank_score: row.opt_f64("rank_score")?,
        matched_subject_keys: Vec::new(),
        matched_subject_titles: Vec::new(),
        matched_subject_count: row.i32("matched_subject_count")?,
        library_provenance: Vec::new(),
        tmdb_collection_id: row.opt_text("tmdb_collection_id")?,
        tmdb_collection_name: row.opt_text("tmdb_collection_name")?,
        owned_in_input: row.bool("owned_in_input")?,
        facet_terms: Vec::new(),
        context_terms: Vec::new(),
        change_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
        tombstoned_by_run_id: row.opt_text("tombstoned_by_run_id")?,
        tombstoned_at: row.opt_timestamp("tombstoned_at")?,
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
    })
}

fn canonical_facet_from_row(run_id: &str, row: &SqlRow) -> AppResult<Option<DiscoveryFacetRecord>> {
    let facet_term = row.text("facet_term")?;
    let Some((facet_name, facet_value)) = canonical_facet_display_value(&facet_term) else {
        return Ok(None);
    };
    Ok(Some(DiscoveryFacetRecord {
        run_id: run_id.to_string(),
        facet_name,
        facet_value,
        smg_count: None,
        local_count: row.opt_i64("local_count")?,
    }))
}

fn canonical_facet_display_value(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    let mut parts = value.splitn(3, ':');
    if !parts.next()?.eq_ignore_ascii_case("canonical") {
        return None;
    }
    let kind = parts.next()?.trim();
    if !kind.eq_ignore_ascii_case("genre") && !kind.eq_ignore_ascii_case("theme") {
        return None;
    }
    let tail = parts.next()?.trim();
    if tail.is_empty() {
        return None;
    }
    Some((kind.to_ascii_lowercase(), canonical_label_from_slug(tail)))
}

fn canonical_label_from_slug(value: &str) -> String {
    value
        .split(|character: char| {
            character == '-' || character == '_' || character == ':' || character.is_whitespace()
        })
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => {
                    let mut word = first.to_uppercase().collect::<String>();
                    word.extend(characters.flat_map(char::to_lowercase));
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn hydrate_discovery_items(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
) -> AppResult<()> {
    if items.is_empty() {
        return Ok(());
    }
    let item_ids = items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
    let mut item_indexes = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        item_indexes.insert(item.id.clone(), index);
    }
    hydrate_discovery_title_children(datastore, items, discovery_title_ids).await?;
    hydrate_item_rank_components(datastore, items, &item_ids, &item_indexes).await?;
    hydrate_item_subject_links(datastore, items, &item_ids, &item_indexes).await?;
    hydrate_item_library_provenance(datastore, items, &item_ids, &item_indexes).await?;
    Ok(())
}

fn discovery_title_ids_from_rows(rows: &[SqlRow]) -> AppResult<Vec<String>> {
    rows.iter()
        .map(|row| row.text("discovery_title_id"))
        .collect()
}

async fn hydrate_discovery_title_children(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
) -> AppResult<()> {
    if items.is_empty() {
        return Ok(());
    }
    let mut title_indexes = HashMap::<String, Vec<usize>>::new();
    for (index, title_id) in discovery_title_ids.iter().enumerate() {
        if !title_id.trim().is_empty() {
            title_indexes
                .entry(title_id.clone())
                .or_default()
                .push(index);
        }
    }
    if title_indexes.is_empty() {
        return Ok(());
    }
    let mut unique_title_ids = title_indexes.keys().cloned().collect::<Vec<_>>();
    unique_title_ids.sort();
    hydrate_title_terms(datastore, items, &unique_title_ids, &title_indexes).await?;
    hydrate_title_source_tags(datastore, items, &unique_title_ids, &title_indexes).await?;
    hydrate_title_ratings(datastore, items, &unique_title_ids, &title_indexes).await?;
    hydrate_title_external_ids(datastore, items, &unique_title_ids, &title_indexes).await?;
    Ok(())
}

async fn hydrate_title_terms(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<()> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, term_kind, term_category, term_value, sort_index
         FROM discovery_title_terms
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, term_kind ASC, sort_index ASC, term_value ASC",
        discovery_title_ids,
    )
    .await?;
    for row in rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        let term_kind = row.text("term_kind")?;
        let term_value = row.text("term_value")?;
        for index in indexes {
            let item = &mut items[*index];
            match term_kind.as_str() {
                "genre" => item.genres.push(term_value.clone()),
                "rating_source" => {
                    if !item.rating_sources.iter().any(|value| value == &term_value) {
                        item.rating_sources.push(term_value.clone());
                    }
                }
                "status_tag" => item.status_tags.push(term_value.clone()),
                "source" => item.sources.push(term_value.clone()),
                "relation_type" => item.relation_types.push(term_value.clone()),
                "relation_subtype" => item.relation_subtypes.push(term_value.clone()),
                "chart_signal" => item.chart_signals.push(term_value.clone()),
                "provider_signal" => item.provider_signals.push(term_value.clone()),
                "facet_term" => item.facet_terms.push(term_value.clone()),
                "context_term" => item.context_terms.push(term_value.clone()),
                _ => {}
            }
        }
    }
    Ok(())
}

async fn hydrate_title_source_tags(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<()> {
    let mut source_tag_indexes = HashMap::<(String, i32), Vec<(usize, usize)>>::new();
    let rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, category, name, sort_index
         FROM discovery_title_source_tags
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, sort_index ASC",
        discovery_title_ids,
    )
    .await?;
    for row in rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        let sort_index = row.i32("sort_index")?;
        let category = empty_to_none(row.text("category")?);
        let name = empty_to_none(row.text("name")?);
        let mut source_tag_item_indexes = Vec::new();
        for index in indexes {
            let source_tag_index = items[*index].source_tags.len();
            items[*index].source_tags.push(DiscoverySourceTagRecord {
                category: category.clone(),
                name: name.clone(),
                values: Vec::new(),
            });
            source_tag_item_indexes.push((*index, source_tag_index));
        }
        source_tag_indexes.insert((discovery_title_id, sort_index), source_tag_item_indexes);
    }

    let value_rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, source_tag_sort_index, source_tag_value, value_sort_index
         FROM discovery_title_source_tag_values
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, source_tag_sort_index ASC, value_sort_index ASC",
        discovery_title_ids,
    )
    .await?;
    for row in value_rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let source_tag_sort_index = row.i32("source_tag_sort_index")?;
        let Some(source_tag_item_indexes) =
            source_tag_indexes.get(&(discovery_title_id, source_tag_sort_index))
        else {
            continue;
        };
        let source_tag_value = row.text("source_tag_value")?;
        for (item_index, source_tag_index) in source_tag_item_indexes {
            items[*item_index].source_tags[*source_tag_index]
                .values
                .push(source_tag_value.clone());
        }
    }
    Ok(())
}

async fn hydrate_title_ratings(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<()> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, rating_source, rating_value, rating_score, normalized, votes, url, sort_index
         FROM discovery_title_ratings
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, sort_index ASC",
        discovery_title_ids,
    )
    .await?;
    for row in rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        let source = row.text("rating_source")?;
        let rating = if let Some(normalized) = row.opt_f64("normalized")? {
            Some(TitleExternalRating {
                source: source.clone(),
                value: row.opt_f64("rating_value")?,
                score: row.opt_f64("rating_score")?,
                normalized,
                votes: row.opt_i32("votes")?,
                url: row.opt_text("url")?.unwrap_or_default(),
            })
        } else {
            None
        };
        for index in indexes {
            if !items[*index]
                .rating_sources
                .iter()
                .any(|value| value == &source)
            {
                items[*index].rating_sources.push(source.clone());
            }
            if let Some(rating) = &rating {
                items[*index].external_ratings.push(rating.clone());
            }
        }
    }
    Ok(())
}

async fn hydrate_title_external_ids(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<()> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, source, external_kind, external_id, external_key, sort_index
         FROM discovery_title_external_ids
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, sort_index ASC, source ASC, external_kind ASC",
        discovery_title_ids,
    )
    .await?;
    for row in rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        let external_id = DiscoveryExternalIdRecord {
            source: row.text("source")?,
            kind: row.text("external_kind")?,
            id: row.text("external_id")?,
            key: row.text("external_key")?,
        };
        for index in indexes {
            items[*index].external_ids.push(external_id.clone());
        }
    }
    Ok(())
}

async fn hydrate_item_rank_components(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    item_ids: &[String],
    item_indexes: &HashMap<String, usize>,
) -> AppResult<()> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT item_id, component_index, component_name, component_value
         FROM discovery_item_rank_components
         WHERE item_id IN ({})
         ORDER BY item_id ASC, component_index ASC",
        item_ids,
    )
    .await?;
    for row in rows {
        let item_id = row.text("item_id")?;
        let Some(index) = item_indexes.get(&item_id).copied() else {
            continue;
        };
        items[index]
            .rank_components
            .push(DiscoveryRankComponentRecord {
                component_index: row.i32("component_index")?,
                component_name: empty_to_none(row.text("component_name")?),
                component_value: empty_to_none(row.text("component_value")?),
            });
    }
    Ok(())
}

async fn hydrate_item_subject_links(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    item_ids: &[String],
    item_indexes: &HashMap<String, usize>,
) -> AppResult<()> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT item_id, link_type, subject_key, sort_index
         FROM discovery_item_subject_links
         WHERE item_id IN ({})
         ORDER BY item_id ASC, link_type ASC, sort_index ASC",
        item_ids,
    )
    .await?;
    for row in rows {
        let item_id = row.text("item_id")?;
        let Some(index) = item_indexes.get(&item_id).copied() else {
            continue;
        };
        let link_type = row.text("link_type")?;
        let subject_key = row.text("subject_key")?;
        match link_type.as_str() {
            "matched" => items[index].matched_subject_keys.push(subject_key),
            "change" => items[index].change_subject_keys.push(subject_key),
            "removed" => items[index].removed_subject_keys.push(subject_key),
            _ => {}
        }
    }
    Ok(())
}

async fn hydrate_item_library_provenance(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    item_ids: &[String],
    item_indexes: &HashMap<String, usize>,
) -> AppResult<()> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT item_id, subject_key, title_id, library_id
         FROM discovery_item_library_provenance
         WHERE item_id IN ({})
         ORDER BY item_id ASC, subject_key ASC, library_id ASC, title_id ASC",
        item_ids,
    )
    .await?;
    for row in rows {
        let item_id = row.text("item_id")?;
        let Some(index) = item_indexes.get(&item_id).copied() else {
            continue;
        };
        items[index]
            .library_provenance
            .push(DiscoveryItemLibraryProvenanceRecord {
                subject_key: row.text("subject_key")?,
                title_id: empty_to_none(row.text("title_id")?),
                library_id: empty_to_none(row.text("library_id")?),
            });
    }
    Ok(())
}

async fn fetch_child_rows(
    datastore: &StoreDatastore,
    sql_template: &str,
    item_ids: &[String],
) -> AppResult<Vec<SqlRow>> {
    let sql = sql_template.replace("{}", &placeholders(item_ids.len()));
    let args = item_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await
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
    _datastore: &StoreDatastore,
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
            SqlArg::I32(section.sort_index),
            SqlArg::Timestamp(section.created_at),
            SqlArg::Timestamp(section.updated_at),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_item_tx(
    tx: &mut SqlTx<'_>,
    _datastore: &StoreDatastore,
    item: &DiscoveryItemRecord,
    language: &str,
) -> AppResult<()> {
    let discovery_title_id = upsert_discovery_title_tx(tx, item, language).await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &insert_sql("discovery_items", OCCURRENCE_COLUMNS),
        &occurrence_args(item, &discovery_title_id),
    )
    .await?;
    insert_item_children_tx(tx, item).await?;
    Ok(())
}

async fn delete_title_more_like_this_items_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM title_more_like_this_items WHERE source_title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

async fn delete_unreferenced_discovery_titles_tx(tx: &mut SqlTx<'_>) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM discovery_titles
         WHERE NOT EXISTS (
            SELECT 1
            FROM discovery_items i
            WHERE i.discovery_title_id = discovery_titles.id
         )
         AND NOT EXISTS (
            SELECT 1
            FROM title_more_like_this_items m
            WHERE m.discovery_title_id = discovery_titles.id
         )",
        &[],
    )
    .await?;
    Ok(())
}

async fn delete_unreferenced_discovery_titles(datastore: &StoreDatastore) -> AppResult<()> {
    SqlRuntime::execute(
        datastore.read_exec(),
        "DELETE FROM discovery_titles
         WHERE NOT EXISTS (
            SELECT 1
            FROM discovery_items i
            WHERE i.discovery_title_id = discovery_titles.id
         )
         AND NOT EXISTS (
            SELECT 1
            FROM title_more_like_this_items m
            WHERE m.discovery_title_id = discovery_titles.id
         )",
        &[],
    )
    .await?;
    Ok(())
}

async fn insert_title_more_like_this_item_tx(
    tx: &mut SqlTx<'_>,
    _datastore: &StoreDatastore,
    title_id: &str,
    item: &DiscoveryItemRecord,
    language: &str,
) -> AppResult<()> {
    let discovery_title_id = upsert_discovery_title_tx(tx, item, language).await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO title_more_like_this_items
         (source_title_id, discovery_title_id, sort_index, rank_score, best_source,
          source_count, edge_count, relation_count, source_subject_count, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(source_title_id, discovery_title_id) DO UPDATE SET
            sort_index = excluded.sort_index,
            rank_score = excluded.rank_score,
            best_source = excluded.best_source,
            source_count = excluded.source_count,
            edge_count = excluded.edge_count,
            relation_count = excluded.relation_count,
            source_subject_count = excluded.source_subject_count,
            updated_at = excluded.updated_at",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(discovery_title_id),
            SqlArg::I32(item.sort_index),
            SqlArg::OptF64(item.rank_score),
            SqlArg::OptText(item.best_source.clone()),
            SqlArg::OptI32(item.source_count),
            SqlArg::OptI32(item.edge_count),
            SqlArg::OptI32(item.relation_count),
            SqlArg::OptI32(item.source_subject_count),
            SqlArg::Timestamp(item.created_at),
            SqlArg::Timestamp(item.updated_at),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_facet_tx(
    tx: &mut SqlTx<'_>,
    _datastore: &StoreDatastore,
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

async fn upsert_discovery_title_tx(
    tx: &mut SqlTx<'_>,
    item: &DiscoveryItemRecord,
    language: &str,
) -> AppResult<String> {
    let language = normalize_discovery_language(language);
    let target_key_norm = discovery_title_target_key_norm(item);
    let discovery_title_id = discovery_title_id_for(&target_key_norm, &language);
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &upsert_discovery_title_sql(),
        &title_args(item, &discovery_title_id, &target_key_norm, &language),
    )
    .await?;
    insert_title_children_tx(tx, item, &discovery_title_id).await?;
    Ok(discovery_title_id)
}

fn title_args(
    item: &DiscoveryItemRecord,
    discovery_title_id: &str,
    target_key_norm: &str,
    language: &str,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(discovery_title_id.to_string()),
        SqlArg::Text(item.target_key.clone()),
        SqlArg::Text(target_key_norm.to_string()),
        SqlArg::Text(language.to_string()),
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
        SqlArg::OptF64(item.rating),
        SqlArg::OptText(item.tmdb_collection_id.clone()),
        SqlArg::OptText(item.tmdb_collection_name.clone()),
        SqlArg::Timestamp(item.created_at),
        SqlArg::Timestamp(item.updated_at),
    ]
}

fn occurrence_args(item: &DiscoveryItemRecord, discovery_title_id: &str) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(item.id.clone()),
        SqlArg::Text(item.run_id.clone()),
        SqlArg::OptText(item.base_generation_id.clone()),
        SqlArg::Text(discovery_title_id.to_string()),
        SqlArg::Text(item.source_run_kind.clone()),
        SqlArg::OptText(item.section_id.clone()),
        SqlArg::I32(item.sort_index),
        SqlArg::OptText(item.best_source.clone()),
        SqlArg::OptI32(item.source_count),
        SqlArg::OptI32(item.edge_count),
        SqlArg::OptI32(item.relation_count),
        SqlArg::OptI32(item.source_subject_count),
        SqlArg::OptF64(item.rank_score),
        SqlArg::I32(item.matched_subject_count),
        SqlArg::Bool(item.owned_in_input),
        SqlArg::OptText(item.tombstoned_by_run_id.clone()),
        SqlArg::OptTimestamp(item.tombstoned_at),
        SqlArg::Timestamp(item.created_at),
        SqlArg::Timestamp(item.updated_at),
    ]
}

async fn insert_item_children_tx(tx: &mut SqlTx<'_>, item: &DiscoveryItemRecord) -> AppResult<()> {
    if let Some(section_id) = item.section_id.as_deref() {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO discovery_section_items
             (run_id, section_id, item_id, sort_index)
             VALUES ({}, {}, {}, {})",
            &[
                SqlArg::Text(item.run_id.clone()),
                SqlArg::Text(section_id.to_string()),
                SqlArg::Text(item.id.clone()),
                SqlArg::I32(item.sort_index),
            ],
        )
        .await?;
    }

    for rank_component in &item.rank_components {
        insert_rank_component_tx(tx, item, rank_component).await?;
    }
    insert_subject_links_tx(tx, item, "matched", &item.matched_subject_keys).await?;
    insert_subject_links_tx(tx, item, "change", &item.change_subject_keys).await?;
    insert_subject_links_tx(tx, item, "removed", &item.removed_subject_keys).await?;
    for provenance in &item.library_provenance {
        insert_library_provenance_tx(tx, item, provenance).await?;
    }
    Ok(())
}

async fn insert_title_children_tx(
    tx: &mut SqlTx<'_>,
    item: &DiscoveryItemRecord,
    discovery_title_id: &str,
) -> AppResult<()> {
    insert_title_terms_tx(tx, discovery_title_id, "genre", None, &item.genres).await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "rating_source",
        None,
        &item.rating_sources,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "status_tag",
        None,
        &item.status_tags,
    )
    .await?;
    insert_title_terms_tx(tx, discovery_title_id, "source", None, &item.sources).await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "relation_type",
        None,
        &item.relation_types,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "relation_subtype",
        None,
        &item.relation_subtypes,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "chart_signal",
        None,
        &item.chart_signals,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "provider_signal",
        None,
        &item.provider_signals,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "facet_term",
        None,
        &item.facet_terms,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "context_term",
        None,
        &item.context_terms,
    )
    .await?;
    if let Some(media_kind) = discovery_item_authoritative_media_kind(item) {
        insert_title_terms_tx(tx, discovery_title_id, "media_kind", None, &[media_kind]).await?;
    }
    for (index, source_tag) in item.source_tags.iter().enumerate() {
        insert_title_source_tag_tx(tx, discovery_title_id, source_tag, index as i32).await?;
    }
    for (index, external_id) in item.external_ids.iter().enumerate() {
        insert_title_external_id_tx(tx, discovery_title_id, external_id, index as i32).await?;
    }
    if item.external_ratings.is_empty() {
        for (index, source) in item.rating_sources.iter().enumerate() {
            insert_title_rating_tx(tx, discovery_title_id, source, None, index as i32).await?;
        }
    } else {
        for (index, rating) in item.external_ratings.iter().enumerate() {
            insert_title_rating_tx(
                tx,
                discovery_title_id,
                &rating.source,
                Some(rating),
                index as i32,
            )
            .await?;
        }
    }
    Ok(())
}

async fn insert_title_terms_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    term_kind: &str,
    term_category: Option<&str>,
    values: &[String],
) -> AppResult<()> {
    for (index, value) in values.iter().enumerate() {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO discovery_title_terms
             (discovery_title_id, term_kind, term_category, term_value, sort_index)
             VALUES ({}, {}, {}, {}, {})
             ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(discovery_title_id.to_string()),
                SqlArg::Text(term_kind.to_string()),
                SqlArg::Text(storage_text(term_category)),
                SqlArg::Text(value.to_string()),
                SqlArg::I32(index as i32),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_title_source_tag_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    source_tag: &DiscoverySourceTagRecord,
    index: i32,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_title_source_tags
         (discovery_title_id, category, name, sort_index)
         VALUES ({}, {}, {}, {})
         ON CONFLICT DO NOTHING",
        &[
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::Text(storage_text(source_tag.category.as_deref())),
            SqlArg::Text(storage_text(source_tag.name.as_deref())),
            SqlArg::I32(index),
        ],
    )
    .await?;
    if let Some(name) = source_tag.name.as_deref() {
        insert_title_terms_tx(
            tx,
            discovery_title_id,
            "source_tag",
            source_tag.category.as_deref(),
            &[name.to_string()],
        )
        .await?;
    }
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "source_tag_value",
        source_tag.category.as_deref(),
        &source_tag.values,
    )
    .await?;
    for (value_index, value) in source_tag.values.iter().enumerate() {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        insert_title_source_tag_value_tx(tx, discovery_title_id, index, value, value_index as i32)
            .await?;
    }
    Ok(())
}

async fn insert_title_source_tag_value_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    source_tag_sort_index: i32,
    value: &str,
    value_sort_index: i32,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_title_source_tag_values
         (discovery_title_id, source_tag_sort_index, source_tag_value, value_sort_index)
         VALUES ({}, {}, {}, {})
         ON CONFLICT DO NOTHING",
        &[
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::I32(source_tag_sort_index),
            SqlArg::Text(value.to_string()),
            SqlArg::I32(value_sort_index),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_title_external_id_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    external_id: &DiscoveryExternalIdRecord,
    index: i32,
) -> AppResult<()> {
    let source = external_id.source.trim();
    let id = external_id.id.trim();
    let key = external_id.key.trim();
    if source.is_empty() || (id.is_empty() && key.is_empty()) {
        return Ok(());
    }
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_title_external_ids
         (discovery_title_id, source, external_kind, external_id, external_key, sort_index)
         VALUES ({}, {}, {}, {}, {}, {})
         ON CONFLICT(discovery_title_id, source, external_kind, external_id, external_key)
         DO UPDATE SET
            sort_index = CASE
                WHEN discovery_title_external_ids.sort_index <= excluded.sort_index
                    THEN discovery_title_external_ids.sort_index
                ELSE excluded.sort_index
            END",
        &[
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::Text(source.to_ascii_lowercase()),
            SqlArg::Text(external_id.kind.trim().to_ascii_lowercase()),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(key.to_string()),
            SqlArg::I32(index),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_title_rating_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    source: &str,
    rating: Option<&TitleExternalRating>,
    index: i32,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_title_ratings
         (discovery_title_id, rating_source, rating_value, rating_score, normalized, votes, url, sort_index)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(discovery_title_id, rating_source) DO UPDATE SET
            rating_value = COALESCE(excluded.rating_value, discovery_title_ratings.rating_value),
            rating_score = COALESCE(excluded.rating_score, discovery_title_ratings.rating_score),
            normalized = COALESCE(excluded.normalized, discovery_title_ratings.normalized),
            votes = COALESCE(excluded.votes, discovery_title_ratings.votes),
            url = COALESCE(NULLIF(excluded.url, ''), discovery_title_ratings.url),
            sort_index = CASE
                WHEN discovery_title_ratings.sort_index <= excluded.sort_index
                    THEN discovery_title_ratings.sort_index
                ELSE excluded.sort_index
            END",
        &[
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::Text(source.to_string()),
            SqlArg::OptF64(rating.and_then(|rating| rating.value)),
            SqlArg::OptF64(rating.and_then(|rating| rating.score)),
            SqlArg::OptF64(rating.map(|rating| rating.normalized)),
            SqlArg::OptI32(rating.and_then(|rating| rating.votes)),
            SqlArg::Text(rating.map(|rating| rating.url.clone()).unwrap_or_default()),
            SqlArg::I32(index),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_rank_component_tx(
    tx: &mut SqlTx<'_>,
    item: &DiscoveryItemRecord,
    component: &DiscoveryRankComponentRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_item_rank_components
         (item_id, run_id, component_index, component_name, component_value)
         VALUES ({}, {}, {}, {}, {})
         ON CONFLICT DO NOTHING",
        &[
            SqlArg::Text(item.id.clone()),
            SqlArg::Text(item.run_id.clone()),
            SqlArg::I32(component.component_index),
            SqlArg::Text(storage_text(component.component_name.as_deref())),
            SqlArg::Text(storage_text(component.component_value.as_deref())),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_subject_links_tx(
    tx: &mut SqlTx<'_>,
    item: &DiscoveryItemRecord,
    link_type: &str,
    subject_keys: &[String],
) -> AppResult<()> {
    for (index, subject_key) in subject_keys.iter().enumerate() {
        let subject_key = subject_key.trim();
        if subject_key.is_empty() {
            continue;
        }
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO discovery_item_subject_links
             (item_id, run_id, link_type, subject_key, sort_index)
             VALUES ({}, {}, {}, {}, {})
             ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(item.id.clone()),
                SqlArg::Text(item.run_id.clone()),
                SqlArg::Text(link_type.to_string()),
                SqlArg::Text(subject_key.to_string()),
                SqlArg::I32(index as i32),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_library_provenance_tx(
    tx: &mut SqlTx<'_>,
    item: &DiscoveryItemRecord,
    provenance: &DiscoveryItemLibraryProvenanceRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_item_library_provenance
         (item_id, run_id, subject_key, title_id, library_id)
         VALUES ({}, {}, {}, {}, {})
         ON CONFLICT DO NOTHING",
        &[
            SqlArg::Text(item.id.clone()),
            SqlArg::Text(item.run_id.clone()),
            SqlArg::Text(provenance.subject_key.clone()),
            SqlArg::Text(storage_text(provenance.title_id.as_deref())),
            SqlArg::Text(storage_text(provenance.library_id.as_deref())),
        ],
    )
    .await?;
    Ok(())
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
    use scryer_application::{
        DISCOVERY_DEFAULT_SCOPE_KEY, DiscoveryItemsQuery, DiscoveryItemsStorageQuery,
    };
    use serde_json::json;

    use crate::storage::sqlite::services::SqliteServices;

    #[test]
    fn canonical_facet_filter_values_accepts_labels_and_terms() {
        let values = canonical_facet_filter_values(
            "genre",
            &[
                "Action".to_string(),
                "Science Fiction".to_string(),
                "canonical:genre:drama".to_string(),
            ],
        );

        assert!(values.contains(&"canonical:genre:action".to_string()));
        assert!(values.contains(&"canonical:genre:science_fiction".to_string()));
        assert!(values.contains(&"canonical:genre:science-fiction".to_string()));
        assert!(values.contains(&"canonical:genre:science fiction".to_string()));
        assert!(values.contains(&"canonical:genre:drama".to_string()));
        assert!(!values.contains(&"action".to_string()));
    }

    #[test]
    fn canonical_facet_display_value_accepts_only_genre_and_theme_terms() {
        assert_eq!(
            canonical_facet_display_value("canonical:genre:science_fiction"),
            Some(("genre".to_string(), "Science Fiction".to_string()))
        );
        assert_eq!(
            canonical_facet_display_value("canonical:theme:psychological"),
            Some(("theme".to_string(), "Psychological".to_string()))
        );
        assert_eq!(canonical_facet_display_value("Drama"), None);
        assert_eq!(
            canonical_facet_display_value("mal:theme:psychological"),
            None
        );
        assert_eq!(canonical_facet_display_value("canonical:source:tmdb"), None);
    }

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
                    sort_index: 0,
                    created_at: now,
                    updated_at: now,
                }],
            )
            .await
            .expect("sections should replace");
        store
            .replace_discovery_items(
                "run-1",
                &[
                    DiscoveryItemRecord {
                        id: "item-row-1".to_string(),
                        run_id: "run-1".to_string(),
                        base_generation_id: Some("run-1".to_string()),
                        source_run_kind: "context_incremental".to_string(),
                        section_id: Some("for_you".to_string()),
                        sort_index: 0,
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
                        background_url: Some(
                            "https://images.example.test/movie-bg.jpg".to_string(),
                        ),
                        overview: Some("Rich canonical overview".to_string()),
                        content_type: Some(String::new()),
                        genres: vec!["Drama".to_string(), "Drama".to_string()],
                        rating: Some(7.5),
                        rating_sources: vec!["tmdb".to_string(), "tmdb".to_string()],
                        external_ratings: vec![TitleExternalRating {
                            source: "imdb".to_string(),
                            value: Some(8.2),
                            score: Some(82.0),
                            normalized: 0.82,
                            votes: Some(1234),
                            url: "https://www.imdb.com/title/tt0000010/".to_string(),
                        }],
                        external_ids: vec![
                            DiscoveryExternalIdRecord {
                                source: "tmdb".to_string(),
                                kind: "movie".to_string(),
                                id: "10".to_string(),
                                key: "tmdb:movie:10".to_string(),
                            },
                            DiscoveryExternalIdRecord {
                                source: "imdb".to_string(),
                                kind: "movie".to_string(),
                                id: "tt0000010".to_string(),
                                key: "imdb:movie:tt0000010".to_string(),
                            },
                        ],
                        status_tags: vec!["available".to_string()],
                        source_tags: vec![DiscoverySourceTagRecord {
                            category: Some("theme".to_string()),
                            name: Some("Isekai".to_string()),
                            values: vec![
                                "theme".to_string(),
                                "Isekai".to_string(),
                                "Isekai".to_string(),
                            ],
                        }],
                        sources: vec!["smg".to_string()],
                        best_source: None,
                        relation_types: Vec::new(),
                        relation_subtypes: Vec::new(),
                        chart_signals: vec!["trending".to_string()],
                        provider_signals: Vec::new(),
                        rank_components: vec![DiscoveryRankComponentRecord {
                            component_index: 0,
                            component_name: Some("score".to_string()),
                            component_value: Some("0.42".to_string()),
                        }],
                        source_count: Some(1),
                        edge_count: Some(1),
                        relation_count: Some(0),
                        source_subject_count: Some(1),
                        rank_score: Some(0.42),
                        matched_subject_keys: vec![
                            "tvdb:series:1".to_string(),
                            "tvdb:series:1".to_string(),
                        ],
                        matched_subject_titles: vec!["Example Series".to_string()],
                        matched_subject_count: 1,
                        library_provenance: vec![
                            DiscoveryItemLibraryProvenanceRecord {
                                subject_key: "tvdb:series:1".to_string(),
                                title_id: None,
                                library_id: Some("series-library-a".to_string()),
                            },
                            DiscoveryItemLibraryProvenanceRecord {
                                subject_key: "tvdb:series:1".to_string(),
                                title_id: None,
                                library_id: Some("series-library-a".to_string()),
                            },
                        ],
                        tmdb_collection_id: None,
                        tmdb_collection_name: None,
                        owned_in_input: false,
                        facet_terms: vec![
                            "Drama".to_string(),
                            "canonical:genre:drama".to_string(),
                            "mal:theme:psychological".to_string(),
                            "canonical:theme:isekai".to_string(),
                        ],
                        context_terms: Vec::new(),
                        change_subject_keys: vec!["tvdb:series:1".to_string()],
                        removed_subject_keys: Vec::new(),
                        tombstoned_by_run_id: None,
                        tombstoned_at: None,
                        created_at: now,
                        updated_at: now,
                    },
                    DiscoveryItemRecord {
                        id: "item-row-raw-only".to_string(),
                        run_id: "run-1".to_string(),
                        base_generation_id: Some("run-1".to_string()),
                        source_run_kind: "context_incremental".to_string(),
                        section_id: None,
                        sort_index: 1,
                        target_key: "tvdb:series:2".to_string(),
                        target_kind: "series".to_string(),
                        resolved: true,
                        resolved_title_id: None,
                        display_title: "Raw Label Series".to_string(),
                        original_title: None,
                        sort_title: Some("Raw Label Series".to_string()),
                        year: Some(2026),
                        poster_path: None,
                        poster_url: None,
                        background_url: None,
                        overview: None,
                        content_type: Some("series".to_string()),
                        genres: vec!["Drama".to_string()],
                        rating: None,
                        rating_sources: Vec::new(),
                        external_ratings: Vec::new(),
                        external_ids: Vec::new(),
                        status_tags: Vec::new(),
                        source_tags: Vec::new(),
                        sources: vec!["smg".to_string()],
                        best_source: None,
                        relation_types: Vec::new(),
                        relation_subtypes: Vec::new(),
                        chart_signals: Vec::new(),
                        provider_signals: Vec::new(),
                        rank_components: Vec::new(),
                        source_count: Some(1),
                        edge_count: Some(0),
                        relation_count: Some(0),
                        source_subject_count: Some(1),
                        rank_score: Some(0.1),
                        matched_subject_keys: vec!["tvdb:series:1".to_string()],
                        matched_subject_titles: vec!["Example Series".to_string()],
                        matched_subject_count: 1,
                        library_provenance: vec![DiscoveryItemLibraryProvenanceRecord {
                            subject_key: "tvdb:series:1".to_string(),
                            title_id: None,
                            library_id: Some("series-library-a".to_string()),
                        }],
                        tmdb_collection_id: None,
                        tmdb_collection_name: None,
                        owned_in_input: false,
                        facet_terms: vec![
                            "Drama".to_string(),
                            "mal:theme:psychological".to_string(),
                        ],
                        context_terms: Vec::new(),
                        change_subject_keys: Vec::new(),
                        removed_subject_keys: Vec::new(),
                        tombstoned_by_run_id: None,
                        tombstoned_at: None,
                        created_at: now,
                        updated_at: now,
                    },
                ],
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
        assert_eq!(read_items.len(), 2);
        let read_item = read_items
            .iter()
            .find(|item| item.id == "item-row-1")
            .expect("canonical fixture item should round trip");
        assert_eq!(read_item.target_key, "tmdb:movie:10");
        assert_eq!(
            read_item.background_url.as_deref(),
            Some("https://images.example.test/movie-bg.jpg")
        );
        assert_eq!(
            read_item.overview.as_deref(),
            Some("Rich canonical overview")
        );
        assert_eq!(read_item.genres, vec!["Drama".to_string()]);
        assert_eq!(
            read_item.rating_sources,
            vec!["tmdb".to_string(), "imdb".to_string()]
        );
        assert_eq!(read_item.external_ratings.len(), 1);
        assert_eq!(read_item.external_ratings[0].source, "imdb");
        assert_eq!(read_item.external_ratings[0].normalized, 0.82);
        assert_eq!(read_item.external_ratings[0].votes, Some(1234));
        assert_eq!(read_item.external_ids.len(), 2);
        assert_eq!(read_item.external_ids[0].source, "tmdb");
        assert_eq!(read_item.external_ids[0].kind, "movie");
        assert_eq!(read_item.external_ids[0].id, "10");
        assert_eq!(read_item.external_ids[0].key, "tmdb:movie:10");
        assert_eq!(read_item.external_ids[1].source, "imdb");
        assert_eq!(read_item.external_ids[1].id, "tt0000010");
        assert_eq!(read_item.source_tags.len(), 1);
        assert_eq!(
            read_item.source_tags[0].values,
            vec!["theme".to_string(), "Isekai".to_string()]
        );
        assert_eq!(read_item.matched_subject_keys, vec!["tvdb:series:1"]);
        assert_eq!(read_item.change_subject_keys, vec!["tvdb:series:1"]);
        assert_eq!(read_item.library_provenance.len(), 1);
        assert_eq!(
            read_item.library_provenance[0].library_id.as_deref(),
            Some("series-library-a")
        );
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO titles (
                id, library_id, name, name_normalized, facet, root_folder_id, created_at
             )
             VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text("source-title-1".to_string()),
                SqlArg::Text("movie_default_library".to_string()),
                SqlArg::Text("Source Title".to_string()),
                SqlArg::Text("source title".to_string()),
                SqlArg::Text("movie".to_string()),
                SqlArg::Text("canonical_root_for_movie_default_library".to_string()),
                SqlArg::Timestamp(now),
            ],
        )
        .await
        .expect("source title should insert");
        let mut sparse_title_rec_item = (*read_item).clone();
        sparse_title_rec_item.background_url = None;
        sparse_title_rec_item.overview = None;
        sparse_title_rec_item.rating = None;
        sparse_title_rec_item.genres.clear();
        sparse_title_rec_item.rating_sources.clear();
        sparse_title_rec_item.external_ratings.clear();
        sparse_title_rec_item.source_tags.clear();
        sparse_title_rec_item.facet_terms.clear();
        store
            .replace_title_more_like_this_items("source-title-1", "eng", &[sparse_title_rec_item])
            .await
            .expect("title recommendations should replace");
        let more_like_this = store
            .list_title_more_like_this_items("source-title-1", 10)
            .await
            .expect("title recommendations should list");
        assert_eq!(more_like_this.len(), 1);
        assert_eq!(more_like_this[0].target_key, "tmdb:movie:10");
        assert_eq!(
            more_like_this[0].background_url.as_deref(),
            Some("https://images.example.test/movie-bg.jpg")
        );
        assert_eq!(
            more_like_this[0].overview.as_deref(),
            Some("Rich canonical overview")
        );
        assert_eq!(more_like_this[0].genres, vec!["Drama".to_string()]);
        assert_eq!(
            more_like_this[0].source_tags[0].values,
            vec!["theme".to_string(), "Isekai".to_string()]
        );
        assert_eq!(more_like_this[0].external_ratings.len(), 1);
        assert_eq!(more_like_this[0].external_ratings[0].source, "imdb");
        let canonical_rows = SqlRuntime::fetch_all(
            store.datastore.read_exec(),
            "SELECT id
             FROM discovery_titles
             WHERE target_key_norm = {} AND language = {}
             ORDER BY id ASC",
            &[
                SqlArg::Text("tmdb:movie:10".to_string()),
                SqlArg::Text("eng".to_string()),
            ],
        )
        .await
        .expect("canonical title rows should query");
        assert_eq!(
            canonical_rows.len(),
            1,
            "snapshot occurrences and title recommendations should share one canonical title row"
        );
        let occurrence_title_id = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT discovery_title_id
             FROM discovery_items
             WHERE id = {}",
            &[SqlArg::Text("item-row-1".to_string())],
        )
        .await
        .expect("occurrence title id should query")
        .expect("occurrence title id should exist")
        .text("discovery_title_id")
        .expect("occurrence title id should read");
        let link_title_id = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT discovery_title_id
             FROM title_more_like_this_items
             WHERE source_title_id = {}",
            &[SqlArg::Text("source-title-1".to_string())],
        )
        .await
        .expect("link title id should query")
        .expect("link title id should exist")
        .text("discovery_title_id")
        .expect("link title id should read");
        assert_eq!(occurrence_title_id, link_title_id);
        let read_facets = store
            .list_discovery_facets("run-1")
            .await
            .expect("facets should list");
        assert_eq!(read_facets.len(), 1);
        assert_eq!(read_facets[0].facet_value, "Drama");
        let catalog_movie_candidates = store
            .list_catalog_personalized_discovery_items(
                "run-1",
                &["series-library-a".to_string()],
                "movie",
                false,
                10,
            )
            .await
            .expect("catalog personalized candidates should apply provenance and media kind");
        assert_eq!(catalog_movie_candidates.total_count, 1);
        assert_eq!(catalog_movie_candidates.items[0].id, "item-row-1");
        let hidden_catalog_candidates = store
            .list_catalog_personalized_discovery_items(
                "run-1",
                &["series-library-b".to_string()],
                "movie",
                false,
                10,
            )
            .await
            .expect("catalog personalized candidates should apply library scope");
        assert!(hidden_catalog_candidates.items.is_empty());
        let personalized_facets = store
            .list_personalized_discovery_facets("run-1", &["series-library-a".to_string()], false)
            .await
            .expect("personalized facets should list from canonical terms");
        assert_eq!(personalized_facets.len(), 2);
        assert!(personalized_facets.iter().all(|facet| {
            facet.smg_count.is_none()
                && facet.local_count == Some(1)
                && !facet.facet_value.contains(':')
        }));
        assert!(
            personalized_facets
                .iter()
                .any(|facet| facet.facet_name == "genre" && facet.facet_value == "Drama")
        );
        assert!(
            personalized_facets
                .iter()
                .any(|facet| facet.facet_name == "theme" && facet.facet_value == "Isekai")
        );
        assert!(personalized_facets.iter().all(|facet| {
            facet.facet_value != "mal:theme:psychological" && facet.facet_value != "Drama:"
        }));
        let hidden_library_facets = store
            .list_personalized_discovery_facets("run-1", &["series-library-b".to_string()], false)
            .await
            .expect("personalized facets should apply library provenance");
        assert!(hidden_library_facets.is_empty());
        let movie_page = store
            .query_discovery_items(&DiscoveryItemsStorageQuery {
                context_run_id: Some("run-1".to_string()),
                public_run_id: None,
                readable_library_ids: vec!["series-library-a".to_string()],
                filters: DiscoveryItemsQuery {
                    target_kinds: vec!["movie".to_string()],
                    include_unresolved: false,
                    ..DiscoveryItemsQuery::default()
                },
                limit: 10,
                offset: 0,
            })
            .await
            .expect("movie query should use normalized media kind");
        assert_eq!(movie_page.total_count, 1);
        assert_eq!(movie_page.items[0].target_key, "tmdb:movie:10");

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
            title_id: None,
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
            sort_index: 0,
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
            genres: Vec::new(),
            rating: None,
            rating_sources: Vec::new(),
            external_ratings: Vec::new(),
            external_ids: Vec::new(),
            status_tags: Vec::new(),
            source_tags: Vec::new(),
            sources: Vec::new(),
            best_source: None,
            relation_types: Vec::new(),
            relation_subtypes: Vec::new(),
            chart_signals: Vec::new(),
            provider_signals: Vec::new(),
            rank_components: Vec::new(),
            source_count: Some(1),
            edge_count: Some(0),
            relation_count: Some(0),
            source_subject_count: Some(0),
            rank_score: Some(0.1),
            matched_subject_keys: Vec::new(),
            matched_subject_titles: Vec::new(),
            matched_subject_count: 0,
            library_provenance: Vec::new(),
            tmdb_collection_id: None,
            tmdb_collection_name: None,
            owned_in_input: false,
            facet_terms: Vec::new(),
            context_terms: Vec::new(),
            change_subject_keys: Vec::new(),
            removed_subject_keys: Vec::new(),
            tombstoned_by_run_id: None,
            tombstoned_at: None,
            created_at: observed_at,
            updated_at: observed_at,
        }
    }
}
