use async_trait::async_trait;
use chrono::{Duration, Utc};
use scryer_application::{
    AppResult, BlocklistRepository, DownloadSourceKind, HousekeepingRepository,
    LibraryProbeRepository, LibraryProbeSignature, NewBlocklistEntry, PendingRelease,
    PendingReleaseRepository, PendingReleaseStatus, ReleaseDecision, SubtitleDownloadRepository,
    WantedItem, WantedItemRepository, WantedItemsQuery, WantedStatus,
    subtitles::{ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry},
};
use scryer_domain::{
    BlocklistEntry, DomainEventType, ExternalSubtitleSourceKind, Id, MediaFacet,
    SubtitleBlocklistEntry, SubtitleDownload,
};

use crate::SqliteServices;
use crate::queries::common::parse_utc_datetime;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};
use crate::queries::sql_runtime::{repo_err, run_with_sqlite_busy_retries};

const LIBRARY_PROBE_COLUMNS: &str = "title_id, path, probe_signature_scheme, probe_signature_value, last_probed_at, last_changed_at";

const UPSERT_LIBRARY_PROBE_SIGNATURE_SQL: &str = "INSERT INTO library_probe_signatures (
    title_id, path, probe_signature_scheme, probe_signature_value, last_probed_at, last_changed_at,
    created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}
)
ON CONFLICT(title_id) DO UPDATE SET
    path = excluded.path,
    probe_signature_scheme = excluded.probe_signature_scheme,
    probe_signature_value = excluded.probe_signature_value,
    last_probed_at = excluded.last_probed_at,
    last_changed_at = excluded.last_changed_at,
    updated_at = excluded.updated_at";

fn library_probe_signature_from_row(row: &SqlRow) -> AppResult<LibraryProbeSignature> {
    Ok(LibraryProbeSignature {
        title_id: row.text("title_id")?,
        path: row.text("path")?,
        probe_signature_scheme: row.opt_text("probe_signature_scheme")?,
        probe_signature_value: row.opt_text("probe_signature_value")?,
        last_probed_at: row.opt_timestamp("last_probed_at")?,
        last_changed_at: row.opt_timestamp("last_changed_at")?,
    })
}

#[derive(Clone)]
pub struct LibraryProbeStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct WantedStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct PendingReleaseStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct BlocklistStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct SubtitleDownloadStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct HousekeepingStore {
    datastore: StoreDatastore,
}

macro_rules! impl_store_new {
    ($store:ident) => {
        impl $store {
            pub(crate) fn new(datastore: StoreDatastore) -> Self {
                Self { datastore }
            }

            pub fn from_sqlite_services(db: &SqliteServices) -> Self {
                Self::new(StoreDatastore::Sqlite {
                    pool: db.pool().clone(),
                    writer_gate: db.writer_gate(),
                })
            }

            pub fn from_postgres_services(db: &crate::postgres::PostgresServices) -> Self {
                Self::new(StoreDatastore::Postgres {
                    pool: db.pool().clone(),
                })
            }
        }
    };
}

impl_store_new!(LibraryProbeStore);
impl_store_new!(WantedStore);
impl_store_new!(PendingReleaseStore);
impl_store_new!(BlocklistStore);
impl_store_new!(SubtitleDownloadStore);
impl_store_new!(HousekeepingStore);

#[async_trait]
impl LibraryProbeRepository for LibraryProbeStore {
    async fn get_probe_signature(
        &self,
        title_id: &str,
    ) -> AppResult<Option<LibraryProbeSignature>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {LIBRARY_PROBE_COLUMNS} FROM library_probe_signatures WHERE title_id = {{}}"
            ),
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?;

        row.as_ref()
            .map(library_probe_signature_from_row)
            .transpose()
    }

    async fn upsert_probe_signature(&self, probe: &LibraryProbeSignature) -> AppResult<()> {
        let now = Utc::now();
        let args = vec![
            SqlArg::Text(probe.title_id.clone()),
            SqlArg::Text(probe.path.clone()),
            SqlArg::OptText(probe.probe_signature_scheme.clone()),
            SqlArg::OptText(probe.probe_signature_value.clone()),
            SqlArg::OptTimestamp(probe.last_probed_at),
            SqlArg::OptTimestamp(probe.last_changed_at),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ];

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "upsert_library_probe_signature",
            move |tx| {
                let args = args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(SqlExec::Tx(tx), UPSERT_LIBRARY_PROBE_SIGNATURE_SQL, &args)
                        .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn delete_probe_signatures_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if title_ids.is_empty() {
            return Ok(0);
        }

        let sql = format!(
            "DELETE FROM library_probe_signatures WHERE title_id IN ({})",
            vec!["{}"; title_ids.len()].join(", ")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_library_probe_signatures_for_title_ids",
            move |tx| {
                let sql = sql.clone();
                let args = args.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
                    Ok(rows as u32)
                })
            },
        )
        .await
    }
}

fn timestamp_arg_for_datastore(datastore: &StoreDatastore, value: &str) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::Text(value.to_string())),
        StoreDatastore::Postgres { .. } => parse_utc_datetime(value).map(SqlArg::Timestamp),
    }
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

fn opt_json_arg_for_datastore(
    datastore: &StoreDatastore,
    value: Option<&str>,
) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::OptText(value.map(str::to_string))),
        StoreDatastore::Postgres { .. } => value
            .map(serde_json::from_str)
            .transpose()
            .map(SqlArg::OptJson)
            .map_err(repo_err),
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

fn required_timestamp_text(row: &SqlRow, column: &str) -> AppResult<String> {
    match row {
        SqlRow::Sqlite(_) => row.text(column),
        SqlRow::Postgres(_) => row.timestamp(column).map(|value| value.to_rfc3339()),
    }
}

fn wanted_seed_row_to_item(row: &SqlRow) -> AppResult<WantedItem> {
    let status = row.text("status")?;
    Ok(WantedItem {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        title_name: None,
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: row.opt_text("episode_id")?,
        collection_id: row.opt_text("collection_id")?,
        season_number: None,
        episode_number: None,
        media_type: row.text("media_type")?,
        search_phase: row.text("search_phase")?,
        next_search_at: opt_timestamp_text(row, "next_search_at")?,
        last_search_at: opt_timestamp_text(row, "last_search_at")?,
        search_count: row.i64("search_count")?,
        baseline_date: opt_timestamp_text(row, "baseline_date")?,
        status: WantedStatus::parse(&status).unwrap_or_default(),
        grabbed_release: row.opt_text("grabbed_release")?,
        current_score: row.opt_i32("current_score")?,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: required_timestamp_text(row, "created_at")?,
        updated_at: required_timestamp_text(row, "updated_at")?,
    })
}

fn release_decision_row_to_item(row: &SqlRow) -> AppResult<ReleaseDecision> {
    Ok(ReleaseDecision {
        id: row.text("id")?,
        wanted_item_id: row.text("wanted_item_id")?,
        title_id: row.text("title_id")?,
        release_title: row.text("release_title")?,
        release_url: row.opt_text("release_url")?,
        release_size_bytes: row.opt_i64("release_size_bytes")?,
        decision_code: row.text("decision_code")?,
        candidate_score: row.i32("candidate_score")?,
        current_score: row.opt_i32("current_score")?,
        score_delta: row.opt_i32("score_delta")?,
        explanation_json: match row {
            SqlRow::Sqlite(_) => row.opt_text("explanation_json")?,
            SqlRow::Postgres(_) => row
                .opt_json("explanation_json")?
                .map(|value| value.to_string()),
        },
        created_at: required_timestamp_text(row, "created_at")?,
    })
}

fn json_text_from_row(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(_) => row.opt_text(column),
        SqlRow::Postgres(_) => row
            .opt_json(column)
            .map(|value| value.map(|json| json.to_string())),
    }
}

fn wanted_row_to_item(row: &SqlRow) -> AppResult<WantedItem> {
    let latest_release_decision = match row.opt_text("latest_decision_id")? {
        Some(id) => Some(ReleaseDecision {
            id,
            wanted_item_id: row.text("latest_decision_wanted_item_id")?,
            title_id: row.text("latest_decision_title_id")?,
            release_title: row.text("latest_decision_release_title")?,
            release_url: row.opt_text("latest_decision_release_url")?,
            release_size_bytes: row.opt_i64("latest_decision_release_size_bytes")?,
            decision_code: row.text("latest_decision_decision_code")?,
            candidate_score: row
                .opt_i32("latest_decision_candidate_score")?
                .unwrap_or_default(),
            current_score: row.opt_i32("latest_decision_current_score")?,
            score_delta: row.opt_i32("latest_decision_score_delta")?,
            explanation_json: json_text_from_row(row, "latest_decision_explanation_json")?,
            created_at: required_timestamp_text(row, "latest_decision_created_at")?,
        }),
        None => None,
    };

    let status = row.text("status")?;
    Ok(WantedItem {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        title_name: row.opt_text("title_name")?,
        title_slug: row.opt_text("title_slug")?,
        title_facet: row.opt_text("title_facet")?,
        library_id: row.opt_text("library_id")?,
        library_name: row.opt_text("library_name")?,
        library_slug: row.opt_text("library_slug")?,
        episode_id: row.opt_text("episode_id")?,
        collection_id: row.opt_text("collection_id")?,
        season_number: row.opt_text("season_number")?,
        episode_number: row.opt_text("episode_number")?,
        media_type: row.text("media_type")?,
        search_phase: row.text("search_phase")?,
        next_search_at: opt_timestamp_text(row, "next_search_at")?,
        last_search_at: opt_timestamp_text(row, "last_search_at")?,
        search_count: row.i64("search_count")?,
        baseline_date: opt_timestamp_text(row, "baseline_date")?,
        status: WantedStatus::parse(&status).unwrap_or_default(),
        grabbed_release: row.opt_text("grabbed_release")?,
        current_score: row.opt_i32("current_score")?,
        latest_release_decision,
        mismatch_recovery_eligible: row.bool("mismatch_recovery_eligible")?,
        created_at: required_timestamp_text(row, "created_at")?,
        updated_at: required_timestamp_text(row, "updated_at")?,
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
                     SELECT 1
                       FROM release_decisions mismatch_any
                      WHERE mismatch_any.wanted_item_id = w.id
                 )
                 AND NOT EXISTS (
                     SELECT 1
                       FROM release_decisions mismatch_other
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
       LEFT JOIN release_decisions latest_decision ON latest_decision.id = (
           SELECT rd.id
             FROM release_decisions rd
            WHERE rd.wanted_item_id = w.id
            ORDER BY rd.created_at DESC
            LIMIT 1
       )"
}

fn append_in_filter(sql: &mut String, args: &mut Vec<SqlArg>, column: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }

    sql.push_str(" AND ");
    sql.push_str(column);
    sql.push_str(" IN (");
    sql.push_str(&placeholders(values.len()));
    sql.push(')');
    args.extend(values.iter().cloned().map(SqlArg::Text));
}

fn append_wanted_query_filters(
    sql: &mut String,
    args: &mut Vec<SqlArg>,
    query: &WantedItemsQuery,
    include_title_search: bool,
) {
    append_in_filter(sql, args, "w.status", &query.statuses);
    append_in_filter(sql, args, "w.media_type", &query.media_types);
    if let Some(title_id) = query.title_id.as_deref() {
        sql.push_str(" AND w.title_id = {}");
        args.push(SqlArg::Text(title_id.to_string()));
    }
    append_in_filter(sql, args, "t.library_id", &query.library_ids);
    if include_title_search
        && let Some(normalized) = query
            .title_search
            .as_deref()
            .map(crate::queries::title_search::normalize_title_search_text)
            .filter(|value| !value.is_empty())
    {
        sql.push_str(
            " AND EXISTS (
                SELECT 1
                  FROM title_search_terms wanted_title_search
                 WHERE wanted_title_search.title_id = w.title_id
                   AND wanted_title_search.term_kind NOT LIKE '%_token'
                   AND (
                        wanted_title_search.normalized_term = {}
                        OR wanted_title_search.normalized_term LIKE {}
                        OR wanted_title_search.normalized_term LIKE {}
                   )
            )",
        );
        args.push(SqlArg::Text(normalized.clone()));
        args.push(SqlArg::Text(format!("{normalized}%")));
        args.push(SqlArg::Text(format!("%{normalized}%")));
    }
    append_in_filter(
        sql,
        args,
        "latest_decision.decision_code",
        &query.latest_decision_codes,
    );
}

fn sqlite_title_search_requires_spellfix(query: &WantedItemsQuery) -> bool {
    query
        .title_search
        .as_deref()
        .map(crate::queries::title_search::normalize_title_search_text)
        .is_some_and(|value| !value.is_empty())
}

fn wanted_upsert_sql(datastore: &StoreDatastore, item: &WantedItem) -> String {
    let conflict_target = if item.collection_id.is_some() {
        "(collection_id) WHERE collection_id IS NOT NULL"
    } else if item.episode_id.is_some() {
        match datastore {
            StoreDatastore::Sqlite { .. } => "(title_id, episode_id)",
            StoreDatastore::Postgres { .. } => {
                "(title_id, episode_id) WHERE episode_id IS NOT NULL"
            }
        }
    } else {
        "(title_id) WHERE episode_id IS NULL AND collection_id IS NULL"
    };

    format!(
        "INSERT INTO wanted_items
         (id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
          last_search_at, search_count, baseline_date, status, grabbed_release, current_score,
          created_at, updated_at)
         VALUES ({{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}})
         ON CONFLICT{conflict_target} DO UPDATE SET
            search_phase = excluded.search_phase,
            next_search_at = CASE
                WHEN excluded.next_search_at IS NULL THEN NULL
                WHEN wanted_items.search_count > 0 AND wanted_items.next_search_at IS NOT NULL
                THEN wanted_items.next_search_at
                WHEN wanted_items.status IN ('paused', 'completed')
                THEN wanted_items.next_search_at
                ELSE excluded.next_search_at
            END,
            baseline_date = excluded.baseline_date,
            status = CASE
                WHEN wanted_items.status IN ('completed', 'paused') AND excluded.status = 'wanted'
                THEN wanted_items.status
                ELSE excluded.status
            END,
            updated_at = excluded.updated_at"
    )
}

fn wanted_upsert_args(datastore: &StoreDatastore, item: &WantedItem) -> AppResult<Vec<SqlArg>> {
    let now = Utc::now().to_rfc3339();
    Ok(vec![
        SqlArg::Text(item.id.clone()),
        SqlArg::Text(item.title_id.clone()),
        SqlArg::OptText(item.episode_id.clone()),
        SqlArg::OptText(item.collection_id.clone()),
        SqlArg::Text(item.media_type.clone()),
        SqlArg::Text(item.search_phase.clone()),
        opt_timestamp_arg_for_datastore(datastore, item.next_search_at.as_deref())?,
        opt_timestamp_arg_for_datastore(datastore, item.last_search_at.as_deref())?,
        SqlArg::I64(item.search_count),
        opt_timestamp_arg_for_datastore(datastore, item.baseline_date.as_deref())?,
        SqlArg::Text(item.status.as_str().to_string()),
        SqlArg::OptText(item.grabbed_release.clone()),
        SqlArg::OptI32(item.current_score),
        timestamp_arg_for_datastore(datastore, &now)?,
        timestamp_arg_for_datastore(datastore, &now)?,
    ])
}

async fn execute_wanted_upsert_tx(
    tx: &mut crate::queries::sql_runtime::SqlTx<'_>,
    datastore: &StoreDatastore,
    item: &WantedItem,
) -> AppResult<String> {
    let sql = wanted_upsert_sql(datastore, item);
    let args = wanted_upsert_args(datastore, item)?;
    SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
    Ok(item.id.clone())
}

async fn execute_datastore_write(
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

async fn fetch_seed_target_tx(
    tx: &mut crate::queries::sql_runtime::SqlTx<'_>,
    item: &WantedItem,
) -> AppResult<Option<WantedItem>> {
    let columns = "SELECT id, title_id, episode_id, collection_id, media_type, search_phase,
                          next_search_at, last_search_at, search_count, baseline_date, status,
                          grabbed_release, current_score, created_at, updated_at
                     FROM wanted_items";
    let (sql, args) = if let Some(collection_id) = item.collection_id.as_deref() {
        (
            format!("{columns} WHERE title_id = {{}} AND collection_id = {{}}"),
            vec![
                SqlArg::Text(item.title_id.clone()),
                SqlArg::Text(collection_id.to_string()),
            ],
        )
    } else if let Some(episode_id) = item.episode_id.as_deref() {
        (
            format!("{columns} WHERE title_id = {{}} AND episode_id = {{}}"),
            vec![
                SqlArg::Text(item.title_id.clone()),
                SqlArg::Text(episode_id.to_string()),
            ],
        )
    } else {
        (
            format!(
                "{columns} WHERE title_id = {{}} AND episode_id IS NULL AND collection_id IS NULL"
            ),
            vec![SqlArg::Text(item.title_id.clone())],
        )
    };

    SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &args)
        .await?
        .as_ref()
        .map(wanted_seed_row_to_item)
        .transpose()
}

fn merge_seeded_wanted_item(item: &WantedItem, existing: Option<&WantedItem>) -> WantedItem {
    let mut seeded = item.clone();
    if let Some(existing) = existing {
        seeded.id = existing.id.clone();
        if existing.search_count > 0 {
            seeded.next_search_at = existing.next_search_at.clone();
        }
        if item.status == WantedStatus::Wanted && existing.status != WantedStatus::Wanted {
            seeded.status = existing.status;
        }
    }
    seeded
}

#[async_trait]
impl WantedItemRepository for WantedStore {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        let item = item.clone();
        let datastore = self.datastore.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "upsert_wanted_item", move |tx| {
            let datastore = datastore.clone();
            let item = item.clone();
            Box::pin(async move { execute_wanted_upsert_tx(tx, &datastore, &item).await })
        })
        .await
    }

    async fn ensure_wanted_item_seeded(&self, item: &WantedItem) -> AppResult<String> {
        let item = item.clone();
        let datastore = self.datastore.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "ensure_wanted_item_seeded", move |tx| {
            let datastore = datastore.clone();
            let item = item.clone();
            Box::pin(async move {
                let existing = fetch_seed_target_tx(tx, &item).await?;
                let seeded = merge_seeded_wanted_item(&item, existing.as_ref());
                execute_wanted_upsert_tx(tx, &datastore, &seeded).await?;
                Ok(existing.map_or(item.id.clone(), |existing| existing.id))
            })
        })
        .await
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<WantedItem>> {
        let mut sql = String::from(
            "SELECT w.id, w.title_id,
                    CAST(NULL AS TEXT) AS title_name,
                    CAST(NULL AS TEXT) AS title_slug,
                    t.facet AS title_facet,
                    t.library_id AS library_id,
                    CAST(NULL AS TEXT) AS library_name,
                    CAST(NULL AS TEXT) AS library_slug,
                    w.episode_id, w.collection_id,
                    e.season_number, e.episode_number, w.media_type, w.search_phase,
                    w.next_search_at, w.last_search_at, w.search_count, w.baseline_date,
                    w.status, w.grabbed_release, w.current_score,
                    CAST(NULL AS TEXT) AS latest_decision_id,
                    FALSE AS mismatch_recovery_eligible,
                    w.created_at, w.updated_at
               FROM wanted_items w
               JOIN titles t ON t.id = w.title_id
               LEFT JOIN episodes e ON e.id = w.episode_id
              WHERE w.status = 'wanted'
                AND w.next_search_at IS NOT NULL
                AND w.next_search_at <= {}
                AND (w.media_type != 'episode' OR w.baseline_date IS NOT NULL)",
        );
        let mut args = vec![timestamp_arg_for_datastore(&self.datastore, now)?];
        if !excluded_facets.is_empty() {
            sql.push_str(" AND t.facet NOT IN (");
            sql.push_str(&placeholders(excluded_facets.len()));
            sql.push(')');
            args.extend(
                excluded_facets
                    .iter()
                    .map(|facet| SqlArg::Text(facet.as_str().to_string())),
            );
        }
        sql.push_str(" ORDER BY w.next_search_at ASC LIMIT {}");
        args.push(SqlArg::I64(batch_limit));

        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(wanted_row_to_item)
            .collect()
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
        let now = Utc::now().to_rfc3339();
        execute_datastore_write(
            &self.datastore,
            "update_wanted_item_status",
            "UPDATE wanted_items
                SET status = {},
                    next_search_at = {},
                    last_search_at = {},
                    search_count = {},
                    current_score = {},
                    grabbed_release = {},
                    updated_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(status.to_string()),
                opt_timestamp_arg_for_datastore(&self.datastore, next_search_at)?,
                opt_timestamp_arg_for_datastore(&self.datastore, last_search_at)?,
                SqlArg::I64(search_count),
                SqlArg::OptI32(current_score),
                SqlArg::OptText(grabbed_release.map(str::to_string)),
                timestamp_arg_for_datastore(&self.datastore, &now)?,
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        let (sql, args) = if let Some(episode_id) = episode_id {
            (
                format!(
                    "{} WHERE w.title_id = {{}} AND w.episode_id = {{}}",
                    wanted_item_select_sql()
                ),
                vec![
                    SqlArg::Text(title_id.to_string()),
                    SqlArg::Text(episode_id.to_string()),
                ],
            )
        } else {
            (
                format!(
                    "{} WHERE w.title_id = {{}} AND w.episode_id IS NULL",
                    wanted_item_select_sql()
                ),
                vec![SqlArg::Text(title_id.to_string())],
            )
        };
        SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args)
            .await?
            .as_ref()
            .map(wanted_row_to_item)
            .transpose()
    }

    async fn complete_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        current_score: Option<i32>,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let (sql, args) = if let Some(episode_id) = episode_id {
            (
                "UPDATE wanted_items
                    SET status = {},
                        next_search_at = {},
                        last_search_at = {},
                        current_score = COALESCE({}, current_score),
                        updated_at = {}
                  WHERE title_id = {} AND episode_id = {}"
                    .to_string(),
                vec![
                    SqlArg::Text(WantedStatus::Completed.as_str().to_string()),
                    opt_timestamp_arg_for_datastore(&self.datastore, None)?,
                    opt_timestamp_arg_for_datastore(&self.datastore, last_search_at)?,
                    SqlArg::OptI32(current_score),
                    timestamp_arg_for_datastore(&self.datastore, &now)?,
                    SqlArg::Text(title_id.to_string()),
                    SqlArg::Text(episode_id.to_string()),
                ],
            )
        } else {
            (
                "UPDATE wanted_items
                    SET status = {},
                        next_search_at = {},
                        last_search_at = {},
                        current_score = COALESCE({}, current_score),
                        updated_at = {}
                  WHERE title_id = {} AND episode_id IS NULL"
                    .to_string(),
                vec![
                    SqlArg::Text(WantedStatus::Completed.as_str().to_string()),
                    opt_timestamp_arg_for_datastore(&self.datastore, None)?,
                    opt_timestamp_arg_for_datastore(&self.datastore, last_search_at)?,
                    SqlArg::OptI32(current_score),
                    timestamp_arg_for_datastore(&self.datastore, &now)?,
                    SqlArg::Text(title_id.to_string()),
                ],
            )
        };
        let rows =
            execute_datastore_write(&self.datastore, "complete_wanted_item_for_title", sql, args)
                .await?;
        Ok(rows > 0)
    }

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_wanted_items_for_title",
            "DELETE FROM wanted_items WHERE title_id = {}",
            vec![SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_wanted_items_for_collection",
            "DELETE FROM wanted_items
              WHERE collection_id = {}
                 OR episode_id IN (
                    SELECT id
                      FROM episodes
                     WHERE collection_id = {}
                 )",
            vec![
                SqlArg::Text(collection_id.to_string()),
                SqlArg::Text(collection_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_wanted_items_for_episode",
            "DELETE FROM wanted_items WHERE episode_id = {}",
            vec![SqlArg::Text(episode_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        execute_datastore_write(
            &self.datastore,
            "reset_fruitless_wanted_items",
            "UPDATE wanted_items
                SET next_search_at = {}, updated_at = {}
              WHERE status = 'wanted'
                AND search_count > 0
                AND current_score IS NULL
                AND (media_type != 'episode' OR baseline_date IS NOT NULL)",
            vec![
                timestamp_arg_for_datastore(&self.datastore, now)?,
                timestamp_arg_for_datastore(&self.datastore, now)?,
            ],
        )
        .await
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        execute_datastore_write(
            &self.datastore,
            "insert_release_decision",
            "INSERT INTO release_decisions
             (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
              decision_code, candidate_score, current_score, score_delta, explanation_json, created_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(decision.id.clone()),
                SqlArg::Text(decision.wanted_item_id.clone()),
                SqlArg::Text(decision.title_id.clone()),
                SqlArg::Text(decision.release_title.clone()),
                SqlArg::OptText(decision.release_url.clone()),
                SqlArg::OptI64(decision.release_size_bytes),
                SqlArg::Text(decision.decision_code.clone()),
                SqlArg::I32(decision.candidate_score),
                SqlArg::OptI32(decision.current_score),
                SqlArg::OptI32(decision.score_delta),
                opt_json_arg_for_datastore(&self.datastore, decision.explanation_json.as_deref())?,
                timestamp_arg_for_datastore(&self.datastore, &decision.created_at)?,
            ],
        )
        .await?;
        Ok(decision.id.clone())
    }

    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        let sql = format!("{} WHERE w.id = {{}}", wanted_item_select_sql());
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(wanted_row_to_item)
        .transpose()
    }

    async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>> {
        if let StoreDatastore::Sqlite { pool, .. } = &self.datastore
            && sqlite_title_search_requires_spellfix(&query)
        {
            return crate::queries::wanted::list_wanted_items_query(pool, &query).await;
        }

        let mut sql = wanted_item_select_sql().to_string();
        sql.push_str(" WHERE 1=1");
        let mut args = Vec::new();
        append_wanted_query_filters(&mut sql, &mut args, &query, true);
        sql.push_str(" ORDER BY w.updated_at DESC LIMIT {} OFFSET {}");
        args.push(SqlArg::I64(query.limit));
        args.push(SqlArg::I64(query.offset));

        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(wanted_row_to_item)
            .collect()
    }

    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        if let StoreDatastore::Sqlite { pool, .. } = &self.datastore
            && sqlite_title_search_requires_spellfix(&query)
        {
            return crate::queries::wanted::count_wanted_items_query(pool, &query).await;
        }

        let mut sql = String::from(
            "SELECT COUNT(*) AS cnt
               FROM wanted_items w
               LEFT JOIN titles t ON t.id = w.title_id
               LEFT JOIN release_decisions latest_decision ON latest_decision.id = (
                   SELECT rd.id
                     FROM release_decisions rd
                    WHERE rd.wanted_item_id = w.id
                    ORDER BY rd.created_at DESC
                    LIMIT 1
               )
              WHERE 1=1",
        );
        let mut args = Vec::new();
        append_wanted_query_filters(&mut sql, &mut args, &query, true);
        SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args)
            .await?
            .map(|row| row.i64("cnt"))
            .transpose()
            .map(|value| value.unwrap_or_default())
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                    decision_code, candidate_score, current_score, score_delta, explanation_json, created_at
               FROM release_decisions
              WHERE title_id = {}
              ORDER BY created_at DESC
              LIMIT {}",
            &[SqlArg::Text(title_id.to_string()), SqlArg::I64(limit)],
        )
        .await?;
        rows.iter().map(release_decision_row_to_item).collect()
    }

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                    decision_code, candidate_score, current_score, score_delta, explanation_json, created_at
               FROM release_decisions
              WHERE wanted_item_id = {}
              ORDER BY created_at DESC
              LIMIT {}",
            &[SqlArg::Text(wanted_item_id.to_string()), SqlArg::I64(limit)],
        )
        .await?;
        rows.iter().map(release_decision_row_to_item).collect()
    }
}

fn housekeeping_cutoff_arg(days: i64) -> SqlArg {
    SqlArg::Timestamp(Utc::now() - Duration::days(days))
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

async fn execute_housekeeping_delete(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: impl Into<String>,
    args: Vec<SqlArg>,
) -> AppResult<u32> {
    let sql = sql.into();
    let rows_affected = SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let sql = sql.clone();
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
    })
    .await?;
    Ok(rows_affected as u32)
}

async fn delete_for_title_ids_shared(
    datastore: &StoreDatastore,
    op_name: &'static str,
    table: &'static str,
    title_ids: &[String],
) -> AppResult<u32> {
    if title_ids.is_empty() {
        return Ok(0);
    }

    let sql = format!(
        "DELETE FROM {table} WHERE title_id IN ({})",
        placeholders(title_ids.len())
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    execute_housekeeping_delete(datastore, op_name, sql, args).await
}

async fn delete_media_files_by_ids_shared(
    datastore: &StoreDatastore,
    ids: &[String],
) -> AppResult<u32> {
    if ids.is_empty() {
        return Ok(0);
    }

    let sql = format!(
        "DELETE FROM media_files WHERE id IN ({})",
        placeholders(ids.len())
    );
    let args = ids.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
    execute_housekeeping_delete(datastore, "delete_media_files_by_ids", sql, args).await
}

#[async_trait]
impl HousekeepingRepository for HousekeepingStore {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_release_decisions_older_than",
            "DELETE FROM release_decisions WHERE created_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_title_history_older_than(&self, _days: i64) -> AppResult<u32> {
        // Legacy title_history rows are retired by migration 0085; nothing remains to prune.
        Ok(0)
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_release_attempts_older_than",
            "DELETE FROM release_download_attempts
              WHERE attempted_at < {}
                AND outcome != 'pending'",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_dispatched_event_outboxes_older_than",
            "DELETE FROM event_outboxes
              WHERE status = 'dispatched'
                AND created_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_history_events_older_than",
            "DELETE FROM history_events WHERE occurred_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        if event_types.is_empty() {
            return Ok(0);
        }

        let sql = format!(
            "DELETE FROM domain_events
              WHERE occurred_at < {{}}
                AND event_type IN ({})",
            placeholders(event_types.len())
        );
        let mut args = vec![housekeeping_cutoff_arg(days)];
        args.extend(
            event_types
                .iter()
                .map(|event_type| SqlArg::Text(event_type.as_str().to_string())),
        );
        execute_housekeeping_delete(
            &self.datastore,
            "delete_domain_events_older_than_for_types",
            sql,
            args,
        )
        .await
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_download_import_artifacts_older_than",
            "DELETE FROM download_import_artifacts
              WHERE created_at < {}
                AND (
                    import_id IS NULL
                    OR NOT EXISTS (
                        SELECT 1
                          FROM imports
                         WHERE imports.id = download_import_artifacts.import_id
                    )
                    OR EXISTS (
                        SELECT 1
                          FROM imports
                         WHERE imports.id = download_import_artifacts.import_id
                           AND imports.status IN ('completed', 'failed', 'skipped')
                    )
                )",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_terminal_imports_older_than",
            "DELETE FROM imports
              WHERE status IN ('completed', 'failed', 'skipped')
                AND updated_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_terminal_download_queue_commands_older_than",
            "DELETE FROM download_queue_commands
              WHERE action = 'delete'
                AND status IN ('completed', 'failed')
                AND updated_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_rule_set_history_older_than",
            "DELETE FROM rule_set_history WHERE created_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        delete_for_title_ids_shared(
            &self.datastore,
            "delete_history_events_for_title_ids",
            "history_events",
            title_ids,
        )
        .await
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        delete_for_title_ids_shared(
            &self.datastore,
            "delete_download_import_artifacts_for_title_ids",
            "download_import_artifacts",
            title_ids,
        )
        .await
    }

    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        delete_for_title_ids_shared(
            &self.datastore,
            "delete_release_attempts_for_title_ids",
            "release_download_attempts",
            title_ids,
        )
        .await
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, file_path FROM media_files",
            &[],
        )
        .await?;
        rows.iter()
            .map(|row| Ok((row.text("id")?, row.text("file_path")?)))
            .collect()
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        delete_media_files_by_ids_shared(&self.datastore, ids).await
    }

    async fn run_database_maintenance(&self) -> AppResult<()> {
        match &self.datastore {
            StoreDatastore::Sqlite { pool, writer_gate } => {
                let _writer = writer_gate.lock().await;
                run_with_sqlite_busy_retries("sqlite_database_maintenance", || async {
                    sqlx::query("PRAGMA optimize")
                        .execute(pool)
                        .await
                        .map_err(repo_err)?;
                    sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
                        .execute(pool)
                        .await
                        .map_err(repo_err)?;
                    Ok(())
                })
                .await
            }
            StoreDatastore::Postgres { pool } => sqlx::query("VACUUM (ANALYZE)")
                .execute(pool)
                .await
                .map(|_| ())
                .map_err(repo_err),
        }
    }
}

const PENDING_RELEASE_COLUMNS: &str =
    "id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
    source_kind, release_score, scoring_log_json, indexer_source, release_guid,
    added_at, delay_until, status, grabbed_at, source_password, published_at, info_hash";

fn pending_release_row_to_item(row: &SqlRow) -> AppResult<PendingRelease> {
    let status = row.text("status")?;
    Ok(PendingRelease {
        id: row.text("id")?,
        wanted_item_id: row.text("wanted_item_id")?,
        title_id: row.text("title_id")?,
        release_title: row.text("release_title")?,
        release_url: row.opt_text("release_url")?,
        release_size_bytes: row.opt_i64("release_size_bytes")?,
        source_kind: row
            .opt_text("source_kind")?
            .and_then(|value| DownloadSourceKind::parse(&value)),
        release_score: row.i32("release_score")?,
        scoring_log_json: json_text_from_row(row, "scoring_log_json")?,
        indexer_source: row.opt_text("indexer_source")?,
        release_guid: row.opt_text("release_guid")?,
        added_at: required_timestamp_text(row, "added_at")?,
        delay_until: required_timestamp_text(row, "delay_until")?,
        status: PendingReleaseStatus::parse(&status).ok_or_else(|| {
            scryer_application::AppError::Repository("invalid pending release status".into())
        })?,
        grabbed_at: opt_timestamp_text(row, "grabbed_at")?,
        source_password: row.opt_text("source_password")?,
        published_at: opt_timestamp_text(row, "published_at")?,
        info_hash: row.opt_text("info_hash")?,
    })
}

async fn fetch_pending_releases(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<PendingRelease>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(pending_release_row_to_item)
        .collect()
}

fn pending_release_insert_args(
    datastore: &StoreDatastore,
    release: &PendingRelease,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(release.id.clone()),
        SqlArg::Text(release.wanted_item_id.clone()),
        SqlArg::Text(release.title_id.clone()),
        SqlArg::Text(release.release_title.clone()),
        SqlArg::OptText(release.release_url.clone()),
        SqlArg::OptI64(release.release_size_bytes),
        SqlArg::OptText(release.source_kind.map(|value| value.as_str().to_string())),
        SqlArg::I32(release.release_score),
        opt_json_arg_for_datastore(datastore, release.scoring_log_json.as_deref())?,
        SqlArg::OptText(release.indexer_source.clone()),
        SqlArg::OptText(release.release_guid.clone()),
        timestamp_arg_for_datastore(datastore, &release.added_at)?,
        timestamp_arg_for_datastore(datastore, &release.delay_until)?,
        SqlArg::Text(release.status.as_str().to_string()),
        opt_timestamp_arg_for_datastore(datastore, release.grabbed_at.as_deref())?,
        SqlArg::OptText(release.source_password.clone()),
        opt_timestamp_arg_for_datastore(datastore, release.published_at.as_deref())?,
        SqlArg::OptText(release.info_hash.clone()),
    ])
}

#[async_trait]
impl PendingReleaseRepository for PendingReleaseStore {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        execute_datastore_write(
            &self.datastore,
            "insert_pending_release",
            "INSERT INTO pending_releases
             (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
              source_kind, release_score, scoring_log_json, indexer_source, release_guid,
              added_at, delay_until, status, grabbed_at, source_password, published_at, info_hash)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            pending_release_insert_args(&self.datastore, release)?,
        )
        .await?;
        Ok(release.id.clone())
    }

    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE status = 'waiting' AND delay_until <= {{}}
              ORDER BY delay_until ASC"
        );
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[timestamp_arg_for_datastore(&self.datastore, now)?],
        )
        .await
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE status = 'waiting'
              ORDER BY delay_until ASC"
        );
        fetch_pending_releases(self.datastore.read_exec(), &sql, &[]).await
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        let sql = format!("SELECT {PENDING_RELEASE_COLUMNS} FROM pending_releases WHERE id = {{}}");
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(pending_release_row_to_item)
        .transpose()
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE wanted_item_id = {{}} AND status = 'waiting'
              ORDER BY release_score DESC"
        );
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(wanted_item_id.to_string())],
        )
        .await
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE title_id = {{}}
              ORDER BY added_at DESC"
        );
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "update_pending_release_status",
            "UPDATE pending_releases
                SET status = {}, grabbed_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(status.as_str().to_string()),
                opt_timestamp_arg_for_datastore(&self.datastore, grabbed_at)?,
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE wanted_item_id = {{}} AND status = 'standby'
              ORDER BY release_score DESC, added_at ASC"
        );
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(wanted_item_id.to_string())],
        )
        .await
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_standby_pending_releases_for_wanted_item",
            "DELETE FROM pending_releases
              WHERE wanted_item_id = {} AND status = 'standby'",
            vec![SqlArg::Text(wanted_item_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE status = 'standby'
              ORDER BY wanted_item_id ASC, release_score DESC, added_at ASC"
        );
        fetch_pending_releases(self.datastore.read_exec(), &sql, &[]).await
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        let rows = execute_datastore_write(
            &self.datastore,
            "compare_and_set_pending_release_status",
            "UPDATE pending_releases
                SET status = {}, grabbed_at = {}
              WHERE id = {} AND status = {}",
            vec![
                SqlArg::Text(next_status.as_str().to_string()),
                opt_timestamp_arg_for_datastore(&self.datastore, grabbed_at)?,
                SqlArg::Text(id.to_string()),
                SqlArg::Text(current_status.as_str().to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "supersede_pending_releases_for_wanted_item",
            "UPDATE pending_releases
                SET status = 'superseded'
              WHERE wanted_item_id = {} AND id != {} AND status = 'waiting'",
            vec![
                SqlArg::Text(wanted_item_id.to_string()),
                SqlArg::Text(except_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_pending_releases_for_title",
            "DELETE FROM pending_releases WHERE title_id = {}",
            vec![SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }
}

const BLOCKLIST_COLUMNS: &str =
    "id, title_id, source_title, source_hint, quality, download_id, reason, data_json, created_at";

fn blocklist_row_to_entry_sql(row: &SqlRow) -> AppResult<BlocklistEntry> {
    Ok(BlocklistEntry {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        source_title: row.opt_text("source_title")?,
        source_hint: row.opt_text("source_hint")?,
        quality: row.opt_text("quality")?,
        download_id: row.opt_text("download_id")?,
        reason: row.opt_text("reason")?,
        data_json: json_text_from_row(row, "data_json")?,
        created_at: required_timestamp_text(row, "created_at")?,
    })
}

async fn fetch_blocklist_entries(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<BlocklistEntry>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(blocklist_row_to_entry_sql)
        .collect()
}

async fn fetch_exists(exec: SqlExec<'_, '_>, sql: &str, args: &[SqlArg]) -> AppResult<bool> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row.bool("matched"))
        .transpose()
        .map(|value| value.unwrap_or(false))
}

#[async_trait]
impl BlocklistRepository for BlocklistStore {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        let id = Id::new().0;
        let now = Utc::now().to_rfc3339();
        let data_json = serde_json::to_string(&entry.data).map_err(repo_err)?;
        execute_datastore_write(
            &self.datastore,
            "insert_blocklist_entry",
            "INSERT INTO blocklist
             (id, title_id, source_title, source_hint, quality, download_id, reason, data_json, created_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(id.clone()),
                SqlArg::Text(entry.title_id.clone()),
                SqlArg::OptText(entry.source_title.clone()),
                SqlArg::OptText(entry.source_hint.clone()),
                SqlArg::OptText(entry.quality.clone()),
                SqlArg::OptText(entry.download_id.clone()),
                SqlArg::OptText(entry.reason.clone()),
                opt_json_arg_for_datastore(&self.datastore, Some(&data_json))?,
                timestamp_arg_for_datastore(&self.datastore, &now)?,
            ],
        )
        .await?;
        Ok(id)
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        let sql = format!(
            "SELECT {BLOCKLIST_COLUMNS}
               FROM blocklist
              WHERE title_id = {{}}
              ORDER BY created_at DESC
              LIMIT {{}}"
        );
        fetch_blocklist_entries(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I64(limit as i64),
            ],
        )
        .await
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        let total = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS cnt FROM blocklist",
            &[],
        )
        .await?
        .map(|row| row.i64("cnt"))
        .transpose()?
        .unwrap_or_default();
        let sql = format!(
            "SELECT {BLOCKLIST_COLUMNS}
               FROM blocklist
              ORDER BY created_at DESC
              LIMIT {{}} OFFSET {{}}"
        );
        let entries = fetch_blocklist_entries(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::I64(limit as i64), SqlArg::I64(offset as i64)],
        )
        .await?;
        Ok((entries, total))
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
        fetch_exists(
            self.datastore.read_exec(),
            "SELECT EXISTS(
                 SELECT 1 FROM blocklist
                  WHERE title_id = {}
                    AND LOWER(TRIM(COALESCE(source_title, ''))) = {}
             ) AS matched",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(source_title),
            ],
        )
        .await
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_blocklist_entry",
            "DELETE FROM blocklist WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        fetch_exists(
            self.datastore.read_exec(),
            "SELECT EXISTS(
                 SELECT 1 FROM blocklist
                  WHERE title_id = {}
                    AND LOWER(TRIM(COALESCE(source_title, ''))) = LOWER(TRIM({}))
             ) AS matched",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(source_title.to_string()),
            ],
        )
        .await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_blocklist_for_title",
            "DELETE FROM blocklist WHERE title_id = {}",
            vec![SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }
}

const SUBTITLE_DOWNLOAD_COLUMNS: &str =
    "id, media_file_id, title_id, episode_id, source_kind, language, provider,
    provider_file_id, file_path, score, hearing_impaired, forced, ai_translated,
    machine_translated, uploader, release_info, synced, downloaded_at";

const SUBTITLE_PROBE_CACHE_COLUMNS: &str =
    "media_file_id, file_path, size_bytes, modified_at, language,
    hearing_impaired, detection_source_language, detection_source_hi, probe_version, updated_at";

const SUBTITLE_BLOCKLIST_COLUMNS: &str =
    "id, media_file_id, provider, provider_file_id, language, reason, created_at";

fn subtitle_download_row_to_item(row: &SqlRow) -> AppResult<SubtitleDownload> {
    let source_kind = row.text("source_kind")?;
    let provider = row.opt_text("provider")?.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });
    Ok(SubtitleDownload {
        id: row.text("id")?,
        media_file_id: row.text("media_file_id")?,
        title_id: row.text("title_id")?,
        episode_id: row.opt_text("episode_id")?,
        source_kind: ExternalSubtitleSourceKind::parse(&source_kind).ok_or_else(|| {
            scryer_application::AppError::Repository("invalid external subtitle source kind".into())
        })?,
        language: row.text("language")?,
        provider,
        provider_file_id: row.opt_text("provider_file_id")?,
        file_path: row.text("file_path")?,
        score: row.opt_i32("score")?,
        hearing_impaired: row.bool("hearing_impaired")?,
        forced: row.bool("forced")?,
        ai_translated: row.bool("ai_translated")?,
        machine_translated: row.bool("machine_translated")?,
        uploader: row.opt_text("uploader")?,
        release_info: row.opt_text("release_info")?,
        synced: row.bool("synced")?,
        downloaded_at: required_timestamp_text(row, "downloaded_at")?,
    })
}

fn subtitle_probe_cache_row_to_entry(row: &SqlRow) -> AppResult<ExternalSubtitleProbeCacheEntry> {
    let detection_source_language =
        ExternalSubtitleDetectionSource::parse(&row.text("detection_source_language")?)
            .ok_or_else(|| {
                scryer_application::AppError::Repository(
                    "invalid subtitle probe language detection source".into(),
                )
            })?;
    let detection_source_hi = ExternalSubtitleDetectionSource::parse(
        &row.text("detection_source_hi")?,
    )
    .ok_or_else(|| {
        scryer_application::AppError::Repository(
            "invalid subtitle probe hi detection source".into(),
        )
    })?;

    Ok(ExternalSubtitleProbeCacheEntry {
        media_file_id: row.text("media_file_id")?,
        file_path: row.text("file_path")?,
        size_bytes: row.i64("size_bytes")?,
        modified_at: opt_timestamp_text(row, "modified_at")?,
        language: row.opt_text("language")?,
        hearing_impaired: row.opt_bool("hearing_impaired")?,
        detection_source_language,
        detection_source_hi,
        probe_version: row.i32("probe_version")?,
        updated_at: required_timestamp_text(row, "updated_at")?,
    })
}

fn subtitle_blocklist_row_to_entry(row: &SqlRow) -> AppResult<SubtitleBlocklistEntry> {
    Ok(SubtitleBlocklistEntry {
        id: row.text("id")?,
        media_file_id: row.text("media_file_id")?,
        provider: row.text("provider")?,
        provider_file_id: row.text("provider_file_id")?,
        language: row.text("language")?,
        reason: row.opt_text("reason")?,
        created_at: required_timestamp_text(row, "created_at")?,
    })
}

async fn fetch_subtitle_downloads(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<SubtitleDownload>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(subtitle_download_row_to_item)
        .collect()
}

#[async_trait]
impl SubtitleDownloadRepository for SubtitleDownloadStore {
    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        let sql = format!(
            "SELECT {SUBTITLE_DOWNLOAD_COLUMNS}
               FROM subtitle_downloads
              WHERE title_id = {{}}
              ORDER BY downloaded_at DESC"
        );
        fetch_subtitle_downloads(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn get(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let sql =
            format!("SELECT {SUBTITLE_DOWNLOAD_COLUMNS} FROM subtitle_downloads WHERE id = {{}}");
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(subtitle_download_row_to_item)
        .transpose()
    }

    async fn list_for_media_file(&self, media_file_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        let sql = format!(
            "SELECT {SUBTITLE_DOWNLOAD_COLUMNS}
               FROM subtitle_downloads
              WHERE media_file_id = {{}}
              ORDER BY downloaded_at DESC"
        );
        fetch_subtitle_downloads(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(media_file_id.to_string())],
        )
        .await
    }

    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<ExternalSubtitleProbeCacheEntry>> {
        let sql = format!(
            "SELECT {SUBTITLE_PROBE_CACHE_COLUMNS}
               FROM external_subtitle_probe_cache
              WHERE media_file_id = {{}}
              ORDER BY file_path ASC"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(media_file_id.to_string())],
        )
        .await?
        .iter()
        .map(subtitle_probe_cache_row_to_entry)
        .collect()
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<SubtitleBlocklistEntry>> {
        let sql = format!(
            "SELECT {SUBTITLE_BLOCKLIST_COLUMNS}
               FROM subtitle_blocklist
              WHERE media_file_id = {{}}
              ORDER BY created_at DESC"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(media_file_id.to_string())],
        )
        .await?
        .iter()
        .map(subtitle_blocklist_row_to_entry)
        .collect()
    }

    async fn insert(&self, download: &SubtitleDownload) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "insert_subtitle_download",
            "INSERT INTO subtitle_downloads
             (id, media_file_id, title_id, episode_id, source_kind, language, provider,
              provider_file_id, file_path, score, hearing_impaired, forced,
              ai_translated, machine_translated, uploader, release_info, synced, downloaded_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT (id) DO UPDATE SET
                media_file_id = excluded.media_file_id,
                title_id = excluded.title_id,
                episode_id = excluded.episode_id,
                source_kind = excluded.source_kind,
                language = excluded.language,
                provider = excluded.provider,
                provider_file_id = excluded.provider_file_id,
                file_path = excluded.file_path,
                score = excluded.score,
                hearing_impaired = excluded.hearing_impaired,
                forced = excluded.forced,
                ai_translated = excluded.ai_translated,
                machine_translated = excluded.machine_translated,
                uploader = excluded.uploader,
                release_info = excluded.release_info,
                synced = excluded.synced,
                downloaded_at = excluded.downloaded_at",
            vec![
                SqlArg::Text(download.id.clone()),
                SqlArg::Text(download.media_file_id.clone()),
                SqlArg::Text(download.title_id.clone()),
                SqlArg::OptText(download.episode_id.clone()),
                SqlArg::Text(download.source_kind.as_str().to_string()),
                SqlArg::Text(download.language.clone()),
                SqlArg::Text(download.provider.clone().unwrap_or_default()),
                SqlArg::OptText(download.provider_file_id.clone()),
                SqlArg::Text(download.file_path.clone()),
                SqlArg::OptI32(download.score),
                SqlArg::Bool(download.hearing_impaired),
                SqlArg::Bool(download.forced),
                SqlArg::Bool(download.ai_translated),
                SqlArg::Bool(download.machine_translated),
                SqlArg::OptText(download.uploader.clone()),
                SqlArg::OptText(download.release_info.clone()),
                SqlArg::Bool(download.synced),
                timestamp_arg_for_datastore(&self.datastore, &download.downloaded_at)?,
            ],
        )
        .await?;
        Ok(())
    }

    async fn upsert_probe_cache_entry(
        &self,
        entry: &ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "upsert_external_subtitle_probe_cache_entry",
            "INSERT INTO external_subtitle_probe_cache
             (media_file_id, file_path, size_bytes, modified_at, language,
              hearing_impaired, detection_source_language, detection_source_hi, probe_version, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT (media_file_id, file_path) DO UPDATE SET
                size_bytes = excluded.size_bytes,
                modified_at = excluded.modified_at,
                language = excluded.language,
                hearing_impaired = excluded.hearing_impaired,
                detection_source_language = excluded.detection_source_language,
                detection_source_hi = excluded.detection_source_hi,
                probe_version = excluded.probe_version,
                updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(entry.media_file_id.clone()),
                SqlArg::Text(entry.file_path.clone()),
                SqlArg::I64(entry.size_bytes),
                opt_timestamp_arg_for_datastore(&self.datastore, entry.modified_at.as_deref())?,
                SqlArg::OptText(entry.language.clone()),
                SqlArg::OptBool(entry.hearing_impaired),
                SqlArg::Text(entry.detection_source_language.as_str().to_string()),
                SqlArg::Text(entry.detection_source_hi.as_str().to_string()),
                SqlArg::I32(entry.probe_version),
                timestamp_arg_for_datastore(&self.datastore, &entry.updated_at)?,
            ],
        )
        .await?;
        Ok(())
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "set_subtitle_download_synced",
            "UPDATE subtitle_downloads SET synced = {} WHERE id = {}",
            vec![SqlArg::Bool(synced), SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_subtitle_download", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let sql = format!(
                    "SELECT {SUBTITLE_DOWNLOAD_COLUMNS} FROM subtitle_downloads WHERE id = {{}}"
                );
                let existing =
                    SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &[SqlArg::Text(id.clone())])
                        .await?
                        .as_ref()
                        .map(subtitle_download_row_to_item)
                        .transpose()?;
                if existing.is_some() {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM subtitle_downloads WHERE id = {}",
                        &[SqlArg::Text(id)],
                    )
                    .await?;
                }
                Ok(existing)
            })
        })
        .await
    }

    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_external_subtitle_probe_cache_entry",
            "DELETE FROM external_subtitle_probe_cache
              WHERE media_file_id = {} AND file_path = {}",
            vec![
                SqlArg::Text(media_file_id.to_string()),
                SqlArg::Text(file_path.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        fetch_exists(
            self.datastore.read_exec(),
            "SELECT EXISTS(
                 SELECT 1 FROM subtitle_blocklist
                  WHERE media_file_id = {} AND provider = {} AND provider_file_id = {}
             ) AS matched",
            &[
                SqlArg::Text(media_file_id.to_string()),
                SqlArg::Text(provider.to_string()),
                SqlArg::Text(provider_file_id.to_string()),
            ],
        )
        .await
    }

    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "blocklist_subtitle_download",
            "INSERT INTO subtitle_blocklist
             (id, media_file_id, provider, provider_file_id, language, reason)
             VALUES ({}, {}, {}, {}, {}, {})
             ON CONFLICT (media_file_id, provider, provider_file_id) DO NOTHING",
            vec![
                SqlArg::Text(uuid::Uuid::new_v4().to_string()),
                SqlArg::Text(media_file_id.to_string()),
                SqlArg::Text(provider.to_string()),
                SqlArg::Text(provider_file_id.to_string()),
                SqlArg::Text(language.to_string()),
                SqlArg::OptText(reason.map(str::to_string)),
            ],
        )
        .await?;
        Ok(())
    }
}
