use super::*;
use crate::library_scan_helpers::require_directory_library_path;

fn movie_refresh_entry_to_library_file(entry: &MovieTopLevelEntry) -> LibraryFile {
    LibraryFile {
        path: entry.path.to_string_lossy().to_string(),
        display_name: entry
            .path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        nfo_path: matching_movie_nfo_path(&entry.path),
        size_bytes: None,
        source_signature_scheme: None,
        source_signature_value: None,
    }
}

fn movie_refresh_entry_contains_path(entry: &MovieTopLevelEntry, path: &str) -> bool {
    if entry.is_dir {
        path.starts_with(format!("{}/", entry.path.to_string_lossy()).as_str())
            || path == entry.path.to_string_lossy().as_ref()
    } else {
        path == entry.path.to_string_lossy().as_ref()
    }
}

pub(super) async fn maybe_probe_existing_series_title_for_background_refresh(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    let probe_outcome =
        run_background_refresh_probe_with_delta(app, &title.id, folder_path, async {
            let file_scan = app
                .services
                .library_scanner
                .scan_directory_for_progress_with_metrics(folder_path.to_string_lossy().as_ref())
                .await?;
            let discovered_paths = file_scan
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<HashSet<_>>();
            let existing_paths = app
                .services
                .media_files
                .list_media_files_for_title(&title.id)
                .await?
                .into_iter()
                .map(|file| file.file_path)
                .collect::<HashSet<_>>();
            Ok::<_, AppError>((file_scan.files, discovered_paths, existing_paths))
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "background series refresh: failed to probe existing title {} at {}: {error}",
                title.id,
                folder_path.display()
            ))
        })?;

    match probe_outcome {
        BackgroundRefreshProbeOutcome::Unchanged => {
            summary.skipped += 1;
        }
        BackgroundRefreshProbeOutcome::Changed(discovered_files) => {
            merge_library_scan_title_work(
                workset,
                super::scan_candidates::episodic_title_work(
                    title.clone(),
                    discovered_files,
                    LibraryScanTitleWalkMode::Additive,
                ),
            );
            summary.matched += 1;
        }
    }

    Ok(())
}

async fn maybe_probe_existing_movie_title_for_background_refresh(
    app: &AppUseCase,
    title: &Title,
    collections: &[Collection],
    entry: &MovieTopLevelEntry,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    let probe_outcome =
        run_background_refresh_probe_with_delta(app, &title.id, &entry.path, async {
            let discovered_files = if entry.is_dir {
                app.services
                    .library_scanner
                    .scan_directory_for_progress_with_metrics(entry.path.to_string_lossy().as_ref())
                    .await?
                    .files
            } else {
                vec![movie_refresh_entry_to_library_file(entry)]
            };

            let discovered_paths = discovered_files
                .iter()
                .map(|file| file.path.clone())
                .collect::<HashSet<_>>();
            let existing_paths = collections
                .iter()
                .filter_map(|collection| collection.ordered_path.clone())
                .filter(|path| movie_refresh_entry_contains_path(entry, path))
                .collect::<HashSet<_>>();

            Ok::<_, AppError>((discovered_files, discovered_paths, existing_paths))
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "background movie refresh: failed to probe existing title {} at {}: {error}",
                title.id,
                entry.path.display()
            ))
        })?;

    match probe_outcome {
        BackgroundRefreshProbeOutcome::Unchanged => {
            summary.skipped += 1;
        }
        BackgroundRefreshProbeOutcome::Changed(discovered_files) => {
            let discovered_paths = discovered_files
                .iter()
                .map(|file| file.path.clone())
                .collect::<HashSet<_>>();
            let mut cleanup = LibraryScanMovieCleanupContext::default();
            for collection in collections {
                let Some(ordered_path) = collection.ordered_path.as_deref() else {
                    continue;
                };
                if movie_refresh_entry_contains_path(entry, ordered_path)
                    && !discovered_paths.contains(ordered_path)
                {
                    cleanup.stale_collection_ids.push(collection.id.clone());
                }
            }

            merge_library_scan_title_work(
                workset,
                super::scan_candidates::movie_title_work(
                    title.clone(),
                    discovered_files,
                    LibraryScanTitleWalkMode::Additive,
                    cleanup,
                ),
            );
            summary.matched += 1;
        }
    }

    Ok(())
}

pub(super) async fn background_refresh_series(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_path: &str,
    session_id: &str,
) -> AppResult<LibraryScanSummary> {
    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    let root = require_directory_library_path(library_path)?;

    let folders = list_child_directories(root).await?;
    coordinator.set_found_titles(folders.len()).await;
    coordinator.publish_progress().await;

    let mut summary = LibraryScanSummary::default();
    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    let mut workset = HashMap::new();

    let mut existing_titles = app.services.titles.list(Some(facet.clone()), None).await?;
    let (mut existing_titles_by_name, mut existing_titles_by_tvdb_id) =
        build_series_title_indexes(&existing_titles);
    let mut existing_titles_by_folder_path = build_series_title_folder_path_index(&existing_titles);

    let mut unknown_folders = Vec::new();
    for folder in folders {
        summary.scanned += 1;
        let folder_key = folder.to_string_lossy().to_string();
        if let Some(&index) = existing_titles_by_folder_path.get(&folder_key) {
            let title = &mut existing_titles[index];
            maybe_probe_existing_series_title_for_background_refresh(
                app,
                title,
                &folder,
                &mut workset,
                &mut summary,
            )
            .await?;
        } else {
            unknown_folders.push(folder);
        }
    }

    for folder_batch in unknown_folders.chunks(LIBRARY_SCAN_BATCH_SIZE) {
        let prepared_candidates = prepare_series_library_scan_candidates(folder_batch).await?;
        let mut unresolved_candidates = Vec::new();

        for candidate in prepared_candidates {
            if let Some(candidate) = process_series_refresh_candidate(
                app,
                actor,
                facet,
                candidate,
                &mut workset,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_folder_path,
                &mut summary,
            )
            .await?
            {
                unresolved_candidates.push(candidate);
            }
        }

        let (ready_candidate_batches, batch_search_results) = resolve_full_scan_metadata_batches(
            app.services.metadata_gateway.clone(),
            &coordinator,
            unresolved_candidates,
            &mut metadata_lookup_stats,
            build_series_metadata_batch_stats,
            series_candidate_batch_search_keys,
            "background series metadata search chunk unexpectedly empty",
        )
        .await?;

        for ready_candidates in ready_candidate_batches {
            for candidate in ready_candidates {
                process_resolved_series_refresh_candidate(
                    app,
                    actor,
                    facet,
                    candidate,
                    &batch_search_results,
                    &mut workset,
                    &mut existing_titles,
                    &mut existing_titles_by_name,
                    &mut existing_titles_by_tvdb_id,
                    &mut existing_titles_by_folder_path,
                    &mut summary,
                )
                .await?;
            }

            coordinator.publish_progress().await;
        }
    }

    summary.absorb(
        &app.execute_library_scan_workset(actor, session_id, workset)
            .await?,
    );
    coordinator.publish_progress().await;

    info!(
        path = %library_path,
        facet = facet.as_str(),
        scanned = summary.scanned,
        imported = summary.imported,
        matched = summary.matched,
        skipped = summary.skipped,
        unmatched = summary.unmatched,
        metadata_lookups = metadata_lookup_stats.logical_lookups,
        metadata_lookup_requests_executed = metadata_lookup_stats.executed_requests,
        metadata_lookup_requests_coalesced = metadata_lookup_stats.coalesced_requests,
        elapsed_ms = elapsed_ms_u64(started_at),
        "background library refresh completed"
    );

    Ok(summary)
}

pub(super) async fn background_refresh_movies(
    app: &AppUseCase,
    actor: &User,
    library_path: &str,
    session_id: &str,
) -> AppResult<LibraryScanSummary> {
    let started_at = Instant::now();
    let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.to_string());
    let root = require_directory_library_path(library_path)?;

    let entries = list_movie_top_level_entries(root).await?;
    coordinator.set_found_titles(entries.len()).await;
    coordinator.publish_progress().await;

    let mut summary = LibraryScanSummary::default();
    let mut metadata_lookup_stats = MetadataLookupBatchStats::default();
    let mut workset = HashMap::new();
    let mut existing_titles = app
        .services
        .titles
        .list(Some(MediaFacet::Movie), None)
        .await?;
    let (
        mut existing_titles_by_name,
        mut existing_titles_by_tvdb_id,
        mut existing_titles_by_imdb_id,
        mut existing_titles_by_tmdb_id,
    ) = build_movie_title_indexes(&existing_titles);
    let existing_title_ids = existing_titles
        .iter()
        .map(|title| title.id.clone())
        .collect::<Vec<_>>();
    let collections_by_title = app
        .services
        .shows
        .list_collections_for_titles(&existing_title_ids)
        .await
        .unwrap_or_default();

    let mut existing_titles_by_probe_path =
        build_movie_probe_path_indexes(root, &existing_titles, &collections_by_title);

    let mut unknown_files = Vec::new();
    for entry in entries {
        summary.scanned += 1;
        let entry_key = entry.path.to_string_lossy().to_string();
        if let Some(&index) = existing_titles_by_probe_path.get(&entry_key) {
            let title = &existing_titles[index];
            let collections = collections_by_title
                .get(&title.id)
                .cloned()
                .unwrap_or_default();
            maybe_probe_existing_movie_title_for_background_refresh(
                app,
                title,
                &collections,
                &entry,
                &mut workset,
                &mut summary,
            )
            .await?;
            continue;
        }

        if entry.is_dir {
            let mut files = app
                .services
                .library_scanner
                .scan_library(entry.path.to_string_lossy().as_ref())
                .await?;
            unknown_files.append(&mut files);
        } else {
            unknown_files.push(movie_refresh_entry_to_library_file(&entry));
        }
    }

    for file_chunk in unknown_files.chunks(LIBRARY_SCAN_BATCH_SIZE) {
        let prepared_candidates =
            prepare_movie_library_scan_candidates(file_chunk, library_path).await?;
        let mut unresolved_candidates = Vec::new();

        for candidate in prepared_candidates {
            if let Some(candidate) = process_movie_refresh_candidate(
                app,
                actor,
                candidate,
                &mut workset,
                &mut existing_titles,
                &mut existing_titles_by_name,
                &mut existing_titles_by_tvdb_id,
                &mut existing_titles_by_imdb_id,
                &mut existing_titles_by_tmdb_id,
                root,
                &mut existing_titles_by_probe_path,
                &mut summary,
            )
            .await?
            {
                unresolved_candidates.push(candidate);
            }
        }

        let (ready_candidate_batches, batch_search_results) = resolve_full_scan_metadata_batches(
            app.services.metadata_gateway.clone(),
            &coordinator,
            unresolved_candidates,
            &mut metadata_lookup_stats,
            build_movie_metadata_batch_stats,
            movie_candidate_batch_search_keys,
            "background movie metadata search chunk unexpectedly empty",
        )
        .await?;

        for ready_candidates in ready_candidate_batches {
            for candidate in ready_candidates {
                process_resolved_movie_refresh_candidate(
                    app,
                    actor,
                    candidate,
                    &batch_search_results,
                    &mut workset,
                    &mut existing_titles,
                    &mut existing_titles_by_name,
                    &mut existing_titles_by_tvdb_id,
                    &mut existing_titles_by_imdb_id,
                    &mut existing_titles_by_tmdb_id,
                    root,
                    &mut existing_titles_by_probe_path,
                    &mut summary,
                )
                .await?;
            }

            coordinator.publish_progress().await;
        }
    }

    summary.absorb(
        &app.execute_library_scan_workset(actor, session_id, workset)
            .await?,
    );
    coordinator.publish_progress().await;

    info!(
        path = %library_path,
        scanned = summary.scanned,
        imported = summary.imported,
        matched = summary.matched,
        skipped = summary.skipped,
        unmatched = summary.unmatched,
        metadata_lookups = metadata_lookup_stats.logical_lookups,
        metadata_lookup_requests_executed = metadata_lookup_stats.executed_requests,
        metadata_lookup_requests_coalesced = metadata_lookup_stats.coalesced_requests,
        elapsed_ms = elapsed_ms_u64(started_at),
        "background movie refresh completed"
    );

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn list_child_directories_deduplicates_symlinked_show_folders() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("Real Show");
        let link = dir.path().join("Linked Show");
        std::fs::create_dir_all(&target).expect("target dir");
        symlink(&target, &link).expect("symlink");

        let child_dirs = list_child_directories(dir.path())
            .await
            .expect("child dirs");

        assert_eq!(child_dirs, vec![link]);
    }
}
