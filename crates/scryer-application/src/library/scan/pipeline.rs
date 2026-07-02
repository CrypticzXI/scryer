use super::*;
use crate::library_discovery::list_series_loose_root_files;
use crate::library_scan_helpers::require_directory_library_path;
use crate::library_scan_metadata::{
    BatchMetadataSearchKey, MovieCandidateEvidence, execute_batch_metadata_searches,
    prepare_movie_candidate_evidence, prepare_series_library_scan_candidate,
    prepare_series_library_scan_candidate_from_file, split_ready_metadata_candidates,
};
use crate::stored_paths::path_to_stored_string;
use std::collections::VecDeque;

use super::scan_title_scan::title_requires_scan_hydration;

/// Concurrent sidecar/evidence reads per scan root. Evidence is cheap (one
/// readdir plus at most two sidecar reads), so this can run much wider than
/// the recursive inventory walks without hammering SMB mounts.
const LIBRARY_SCAN_EVIDENCE_CONCURRENCY: usize = 32;
/// SMG match batches allowed in flight at once. Supersedes the old
/// scan-path `LIBRARY_METADATA_LOOKUP_CONCURRENCY` gate.
const LIBRARY_SCAN_METADATA_IN_FLIGHT_BATCHES: usize = 2;
/// Timer flush for the match batcher: a partial batch is dispatched this long
/// after its first candidate arrived.
const LIBRARY_SCAN_MATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(50);
/// Rendezvous storage high-water mark. Once this many file paths are parked
/// waiting for match decisions, the inventory phase pauses until storage
/// drains. Evidence emission is never paused.
const LIBRARY_SCAN_MEDIA_INVENTORY_PATH_HIGH_WATER: usize = 100_000;
/// Discovery-to-match channel capacity (at least two SMG batches).
const LIBRARY_SCAN_MATCH_INPUT_QUEUE_CAPACITY: usize = 2 * LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE;
/// Cap on candidates parked in the match worker waiting for SMG results
/// before the worker stops pulling intake and lets channel backpressure hold.
const LIBRARY_SCAN_MATCH_PENDING_HIGH_WATER: usize = 4 * LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE;
const LIBRARY_SCAN_CANDIDATE_EVENT_QUEUE_CAPACITY: usize = 64;
const LIBRARY_SCAN_INVENTORY_EVENT_QUEUE_CAPACITY: usize = 64;
const LIBRARY_SCAN_MATCH_EVENT_QUEUE_CAPACITY: usize = 64;
/// Hydration runs downstream of matching in bulk batches so a fresh episodic
/// library does not degrade into one SMG metadata call per title.
const LIBRARY_SCAN_HYDRATION_IN_FLIGHT_BATCHES: usize = 2;

pub(super) type ScanCandidateKey = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LibraryScanPipelineKind {
    Movie,
    Series,
}

pub(super) enum ScanPipelineCandidate {
    Movie(Box<PreparedMovieLibraryScanCandidate>),
    Series(Box<PreparedSeriesLibraryScanCandidate>),
}

impl ScanPipelineCandidate {
    fn batch_search_keys(&self) -> AppResult<Vec<BatchMetadataSearchKey>> {
        match self {
            Self::Movie(candidate) => movie_candidate_batch_search_keys(candidate),
            Self::Series(candidate) => series_candidate_batch_search_keys(candidate),
        }
    }
}

/// Events emitted by the root enumerator and candidate jobs.
enum ScanCandidateJobEvent {
    Candidate {
        key: ScanCandidateKey,
        candidate: ScanPipelineCandidate,
        /// Scoped candidates (loose root files) carry their own file list in
        /// the matched work and never rendezvous with an inventory result.
        scoped: bool,
        inventory_cancel: CancellationToken,
    },
    Skipped {
        item_path: String,
    },
    EvidenceFailed {
        item_path: String,
        error: AppError,
    },
    EvidenceDone {
        metrics: CandidateJobMetrics,
    },
    DiscoveryFailed {
        error: AppError,
    },
}

/// Events emitted by recursive inventory/count walks.
enum ScanInventoryJobEvent {
    Inventory {
        key: ScanCandidateKey,
        files: Vec<LibraryFile>,
    },
    InventoryFailed {
        key: ScanCandidateKey,
        item_path: String,
        error: AppError,
    },
    InventoryCanceled {
        key: ScanCandidateKey,
    },
}

#[derive(Clone, Copy, Debug, Default)]
struct CandidateJobMetrics {
    candidates_emitted: usize,
    skipped: usize,
    failed: usize,
    inline_inventory_emitted: usize,
    inventory_walks_queued: usize,
    inventory_walks_started: usize,
}

/// Events emitted by the SMG match worker.
enum ScanMatchWorkerEvent {
    Matched {
        key: ScanCandidateKey,
        work: Box<LibraryScanTitleWork>,
    },
    Terminal {
        key: ScanCandidateKey,
    },
    Done(Box<ScanMatchWorkerReport>),
}

struct ScanMatchWorkerReport {
    summary: LibraryScanSummary,
    unmatched_items: Vec<LibraryScanUnmatchedItem>,
    seen_paths: HashSet<String>,
    stats: MetadataLookupBatchStats,
}

/// Staging queue handed to the shared candidate-processing functions. The
/// match worker inspects the staged work after each call to decide whether
/// the candidate matched (work present) or reached a non-media terminal.
struct PipelineTitleWorkSink {
    staged: Option<LibraryScanTitleWork>,
}

impl LibraryScanTitleWorkQueue for PipelineTitleWorkSink {
    fn enqueue(&mut self, work: LibraryScanTitleWork) -> bool {
        self.staged = Some(work);
        true
    }
}

enum CandidateMatchState {
    Pending,
    MatchedAwaitingInventory(Box<LibraryScanTitleWork>),
    Dispatched,
    Terminal,
}

enum CandidateInventoryState {
    Pending,
    Ready(Vec<LibraryFile>),
    Consumed,
    Failed,
    Canceled,
}

struct CandidateRuntime {
    item_path: String,
    scoped: bool,
    match_state: CandidateMatchState,
    inventory: CandidateInventoryState,
    inventory_cancel: CancellationToken,
}

impl CandidateRuntime {
    fn inventory_terminal(&self) -> bool {
        !matches!(self.inventory, CandidateInventoryState::Pending)
    }
}

pub(super) struct LibraryScanPipelineRequest<'a> {
    pub(super) app: &'a AppUseCase,
    pub(super) actor: &'a User,
    pub(super) facet: &'a MediaFacet,
    pub(super) library_id: &'a str,
    pub(super) library_path: &'a str,
    pub(super) session_id: &'a str,
    pub(super) mark_discovery_complete_on_drain: bool,
    pub(super) cancel_token: Option<CancellationToken>,
    pub(super) scan_hints: Option<LibraryScanHintSet>,
    pub(super) kind: LibraryScanPipelineKind,
}

pub(super) async fn run_library_scan_pipeline(
    request: LibraryScanPipelineRequest<'_>,
) -> AppResult<LibraryScanSummary> {
    let LibraryScanPipelineRequest {
        app,
        actor,
        facet,
        library_id,
        library_path,
        session_id,
        mark_discovery_complete_on_drain,
        cancel_token,
        scan_hints,
        kind,
    } = request;

    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    require_directory_library_path(library_path)?;

    let (candidate_events_tx, mut candidate_events_rx) =
        tokio::sync::mpsc::channel(LIBRARY_SCAN_CANDIDATE_EVENT_QUEUE_CAPACITY);
    let (inventory_events_tx, mut inventory_events_rx) =
        tokio::sync::mpsc::channel(LIBRARY_SCAN_INVENTORY_EVENT_QUEUE_CAPACITY);
    let (match_input_tx, match_input_rx) =
        tokio::sync::mpsc::channel(LIBRARY_SCAN_MATCH_INPUT_QUEUE_CAPACITY);
    let (match_events_tx, mut match_events_rx) =
        tokio::sync::mpsc::channel(LIBRARY_SCAN_MATCH_EVENT_QUEUE_CAPACITY);
    let (storage_watch_tx, storage_watch_rx) = tokio::sync::watch::channel(0usize);

    let jobs_handle = spawn_candidate_jobs(CandidateJobContext {
        app: app.clone(),
        session_id: session_id.to_string(),
        library_path: library_path.to_string(),
        kind,
        scan_hints,
        mark_discovery_complete_on_drain,
        cancel_token: cancel_token.clone(),
        candidate_events: candidate_events_tx,
        inventory_events: inventory_events_tx,
        storage_watch: storage_watch_rx,
    })?;

    let worker_handle = tokio::spawn(run_scan_match_worker(
        ScanMatchWorkerContext {
            app: app.clone(),
            actor: actor.clone(),
            facet: facet.clone(),
            library_id: library_id.to_string(),
            library_path: library_path.to_string(),
            session_id: session_id.to_string(),
            metadata_language: app.metadata_language().await,
            kind,
        },
        match_input_rx,
        match_events_tx,
        cancel_token.clone(),
    ));

    let mut pool = LibraryScanMediaAnalysisPool::for_scan_pipeline(
        app,
        actor,
        session_id,
        cancel_token.clone(),
    )
    .await?;

    let mut summary = LibraryScanSummary::default();
    let mut candidates: HashMap<ScanCandidateKey, CandidateRuntime> = HashMap::new();
    let mut forward_queue: VecDeque<(ScanCandidateKey, ScanPipelineCandidate)> = VecDeque::new();
    let mut match_input_tx = Some(match_input_tx);
    let mut evidence_done = false;
    let mut inventory_done = false;
    let mut match_done = false;
    let mut worker_report: Option<Box<ScanMatchWorkerReport>> = None;
    let mut stored_inventory_paths = 0usize;
    let mut file_total_marked = false;
    let mut discovery_error: Option<AppError> = None;
    // Duplicate candidates resolving to already-covered title work are
    // deduplicated by the analysis pool; they count as skipped, not matched.
    let mut media_dedup_skips = 0usize;

    let mut hydration = ScanHydrationBatcher::new(app.clone(), cancel_token.clone());

    loop {
        if evidence_done && match_done && inventory_done && forward_queue.is_empty() {
            break;
        }
        if discovery_error.is_some() {
            break;
        }

        // Close the match worker input once all evidence has been forwarded;
        // recursive inventory/count walks must not hold title matching open.
        if evidence_done && forward_queue.is_empty() {
            match_input_tx = None;
        }

        let hydration_deadline = hydration.deadline_instant();
        tokio::select! {
            event = candidate_events_rx.recv(), if !evidence_done => {
                match event {
                    Some(ScanCandidateJobEvent::EvidenceDone { metrics }) => {
                        evidence_done = true;
                        info!(
                            path = %library_path,
                            facet = facet.as_str(),
                            candidates = metrics.candidates_emitted,
                            skipped = metrics.skipped,
                            failed = metrics.failed,
                            inline_inventory = metrics.inline_inventory_emitted,
                            inventory_walks_queued = metrics.inventory_walks_queued,
                            inventory_walks_started = metrics.inventory_walks_started,
                            elapsed_ms = elapsed_ms_u64(started_at),
                            "library scan evidence phase completed"
                        );
                    }
                    Some(event) => {
                        if let Err(error) = handle_candidate_job_event(CandidateEventContext {
                            app,
                            facet,
                            library_id,
                            coordinator: &coordinator,
                            summary: &mut summary,
                            candidates: &mut candidates,
                            forward_queue: &mut forward_queue,
                            discovery_error: &mut discovery_error,
                        }, event).await {
                            discovery_error = Some(error);
                        }
                    }
                    None => {
                        evidence_done = true;
                    }
                }
            }
            event = inventory_events_rx.recv(), if !inventory_done => {
                match event {
                    Some(event) => {
                        handle_inventory_job_event(InventoryEventContext {
                            coordinator: &coordinator,
                            candidates: &mut candidates,
                            hydration: &mut hydration,
                            pool: &mut pool,
                            media_dedup_skips: &mut media_dedup_skips,
                            stored_inventory_paths: &mut stored_inventory_paths,
                            storage_watch: &storage_watch_tx,
                        }, event).await?;
                    }
                    None => {
                        inventory_done = true;
                    }
                }
            }
            permit = async {
                match match_input_tx.as_ref() {
                    Some(tx) => tx.reserve().await.ok(),
                    None => None,
                }
            }, if !forward_queue.is_empty() && match_input_tx.is_some() => {
                match permit {
                    Some(permit) => {
                        if let Some(entry) = forward_queue.pop_front() {
                            permit.send(entry);
                        }
                    }
                    None => {
                        // The worker is gone; drain the queue so the scan can
                        // settle instead of spinning.
                        forward_queue.clear();
                    }
                }
            }
            event = match_events_rx.recv(), if !match_done => {
                match event {
                    Some(ScanMatchWorkerEvent::Matched { key, work }) => {
                        handle_match_decision(
                            &coordinator,
                            &mut candidates,
                            &mut hydration,
                            &mut pool,
                            &mut media_dedup_skips,
                            &mut stored_inventory_paths,
                            &storage_watch_tx,
                            key,
                            Some(*work),
                        ).await?;
                    }
                    Some(ScanMatchWorkerEvent::Terminal { key }) => {
                        handle_match_decision(
                            &coordinator,
                            &mut candidates,
                            &mut hydration,
                            &mut pool,
                            &mut media_dedup_skips,
                            &mut stored_inventory_paths,
                            &storage_watch_tx,
                            key,
                            None,
                        ).await?;
                    }
                    Some(ScanMatchWorkerEvent::Done(report)) => {
                        info!(
                            path = %library_path,
                            facet = facet.as_str(),
                            scanned = report.summary.scanned,
                            matched = report.summary.matched,
                            unmatched = report.summary.unmatched,
                            skipped = report.summary.skipped,
                            metadata_lookups = report.stats.logical_lookups,
                            metadata_lookup_requests_executed = report.stats.executed_requests,
                            elapsed_ms = elapsed_ms_u64(started_at),
                            "library scan match phase completed"
                        );
                        worker_report = Some(report);
                        match_done = true;
                    }
                    None => {
                        match_done = true;
                    }
                }
            }
            hydrated = hydration.join_next(), if hydration.has_in_flight() => {
                for work in hydrated? {
                    if !pool.enqueue(work) {
                        media_dedup_skips = media_dedup_skips.saturating_add(1);
                    }
                }
                pool.pump().await?;
            }
            _ = async {
                match hydration_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            }, if hydration_deadline.is_some() => {
                hydration.flush_due();
            }
            _ = async {
                match cancel_token.as_ref() {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            }, if cancel_token.is_some() => {
                break;
            }
        }

        hydration.maybe_flush();
        pool.pump().await?;

        try_mark_file_total_known(TotalKnownLatchContext {
            coordinator: &coordinator,
            pool: &mut pool,
            candidates: &candidates,
            hydration: &hydration,
            file_total_marked: &mut file_total_marked,
            match_done,
            cancel_token: cancel_token.as_ref(),
            started_at,
            library_path,
            facet,
        })
        .await?;
    }

    drop(match_input_tx);

    if let Some(error) = discovery_error {
        // Root discovery failed: settle workers and fail the scan without a
        // success progress latch.
        candidate_events_rx.close();
        inventory_events_rx.close();
        match_events_rx.close();
        jobs_handle.abort();
        let _ = worker_handle.await;
        pool.drain_for_failure().await?;
        return Err(error);
    }

    // Drain any candidate/inventory/match events that raced with loop exit.
    while let Some(event) = candidate_events_rx.recv().await {
        match event {
            ScanCandidateJobEvent::EvidenceDone { .. } => {}
            event => {
                if let Err(error) = handle_candidate_job_event(
                    CandidateEventContext {
                        app,
                        facet,
                        library_id,
                        coordinator: &coordinator,
                        summary: &mut summary,
                        candidates: &mut candidates,
                        forward_queue: &mut forward_queue,
                        discovery_error: &mut discovery_error,
                    },
                    event,
                )
                .await
                {
                    discovery_error = Some(error);
                }
            }
        }
    }
    while let Some(event) = inventory_events_rx.recv().await {
        handle_inventory_job_event(
            InventoryEventContext {
                coordinator: &coordinator,
                candidates: &mut candidates,
                hydration: &mut hydration,
                pool: &mut pool,
                media_dedup_skips: &mut media_dedup_skips,
                stored_inventory_paths: &mut stored_inventory_paths,
                storage_watch: &storage_watch_tx,
            },
            event,
        )
        .await?;
    }

    if let Some(error) = discovery_error {
        match_events_rx.close();
        let _ = worker_handle.await;
        pool.drain_for_failure().await?;
        return Err(error);
    }

    // The worker's final report may not have been consumed if the loop broke
    // on cancellation; drain match events so the summary is not lost.
    while !match_done {
        match match_events_rx.recv().await {
            Some(ScanMatchWorkerEvent::Matched { key, work }) => {
                handle_match_decision(
                    &coordinator,
                    &mut candidates,
                    &mut hydration,
                    &mut pool,
                    &mut media_dedup_skips,
                    &mut stored_inventory_paths,
                    &storage_watch_tx,
                    key,
                    Some(*work),
                )
                .await?;
            }
            Some(ScanMatchWorkerEvent::Terminal { key }) => {
                handle_match_decision(
                    &coordinator,
                    &mut candidates,
                    &mut hydration,
                    &mut pool,
                    &mut media_dedup_skips,
                    &mut stored_inventory_paths,
                    &storage_watch_tx,
                    key,
                    None,
                )
                .await?;
            }
            Some(ScanMatchWorkerEvent::Done(report)) => {
                info!(
                    path = %library_path,
                    facet = facet.as_str(),
                    scanned = report.summary.scanned,
                    matched = report.summary.matched,
                    unmatched = report.summary.unmatched,
                    skipped = report.summary.skipped,
                    metadata_lookups = report.stats.logical_lookups,
                    metadata_lookup_requests_executed = report.stats.executed_requests,
                    elapsed_ms = elapsed_ms_u64(started_at),
                    "library scan match phase completed"
                );
                worker_report = Some(report);
                match_done = true;
            }
            None => break,
        }
    }

    let canceled = library_scan_cancel_requested(cancel_token.as_ref());
    if canceled {
        hydration.abort();
    } else {
        hydration.flush_due();
        for work in hydration.drain().await? {
            pool.enqueue(work);
        }
        try_mark_file_total_known(TotalKnownLatchContext {
            coordinator: &coordinator,
            pool: &mut pool,
            candidates: &candidates,
            hydration: &hydration,
            file_total_marked: &mut file_total_marked,
            match_done,
            cancel_token: cancel_token.as_ref(),
            started_at,
            library_path,
            facet,
        })
        .await?;
    }

    if let Some(report) = worker_report.take() {
        summary.absorb(&report.summary);
        if media_dedup_skips > 0 {
            summary.matched = summary.matched.saturating_sub(media_dedup_skips);
            summary.skipped = summary.skipped.saturating_add(media_dedup_skips);
        }
        try_mark_file_total_known(TotalKnownLatchContext {
            coordinator: &coordinator,
            pool: &mut pool,
            candidates: &candidates,
            hydration: &hydration,
            file_total_marked: &mut file_total_marked,
            match_done,
            cancel_token: cancel_token.as_ref(),
            started_at,
            library_path,
            facet,
        })
        .await?;

        pool.close_input();
        summary.absorb(&pool.finish().await?);
        info!(
            path = %library_path,
            facet = facet.as_str(),
            imported = summary.imported,
            skipped = summary.skipped,
            elapsed_ms = elapsed_ms_u64(started_at),
            "library scan analysis phase completed"
        );

        if !canceled {
            let mut seen_paths = report.seen_paths;
            for runtime in candidates.values() {
                let trimmed = runtime.item_path.trim();
                if !trimmed.is_empty() {
                    seen_paths.insert(normalize_library_scan_item_path(trimmed));
                }
            }
            reconcile_library_scan_unmatched_items(app, facet, library_path, &seen_paths).await?;
            coordinator.publish_progress().await;
        }

        info!(
            path = %library_path,
            facet = facet.as_str(),
            scanned = summary.scanned,
            matched = summary.matched,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            metadata_lookups = report.stats.logical_lookups,
            metadata_lookup_requests_executed = report.stats.executed_requests,
            metadata_lookup_requests_coalesced = report.stats.coalesced_requests,
            match_batch_size = LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE,
            match_in_flight_batches = LIBRARY_SCAN_METADATA_IN_FLIGHT_BATCHES,
            elapsed_ms = elapsed_ms_u64(started_at),
            "{} library scan completed",
            facet.as_str()
        );

        if !report.unmatched_items.is_empty() {
            info!(
                count = report.unmatched_items.len(),
                facet = facet.as_str(),
                "{} library scan unmatched items follow",
                facet.as_str()
            );
            for unmatched in &report.unmatched_items {
                info!(
                    path = %unmatched.item_path,
                    display_name = %unmatched.display_name,
                    query = %unmatched.query,
                    year_hint = ?unmatched.year_hint,
                    reason = %unmatched.reason_code,
                    error_message = ?unmatched.error_message,
                    search_attempts = %format_library_scan_unmatched_search_attempts(&unmatched.search_attempts),
                    "{} library scan unmatched item",
                    facet.as_str()
                );
            }
        }
    } else {
        // Worker never reported (cancellation before drain); settle the pool.
        pool.close_input();
        summary.absorb(&pool.finish().await?);
        let _ = worker_handle.await;
    }

    Ok(summary)
}

fn matched_inventory_totals_ready(
    candidates: &HashMap<ScanCandidateKey, CandidateRuntime>,
) -> bool {
    candidates
        .values()
        .all(|runtime| match runtime.match_state {
            CandidateMatchState::Pending | CandidateMatchState::MatchedAwaitingInventory(_) => {
                false
            }
            CandidateMatchState::Dispatched | CandidateMatchState::Terminal => {
                runtime.scoped || runtime.inventory_terminal()
            }
        })
}

struct CandidateEventContext<'a> {
    app: &'a AppUseCase,
    facet: &'a MediaFacet,
    library_id: &'a str,
    coordinator: &'a LibraryScanCoordinator,
    summary: &'a mut LibraryScanSummary,
    candidates: &'a mut HashMap<ScanCandidateKey, CandidateRuntime>,
    forward_queue: &'a mut VecDeque<(ScanCandidateKey, ScanPipelineCandidate)>,
    discovery_error: &'a mut Option<AppError>,
}

async fn handle_candidate_job_event(
    ctx: CandidateEventContext<'_>,
    event: ScanCandidateJobEvent,
) -> AppResult<()> {
    match event {
        ScanCandidateJobEvent::Candidate {
            key,
            candidate,
            scoped,
            inventory_cancel,
        } => {
            let item_path = match &candidate {
                ScanPipelineCandidate::Movie(movie) => {
                    normalize_library_scan_item_path(&movie.file.path)
                }
                ScanPipelineCandidate::Series(series) => series.item_path().trim().to_string(),
            };
            ctx.candidates.insert(
                key,
                CandidateRuntime {
                    item_path,
                    scoped,
                    match_state: CandidateMatchState::Pending,
                    inventory: CandidateInventoryState::Pending,
                    inventory_cancel,
                },
            );
            ctx.forward_queue.push_back((key, candidate));
        }
        ScanCandidateJobEvent::Skipped { item_path } => {
            ctx.summary.scanned += 1;
            ctx.summary.skipped += 1;
            clear_library_scan_unmatched_item(ctx.app, ctx.facet, ctx.library_id, &item_path)
                .await?;
            ctx.coordinator.mark_title_match_completed(1).await;
            ctx.coordinator.publish_progress().await;
        }
        ScanCandidateJobEvent::EvidenceFailed { item_path, error } => {
            warn!(
                item_path = %item_path,
                error = %error,
                "library scan candidate evidence failed"
            );
            ctx.summary.scanned += 1;
            ctx.summary.unmatched += 1;
            ctx.coordinator.mark_title_match_completed(1).await;
            ctx.coordinator.publish_progress().await;
        }
        ScanCandidateJobEvent::EvidenceDone { .. } => {
            // The coordinator consumes this lifecycle event directly.
        }
        ScanCandidateJobEvent::DiscoveryFailed { error } => {
            *ctx.discovery_error = Some(error);
        }
    }
    Ok(())
}

struct InventoryEventContext<'a> {
    coordinator: &'a LibraryScanCoordinator,
    candidates: &'a mut HashMap<ScanCandidateKey, CandidateRuntime>,
    hydration: &'a mut ScanHydrationBatcher,
    pool: &'a mut LibraryScanMediaAnalysisPool,
    media_dedup_skips: &'a mut usize,
    stored_inventory_paths: &'a mut usize,
    storage_watch: &'a tokio::sync::watch::Sender<usize>,
}

async fn handle_inventory_job_event(
    ctx: InventoryEventContext<'_>,
    event: ScanInventoryJobEvent,
) -> AppResult<()> {
    match event {
        ScanInventoryJobEvent::Inventory { key, files } => {
            handle_inventory_ready(
                ctx.coordinator,
                ctx.candidates,
                ctx.hydration,
                ctx.pool,
                ctx.media_dedup_skips,
                ctx.stored_inventory_paths,
                ctx.storage_watch,
                key,
                files,
            )
            .await?;
        }
        ScanInventoryJobEvent::InventoryFailed {
            key,
            item_path,
            error,
        } => {
            warn!(
                item_path = %item_path,
                error = %error,
                "library scan candidate inventory failed"
            );
            if let Some(runtime) = ctx.candidates.get_mut(&key) {
                runtime.inventory = CandidateInventoryState::Failed;
                if let CandidateMatchState::MatchedAwaitingInventory(_) =
                    std::mem::replace(&mut runtime.match_state, CandidateMatchState::Terminal)
                {
                    // Matched but inventory failed: no media analysis for it.
                }
            }
        }
        ScanInventoryJobEvent::InventoryCanceled { key } => {
            if let Some(runtime) = ctx.candidates.get_mut(&key) {
                runtime.inventory = CandidateInventoryState::Canceled;
            }
        }
    }
    Ok(())
}

struct TotalKnownLatchContext<'a> {
    coordinator: &'a LibraryScanCoordinator,
    pool: &'a mut LibraryScanMediaAnalysisPool,
    candidates: &'a HashMap<ScanCandidateKey, CandidateRuntime>,
    hydration: &'a ScanHydrationBatcher,
    file_total_marked: &'a mut bool,
    match_done: bool,
    cancel_token: Option<&'a CancellationToken>,
    started_at: Instant,
    library_path: &'a str,
    facet: &'a MediaFacet,
}

async fn try_mark_file_total_known(ctx: TotalKnownLatchContext<'_>) -> AppResult<()> {
    if *ctx.file_total_marked
        || !ctx.match_done
        || library_scan_cancel_requested(ctx.cancel_token)
        || !ctx.hydration.is_idle()
        || !matched_inventory_totals_ready(ctx.candidates)
    {
        return Ok(());
    }

    // Promotion publishes totals for queued pre-counted work. The progress
    // tracker intentionally ignores later file_total_delta values once the
    // known latch is set, so this must happen before mark_file_total_known.
    ctx.pool.pump().await?;
    if !matched_inventory_totals_ready(ctx.candidates) {
        return Ok(());
    }

    ctx.coordinator.mark_file_total_known().await;
    ctx.coordinator.publish_progress().await;
    *ctx.file_total_marked = true;
    info!(
        path = %ctx.library_path,
        facet = ctx.facet.as_str(),
        elapsed_ms = elapsed_ms_u64(ctx.started_at),
        "library scan inventory totals ready"
    );
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "rendezvous updates shared pipeline state for one candidate in one place"
)]
async fn handle_inventory_ready(
    coordinator: &LibraryScanCoordinator,
    candidates: &mut HashMap<ScanCandidateKey, CandidateRuntime>,
    hydration: &mut ScanHydrationBatcher,
    pool: &mut LibraryScanMediaAnalysisPool,
    media_dedup_skips: &mut usize,
    stored_inventory_paths: &mut usize,
    storage_watch: &tokio::sync::watch::Sender<usize>,
    key: ScanCandidateKey,
    files: Vec<LibraryFile>,
) -> AppResult<()> {
    let Some(runtime) = candidates.get_mut(&key) else {
        return Ok(());
    };

    match &mut runtime.match_state {
        CandidateMatchState::MatchedAwaitingInventory(_) => {
            let CandidateMatchState::MatchedAwaitingInventory(work) =
                std::mem::replace(&mut runtime.match_state, CandidateMatchState::Dispatched)
            else {
                unreachable!("checked variant above");
            };
            runtime.inventory = CandidateInventoryState::Consumed;
            dispatch_media_work(
                coordinator,
                hydration,
                pool,
                media_dedup_skips,
                *work,
                files,
            )
            .await?;
        }
        CandidateMatchState::Pending => {
            *stored_inventory_paths = stored_inventory_paths.saturating_add(files.len());
            let _ = storage_watch.send(*stored_inventory_paths);
            runtime.inventory = CandidateInventoryState::Ready(files);
        }
        CandidateMatchState::Dispatched | CandidateMatchState::Terminal => {
            // Unmatched/duplicate inventory: discard immediately.
            runtime.inventory = CandidateInventoryState::Consumed;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "match decisions update shared pipeline state for one candidate in one place"
)]
async fn handle_match_decision(
    coordinator: &LibraryScanCoordinator,
    candidates: &mut HashMap<ScanCandidateKey, CandidateRuntime>,
    hydration: &mut ScanHydrationBatcher,
    pool: &mut LibraryScanMediaAnalysisPool,
    media_dedup_skips: &mut usize,
    stored_inventory_paths: &mut usize,
    storage_watch: &tokio::sync::watch::Sender<usize>,
    key: ScanCandidateKey,
    matched_work: Option<LibraryScanTitleWork>,
) -> AppResult<()> {
    let Some(runtime) = candidates.get_mut(&key) else {
        return Ok(());
    };

    match matched_work {
        Some(work) if runtime.scoped => {
            // Scoped work already carries its own file list.
            runtime.match_state = CandidateMatchState::Dispatched;
            let files = work.discovered_files.clone().unwrap_or_default();
            dispatch_media_work(coordinator, hydration, pool, media_dedup_skips, work, files)
                .await?;
        }
        Some(work) => {
            match std::mem::replace(&mut runtime.inventory, CandidateInventoryState::Consumed) {
                CandidateInventoryState::Ready(files) => {
                    *stored_inventory_paths = stored_inventory_paths.saturating_sub(files.len());
                    let _ = storage_watch.send(*stored_inventory_paths);
                    runtime.match_state = CandidateMatchState::Dispatched;
                    dispatch_media_work(
                        coordinator,
                        hydration,
                        pool,
                        media_dedup_skips,
                        work,
                        files,
                    )
                    .await?;
                }
                CandidateInventoryState::Pending => {
                    runtime.inventory = CandidateInventoryState::Pending;
                    runtime.match_state =
                        CandidateMatchState::MatchedAwaitingInventory(Box::new(work));
                }
                CandidateInventoryState::Failed => {
                    runtime.inventory = CandidateInventoryState::Failed;
                    runtime.match_state = CandidateMatchState::Terminal;
                    warn!(
                        item_path = %runtime.item_path,
                        "matched candidate has failed media inventory; skipping analysis"
                    );
                }
                CandidateInventoryState::Canceled | CandidateInventoryState::Consumed => {
                    runtime.match_state = CandidateMatchState::Terminal;
                }
            }
        }
        None => {
            // Unmatched/failed/skipped: cancel any in-flight inventory walk
            // and discard stored file lists at decision time.
            runtime.inventory_cancel.cancel();
            if let CandidateInventoryState::Ready(files) =
                std::mem::replace(&mut runtime.inventory, CandidateInventoryState::Consumed)
            {
                *stored_inventory_paths = stored_inventory_paths.saturating_sub(files.len());
                let _ = storage_watch.send(*stored_inventory_paths);
            } else if matches!(runtime.inventory, CandidateInventoryState::Consumed) {
                // preserved state
            }
            runtime.match_state = CandidateMatchState::Terminal;
        }
    }
    Ok(())
}

async fn dispatch_media_work(
    coordinator: &LibraryScanCoordinator,
    hydration: &mut ScanHydrationBatcher,
    pool: &mut LibraryScanMediaAnalysisPool,
    media_dedup_skips: &mut usize,
    mut work: LibraryScanTitleWork,
    files: Vec<LibraryFile>,
) -> AppResult<()> {
    if work
        .discovered_files
        .as_ref()
        .is_none_or(|existing| existing.is_empty())
    {
        work.full_folder = true;
        work.discovered_files = Some(files);
    }

    if hydration.submit(work).await? {
        return Ok(());
    }

    // No hydration needed: enqueue straight into the analysis pool.
    let work = hydration.take_passthrough();
    if let Some(work) = work {
        if !pool.enqueue(work) {
            *media_dedup_skips = media_dedup_skips.saturating_add(1);
        }
        pool.pump().await?;
        coordinator.publish_progress().await;
    }
    Ok(())
}

/// Batches titles that need SMG hydration before media analysis so hydration
/// stays in bulk requests and off the candidate-to-match critical path.
struct ScanHydrationBatcher {
    app: AppUseCase,
    cancel_token: Option<CancellationToken>,
    pending: Vec<LibraryScanTitleWork>,
    passthrough: Option<LibraryScanTitleWork>,
    first_pending_at: Option<Instant>,
    flush_requested: bool,
    in_flight: tokio::task::JoinSet<AppResult<Vec<LibraryScanTitleWork>>>,
}

impl ScanHydrationBatcher {
    fn new(app: AppUseCase, cancel_token: Option<CancellationToken>) -> Self {
        Self {
            app,
            cancel_token,
            pending: Vec::new(),
            passthrough: None,
            first_pending_at: None,
            flush_requested: false,
            in_flight: tokio::task::JoinSet::new(),
        }
    }

    /// Returns true when the work was queued for hydration; false when the
    /// work does not need hydration (retrieve it with `take_passthrough`).
    async fn submit(&mut self, work: LibraryScanTitleWork) -> AppResult<bool> {
        let metadata_language = self.app.metadata_language().await;
        if title_requires_scan_hydration(&self.app, &work.title, &metadata_language).await? {
            if self.pending.is_empty() {
                self.first_pending_at = Some(Instant::now());
            }
            self.pending.push(work);
            if self.pending.len() >= crate::catalog_workflow::HYDRATION_BULK_BATCH_SIZE {
                self.flush_requested = true;
            }
            Ok(true)
        } else {
            self.passthrough = Some(work);
            Ok(false)
        }
    }

    fn take_passthrough(&mut self) -> Option<LibraryScanTitleWork> {
        self.passthrough.take()
    }

    fn deadline_instant(&self) -> Option<tokio::time::Instant> {
        if self.pending.is_empty()
            || self.in_flight.len() >= LIBRARY_SCAN_HYDRATION_IN_FLIGHT_BATCHES
        {
            // A full in-flight window wakes via join_next; arming the timer
            // too would spin on an already-expired deadline.
            return None;
        }
        self.first_pending_at
            .map(|first| tokio::time::Instant::from_std(first) + LIBRARY_SCAN_MATCH_FLUSH_INTERVAL)
    }

    fn flush_due(&mut self) {
        self.flush_requested = true;
    }

    fn maybe_flush(&mut self) {
        if !self.flush_requested
            || self.pending.is_empty()
            || self.in_flight.len() >= LIBRARY_SCAN_HYDRATION_IN_FLIGHT_BATCHES
        {
            return;
        }
        self.flush_requested = false;
        self.first_pending_at = None;

        let batch = std::mem::take(&mut self.pending);
        let app = self.app.clone();
        let cancel_token = self.cancel_token.clone();
        self.in_flight.spawn(async move {
            hydrate_library_scan_title_works(&app, batch, cancel_token.as_ref()).await
        });
    }

    fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }

    async fn join_next(&mut self) -> AppResult<Vec<LibraryScanTitleWork>> {
        match self.in_flight.join_next().await {
            Some(Ok(result)) => result,
            Some(Err(error)) if error.is_cancelled() => Ok(Vec::new()),
            Some(Err(error)) => Err(AppError::Repository(error.to_string())),
            None => Ok(Vec::new()),
        }
    }

    fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.in_flight.is_empty() && self.passthrough.is_none()
    }

    fn abort(&mut self) {
        self.pending.clear();
        self.in_flight.abort_all();
    }

    async fn drain(&mut self) -> AppResult<Vec<LibraryScanTitleWork>> {
        self.flush_requested = true;
        self.maybe_flush();
        let mut works = Vec::new();
        while !self.in_flight.is_empty() || !self.pending.is_empty() {
            works.extend(self.join_next().await?);
            self.flush_requested = true;
            self.maybe_flush();
            if self.in_flight.is_empty() && self.pending.is_empty() {
                break;
            }
        }
        Ok(works)
    }
}

async fn hydrate_library_scan_title_works(
    app: &AppUseCase,
    works: Vec<LibraryScanTitleWork>,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<Vec<LibraryScanTitleWork>> {
    let session = None::<()>;
    let _ = session;
    let targets = works
        .iter()
        .map(|work| crate::catalog_workflow::HydrationTarget {
            title: work.title.clone(),
            requested_tvdb_id: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::LibraryScanFull,
        })
        .collect::<Vec<_>>();

    let outcome = app
        .hydrate_titles_bulk_cancellable(targets, cancel_token)
        .await?;

    let mut hydrated_by_id: HashMap<String, Title> = outcome.hydrated_titles.into_iter().collect();
    let failed: HashMap<String, String> = outcome.failed_titles.into_iter().collect();

    let mut ready = Vec::with_capacity(works.len());
    for mut work in works {
        if let Some(reason) = failed.get(&work.title.id) {
            warn!(
                title_id = %work.title.id,
                reason = %reason,
                "library scan title hydration failed"
            );
            continue;
        }
        if let Some(hydrated) = hydrated_by_id.remove(&work.title.id) {
            work.title = hydrated;
        }
        ready.push(work);
    }
    Ok(ready)
}

struct CandidateJobContext {
    app: AppUseCase,
    session_id: String,
    library_path: String,
    kind: LibraryScanPipelineKind,
    scan_hints: Option<LibraryScanHintSet>,
    mark_discovery_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
    candidate_events: tokio::sync::mpsc::Sender<ScanCandidateJobEvent>,
    inventory_events: tokio::sync::mpsc::Sender<ScanInventoryJobEvent>,
    storage_watch: tokio::sync::watch::Receiver<usize>,
}

fn spawn_candidate_jobs(ctx: CandidateJobContext) -> AppResult<tokio::task::JoinHandle<()>> {
    Ok(tokio::spawn(async move {
        let result = match ctx.kind {
            LibraryScanPipelineKind::Movie => run_movie_candidate_jobs(&ctx).await,
            LibraryScanPipelineKind::Series => run_series_candidate_jobs(&ctx).await,
        };
        if let Err(error) = result {
            let _ = ctx
                .candidate_events
                .send(ScanCandidateJobEvent::DiscoveryFailed { error })
                .await;
        }
    }))
}

enum EvidenceJobOutput {
    Candidate {
        key: ScanCandidateKey,
        candidate: ScanPipelineCandidate,
        scoped: bool,
        inline_inventory: Option<Vec<LibraryFile>>,
        inventory_target: Option<PathBuf>,
    },
    Skipped {
        item_path: String,
    },
    Failed {
        item_path: String,
        error: AppError,
    },
}

struct CandidateJobRunner<'a> {
    ctx: &'a CandidateJobContext,
    next_key: ScanCandidateKey,
    evidence_set: tokio::task::JoinSet<EvidenceJobOutput>,
    inventory_set: tokio::task::JoinSet<()>,
    inline_inventory_queue: VecDeque<(ScanCandidateKey, Vec<LibraryFile>)>,
    inventory_queue: VecDeque<(ScanCandidateKey, PathBuf, CancellationToken)>,
    cancel_tokens: HashMap<ScanCandidateKey, CancellationToken>,
    metrics: CandidateJobMetrics,
}

impl<'a> CandidateJobRunner<'a> {
    fn new(ctx: &'a CandidateJobContext) -> Self {
        Self {
            ctx,
            next_key: 0,
            evidence_set: tokio::task::JoinSet::new(),
            inventory_set: tokio::task::JoinSet::new(),
            inline_inventory_queue: VecDeque::new(),
            inventory_queue: VecDeque::new(),
            cancel_tokens: HashMap::new(),
            metrics: CandidateJobMetrics::default(),
        }
    }

    fn allocate_key(&mut self) -> ScanCandidateKey {
        let key = self.next_key;
        self.next_key = self.next_key.saturating_add(1);
        key
    }

    async fn forward_evidence_output(&mut self, output: EvidenceJobOutput) -> bool {
        match output {
            EvidenceJobOutput::Candidate {
                key,
                candidate,
                scoped,
                inline_inventory,
                inventory_target,
            } => {
                let inventory_cancel = self.cancel_tokens.entry(key).or_default().clone();
                if self
                    .ctx
                    .candidate_events
                    .send(ScanCandidateJobEvent::Candidate {
                        key,
                        candidate,
                        scoped,
                        inventory_cancel: inventory_cancel.clone(),
                    })
                    .await
                    .is_err()
                {
                    return false;
                }
                self.metrics.candidates_emitted = self.metrics.candidates_emitted.saturating_add(1);
                if let Some(files) = inline_inventory {
                    self.metrics.inline_inventory_emitted =
                        self.metrics.inline_inventory_emitted.saturating_add(1);
                    if !scoped && !self.try_forward_inline_inventory(key, files) {
                        return false;
                    }
                } else if let Some(target) = inventory_target {
                    self.metrics.inventory_walks_queued =
                        self.metrics.inventory_walks_queued.saturating_add(1);
                    self.inventory_queue
                        .push_back((key, target, inventory_cancel));
                }
                true
            }
            EvidenceJobOutput::Skipped { item_path } => {
                self.metrics.skipped = self.metrics.skipped.saturating_add(1);
                self.ctx
                    .candidate_events
                    .send(ScanCandidateJobEvent::Skipped { item_path })
                    .await
                    .is_ok()
            }
            EvidenceJobOutput::Failed { item_path, error } => {
                self.metrics.failed = self.metrics.failed.saturating_add(1);
                self.ctx
                    .candidate_events
                    .send(ScanCandidateJobEvent::EvidenceFailed { item_path, error })
                    .await
                    .is_ok()
            }
        }
    }

    fn try_forward_inline_inventory(
        &mut self,
        key: ScanCandidateKey,
        files: Vec<LibraryFile>,
    ) -> bool {
        match self
            .ctx
            .inventory_events
            .try_send(ScanInventoryJobEvent::Inventory { key, files })
        {
            Ok(()) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(
                ScanInventoryJobEvent::Inventory { key, files },
            )) => {
                self.inline_inventory_queue.push_back((key, files));
                true
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => true,
        }
    }

    async fn flush_inline_inventory(&mut self, block: bool) -> bool {
        while let Some((key, files)) = self.inline_inventory_queue.pop_front() {
            let event = ScanInventoryJobEvent::Inventory { key, files };
            if block {
                if self.ctx.inventory_events.send(event).await.is_err() {
                    return false;
                }
                continue;
            }

            match self.ctx.inventory_events.try_send(event) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(
                    ScanInventoryJobEvent::Inventory { key, files },
                )) => {
                    self.inline_inventory_queue.push_front((key, files));
                    return true;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return false,
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => return true,
            }
        }
        true
    }

    /// Launch queued inventory walks. In the evidence loop this is
    /// non-blocking (`block = false`): if no walk permit is free or the
    /// rendezvous high-water gate is engaged, queued inventory simply waits —
    /// evidence emission is never paused for inventory. `settle` passes
    /// `block = true` once no evidence work remains.
    async fn launch_pending_inventory(&mut self, block: bool) {
        while !self.inventory_queue.is_empty() {
            if library_scan_cancel_requested(self.ctx.cancel_token.as_ref()) {
                self.inventory_queue.clear();
                return;
            }

            // Rendezvous storage high-water gate: pause the inventory phase
            // (never evidence) until stored file lists drain.
            let mut storage = self.ctx.storage_watch.clone();
            if !block && *storage.borrow() >= LIBRARY_SCAN_MEDIA_INVENTORY_PATH_HIGH_WATER {
                return;
            }
            while *storage.borrow() >= LIBRARY_SCAN_MEDIA_INVENTORY_PATH_HIGH_WATER {
                if storage.changed().await.is_err() {
                    return;
                }
            }

            let semaphore = self
                .ctx
                .app
                .runtime
                .library
                .library_scan_title_walk_limit
                .clone();
            let permit = if block {
                tokio::select! {
                    permit = semaphore.acquire_owned() => match permit {
                        Ok(permit) => permit,
                        Err(_) => return,
                    },
                    _ = async {
                        match self.ctx.cancel_token.as_ref() {
                            Some(token) => token.cancelled().await,
                            None => std::future::pending::<()>().await,
                        }
                    }, if self.ctx.cancel_token.is_some() => {
                        self.inventory_queue.clear();
                        return;
                    }
                }
            } else {
                match semaphore.try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => return,
                }
            };
            let Some((key, target, cancel)) = self.inventory_queue.pop_front() else {
                return;
            };
            self.metrics.inventory_walks_started =
                self.metrics.inventory_walks_started.saturating_add(1);
            let events = self.ctx.inventory_events.clone();
            let app = self.ctx.app.clone();
            let kind = self.ctx.kind;
            let scan_cancel = self.ctx.cancel_token.clone();
            self.inventory_set.spawn(async move {
                let _permit = permit;
                if cancel.is_cancelled() || library_scan_cancel_requested(scan_cancel.as_ref()) {
                    let _ = events
                        .send(ScanInventoryJobEvent::InventoryCanceled { key })
                        .await;
                    return;
                }

                let target_str = path_to_stored_string(&target);
                let result = match kind {
                    LibraryScanPipelineKind::Movie => {
                        app.services
                            .library
                            .library_scanner
                            .scan_directory(target_str.as_str())
                            .await
                    }
                    LibraryScanPipelineKind::Series => app
                        .services
                        .library
                        .library_scanner
                        .scan_directory_for_progress_with_metrics(target_str.as_str())
                        .await
                        .map(|scan| scan.files),
                };

                let event = match result {
                    Ok(_) if cancel.is_cancelled() => {
                        ScanInventoryJobEvent::InventoryCanceled { key }
                    }
                    Ok(mut files) => {
                        files.sort_by(|left, right| left.path.cmp(&right.path));
                        ScanInventoryJobEvent::Inventory { key, files }
                    }
                    Err(error) => ScanInventoryJobEvent::InventoryFailed {
                        key,
                        item_path: target_str,
                        error,
                    },
                };
                let _ = events.send(event).await;
            });
        }
    }

    async fn drain_evidence(&mut self) -> bool {
        while !self.evidence_set.is_empty() {
            if library_scan_cancel_requested(self.ctx.cancel_token.as_ref()) {
                self.evidence_set.abort_all();
                return false;
            }
            self.launch_pending_inventory(false).await;
            if !self.flush_inline_inventory(false).await {
                return false;
            }
            match self.evidence_set.join_next().await {
                Some(Ok(output)) => {
                    if !self.forward_evidence_output(output).await {
                        return false;
                    }
                }
                Some(Err(error)) if error.is_cancelled() => {}
                Some(Err(error)) => {
                    self.metrics.failed = self.metrics.failed.saturating_add(1);
                    warn!(error = %error, "library scan evidence task failed");
                }
                None => break,
            }
        }
        true
    }

    async fn send_evidence_done(&self) -> bool {
        self.ctx
            .candidate_events
            .send(ScanCandidateJobEvent::EvidenceDone {
                metrics: self.metrics,
            })
            .await
            .is_ok()
    }

    async fn settle(mut self) {
        while !library_scan_cancel_requested(self.ctx.cancel_token.as_ref()) {
            if !self.flush_inline_inventory(true).await {
                break;
            }
            self.launch_pending_inventory(true).await;
            if self.evidence_set.is_empty()
                && self.inventory_set.is_empty()
                && self.inline_inventory_queue.is_empty()
                && self.inventory_queue.is_empty()
            {
                break;
            }

            let cancelled = async {
                match self.ctx.cancel_token.as_ref() {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = cancelled, if self.ctx.cancel_token.is_some() => {
                    break;
                }
                output = self.evidence_set.join_next(), if !self.evidence_set.is_empty() => {
                    if let Some(Ok(output)) = output
                        && !self.forward_evidence_output(output).await
                    {
                        break;
                    }
                }
                _ = self.inventory_set.join_next(), if !self.inventory_set.is_empty() => {}
            }
        }

        // Abort-and-join both pools; a no-op when they drained normally.
        // Joining (not just aborting) is required so cancellation cannot
        // leave tasks parked in the sets.
        self.inline_inventory_queue.clear();
        self.inventory_queue.clear();
        self.evidence_set.abort_all();
        while self.evidence_set.join_next().await.is_some() {}
        self.inventory_set.abort_all();
        while self.inventory_set.join_next().await.is_some() {}
    }
}

async fn run_movie_candidate_jobs(ctx: &CandidateJobContext) -> AppResult<()> {
    let root = require_directory_library_path(&ctx.library_path)?.to_path_buf();
    let discovered_entries =
        stream_movie_top_level_entries_batched(&root, LIBRARY_SCAN_MOVIE_BATCH_SIZE).await?;
    let mut queued_entries = spawn_library_discovery_queue(
        ctx.app.clone(),
        ctx.session_id.clone(),
        discovered_entries,
        false,
        ctx.mark_discovery_complete_on_drain,
        ctx.cancel_token.clone(),
    );

    let mut runner = CandidateJobRunner::new(ctx);
    let mut pending_entries: VecDeque<MovieTopLevelEntry> = VecDeque::new();
    let mut discovery_closed = false;

    loop {
        if library_scan_cancel_requested(ctx.cancel_token.as_ref()) {
            pending_entries.clear();
            runner.evidence_set.abort_all();
            runner.inline_inventory_queue.clear();
            runner.inventory_queue.clear();
            runner.inventory_set.abort_all();
            break;
        }

        while runner.evidence_set.len() < LIBRARY_SCAN_EVIDENCE_CONCURRENCY {
            let Some(entry) = pending_entries.pop_front() else {
                break;
            };
            let key = runner.allocate_key();
            let scanner = ctx.app.services.library.library_scanner.clone();
            let library_path = ctx.library_path.clone();
            let scan_hints = ctx.scan_hints.clone();
            runner.evidence_set.spawn(async move {
                movie_evidence_job(scanner, entry, library_path, scan_hints, key).await
            });
        }

        runner.launch_pending_inventory(false).await;
        if !runner.flush_inline_inventory(false).await {
            return Ok(());
        }

        if discovery_closed && pending_entries.is_empty() && runner.evidence_set.is_empty() {
            break;
        }

        tokio::select! {
            maybe_batch = queued_entries.recv(), if !discovery_closed => {
                match maybe_batch {
                    Some(Ok(batch)) => pending_entries.extend(batch),
                    Some(Err(error)) => return Err(error),
                    None => discovery_closed = true,
                }
            }
            Some(output) = runner.evidence_set.join_next(), if !runner.evidence_set.is_empty() => {
                if let Ok(output) = output
                    && !runner.forward_evidence_output(output).await
                {
                    return Ok(());
                }
            }
        }
    }

    if !runner.drain_evidence().await || !runner.send_evidence_done().await {
        return Ok(());
    }
    if !runner.drain_evidence().await || !runner.send_evidence_done().await {
        return Ok(());
    }
    runner.settle().await;
    Ok(())
}

async fn movie_evidence_job(
    scanner: Arc<dyn LibraryScanner>,
    entry: MovieTopLevelEntry,
    library_path: String,
    scan_hints: Option<LibraryScanHintSet>,
    key: ScanCandidateKey,
) -> EvidenceJobOutput {
    let item_path = path_to_stored_string(&entry.path);
    let is_dir = entry.is_dir;
    let entry_path = entry.path.clone();
    match prepare_movie_candidate_evidence(scanner, entry, library_path, scan_hints.as_ref()).await
    {
        Ok(MovieCandidateEvidence::Candidate {
            candidate,
            inline_inventory,
        }) => EvidenceJobOutput::Candidate {
            key,
            candidate: ScanPipelineCandidate::Movie(candidate),
            scoped: false,
            inventory_target: (inline_inventory.is_none() && is_dir).then_some(entry_path),
            inline_inventory,
        },
        Ok(MovieCandidateEvidence::Skipped { item_path }) => {
            EvidenceJobOutput::Skipped { item_path }
        }
        Err(error) => EvidenceJobOutput::Failed { item_path, error },
    }
}

async fn run_series_candidate_jobs(ctx: &CandidateJobContext) -> AppResult<()> {
    let root = require_directory_library_path(&ctx.library_path)?.to_path_buf();
    let discovered_folders =
        stream_child_directories_batched(&root, LIBRARY_SCAN_SERIES_BATCH_SIZE).await?;
    let mut queued_folders = spawn_library_discovery_queue(
        ctx.app.clone(),
        ctx.session_id.clone(),
        discovered_folders,
        false,
        false,
        ctx.cancel_token.clone(),
    );

    let coordinator = LibraryScanCoordinator::new(ctx.app.clone(), ctx.session_id.clone());
    let mut runner = CandidateJobRunner::new(ctx);
    let mut pending_folders: VecDeque<PathBuf> = VecDeque::new();
    let mut discovery_closed = false;

    loop {
        if library_scan_cancel_requested(ctx.cancel_token.as_ref()) {
            pending_folders.clear();
            runner.evidence_set.abort_all();
            runner.inline_inventory_queue.clear();
            runner.inventory_queue.clear();
            runner.inventory_set.abort_all();
            break;
        }

        while runner.evidence_set.len() < LIBRARY_SCAN_EVIDENCE_CONCURRENCY {
            let Some(folder) = pending_folders.pop_front() else {
                break;
            };
            let key = runner.allocate_key();
            let scan_hints = ctx.scan_hints.clone();
            runner
                .evidence_set
                .spawn(async move { series_evidence_job(folder, scan_hints, key).await });
        }

        runner.launch_pending_inventory(false).await;
        if !runner.flush_inline_inventory(false).await {
            return Ok(());
        }

        if discovery_closed && pending_folders.is_empty() && runner.evidence_set.is_empty() {
            break;
        }

        tokio::select! {
            maybe_batch = queued_folders.recv(), if !discovery_closed => {
                match maybe_batch {
                    Some(Ok(batch)) => pending_folders.extend(batch),
                    Some(Err(error)) => return Err(error),
                    None => discovery_closed = true,
                }
            }
            Some(output) = runner.evidence_set.join_next(), if !runner.evidence_set.is_empty() => {
                if let Ok(output) = output
                    && !runner.forward_evidence_output(output).await
                {
                    return Ok(());
                }
            }
        }
    }

    // Loose root-level files become scoped candidates after the folder pass.
    if !library_scan_cancel_requested(ctx.cancel_token.as_ref()) {
        let loose_root_files = list_series_loose_root_files(&root).await?;
        if !loose_root_files.is_empty() {
            coordinator
                .register_discovery_batch(loose_root_files.len(), false)
                .await;
            coordinator.publish_progress().await;
            for file in loose_root_files {
                let key = runner.allocate_key();
                let scan_hints = ctx.scan_hints.clone();
                let library_path = ctx.library_path.clone();
                runner.evidence_set.spawn(async move {
                    series_loose_file_evidence_job(file, library_path, scan_hints, key).await
                });
            }
        }
    }

    // DiscoveryDone: the root-level pass (folders plus loose files) is
    // complete; the title-match total is now deterministic even though
    // evidence and inventory jobs may still be running.
    if ctx.mark_discovery_complete_on_drain
        && !library_scan_cancel_requested(ctx.cancel_token.as_ref())
    {
        coordinator.mark_discovery_complete(false).await;
        coordinator.publish_progress().await;
    }

    runner.settle().await;
    Ok(())
}

async fn series_evidence_job(
    folder: PathBuf,
    scan_hints: Option<LibraryScanHintSet>,
    key: ScanCandidateKey,
) -> EvidenceJobOutput {
    let item_path = path_to_stored_string(&folder);
    match prepare_series_library_scan_candidate(folder.clone(), scan_hints.as_ref()).await {
        Ok(candidate) => EvidenceJobOutput::Candidate {
            key,
            candidate: ScanPipelineCandidate::Series(Box::new(candidate)),
            scoped: false,
            inline_inventory: None,
            inventory_target: Some(folder),
        },
        Err(error) => EvidenceJobOutput::Failed { item_path, error },
    }
}

async fn series_loose_file_evidence_job(
    file: LibraryFile,
    library_path: String,
    scan_hints: Option<LibraryScanHintSet>,
    key: ScanCandidateKey,
) -> EvidenceJobOutput {
    let item_path = file.path.clone();
    match prepare_series_library_scan_candidate_from_file(file, &library_path, scan_hints.as_ref())
        .await
    {
        Ok(candidate) => EvidenceJobOutput::Candidate {
            key,
            candidate: ScanPipelineCandidate::Series(Box::new(candidate)),
            scoped: true,
            inline_inventory: None,
            inventory_target: None,
        },
        Err(error) => EvidenceJobOutput::Failed { item_path, error },
    }
}

struct ScanMatchWorkerContext {
    app: AppUseCase,
    actor: User,
    facet: MediaFacet,
    library_id: String,
    library_path: String,
    session_id: String,
    metadata_language: String,
    kind: LibraryScanPipelineKind,
}

struct QueuedMatchCandidate {
    key: ScanCandidateKey,
    candidate: ScanPipelineCandidate,
    queued_at: Instant,
}

struct ScanMatchWorkerState {
    existing_titles: Vec<Title>,
    existing_titles_by_name: HashMap<String, usize>,
    existing_titles_by_tvdb_id: HashMap<String, usize>,
    existing_titles_by_imdb_id: HashMap<String, usize>,
    existing_titles_by_tmdb_id: HashMap<String, usize>,
    search_results: MetadataSearchResults,
    accounted_search_keys: HashSet<BatchMetadataSearchKey>,
    report: ScanMatchWorkerReport,
}

async fn run_scan_match_worker(
    ctx: ScanMatchWorkerContext,
    mut input: tokio::sync::mpsc::Receiver<(ScanCandidateKey, ScanPipelineCandidate)>,
    events: tokio::sync::mpsc::Sender<ScanMatchWorkerEvent>,
    cancel_token: Option<CancellationToken>,
) {
    let coordinator = LibraryScanCoordinator::new(ctx.app.clone(), ctx.session_id.clone());

    let library_ids = vec![ctx.library_id.clone()];
    let existing_titles = match ctx
        .app
        .services
        .catalog
        .titles
        .list_for_libraries(Some(ctx.facet.clone()), &library_ids, None)
        .await
    {
        Ok(titles) => titles,
        Err(error) => {
            warn!(error = %error, "library scan match worker failed to load existing titles");
            Vec::new()
        }
    };
    let (by_name, by_tvdb, by_imdb, by_tmdb) = match ctx.kind {
        LibraryScanPipelineKind::Movie => build_movie_title_indexes(&existing_titles),
        LibraryScanPipelineKind::Series => build_series_title_indexes(&existing_titles),
    };
    let mut state = ScanMatchWorkerState {
        existing_titles,
        existing_titles_by_name: by_name,
        existing_titles_by_tvdb_id: by_tvdb,
        existing_titles_by_imdb_id: by_imdb,
        existing_titles_by_tmdb_id: by_tmdb,
        search_results: MetadataSearchResults::new(),
        accounted_search_keys: HashSet::new(),
        report: ScanMatchWorkerReport {
            summary: LibraryScanSummary::default(),
            unmatched_items: Vec::new(),
            seen_paths: HashSet::new(),
            stats: MetadataLookupBatchStats::default(),
        },
    };

    let mut pending: Vec<QueuedMatchCandidate> = Vec::new();
    let mut in_flight_keys: HashSet<BatchMetadataSearchKey> = HashSet::new();
    let mut search_set: tokio::task::JoinSet<(
        Vec<BatchMetadataSearchKey>,
        AppResult<MetadataSearchResults>,
    )> = tokio::task::JoinSet::new();
    let mut intake_open = true;
    // Set when every pending candidate is waiting on an in-flight key, so the
    // expired flush timer does not busy-loop until new state arrives.
    let mut flush_blocked = false;

    loop {
        if library_scan_cancel_requested(cancel_token.as_ref()) {
            break;
        }
        if !intake_open && pending.is_empty() && search_set.is_empty() {
            break;
        }

        let flush_deadline = if flush_blocked {
            None
        } else {
            pending.first().map(|queued| {
                tokio::time::Instant::from_std(queued.queued_at) + LIBRARY_SCAN_MATCH_FLUSH_INTERVAL
            })
        };

        tokio::select! {
            biased;
            _ = async {
                match cancel_token.as_ref() {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            }, if cancel_token.is_some() => {
                break;
            }
            Some(joined) = search_set.join_next(), if !search_set.is_empty() => {
                let (chunk, result) = match joined {
                    Ok(entry) => entry,
                    Err(error) => {
                        warn!(error = %error, "library scan match batch task failed");
                        continue;
                    }
                };
                for key in &chunk {
                    in_flight_keys.remove(key);
                }
                flush_blocked = false;
                match result {
                    Ok(results) => {
                        state.search_results.extend(results);
                        if resolve_ready_candidates(&ctx, &coordinator, &mut state, &mut pending, &events)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        // SMG batch failure: terminal metadata failure for every
                        // candidate keyed into that chunk; the scan continues.
                        if fail_candidates_for_chunk(&ctx, &coordinator, &mut state, &mut pending, &events, &chunk, &error)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            maybe_candidate = input.recv(), if intake_open
                && pending.len() < LIBRARY_SCAN_MATCH_PENDING_HIGH_WATER => {
                match maybe_candidate {
                    Some((key, candidate)) => {
                        if intake_candidate(&ctx, &coordinator, &mut state, &mut pending, &events, key, candidate)
                            .await
                            .is_err()
                        {
                            break;
                        }
                        flush_blocked = false;
                        if pending.len() == 1 {
                            coordinator.publish_progress().await;
                        }
                    }
                    None => {
                        intake_open = false;
                        flush_blocked = false;
                    }
                }
            }
            _ = async {
                match flush_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            }, if flush_deadline.is_some() && search_set.len() < LIBRARY_SCAN_METADATA_IN_FLIGHT_BATCHES => {
                // Timer flush below.
            }
        }

        // Flush policy: full batch, timer expiry, or closed intake.
        while search_set.len() < LIBRARY_SCAN_METADATA_IN_FLIGHT_BATCHES && !pending.is_empty() {
            let timer_expired = pending.first().is_some_and(|queued| {
                queued.queued_at.elapsed() >= LIBRARY_SCAN_MATCH_FLUSH_INTERVAL
            });
            let size_ready = pending.len() >= LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE;
            if !size_ready && !timer_expired && intake_open {
                break;
            }

            let chunk = match next_pipeline_search_chunk(
                &state.search_results,
                &in_flight_keys,
                &pending,
            ) {
                Ok(chunk) => chunk,
                Err(error) => {
                    warn!(error = %error, "library scan match worker failed to build search chunk");
                    break;
                }
            };
            if chunk.is_empty() {
                // Everything pending is waiting on an in-flight key; disarm
                // the timer until results or new candidates arrive.
                flush_blocked = true;
                break;
            }
            for key in &chunk {
                in_flight_keys.insert(key.clone());
            }
            for key in &chunk {
                if state.accounted_search_keys.insert(key.clone()) {
                    state.report.stats.executed_requests =
                        state.report.stats.executed_requests.saturating_add(1);
                }
            }
            let gateway = ctx.app.services.library.metadata_gateway.clone();
            let language = ctx.metadata_language.clone();
            let batch_cancel = cancel_token.clone();
            search_set.spawn(async move {
                let result = execute_batch_metadata_searches(
                    gateway,
                    chunk.clone(),
                    &language,
                    batch_cancel.as_ref(),
                )
                .await;
                (chunk, result)
            });
        }
    }

    if !library_scan_cancel_requested(cancel_token.as_ref()) {
        coordinator.mark_metadata_total_known().await;
        coordinator.publish_progress().await;
    }

    let report = std::mem::replace(
        &mut state.report,
        ScanMatchWorkerReport {
            summary: LibraryScanSummary::default(),
            unmatched_items: Vec::new(),
            seen_paths: HashSet::new(),
            stats: MetadataLookupBatchStats::default(),
        },
    );
    let _ = events
        .send(ScanMatchWorkerEvent::Done(Box::new(report)))
        .await;
}

async fn intake_candidate(
    ctx: &ScanMatchWorkerContext,
    coordinator: &LibraryScanCoordinator,
    state: &mut ScanMatchWorkerState,
    pending: &mut Vec<QueuedMatchCandidate>,
    events: &tokio::sync::mpsc::Sender<ScanMatchWorkerEvent>,
    key: ScanCandidateKey,
    candidate: ScanPipelineCandidate,
) -> Result<(), ()> {
    state.report.summary.scanned += 1;
    let item_path = match &candidate {
        ScanPipelineCandidate::Movie(movie) => normalize_library_scan_item_path(&movie.file.path),
        ScanPipelineCandidate::Series(series) => series.item_path().trim().to_string(),
    };
    if !item_path.is_empty() {
        state.report.seen_paths.insert(item_path);
    }

    let mut sink = PipelineTitleWorkSink { staged: None };
    let unresolved = match candidate {
        ScanPipelineCandidate::Movie(movie) => process_movie_full_scan_candidate(
            &ctx.app,
            &ctx.actor,
            &ctx.facet,
            &ctx.library_id,
            &ctx.library_path,
            &ctx.session_id,
            coordinator,
            *movie,
            &mut sink,
            &mut state.existing_titles,
            &mut state.existing_titles_by_name,
            &mut state.existing_titles_by_tvdb_id,
            &mut state.existing_titles_by_imdb_id,
            &mut state.existing_titles_by_tmdb_id,
            &mut state.report.summary,
            &mut state.report.unmatched_items,
        )
        .await
        .map(|candidate| candidate.map(|c| ScanPipelineCandidate::Movie(Box::new(c)))),
        ScanPipelineCandidate::Series(series) => process_series_full_scan_candidate(
            &ctx.app,
            &ctx.actor,
            &ctx.facet,
            &ctx.library_id,
            &ctx.library_path,
            &ctx.session_id,
            coordinator,
            *series,
            &mut state.existing_titles,
            &mut state.existing_titles_by_name,
            &mut state.existing_titles_by_tvdb_id,
            &mut state.existing_titles_by_imdb_id,
            &mut state.existing_titles_by_tmdb_id,
            &mut sink,
            &mut state.report.summary,
            &mut state.report.unmatched_items,
        )
        .await
        .map(|candidate| candidate.map(|c| ScanPipelineCandidate::Series(Box::new(c)))),
    };

    let unresolved = match unresolved {
        Ok(unresolved) => unresolved,
        Err(error) => {
            warn!(error = %error, "library scan candidate processing failed");
            state.report.summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
            return send_terminal(events, &mut sink, key).await;
        }
    };

    match unresolved {
        Some(candidate) => {
            // Register the SMG lookup for metadata progress before queueing.
            if let Ok(keys) = candidate.batch_search_keys()
                && !keys.is_empty()
            {
                state.report.stats.logical_lookups =
                    state.report.stats.logical_lookups.saturating_add(1);
                coordinator.add_metadata_total(1).await;
            }
            pending.push(QueuedMatchCandidate {
                key,
                candidate,
                queued_at: Instant::now(),
            });
            Ok(())
        }
        None => send_terminal(events, &mut sink, key).await,
    }
}

async fn send_terminal(
    events: &tokio::sync::mpsc::Sender<ScanMatchWorkerEvent>,
    sink: &mut PipelineTitleWorkSink,
    key: ScanCandidateKey,
) -> Result<(), ()> {
    let event = match sink.staged.take() {
        Some(work) => ScanMatchWorkerEvent::Matched {
            key,
            work: Box::new(work),
        },
        None => ScanMatchWorkerEvent::Terminal { key },
    };
    events.send(event).await.map_err(|_| ())
}

fn next_pipeline_search_chunk(
    search_results: &MetadataSearchResults,
    in_flight_keys: &HashSet<BatchMetadataSearchKey>,
    pending: &[QueuedMatchCandidate],
) -> AppResult<Vec<BatchMetadataSearchKey>> {
    let mut chunk = Vec::new();
    let mut seen = HashSet::new();

    for queued in pending {
        if chunk.len() >= LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE {
            break;
        }
        let mut selected = None;
        for key in queued.candidate.batch_search_keys()? {
            // Resolved keys fall through to the candidate's next variant.
            if search_results.contains_key(&key) {
                continue;
            }
            // A key already dispatched in a previous in-flight batch means
            // the candidate waits for that result instead of eagerly
            // dispatching its fallback variants (no double-dispatch).
            if in_flight_keys.contains(&key) {
                break;
            }
            // A key another candidate already claimed within this chunk is
            // covered by this same batch; fall through to the next variant
            // so same-named siblings keep their distinguishing queries.
            if seen.contains(&key) {
                continue;
            }
            selected = Some(key);
            break;
        }
        if let Some(key) = selected {
            seen.insert(key.clone());
            chunk.push(key);
        }
    }

    Ok(chunk)
}

async fn resolve_ready_candidates(
    ctx: &ScanMatchWorkerContext,
    coordinator: &LibraryScanCoordinator,
    state: &mut ScanMatchWorkerState,
    pending: &mut Vec<QueuedMatchCandidate>,
    events: &tokio::sync::mpsc::Sender<ScanMatchWorkerEvent>,
) -> Result<(), ()> {
    let queued = std::mem::take(pending);
    let (ready, still_pending) = match split_ready_metadata_candidates(
        queued,
        &state.search_results,
        |queued: &QueuedMatchCandidate| queued.candidate.batch_search_keys(),
    ) {
        Ok(split) => split,
        Err(error) => {
            warn!(error = %error, "library scan match worker failed to split ready candidates");
            return Ok(());
        }
    };
    *pending = still_pending;

    for queued in ready {
        let QueuedMatchCandidate { key, candidate, .. } = queued;
        let mut sink = PipelineTitleWorkSink { staged: None };
        let result = match candidate {
            ScanPipelineCandidate::Movie(movie) => {
                process_resolved_movie_full_scan_candidate(
                    &ctx.app,
                    &ctx.actor,
                    &ctx.facet,
                    &ctx.library_id,
                    &ctx.library_path,
                    &ctx.session_id,
                    coordinator,
                    *movie,
                    &state.search_results,
                    &mut sink,
                    &mut state.existing_titles,
                    &mut state.existing_titles_by_name,
                    &mut state.existing_titles_by_tvdb_id,
                    &mut state.existing_titles_by_imdb_id,
                    &mut state.existing_titles_by_tmdb_id,
                    &mut state.report.summary,
                    &mut state.report.unmatched_items,
                )
                .await
            }
            ScanPipelineCandidate::Series(series) => {
                process_resolved_series_full_scan_candidate(
                    &ctx.app,
                    &ctx.actor,
                    &ctx.facet,
                    &ctx.library_id,
                    &ctx.library_path,
                    &ctx.session_id,
                    coordinator,
                    *series,
                    &state.search_results,
                    &mut sink,
                    &mut state.existing_titles,
                    &mut state.existing_titles_by_name,
                    &mut state.existing_titles_by_tvdb_id,
                    &mut state.existing_titles_by_imdb_id,
                    &mut state.existing_titles_by_tmdb_id,
                    &mut state.report.summary,
                    &mut state.report.unmatched_items,
                )
                .await
            }
        };
        if let Err(error) = result {
            warn!(error = %error, "library scan resolved candidate processing failed");
            state.report.summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
        }
        coordinator.mark_metadata_completed(1).await;
        send_terminal(events, &mut sink, key).await?;
    }
    coordinator.publish_progress().await;
    Ok(())
}

async fn fail_candidates_for_chunk(
    ctx: &ScanMatchWorkerContext,
    coordinator: &LibraryScanCoordinator,
    state: &mut ScanMatchWorkerState,
    pending: &mut Vec<QueuedMatchCandidate>,
    events: &tokio::sync::mpsc::Sender<ScanMatchWorkerEvent>,
    chunk: &[BatchMetadataSearchKey],
    error: &AppError,
) -> Result<(), ()> {
    warn!(
        error = %error,
        chunk_size = chunk.len(),
        "library scan SMG match batch failed; failing affected candidates"
    );
    let chunk_keys: HashSet<&BatchMetadataSearchKey> = chunk.iter().collect();
    let queued = std::mem::take(pending);
    let mut failed = Vec::new();

    for queued_candidate in queued {
        let affected = queued_candidate
            .candidate
            .batch_search_keys()
            .map(|keys| keys.iter().any(|key| chunk_keys.contains(key)))
            .unwrap_or(true);
        if affected {
            failed.push(queued_candidate);
        } else {
            pending.push(queued_candidate);
        }
    }

    for queued_candidate in failed {
        let QueuedMatchCandidate { key, candidate, .. } = queued_candidate;
        let unmatched_item = match &candidate {
            ScanPipelineCandidate::Movie(movie) => build_movie_unmatched_scan_item(
                &ctx.facet,
                &ctx.library_id,
                &ctx.session_id,
                &ctx.library_path,
                movie,
                &state.search_results,
                Some("metadata_search_failed"),
                Some(error.to_string()),
            ),
            ScanPipelineCandidate::Series(series) => build_series_unmatched_scan_item(
                &ctx.facet,
                &ctx.library_id,
                &ctx.session_id,
                &ctx.library_path,
                series,
                &state.search_results,
                Some("metadata_search_failed"),
                Some(error.to_string()),
            ),
        };
        if let Err(persist_error) =
            persist_library_scan_unmatched_item(&ctx.app, &unmatched_item).await
        {
            warn!(
                error = %persist_error,
                "failed to persist unmatched item for failed SMG batch"
            );
        }
        state.report.unmatched_items.push(unmatched_item);
        state.report.summary.unmatched += 1;
        coordinator.mark_title_match_completed(1).await;
        coordinator.mark_metadata_failed(1).await;
        events
            .send(ScanMatchWorkerEvent::Terminal { key })
            .await
            .map_err(|_| ())?;
    }
    coordinator.publish_progress().await;
    Ok(())
}
