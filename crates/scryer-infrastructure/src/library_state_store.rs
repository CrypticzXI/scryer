use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppResult, BlocklistRepository, CutoffUnmetQualitySummary, EpisodeScopedMediaFile,
    HousekeepingRepository, InsertMediaFileInput, LibraryProbeRepository, LibraryProbeSignature,
    LibraryScanUnmatchedItem, LibraryScanUnmatchedItemRepository, MediaFileAnalysis,
    MediaFileRepository, NewBlocklistEntry, PendingImportStatus, PendingRelease,
    PendingReleaseRepository, ReleaseDecision, SubtitleDownloadRepository,
    TitleEpisodeProgressSummary, TitleMediaFile, TitleMediaSizeSummary, TitleQualitySummary,
    WantedItem, WantedItemRepository, WantedItemsQuery, subtitles::ExternalSubtitleProbeCacheEntry,
};
use scryer_domain::{BlocklistEntry, DomainEventType, MediaFacet};

mod postgres;

use self::postgres::PostgresLibraryStateSql;
use crate::SqliteServices;
use crate::queries::housekeeping::list_all_media_file_paths_query;
use crate::queries::library_scan_unmatched::get_library_scan_unmatched_item_query;
use crate::queries::media_file::{
    get_media_file_by_id_query, get_media_file_by_path_query,
    list_cutoff_unmet_quality_summaries_query, list_live_media_files_for_episode_ids_query,
    list_media_files_for_title_query, list_title_episode_progress_summaries_query,
    list_title_media_size_summaries_query, list_title_quality_summaries_query,
};
use crate::queries::sql_runtime::run_with_sqlite_busy_retries;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};
use crate::queries::subtitle::{
    delete_external_subtitle_probe_cache_entry, get_subtitle_download,
    is_blocklisted as is_subtitle_blocklisted, list_blocklist_for_media_file,
    list_external_subtitle_probe_cache_for_media_file, list_subtitle_downloads_for_media_file,
    list_subtitle_downloads_for_title, upsert_external_subtitle_probe_cache_entry,
};

fn blocklist_row_to_entry(row: crate::queries::blocklist::BlocklistRow) -> BlocklistEntry {
    BlocklistEntry {
        id: row.id,
        title_id: row.title_id,
        source_title: row.source_title,
        source_hint: row.source_hint,
        quality: row.quality,
        download_id: row.download_id,
        reason: row.reason,
        data_json: row.data_json,
        created_at: row.created_at,
    }
}

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
pub struct LibraryStateStore {
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
impl_store_new!(LibraryStateStore);

macro_rules! dispatch_library_state_backend {
    ($store:expr, $trait_name:ident :: $method:ident ( $($arg:expr),* $(,)? )) => {{
        match &($store).datastore {
            StoreDatastore::Sqlite { pool, writer_gate } => {
                let backend = SqliteLibraryStateSql::new(pool.clone(), writer_gate.clone());
                $trait_name::$method(&backend, $($arg),*).await
            }
            StoreDatastore::Postgres { pool } => {
                let backend = PostgresLibraryStateSql::new(pool.clone());
                $trait_name::$method(&backend, $($arg),*).await
            }
        }
    }};
}

#[derive(Clone)]
struct SqliteLibraryStateSql {
    pool: sqlx::SqlitePool,
    writer_gate: Arc<tokio::sync::Mutex<()>>,
}

impl SqliteLibraryStateSql {
    fn new(pool: sqlx::SqlitePool, writer_gate: Arc<tokio::sync::Mutex<()>>) -> Self {
        Self { pool, writer_gate }
    }

    async fn with_writer_lock<T>(
        &self,
        future: impl std::future::Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        let _writer = self.writer_gate.lock().await;
        future.await
    }
}

#[async_trait]
impl LibraryScanUnmatchedItemRepository for SqliteLibraryStateSql {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("upsert_library_scan_unmatched_item", || {
            crate::queries::library_scan_unmatched::upsert_library_scan_unmatched_item_query(
                &self.pool, item,
            )
        })
        .await
    }

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        get_library_scan_unmatched_item_query(&self.pool, id).await
    }

    async fn delete_library_scan_unmatched_item(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_library_scan_unmatched_item", || {
            crate::queries::library_scan_unmatched::delete_library_scan_unmatched_item_query(
                &self.pool,
                library_id,
                facet.clone(),
                item_path,
            )
        })
        .await
    }

    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32> {
        self.with_writer_lock(
            crate::queries::library_scan_unmatched::delete_library_scan_unmatched_items_for_library_query(
                &self.pool,
                library_id,
            ),
        )
        .await
    }

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        crate::queries::library_scan_unmatched::list_library_scan_unmatched_items_query(
            &self.pool, facet, scan_root, status, limit, offset,
        )
        .await
    }

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        crate::queries::library_scan_unmatched::count_library_scan_unmatched_items_query(
            &self.pool, facet, scan_root, status,
        )
        .await
    }
}

#[async_trait]
impl MediaFileRepository for SqliteLibraryStateSql {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("insert_media_file", || {
            crate::queries::media_file::insert_media_file_query(&self.pool, input)
        })
        .await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("link_file_to_episode", || {
            crate::queries::media_file::link_file_to_episode_query(&self.pool, file_id, episode_id)
        })
        .await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        list_media_files_for_title_query(&self.pool, title_id).await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        list_live_media_files_for_episode_ids_query(&self.pool, title_id, episode_ids).await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        list_title_media_size_summaries_query(&self.pool, title_ids).await
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        list_title_quality_summaries_query(&self.pool, title_ids).await
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        list_cutoff_unmet_quality_summaries_query(&self.pool, title_ids).await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        list_title_episode_progress_summaries_query(&self.pool, title_ids).await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("update_media_file_analysis", || {
            crate::queries::media_file::update_media_file_analysis_query(
                &self.pool, file_id, &analysis,
            )
        })
        .await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("update_media_file_source_signature", || {
            crate::queries::media_file::update_media_file_source_signature_query(
                &self.pool,
                file_id,
                size_bytes,
                source_signature_scheme.as_deref(),
                source_signature_value.as_deref(),
            )
        })
        .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("update_media_file_path", || {
            crate::queries::media_file::update_media_file_path_query(&self.pool, file_id, file_path)
        })
        .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("mark_scan_failed", || {
            crate::queries::media_file::mark_scan_failed_query(&self.pool, file_id, error)
        })
        .await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_media_file", || {
            crate::queries::media_file::delete_media_file_query(&self.pool, file_id)
        })
        .await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        get_media_file_by_id_query(&self.pool, file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        get_media_file_by_path_query(&self.pool, file_path).await
    }
}

#[async_trait]
impl WantedItemRepository for SqliteLibraryStateSql {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("upsert_wanted_item", || {
            crate::queries::wanted::upsert_wanted_item_query(&self.pool, item)
        })
        .await
    }

    async fn ensure_wanted_item_seeded(&self, item: &WantedItem) -> AppResult<String> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("ensure_wanted_item_seeded", || {
            crate::queries::wanted::ensure_wanted_item_seeded_query(&self.pool, item)
        })
        .await
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<WantedItem>> {
        crate::queries::wanted::list_due_wanted_items_query(
            &self.pool,
            now,
            batch_limit,
            excluded_facets,
        )
        .await
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
        self.with_writer_lock(crate::queries::wanted::update_wanted_item_status_query(
            &self.pool,
            id,
            status,
            next_search_at,
            last_search_at,
            search_count,
            current_score,
            grabbed_release,
        ))
        .await
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        crate::queries::wanted::get_wanted_item_for_title_query(&self.pool, title_id, episode_id)
            .await
    }

    async fn complete_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        current_score: Option<i32>,
    ) -> AppResult<bool> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("complete_wanted_item_for_title", || {
            crate::queries::wanted::complete_wanted_item_for_title_query(
                &self.pool,
                title_id,
                episode_id,
                last_search_at,
                current_score,
            )
        })
        .await
    }

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        self.with_writer_lock(crate::queries::wanted::delete_wanted_items_for_title_query(
            &self.pool, title_id,
        ))
        .await
    }

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        self.with_writer_lock(
            crate::queries::wanted::delete_wanted_items_for_collection_query(
                &self.pool,
                collection_id,
            ),
        )
        .await
    }

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        self.with_writer_lock(
            crate::queries::wanted::delete_wanted_items_for_episode_query(&self.pool, episode_id),
        )
        .await
    }

    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        self.with_writer_lock(crate::queries::wanted::reset_fruitless_wanted_items_query(
            &self.pool, now,
        ))
        .await
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        self.with_writer_lock(crate::queries::wanted::insert_release_decision_query(
            &self.pool, decision,
        ))
        .await
    }

    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        crate::queries::wanted::get_wanted_item_by_id_query(&self.pool, id).await
    }

    async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>> {
        crate::queries::wanted::list_wanted_items_query(&self.pool, &query).await
    }

    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        crate::queries::wanted::count_wanted_items_query(&self.pool, &query).await
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        crate::queries::wanted::list_release_decisions_for_title_query(&self.pool, title_id, limit)
            .await
    }

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        crate::queries::wanted::list_release_decisions_for_wanted_item_query(
            &self.pool,
            wanted_item_id,
            limit,
        )
        .await
    }
}

#[async_trait]
impl HousekeepingRepository for SqliteLibraryStateSql {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_release_decisions_older_than", || {
            crate::queries::housekeeping::delete_release_decisions_older_than_query(
                &self.pool, days,
            )
        })
        .await
    }

    async fn delete_title_history_older_than(&self, _days: i64) -> AppResult<u32> {
        // Legacy title_history rows are retired by migration 0085; nothing remains to prune.
        Ok(0)
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_release_attempts_older_than", || {
            crate::queries::housekeeping::delete_release_attempts_older_than_query(&self.pool, days)
        })
        .await
    }

    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_dispatched_event_outboxes_older_than", || {
            crate::queries::housekeeping::delete_dispatched_event_outboxes_older_than_query(
                &self.pool, days,
            )
        })
        .await
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_history_events_older_than", || {
            crate::queries::housekeeping::delete_history_events_older_than_query(&self.pool, days)
        })
        .await
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_domain_events_older_than_for_types", || {
            crate::queries::housekeeping::delete_domain_events_older_than_for_types_query(
                &self.pool,
                days,
                event_types,
            )
        })
        .await
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_download_import_artifacts_older_than", || {
            crate::queries::housekeeping::delete_download_import_artifacts_older_than_query(
                &self.pool, days,
            )
        })
        .await
    }

    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_terminal_imports_older_than", || {
            crate::queries::housekeeping::delete_terminal_imports_older_than_query(&self.pool, days)
        })
        .await
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_terminal_download_queue_commands_older_than", || {
            crate::queries::housekeeping::delete_terminal_download_queue_commands_older_than_query(
                &self.pool, days,
            )
        })
        .await
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_rule_set_history_older_than", || {
            crate::queries::housekeeping::delete_rule_set_history_older_than_query(&self.pool, days)
        })
        .await
    }

    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.with_writer_lock(
            crate::queries::housekeeping::delete_history_events_for_title_ids_query(
                &self.pool, title_ids,
            ),
        )
        .await
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        self.with_writer_lock(
            crate::queries::housekeeping::delete_download_import_artifacts_for_title_ids_query(
                &self.pool, title_ids,
            ),
        )
        .await
    }

    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.with_writer_lock(
            crate::queries::housekeeping::delete_release_attempts_for_title_ids_query(
                &self.pool, title_ids,
            ),
        )
        .await
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        list_all_media_file_paths_query(&self.pool).await
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_media_files_by_ids", || {
            crate::queries::housekeeping::delete_media_files_by_ids_query(&self.pool, ids)
        })
        .await
    }
}

#[async_trait]
impl PendingReleaseRepository for SqliteLibraryStateSql {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        self.with_writer_lock(
            crate::queries::pending_releases::insert_pending_release_query(&self.pool, release),
        )
        .await
    }

    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        crate::queries::pending_releases::list_expired_pending_releases_query(&self.pool, now).await
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        crate::queries::pending_releases::list_waiting_pending_releases_query(&self.pool).await
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        crate::queries::pending_releases::get_pending_release_query(&self.pool, id).await
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        crate::queries::pending_releases::list_pending_releases_for_wanted_item_query(
            &self.pool,
            wanted_item_id,
        )
        .await
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        crate::queries::pending_releases::list_pending_releases_for_title_query(
            &self.pool, title_id,
        )
        .await
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        self.with_writer_lock(
            crate::queries::pending_releases::update_pending_release_status_query(
                &self.pool, id, status, grabbed_at,
            ),
        )
        .await
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        crate::queries::pending_releases::list_standby_pending_releases_for_wanted_item_query(
            &self.pool,
            wanted_item_id,
        )
        .await
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        self.with_writer_lock(
            crate::queries::pending_releases::delete_standby_pending_releases_for_wanted_item_query(
                &self.pool,
                wanted_item_id,
            ),
        )
        .await
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        crate::queries::pending_releases::list_all_standby_pending_releases_query(&self.pool).await
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: scryer_application::PendingReleaseStatus,
        next_status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        self.with_writer_lock(
            crate::queries::pending_releases::compare_and_set_pending_release_status_query(
                &self.pool,
                id,
                current_status,
                next_status,
                grabbed_at,
            ),
        )
        .await
    }

    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        self.with_writer_lock(
            crate::queries::pending_releases::supersede_pending_releases_for_wanted_item_query(
                &self.pool,
                wanted_item_id,
                except_id,
            ),
        )
        .await
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        self.with_writer_lock(
            crate::queries::pending_releases::delete_pending_releases_for_title_query(
                &self.pool, title_id,
            ),
        )
        .await
    }
}

#[async_trait]
impl BlocklistRepository for SqliteLibraryStateSql {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        let data_json = serde_json::to_string(&entry.data)
            .map_err(|err| scryer_application::AppError::Repository(err.to_string()))?;
        self.with_writer_lock(crate::queries::blocklist::insert_blocklist_entry_query(
            &self.pool,
            &entry.title_id,
            entry.source_title.as_deref(),
            entry.source_hint.as_deref(),
            entry.quality.as_deref(),
            entry.download_id.as_deref(),
            entry.reason.as_deref(),
            Some(&data_json),
        ))
        .await
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        crate::queries::blocklist::list_blocklist_for_title_query(&self.pool, title_id, limit)
            .await
            .map(|rows| rows.into_iter().map(blocklist_row_to_entry).collect())
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        crate::queries::blocklist::list_blocklist_all_query(&self.pool, limit, offset)
            .await
            .map(|(rows, total)| {
                let entries = rows.into_iter().map(blocklist_row_to_entry).collect();
                (entries, total)
            })
    }

    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool> {
        crate::queries::blocklist::has_recorded_download_failure_query(
            &self.pool,
            title_id,
            source_title,
        )
        .await
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        self.with_writer_lock(crate::queries::blocklist::delete_blocklist_entry_query(
            &self.pool, id,
        ))
        .await
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        crate::queries::blocklist::is_blocklisted_query(&self.pool, title_id, source_title).await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.with_writer_lock(crate::queries::blocklist::delete_blocklist_for_title_query(
            &self.pool, title_id,
        ))
        .await
    }
}

#[async_trait]
impl SubtitleDownloadRepository for SqliteLibraryStateSql {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        list_subtitle_downloads_for_title(&self.pool, title_id).await
    }

    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        get_subtitle_download(&self.pool, id).await
    }

    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        list_subtitle_downloads_for_media_file(&self.pool, media_file_id).await
    }

    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<ExternalSubtitleProbeCacheEntry>> {
        list_external_subtitle_probe_cache_for_media_file(&self.pool, media_file_id).await
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>> {
        list_blocklist_for_media_file(&self.pool, media_file_id).await
    }

    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("insert_subtitle_download", || {
            crate::queries::subtitle::insert_subtitle_download(&self.pool, download)
        })
        .await
    }

    async fn upsert_probe_cache_entry(
        &self,
        entry: &ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("upsert_external_subtitle_probe_cache_entry", || {
            upsert_external_subtitle_probe_cache_entry(&self.pool, entry)
        })
        .await
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("set_subtitle_download_synced", || {
            crate::queries::subtitle::update_subtitle_download_synced(&self.pool, id, synced)
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_subtitle_download", || {
            crate::queries::subtitle::delete_subtitle_download(&self.pool, id)
        })
        .await
    }

    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("delete_external_subtitle_probe_cache_entry", || {
            delete_external_subtitle_probe_cache_entry(&self.pool, media_file_id, file_path)
        })
        .await
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        is_subtitle_blocklisted(&self.pool, media_file_id, provider, provider_file_id).await
    }

    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        let _writer = self.writer_gate.lock().await;
        run_with_sqlite_busy_retries("blocklist_subtitle_download", || {
            crate::queries::subtitle::insert_blocklist_entry(
                &self.pool,
                media_file_id,
                provider,
                provider_file_id,
                language,
                reason,
            )
        })
        .await
        .map(|_| ())
    }
}

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
            SqlArg::OptTimestamp(probe.last_probed_at.clone()),
            SqlArg::OptTimestamp(probe.last_changed_at.clone()),
            SqlArg::Timestamp(now.clone()),
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

#[async_trait]
impl LibraryScanUnmatchedItemRepository for LibraryStateStore {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        dispatch_library_state_backend!(
            self,
            LibraryScanUnmatchedItemRepository::upsert_library_scan_unmatched_item(item)
        )
    }

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        dispatch_library_state_backend!(
            self,
            LibraryScanUnmatchedItemRepository::get_library_scan_unmatched_item(id)
        )
    }

    async fn delete_library_scan_unmatched_item(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            LibraryScanUnmatchedItemRepository::delete_library_scan_unmatched_item(
                library_id, facet, item_path,
            )
        )
    }

    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            LibraryScanUnmatchedItemRepository::delete_for_library(library_id)
        )
    }

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        dispatch_library_state_backend!(
            self,
            LibraryScanUnmatchedItemRepository::list_library_scan_unmatched_items(
                facet, scan_root, status, limit, offset,
            )
        )
    }

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        dispatch_library_state_backend!(
            self,
            LibraryScanUnmatchedItemRepository::count_library_scan_unmatched_items(
                facet, scan_root, status,
            )
        )
    }
}

#[async_trait]
impl MediaFileRepository for LibraryStateStore {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        dispatch_library_state_backend!(self, MediaFileRepository::insert_media_file(input))
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::link_file_to_episode(file_id, episode_id)
        )
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::list_media_files_for_title(title_id)
        )
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::list_live_media_files_for_episode_ids(title_id, episode_ids)
        )
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::list_title_media_size_summaries(title_ids)
        )
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::list_title_quality_summaries(title_ids)
        )
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::list_cutoff_unmet_quality_summaries(title_ids)
        )
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::list_title_episode_progress_summaries(title_ids)
        )
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::update_media_file_analysis(file_id, analysis)
        )
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::update_media_file_source_signature(
                file_id,
                size_bytes,
                source_signature_scheme,
                source_signature_value,
            )
        )
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::update_media_file_path(file_id, file_path)
        )
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        dispatch_library_state_backend!(self, MediaFileRepository::mark_scan_failed(file_id, error))
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        dispatch_library_state_backend!(self, MediaFileRepository::delete_media_file(file_id))
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        dispatch_library_state_backend!(self, MediaFileRepository::get_media_file_by_id(file_id))
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        dispatch_library_state_backend!(
            self,
            MediaFileRepository::get_media_file_by_path(file_path)
        )
    }
}

#[async_trait]
impl WantedItemRepository for LibraryStateStore {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        dispatch_library_state_backend!(self, WantedItemRepository::upsert_wanted_item(item))
    }

    async fn ensure_wanted_item_seeded(&self, item: &WantedItem) -> AppResult<String> {
        dispatch_library_state_backend!(self, WantedItemRepository::ensure_wanted_item_seeded(item))
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<WantedItem>> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::list_due_wanted_items(now, batch_limit, excluded_facets)
        )
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
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::update_wanted_item_status(
                id,
                status,
                next_search_at,
                last_search_at,
                search_count,
                current_score,
                grabbed_release,
            )
        )
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::get_wanted_item_for_title(title_id, episode_id)
        )
    }

    async fn complete_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        current_score: Option<i32>,
    ) -> AppResult<bool> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::complete_wanted_item_for_title(
                title_id,
                episode_id,
                last_search_at,
                current_score,
            )
        )
    }

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::delete_wanted_items_for_title(title_id)
        )
    }

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::delete_wanted_items_for_collection(collection_id)
        )
    }

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::delete_wanted_items_for_episode(episode_id)
        )
    }

    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::reset_fruitless_wanted_items(now)
        )
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::insert_release_decision(decision)
        )
    }

    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        dispatch_library_state_backend!(self, WantedItemRepository::get_wanted_item_by_id(id))
    }

    async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>> {
        dispatch_library_state_backend!(self, WantedItemRepository::list_wanted_items(query))
    }

    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        dispatch_library_state_backend!(self, WantedItemRepository::count_wanted_items(query))
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::list_release_decisions_for_title(title_id, limit)
        )
    }

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        dispatch_library_state_backend!(
            self,
            WantedItemRepository::list_release_decisions_for_wanted_item(wanted_item_id, limit)
        )
    }
}

#[async_trait]
impl HousekeepingRepository for LibraryStateStore {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_release_decisions_older_than(days)
        )
    }

    async fn delete_title_history_older_than(&self, days: i64) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_title_history_older_than(days)
        )
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_release_attempts_older_than(days)
        )
    }

    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_dispatched_event_outboxes_older_than(days)
        )
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_history_events_older_than(days)
        )
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_domain_events_older_than_for_types(days, event_types)
        )
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_download_import_artifacts_older_than(days)
        )
    }

    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_terminal_imports_older_than(days)
        )
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_terminal_download_queue_commands_older_than(days)
        )
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_rule_set_history_older_than(days)
        )
    }

    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_history_events_for_title_ids(title_ids)
        )
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_download_import_artifacts_for_title_ids(title_ids)
        )
    }

    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_release_attempts_for_title_ids(title_ids)
        )
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        dispatch_library_state_backend!(self, HousekeepingRepository::list_all_media_file_paths())
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        dispatch_library_state_backend!(
            self,
            HousekeepingRepository::delete_media_files_by_ids(ids)
        )
    }
}

#[async_trait]
impl PendingReleaseRepository for LibraryStateStore {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::insert_pending_release(release)
        )
    }

    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::list_expired_pending_releases(now)
        )
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::list_waiting_pending_releases()
        )
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        dispatch_library_state_backend!(self, PendingReleaseRepository::get_pending_release(id))
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::list_pending_releases_for_wanted_item(wanted_item_id)
        )
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::list_pending_releases_for_title(title_id)
        )
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::update_pending_release_status(id, status, grabbed_at)
        )
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::list_standby_pending_releases_for_wanted_item(wanted_item_id)
        )
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::delete_standby_pending_releases_for_wanted_item(
                wanted_item_id
            )
        )
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::list_all_standby_pending_releases()
        )
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: scryer_application::PendingReleaseStatus,
        next_status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::compare_and_set_pending_release_status(
                id,
                current_status,
                next_status,
                grabbed_at,
            )
        )
    }

    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::supersede_pending_releases_for_wanted_item(
                wanted_item_id,
                except_id,
            )
        )
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            PendingReleaseRepository::delete_pending_releases_for_title(title_id)
        )
    }
}

#[async_trait]
impl BlocklistRepository for LibraryStateStore {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        dispatch_library_state_backend!(self, BlocklistRepository::add(entry))
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        dispatch_library_state_backend!(self, BlocklistRepository::list_for_title(title_id, limit))
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        dispatch_library_state_backend!(self, BlocklistRepository::list_all(limit, offset))
    }

    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool> {
        dispatch_library_state_backend!(
            self,
            BlocklistRepository::has_recorded_download_failure(title_id, source_title)
        )
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        dispatch_library_state_backend!(self, BlocklistRepository::remove(id))
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        dispatch_library_state_backend!(
            self,
            BlocklistRepository::is_blocklisted(title_id, source_title)
        )
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        dispatch_library_state_backend!(self, BlocklistRepository::delete_for_title(title_id))
    }
}

#[async_trait]
impl SubtitleDownloadRepository for LibraryStateStore {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        dispatch_library_state_backend!(self, SubtitleDownloadRepository::list_for_title(title_id))
    }

    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        dispatch_library_state_backend!(self, SubtitleDownloadRepository::get(id))
    }

    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        dispatch_library_state_backend!(
            self,
            SubtitleDownloadRepository::list_for_media_file(media_file_id)
        )
    }

    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<ExternalSubtitleProbeCacheEntry>> {
        dispatch_library_state_backend!(
            self,
            SubtitleDownloadRepository::list_probe_cache_for_media_file(media_file_id)
        )
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>> {
        dispatch_library_state_backend!(
            self,
            SubtitleDownloadRepository::list_blocklist_for_media_file(media_file_id)
        )
    }

    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        dispatch_library_state_backend!(self, SubtitleDownloadRepository::insert(download))
    }

    async fn upsert_probe_cache_entry(
        &self,
        entry: &ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            SubtitleDownloadRepository::upsert_probe_cache_entry(entry)
        )
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        dispatch_library_state_backend!(self, SubtitleDownloadRepository::set_synced(id, synced))
    }

    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        dispatch_library_state_backend!(self, SubtitleDownloadRepository::delete(id))
    }

    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            SubtitleDownloadRepository::delete_probe_cache_entry(media_file_id, file_path)
        )
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        dispatch_library_state_backend!(
            self,
            SubtitleDownloadRepository::is_blocklisted(media_file_id, provider, provider_file_id,)
        )
    }

    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        dispatch_library_state_backend!(
            self,
            SubtitleDownloadRepository::blocklist(
                media_file_id,
                provider,
                provider_file_id,
                language,
                reason,
            )
        )
    }
}
