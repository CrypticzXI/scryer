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
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
    cancel_token: Option<&CancellationToken>,
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
                workset,
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
    }

    Ok(())
}

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
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
    seen_paths: &mut HashSet<String>,
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
            workset,
            summary,
            unmatched_items,
        )
        .await?
        {
            unresolved_candidates.push(candidate);
        }
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
                workset,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                summary,
                unmatched_items,
            )
            .await?;
        }

        coordinator.publish_progress().await;
    }

    coordinator.publish_progress().await;
    Ok(())
}

pub(super) async fn scan_library_movies(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    mark_discovery_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
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
    let mut workset = HashMap::new();
    let metadata_language = app.metadata_language().await;
    let mut prepared_entries = stream_prepared_movie_library_scan_entries(
        app.services.library.library_scanner.clone(),
        queued_discovered_entries,
        library_path.to_string(),
        LIBRARY_SCAN_MOVIE_BATCH_SIZE,
        cancel_token.clone(),
    )?;
    let mut metadata_resolver = StreamingMovieMetadataResolver::new(
        app.services.library.metadata_gateway.clone(),
        metadata_language.clone(),
    );

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

                    if let Some(candidate) = process_movie_full_scan_candidate(
                        app,
                        actor,
                        facet,
                        library_id,
                        library_path,
                        session_id,
                        &coordinator,
                        *candidate,
                        &mut workset,
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

        let (ready_candidate_batches, metadata_progress) = metadata_resolver
            .ingest_candidates(unresolved_candidates, cancel_token.as_ref())
            .await?;
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
            &mut workset,
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

    let metadata_lookup_stats = metadata_resolver.stats();
    if !library_scan_cancel_requested(cancel_token.as_ref()) {
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
            &mut workset,
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

        summary.absorb(
            &app.execute_library_scan_workset(actor, session_id, workset, cancel_token.clone())
                .await?,
        );

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

pub(super) async fn scan_library_series(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    mark_discovery_complete_on_drain: bool,
    cancel_token: Option<CancellationToken>,
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
    let (mut existing_titles_by_name, mut existing_titles_by_tvdb_id) =
        build_series_title_indexes(&existing_titles);

    let mut summary = LibraryScanSummary::default();
    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    let mut seen_paths = HashSet::new();
    let mut unmatched_items = Vec::new();
    let mut workset = HashMap::new();
    let metadata_language = app.metadata_language().await;

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

        process_series_candidate_batch(
            app,
            actor,
            facet,
            library_id,
            library_path,
            session_id,
            &coordinator,
            prepare_series_library_scan_candidates(&folder_batch).await?,
            &metadata_language,
            &mut metadata_lookup_stats,
            &mut existing_titles,
            &mut existing_titles_by_name,
            &mut existing_titles_by_tvdb_id,
            &mut workset,
            &mut summary,
            &mut unmatched_items,
            &mut seen_paths,
            cancel_token.as_ref(),
        )
        .await?;
    }

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
                prepare_series_library_scan_candidates_from_files(&loose_root_files, library_path)
                    .await?,
                &metadata_language,
                &mut metadata_lookup_stats,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut workset,
                &mut summary,
                &mut unmatched_items,
                &mut seen_paths,
                cancel_token.as_ref(),
            )
            .await?;
        }
    }

    if !library_scan_cancel_requested(cancel_token.as_ref()) {
        summary.absorb(
            &app.execute_library_scan_workset(actor, session_id, workset, cancel_token.clone())
                .await?,
        );

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
