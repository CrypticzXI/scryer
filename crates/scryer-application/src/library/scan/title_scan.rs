use super::*;
use crate::library_scan_unmatched::build_title_bound_unmatched_scan_item;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use scryer_domain::VIDEO_EXTENSIONS;

fn parsed_release_has_title_scan_episode_identity(
    parsed: &crate::ParsedReleaseMetadata,
    facet: &scryer_domain::MediaFacet,
) -> bool {
    matches!(
        parsed.episode.as_ref(),
        Some(ep)
            if !ep.episode_numbers.is_empty()
                || ep.air_date.is_some()
                || !ep.special_absolute_episode_numbers.is_empty()
                || (ep.absolute_episode.is_some()
                    && *facet == scryer_domain::MediaFacet::Anime)
    )
}

fn parse_release_from_immediate_parent_for_title_scan(
    source_path: &std::path::Path,
    parse_context: &crate::ReleaseParseContext,
) -> Option<crate::ParsedReleaseMetadata> {
    source_path
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.trim().is_empty())
        .map(|name| crate::parse_release_metadata_for_target(name.as_str(), parse_context))
}

async fn immediate_parent_has_single_video_file(source_path: &std::path::Path) -> bool {
    let Some(parent) = source_path.parent() else {
        return false;
    };

    let mut entries = match tokio::fs::read_dir(parent).await {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    let mut video_count = 0usize;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let Ok(file_type) = entry.file_type().await else {
            return false;
        };
        if !file_type.is_file() {
            continue;
        }

        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if VIDEO_EXTENSIONS
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(extension))
        {
            video_count += 1;
            if video_count > 1 {
                return false;
            }
        }
    }

    video_count == 1
}

fn fill_missing_title_scan_release_metadata(
    target: &mut crate::ParsedReleaseMetadata,
    fallback: &crate::ParsedReleaseMetadata,
    facet: &scryer_domain::MediaFacet,
) {
    if !parsed_release_has_title_scan_episode_identity(target, facet) && fallback.episode.is_some()
    {
        target.episode = fallback.episode.clone();
    }
    if target.imdb_id.is_none() {
        target.imdb_id = fallback.imdb_id.clone();
    }
    if target.tmdb_id.is_none() {
        target.tmdb_id = fallback.tmdb_id.clone();
    }
    if target.year.is_none() {
        target.year = fallback.year;
    }
    if target.quality.is_none() {
        target.quality = fallback.quality.clone();
    }
    if target.source.is_none() {
        target.source = fallback.source;
    }
    if target.video_codec.is_none() {
        target.video_codec = fallback.video_codec;
    }
    if target.video_encoding.is_none() {
        target.video_encoding = fallback.video_encoding.clone();
    }
    if target.audio.is_none() {
        target.audio = fallback.audio;
    }
    if target.audio_channels.is_none() {
        target.audio_channels = fallback.audio_channels.clone();
    }
    if target.release_group.is_none() {
        target.release_group = fallback.release_group.clone();
    }
    if target.streaming_service.is_none() {
        target.streaming_service = fallback.streaming_service;
    }
    if target.edition.is_none() {
        target.edition = fallback.edition.clone();
    }
    if target.normalized_title.trim().is_empty() && !fallback.normalized_title.trim().is_empty() {
        target.normalized_title = fallback.normalized_title.clone();
    }
    if target.normalized_title_variants.is_empty() && !fallback.normalized_title_variants.is_empty()
    {
        target.normalized_title_variants = fallback.normalized_title_variants.clone();
    }
}

async fn parse_title_scan_release_metadata(
    source_path: &std::path::Path,
    display_name: &str,
    facet: &scryer_domain::MediaFacet,
    parse_context: &crate::ReleaseParseContext,
) -> crate::ParsedReleaseMetadata {
    let mut parsed = crate::parse_release_metadata_for_target(
        source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(display_name),
        parse_context,
    );
    if parsed_release_has_title_scan_episode_identity(&parsed, facet) {
        return parsed;
    }

    let Some(parent_release) =
        parse_release_from_immediate_parent_for_title_scan(source_path, parse_context)
    else {
        return parsed;
    };
    let Some(parent_episode) = parent_release.episode.as_ref() else {
        return parsed;
    };
    if parent_episode.full_season
        || !parsed_release_has_title_scan_episode_identity(&parent_release, facet)
        || !immediate_parent_has_single_video_file(source_path).await
    {
        return parsed;
    }

    fill_missing_title_scan_release_metadata(&mut parsed, &parent_release, facet);
    parsed
}

fn hydration_source_for_scan_mode(
    mode: LibraryScanMode,
) -> crate::catalog_workflow::HydrationSource {
    match mode {
        LibraryScanMode::Full => crate::catalog_workflow::HydrationSource::LibraryScanFull,
        LibraryScanMode::Additive => crate::catalog_workflow::HydrationSource::LibraryScanAdditive,
    }
}

pub(super) async fn title_requires_scan_hydration(
    app: &AppUseCase,
    title: &Title,
    metadata_language: &str,
) -> AppResult<bool> {
    if !title
        .external_ids
        .iter()
        .any(|external_id| external_id.source.eq_ignore_ascii_case("tvdb"))
    {
        return Ok(false);
    }

    if title.metadata_fetched_at.is_none()
        || title.metadata_language.as_deref() != Some(metadata_language)
    {
        return Ok(true);
    }

    let Some(handler) = app.facet_registry.get(&title.facet) else {
        return Ok(false);
    };
    if !handler.has_episodes() {
        return Ok(false);
    }

    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await?;
    Ok(episodes.is_empty())
}

async fn discover_movie_title_files(
    app: &AppUseCase,
    title: &Title,
) -> AppResult<Vec<LibraryFile>> {
    let import_paths = crate::import_workflow::resolve_import_paths(app, title).await?;
    let media_root_path = PathBuf::from(&import_paths.media_root);
    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .unwrap_or_default();
    let mut discovered_files = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut candidate_paths = Vec::<PathBuf>::new();
    let mut seen_candidate_paths = HashSet::new();

    for collection in collections {
        let Some(ordered_path) = collection.ordered_path else {
            continue;
        };
        let ordered_path_buf = stored_path_to_path_buf(&ordered_path);
        if let Some(parent) = ordered_path_buf.parent()
            && parent != media_root_path.as_path()
            && seen_candidate_paths.insert(path_to_stored_string(parent))
        {
            candidate_paths.push(parent.to_path_buf());
        }
        if !seen_paths.insert(ordered_path.clone()) {
            continue;
        }

        match tokio::fs::metadata(&ordered_path_buf).await {
            Ok(metadata) if metadata.is_file() => {}
            Ok(metadata) if metadata.is_dir() => {
                if seen_candidate_paths.insert(ordered_path.clone()) {
                    candidate_paths.push(ordered_path_buf);
                }
                continue;
            }
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %ordered_path,
                    "failed to inspect tracked movie path during title scan discovery"
                );
                continue;
            }
        }

        discovered_files.push(LibraryFile {
            path: ordered_path.clone(),
            display_name: ordered_path_buf
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: matching_movie_nfo_path(&ordered_path_buf),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        });
    }

    if !discovered_files.is_empty() {
        return Ok(discovered_files);
    }

    let default_candidate_path = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(stored_path_to_path_buf)
        .unwrap_or_else(|| {
            crate::effective_title_folder_path(
                &import_paths.media_root,
                title,
                &import_paths.folder_template,
                None,
            )
        });
    if default_candidate_path != media_root_path
        && seen_candidate_paths.insert(path_to_stored_string(&default_candidate_path))
    {
        candidate_paths.push(default_candidate_path);
    }

    for candidate_path in candidate_paths {
        match tokio::fs::metadata(&candidate_path).await {
            Ok(metadata) if metadata.is_dir() => {
                let files = app
                    .services
                    .library
                    .library_scanner
                    .scan_library(path_to_stored_string(&candidate_path).as_str())
                    .await?;
                for file in files {
                    if seen_paths.insert(file.path.clone()) {
                        discovered_files.push(file);
                    }
                }
                if !discovered_files.is_empty() {
                    return Ok(discovered_files);
                }
            }
            Ok(metadata) if metadata.is_file() => {
                return Ok(vec![LibraryFile {
                    path: path_to_stored_string(&candidate_path),
                    display_name: candidate_path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    nfo_path: matching_movie_nfo_path(&candidate_path),
                    size_bytes: None,
                    source_signature_scheme: None,
                    source_signature_value: None,
                }]);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => warn!(
                error = %error,
                title_id = %title.id,
                path = %candidate_path.display(),
                "failed to inspect movie scan candidate path"
            ),
        }
    }

    Ok(Vec::new())
}

async fn tracked_movie_path_confirmed_missing(path: &Path) -> bool {
    match tokio::fs::metadata(path).await {
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(parent) = path.parent() else {
                return false;
            };
            tokio::fs::metadata(parent)
                .await
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        }
        Err(error) => {
            warn!(
                error = %error,
                path = %path.display(),
                "failed to inspect tracked movie path during stale cleanup"
            );
            false
        }
    }
}

async fn cleanup_missing_movie_title_records(
    app: &AppUseCase,
    title: &Title,
    cleanup: LibraryScanMovieCleanupContext,
) -> bool {
    let mut title_updated = false;

    let media_files = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(media_files) => media_files,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list movie media files during stale cleanup"
            );
            Vec::new()
        }
    };

    for media_file in media_files {
        let file_path = stored_path_to_path_buf(&media_file.file_path);
        if !tracked_movie_path_confirmed_missing(file_path.as_path()).await {
            continue;
        }
        if let Err(error) = app
            .services
            .library
            .media_files
            .delete_media_file(&media_file.id)
            .await
        {
            warn!(
                error = %error,
                title_id = %title.id,
                media_file_id = %media_file.id,
                file_path = %media_file.file_path,
                "failed to delete stale movie media file after title scan"
            );
        } else {
            title_updated = true;
        }
    }

    let collections = match app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
    {
        Ok(collections) => collections,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to list movie collections during stale cleanup"
            );
            Vec::new()
        }
    };
    let cleanup_ids = cleanup
        .stale_collection_ids
        .into_iter()
        .collect::<HashSet<_>>();

    for collection in collections {
        let missing_by_path = if let Some(path) = collection.ordered_path.as_deref() {
            tracked_movie_path_confirmed_missing(Path::new(path)).await
        } else {
            false
        };
        if !missing_by_path && !cleanup_ids.contains(&collection.id) {
            continue;
        }
        if cleanup_ids.contains(&collection.id) && !missing_by_path {
            continue;
        }

        if let Err(error) = app
            .services
            .catalog
            .shows
            .delete_collection(&collection.id)
            .await
        {
            warn!(
                error = %error,
                collection_id = %collection.id,
                title_id = %title.id,
                "failed to delete stale movie collection after title scan"
            );
        } else {
            title_updated = true;
        }
    }

    title_updated
}

async fn hydrate_library_scan_workset(
    app: &AppUseCase,
    coordinator: &LibraryScanCoordinator,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    hydration_targets: Vec<crate::catalog_workflow::HydrationTarget>,
    track_metadata_progress: bool,
    cancel_token: Option<&CancellationToken>,
) -> AppResult<()> {
    for chunk in hydration_targets.chunks(crate::catalog_workflow::HYDRATION_BULK_BATCH_SIZE) {
        if library_scan_cancel_requested(cancel_token) {
            break;
        }
        let hydration_outcome = app
            .hydrate_titles_bulk_cancellable(chunk.to_vec(), cancel_token)
            .await?;

        for (title_id, hydrated) in hydration_outcome.hydrated_titles {
            if let Some(work) = workset.get_mut(&title_id) {
                work.title = hydrated;
            }
            if track_metadata_progress {
                coordinator.mark_metadata_completed(1).await;
            }
        }

        for (title_id, reason) in hydration_outcome.failed_titles {
            if let Some(work) = workset.remove(&title_id) {
                warn!(
                    title_id = %title_id,
                    reason = %reason,
                    "library scan title hydration failed"
                );
                if track_metadata_progress {
                    coordinator.mark_metadata_failed(1).await;
                }
                coordinator
                    .mark_file_failed(work.discovered_file_count())
                    .await;
            }
        }

        if track_metadata_progress {
            coordinator.publish_progress().await;
        }
    }

    Ok(())
}

impl AppUseCase {
    pub(crate) async fn execute_library_scan_workset(
        &self,
        actor: &User,
        session_id: &str,
        mut workset: HashMap<String, LibraryScanTitleWork>,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryScanSummary> {
        if library_scan_cancel_requested(cancel_token.as_ref()) {
            return Ok(LibraryScanSummary::default());
        }

        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        let metadata_language = self.metadata_language().await;
        let file_total = workset
            .values()
            .map(LibraryScanTitleWork::discovered_file_count)
            .sum::<usize>();
        coordinator.add_file_total(file_total).await;
        coordinator.mark_file_total_known().await;

        let hydration_source = self
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await
            .map(|session| hydration_source_for_scan_mode(session.mode))
            .unwrap_or(crate::catalog_workflow::HydrationSource::LibraryScanFull);

        let mut hydration_targets = Vec::new();
        for work in workset.values() {
            let needs_hydration =
                title_requires_scan_hydration(self, &work.title, &metadata_language).await?;
            if needs_hydration {
                hydration_targets.push(crate::catalog_workflow::HydrationTarget {
                    title: work.title.clone(),
                    requested_tvdb_id: None,
                    sync_wanted_after_completion: false,
                    source: hydration_source,
                });
            }
        }

        let track_hydration_metadata_progress = self
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await
            .is_none_or(|session| session.metadata_progress.total == 0);

        if track_hydration_metadata_progress {
            coordinator
                .add_metadata_total(hydration_targets.len())
                .await;
            coordinator.mark_metadata_total_known().await;
            coordinator.publish_progress().await;
        }
        if !hydration_targets.is_empty() {
            hydrate_library_scan_workset(
                self,
                &coordinator,
                &mut workset,
                hydration_targets,
                track_hydration_metadata_progress,
                cancel_token.as_ref(),
            )
            .await?;
        }

        self.run_library_scan_title_work_pool(actor, session_id, workset, cancel_token)
            .await
    }

    async fn run_library_scan_title_work_pool(
        &self,
        actor: &User,
        session_id: &str,
        workset: HashMap<String, LibraryScanTitleWork>,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryScanSummary> {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        let mut summary = LibraryScanSummary::default();
        let mut pending = workset.into_values();
        let mut work_set = tokio::task::JoinSet::new();

        for _ in 0..LIBRARY_SCAN_TITLE_WALK_CONCURRENCY {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            let Some(work) = pending.next() else {
                break;
            };
            let app = self.clone();
            let actor = actor.clone();
            let session_id = session_id.to_string();
            let title_id = work.title.id.clone();
            let discovered_file_count = work.discovered_file_count();
            let absorb_walk_summary =
                matches!(work.facet_plan, LibraryScanTitleFacetPlan::Movie(_));
            let created_in_scan = work.created_in_scan;
            let walk_cancel_token = cancel_token.clone();
            work_set.spawn(async move {
                let result = app
                    .walk_library_title(
                        &actor,
                        LibraryScanTitleWalkRequest {
                            work,
                            session_id: Some(session_id),
                            cancel_token: walk_cancel_token,
                        },
                    )
                    .await;
                (
                    title_id,
                    discovered_file_count,
                    absorb_walk_summary,
                    created_in_scan,
                    result,
                )
            });
        }

        while let Some(result) = work_set.join_next().await {
            let (
                title_id,
                discovered_file_count,
                absorb_walk_summary,
                created_in_scan,
                walk_result,
            ) = result.map_err(|error| AppError::Repository(error.to_string()))?;

            match walk_result {
                Ok(walk_result) => {
                    if absorb_walk_summary {
                        let mut delta = walk_result.summary;
                        if created_in_scan {
                            delta.imported = delta.imported.saturating_sub(1);
                        }
                        summary.absorb(&delta);
                    }
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %title_id,
                        "library scan title walk failed"
                    );
                    coordinator.mark_file_failed(discovered_file_count).await;
                    coordinator.publish_progress().await;
                }
            }

            if !library_scan_cancel_requested(cancel_token.as_ref())
                && let Some(work) = pending.next()
            {
                let app = self.clone();
                let actor = actor.clone();
                let session_id = session_id.to_string();
                let title_id = work.title.id.clone();
                let discovered_file_count = work.discovered_file_count();
                let absorb_walk_summary =
                    matches!(work.facet_plan, LibraryScanTitleFacetPlan::Movie(_));
                let created_in_scan = work.created_in_scan;
                let walk_cancel_token = cancel_token.clone();
                work_set.spawn(async move {
                    let result = app
                        .walk_library_title(
                            &actor,
                            LibraryScanTitleWalkRequest {
                                work,
                                session_id: Some(session_id),
                                cancel_token: walk_cancel_token,
                            },
                        )
                        .await;
                    (
                        title_id,
                        discovered_file_count,
                        absorb_walk_summary,
                        created_in_scan,
                        result,
                    )
                });
            }
        }

        Ok(summary)
    }

    pub async fn scan_title_library(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<LibraryScanSummary> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_management_permission(actor, &title.library_id)
            .await?;

        let facet_plan = match title.facet {
            MediaFacet::Movie => {
                LibraryScanTitleFacetPlan::Movie(LibraryScanMovieCleanupContext::default())
            }
            MediaFacet::Series | MediaFacet::Anime => LibraryScanTitleFacetPlan::Episodic,
        };
        let work = LibraryScanTitleWork {
            title,
            facet_plan,
            discovered_files: None,
            mode: LibraryScanTitleWalkMode::OneOff,
            created_in_scan: false,
        };

        let metadata_language = self.metadata_language().await;
        let mut request = LibraryScanTitleWalkRequest {
            work,
            session_id: None,
            cancel_token: None,
        };

        if title_requires_scan_hydration(self, &request.work.title, &metadata_language).await? {
            let mut hydration_outcome = self
                .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
                    title: request.work.title.clone(),
                    requested_tvdb_id: None,
                    sync_wanted_after_completion: false,
                    source: crate::catalog_workflow::HydrationSource::Interactive,
                }])
                .await?;
            request.work.title = hydration_outcome
                .hydrated_titles
                .remove(title_id)
                .ok_or_else(|| {
                    AppError::Repository(
                        hydration_outcome
                            .failed_titles
                            .remove(title_id)
                            .unwrap_or_else(|| {
                                "title metadata hydration failed before title scan".to_string()
                            }),
                    )
                })?;
        }

        Ok(self.walk_library_title(actor, request).await?.summary)
    }

    pub(crate) async fn scan_title_library_with_discovered_files(
        &self,
        actor: &User,
        title: Title,
        discovered_files: Vec<LibraryFile>,
    ) -> AppResult<LibraryScanSummary> {
        self.require_library_management_permission(actor, &title.library_id)
            .await?;

        let facet_plan = match title.facet {
            MediaFacet::Movie => {
                LibraryScanTitleFacetPlan::Movie(LibraryScanMovieCleanupContext::default())
            }
            MediaFacet::Series | MediaFacet::Anime => LibraryScanTitleFacetPlan::Episodic,
        };
        let mut request = LibraryScanTitleWalkRequest {
            work: LibraryScanTitleWork {
                title,
                facet_plan,
                discovered_files: Some(discovered_files),
                mode: LibraryScanTitleWalkMode::OneOff,
                created_in_scan: false,
            },
            session_id: None,
            cancel_token: None,
        };

        let metadata_language = self.metadata_language().await;
        if title_requires_scan_hydration(self, &request.work.title, &metadata_language).await? {
            let title_id = request.work.title.id.clone();
            let mut hydration_outcome = self
                .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
                    title: request.work.title.clone(),
                    requested_tvdb_id: None,
                    sync_wanted_after_completion: false,
                    source: crate::catalog_workflow::HydrationSource::Interactive,
                }])
                .await?;
            request.work.title = hydration_outcome
                .hydrated_titles
                .remove(&title_id)
                .ok_or_else(|| {
                    AppError::Repository(
                        hydration_outcome
                            .failed_titles
                            .remove(&title_id)
                            .unwrap_or_else(|| {
                                "title metadata hydration failed before title scan".to_string()
                            }),
                    )
                })?;
        }

        Ok(self.walk_library_title(actor, request).await?.summary)
    }

    pub(crate) async fn walk_library_title(
        &self,
        actor: &User,
        request: LibraryScanTitleWalkRequest,
    ) -> AppResult<LibraryTitleWalkResult> {
        let LibraryScanTitleWalkRequest {
            work,
            session_id,
            cancel_token,
        } = request;
        match work.facet_plan {
            LibraryScanTitleFacetPlan::Movie(cleanup) => {
                self.walk_movie_library_title(
                    work.title,
                    session_id.as_deref(),
                    work.discovered_files,
                    cleanup,
                    cancel_token,
                )
                .await
            }
            LibraryScanTitleFacetPlan::Episodic => {
                self.walk_episodic_library_title(
                    actor,
                    work.title,
                    session_id.as_deref(),
                    work.discovered_files,
                    work.mode,
                    cancel_token,
                )
                .await
            }
        }
    }

    async fn walk_movie_library_title(
        &self,
        title: Title,
        session_id: Option<&str>,
        pre_scanned_files: Option<Vec<LibraryFile>>,
        cleanup: LibraryScanMovieCleanupContext,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryTitleWalkResult> {
        let started_at = Instant::now();
        let session_coordinator =
            session_id.map(|value| LibraryScanCoordinator::new(self.clone(), value.to_string()));
        let mut summary = LibraryScanSummary::default();
        let discovered_files = match pre_scanned_files {
            Some(files) => files,
            None => {
                let files = discover_movie_title_files(self, &title).await?;
                if let Some(coordinator) = session_coordinator.as_ref() {
                    coordinator.add_file_total(files.len()).await;
                    coordinator.mark_file_total_known().await;
                }
                files
            }
        };
        let discovered_file_count = discovered_files.len();

        debug!(
            title_id = %title.id,
            title_name = %title.name,
            session_id = session_id.unwrap_or("none"),
            pre_scanned_file_count = discovered_file_count,
            "movie title scan stage: start"
        );

        for file in &discovered_files {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            finalize_movie_scan_file(self, &title, file, &mut summary, cancel_token.as_ref()).await;
            if let Some(coordinator) = session_coordinator.as_ref() {
                coordinator.mark_file_completed(1).await;
            }
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
        }

        if !library_scan_cancel_requested(cancel_token.as_ref())
            && cleanup_missing_movie_title_records(self, &title, cleanup).await
        {
            self.emit_title_updated_activity(None, &title).await;
        }

        info!(
            title_id = %title.id,
            title_name = %title.name,
            files = discovered_file_count,
            imported = summary.imported,
            skipped = summary.skipped,
            elapsed_ms = elapsed_ms_u64(started_at),
            "movie title scan completed"
        );
        if let Some(coordinator) = session_coordinator.as_ref() {
            coordinator.publish_progress().await;
        }

        Ok(LibraryTitleWalkResult { summary })
    }

    async fn walk_episodic_library_title(
        &self,
        actor: &User,
        title: Title,
        session_id: Option<&str>,
        pre_scanned_files: Option<Vec<LibraryFile>>,
        mode: LibraryScanTitleWalkMode,
        cancel_token: Option<CancellationToken>,
    ) -> AppResult<LibraryTitleWalkResult> {
        let started_at = Instant::now();
        let session_coordinator =
            session_id.map(|value| LibraryScanCoordinator::new(self.clone(), value.to_string()));
        let scoped_discovered_files = pre_scanned_files.is_some();
        let pre_scanned_file_count = pre_scanned_files.as_ref().map(Vec::len);
        let scan_mode = mode.as_file_finalize_mode();

        let handler = self.facet_registry.get(&title.facet).ok_or_else(|| {
            AppError::Validation("library scan is not supported for this facet".into())
        })?;
        if !handler.has_episodes() {
            return Err(AppError::Validation(
                "title library scan is only supported for episodic titles".into(),
            ));
        }

        let import_paths = crate::import_workflow::resolve_import_paths(self, &title).await?;
        let title_dir = crate::effective_title_folder_path(
            &import_paths.media_root,
            &title,
            &import_paths.folder_template,
            None,
        );
        let title_dir_str = path_to_stored_string(&title_dir);
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            session_id = session_id.unwrap_or("none"),
            scan_mode = %scan_mode.as_str(),
            title_dir = %title_dir_str,
            pre_scanned_file_count,
            "title scan stage: start"
        );
        let mut walk_elapsed = Duration::ZERO;
        let mut stat_elapsed = Duration::ZERO;
        let mut analyze_elapsed = Duration::ZERO;
        let mut db_elapsed = Duration::ZERO;

        if !scoped_discovered_files && tokio::fs::metadata(&title_dir).await.is_err() {
            tokio::fs::create_dir_all(&title_dir).await.map_err(|err| {
                AppError::Repository(format!(
                    "failed to recreate title directory {}: {err}",
                    title_dir.display()
                ))
            })?;
        }

        let discovered_files = match pre_scanned_files {
            Some(files) => files,
            None => {
                let scan_result = scan_episodic_title_directory_for_progress_metrics(
                    self.services.library.library_scanner.clone(),
                    &title_dir,
                )
                .await?;
                walk_elapsed =
                    walk_elapsed.saturating_add(Duration::from_millis(scan_result.walk_ms));
                stat_elapsed =
                    stat_elapsed.saturating_add(Duration::from_millis(scan_result.stat_ms));
                if let Some(coordinator) = session_coordinator.as_ref() {
                    coordinator.add_file_total(scan_result.files.len()).await;
                    coordinator.mark_file_total_known().await;
                }
                scan_result.files
            }
        };
        let db_started = Instant::now();
        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let title_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            discovered_files = discovered_files.len(),
            existing_files = existing_files.len(),
            collections = collections.len(),
            title_episodes = title_episodes.len(),
            "title scan stage: db state loaded"
        );
        let episode_lookup = build_title_episode_lookup(&collections, &title_episodes);
        let parse_context =
            crate::build_release_parse_context_for_title(&title, &title_episodes, None);
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            "title scan stage: episode lookup built"
        );

        let mut existing_records_by_path: HashMap<String, TitleMediaFile> = HashMap::new();
        let mut episode_links: HashSet<(String, String)> = HashSet::new();

        for file in &existing_files {
            existing_records_by_path
                .entry(file.file_path.clone())
                .or_insert_with(|| file.clone());
            if let Some(episode_id) = file.episode_id.as_ref() {
                episode_links.insert((file.id.clone(), episode_id.clone()));
            }
        }
        let mut remaining_existing_paths = existing_records_by_path
            .keys()
            .cloned()
            .collect::<HashSet<_>>();

        let mut summary = LibraryScanSummary::default();
        let mut layout_summary = TitleScanLayoutSummary::default();
        let mut seen_paths = HashSet::new();
        let analysis_limit = self.runtime.library.library_scan_analysis_limit.clone();
        let mut pending_progress = TitleScanProgressDelta::default();
        let mut unchanged_file_skips = 0usize;
        let mut analyzed_files = 0usize;
        let actor_user_id = Some(actor.id.clone());

        'file_chunks: for file_chunk in discovered_files.chunks(TITLE_SCAN_FILE_BATCH_SIZE) {
            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break;
            }
            let files = file_chunk.to_vec();
            let mut planned_files = Vec::new();
            let mut title_updated_in_batch = false;

            for file in files {
                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    break;
                }
                if !file.path.trim().is_empty() {
                    seen_paths.insert(file.path.clone());
                }
                remaining_existing_paths.remove(&file.path);
                summary.scanned += 1;

                let source_path = stored_path_to_path_buf(&file.path);
                let parsed = parse_title_scan_release_metadata(
                    &source_path,
                    file.display_name.as_str(),
                    &title.facet,
                    &parse_context,
                )
                .await;

                let ep_meta = match parsed.episode.as_ref() {
                    Some(ep) if !ep.episode_numbers.is_empty() => ep,
                    Some(ep) if ep.air_date.is_some() => ep,
                    Some(ep) if !ep.special_absolute_episode_numbers.is_empty() => ep,
                    Some(ep)
                        if ep.absolute_episode.is_some()
                            && title.facet == scryer_domain::MediaFacet::Anime =>
                    {
                        ep
                    }
                    _ => {
                        debug!(
                            title_id = %title.id,
                            title_name = %title.name,
                            file_path = %file.path,
                            display_name = %file.display_name,
                            title_dir = %title_dir_str,
                            discovered_files = discovered_files.len(),
                            parsed_episode = ?parsed.episode,
                            "title scan: episode identity missing"
                        );
                        let unmatched_item = build_title_bound_unmatched_scan_item(
                            &title.facet,
                            &title.library_id,
                            &title.id,
                            session_id,
                            &title_dir_str,
                            &file.path,
                            &file.display_name,
                            &title.name,
                            title.year.map(|value| value as u32),
                            "episode_identity_missing",
                        );
                        if let Err(error) =
                            persist_library_scan_unmatched_item(self, &unmatched_item).await
                        {
                            warn!(
                                error = %error,
                                title_id = %title.id,
                                file_path = %file.path,
                                "failed to persist unmatched title scan item"
                            );
                        }
                        summary.unmatched += 1;
                        pending_progress.absorb(TitleScanProgressDelta::completed(1));
                        flush_title_scan_progress_batch(self, session_id, &mut pending_progress)
                            .await;
                        continue;
                    }
                };

                let season_str = ep_meta.season.unwrap_or(1).to_string();
                let target_episodes =
                    resolve_target_episodes_from_lookup(ep_meta, &season_str, &episode_lookup);

                if target_episodes.is_empty() {
                    debug!(
                        title_id = %title.id,
                        title_name = %title.name,
                        file_path = %file.path,
                        display_name = %file.display_name,
                        title_dir = %title_dir_str,
                        discovered_files = discovered_files.len(),
                        parsed_episode = ?ep_meta,
                        lookup_keys = episode_lookup.key_count(),
                        attempted_season = %season_str,
                        "title scan: episode lookup failed"
                    );
                    let unmatched_item = build_title_bound_unmatched_scan_item(
                        &title.facet,
                        &title.library_id,
                        &title.id,
                        session_id,
                        &title_dir_str,
                        &file.path,
                        &file.display_name,
                        &title.name,
                        title.year.map(|value| value as u32),
                        "episode_lookup_failed",
                    );
                    if let Err(error) =
                        persist_library_scan_unmatched_item(self, &unmatched_item).await
                    {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %file.path,
                            "failed to persist unmatched title scan item"
                        );
                    }
                    summary.unmatched += 1;
                    pending_progress.absorb(TitleScanProgressDelta::completed(1));
                    flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                    continue;
                }

                let snapshot = if let Some(snapshot) = file_source_snapshot_from_library_file(&file)
                {
                    snapshot
                } else {
                    let stat_started = Instant::now();
                    let metadata = match tokio::fs::metadata(&source_path).await {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());
                            warn!(
                                error = %error,
                                title_id = %title.id,
                                file_path = %file.path,
                                "failed to read file metadata during title scan"
                            );
                            summary.skipped += 1;
                            pending_progress.absorb(TitleScanProgressDelta::completed(1));
                            flush_title_scan_progress_batch(
                                self,
                                session_id,
                                &mut pending_progress,
                            )
                            .await;
                            continue;
                        }
                    };
                    stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());

                    FileSourceSnapshot {
                        size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                        signature: file_source_signature_from_metadata(&metadata),
                    }
                };

                summary.matched += 1;
                let layout_observation =
                    classify_title_scan_layout(&title_dir, &source_path, &target_episodes);
                layout_summary.observe(layout_observation);

                let record = if let Some(existing) = existing_records_by_path.get(&file.path) {
                    let desired_scheme = snapshot
                        .signature
                        .as_ref()
                        .map(|value| value.scheme.clone());
                    let desired_value =
                        snapshot.signature.as_ref().map(|value| value.value.clone());
                    PlannedTitleScanRecord::Existing {
                        file_id: existing.id.clone(),
                        should_skip_analysis: title_media_file_matches_snapshot(
                            existing, &snapshot,
                        ),
                        should_refresh_source_signature: existing.size_bytes != snapshot.size_bytes
                            || existing.source_signature_scheme != desired_scheme
                            || existing.source_signature_value != desired_value
                            || existing.scan_status != "scanned",
                    }
                } else {
                    PlannedTitleScanRecord::New
                };

                planned_files.push(PlannedTitleScanFile {
                    file,
                    parsed,
                    target_episodes,
                    snapshot,
                    record,
                });
            }

            planned_files.sort_by(|left, right| left.file.path.cmp(&right.file.path));
            debug!(
                title_id = %title.id,
                title_name = %title.name,
                chunk_files = planned_files.len(),
                "title scan stage: chunk planned"
            );
            debug!(
                title_id = %title.id,
                title_name = %title.name,
                "title scan stage: analysis phase begin"
            );

            let mut analysis_set = tokio::task::JoinSet::new();
            let mut pending_analysis_plans = std::collections::VecDeque::new();
            for plan in planned_files {
                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    break;
                }
                let should_analyze = match &plan.record {
                    PlannedTitleScanRecord::Existing {
                        should_skip_analysis,
                        ..
                    } => !should_skip_analysis,
                    PlannedTitleScanRecord::New => true,
                };

                if !should_analyze {
                    unchanged_file_skips += 1;
                    let file_path = plan.file.path.clone();
                    let outcome = finalize_title_scan_file(
                        self,
                        &title,
                        plan,
                        None,
                        scan_mode.clone(),
                        &mut episode_links,
                        &mut summary,
                        &mut db_elapsed,
                    )
                    .await;
                    if outcome.progress.failed == 0
                        && let Err(error) = clear_library_scan_unmatched_item(
                            self,
                            &title.facet,
                            &title.library_id,
                            &file_path,
                        )
                        .await
                    {
                        warn!(
                            error = %error,
                            title_id = %title.id,
                            file_path = %file_path,
                            "failed to clear unmatched title scan item"
                        );
                    }
                    pending_progress.absorb(outcome.progress);
                    title_updated_in_batch |= outcome.title_updated;
                    flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                    continue;
                }

                analyzed_files += 1;
                pending_analysis_plans.push_back(plan);
            }
            debug!(
                title_id = %title.id,
                title_name = %title.name,
                pending_analysis = pending_analysis_plans.len(),
                "title scan stage: analysis tasks queued"
            );

            while !pending_analysis_plans.is_empty() || !analysis_set.is_empty() {
                while !library_scan_cancel_requested(cancel_token.as_ref())
                    && analysis_set.len() < GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY
                {
                    let Some(plan) = pending_analysis_plans.pop_front() else {
                        break;
                    };
                    let analyzer = self.services.library.media_analyzer.clone();
                    let analysis_limit = analysis_limit.clone();
                    let file_path = plan.file.path.clone();
                    analysis_set.spawn(async move {
                        tracing::debug!(file_path = %file_path, "title scan analysis task: start");
                        let _permit = analysis_limit
                            .acquire_owned()
                            .await
                            .map_err(|error| AppError::Repository(error.to_string()))?;
                        let analysis_started = Instant::now();
                        let outcome =
                            analyzer.analyze_file(stored_path_to_path_buf(&file_path)).await?;
                        tracing::debug!(file_path = %file_path, "title scan analysis task: complete");
                        Ok::<(PlannedTitleScanFile, MediaAnalysisOutcome, Duration), AppError>((
                            plan,
                            outcome,
                            analysis_started.elapsed(),
                        ))
                    });
                }

                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    pending_analysis_plans.clear();
                    analysis_set.abort_all();
                    break;
                }

                let Some(result) =
                    await_cancellable(cancel_token.as_ref(), analysis_set.join_next())
                        .await
                        .flatten()
                else {
                    pending_analysis_plans.clear();
                    analysis_set.abort_all();
                    break;
                };

                let (plan, analysis_outcome, analysis_duration) =
                    result.map_err(|error| AppError::Repository(error.to_string()))??;
                analyze_elapsed = analyze_elapsed.saturating_add(analysis_duration);
                if library_scan_cancel_requested(cancel_token.as_ref()) {
                    continue;
                }
                let file_path = plan.file.path.clone();
                debug!(
                    title_id = %title.id,
                    title_name = %title.name,
                    file_path = %file_path,
                    "title scan stage: finalize file begin"
                );
                let outcome = finalize_title_scan_file(
                    self,
                    &title,
                    plan,
                    Some(analysis_outcome),
                    scan_mode.clone(),
                    &mut episode_links,
                    &mut summary,
                    &mut db_elapsed,
                )
                .await;
                if outcome.progress.failed == 0
                    && let Err(error) = clear_library_scan_unmatched_item(
                        self,
                        &title.facet,
                        &title.library_id,
                        &file_path,
                    )
                    .await
                {
                    warn!(
                        error = %error,
                        title_id = %title.id,
                        file_path = %file_path,
                        "failed to clear unmatched title scan item"
                    );
                }
                pending_progress.absorb(outcome.progress);
                title_updated_in_batch |= outcome.title_updated;
                flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                debug!(
                    title_id = %title.id,
                    title_name = %title.name,
                    file_path = %file_path,
                    "title scan stage: finalize file complete"
                );
            }

            if title_updated_in_batch {
                self.emit_title_updated_activity(actor_user_id.clone(), &title)
                    .await;
            }

            if library_scan_cancel_requested(cancel_token.as_ref()) {
                break 'file_chunks;
            }
        }

        flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;

        if !library_scan_cancel_requested(cancel_token.as_ref()) && !scoped_discovered_files {
            reconcile_library_scan_unmatched_items(self, &title.facet, &title_dir_str, &seen_paths)
                .await?;
            let mut title_updated_after_scan = false;
            for stale_path in remaining_existing_paths {
                let Some(record) = existing_records_by_path.get(&stale_path).cloned() else {
                    continue;
                };
                if !stale_path.starts_with(title_dir_str.as_str()) {
                    continue;
                }
                if stored_path_to_path_buf(&record.file_path).exists() {
                    continue;
                }
                let db_started = Instant::now();
                let delete_result = self
                    .services
                    .library
                    .media_files
                    .delete_media_file(&record.id)
                    .await;
                db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                if let Err(error) = delete_result {
                    warn!(
                        error = %error,
                        title_id = %title.id,
                        file_path = %record.file_path,
                        "failed to delete stale media file during title scan"
                    );
                } else {
                    title_updated_after_scan = true;
                }
            }

            if title.folder_path.as_deref() != Some(title_dir_str.as_str()) {
                let db_started = Instant::now();
                self.services
                    .catalog
                    .titles
                    .set_folder_path(&title.id, &title_dir_str)
                    .await?;
                db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                title_updated_after_scan = true;
            }

            if let Some(use_season_folders) = layout_summary.inferred_use_season_folders()
                && crate::import_workflow::use_season_folders(&title) != use_season_folders
            {
                let tags = merge_title_scan_option_tags(title.tags.clone(), use_season_folders);
                let db_started = Instant::now();
                self.apply_title_metadata_update(
                    Some(actor.id.clone()),
                    &title.id,
                    None,
                    None,
                    Some(tags),
                )
                .await?;
                db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
                title_updated_after_scan = true;
            }

            if title_updated_after_scan {
                self.emit_title_updated_activity(actor_user_id, &title)
                    .await;
            }
        }

        debug!(
            title_id = %title.id,
            path = %title_dir.display(),
            scanned = summary.scanned,
            matched = summary.matched,
            imported = summary.imported,
            skipped = summary.skipped,
            unmatched = summary.unmatched,
            walk_ms = u64::try_from(walk_elapsed.as_millis()).unwrap_or(u64::MAX),
            stat_ms = u64::try_from(stat_elapsed.as_millis()).unwrap_or(u64::MAX),
            analyze_ms = u64::try_from(analyze_elapsed.as_millis()).unwrap_or(u64::MAX),
            db_ms = u64::try_from(db_elapsed.as_millis()).unwrap_or(u64::MAX),
            analyzed_files,
            unchanged_file_skips,
            batch_size = TITLE_SCAN_FILE_BATCH_SIZE,
            worker_concurrency = GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY,
            elapsed_ms = elapsed_ms_u64(started_at),
            "title library scan completed"
        );

        Ok(LibraryTitleWalkResult { summary })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LibraryScanTitleWalkRequest {
    pub(crate) work: LibraryScanTitleWork,
    pub(crate) session_id: Option<String>,
    pub(crate) cancel_token: Option<CancellationToken>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{Episode, ExternalId, MediaFacet, Title};
    use std::path::Path;

    fn numeric_series_title() -> Title {
        Title {
            id: "title-13".into(),
            name: "13".into(),
            facet: MediaFacet::Series,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            monitored: true,
            tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".into(),
                value: "131313".into(),
            }],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn numeric_series_episode(season: &str, episode: &str) -> Episode {
        Episode {
            id: format!("episode-{season}-{episode}"),
            title_id: "title-13".into(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some(episode.into()),
            season_number: Some(season.into()),
            episode_label: Some(format!("S{season:0>2}E{episode:0>2}")),
            title: Some(format!("Day {season} 800 A.M. 900 A.M.")),
            air_date: None,
            duration_seconds: None,
            image_url: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn parses_anonymized_numeric_series_season_two_filename_for_title_scan() {
        let title = numeric_series_title();
        let episodes = vec![
            numeric_series_episode("1", "1"),
            numeric_series_episode("2", "1"),
        ];
        let parse_context = crate::build_release_parse_context_for_title(&title, &episodes, None);
        let path = Path::new(
            "/library/13 (2024)/Season 02/13 (2024) - S02E01 - Day 2 800 A.M. 900 A.M. [WEBDL-1080p] [EAC3 5.1] [h265].mkv",
        );

        let parsed = parse_title_scan_release_metadata(
            path,
            "13 (2024) - S02E01",
            &title.facet,
            &parse_context,
        )
        .await;
        let episode = parsed.episode.as_ref().expect("episode metadata");

        assert_eq!(parsed.normalized_title, "13");
        assert_eq!(episode.season, Some(2));
        assert_eq!(episode.episode_numbers, vec![1]);
    }
}
