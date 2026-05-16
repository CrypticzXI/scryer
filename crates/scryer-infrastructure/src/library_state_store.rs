use async_trait::async_trait;
use scryer_application::{
    AppResult, BlocklistRepository, CutoffUnmetQualitySummary, EpisodeScopedMediaFile,
    HousekeepingRepository, InsertMediaFileInput, LibraryProbeRepository, LibraryProbeSignature,
    LibraryScanUnmatchedItem, LibraryScanUnmatchedItemRepository, MediaFileAnalysis,
    MediaFileRepository, NewBlocklistEntry, PendingImportStatus, PendingRelease,
    PendingReleaseRepository, ReleaseDecision, SubtitleDownloadRepository,
    TitleEpisodeProgressSummary, TitleImageBlob, TitleImageKind, TitleImageReplacement,
    TitleImageRepository, TitleImageSyncTask, TitleMediaFile, TitleMediaSizeSummary,
    TitleQualitySummary, WantedItem, WantedItemRepository, WantedItemsQuery,
    subtitles::ExternalSubtitleProbeCacheEntry,
};
use scryer_domain::{BlocklistEntry, DomainEventType, MediaFacet};

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
use crate::queries::subtitle::{
    delete_external_subtitle_probe_cache_entry, get_subtitle_download,
    is_blocklisted as is_subtitle_blocklisted, list_blocklist_for_media_file,
    list_external_subtitle_probe_cache_for_media_file, list_subtitle_downloads_for_media_file,
    list_subtitle_downloads_for_title, upsert_external_subtitle_probe_cache_entry,
};
use crate::queries::workflow::get_library_probe_signature_query;
use crate::title_images::{get_title_image_blob_query, list_titles_requiring_image_refresh_query};

#[derive(Clone)]
pub struct LibraryStateStore<S> {
    sql: S,
}

pub trait LibraryStateSql: Clone + Send + Sync + 'static {}

impl<T> LibraryStateSql for T where T: Clone + Send + Sync + 'static {}

impl<S> LibraryStateStore<S> {
    pub(crate) fn from_sql(sql: S) -> Self {
        Self { sql }
    }
}

pub type SqliteLibraryStateStore = LibraryStateStore<SqliteLibraryStateSql>;

#[derive(Clone)]
pub struct SqliteLibraryStateSql {
    db: SqliteServices,
}

impl SqliteLibraryStateStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self::from_sql(SqliteLibraryStateSql::new(db))
    }
}

impl SqliteLibraryStateSql {
    fn new(db: &SqliteServices) -> Self {
        Self { db: db.clone() }
    }
}

#[async_trait]
impl LibraryProbeRepository for SqliteLibraryStateSql {
    async fn get_probe_signature(
        &self,
        title_id: &str,
    ) -> AppResult<Option<LibraryProbeSignature>> {
        get_library_probe_signature_query(self.db.pool(), title_id)
            .await
            .map(|record| {
                record.map(|record| LibraryProbeSignature {
                    title_id: record.title_id,
                    path: record.path,
                    probe_signature_scheme: record.probe_signature_scheme,
                    probe_signature_value: record.probe_signature_value,
                    last_probed_at: record
                        .last_probed_at
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                        .map(|value| value.with_timezone(&chrono::Utc)),
                    last_changed_at: record
                        .last_changed_at
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                        .map(|value| value.with_timezone(&chrono::Utc)),
                })
            })
    }

    async fn upsert_probe_signature(&self, probe: &LibraryProbeSignature) -> AppResult<()> {
        self.db
            .upsert_library_probe_signature(
                &probe.title_id,
                &probe.path,
                probe.probe_signature_scheme.clone(),
                probe.probe_signature_value.clone(),
                probe.last_probed_at.map(|value| value.to_rfc3339()),
                probe.last_changed_at.map(|value| value.to_rfc3339()),
            )
            .await
    }

    async fn delete_probe_signatures_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        crate::queries::workflow::delete_library_probe_signatures_for_title_ids_query(
            self.db.pool(),
            title_ids,
        )
        .await
    }
}

#[async_trait]
impl LibraryScanUnmatchedItemRepository for SqliteLibraryStateSql {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        self.db.upsert_library_scan_unmatched_item(item).await
    }

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        get_library_scan_unmatched_item_query(&self.db.pool, id).await
    }

    async fn delete_library_scan_unmatched_item(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        self.db
            .delete_library_scan_unmatched_item(library_id, facet, item_path)
            .await
    }

    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32> {
        crate::queries::library_scan_unmatched::delete_library_scan_unmatched_items_for_library_query(
            self.db.pool(),
            library_id,
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
        self.db
            .list_library_scan_unmatched_items(facet, scan_root, status, limit, offset)
            .await
    }

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        self.db
            .count_library_scan_unmatched_items(facet, scan_root, status)
            .await
    }
}

#[async_trait]
impl MediaFileRepository for SqliteLibraryStateSql {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        self.db.insert_media_file(input).await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        self.db.link_file_to_episode(file_id, episode_id).await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        list_media_files_for_title_query(&self.db.pool, title_id).await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        list_live_media_files_for_episode_ids_query(&self.db.pool, title_id, episode_ids).await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        list_title_media_size_summaries_query(&self.db.pool, title_ids).await
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        list_title_quality_summaries_query(&self.db.pool, title_ids).await
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        list_cutoff_unmet_quality_summaries_query(&self.db.pool, title_ids).await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        list_title_episode_progress_summaries_query(&self.db.pool, title_ids).await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        self.db.update_media_file_analysis(file_id, analysis).await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        self.db
            .update_media_file_source_signature(
                file_id,
                size_bytes,
                source_signature_scheme,
                source_signature_value,
            )
            .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        self.db.update_media_file_path(file_id, file_path).await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        self.db.mark_scan_failed(file_id, error).await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        self.db.delete_media_file(file_id).await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        get_media_file_by_id_query(&self.db.pool, file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        get_media_file_by_path_query(&self.db.pool, file_path).await
    }
}

#[async_trait]
impl WantedItemRepository for SqliteLibraryStateSql {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        self.db.upsert_wanted_item(item).await
    }

    async fn ensure_wanted_item_seeded(&self, item: &WantedItem) -> AppResult<String> {
        self.db.ensure_wanted_item_seeded_atomic(item.clone()).await
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<WantedItem>> {
        self.db
            .list_due_wanted_items(now, batch_limit, excluded_facets)
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
        self.db
            .update_wanted_item_status(
                id,
                status,
                next_search_at,
                last_search_at,
                search_count,
                current_score,
                grabbed_release,
            )
            .await
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        self.db
            .get_wanted_item_for_title(title_id, episode_id)
            .await
    }

    async fn complete_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        current_score: Option<i32>,
    ) -> AppResult<bool> {
        self.db
            .complete_wanted_item_for_title(title_id, episode_id, last_search_at, current_score)
            .await
    }

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        self.db.delete_wanted_items_for_title(title_id).await
    }

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        self.db
            .delete_wanted_items_for_collection(collection_id)
            .await
    }

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        self.db.delete_wanted_items_for_episode(episode_id).await
    }

    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        self.db.reset_fruitless_wanted_items(now).await
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        self.db.insert_release_decision(decision).await
    }

    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        self.db.get_wanted_item_by_id(id).await
    }

    async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>> {
        self.db.list_wanted_items(query).await
    }

    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        self.db.count_wanted_items(query).await
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        self.db
            .list_release_decisions_for_title(title_id, limit)
            .await
    }

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        self.db
            .list_release_decisions_for_wanted_item(wanted_item_id, limit)
            .await
    }
}

#[async_trait]
impl HousekeepingRepository for SqliteLibraryStateSql {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        self.db.delete_release_decisions_older_than(days).await
    }

    async fn delete_title_history_older_than(&self, _days: i64) -> AppResult<u32> {
        // Legacy title_history rows are retired by migration 0085; nothing remains to prune.
        Ok(0)
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        self.db.delete_release_attempts_older_than(days).await
    }

    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        self.db
            .delete_dispatched_event_outboxes_older_than(days)
            .await
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        self.db.delete_history_events_older_than(days).await
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        self.db
            .delete_domain_events_older_than_for_types(days, event_types)
            .await
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        self.db
            .delete_download_import_artifacts_older_than(days)
            .await
    }

    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        self.db.delete_terminal_imports_older_than(days).await
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        self.db
            .delete_terminal_download_queue_commands_older_than(days)
            .await
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        self.db.delete_rule_set_history_older_than(days).await
    }

    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        crate::queries::housekeeping::delete_history_events_for_title_ids_query(
            self.db.pool(),
            title_ids,
        )
        .await
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        crate::queries::housekeeping::delete_download_import_artifacts_for_title_ids_query(
            self.db.pool(),
            title_ids,
        )
        .await
    }

    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        crate::queries::housekeeping::delete_release_attempts_for_title_ids_query(
            self.db.pool(),
            title_ids,
        )
        .await
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        list_all_media_file_paths_query(self.db.pool()).await
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        self.db.delete_media_files_by_ids(ids).await
    }
}

#[async_trait]
impl PendingReleaseRepository for SqliteLibraryStateSql {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        self.db.insert_pending_release(release).await
    }

    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        self.db.list_expired_pending_releases(now).await
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        self.db.list_waiting_pending_releases().await
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        self.db.get_pending_release(id).await
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        self.db
            .list_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        crate::queries::pending_releases::list_pending_releases_for_title_query(
            &self.db.pool,
            title_id,
        )
        .await
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        self.db
            .update_pending_release_status(id, status, grabbed_at)
            .await
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        self.db
            .list_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        self.db
            .delete_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        self.db.list_all_standby_pending_releases().await
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: scryer_application::PendingReleaseStatus,
        next_status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        self.db
            .compare_and_set_pending_release_status(id, current_status, next_status, grabbed_at)
            .await
    }

    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        self.db
            .supersede_pending_releases_for_wanted_item(wanted_item_id, except_id)
            .await
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        self.db.delete_pending_releases_for_title(title_id).await
    }
}

#[async_trait]
impl BlocklistRepository for SqliteLibraryStateSql {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        let data_json = serde_json::to_string(&entry.data)
            .map_err(|err| scryer_application::AppError::Repository(err.to_string()))?;
        self.db
            .insert_blocklist_entry(
                entry.title_id.clone(),
                entry.source_title.clone(),
                entry.source_hint.clone(),
                entry.quality.clone(),
                entry.download_id.clone(),
                entry.reason.clone(),
                Some(data_json),
            )
            .await
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        self.db.list_blocklist_for_title(title_id, limit).await
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        self.db.list_blocklist_all(limit, offset).await
    }

    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool> {
        self.db
            .has_recorded_download_failure(title_id, source_title)
            .await
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        self.db.delete_blocklist_entry(id).await
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        self.db.is_blocklisted(title_id, source_title).await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.db.delete_blocklist_for_title(title_id).await
    }
}

#[async_trait]
impl SubtitleDownloadRepository for SqliteLibraryStateSql {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        list_subtitle_downloads_for_title(self.db.pool(), title_id).await
    }

    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        get_subtitle_download(self.db.pool(), id).await
    }

    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        list_subtitle_downloads_for_media_file(self.db.pool(), media_file_id).await
    }

    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<ExternalSubtitleProbeCacheEntry>> {
        list_external_subtitle_probe_cache_for_media_file(self.db.pool(), media_file_id).await
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>> {
        list_blocklist_for_media_file(self.db.pool(), media_file_id).await
    }

    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        self.db.insert_subtitle_download(download).await
    }

    async fn upsert_probe_cache_entry(
        &self,
        entry: &ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        run_with_sqlite_busy_retries("upsert_external_subtitle_probe_cache_entry", || {
            upsert_external_subtitle_probe_cache_entry(self.db.pool(), entry)
        })
        .await
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        self.db.set_subtitle_download_synced(id, synced).await
    }

    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        self.db.delete_subtitle_download(id).await
    }

    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        run_with_sqlite_busy_retries("delete_external_subtitle_probe_cache_entry", || {
            delete_external_subtitle_probe_cache_entry(self.db.pool(), media_file_id, file_path)
        })
        .await
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        is_subtitle_blocklisted(self.db.pool(), media_file_id, provider, provider_file_id).await
    }

    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        self.db
            .blocklist_subtitle_download(
                media_file_id,
                provider,
                provider_file_id,
                language,
                reason,
            )
            .await
            .map(|_| ())
    }
}

#[async_trait]
impl TitleImageRepository for SqliteLibraryStateSql {
    async fn list_titles_requiring_image_refresh(
        &self,
        kind: TitleImageKind,
        limit: usize,
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        list_titles_requiring_image_refresh_query(&self.db.pool, kind, limit).await
    }

    async fn replace_title_image(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
    ) -> AppResult<()> {
        self.db.replace_title_image(title_id, replacement).await
    }

    async fn replace_title_image_and_append_event(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
        event: scryer_domain::NewDomainEvent,
    ) -> AppResult<scryer_domain::DomainEvent> {
        self.db
            .replace_title_image_and_append_event(title_id, replacement, event)
            .await
    }

    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        get_title_image_blob_query(&self.db.pool, title_id, kind, variant_key).await
    }
}

#[async_trait]
impl<S> LibraryProbeRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + LibraryProbeRepository,
{
    async fn get_probe_signature(
        &self,
        title_id: &str,
    ) -> AppResult<Option<LibraryProbeSignature>> {
        self.sql.get_probe_signature(title_id).await
    }

    async fn upsert_probe_signature(&self, probe: &LibraryProbeSignature) -> AppResult<()> {
        self.sql.upsert_probe_signature(probe).await
    }

    async fn delete_probe_signatures_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.sql
            .delete_probe_signatures_for_title_ids(title_ids)
            .await
    }
}

#[async_trait]
impl<S> LibraryScanUnmatchedItemRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + LibraryScanUnmatchedItemRepository,
{
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        self.sql.upsert_library_scan_unmatched_item(item).await
    }

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        self.sql.get_library_scan_unmatched_item(id).await
    }

    async fn delete_library_scan_unmatched_item(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        self.sql
            .delete_library_scan_unmatched_item(library_id, facet, item_path)
            .await
    }

    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32> {
        self.sql.delete_for_library(library_id).await
    }

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        self.sql
            .list_library_scan_unmatched_items(facet, scan_root, status, limit, offset)
            .await
    }

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        self.sql
            .count_library_scan_unmatched_items(facet, scan_root, status)
            .await
    }
}

#[async_trait]
impl<S> MediaFileRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + MediaFileRepository,
{
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        self.sql.insert_media_file(input).await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        self.sql.link_file_to_episode(file_id, episode_id).await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        self.sql.list_media_files_for_title(title_id).await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        self.sql
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        self.sql.list_title_media_size_summaries(title_ids).await
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        self.sql.list_title_quality_summaries(title_ids).await
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        self.sql
            .list_cutoff_unmet_quality_summaries(title_ids)
            .await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        self.sql
            .list_title_episode_progress_summaries(title_ids)
            .await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        self.sql.update_media_file_analysis(file_id, analysis).await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        self.sql
            .update_media_file_source_signature(
                file_id,
                size_bytes,
                source_signature_scheme,
                source_signature_value,
            )
            .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        self.sql.update_media_file_path(file_id, file_path).await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        self.sql.mark_scan_failed(file_id, error).await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        self.sql.delete_media_file(file_id).await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        self.sql.get_media_file_by_id(file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        self.sql.get_media_file_by_path(file_path).await
    }
}

#[async_trait]
impl<S> WantedItemRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + WantedItemRepository,
{
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        self.sql.upsert_wanted_item(item).await
    }

    async fn ensure_wanted_item_seeded(&self, item: &WantedItem) -> AppResult<String> {
        self.sql.ensure_wanted_item_seeded(item).await
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<WantedItem>> {
        self.sql
            .list_due_wanted_items(now, batch_limit, excluded_facets)
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
        self.sql
            .update_wanted_item_status(
                id,
                status,
                next_search_at,
                last_search_at,
                search_count,
                current_score,
                grabbed_release,
            )
            .await
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        self.sql
            .get_wanted_item_for_title(title_id, episode_id)
            .await
    }

    async fn complete_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        current_score: Option<i32>,
    ) -> AppResult<bool> {
        self.sql
            .complete_wanted_item_for_title(title_id, episode_id, last_search_at, current_score)
            .await
    }

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        self.sql.delete_wanted_items_for_title(title_id).await
    }

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        self.sql
            .delete_wanted_items_for_collection(collection_id)
            .await
    }

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        self.sql.delete_wanted_items_for_episode(episode_id).await
    }

    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        self.sql.reset_fruitless_wanted_items(now).await
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        self.sql.insert_release_decision(decision).await
    }

    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        self.sql.get_wanted_item_by_id(id).await
    }

    async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>> {
        self.sql.list_wanted_items(query).await
    }

    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        self.sql.count_wanted_items(query).await
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        self.sql
            .list_release_decisions_for_title(title_id, limit)
            .await
    }

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        self.sql
            .list_release_decisions_for_wanted_item(wanted_item_id, limit)
            .await
    }
}

#[async_trait]
impl<S> HousekeepingRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + HousekeepingRepository,
{
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql.delete_release_decisions_older_than(days).await
    }

    async fn delete_title_history_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql.delete_title_history_older_than(days).await
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql.delete_release_attempts_older_than(days).await
    }

    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql
            .delete_dispatched_event_outboxes_older_than(days)
            .await
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql.delete_history_events_older_than(days).await
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        self.sql
            .delete_domain_events_older_than_for_types(days, event_types)
            .await
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql
            .delete_download_import_artifacts_older_than(days)
            .await
    }

    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql.delete_terminal_imports_older_than(days).await
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        self.sql
            .delete_terminal_download_queue_commands_older_than(days)
            .await
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        self.sql.delete_rule_set_history_older_than(days).await
    }

    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.sql
            .delete_history_events_for_title_ids(title_ids)
            .await
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        self.sql
            .delete_download_import_artifacts_for_title_ids(title_ids)
            .await
    }

    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.sql
            .delete_release_attempts_for_title_ids(title_ids)
            .await
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        self.sql.list_all_media_file_paths().await
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        self.sql.delete_media_files_by_ids(ids).await
    }
}

#[async_trait]
impl<S> PendingReleaseRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + PendingReleaseRepository,
{
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        self.sql.insert_pending_release(release).await
    }

    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        self.sql.list_expired_pending_releases(now).await
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        self.sql.list_waiting_pending_releases().await
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        self.sql.get_pending_release(id).await
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        self.sql
            .list_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        self.sql.list_pending_releases_for_title(title_id).await
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        self.sql
            .update_pending_release_status(id, status, grabbed_at)
            .await
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        self.sql
            .list_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        self.sql
            .delete_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        self.sql.list_all_standby_pending_releases().await
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: scryer_application::PendingReleaseStatus,
        next_status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        self.sql
            .compare_and_set_pending_release_status(id, current_status, next_status, grabbed_at)
            .await
    }

    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        self.sql
            .supersede_pending_releases_for_wanted_item(wanted_item_id, except_id)
            .await
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        self.sql.delete_pending_releases_for_title(title_id).await
    }
}

#[async_trait]
impl<S> BlocklistRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + BlocklistRepository,
{
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        self.sql.add(entry).await
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        BlocklistRepository::list_for_title(&self.sql, title_id, limit).await
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        self.sql.list_all(limit, offset).await
    }

    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool> {
        self.sql
            .has_recorded_download_failure(title_id, source_title)
            .await
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        self.sql.remove(id).await
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        BlocklistRepository::is_blocklisted(&self.sql, title_id, source_title).await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.sql.delete_for_title(title_id).await
    }
}

#[async_trait]
impl<S> SubtitleDownloadRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + SubtitleDownloadRepository,
{
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        SubtitleDownloadRepository::list_for_title(&self.sql, title_id).await
    }

    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        self.sql.get(id).await
    }

    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        self.sql.list_for_media_file(media_file_id).await
    }

    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<ExternalSubtitleProbeCacheEntry>> {
        self.sql
            .list_probe_cache_for_media_file(media_file_id)
            .await
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>> {
        self.sql.list_blocklist_for_media_file(media_file_id).await
    }

    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        self.sql.insert(download).await
    }

    async fn upsert_probe_cache_entry(
        &self,
        entry: &ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        self.sql.upsert_probe_cache_entry(entry).await
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        self.sql.set_synced(id, synced).await
    }

    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        self.sql.delete(id).await
    }

    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        self.sql
            .delete_probe_cache_entry(media_file_id, file_path)
            .await
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        SubtitleDownloadRepository::is_blocklisted(
            &self.sql,
            media_file_id,
            provider,
            provider_file_id,
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
        self.sql
            .blocklist(media_file_id, provider, provider_file_id, language, reason)
            .await
    }
}

#[async_trait]
impl<S> TitleImageRepository for LibraryStateStore<S>
where
    S: LibraryStateSql + TitleImageRepository,
{
    async fn list_titles_requiring_image_refresh(
        &self,
        kind: TitleImageKind,
        limit: usize,
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        self.sql
            .list_titles_requiring_image_refresh(kind, limit)
            .await
    }

    async fn replace_title_image(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
    ) -> AppResult<()> {
        self.sql.replace_title_image(title_id, replacement).await
    }

    async fn replace_title_image_and_append_event(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
        event: scryer_domain::NewDomainEvent,
    ) -> AppResult<scryer_domain::DomainEvent> {
        self.sql
            .replace_title_image_and_append_event(title_id, replacement, event)
            .await
    }

    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        self.sql
            .get_title_image_blob(title_id, kind, variant_key)
            .await
    }
}
