use super::*;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

async fn ensure_title_folder_path_if_missing(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) {
    let folder_path = path_to_stored_string(folder_path).trim().to_string();
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

fn normalize_title_folder_path(path: Option<String>) -> Option<String> {
    path.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn scanned_movie_entry_folder_path(scan_root: &Path, representative_path: &str) -> Option<String> {
    let representative_path = representative_path.trim();
    if representative_path.is_empty() {
        return None;
    }

    let item_path = stored_path_to_path_buf(representative_path);
    if let Ok(relative) = item_path.strip_prefix(scan_root)
        && let Some(first_component) = relative.components().next()
    {
        let entry_path = scan_root.join(first_component.as_os_str());
        let entry_path = path_to_stored_string(&entry_path).trim().to_string();
        if entry_path.is_empty() || entry_path == representative_path {
            return None;
        }
        return Some(entry_path);
    }

    let parent = path_to_stored_string(item_path.parent()?)
        .trim()
        .to_string();
    if parent.is_empty() || parent == path_to_stored_string(scan_root) {
        None
    } else {
        Some(parent)
    }
}

async fn sync_movie_title_folder_path_for_scan(
    app: &AppUseCase,
    title: &mut Title,
    scan_root: &Path,
    representative_path: &str,
) -> (Option<String>, Option<String>) {
    let scan_folder_path = scanned_movie_entry_folder_path(scan_root, representative_path);
    let current_folder_path = normalize_title_folder_path(title.folder_path.clone());

    if current_folder_path.is_some() {
        return (current_folder_path, scan_folder_path);
    }

    let Some(folder_path) = scan_folder_path.as_deref() else {
        return (None, scan_folder_path);
    };

    match app
        .services
        .catalog
        .titles
        .set_folder_path(&title.id, folder_path)
        .await
    {
        Ok(()) => {
            title.folder_path = Some(folder_path.to_string());
        }
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                representative_path = %representative_path,
                "failed to synchronize movie title folder path during library scan"
            );
        }
    }

    (
        normalize_title_folder_path(title.folder_path.clone()),
        scan_folder_path,
    )
}

fn sync_existing_title_folder_path_in_memory(existing_titles: &mut [Title], title: &Title) {
    if let Some(existing) = existing_titles
        .iter_mut()
        .find(|existing| existing.id == title.id)
    {
        existing.folder_path = title.folder_path.clone();
    }
}

fn find_existing_movie_title_index(
    candidate: &PreparedMovieLibraryScanCandidate,
    existing_titles: &[Title],
    existing_titles_by_name: &HashMap<String, usize>,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
    existing_titles_by_imdb_id: &HashMap<String, usize>,
    existing_titles_by_tmdb_id: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(identity_hint) = candidate
        .identity_hint
        .as_ref()
        .filter(|hint| hint.is_external_import_hint())
    {
        if let Some(tmdb_id) = identity_hint.tmdb_id.as_deref()
            && let Some(&index) = existing_titles_by_tmdb_id.get(tmdb_id)
        {
            return Some(index);
        }
        if let Some(imdb_id) = identity_hint.imdb_id.as_deref()
            && let Some(&index) = existing_titles_by_imdb_id.get(imdb_id)
        {
            return Some(index);
        }
        if let Some(tvdb_id) = identity_hint.tvdb_id.as_deref()
            && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
        {
            return Some(index);
        }
        return None;
    }

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

    if let Some(parsed_tmdb_id) = candidate.parsed_release.tmdb_id.as_deref()
        && let Some(&index) = existing_titles_by_tmdb_id.get(parsed_tmdb_id)
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
    existing_titles_by_imdb_id: &HashMap<String, usize>,
    existing_titles_by_tmdb_id: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(identity_hint) = candidate
        .identity_hint
        .as_ref()
        .filter(|hint| hint.is_external_import_hint())
    {
        if let Some(tvdb_id) = identity_hint.tvdb_id.as_deref()
            && let Some(&index) = existing_titles_by_tvdb_id.get(tvdb_id)
        {
            return Some(index);
        }
        if let Some(imdb_id) = identity_hint.imdb_id.as_deref()
            && let Some(&index) = existing_titles_by_imdb_id.get(imdb_id)
        {
            return Some(index);
        }
        if let Some(tmdb_id) = identity_hint.tmdb_id.as_deref()
            && let Some(&index) = existing_titles_by_tmdb_id.get(tmdb_id)
        {
            return Some(index);
        }
        return None;
    }

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

async fn load_existing_title_for_media_file_path(
    app: &AppUseCase,
    file_path: &str,
) -> AppResult<Option<Title>> {
    let Some(existing_media_file) = app
        .services
        .library
        .media_files
        .get_media_file_by_path(file_path)
        .await?
    else {
        return Ok(None);
    };

    app.services
        .catalog
        .titles
        .get_by_id(&existing_media_file.title_id)
        .await
}

enum MovieCandidateResolution {
    Ready(Box<Title>),
    Skipped,
    Unresolved(Box<PreparedMovieLibraryScanCandidate>),
}

enum MovieMetadataResolution {
    Ready(Title),
    ReadyCreated { index: usize, title: Title },
    CreateFailed(AppError),
    Unmatched,
}

async fn create_title_without_hydration_for_library_scan(
    app: &AppUseCase,
    actor: &User,
    library_id: &str,
    request: NewTitle,
) -> AppResult<CreateTitleOutcome> {
    app.create_title_without_hydration_after_library_authorization(
        actor,
        request,
        library_id.to_string(),
    )
    .await
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

fn movie_cleanup_context(
    canonical_folder_path: Option<String>,
    scan_folder_path: Option<String>,
) -> LibraryScanMovieCleanupContext {
    LibraryScanMovieCleanupContext {
        canonical_folder_path,
        scan_folder_path,
        ..Default::default()
    }
}

fn merge_default_movie_title_work(
    executor: &mut LibraryScanTitleWorkExecutor,
    title: Title,
    discovered_files: Vec<LibraryFile>,
    mode: LibraryScanTitleWalkMode,
    cleanup: LibraryScanMovieCleanupContext,
    created_in_scan: bool,
) -> bool {
    executor.enqueue(movie_title_work(
        title,
        discovered_files,
        mode,
        cleanup,
        created_in_scan,
    ))
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

fn deferred_episodic_title_work(
    title: Title,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) -> LibraryScanTitleWork {
    LibraryScanTitleWork {
        title,
        facet_plan: LibraryScanTitleFacetPlan::Episodic,
        discovered_files: None,
        mode,
        created_in_scan,
    }
}

pub(super) async fn scan_episodic_title_directory_for_progress_metrics(
    library_scanner: Arc<dyn LibraryScanner>,
    folder_path: &Path,
) -> AppResult<LibraryDirectoryScanResult> {
    library_scanner
        .scan_directory_for_progress_with_metrics(path_to_stored_string(folder_path).as_str())
        .await
}

async fn merge_series_title_work_for_index(
    app: &AppUseCase,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut [Title],
    index: usize,
    folder_path: &Path,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) {
    ensure_title_folder_path_if_missing(app, &mut existing_titles[index], folder_path).await;
    executor.enqueue(deferred_episodic_title_work(
        existing_titles[index].clone(),
        mode,
        created_in_scan,
    ));
}

#[expect(
    clippy::too_many_arguments,
    reason = "series title insertion updates shared indexes and executor state together"
)]
async fn append_series_title_and_merge_work(
    app: &AppUseCase,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    title: Title,
    folder_path: &Path,
    mode: LibraryScanTitleWalkMode,
    created_in_scan: bool,
) -> usize {
    let index = append_series_title(
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
        title,
    );
    merge_series_title_work_for_index(
        app,
        executor,
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
    candidate: PreparedMovieLibraryScanCandidate,
    existing_titles: &mut [Title],
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
        return Ok(MovieCandidateResolution::Ready(Box::new(
            existing_titles[index].clone(),
        )));
    }

    if !candidate.metadata_lookup_attempted {
        return Ok(MovieCandidateResolution::Skipped);
    }

    Ok(MovieCandidateResolution::Unresolved(Box::new(candidate)))
}

#[expect(
    clippy::too_many_arguments,
    reason = "metadata matches update the same in-memory title indexes and creation context together"
)]
async fn resolve_movie_metadata_match(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
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

    match create_title_without_hydration_for_library_scan(
        app,
        actor,
        library_id,
        build_new_title_from_metadata_match(facet, &selected),
    )
    .await
    {
        Ok(created) => {
            let was_created = !created.reused_existing;
            let created_title = created.title;
            let index = append_movie_title(
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                created_title.clone(),
            );
            Ok(if was_created {
                MovieMetadataResolution::ReadyCreated {
                    index,
                    title: created_title,
                }
            } else {
                MovieMetadataResolution::Ready(created_title)
            })
        }
        Err(error) => Ok(MovieMetadataResolution::CreateFailed(error)),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "movie full-scan processing coordinates shared scan state across a single candidate"
)]
pub(super) async fn process_movie_full_scan_candidate(
    app: &AppUseCase,
    _actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    _session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedMovieLibraryScanCandidate,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut [Title],
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    _unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<Option<PreparedMovieLibraryScanCandidate>> {
    let discovered_files = candidate.discovered_files.clone();
    let item_path = normalize_library_scan_item_path(&candidate.file.path);
    let representative_path = candidate.file.path.clone();
    let scan_root = Path::new(library_path);

    if let Some(mut title) =
        load_existing_title_for_media_file_path(app, &candidate.file.path).await?
    {
        let (canonical_folder_path, scan_folder_path) =
            sync_movie_title_folder_path_for_scan(app, &mut title, scan_root, &representative_path)
                .await;
        sync_existing_title_folder_path_in_memory(existing_titles, &title);
        let queued = merge_default_movie_title_work(
            executor,
            title,
            discovered_files,
            LibraryScanTitleWalkMode::Full,
            movie_cleanup_context(canonical_folder_path, scan_folder_path),
            false,
        );
        if queued {
            summary.matched += 1;
        } else {
            summary.skipped += 1;
        }
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    match resolve_movie_scan_candidate(
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
            let mut title = *title;
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                scan_root,
                &representative_path,
            )
            .await;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                false,
            );
            if queued {
                summary.matched += 1;
            } else {
                summary.skipped += 1;
            }
            clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(None)
        }
        MovieCandidateResolution::Skipped => {
            summary.skipped += 1;
            clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(None)
        }
        MovieCandidateResolution::Unresolved(candidate) => Ok(Some(*candidate)),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "series full-scan processing coordinates shared scan state across a single candidate"
)]
pub(super) async fn process_series_full_scan_candidate(
    app: &AppUseCase,
    _actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    _library_path: &str,
    _session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedSeriesLibraryScanCandidate,
    existing_titles: &mut [Title],
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    executor: &mut LibraryScanTitleWorkExecutor,
    summary: &mut LibraryScanSummary,
    _unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<Option<PreparedSeriesLibraryScanCandidate>> {
    let item_path = candidate.item_path().trim().to_string();
    if candidate.folder_name.as_deref().is_none() {
        summary.skipped += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    if let Some(file) = candidate.source_file.as_ref()
        && let Some(title) = load_existing_title_for_media_file_path(app, &file.path).await?
    {
        executor.enqueue(episodic_title_work(
            title,
            vec![file.clone()],
            LibraryScanTitleWalkMode::Full,
            false,
        ));
        summary.matched += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    if let Some(index) = find_existing_series_title_index(
        &candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    ) {
        if let Some(file) = candidate.source_file.as_ref() {
            executor.enqueue(episodic_title_work(
                existing_titles[index].clone(),
                vec![file.clone()],
                LibraryScanTitleWalkMode::Full,
                false,
            ));
        } else {
            merge_series_title_work_for_index(
                app,
                executor,
                existing_titles,
                index,
                &candidate.folder_path,
                LibraryScanTitleWalkMode::Full,
                false,
            )
            .await;
        }
        summary.matched += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    if !candidate.metadata_lookup_attempted {
        summary.skipped += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(None);
    }

    Ok(Some(candidate))
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved movie scan candidates update shared scan state, indexes, and reporting together"
)]
pub(super) async fn process_resolved_movie_full_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<()> {
    let discovered_files = candidate.discovered_files.clone();
    let scan_root = Path::new(library_path);
    match resolve_movie_metadata_match(
        app,
        actor,
        facet,
        library_id,
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
        MovieMetadataResolution::Ready(mut title) => {
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                scan_root,
                &candidate.file.path,
            )
            .await;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                false,
            );
            if queued {
                summary.matched += 1;
            } else {
                summary.skipped += 1;
            }
            clear_library_scan_unmatched_item(app, facet, library_id, &candidate.file.path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
        MovieMetadataResolution::ReadyCreated { mut title, .. } => {
            let (canonical_folder_path, scan_folder_path) = sync_movie_title_folder_path_for_scan(
                app,
                &mut title,
                scan_root,
                &candidate.file.path,
            )
            .await;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Full,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                true,
            );
            if queued {
                summary.imported += 1;
                summary.matched += 1;
            } else {
                summary.skipped += 1;
            }
            clear_library_scan_unmatched_item(app, facet, library_id, &candidate.file.path).await?;
            coordinator.mark_title_match_completed(1).await;
            Ok(())
        }
        MovieMetadataResolution::CreateFailed(error) => {
            warn!(
                file = %candidate.file.path,
                query = %candidate.query,
                error = %error,
                "movie scan: failed to create title from search result"
            );
            let unmatched_item = build_movie_unmatched_scan_item(
                facet,
                library_id,
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
            Ok(())
        }
        MovieMetadataResolution::Unmatched => {
            let unmatched_item = build_movie_unmatched_scan_item(
                facet,
                library_id,
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
            Ok(())
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved series scan candidates update shared scan state, indexes, and reporting together"
)]
pub(super) async fn process_resolved_series_full_scan_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
    session_id: &str,
    coordinator: &LibraryScanCoordinator,
    candidate: PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
    unmatched_items: &mut Vec<LibraryScanUnmatchedItem>,
) -> AppResult<()> {
    let item_path = candidate.item_path().trim().to_string();
    let Some(folder_name) = candidate.folder_name.as_deref() else {
        summary.skipped += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
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
            library_id,
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
        if let Some(file) = candidate.source_file.as_ref() {
            executor.enqueue(episodic_title_work(
                existing_titles[index].clone(),
                vec![file.clone()],
                LibraryScanTitleWalkMode::Full,
                false,
            ));
        } else {
            merge_series_title_work_for_index(
                app,
                executor,
                existing_titles,
                index,
                &candidate.folder_path,
                LibraryScanTitleWalkMode::Full,
                false,
            )
            .await;
        }
        summary.matched += 1;
        clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
        coordinator.mark_title_match_completed(1).await;
        return Ok(());
    }

    if candidate.source_file.is_some() {
        let unmatched_item = build_series_unmatched_scan_item(
            facet,
            library_id,
            session_id,
            library_path,
            &candidate,
            batch_search_results,
            Some("title_not_in_catalog"),
            None,
        );
        persist_library_scan_unmatched_item(app, &unmatched_item).await?;
        unmatched_items.push(unmatched_item);
        summary.unmatched += 1;
        coordinator.mark_title_match_completed(1).await;
        return Ok(());
    }

    match create_title_without_hydration_for_library_scan(
        app,
        actor,
        library_id,
        build_new_title_from_metadata_match(facet, &selected),
    )
    .await
    {
        Ok(created) => {
            let was_created = !created.reused_existing;
            append_series_title_and_merge_work(
                app,
                executor,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                created.title,
                &candidate.folder_path,
                LibraryScanTitleWalkMode::Full,
                was_created,
            )
            .await;
            if was_created {
                summary.imported += 1;
            }
            summary.matched += 1;
            clear_library_scan_unmatched_item(app, facet, library_id, &item_path).await?;
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
                library_id,
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
    executor: &mut LibraryScanTitleWorkExecutor,
    summary: &mut LibraryScanSummary,
) -> AppResult<()> {
    ensure_title_folder_path_if_missing(app, title, folder_path).await;
    update_series_title_folder_path_index(existing_titles_by_folder_path, title, index);
    maybe_probe_existing_series_title_for_background_refresh(
        app,
        title,
        folder_path,
        executor,
        summary,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "series refresh candidates need shared title indexes and executor state in one step"
)]
pub(super) async fn process_series_refresh_candidate(
    app: &AppUseCase,
    candidate: PreparedSeriesLibraryScanCandidate,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut [Title],
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    summary: &mut LibraryScanSummary,
) -> AppResult<Option<PreparedSeriesLibraryScanCandidate>> {
    if candidate.folder_name.as_deref().is_none() {
        summary.skipped += 1;
        return Ok(None);
    }

    if let Some(index) = find_existing_series_title_index(
        &candidate,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    ) {
        refresh_existing_series_title_match(
            app,
            &mut existing_titles[index],
            index,
            &candidate.folder_path,
            existing_titles_by_folder_path,
            executor,
            summary,
        )
        .await?;
        return Ok(None);
    }

    if candidate.query.trim().is_empty() {
        summary.skipped += 1;
        return Ok(None);
    }

    Ok(Some(candidate))
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved series refresh candidates update indexes and background work in one place"
)]
pub(super) async fn process_resolved_series_refresh_candidate(
    app: &AppUseCase,
    actor: &User,
    facet: &MediaFacet,
    library_id: &str,
    candidate: PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
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
            executor,
            summary,
        )
        .await?;
        return Ok(());
    }

    match create_title_without_hydration_for_library_scan(
        app,
        actor,
        library_id,
        build_new_title_from_metadata_match(facet, &selected),
    )
    .await
    {
        Ok(created) => {
            let was_created = !created.reused_existing;
            let index = append_series_title_and_merge_work(
                app,
                executor,
                existing_titles,
                existing_titles_by_name,
                existing_titles_by_tvdb_id,
                existing_titles_by_imdb_id,
                existing_titles_by_tmdb_id,
                created.title,
                &candidate.folder_path,
                LibraryScanTitleWalkMode::Additive,
                was_created,
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

#[expect(
    clippy::too_many_arguments,
    reason = "movie refresh candidates need shared indexes, probe paths, and executor state together"
)]
pub(super) async fn process_movie_refresh_candidate(
    app: &AppUseCase,
    _actor: &User,
    _library_id: &str,
    candidate: PreparedMovieLibraryScanCandidate,
    executor: &mut LibraryScanTitleWorkExecutor,
    existing_titles: &mut [Title],
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
            let mut title = *title;
            let (canonical_folder_path, scan_folder_path) =
                sync_movie_title_folder_path_for_scan(app, &mut title, root, &representative_path)
                    .await;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            if let Some(index) = existing_titles
                .iter()
                .position(|existing| existing.id == title.id)
            {
                update_movie_probe_path_index(
                    existing_titles_by_probe_path,
                    root,
                    &representative_path,
                    index,
                );
            }
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                false,
            );
            if queued {
                summary.matched += 1;
            } else {
                summary.skipped += 1;
            }
            Ok(None)
        }
        MovieCandidateResolution::Skipped => {
            summary.skipped += 1;
            Ok(None)
        }
        MovieCandidateResolution::Unresolved(candidate) => Ok(Some(*candidate)),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "resolved movie refresh candidates update indexes and background work in one place"
)]
pub(super) async fn process_resolved_movie_refresh_candidate(
    app: &AppUseCase,
    actor: &User,
    library_id: &str,
    candidate: PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    executor: &mut LibraryScanTitleWorkExecutor,
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
        library_id,
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
        MovieMetadataResolution::Ready(mut title) => {
            let (canonical_folder_path, scan_folder_path) =
                sync_movie_title_folder_path_for_scan(app, &mut title, root, &representative_path)
                    .await;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            if let Some(index) = existing_titles
                .iter()
                .position(|existing| existing.id == title.id)
            {
                update_movie_probe_path_index(
                    existing_titles_by_probe_path,
                    root,
                    &representative_path,
                    index,
                );
            }
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                false,
            );
            if queued {
                summary.matched += 1;
            } else {
                summary.skipped += 1;
            }
            Ok(())
        }
        MovieMetadataResolution::ReadyCreated { index, mut title } => {
            let (canonical_folder_path, scan_folder_path) =
                sync_movie_title_folder_path_for_scan(app, &mut title, root, &representative_path)
                    .await;
            sync_existing_title_folder_path_in_memory(existing_titles, &title);
            update_movie_probe_path_index(
                existing_titles_by_probe_path,
                root,
                &representative_path,
                index,
            );
            let queued = merge_default_movie_title_work(
                executor,
                title,
                discovered_files,
                LibraryScanTitleWalkMode::Additive,
                movie_cleanup_context(canonical_folder_path, scan_folder_path),
                true,
            );
            if queued {
                summary.imported += 1;
                summary.matched += 1;
            } else {
                summary.skipped += 1;
            }
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
    use chrono::Utc;
    use scryer_domain::MediaFacet;
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

    fn build_series_title(id: &str) -> Title {
        Title {
            id: id.to_string(),
            library_id: "library".to_string(),
            name: "Test Series".to_string(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: Vec::new(),
            external_ids: Vec::new(),
            root_folder_id: "root".to_string(),
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: "test series".to_string(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: Vec::new(),
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn series_candidate_with_nfo_ids(
        nfo_meta: crate::nfo::NfoMetadata,
    ) -> PreparedSeriesLibraryScanCandidate {
        PreparedSeriesLibraryScanCandidate {
            folder_path: std::path::PathBuf::from("/library/Show"),
            folder_name: Some("Show".to_string()),
            source_file: None,
            nfo_meta: Some(nfo_meta),
            identity_hint: None,
            query: String::new(),
            year_hint: None,
            search_candidates: Vec::new(),
            title_match_candidates: Vec::new(),
            metadata_lookup_attempted: true,
        }
    }

    #[test]
    fn find_existing_series_title_index_resolves_via_imdb_and_tmdb() {
        let mut imdb_title = build_series_title("series-imdb");
        imdb_title.external_ids = vec![scryer_domain::ExternalId {
            source: "imdb".to_string(),
            value: "tt2222222".to_string(),
        }];
        let mut tmdb_title = build_series_title("series-tmdb");
        tmdb_title.external_ids = vec![scryer_domain::ExternalId {
            source: "tmdb".to_string(),
            value: "55555".to_string(),
        }];
        let existing_titles = vec![imdb_title, tmdb_title];
        let (by_name, by_tvdb, by_imdb, by_tmdb) = build_series_title_indexes(&existing_titles);

        // A re-scanned series whose tvdb isn't locally indexed still resolves via
        // its NFO imdb/tmdb id, mirroring the movie scan (no SMG round-trip).
        let imdb_candidate = series_candidate_with_nfo_ids(crate::nfo::NfoMetadata {
            imdb_id: Some("tt2222222".to_string()),
            ..Default::default()
        });
        assert_eq!(
            find_existing_series_title_index(
                &imdb_candidate,
                &existing_titles,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(0)
        );

        let tmdb_candidate = series_candidate_with_nfo_ids(crate::nfo::NfoMetadata {
            tmdb_id: Some("55555".to_string()),
            ..Default::default()
        });
        assert_eq!(
            find_existing_series_title_index(
                &tmdb_candidate,
                &existing_titles,
                &by_name,
                &by_tvdb,
                &by_imdb,
                &by_tmdb,
            ),
            Some(1)
        );
    }

    #[test]
    fn deferred_episodic_title_work_requests_full_file_walk() {
        let work = deferred_episodic_title_work(
            build_series_title("title-1"),
            LibraryScanTitleWalkMode::Full,
            false,
        );

        assert!(work.discovered_files.is_none());
    }

    #[test]
    fn merge_library_scan_title_work_preserves_full_walk_requirement() {
        let title = build_series_title("title-1");
        let mut workset = HashMap::new();

        assert!(merge_library_scan_title_work(
            &mut workset,
            episodic_title_work(
                title.clone(),
                vec![build_library_file("/library/Show/loose.mkv")],
                LibraryScanTitleWalkMode::Full,
                false,
            ),
        ));
        assert_eq!(
            workset
                .get("title-1")
                .and_then(|work| work.discovered_files.as_ref())
                .map(Vec::len),
            Some(1),
        );

        assert!(merge_library_scan_title_work(
            &mut workset,
            deferred_episodic_title_work(title.clone(), LibraryScanTitleWalkMode::Full, false),
        ));
        assert!(
            workset
                .get("title-1")
                .expect("merged title work")
                .discovered_files
                .is_none()
        );

        assert!(merge_library_scan_title_work(
            &mut workset,
            episodic_title_work(
                title,
                vec![build_library_file("/library/Show/another.mkv")],
                LibraryScanTitleWalkMode::Full,
                false,
            ),
        ));
        assert!(
            workset
                .get("title-1")
                .expect("merged title work")
                .discovered_files
                .is_none()
        );
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
