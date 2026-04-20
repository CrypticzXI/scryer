use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

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
const LIBRARY_SCAN_MOVIE_BATCH_SIZE: usize = 32;
const LIBRARY_SCAN_SERIES_BATCH_SIZE: usize = 8;
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

#[derive(Clone, Debug)]
enum StartedLibraryScanOutcome {
    Completed(LibraryScanSummary),
    Canceled(LibraryScanSummary),
}

#[derive(Clone, Debug)]
struct InvalidLibraryRoot {
    path: String,
    reason: String,
}

pub(crate) fn library_scan_cancel_requested(token: Option<&CancellationToken>) -> bool {
    token.is_some_and(CancellationToken::is_cancelled)
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
    pub(crate) async fn ensure_library_scan_cancellation_token(
        &self,
        session_id: &str,
        mode: LibraryScanMode,
    ) -> Option<CancellationToken> {
        if mode != LibraryScanMode::Full {
            return None;
        }

        let mut tokens = self
            .runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await;
        if let Some(existing) = tokens.get(session_id).cloned() {
            return Some(existing);
        }

        let token = CancellationToken::new();
        tokens.insert(session_id.to_string(), token.clone());
        Some(token)
    }

    async fn library_scan_cancellation_token(&self, session_id: &str) -> Option<CancellationToken> {
        self.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .get(session_id)
            .cloned()
    }

    pub(crate) async fn clear_library_scan_cancellation_token(&self, session_id: &str) {
        self.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .remove(session_id);
    }

    pub async fn cancel_library_scan(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<CancelLibraryScanResult> {
        require(actor, &Entitlement::ManageTitle)?;

        let session = self
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("library scan session {session_id}")))?;

        if session.mode != LibraryScanMode::Full {
            return Err(AppError::Validation(
                "only full library scans can be canceled".into(),
            ));
        }

        let token = self
            .library_scan_cancellation_token(session_id)
            .await
            .ok_or_else(|| AppError::Validation("library scan session is not cancelable".into()))?;
        token.cancel();

        Ok(CancelLibraryScanResult {
            session_id: session_id.to_string(),
            accepted: true,
        })
    }

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
        self.ensure_library_scan_cancellation_token(&session.session_id, LibraryScanMode::Full)
            .await;
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
                LibraryScanCoordinator::new(app.clone(), session_id.clone())
                    .fail()
                    .await;
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

        self.ensure_library_scan_cancellation_token(&session.session_id, mode.clone())
            .await;
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
            Ok(StartedLibraryScanOutcome::Completed(summary))
            | Ok(StartedLibraryScanOutcome::Canceled(summary)) => {
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
    ) -> AppResult<StartedLibraryScanOutcome> {
        let library_paths = self.read_library_paths_for_scan_facet(&facet).await?;
        let cancel_token = self.library_scan_cancellation_token(session_id).await;
        let should_apply_import_monitor_snapshot = mode == LibraryScanMode::Full;
        let summary = self
            .execute_started_library_scan_session(
                actor,
                &facet,
                &library_paths,
                session_id,
                mode,
                cancel_token.clone(),
            )
            .await?;
        if library_scan_cancel_requested(cancel_token.as_ref()) {
            self.cancel_started_library_scan_session(session_id, &summary)
                .await;
            Ok(StartedLibraryScanOutcome::Canceled(summary))
        } else {
            if should_apply_import_monitor_snapshot
                && let Err(error) = self
                    .apply_pending_external_import_monitor_snapshot_for_facet(&facet)
                    .await
            {
                let warning_message =
                    "Imported Sonarr/Radarr monitored state could not be applied after this scan. Scryer will retry on the next full scan.".to_string();
                let _ = self
                    .runtime
                    .library
                    .library_scan_tracker
                    .set_warning_message(session_id, Some(warning_message))
                    .await;
                warn!(
                    facet = facet.as_str(),
                    session_id,
                    error = %error,
                    "failed to apply pending external import monitoring snapshot after full scan"
                );
            }
            self.finalize_started_library_scan_session(session_id, &summary)
                .await;
            Ok(StartedLibraryScanOutcome::Completed(summary))
        }
    }

    async fn read_library_paths_for_scan_facet(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Vec<String>> {
        let configured_roots = self.root_folders_for_facet(facet).await?;
        let mut roots = Vec::with_capacity(configured_roots.len());
        let mut seen_roots = HashSet::new();

        for root in configured_roots {
            let path = root.path.trim().to_string();
            if path.is_empty() || !seen_roots.insert(path.clone()) {
                continue;
            }
            roots.push(path);
        }

        if roots.is_empty() {
            return Err(AppError::Validation(format!(
                "{} library roots are not configured",
                facet.as_str()
            )));
        }

        Ok(roots)
    }

    async fn execute_started_library_scan_session(
        &self,
        actor: &User,
        facet: &MediaFacet,
        library_paths: &[String],
        session_id: &str,
        mode: LibraryScanMode,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryScanSummary> {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        let mut valid_roots = Vec::new();
        let mut invalid_roots = Vec::new();

        for library_path in library_paths {
            match tokio::fs::metadata(library_path).await {
                Ok(metadata) if metadata.is_dir() => valid_roots.push(library_path.as_str()),
                Ok(_) => invalid_roots.push(InvalidLibraryRoot {
                    path: library_path.clone(),
                    reason: "path exists but is not a directory".to_string(),
                }),
                Err(error) => invalid_roots.push(InvalidLibraryRoot {
                    path: library_path.clone(),
                    reason: error.to_string(),
                }),
            }
        }

        if valid_roots.is_empty() {
            if let Some(invalid_root) = invalid_roots.first() {
                return Err(AppError::Validation(format!(
                    "library path is not a directory: {}",
                    invalid_root.path
                )));
            }

            return Err(AppError::Validation(format!(
                "{} library roots are not configured",
                facet.as_str()
            )));
        }

        let mut summary = LibraryScanSummary::default();

        if !invalid_roots.is_empty() {
            warn!(
                session_id = %session_id,
                facet = facet.as_str(),
                invalid_root_count = invalid_roots.len(),
                valid_root_count = valid_roots.len(),
                "skipping invalid library roots during scan"
            );
            for invalid_root in &invalid_roots {
                warn!(
                    session_id = %session_id,
                    facet = facet.as_str(),
                    library_path = %invalid_root.path,
                    reason = %invalid_root.reason,
                    "skipping invalid library root"
                );
            }

            summary.skipped = summary.skipped.saturating_add(invalid_roots.len());
            coordinator.add_metadata_total(invalid_roots.len()).await;
            coordinator.mark_metadata_failed(invalid_roots.len()).await;
            coordinator.publish_progress().await;
        }

        let valid_root_count = valid_roots.len();

        for (root_index, library_path) in valid_roots.into_iter().enumerate() {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            let finalize_discovery_on_drain =
                mode == LibraryScanMode::Full && root_index + 1 == valid_root_count;
            let root_summary = match (mode.clone(), facet) {
                (LibraryScanMode::Full, MediaFacet::Movie) => {
                    scan_library_movies(
                        self,
                        actor,
                        facet,
                        library_path,
                        session_id,
                        finalize_discovery_on_drain,
                        cancel_token.clone(),
                    )
                    .await?
                }
                (LibraryScanMode::Full, MediaFacet::Series | MediaFacet::Anime) => {
                    scan_library_series(
                        self,
                        actor,
                        facet,
                        library_path,
                        session_id,
                        finalize_discovery_on_drain,
                        cancel_token.clone(),
                    )
                    .await?
                }
                (LibraryScanMode::Additive, MediaFacet::Movie) => {
                    background_refresh_movies(self, actor, library_path, session_id).await?
                }
                (LibraryScanMode::Additive, MediaFacet::Series | MediaFacet::Anime) => {
                    background_refresh_series(self, actor, facet, library_path, session_id).await?
                }
            };
            summary.absorb(&root_summary);
        }

        if mode == LibraryScanMode::Additive || library_scan_cancel_requested(cancel_token.as_ref())
        {
            coordinator.mark_discovery_complete(false).await;
            coordinator.publish_progress().await;
        }

        Ok(summary)
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

    async fn cancel_started_library_scan_session(
        &self,
        session_id: &str,
        summary: &LibraryScanSummary,
    ) {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        coordinator.set_summary(summary.clone()).await;
        coordinator.cancel().await;
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

        result.map(|outcome| match outcome {
            StartedLibraryScanOutcome::Completed(summary)
            | StartedLibraryScanOutcome::Canceled(summary) => summary,
        })
    }
}
