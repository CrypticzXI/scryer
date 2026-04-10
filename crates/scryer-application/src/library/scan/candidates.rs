use super::*;

async fn ensure_title_folder_path_if_missing(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) {
    let folder_path = folder_path.to_string_lossy().trim().to_string();
    if folder_path.is_empty()
        || title
            .folder_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return;
    }

    match app
        .services
        .catalog
        .titles
        .set_folder_path(&title.id, folder_path.as_str())
        .await
    {
        Ok(()) => {
            title.folder_path = Some(folder_path);
        }
        Err(error) => warn!(
            error = %error,
            folder_path = %folder_path,
            "failed to persist discovered title folder path during library scan"
        ),
    }
}

fn external_ids_from_nfo(nfo_meta: &crate::nfo::NfoMetadata) -> Vec<ExternalId> {
    let mut external_ids = Vec::new();
    if let Some(tvdb_id) = nfo_meta.tvdb_id.as_deref() {
        external_ids.push(ExternalId {
            source: "tvdb".into(),
            value: tvdb_id.to_string(),
        });
    }
    if let Some(imdb_id) = nfo_meta.imdb_id.as_deref() {
        external_ids.push(ExternalId {
            source: "imdb".into(),
            value: imdb_id.to_string(),
        });
    }
    if let Some(tmdb_id) = nfo_meta.tmdb_id.as_deref() {
        external_ids.push(ExternalId {
            source: "tmdb".into(),
            value: tmdb_id.to_string(),
        });
    }
    external_ids
}

fn build_new_movie_title_from_nfo(
    candidate: &PreparedMovieLibraryScanCandidate,
    facet: &MediaFacet,
) -> Option<NewTitle> {
    let nfo_meta = candidate.nfo_meta.as_ref()?;
    let _tvdb_id = nfo_meta.tvdb_id.as_deref()?;

    Some(NewTitle {
        name: nfo_meta
            .title
            .clone()
            .unwrap_or_else(|| candidate.query.clone()),
        facet: facet.clone(),
        monitored: false,
        tags: vec![],
        external_ids: external_ids_from_nfo(nfo_meta),
        min_availability: None,
        year: nfo_meta.year,
        ..Default::default()
    })
}

fn build_new_series_title_from_nfo(
    candidate: &PreparedSeriesLibraryScanCandidate,
    facet: &MediaFacet,
) -> Option<NewTitle> {
    let nfo_meta = candidate.nfo_meta.as_ref()?;
    let _tvdb_id = nfo_meta.tvdb_id.as_deref()?;
    let fallback_name = candidate
        .folder_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(candidate.query.as_str());

    Some(NewTitle {
        name: nfo_meta
            .title
            .clone()
            .unwrap_or_else(|| fallback_name.to_string()),
        facet: facet.clone(),
        monitored: false,
        tags: vec![],
        external_ids: external_ids_from_nfo(nfo_meta),
        min_availability: None,
        year: nfo_meta.year,
        ..Default::default()
    })
}

fn find_existing_movie_title_index(
    candidate: &PreparedMovieLibraryScanCandidate,
    existing_titles: &[Title],
    existing_titles_by_name: &HashMap<String, usize>,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
    existing_titles_by_imdb_id: &HashMap<String, usize>,
    existing_titles_by_tmdb_id: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(tvdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.tvdb_id.as_deref())
        && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
    {
        return Some(index);
    }

    if let Some(nfo_imdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.imdb_id.as_deref())
        .and_then(crate::normalize::normalize_imdb_id)
        && let Some(&index) = existing_titles_by_imdb_id.get(&nfo_imdb_id)
    {
        return Some(index);
    }

    if let Some(nfo_tmdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.tmdb_id.as_deref())
        .map(str::to_string)
        && let Some(&index) = existing_titles_by_tmdb_id.get(&nfo_tmdb_id)
    {
        return Some(index);
    }

    if let Some(parsed_imdb_id) = candidate
        .parsed_release
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
        && let Some(&index) = existing_titles_by_imdb_id.get(&parsed_imdb_id)
    {
        return Some(index);
    }

    if let Some(parsed_tmdb_id) = candidate.parsed_release.tmdb_id.map(|id| id.to_string())
        && let Some(&index) = existing_titles_by_tmdb_id.get(&parsed_tmdb_id)
    {
        return Some(index);
    }

    candidate.query_variants.iter().find_map(|query_variant| {
        let normalized = crate::title_matching::canonical_lookup_key(query_variant);
        existing_titles_by_name
            .get(&normalized)
            .copied()
            .filter(|index| {
                let title = &existing_titles[*index];
                candidate.year_hint.is_none()
                    || title.year.map(|value| value as u32) == candidate.year_hint
                    || title.year.is_none()
            })
    })
}

fn find_existing_series_title_index(
    candidate: &PreparedSeriesLibraryScanCandidate,
    existing_titles: &[Title],
    existing_titles_by_name: &HashMap<String, usize>,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(tvdb_id) = candidate
        .nfo_meta
        .as_ref()
        .and_then(|meta| meta.tvdb_id.as_deref())
        && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
    {
        return Some(index);
    }

    candidate
        .title_match_candidates
        .iter()
        .find_map(|name_key| {
            existing_titles_by_name
                .get(name_key)
                .copied()
                .filter(|index| {
                    let title = &existing_titles[*index];
                    candidate.year_hint.is_none()
                        || title.year.map(|value| value as u32) == candidate.year_hint
                        || title.year.is_none()
                })
        })
}

enum MovieCandidateResolution {
    Ready(Title),
    ReadyCreated { index: usize, title: Title },
    CreateFailed(AppError),
    Skipped,
    Unresolved(Box<PreparedMovieLibraryScanCandidate>),
}

enum MovieMetadataResolution {
    Ready(Title),
    ReadyCreated { index: usize, title: Title },
    CreateFailed(AppError),
    Unmatched,
}

pub(super) fn movie_title_work(
    title: Title,
    pre_scanned_files: Vec<LibraryFile>,
    mode: LibraryScanTitleWalkMode,
    cleanup: LibraryScanMovieCleanupContext,
    created_in_scan: bool,
) -> LibraryScanTitleWork {
    LibraryScanTitleWork {
        title,
        facet_plan: LibraryScanTitleFacetPlan::Movie(cleanup),
        discovered_files: Some(pre_scanned_files),
        mode,
        created_in_scan,
    }
}

fn merge_default_movie_title_work(
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    title: Title,
    discovered_files: Vec<LibraryFile>,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) {
    merge_library_scan_title_work(
        workset,
        movie_title_work(
            title,
            discovered_files,
            mode,
            LibraryScanMovieCleanupContext::default(),
            created_in_scan,
        ),
    );
}

pub(super) fn episodic_title_work(
    title: Title,
    pre_scanned_files: Vec<LibraryFile>,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) -> LibraryScanTitleWork {
    LibraryScanTitleWork {
        title,
        facet_plan: LibraryScanTitleFacetPlan::Episodic,
        discovered_files: Some(pre_scanned_files),
        mode,
        created_in_scan,
    }
}

async fn scan_title_files_for_library_scan_session(
    library_scanner: Arc<dyn LibraryScanner>,
    title_id: &str,
    folder_path: &Path,
) -> Vec<LibraryFile> {
    let started_at = Instant::now();
    let folder_path_display = folder_path.display().to_string();
    let pre_scan = match scan_episodic_title_directory_for_progress_metrics(
        library_scanner,
        folder_path,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title_id,
                folder_path = %folder_path_display,
                "failed to scan episodic title folder for library scan progress"
            );
            return Vec::new();
        }
    };

    let files_count = pre_scan.files.len();

    info!(
        title_id = %title_id,
        path = %folder_path_display,
        files = files_count,
        walk_ms = pre_scan.walk_ms,
        stat_ms = pre_scan.stat_ms,
        analyze_ms = 0u64,
        db_ms = 0u64,
        elapsed_ms = elapsed_ms_u64(started_at),
        "episodic title directory scan completed"
    );

    pre_scan.files
}

pub(super) async fn scan_episodic_title_directory_for_progress_metrics(
    library_scanner: Arc<dyn LibraryScanner>,
    folder_path: &Path,
) -> AppResult<LibraryDirectoryScanResult> {
    library_scanner
        .scan_directory_for_progress_with_metrics(folder_path.to_string_lossy().as_ref())
        .await
}

async fn merge_series_title_work_for_index(
    app: &AppUseCase,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut [Title],
    index: usize,
    folder_path: &Path,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) {
    let title_id = existing_titles[index].id.clone();
    let pre_scanned_files = scan_title_files_for_library_scan_session(
        app.services.library.library_scanner.clone(),
        &title_id,
        folder_path,
    )
    .await;
    ensure_title_folder_path_if_missing(app, &mut existing_titles[index], folder_path).await;
    merge_library_scan_title_work(
        workset,
        episodic_title_work(
            existing_titles[index].clone(),
            pre_scanned_files,
            mode,
            created_in_scan,
        ),
    );
}

async fn append_series_title_and_merge_work(
    app: &AppUseCase,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    title: Title,
    folder_path: &Path,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) -> usize {
    let index = append_series_title(
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        title,
    );
    merge_series_title_work_for_index(
        app,
        workset,
        existing_titles,
        index,
        folder_path,
        mode,
        created_in_scan,
    )
    .await;
    index
}

async fn resolve_movie_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    candidate: PreparedMovieLibraryScanCandidate,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) -> AppResult<MovieCandidateResolution> {
    if let Some(index) = find_existing_movie_title_index(
        &candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    ) {
        return Ok(MovieCandidateResolution::Ready(
            existing_titles[index].clone(),
        ));
    }

    if let Some(new_title) = build_new_movie_title_from_nfo(&candidate, facet) {
        match app.create_title_without_hydration(actor, new_title).await {
            Ok(created) => {
                let index = append_movie_title(
                    existing_titles,
                    existing_titles_by_name,
                    existing_titles_by_tvdb_id,
                    existing_titles_by_imdb_id,
                    existing_titles_by_tmdb_id,
                    created.clone(),
                );
                return Ok(MovieCandidateResolution::ReadyCreated {
                    index,
                    title: created,
                });
            }
            Err(error) => return Ok(MovieCandidateResolution::CreateFailed(error)),
        }
    }

    if candidate.query.trim().is_empty() {
        return Ok(MovieCandidateResolution::Skipped);
    }

    Ok(MovieCandidateResolution::Unresolved(Box::new(candidate)))
}

async fn resolve_movie_metadata_match(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    candidate: &PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) -> AppResult<MovieMetadataResolution> {
    let selected_metadata =
        select_movie_metadata_from_batch_results(candidate, batch_search_results)?;
    let Some(selected) = selected_metadata else {
        return Ok(MovieMetadataResolution::Unmatched);
    };

    if let Some(index) = find_existing_title_index_for_metadata_match(
        &selected,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
    ) {
        return Ok(MovieMetadataResolution::Ready(
            existing_titles[index].clone(),
        ));
    }

    match app
        .create_title_without_hydration(
            actor,
            build_new_title_from_metadata_match(facet, &selected),
        )
        .await
    {
        Ok(created) => {
            let index = append_movie_title(
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                created.clone(),
            );
            Ok(MovieMetadataResolution::ReadyCreated {
                index,
                title: created,
            })
        }
        Err(error) => Ok(MovieMetadataResolution::CreateFailed(error)),
    }
}

pub(super) async fn process_movie_full_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedMovieLibraryScanCandidate,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<Option<PreparedMovieLibraryScanCandidate>> {
    let discovered_files = candidate.discovered_files.clone();
    let item_path = normalize_library_scan_item_path(&candidate.file.path);

    match resolve_movie_scan_candidate(
        app,
        actor,
        facet,
        candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
    .await?
    {
        MovieCandidateResolution::Ready(title) => {
            summary.matched += 1;
            merge_default_movie_title_work(
                workset,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                false,
            );
            clear_library_scan_unmatched_item(app, facet, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(None)
        }
        MovieCandidateResolution::ReadyCreated { title, .. } => {
            summary.imported += 1;
            summary.matched += 1;
            merge_default_movie_title_work(
                workset,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                true,
            );
            clear_library_scan_unmatched_item(app, facet, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(None)
        }
        MovieCandidateResolution::CreateFailed(error) => Err(error),
        MovieCandidateResolution::Skipped => {
            summary.skipped += 1;
            clear_library_scan_unmatched_item(app, facet, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(None)
        }
        MovieCandidateResolution::Unresolved(candidate) => Ok(Some(*candidate)),
    }
}

pub(super) async fn process_series_full_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedSeriesLibraryScanCandidate,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<Option<PreparedSeriesLibraryScanCandidate>> {
    let item_path = candidate.folder_path.to_string_lossy().trim().to_string();
    let folder_name = match candidate.folder_name.as_deref() {
        Some(name) => name.to_string(),
        None => {
            summary.skipped += 1;
            clear_library_scan_unmatched_item(app, facet, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
            return Ok(None);
        }
    };

    if let Some(index) = find_existing_series_title_index(
        &candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
    ) {
        merge_series_title_work_for_index(
            app,
            workset,
            existing_titles,
            index,
            &candidate.folder_path,
            LibraryScanTitleWalkMode::Full,
            false,
        )
        .await;
        summary.matched += 1;
        clear_library_scan_unmatched_item(app, facet, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    if let Some(new_title) = build_new_series_title_from_nfo(&candidate, facet) {
        match app.create_title_without_hydration(actor, new_title).await {
            Ok(created) => {
                append_series_title_and_merge_work(
                    app,
                    workset,
                    existing_titles,
                    existing_titles_by_name,
                    existing_titles_by_tvdb_id,
                    created,
                    &candidate.folder_path,
                    LibraryScanTitleWalkMode::Full,
                    true,
                )
                .await;
                summary.imported += 1;
                summary.matched += 1;
                clear_library_scan_unmatched_item(app, facet, &item_path).await?;
            }
            Err(error) => {
                let tvdb_id = candidate
                    .nfo_meta
                    .as_ref()
                    .and_then(|metadata| metadata.tvdb_id.as_deref())
                    .unwrap_or_default();
                warn!(
                    folder = %folder_name,
                    tvdb_id = %tvdb_id,
                    error = %error,
                    "series scan: failed to create title from NFO"
                );
                let unmatched_item = build_series_unmatched_scan_item(
                    facet,
                    session_id,
                    library_path,
                    &candidate,
                    &MetadataSearchResults::new(),
                    Some("title_create_from_nfo_failed"),
                    Some(error.to_string()),
                );
                persist_library_scan_unmatched_item(app, &unmatched_item).await?;
                unmatched_items.push(unmatched_item);
                summary.unmatched += 1;
            }
        }
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    if candidate.query.trim().is_empty() {
        summary.skipped += 1;
        clear_library_scan_unmatched_item(app, facet, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    Ok(Some(candidate))
}

pub(super) async fn process_resolved_movie_full_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<()> {
    let discovered_files = candidate.discovered_files.clone();
    match resolve_movie_metadata_match(
        app,
        actor,
        facet,
        &candidate,
        batch_search_results,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
    .await?
    {
        MovieMetadataResolution::Ready(title) => {
            summary.matched += 1;
            merge_default_movie_title_work(
                workset,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                false,
            );
            clear_library_scan_unmatched_item(app, facet, &candidate.file.path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
        MovieMetadataResolution::ReadyCreated { title, .. } => {
            summary.imported += 1;
            summary.matched += 1;
            merge_default_movie_title_work(
                workset,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                true,
            );
            clear_library_scan_unmatched_item(app, facet, &candidate.file.path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
        MovieMetadataResolution::CreateFailed(error) => Err(error),
        MovieMetadataResolution::Unmatched => {
            let unmatched_item = build_movie_unmatched_scan_item(
                facet,
                session_id,
                library_path,
                &candidate,
                batch_search_results,
            );
            persist_library_scan_unmatched_item(app, &unmatched_item).await?;
            unmatched_items.push(unmatched_item);
            summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
    }
}

pub(super) async fn process_resolved_series_full_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<()> {
    let Some(folder_name) = candidate.folder_name.as_deref() else {
        summary.skipped += 1;
        clear_library_scan_unmatched_item(
            app,
            facet,
            candidate.folder_path.to_string_lossy().as_ref(),
        )
        .await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(());
    };

    let selected_metadata =
        select_series_metadata_from_batch_results(&candidate, batch_search_results)?;
    let Some(selected) = selected_metadata else {
        info!(
            folder = %folder_name,
            query = %candidate.query,
            "series scan: no metadata match"
        );
        let unmatched_item = build_series_unmatched_scan_item(
            facet,
            session_id,
            library_path,
            &candidate,
            batch_search_results,
            None,
            None,
        );
        persist_library_scan_unmatched_item(app, &unmatched_item).await?;
        unmatched_items.push(unmatched_item);
        summary.unmatched += 1;
        coordinator.mark_title_match_completed(1).await;
        return Ok(());
    };

    if let Some(index) = find_existing_title_index_for_metadata_match(
        &selected,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
    ) {
        merge_series_title_work_for_index(
            app,
            workset,
            existing_titles,
            index,
            &candidate.folder_path,
            LibraryScanTitleWalkMode::Full,
            false,
        )
        .await;
        summary.matched += 1;
        clear_library_scan_unmatched_item(
            app,
            facet,
            candidate.folder_path.to_string_lossy().as_ref(),
        )
        .await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(());
    }

    match app
        .create_title_without_hydration(
            actor,
            build_new_title_from_metadata_match(facet, &selected),
        )
        .await
    {
        Ok(created) => {
            append_series_title_and_merge_work(
                app,
                workset,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                created,
                &candidate.folder_path,
                LibraryScanTitleWalkMode::Full,
                true,
            )
            .await;
            summary.imported += 1;
            summary.matched += 1;
            clear_library_scan_unmatched_item(
                app,
                facet,
                candidate.folder_path.to_string_lossy().as_ref(),
            )
            .await?;
            coordinator.mark_title_match_completed(1).await;
        }
        Err(error) => {
            warn!(
                folder = %folder_name,
                tvdb_id = %selected.tvdb_id,
                error = %error,
                "series scan: failed to create title from search"
            );
            let unmatched_item = build_series_unmatched_scan_item(
                facet,
                session_id,
                library_path,
                &candidate,
                batch_search_results,
                Some("title_create_from_search_failed"),
                Some(error.to_string()),
            );
            persist_library_scan_unmatched_item(app, &unmatched_item).await?;
            unmatched_items.push(unmatched_item);
            summary.unmatched += 1;
            coordinator.mark_title_match_completed(1).await;
        }
    }

    Ok(())
}

async fn refresh_existing_series_title_match(
    app: &AppUseCase,
    title: &mut Title,
    index: usize,
    folder_path: &Path,
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    ensure_title_folder_path_if_missing(app, title, folder_path).await;
    update_series_title_folder_path_index(existing_titles_by_folder_path, title, index);
    maybe_probe_existing_series_title_for_background_refresh(
        app,
        title,
        folder_path,
        workset,
        summary,
    )
    .await
}

pub(super) async fn process_series_refresh_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    candidate: PreparedSeriesLibraryScanCandidate,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<Option<PreparedSeriesLibraryScanCandidate>> {
    let folder_name = match candidate.folder_name.as_deref() {
        Some(value) => value.to_string(),
        None => {
            summary.skipped += 1;
            return Ok(None);
        }
    };

    if let Some(index) = find_existing_series_title_index(
        &candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
    ) {
        refresh_existing_series_title_match(
            app,
            &mut existing_titles[index],
            index,
            &candidate.folder_path,
            existing_titles_by_folder_path,
            workset,
            summary,
        )
        .await?;
        return Ok(None);
    }

    if let Some(new_title) = build_new_series_title_from_nfo(&candidate, facet) {
        match app.create_title_without_hydration(actor, new_title).await {
            Ok(created) => {
                let index = append_series_title_and_merge_work(
                    app,
                    workset,
                    existing_titles,
                    existing_titles_by_name,
                    existing_titles_by_tvdb_id,
                    created,
                    &candidate.folder_path,
                    LibraryScanTitleWalkMode::Additive,
                    true,
                )
                .await;
                update_series_title_folder_path_index(
                    existing_titles_by_folder_path,
                    &existing_titles[index],
                    index,
                );
                summary.matched += 1;
            }
            Err(error) => {
                let tvdb_id = candidate
                    .nfo_meta
                    .as_ref()
                    .and_then(|metadata| metadata.tvdb_id.as_deref())
                    .unwrap_or_default();
                warn!(
                    folder = %folder_name,
                    tvdb_id = %tvdb_id,
                    error = %error,
                    "background series refresh: failed to create title from NFO"
                );
                summary.unmatched += 1;
            }
        }
        return Ok(None);
    }

    if candidate.query.trim().is_empty() {
        summary.skipped += 1;
        return Ok(None);
    }

    Ok(Some(candidate))
}

pub(super) async fn process_resolved_series_refresh_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    candidate: PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    let Some(folder_name) = candidate.folder_name.as_deref() else {
        summary.skipped += 1;
        return Ok(());
    };

    let selected_metadata =
        select_series_metadata_from_batch_results(&candidate, batch_search_results)?;
    let Some(selected) = selected_metadata else {
        summary.unmatched += 1;
        return Ok(());
    };

    if let Some(index) = find_existing_title_index_for_metadata_match(
        &selected,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
    ) {
        refresh_existing_series_title_match(
            app,
            &mut existing_titles[index],
            index,
            &candidate.folder_path,
            existing_titles_by_folder_path,
            workset,
            summary,
        )
        .await?;
        return Ok(());
    }

    match app
        .create_title_without_hydration(
            actor,
            build_new_title_from_metadata_match(facet, &selected),
        )
        .await
    {
        Ok(created) => {
            let index = append_series_title_and_merge_work(
                app,
                workset,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                created,
                &candidate.folder_path,
                LibraryScanTitleWalkMode::Additive,
                true,
            )
            .await;
            update_series_title_folder_path_index(
                existing_titles_by_folder_path,
                &existing_titles[index],
                index,
            );
            summary.matched += 1;
        }
        Err(error) => {
            warn!(
                folder = %folder_name,
                tvdb_id = %selected.tvdb_id,
                error = %error,
                "background series refresh: failed to create title"
            );
            summary.unmatched += 1;
        }
    }

    Ok(())
}

pub(super) async fn process_movie_refresh_candidate(
    app: &AppUseCase,
    actor: &User,
    candidate: PreparedMovieLibraryScanCandidate,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    root: &Path,
    existing_titles_by_probe_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<Option<PreparedMovieLibraryScanCandidate>> {
    let representative_path = candidate.file.path.clone();
    let discovered_files = candidate.discovered_files.clone();

    match resolve_movie_scan_candidate(
        app,
        actor,
        &MediaFacet::Movie,
        candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
    .await?
    {
        MovieCandidateResolution::Ready(title) => {
            merge_default_movie_title_work(
                workset,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                false,
            );
            summary.matched += 1;
            Ok(None)
        }
        MovieCandidateResolution::ReadyCreated { index, title } => {
            update_movie_probe_path_index(
                existing_titles_by_probe_path,
                root,
                &representative_path,
                index,
            );
            merge_default_movie_title_work(
                workset,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                true,
            );
            summary.imported += 1;
            summary.matched += 1;
            Ok(None)
        }
        MovieCandidateResolution::CreateFailed(error) => {
            warn!(
                path = %representative_path,
                error = %error,
                "background movie refresh: failed to create title from NFO"
            );
            summary.unmatched += 1;
            Ok(None)
        }
        MovieCandidateResolution::Skipped => {
            summary.skipped += 1;
            Ok(None)
        }
        MovieCandidateResolution::Unresolved(candidate) => Ok(Some(*candidate)),
    }
}

pub(super) async fn process_resolved_movie_refresh_candidate(
    app: &AppUseCase,
    actor: &User,
    candidate: PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    root: &Path,
    existing_titles_by_probe_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    let representative_path = candidate.file.path.clone();
    let discovered_files = candidate.discovered_files.clone();
    match resolve_movie_metadata_match(
        app,
        actor,
        &MediaFacet::Movie,
        &candidate,
        batch_search_results,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
    .await?
    {
        MovieMetadataResolution::Ready(title) => {
            merge_default_movie_title_work(
                workset,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                false,
            );
            summary.matched += 1;
            Ok(())
        }
        MovieMetadataResolution::ReadyCreated { index, title } => {
            update_movie_probe_path_index(
                existing_titles_by_probe_path,
                root,
                &representative_path,
                index,
            );
            merge_default_movie_title_work(
                workset,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                true,
            );
            summary.imported += 1;
            summary.matched += 1;
            Ok(())
        }
        MovieMetadataResolution::CreateFailed(error) => {
            warn!(
                path = %representative_path,
                error = %error,
                "background movie refresh: failed to create title"
            );
            summary.unmatched += 1;
            Ok(())
        }
        MovieMetadataResolution::Unmatched => {
            summary.unmatched += 1;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct CountingLibraryScanner {
        metrics_calls: Arc<Mutex<Vec<String>>>,
        progress_calls: Arc<Mutex<Vec<String>>>,
    }

    impl CountingLibraryScanner {
        fn metrics_call_count(&self) -> usize {
            self.metrics_calls.lock().unwrap().len()
        }

        fn progress_call_count(&self) -> usize {
            self.progress_calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl LibraryScanner for CountingLibraryScanner {
        async fn scan_library(&self, _root: &str) -> AppResult<Vec<LibraryFile>> {
            panic!("unused in test")
        }

        async fn scan_library_batched(
            &self,
            _root: &str,
            _batch_size: usize,
        ) -> AppResult<LibraryFileBatchReceiver> {
            panic!("unused in test")
        }

        async fn scan_directory_batched(
            &self,
            _root: &str,
            _batch_size: usize,
        ) -> AppResult<LibraryFileBatchReceiver> {
            panic!("unused in test")
        }

        async fn scan_directory_with_metrics(
            &self,
            root: &str,
        ) -> AppResult<LibraryDirectoryScanResult> {
            self.metrics_calls.lock().unwrap().push(root.to_string());
            Ok(LibraryDirectoryScanResult {
                files: vec![build_library_file(&format!("{root}/Episode.mkv"))],
                walk_ms: 1,
                stat_ms: 1,
                elapsed_ms: 2,
            })
        }

        async fn scan_directory_for_progress_with_metrics(
            &self,
            root: &str,
        ) -> AppResult<LibraryDirectoryScanResult> {
            self.progress_calls.lock().unwrap().push(root.to_string());
            Ok(LibraryDirectoryScanResult {
                files: vec![build_library_file(&format!("{root}/Episode.mkv"))],
                walk_ms: 1,
                stat_ms: 0,
                elapsed_ms: 1,
            })
        }
    }

    fn build_library_file(path: &str) -> LibraryFile {
        LibraryFile {
            path: path.to_string(),
            display_name: Path::new(path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }
    }

    #[tokio::test]
    async fn scan_episodic_title_directory_for_progress_metrics_uses_progress_scan_path() {
        let scanner = CountingLibraryScanner::default();
        let folder_path = Path::new("/library/Show");

        let result = scan_episodic_title_directory_for_progress_metrics(
            Arc::new(scanner.clone()),
            folder_path,
        )
        .await
        .expect("scan episodic title directory");

        assert_eq!(scanner.progress_call_count(), 1);
        assert_eq!(scanner.metrics_call_count(), 0);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "/library/Show/Episode.mkv");
        assert!(result.files[0].source_signature_scheme.is_none());
        assert!(result.files[0].source_signature_value.is_none());
    }
}
