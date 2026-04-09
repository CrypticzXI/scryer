use super::*;
use crate::library_scan_helpers::require_directory_library_path;

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

pub(super) async fn scan_library_movies(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_path: &str,
    session_id: &str,
) -> AppResult<LibraryScanSummary> {
    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    let discovered_files = app
        .services
        .library
        .library_scanner
        .scan_library_batched(library_path, LIBRARY_SCAN_BATCH_SIZE)
        .await?;
    let mut queued_discovered_files =
        spawn_library_discovery_queue(app.clone(), session_id.to_string(), discovered_files, false);

    let mut existing_titles = app
        .services
        .catalog
        .titles
        .list(Some(facet.clone()), None)
        .await?;
    let (
        mut existing_titles_by_name,
        mut existing_titles_by_tvdb_id,
        mut existing_titles_by_imdb_id,
        mut existing_titles_by_tmdb_id,
    ) = build_movie_title_indexes(&existing_titles);

    let mut summary = LibraryScanSummary::default();
    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    let mut seen_paths = HashSet::new();
    let mut unmatched_items = Vec::new();
    let mut workset = HashMap::new();

    while let Some(file_chunk) = queued_discovered_files.recv().await {
        let file_chunk = file_chunk?;
        if file_chunk.is_empty() {
            continue;
        }
        let prepared_candidates =
            prepare_movie_library_scan_candidates(&file_chunk, library_path).await?;
        let mut unresolved_candidates = Vec::new();

        for candidate in prepared_candidates {
            summary.scanned += 1;
            let item_path = normalize_library_scan_item_path(&candidate.file.path);
            if !item_path.is_empty() {
                seen_paths.insert(item_path);
            }

            if let Some(candidate) = process_movie_full_scan_candidate(
                app,
                actor,
                facet,
                &coordinator,
                candidate,
                &mut workset,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                &mut summary,
            )
            .await?
            {
                unresolved_candidates.push(candidate);
            }
        }

        let (ready_candidate_batches, batch_search_results) = resolve_full_scan_metadata_batches(
            app.services.library.metadata_gateway.clone(),
            &coordinator,
            unresolved_candidates,
            &mut metadata_lookup_stats,
            build_movie_metadata_batch_stats,
            movie_candidate_batch_search_keys,
            "movie metadata search chunk unexpectedly empty",
        )
        .await?;

        for ready_candidates in ready_candidate_batches {
            for candidate in ready_candidates {
                process_resolved_movie_full_scan_candidate(
                    app,
                    actor,
                    facet,
                    library_path,
                    session_id,
                    &coordinator,
                    candidate,
                    &batch_search_results,
                    &mut workset,
                    &mut existing_titles,
                    &mut existing_titles_by_name,
                    &mut existing_titles_by_tvdb_id,
                    &mut existing_titles_by_imdb_id,
                    &mut existing_titles_by_tmdb_id,
                    &mut summary,
                    &mut unmatched_items,
                )
                .await?;
            }

            coordinator.publish_progress().await;
        }

        coordinator.publish_progress().await;
    }

    summary.absorb(
        &app.execute_library_scan_workset(actor, session_id, workset)
            .await?,
    );

    finalize_full_library_scan(app, &coordinator, facet, library_path, &seen_paths).await?;

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
        batch_size = LIBRARY_SCAN_BATCH_SIZE,
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
    library_path: &str,
    session_id: &str,
) -> AppResult<LibraryScanSummary> {
    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    let root = require_directory_library_path(library_path)?;
    let discovered_folders =
        stream_child_directories_batched(root, LIBRARY_SCAN_BATCH_SIZE).await?;
    let mut queued_discovered_folders = spawn_library_discovery_queue(
        app.clone(),
        session_id.to_string(),
        discovered_folders,
        false,
    );

    let mut existing_titles = app
        .services
        .catalog
        .titles
        .list(Some(facet.clone()), None)
        .await?;
    let (mut existing_titles_by_name, mut existing_titles_by_tvdb_id) =
        build_series_title_indexes(&existing_titles);

    let mut summary = LibraryScanSummary::default();
    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    let mut seen_paths = HashSet::new();
    let mut unmatched_items = Vec::new();
    let mut workset = HashMap::new();

    while let Some(folder_batch_result) = queued_discovered_folders.recv().await {
        let folder_batch = folder_batch_result?;
        if folder_batch.is_empty() {
            continue;
        }

        let prepared_candidates = prepare_series_library_scan_candidates(&folder_batch).await?;
        let mut unresolved_candidates = Vec::new();

        for candidate in prepared_candidates {
            summary.scanned += 1;
            let item_path = candidate.folder_path.to_string_lossy().trim().to_string();
            if !item_path.is_empty() {
                seen_paths.insert(item_path);
            }

            if let Some(candidate) = process_series_full_scan_candidate(
                app,
                actor,
                facet,
                library_path,
                session_id,
                &coordinator,
                candidate,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut workset,
                &mut summary,
                &mut unmatched_items,
            )
            .await?
            {
                unresolved_candidates.push(candidate);
            }
        }

        let (ready_candidate_batches, batch_search_results) = resolve_full_scan_metadata_batches(
            app.services.library.metadata_gateway.clone(),
            &coordinator,
            unresolved_candidates,
            &mut metadata_lookup_stats,
            build_series_metadata_batch_stats,
            series_candidate_batch_search_keys,
            "series metadata search chunk unexpectedly empty",
        )
        .await?;

        for ready_candidates in ready_candidate_batches {
            for candidate in ready_candidates {
                process_resolved_series_full_scan_candidate(
                    app,
                    actor,
                    facet,
                    library_path,
                    session_id,
                    &coordinator,
                    candidate,
                    &batch_search_results,
                    &mut workset,
                    &mut existing_titles,
                    &mut existing_titles_by_name,
                    &mut existing_titles_by_tvdb_id,
                    &mut summary,
                    &mut unmatched_items,
                )
                .await?;
            }

            coordinator.publish_progress().await;
        }

        coordinator.publish_progress().await;
    }

    summary.absorb(
        &app.execute_library_scan_workset(actor, session_id, workset)
            .await?,
    );

    finalize_full_library_scan(app, &coordinator, facet, library_path, &seen_paths).await?;

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
        batch_size = LIBRARY_SCAN_BATCH_SIZE,
        worker_concurrency = LIBRARY_METADATA_LOOKUP_CONCURRENCY,
        elapsed_ms = elapsed_ms_u64(started_at),
        "series library scan completed"
    );

    if !unmatched_items.is_empty() {
        info!(
            count = unmatched_items.len(),
            facet = facet.as_str(),
            "series library scan unmatched folders follow"
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
                "series library scan unmatched folder"
            );
        }
    }

    Ok(summary)
}
