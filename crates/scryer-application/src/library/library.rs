use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::library_discovery::{
    BackgroundRefreshProbeOutcome, MovieTopLevelEntry, elapsed_ms_u64, list_child_directories,
    list_movie_top_level_entries, matching_movie_nfo_path, run_background_refresh_probe_with_delta,
    stream_child_directories_batched, stream_movie_top_level_entries_batched,
};
use crate::library_scan::{LibraryDirectoryScanResult, source_signature_from_std_metadata};
use crate::library_scan_coordinator::LibraryScanCoordinator;
use crate::library_scan_helpers::{
    LibraryScanSessionDropGuard, spawn_library_discovery_queue,
    wait_for_projected_library_scan_session,
};
use crate::library_scan_metadata::{
    MetadataLookupBatchStats, MetadataSearchResults, PreparedMovieLibraryScanCandidate,
    PreparedMovieLibraryScanEntry, PreparedSeriesLibraryScanCandidate,
    StreamingMetadataProgressUpdate, StreamingMovieMetadataResolver,
    build_movie_metadata_batch_stats, build_series_metadata_batch_stats,
    movie_candidate_batch_search_keys, prepare_movie_library_scan_entries,
    prepare_series_library_scan_candidates, resolve_full_scan_metadata_batches,
    select_movie_metadata_from_batch_results, select_series_metadata_from_batch_results,
    series_candidate_batch_search_keys, stream_prepared_movie_library_scan_entries,
};
use crate::library_scan_titles::{
    append_movie_title, append_series_title, build_movie_probe_path_indexes,
    build_movie_title_indexes, build_new_title_from_metadata_match,
    build_series_title_folder_path_index, build_series_title_indexes,
    find_existing_title_index_for_metadata_match, update_movie_probe_path_index,
    update_series_title_folder_path_index,
};
use crate::library_scan_unmatched::{
    build_movie_unmatched_scan_item, build_series_unmatched_scan_item,
    clear_library_scan_unmatched_item, format_library_scan_unmatched_search_attempts,
    normalize_library_scan_item_path, persist_library_scan_unmatched_item,
    reconcile_library_scan_unmatched_items,
};
use tracing::{info, warn};

const LIBRARY_METADATA_LOOKUP_CONCURRENCY: usize = 4;
const LIBRARY_SCAN_BATCH_SIZE: usize = 128;
const LIBRARY_SCAN_TITLE_WALK_CONCURRENCY: usize = 4;
const TITLE_SCAN_FILE_BATCH_SIZE: usize = 128;
#[path = "scan/candidates.rs"]
mod scan_candidates;
#[path = "scan/full.rs"]
mod scan_full;
#[path = "scan/refresh.rs"]
mod scan_refresh;
#[path = "scan/title_files.rs"]
mod scan_title_files;
#[path = "scan/title_finalize.rs"]
mod scan_title_finalize;
#[path = "scan/title_scan.rs"]
mod scan_title_scan;

use scan_candidates::{
    process_movie_full_scan_candidate, process_movie_refresh_candidate,
    process_resolved_movie_full_scan_candidate, process_resolved_movie_refresh_candidate,
    process_resolved_series_full_scan_candidate, process_resolved_series_refresh_candidate,
    process_series_full_scan_candidate, process_series_refresh_candidate,
    scan_episodic_title_directory_for_progress_metrics,
};
use scan_full::{scan_library_movies, scan_library_series};
use scan_refresh::{
    background_refresh_movies, background_refresh_series,
    maybe_probe_existing_series_title_for_background_refresh,
};
use scan_title_files::{
    FileSourceSnapshot, PlannedTitleScanFile, PlannedTitleScanRecord, TitleScanLayoutSummary,
    build_title_episode_lookup, classify_title_scan_layout, file_source_signature_from_metadata,
    file_source_snapshot_from_library_file, merge_title_scan_option_tags,
    resolve_target_episodes_from_lookup, title_media_file_matches_snapshot,
};
use scan_title_finalize::{finalize_movie_scan_file, finalize_title_scan_file};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LibraryScanTitleWalkMode {
    Full,
    Additive,
    OneOff,
}

impl LibraryScanTitleWalkMode {
    fn as_file_finalize_mode(self) -> LibraryScanMode {
        match self {
            Self::Additive => LibraryScanMode::Additive,
            Self::Full | Self::OneOff => LibraryScanMode::Full,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct LibraryScanMovieCleanupContext {
    stale_collection_ids: Vec<String>,
}

#[derive(Clone, Debug)]
enum LibraryScanTitleFacetPlan {
    Movie(LibraryScanMovieCleanupContext),
    Episodic,
}

#[derive(Clone, Debug)]
pub(crate) struct LibraryScanTitleWork {
    title: Title,
    facet_plan: LibraryScanTitleFacetPlan,
    discovered_files: Option<Vec<LibraryFile>>,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
}

impl LibraryScanTitleWork {
    fn discovered_file_count(&self) -> usize {
        self.discovered_files
            .as_ref()
            .map(Vec::len)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LibraryTitleWalkResult {
    summary: LibraryScanSummary,
}

fn append_unique_library_files(target: &mut Vec<LibraryFile>, files: Vec<LibraryFile>) -> usize {
    let mut added = 0usize;

    for file in files {
        if target.iter().any(|existing| existing.path == file.path) {
            continue;
        }

        target.push(file);
        added = added.saturating_add(1);
    }

    added
}

fn merge_library_scan_title_work(
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    mut work: LibraryScanTitleWork,
) {
    let title_id = work.title.id.clone();
    match workset.get_mut(&title_id) {
        Some(existing) => {
            if let Some(files) = work.discovered_files.take() {
                let existing_files = existing.discovered_files.get_or_insert_with(Vec::new);
                append_unique_library_files(existing_files, files);
            }

            if let (
                LibraryScanTitleFacetPlan::Movie(existing_cleanup),
                LibraryScanTitleFacetPlan::Movie(new_cleanup),
            ) = (&mut existing.facet_plan, work.facet_plan)
            {
                for collection_id in new_cleanup.stale_collection_ids {
                    if !existing_cleanup
                        .stale_collection_ids
                        .contains(&collection_id)
                    {
                        existing_cleanup.stale_collection_ids.push(collection_id);
                    }
                }
            }

            existing.title = work.title;
            existing.mode = work.mode;
            existing.created_in_scan |= work.created_in_scan;
        }
        None => {
            workset.insert(title_id, work);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TitleScanProgressDelta {
    completed: usize,
    failed: usize,
}

impl TitleScanProgressDelta {
    fn completed(count: usize) -> Self {
        Self {
            completed: count,
            failed: 0,
        }
    }

    fn failed(count: usize) -> Self {
        Self {
            completed: 0,
            failed: count,
        }
    }

    fn total(self) -> usize {
        self.completed.saturating_add(self.failed)
    }

    fn absorb(&mut self, other: Self) {
        self.completed = self.completed.saturating_add(other.completed);
        self.failed = self.failed.saturating_add(other.failed);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TitleScanFinalizeOutcome {
    progress: TitleScanProgressDelta,
    title_updated: bool,
}

async fn flush_title_scan_progress_batch(
    app: &AppUseCase,
    session_id: Option<&str>,
    pending_progress: &mut TitleScanProgressDelta,
) {
    let Some(session_id) = session_id else {
        *pending_progress = TitleScanProgressDelta::default();
        return;
    };
    if pending_progress.total() == 0 {
        return;
    }

    let delta = std::mem::take(pending_progress);
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    if delta.completed > 0 {
        coordinator.mark_file_completed(delta.completed).await;
    }
    if delta.failed > 0 {
        coordinator.mark_file_failed(delta.failed).await;
    }
    coordinator.publish_progress().await;
}

impl AppUseCase {
    pub async fn scan_library(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<LibraryScanSummary> {
        self.scan_library_with_tracking(actor, facet, None, LibraryScanMode::Full)
            .await
    }

    pub async fn trigger_library_scan(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<LibraryScanSession> {
        require(actor, &Entitlement::ManageTitle)?;

        let (_coordinator, session) =
            LibraryScanCoordinator::start(self.clone(), facet.clone(), LibraryScanMode::Full, None)
                .await?;
        let mut session_guard =
            LibraryScanSessionDropGuard::new(self.clone(), session.session_id.clone());

        let app = self.clone();
        let actor = actor.clone();
        let session_id = session.session_id.clone();
        tokio::spawn(async move {
            if let Err(error) = app
                .run_started_library_scan_session(
                    &actor,
                    facet.clone(),
                    &session_id,
                    LibraryScanMode::Full,
                )
                .await
            {
                warn!(
                    error = %error,
                    session_id = %session_id,
                    facet = facet.as_str(),
                    "library scan task failed"
                );
            }
        });

        session_guard.disarm();
        Ok(session)
    }

    pub(crate) async fn scan_library_with_tracking(
        &self,
        actor: &User,
        facet: MediaFacet,
        session_id_override: Option<String>,
        mode: LibraryScanMode,
    ) -> AppResult<LibraryScanSummary> {
        require(actor, &Entitlement::ManageTitle)?;

        let (_coordinator, session) = LibraryScanCoordinator::start(
            self.clone(),
            facet.clone(),
            mode.clone(),
            session_id_override,
        )
        .await?;
        let mut session_guard =
            LibraryScanSessionDropGuard::new(self.clone(), session.session_id.clone());

        let result = self
            .run_started_library_scan_session(actor, facet, &session.session_id, mode)
            .await;

        if result.is_err() {
            LibraryScanCoordinator::new(self.clone(), session.session_id.clone())
                .fail()
                .await;
        }

        session_guard.disarm();

        match result {
            Ok(summary) => {
                let projected_session =
                    wait_for_projected_library_scan_session(self, &session.session_id).await?;

                if projected_session.status == LibraryScanStatus::Failed {
                    return Err(AppError::Repository("library scan failed".into()));
                }

                Ok(projected_session.summary.unwrap_or(summary))
            }
            Err(error) => Err(error),
        }
    }

    async fn run_started_library_scan_session(
        &self,
        actor: &User,
        facet: MediaFacet,
        session_id: &str,
        mode: LibraryScanMode,
    ) -> AppResult<LibraryScanSummary> {
        let library_path = self.read_library_path_for_scan_facet(&facet).await?;
        let summary = self
            .execute_started_library_scan_session(actor, &facet, &library_path, session_id, mode)
            .await?;
        self.finalize_started_library_scan_session(session_id, &summary)
            .await;
        Ok(summary)
    }

    async fn read_library_path_for_scan_facet(&self, facet: &MediaFacet) -> AppResult<String> {
        let path_key = match facet {
            MediaFacet::Movie => "movies.path",
            MediaFacet::Series => "series.path",
            MediaFacet::Anime => "anime.path",
        };

        self.read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, path_key, None)
            .await?
            .ok_or_else(|| AppError::Validation(format!("{path_key} is not configured")))
    }

    async fn execute_started_library_scan_session(
        &self,
        actor: &User,
        facet: &MediaFacet,
        library_path: &str,
        session_id: &str,
        mode: LibraryScanMode,
    ) -> AppResult<LibraryScanSummary> {
        match (mode, facet) {
            (LibraryScanMode::Full, MediaFacet::Movie) => {
                scan_library_movies(self, actor, facet, library_path, session_id).await
            }
            (LibraryScanMode::Full, MediaFacet::Series | MediaFacet::Anime) => {
                scan_library_series(self, actor, facet, library_path, session_id).await
            }
            (LibraryScanMode::Additive, MediaFacet::Movie) => {
                background_refresh_movies(self, actor, library_path, session_id).await
            }
            (LibraryScanMode::Additive, MediaFacet::Series | MediaFacet::Anime) => {
                background_refresh_series(self, actor, facet, library_path, session_id).await
            }
        }
    }

    async fn finalize_started_library_scan_session(
        &self,
        session_id: &str,
        summary: &LibraryScanSummary,
    ) {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        coordinator.set_summary(summary.clone()).await;
        coordinator.publish_progress().await;
        coordinator.maybe_complete().await;
    }

    pub(crate) async fn background_library_refresh_with_tracking(
        &self,
        actor: &User,
        facet: MediaFacet,
        session_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        require(actor, &Entitlement::ManageTitle)?;

        let (_coordinator, session) = LibraryScanCoordinator::start(
            self.clone(),
            facet.clone(),
            LibraryScanMode::Additive,
            Some(session_id.to_string()),
        )
        .await?;

        let result = self
            .run_started_library_scan_session(
                actor,
                facet,
                &session.session_id,
                LibraryScanMode::Additive,
            )
            .await;

        if result.is_err() {
            LibraryScanCoordinator::new(self.clone(), session.session_id.clone())
                .fail()
                .await;
        }

        result
    }
}
