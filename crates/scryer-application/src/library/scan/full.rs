use super::*;
use crate::library_discovery::list_series_loose_root_files;
use crate::library_scan_helpers::require_directory_library_path;
use crate::library_scan_metadata::prepare_series_library_scan_candidates_from_files;

async fn finalize_full_library_scan(
    app: &AppUseCase,
    coordinator: &LibraryScanCoordinator,
    facet: &MediaFacet,
    library_path: &str,
    seen_paths: &HashSet<String>,
) -> AppResult<()> {
    reconcile_library_scan_unmatched_items(app, facet, library_path, seen_paths).await?;
    coordinator.publish_progress().await;
    Ok(())
}

async fn apply_streaming_metadata_progress(
    coordinator: &LibraryScanCoordinator,
    progress: StreamingMetadataProgressUpdate,
) {
    if progress.total_delta > 0 {
        coordinator.add_metadata_total(progress.total_delta).await;
    }
    if progress.completed_delta > 0 {
        coordinator
            .mark_metadata_completed(progress.completed_delta)
            .await;
    }
    if progress.total_known {
        coordinator.mark_metadata_total_known().await;
    }
    if progress.has_changes() {
        coordinator.publish_progress().await;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "movie scan batches coordinate shared scan state, indexes, and progress reporting"
)]
async fn process_ready_movie_candidate_batches(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    ready_candidate_batches: Vec<Vec<PreparedMovieLibraryScanCandidate>>,
    batch_search_results: &MetadataSearchResults,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
    cancel_token: Option<&CancellationToken>,
    pump_executor: bool,
) -> AppResult<()> {
    for ready_candidates in ready_candidate_batches {
        if library_scan_cancel_requested(cancel_token) {
            break;
        }
        for candidate in ready_candidates {
            if library_scan_cancel_requested(cancel_token) {
                break;
            }
            process_resolved_movie_full_scan_candidate(
                app,
                actor,
                facet,
                library_id,
                library_path,
                session_id,
                coordinator,
                candidate,
                batch_search_results,
                executor,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                summary,
                unmatched_items,
            )
            .await?;
        }

        coordinator.publish_progress().await;
        if pump_executor {
            executor.pump().await?;
        }
    }

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "movie hinted batches coordinate shared scan state, indexes, and progress reporting"
)]
async fn flush_hinted_movie_candidate_batch(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    pending_candidates: &mut Vec<PreparedMovieLibraryScanCandidate>,
    metadata_resolver: &mut StreamingMovieMetadataResolver,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<()> {
    if pending_candidates.is_empty() || library_scan_cancel_requested(cancel_token) {
        return Ok(());
    }

    let candidates = std::mem::take(pending_candidates);
    let (ready_candidate_batches, metadata_progress) = metadata_resolver
        .ingest_candidates(candidates, cancel_token)
        .await?;
    apply_streaming_metadata_progress(coordinator, metadata_progress).await;
    process_ready_movie_candidate_batches(
        app,
        actor,
        facet,
        library_id,
        library_path,
        session_id,
        coordinator,
        ready_candidate_batches,
        metadata_resolver.search_results(),
        executor,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
        summary,
        unmatched_items,
        cancel_token,
        false,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "series scan batches coordinate shared scan state, indexes, and progress reporting"
)]
async fn process_series_candidate_batch(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    prepared_candidates: Vec<PreparedSeriesLibraryScanCandidate>,
    metadata_language: &str,
    metadata_lookup_stats: &mut MetadataLookupBatchStats,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    executor: &mut LibraryScanTitleWorkExecutor,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
    seen_paths: &mut HashSet<String>,
    pump_executor: bool,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<()> {
    let mut unresolved_candidates = Vec::new();

    for candidate in prepared_candidates {
        if library_scan_cancel_requested(cancel_token) {
            break;
        }
        summary.scanned += 1;
        let item_path = candidate.item_path().trim().to_string();
        if !item_path.is_empty() {
            seen_paths.insert(item_path);
        }

        if let Some(candidate) = process_series_full_scan_candidate(
            app,
            actor,
            facet,
            library_id,
            library_path,
            session_id,
            coordinator,
            candidate,
            existing_titles,
            existing_titles_by_name,
            existing_titles_by_tvdb_id,
            existing_titles_by_imdb_id,
            existing_titles_by_tmdb_id,
            executor,
            summary,
            unmatched_items,
        )
        .await?
        {
            unresolved_candidates.push(candidate);
        }
    }

    if pump_executor {
        executor.pump().await?;
    }

    let (ready_candidate_batches, batch_search_results) = resolve_full_scan_metadata_batches(
        app.services.library.metadata_gateway.clone(),
        metadata_language,
        coordinator,
        unresolved_candidates,
        metadata_lookup_stats,
        build_series_metadata_batch_stats,
        series_candidate_batch_search_keys,
        "series metadata search chunk unexpectedly empty",
        cancel_token,
    )
    .await?;

    for ready_candidates in ready_candidate_batches {
        if library_scan_cancel_requested(cancel_token) {
            break;
        }
        for candidate in ready_candidates {
            if library_scan_cancel_requested(cancel_token) {
                break;
            }
            process_resolved_series_full_scan_candidate(
                app,
                actor,
                facet,
                library_id,
                library_path,
                session_id,
                coordinator,
                candidate,
                &batch_search_results,
                executor,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                summary,
                unmatched_items,
            )
            .await?;
        }

        coordinator.publish_progress().await;
        if pump_executor {
            executor.pump().await?;
        }
    }

    coordinator.publish_progress().await;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "hinted series scan flush coordinates shared scan state and indexes"
)]
async fn flush_hinted_series_candidate_batch(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    pending_hinted_candidates: &mut Vec<PreparedSeriesLibraryScanCandidate>,
    metadata_language: &str,
    metadata_lookup_stats: &mut MetadataLookupBatchStats,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    executor: &mut LibraryScanTitleWorkExecutor,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
    seen_paths: &mut HashSet<String>,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<()> {
    if pending_hinted_candidates.is_empty() {
        return Ok(());
    }

    let candidates = std::mem::take(pending_hinted_candidates);
    process_series_candidate_batch(
        app,
        actor,
        facet,
        library_id,
        library_path,
        session_id,
        coordinator,
        candidates,
        metadata_language,
        metadata_lookup_stats,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
        executor,
        summary,
        unmatched_items,
        seen_paths,
        false,
        cancel_token,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "movie library scans are top-level orchestration entry points with explicit runtime state"
)]
pub(super) async fn scan_library_movies(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    mark_discovery_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<LibraryScanSummary> {
    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    let root = require_directory_library_path(library_path)?;
    let discovered_entries =
        stream_movie_top_level_entries_batched(root, LIBRARY_SCAN_MOVIE_BATCH_SIZE).await?;
    let queued_discovered_entries = spawn_library_discovery_queue(
        app.clone(),
        session_id.to_string(),
        discovered_entries,
        false,
        mark_discovery_complete_on_drain,
        cancel_token.clone(),
    );

    let library_ids = vec![library_id.to_string()];
    let mut existing_titles = app
        .services
        .catalog
        .titles
        .list_for_libraries(Some(facet.clone()), &library_ids, None)
        .await?;
    let (
        mut existing_titles_by_name,
        mut existing_titles_by_tvdb_id,
        mut existing_titles_by_imdb_id,
        mut existing_titles_by_tmdb_id,
    ) = build_movie_title_indexes(&existing_titles);

    let mut summary = LibraryScanSummary::default();
    let mut seen_paths = HashSet::new();
    let mut unmatched_items = Vec::new();
    let mut executor =
        LibraryScanTitleWorkExecutor::for_scan(app, actor, session_id, cancel_token.clone())
            .await?;
    let metadata_language = app.metadata_language().await;
    let mut prepared_entries = stream_prepared_movie_library_scan_entries(
        app.services.library.library_scanner.clone(),
        queued_discovered_entries,
        library_path.to_string(),
        LIBRARY_SCAN_MOVIE_BATCH_SIZE,
        cancel_token.clone(),
        scan_hints.cloned(),
    )?;
    let mut metadata_resolver = StreamingMovieMetadataResolver::new(
        app.services.library.metadata_gateway.clone(),
        metadata_language.clone(),
    );
    let mut pending_hinted_candidates = Vec::new();

    while let Some(prepared_batch_result) =
        await_cancellable(cancel_token.as_ref(), prepared_entries.recv())
            .await
            .flatten()
    {
        if library_scan_cancel_requested(cancel_token.as_ref()) {
            break;
        }
        let prepared_batch = prepared_batch_result?;
        if prepared_batch.is_empty() {
            continue;
        }

        let mut unresolved_candidates = Vec::new();
        let mut saw_hinted_candidate = false;

        for prepared_entry in prepared_batch {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            match prepared_entry {
                PreparedMovieLibraryScanEntry::Candidate(candidate) => {
                    summary.scanned += 1;
                    let item_path = normalize_library_scan_item_path(&candidate.file.path);
                    if !item_path.is_empty() {
                        seen_paths.insert(item_path);
                    }
                    let is_hinted_candidate = candidate.has_external_import_identity_ids();
                    saw_hinted_candidate |= is_hinted_candidate;

                    if let Some(candidate) = process_movie_full_scan_candidate(
                        app,
                        actor,
                        facet,
                        library_id,
                        library_path,
                        session_id,
                        &coordinator,
                        *candidate,
                        &mut executor,
                        &mut existing_titles,
                        &mut existing_titles_by_name,
                        &mut existing_titles_by_tvdb_id,
                        &mut existing_titles_by_imdb_id,
                        &mut existing_titles_by_tmdb_id,
                        &mut summary,
                        &mut unmatched_items,
                    )
                    .await?
                    {
                        unresolved_candidates.push(candidate);
                    }
                }
                PreparedMovieLibraryScanEntry::Skipped { item_path } => {
                    summary.scanned += 1;
                    summary.skipped += 1;
                    clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
                    coordinator.mark_title_match_completed(1).await;
                }
            }
        }

        let mut standard_unresolved_candidates = Vec::new();
        for candidate in unresolved_candidates {
            if candidate.has_external_import_identity_ids() {
                pending_hinted_candidates.push(candidate);
            } else {
                standard_unresolved_candidates.push(candidate);
            }
        }

        if !standard_unresolved_candidates.is_empty() {
            let (ready_candidate_batches, metadata_progress) = metadata_resolver
                .ingest_candidates(standard_unresolved_candidates, cancel_token.as_ref())
                .await?;
            apply_streaming_metadata_progress(&coordinator, metadata_progress).await;
            let can_pump_executor = pending_hinted_candidates.is_empty() && !saw_hinted_candidate;
            process_ready_movie_candidate_batches(
                app,
                actor,
                facet,
                library_id,
                library_path,
                session_id,
                &coordinator,
                ready_candidate_batches,
                metadata_resolver.search_results(),
                &mut executor,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                &mut summary,
                &mut unmatched_items,
                cancel_token.as_ref(),
                can_pump_executor,
            )
            .await?;
            if can_pump_executor {
                executor.pump().await?;
            }
        } else if pending_hinted_candidates.len() >= LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE {
            flush_hinted_movie_candidate_batch(
                app,
                actor,
                facet,
                library_id,
                library_path,
                session_id,
                &coordinator,
                &mut pending_hinted_candidates,
                &mut metadata_resolver,
                &mut executor,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                &mut summary,
                &mut unmatched_items,
                cancel_token.as_ref(),
            )
            .await?;
        }
    }

    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    if !library_scan_cancel_requested(cancel_token.as_ref()) {
        flush_hinted_movie_candidate_batch(
            app,
            actor,
            facet,
            library_id,
            library_path,
            session_id,
            &coordinator,
            &mut pending_hinted_candidates,
            &mut metadata_resolver,
            &mut executor,
            &mut existing_titles,
            &mut existing_titles_by_name,
            &mut existing_titles_by_tvdb_id,
            &mut existing_titles_by_imdb_id,
            &mut existing_titles_by_tmdb_id,
            &mut summary,
            &mut unmatched_items,
            cancel_token.as_ref(),
        )
        .await?;

        metadata_lookup_stats = metadata_resolver.stats();
        let (ready_candidate_batches, metadata_progress) =
            metadata_resolver.finish(cancel_token.as_ref()).await?;
        apply_streaming_metadata_progress(&coordinator, metadata_progress).await;
        process_ready_movie_candidate_batches(
            app,
            actor,
            facet,
            library_id,
            library_path,
            session_id,
            &coordinator,
            ready_candidate_batches,
            metadata_resolver.search_results(),
            &mut executor,
            &mut existing_titles,
            &mut existing_titles_by_name,
            &mut existing_titles_by_tvdb_id,
            &mut existing_titles_by_imdb_id,
            &mut existing_titles_by_tmdb_id,
            &mut summary,
            &mut unmatched_items,
            cancel_token.as_ref(),
            true,
        )
        .await?;
    }
    executor.close_input();
    summary.absorb(&executor.finish().await?);
    if !library_scan_cancel_requested(cancel_token.as_ref()) {
        finalize_full_library_scan(app, &coordinator, facet, library_path, &seen_paths).await?;
    }

    info!(
        path = %library_path,
        scanned = summary.scanned,
        matched = summary.matched,
        imported = summary.imported,
        skipped = summary.skipped,
        unmatched = summary.unmatched,
        metadata_lookups = metadata_lookup_stats.logical_lookups,
        metadata_lookup_requests_executed = metadata_lookup_stats.executed_requests,
        metadata_lookup_requests_coalesced = metadata_lookup_stats.coalesced_requests,
        batch_size = LIBRARY_SCAN_MOVIE_BATCH_SIZE,
        worker_concurrency = LIBRARY_METADATA_LOOKUP_CONCURRENCY,
        elapsed_ms = elapsed_ms_u64(started_at),
        "movie library scan completed"
    );

    if !unmatched_items.is_empty() {
        info!(
            count = unmatched_items.len(),
            "movie library scan unmatched files follow"
        );
        for unmatched in unmatched_items {
            info!(
                path = %unmatched.item_path,
                display_name = %unmatched.display_name,
                query = %unmatched.query,
                year_hint = ?unmatched.year_hint,
                reason = %unmatched.reason_code,
                search_attempts = %format_library_scan_unmatched_search_attempts(&unmatched.search_attempts),
                "movie library scan unmatched file"
            );
        }
    }

    Ok(summary)
}

#[expect(
    clippy::too_many_arguments,
    reason = "series library scans are top-level orchestration entry points with explicit runtime state"
)]
pub(super) async fn scan_library_series(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    mark_discovery_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
    scan_hints: Option<&LibraryScanHintSet>,
) -> AppResult<LibraryScanSummary> {
    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    let root = require_directory_library_path(library_path)?;
    let discovered_folders =
        stream_child_directories_batched(root, LIBRARY_SCAN_SERIES_BATCH_SIZE).await?;
    let mut queued_discovered_folders = spawn_library_discovery_queue(
        app.clone(),
        session_id.to_string(),
        discovered_folders,
        false,
        mark_discovery_complete_on_drain,
        cancel_token.clone(),
    );

    let library_ids = vec![library_id.to_string()];
    let mut existing_titles = app
        .services
        .catalog
        .titles
        .list_for_libraries(Some(facet.clone()), &library_ids, None)
        .await?;
    let (
        mut existing_titles_by_name,
        mut existing_titles_by_tvdb_id,
        mut existing_titles_by_imdb_id,
        mut existing_titles_by_tmdb_id,
    ) = build_series_title_indexes(&existing_titles);

    let mut summary = LibraryScanSummary::default();
    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    let mut seen_paths = HashSet::new();
    let mut unmatched_items = Vec::new();
    let mut executor =
        LibraryScanTitleWorkExecutor::for_scan(app, actor, session_id, cancel_token.clone())
            .await?;
    let metadata_language = app.metadata_language().await;
    let mut pending_hinted_candidates = Vec::new();

    while let Some(folder_batch_result) =
        await_cancellable(cancel_token.as_ref(), queued_discovered_folders.recv())
            .await
            .flatten()
    {
        if library_scan_cancel_requested(cancel_token.as_ref()) {
            break;
        }
        let folder_batch = folder_batch_result?;
        if folder_batch.is_empty() {
            continue;
        }

        let prepared_candidates =
            prepare_series_library_scan_candidates(&folder_batch, scan_hints).await?;
        let mut standard_candidates = Vec::new();
        for candidate in prepared_candidates {
            if candidate.has_external_import_identity_ids() {
                pending_hinted_candidates.push(candidate);
            } else {
                standard_candidates.push(candidate);
            }
        }

        if !standard_candidates.is_empty() {
            let can_pump_executor = pending_hinted_candidates.is_empty();
            process_series_candidate_batch(
                app,
                actor,
                facet,
                library_id,
                library_path,
                session_id,
                &coordinator,
                standard_candidates,
                &metadata_language,
                &mut metadata_lookup_stats,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                &mut executor,
                &mut summary,
                &mut unmatched_items,
                &mut seen_paths,
                can_pump_executor,
                cancel_token.as_ref(),
            )
            .await?;
            if can_pump_executor {
                executor.pump().await?;
            }
        } else if pending_hinted_candidates.len() >= LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE {
            flush_hinted_series_candidate_batch(
                app,
                actor,
                facet,
                library_id,
                library_path,
                session_id,
                &coordinator,
                &mut pending_hinted_candidates,
                &metadata_language,
                &mut metadata_lookup_stats,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                &mut executor,
                &mut summary,
                &mut unmatched_items,
                &mut seen_paths,
                cancel_token.as_ref(),
            )
            .await?;
        }
    }

    flush_hinted_series_candidate_batch(
        app,
        actor,
        facet,
        library_id,
        library_path,
        session_id,
        &coordinator,
        &mut pending_hinted_candidates,
        &metadata_language,
        &mut metadata_lookup_stats,
        &mut existing_titles,
        &mut existing_titles_by_name,
        &mut existing_titles_by_tvdb_id,
        &mut existing_titles_by_imdb_id,
        &mut existing_titles_by_tmdb_id,
        &mut executor,
        &mut summary,
        &mut unmatched_items,
        &mut seen_paths,
        cancel_token.as_ref(),
    )
    .await?;

    if !library_scan_cancel_requested(cancel_token.as_ref()) {
        let loose_root_files = list_series_loose_root_files(root).await?;
        if !loose_root_files.is_empty() {
            coordinator
                .register_discovery_batch(loose_root_files.len(), false)
                .await;
            coordinator.publish_progress().await;
            process_series_candidate_batch(
                app,
                actor,
                facet,
                library_id,
                library_path,
                session_id,
                &coordinator,
                prepare_series_library_scan_candidates_from_files(
                    &loose_root_files,
                    library_path,
                    scan_hints,
                )
                .await?,
                &metadata_language,
                &mut metadata_lookup_stats,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                &mut executor,
                &mut summary,
                &mut unmatched_items,
                &mut seen_paths,
                true,
                cancel_token.as_ref(),
            )
            .await?;
            executor.pump().await?;
        }
    }

    executor.close_input();
    summary.absorb(&executor.finish().await?);
    if !library_scan_cancel_requested(cancel_token.as_ref()) {
        finalize_full_library_scan(app, &coordinator, facet, library_path, &seen_paths).await?;
    }

    info!(
        path = %library_path,
        facet = facet.as_str(),
        folders = summary.scanned,
        imported = summary.imported,
        skipped = summary.skipped,
        unmatched = summary.unmatched,
        metadata_lookups = metadata_lookup_stats.logical_lookups,
        metadata_lookup_requests_executed = metadata_lookup_stats.executed_requests,
        metadata_lookup_requests_coalesced = metadata_lookup_stats.coalesced_requests,
        batch_size = LIBRARY_SCAN_SERIES_BATCH_SIZE,
        worker_concurrency = LIBRARY_METADATA_LOOKUP_CONCURRENCY,
        elapsed_ms = elapsed_ms_u64(started_at),
        "{} library scan completed",
        facet.as_str()
    );

    if !unmatched_items.is_empty() {
        info!(
            count = unmatched_items.len(),
            facet = facet.as_str(),
            "{} library scan unmatched folders follow",
            facet.as_str()
        );
        for unmatched in unmatched_items {
            info!(
                path = %unmatched.item_path,
                display_name = %unmatched.display_name,
                query = %unmatched.query,
                year_hint = ?unmatched.year_hint,
                reason = %unmatched.reason_code,
                error_message = ?unmatched.error_message,
                search_attempts = %format_library_scan_unmatched_search_attempts(&unmatched.search_attempts),
                "{} library scan unmatched folder",
                facet.as_str()
            );
        }
    }

    Ok(summary)
}
