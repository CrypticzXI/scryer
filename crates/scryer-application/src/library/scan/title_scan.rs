use super::*;

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
    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .unwrap_or_default();
    let mut discovered_files = Vec::new();
    let mut seen_paths = HashSet::new();

    for collection in collections {
        let Some(ordered_path) = collection.ordered_path else {
            continue;
        };
        if !seen_paths.insert(ordered_path.clone()) {
            continue;
        }

        discovered_files.push(LibraryFile {
            path: ordered_path.clone(),
            display_name: Path::new(&ordered_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: matching_movie_nfo_path(Path::new(&ordered_path)),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        });
    }

    if !discovered_files.is_empty() {
        return Ok(discovered_files);
    }

    let (media_root, _) = crate::import_workflow::resolve_import_paths(app, title).await?;
    let candidate_path = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&media_root).join(&title.name));

    match tokio::fs::metadata(&candidate_path).await {
        Ok(metadata) if metadata.is_dir() => Ok(app
            .services
            .library
            .library_scanner
            .scan_library(candidate_path.to_string_lossy().as_ref())
            .await?),
        Ok(metadata) if metadata.is_file() => Ok(vec![LibraryFile {
            path: candidate_path.to_string_lossy().to_string(),
            display_name: candidate_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: matching_movie_nfo_path(&candidate_path),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }]),
        Ok(_) | Err(_) => Ok(Vec::new()),
    }
}

async fn hydrate_library_scan_workset(
    app: &AppUseCase,
    coordinator: &LibraryScanCoordinator,
    workset: &mut HashMap<String, LibraryScanTitleWork>,
    hydration_targets: Vec<crate::catalog_workflow::HydrationTarget>,
    track_metadata_progress: bool,
) -> AppResult<()> {
    for chunk in hydration_targets.chunks(crate::catalog_workflow::HYDRATION_BULK_BATCH_SIZE) {
        let hydration_outcome = app.hydrate_titles_bulk(chunk.to_vec()).await?;

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
    ) -> AppResult<LibraryScanSummary> {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        let metadata_language = self.metadata_language().await;
        let file_total = workset
            .values()
            .map(LibraryScanTitleWork::discovered_file_count)
            .sum::<usize>();
        coordinator.add_file_total(file_total).await;
        coordinator.mark_file_total_known().await;

        let mut hydration_targets = Vec::new();
        for work in workset.values() {
            let needs_hydration =
                title_requires_scan_hydration(self, &work.title, &metadata_language).await?;
            if needs_hydration {
                hydration_targets.push(crate::catalog_workflow::HydrationTarget {
                    title: work.title.clone(),
                    requested_tvdb_id: None,
                    sync_wanted_after_completion: false,
                });
            }
        }

        let track_hydration_metadata_progress = self
            .runtime
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
            )
            .await?;
        }

        self.run_library_scan_title_work_pool(actor, session_id, workset)
            .await
    }

    async fn run_library_scan_title_work_pool(
        &self,
        actor: &User,
        session_id: &str,
        workset: HashMap<String, LibraryScanTitleWork>,
    ) -> AppResult<LibraryScanSummary> {
        let coordinator = LibraryScanCoordinator::new(self.clone(), session_id.to_string());
        let mut summary = LibraryScanSummary::default();
        let mut pending = workset.into_values();
        let mut work_set = tokio::task::JoinSet::new();

        for _ in 0..LIBRARY_SCAN_TITLE_WALK_CONCURRENCY {
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
            work_set.spawn(async move {
                let result = app
                    .walk_library_title(
                        &actor,
                        LibraryScanTitleWalkRequest {
                            work,
                            session_id: Some(session_id),
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

            if let Some(work) = pending.next() {
                let app = self.clone();
                let actor = actor.clone();
                let session_id = session_id.to_string();
                let title_id = work.title.id.clone();
                let discovered_file_count = work.discovered_file_count();
                let absorb_walk_summary =
                    matches!(work.facet_plan, LibraryScanTitleFacetPlan::Movie(_));
                let created_in_scan = work.created_in_scan;
                work_set.spawn(async move {
                    let result = app
                        .walk_library_title(
                            &actor,
                            LibraryScanTitleWalkRequest {
                                work,
                                session_id: Some(session_id),
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
        require(actor, &Entitlement::ManageTitle)?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;

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
        };

        if title_requires_scan_hydration(self, &request.work.title, &metadata_language).await? {
            let mut hydration_outcome = self
                .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
                    title: request.work.title.clone(),
                    requested_tvdb_id: None,
                    sync_wanted_after_completion: false,
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

    pub(crate) async fn walk_library_title(
        &self,
        actor: &User,
        request: LibraryScanTitleWalkRequest,
    ) -> AppResult<LibraryTitleWalkResult> {
        let LibraryScanTitleWalkRequest { work, session_id } = request;
        match work.facet_plan {
            LibraryScanTitleFacetPlan::Movie(cleanup) => {
                self.walk_movie_library_title(
                    work.title,
                    session_id.as_deref(),
                    work.discovered_files,
                    cleanup,
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

        info!(
            title_id = %title.id,
            title_name = %title.name,
            session_id = session_id.unwrap_or("none"),
            pre_scanned_file_count = discovered_file_count,
            "movie title scan stage: start"
        );

        for file in &discovered_files {
            finalize_movie_scan_file(self, &title, file, &mut summary).await;
            if let Some(coordinator) = session_coordinator.as_ref() {
                coordinator.mark_file_completed(1).await;
            }
        }

        for collection_id in cleanup.stale_collection_ids {
            if let Err(error) = self
                .services
                .catalog
                .shows
                .delete_collection(&collection_id)
                .await
            {
                warn!(
                    error = %error,
                    collection_id = %collection_id,
                    title_id = %title.id,
                    "failed to delete stale collection after movie title walk"
                );
            }
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
    ) -> AppResult<LibraryTitleWalkResult> {
        require(actor, &Entitlement::ManageTitle)?;
        let started_at = Instant::now();
        let session_coordinator =
            session_id.map(|value| LibraryScanCoordinator::new(self.clone(), value.to_string()));
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

        let (media_root, _) = crate::import_workflow::resolve_import_paths(self, &title).await?;
        let title_dir = title
            .folder_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&media_root).join(&title.name));
        let title_dir_str = title_dir.to_string_lossy().to_string();
        info!(
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

        if tokio::fs::metadata(&title_dir).await.is_err() {
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
        info!(
            title_id = %title.id,
            title_name = %title.name,
            discovered_files = discovered_files.len(),
            existing_files = existing_files.len(),
            collections = collections.len(),
            title_episodes = title_episodes.len(),
            "title scan stage: db state loaded"
        );
        let episode_lookup = build_title_episode_lookup(&collections, &title_episodes);
        info!(
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
        let analysis_limit = self.runtime.library_scan_analysis_limit.clone();
        let mut pending_progress = TitleScanProgressDelta::default();
        let mut unchanged_file_skips = 0usize;
        let mut analyzed_files = 0usize;
        let actor_user_id = Some(actor.id.clone());

        for file_chunk in discovered_files.chunks(TITLE_SCAN_FILE_BATCH_SIZE) {
            let files = file_chunk.to_vec();
            let mut planned_files = Vec::new();
            let mut title_updated_in_batch = false;

            for file in files {
                remaining_existing_paths.remove(&file.path);
                summary.scanned += 1;

                let source_path = Path::new(&file.path);
                let parsed = parse_release_metadata(
                    source_path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or(file.display_name.as_str()),
                );

                let ep_meta = match parsed.episode.as_ref() {
                    Some(ep) if !ep.episode_numbers.is_empty() => ep,
                    Some(ep) if ep.air_date.is_some() => ep,
                    Some(ep)
                        if ep.absolute_episode.is_some()
                            && title.facet == scryer_domain::MediaFacet::Anime =>
                    {
                        ep
                    }
                    _ => {
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
                    let metadata = match tokio::fs::metadata(source_path).await {
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
                    classify_title_scan_layout(&title_dir, source_path, &target_episodes);
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
            info!(
                title_id = %title.id,
                title_name = %title.name,
                chunk_files = planned_files.len(),
                "title scan stage: chunk planned"
            );
            info!(
                title_id = %title.id,
                title_name = %title.name,
                "title scan stage: analysis phase begin"
            );

            let mut analysis_set = tokio::task::JoinSet::new();
            let mut pending_analysis_plans = HashMap::new();
            for plan in planned_files {
                let should_analyze = match &plan.record {
                    PlannedTitleScanRecord::Existing {
                        should_skip_analysis,
                        ..
                    } => !should_skip_analysis,
                    PlannedTitleScanRecord::New => true,
                };

                if !should_analyze {
                    unchanged_file_skips += 1;
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
                    pending_progress.absorb(outcome.progress);
                    title_updated_in_batch |= outcome.title_updated;
                    flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                    continue;
                }

                analyzed_files += 1;
                let analyzer = self.services.library.media_analyzer.clone();
                let analysis_limit = analysis_limit.clone();
                let file_path = plan.file.path.clone();
                pending_analysis_plans.insert(file_path.clone(), plan);
                analysis_set.spawn(async move {
                    tracing::info!(file_path = %file_path, "title scan analysis task: start");
                    let _permit = analysis_limit
                        .acquire_owned()
                        .await
                        .map_err(|error| AppError::Repository(error.to_string()))?;
                    let analysis_started = Instant::now();
                    let outcome = analyzer.analyze_file(PathBuf::from(&file_path)).await?;
                    tracing::info!(file_path = %file_path, "title scan analysis task: complete");
                    Ok::<(String, MediaAnalysisOutcome, Duration), AppError>((
                        file_path,
                        outcome,
                        analysis_started.elapsed(),
                    ))
                });
            }
            info!(
                title_id = %title.id,
                title_name = %title.name,
                pending_analysis = pending_analysis_plans.len(),
                "title scan stage: analysis tasks spawned"
            );

            while let Some(result) = analysis_set.join_next().await {
                let (file_path, outcome, analysis_duration) =
                    result.map_err(|error| AppError::Repository(error.to_string()))??;
                analyze_elapsed = analyze_elapsed.saturating_add(analysis_duration);
                let Some(plan) = pending_analysis_plans.remove(&file_path) else {
                    warn!(
                        title_id = %title.id,
                        file_path = %file_path,
                        "missing planned title scan file for completed analysis result"
                    );
                    continue;
                };
                info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    file_path = %file_path,
                    "title scan stage: finalize file begin"
                );
                let outcome = finalize_title_scan_file(
                    self,
                    &title,
                    plan,
                    Some(outcome),
                    scan_mode.clone(),
                    &mut episode_links,
                    &mut summary,
                    &mut db_elapsed,
                )
                .await;
                pending_progress.absorb(outcome.progress);
                title_updated_in_batch |= outcome.title_updated;
                flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;
                info!(
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
        }

        flush_title_scan_progress_batch(self, session_id, &mut pending_progress).await;

        let mut title_updated_after_scan = false;
        for stale_path in remaining_existing_paths {
            let Some(record) = existing_records_by_path.get(&stale_path).cloned() else {
                continue;
            };
            if !stale_path.starts_with(title_dir_str.as_str()) {
                continue;
            }
            if Path::new(&record.file_path).exists() {
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
            self.update_title_metadata(actor, &title.id, None, None, Some(tags))
                .await?;
            db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            title_updated_after_scan = true;
        }

        if title_updated_after_scan {
            self.emit_title_updated_activity(actor_user_id, &title)
                .await;
        }

        info!(
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
}
