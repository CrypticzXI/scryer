const MANUAL_IMPORT_POLLER_INTERVAL_SECONDS: u64 = 2;
const MANUAL_IMPORT_RECOVERY_BATCH_SIZE: usize = 500;
const MANUAL_IMPORT_SOURCE_UNAVAILABLE: &str = "download is no longer available for manual import";
pub async fn start_background_manual_import_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let worker = PollingWorker::new("manual_import_poller", token);
    tracing::info!(
        interval_seconds = MANUAL_IMPORT_POLLER_INTERVAL_SECONDS,
        "manual import poller started"
    );
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
        MANUAL_IMPORT_POLLER_INTERVAL_SECONDS,
    ));
    loop {
        if !worker.wait_for_tick(&mut interval).await {
            return;
        }

        match app
            .services
            .workflow
            .imports
            .recover_stale_processing_imports_for_type(
                ImportType::ManualImport,
                IMPORT_STALE_RECOVERY_SECONDS,
            )
            .await
        {
            Ok(recovered) if recovered > 0 => {
                worker.warn_recovered("recover_stale_manual_imports", recovered);
            }
            Err(error) => {
                worker.warn_error("recover_stale_manual_imports", &error);
            }
            _ => {}
        }

        recover_completed_manual_imports(&app, &worker).await;

        let pending = match app
            .services
            .workflow
            .imports
            .list_pending_imports_for_type(ImportType::ManualImport)
            .await
        {
            Ok(records) => records,
            Err(error) => {
                worker.warn_error("list_pending_manual_imports", &error);
                continue;
            }
        };

        for record in pending {
            let payload =
                match serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json) {
                    Ok(payload) => payload,
                    Err(error) => {
                        let result_json = manual_import_result_json(
                            &record.id,
                            &ManualImportRequestPayload {
                                requested_by_user_id: None,
                                title_id: None,
                                download_client_item_id: record.source_ref.clone(),
                                client_id: None,
                                client_type: record.source_system.clone(),
                                files: Vec::new(),
                                selection_id: None,
                                release_evidence: None,
                                requested_at: record.created_at.clone(),
                            },
                            ImportStatus::Failed,
                            Some(ImportErrorCode::Unknown),
                            Some(format!("invalid manual import payload: {error}")),
                            Vec::new(),
                        );
                        let _ = app
                            .update_import_status_and_notify(
                                &record.id,
                                ImportStatus::Failed,
                                result_json,
                            )
                            .await;
                        continue;
                    }
                };

            let _import_permit = app.runtime.imports.execution_coordinator.acquire().await;
            let outcome =
                match execute_queued_manual_import_with_outcome(&app, &record.id, &payload).await {
                    Ok(result) => result,
                    Err(error) => QueuedManualImportOutcome {
                        status: ImportStatus::Failed,
                        result_json: manual_import_result_json(
                            &record.id,
                            &payload,
                            ImportStatus::Failed,
                            Some(classify_manual_import_error_message(&error.to_string())),
                            Some(error.to_string()),
                            Vec::new(),
                        ),
                        files_imported_this_pass: 0,
                        completed: None,
                        title_id: payload.title_id.clone(),
                        expected_mapping_count: None,
                        prior_import_proven: false,
                    },
                };

            if let Err(error) = app
                .update_import_status_and_notify(
                    &record.id,
                    outcome.status,
                    outcome.result_json.clone(),
                )
                .await
            {
                worker.warn_error("finalize_manual_import_request", &error);
                continue;
            }

            let has_successful_import = outcome.files_imported_this_pass > 0
                || outcome.status == ImportStatus::Completed
                || outcome.prior_import_proven;
            let terminalized = if has_successful_import {
                if let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() {
                    let tracked_id = crate::tracked_downloads::tracked_download_id(
                        payload.client_id.as_deref(),
                        &payload.client_type,
                        &payload.download_client_item_id,
                    );
                    let reconciliation = if outcome.prior_import_proven {
                        handle.mark_imported(tracked_id).await.map(|()| true)
                    } else {
                        handle
                            .reconcile_manual_import(
                                tracked_id,
                                outcome.files_imported_this_pass,
                                outcome.expected_mapping_count,
                            )
                            .await
                    };
                    match reconciliation {
                        Ok(terminalized) => terminalized,
                        Err(error) => {
                            worker.warn_error("reconcile_manual_import", &error);
                            false
                        }
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if terminalized {
                let source_identity = DownloadSourceIdentity::new(
                    payload.client_id.as_deref(),
                    &payload.client_type,
                    &payload.download_client_item_id,
                );
                if let Err(error) = app
                    .services
                    .workflow
                    .imports
                    .delete_manual_import_selections_for_source(&source_identity)
                    .await
                {
                    worker.warn_error("cleanup_terminal_manual_import_selections", &error);
                }
            }

            maybe_remove_completed_manual_import_download(
                &app,
                outcome.completed.as_ref(),
                outcome.title_id.as_deref(),
                terminalized,
            )
            .await;
        }
    }
}
async fn maybe_remove_completed_manual_import_download(
    app: &AppUseCase,
    completed: Option<&CompletedDownload>,
    title_id: Option<&str>,
    imported: bool,
) {
    if !imported {
        return;
    }

    let Some(completed) = completed else {
        return;
    };

    let (library_id, resolved_facet) = cleanup_routing_scope_for_title_id(app, title_id).await;
    let facet = resolved_facet.or_else(|| facet_for_completed_download(completed));

    let Some(facet) = facet else {
        return;
    };

    let _ = reconcile_terminal_download_cleanup(
        app,
        &completed.client_id,
        &completed.client_type,
        &completed.download_client_item_id,
        library_id.as_deref(),
        Some(&facet),
        TrackedDownloadState::Imported,
    )
    .await;
}
// ---------------------------------------------------------------------------
// Manual import: preview & execute
// ---------------------------------------------------------------------------

/// A single file in a manual import preview with auto-detected episode info.
struct ManualImportFilePreview {
    file_path: String,
    file_name: String,
    size_bytes: i64,
    quality: Option<String>,
    parsed_season: Option<u32>,
    parsed_episodes: Vec<u32>,
    suggested_episode_id: Option<String>,
    suggested_episode_label: Option<String>,
    suggested_series_movie_link_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ManualImportSeriesMovieTarget {
    pub series_movie_link_id: String,
    pub movie_title: String,
    pub year: Option<i32>,
    pub runtime_minutes: Option<i32>,
}

/// Internal file metadata used to construct a server-owned manual-import selection.
struct ManualImportPreview {
    files: Vec<ManualImportFilePreview>,
}

/// A file selected from a server-owned manual-import selection. Its source path remains internal.
pub struct ManualImportSelectionFilePreview {
    pub candidate_id: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub quality: Option<String>,
    pub parsed_season: Option<u32>,
    pub parsed_episodes: Vec<u32>,
    pub suggested_episode_id: Option<String>,
    pub suggested_episode_label: Option<String>,
    pub suggested_series_movie_link_id: Option<String>,
}

pub struct ManualImportSelectionPreview {
    pub selection_id: String,
    pub files: Vec<ManualImportSelectionFilePreview>,
    pub available_episodes: Vec<scryer_domain::Episode>,
    pub available_series_movies: Vec<ManualImportSeriesMovieTarget>,
}

/// The only client-controlled portion of a manual-import mapping.
#[derive(Clone, Debug)]
pub struct ManualImportCandidateMapping {
    pub candidate_id: String,
    pub episode_id: Option<String>,
    pub series_movie_link_id: Option<String>,
}

async fn manual_import_preview_targets(
    app: &AppUseCase,
    title_id: &str,
) -> AppResult<(
    Vec<scryer_domain::Episode>,
    Vec<ManualImportSeriesMovieTarget>,
)> {
    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(title_id)
        .await?;
    let mut all_episodes = Vec::new();
    for collection in &collections {
        let episodes = app
            .services
            .catalog
            .shows
            .list_episodes_for_collection(&collection.id)
            .await?;
        all_episodes.extend(episodes);
    }

    let series_movies = app
        .services
        .catalog
        .shows
        .list_series_movie_links_for_title(title_id)
        .await?
        .into_iter()
        .map(|link| ManualImportSeriesMovieTarget {
            series_movie_link_id: link.id,
            movie_title: link.movie.title,
            year: link.movie.year,
            runtime_minutes: link.movie.runtime_minutes,
        })
        .collect();

    Ok((all_episodes, series_movies))
}

fn manual_import_source_unavailable() -> AppError {
    AppError::NotFound(MANUAL_IMPORT_SOURCE_UNAVAILABLE.to_string())
}

pub(crate) async fn resolve_current_manual_import_source(
    app: &AppUseCase,
    actor: &User,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    title_id: &str,
) -> AppResult<CompletedDownload> {
    let client_id = client_id.trim();
    let client_type = client_type.trim();
    let download_client_item_id = download_client_item_id.trim();
    let authorized = authorize_manual_import_source(
        app,
        actor,
        client_id,
        client_type,
        download_client_item_id,
        title_id,
    )
    .await?;
    resolve_authorized_manual_import_source(app, &authorized.identity).await
}

struct AuthorizedManualImportSource {
    identity: DownloadSourceIdentity,
    facet: MediaFacet,
}

async fn authorize_manual_import_source(
    app: &AppUseCase,
    actor: &User,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    title_id: &str,
) -> AppResult<AuthorizedManualImportSource> {
    if client_id.is_empty() || client_type.is_empty() || download_client_item_id.is_empty() {
        return Err(manual_import_source_unavailable());
    }

    let source_identity =
        DownloadSourceIdentity::new(Some(client_id), client_type, download_client_item_id);
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(manual_import_source_unavailable)?;
    app.require_library_permission(
        actor,
        &title.library_id,
        scryer_domain::LibraryPermission::ResolveImports,
    )
    .await?;

    let submission = app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id(&source_identity)
        .await?
        .ok_or_else(manual_import_source_unavailable)?;
    if !matches!(&submission.scope, crate::SubmissionScope::Orphan)
        && (submission.title_id.trim().is_empty() || submission.title_id != title.id)
    {
        return Err(manual_import_source_unavailable());
    }

    Ok(AuthorizedManualImportSource {
        identity: source_identity,
        facet: title.facet,
    })
}

async fn resolve_authorized_manual_import_source(
    app: &AppUseCase,
    identity: &DownloadSourceIdentity,
) -> AppResult<CompletedDownload> {
    if let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() {
        match handle.completed_source(identity.clone()).await {
            Ok(Some(completed)) if completed_download_matches_source(&completed, identity) => {
                return Ok(completed);
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    client_id = ?identity.client_id,
                    client_type = %identity.client_type,
                    download_client_item_id = %identity.item_id,
                    "retained manual-import source lookup failed; falling back to live lookup"
                );
            }
        }
    }

    resolve_live_manual_import_source(app, identity).await
}

async fn resolve_live_manual_import_source(
    app: &AppUseCase,
    identity: &DownloadSourceIdentity,
) -> AppResult<CompletedDownload> {
    let completed = match app
        .resolve_manual_import_source(
            identity.client_id.as_deref(),
            Some(identity.client_type.as_str()),
            identity.item_id.as_str(),
        )
        .await?
    {
        crate::ManualImportSourceResolution::Eligible {
            completed: Some(completed),
        } => *completed,
        _ => return Err(manual_import_source_unavailable()),
    };

    if !completed_download_matches_source(&completed, identity) {
        return Err(manual_import_source_unavailable());
    }

    Ok(completed)
}

fn completed_download_matches_source(
    completed: &CompletedDownload,
    identity: &DownloadSourceIdentity,
) -> bool {
    completed.client_id == identity.client_id.as_deref().unwrap_or_default()
        && completed
            .client_type
            .eq_ignore_ascii_case(identity.client_type.as_str())
        && completed.download_client_item_id == identity.item_id
}

fn import_record_proves_prior_import(record: &ImportRecord, current_import_id: &str) -> bool {
    if record.id == current_import_id {
        return false;
    }

    let canonical_result = record
        .result_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<scryer_domain::ImportResult>(json).ok());
    match record.status {
        ImportStatus::Completed if record.import_type != ImportType::ManualImport => true,
        ImportStatus::Completed => canonical_result.is_some_and(|result| {
            result.decision == ImportDecision::Imported
                || result.skip_reason == Some(ImportSkipReason::AlreadyImported)
        }),
        ImportStatus::Skipped => canonical_result
            .is_some_and(|result| result.skip_reason == Some(ImportSkipReason::AlreadyImported)),
        _ => false,
    }
}

async fn manual_import_source_was_already_imported(
    app: &AppUseCase,
    source: &AuthorizedManualImportSource,
    current_import_id: &str,
) -> AppResult<bool> {
    if app
        .services
        .workflow
        .download_submissions
        .get_tracked_state(&source.identity)
        .await?
        .as_deref()
        == Some(TrackedDownloadState::Imported.as_str())
    {
        return Ok(true);
    }
    if source.facet != MediaFacet::Movie {
        return Ok(false);
    }

    Ok(app
        .services
        .workflow
        .imports
        .list_imports_for_identities(std::slice::from_ref(&source.identity))
        .await?
        .iter()
        .any(|record| import_record_proves_prior_import(record, current_import_id)))
}

fn source_path_canonical(source_path: &Path) -> AppResult<PathBuf> {
    std::fs::canonicalize(source_path).map_err(|err| {
        AppError::Validation(format!(
            "manual import path is not accessible: {} ({err})",
            source_path.display()
        ))
    })
}

fn source_entry_location_under_parent(source_path: &Path) -> AppResult<PathBuf> {
    let parent = source_path.parent().ok_or_else(|| {
        AppError::Validation(format!(
            "manual import file must have a parent directory: {}",
            source_path.display()
        ))
    })?;
    let file_name = source_path.file_name().ok_or_else(|| {
        AppError::Validation(format!(
            "manual import file must have a file name: {}",
            source_path.display()
        ))
    })?;
    let parent = source_path_canonical(parent)?;
    Ok(parent.join(file_name))
}

pub(crate) fn validate_manual_import_source_under_trusted_root(
    source_path: &Path,
    trusted_root: &Path,
) -> AppResult<()> {
    let source_entry_location = source_entry_location_under_parent(source_path)?;
    if source_entry_location != trusted_root && !source_entry_location.starts_with(trusted_root) {
        return Err(AppError::Validation(format!(
            "manual import file path is outside the trusted source root: {}",
            source_path.display()
        )));
    }

    let canonical = std::fs::canonicalize(source_path).map_err(|err| {
        AppError::Validation(format!(
            "manual import file is not accessible: {} ({err})",
            source_path.display()
        ))
    })?;
    if canonical != trusted_root && !canonical.starts_with(trusted_root) {
        return Err(AppError::Validation(format!(
            "manual import file is outside the trusted source root: {}",
            source_path.display()
        )));
    }

    let metadata = std::fs::symlink_metadata(source_path).map_err(|err| {
        AppError::Validation(format!(
            "manual import file is not accessible: {} ({err})",
            source_path.display()
        ))
    })?;
    if !metadata.file_type().is_symlink() {
        return Ok(());
    }
    Ok(())
}

/// Scan a completed download's directory and attempt to auto-match files to episodes.
async fn preview_manual_import(
    app: &AppUseCase,
    completed: &CompletedDownload,
    title_id: &str,
    release_evidence: &ReleaseEvidence,
    available_episodes: &[scryer_domain::Episode],
) -> AppResult<ManualImportPreview> {
    // Scan for video files (recursive, no sample filtering - let user see everything)
    let dest_dir = Path::new(&completed.dest_dir);
    let video_files = find_video_files(dest_dir, false)?;
    let grabbed_episode_ids = match release_evidence.scope() {
        Some(SubmissionScope::Episode { episode_id }) => {
            HashSet::from([episode_id.clone()])
        }
        Some(SubmissionScope::EpisodeSet { episode_ids }) => {
            episode_ids.iter().cloned().collect()
        }
        Some(SubmissionScope::Collection { collection_id }) => available_episodes
            .iter()
            .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
            .map(|episode| episode.id.clone())
            .collect(),
        Some(
            SubmissionScope::Title
            | SubmissionScope::SeriesMovie { .. }
            | SubmissionScope::Orphan,
        )
        | None => HashSet::new(),
    };
    let grabbed_series_movie_link_id = match release_evidence.scope() {
        Some(SubmissionScope::SeriesMovie {
            series_movie_link_id,
        }) => Some(series_movie_link_id.clone()),
        _ => None,
    };
    let grabbed_fallback_path = (grabbed_episode_ids.len() == 1
        || grabbed_series_movie_link_id.is_some())
    .then(|| {
        video_files
            .iter()
            .max_by_key(|path| std::fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0))
            .cloned()
    })
    .flatten();

    // For each file, parse and attempt auto-match
    let mut previews = Vec::new();
    for path in &video_files {
        let file_name = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown")
            .to_string();
        let size = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);

        // File parsing is only for user-facing episode suggestions. It must
        // not become release/quality evidence for the later import.
        let parsed = parsed_release_from_file_stem(path);

        let mut suggested_episode_id = None;
        let mut suggested_episode_label = None;
        let mut parsed_season = None;
        let mut parsed_episodes = Vec::new();

        if let Some(ref ep_meta) = parsed.episode {
            parsed_season = ep_meta.season;
            parsed_episodes = ep_meta.episode_numbers.clone();

            let season_str = ep_meta
                .season
                .map(|s| s.to_string())
                .unwrap_or_else(|| "1".to_string());
            if let Some(ep_num) = ep_meta.episode_numbers.first() {
                let ep_str = ep_num.to_string();
                if let Ok(Some(episode)) = app
                    .services
                    .catalog
                    .shows
                    .find_episode_by_title_and_numbers(title_id, &season_str, &ep_str)
                    .await
                {
                    let label = format!(
                        "S{:02}E{:02}{}",
                        ep_meta.season.unwrap_or(1),
                        ep_num,
                        episode
                            .title
                            .as_ref()
                            .map(|t| format!(" - {}", t))
                            .unwrap_or_default()
                    );
                    suggested_episode_id = Some(episode.id.clone());
                    suggested_episode_label = Some(label);
                }
            }

            // Anime absolute fallback
            if suggested_episode_id.is_none()
                && let Some(abs) = ep_meta.absolute_episode
            {
                let abs_str = abs.to_string();
                if let Ok(Some(episode)) = app
                    .services
                    .catalog
                    .shows
                    .find_episode_by_title_and_absolute_number(title_id, &abs_str)
                    .await
                {
                    let label = format!(
                        "#{}{}",
                        abs,
                        episode
                            .title
                            .as_ref()
                            .map(|t| format!(" - {}", t))
                            .unwrap_or_default()
                    );
                    suggested_episode_id = Some(episode.id.clone());
                    suggested_episode_label = Some(label);
                }
            }
        }

        let scoped_suggestion = manual_episode_suggestion_for_grabbed_scope(
            suggested_episode_id.clone(),
            &grabbed_episode_ids,
            grabbed_fallback_path
                .as_ref()
                .is_some_and(|fallback| fallback == path),
        );
        if scoped_suggestion != suggested_episode_id {
            suggested_episode_label = scoped_suggestion.as_deref().and_then(|episode_id| {
                available_episodes
                    .iter()
                    .find(|episode| episode.id == episode_id)
                    .map(manual_import_episode_label)
            });
            suggested_episode_id = scoped_suggestion;
        }

        previews.push(ManualImportFilePreview {
            file_path: path_to_stored_string(path),
            file_name,
            size_bytes: size,
            quality: None,
            parsed_season,
            parsed_episodes,
            suggested_episode_id,
            suggested_episode_label,
            suggested_series_movie_link_id: grabbed_series_movie_link_id
                .clone()
                .filter(|_| {
                    grabbed_fallback_path
                        .as_ref()
                        .is_some_and(|fallback| fallback == path)
                }),
        });
    }

    Ok(ManualImportPreview { files: previews })
}

fn manual_episode_suggestion_for_grabbed_scope(
    parsed_suggestion: Option<String>,
    grabbed_episode_ids: &HashSet<String>,
    use_single_episode_fallback: bool,
) -> Option<String> {
    if grabbed_episode_ids.is_empty()
        || parsed_suggestion
            .as_ref()
            .is_some_and(|episode_id| grabbed_episode_ids.contains(episode_id))
    {
        return parsed_suggestion;
    }
    if use_single_episode_fallback && grabbed_episode_ids.len() == 1 {
        return grabbed_episode_ids.iter().next().cloned();
    }
    None
}

fn manual_import_episode_label(episode: &scryer_domain::Episode) -> String {
    episode.episode_label.clone().unwrap_or_else(|| {
        let season = episode.season_number.as_deref().unwrap_or("1");
        let number = episode.episode_number.as_deref().unwrap_or("?");
        format!(
            "S{season:0>2}E{number:0>2}{}",
            episode
                .title
                .as_ref()
                .map(|title| format!(" - {title}"))
                .unwrap_or_default()
        )
    })
}

/// Creates a durable, server-owned selection for files from a tracked completed download.
/// The caller receives opaque candidate IDs; canonical source paths remain in workflow storage.
pub async fn begin_manual_import_selection(
    app: &AppUseCase,
    actor: &User,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    title_id: &str,
) -> AppResult<ManualImportSelectionPreview> {
    let client_id = client_id.trim();
    let client_type = client_type.trim().to_ascii_lowercase();
    let source_ref = download_client_item_id.trim();
    let completed = resolve_current_manual_import_source(
        app,
        actor,
        client_id,
        &client_type,
        source_ref,
        title_id,
    )
    .await?;
    let release_evidence =
        resolve_release_evidence_for_completed_download(app, &completed, None).await?;
    if let Some(submission_title_id) = release_evidence.title_id()
        && submission_title_id != title_id
    {
        return Err(AppError::Validation(
            "manual import title does not match the Scryer submission that grabbed this download"
                .to_string(),
        ));
    }
    let release_evidence_json = serde_json::to_string(&release_evidence)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let trusted_root = std::fs::canonicalize(&completed.dest_dir)
        .map_err(|_| manual_import_source_unavailable())?;
    let source_identity = DownloadSourceIdentity::new(
        Some(&completed.client_id),
        &completed.client_type,
        &completed.download_client_item_id,
    );
    let selection_id = scryer_domain::Id::new().0;
    let prior_candidate_ids = app
        .services
        .workflow
        .imports
        .find_manual_import_selection(&actor.id, title_id, &source_identity)
        .await?
        .map(|selection| {
            selection
                .candidates
                .into_iter()
                .map(|candidate| (candidate.canonical_path, candidate.id))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();
    let (all_episodes, available_series_movies) =
        manual_import_preview_targets(app, title_id).await?;

    let mut candidates = Vec::new();
    let mut files = Vec::new();
    let preview = preview_manual_import(
        app,
        &completed,
        title_id,
        &release_evidence,
        &all_episodes,
    )
    .await?;
    for file in preview.files {
        let source_path = stored_path_to_path_buf(&file.file_path);
        validate_manual_import_source_under_trusted_root(&source_path, &trusted_root)?;
        let canonical_path =
            path_to_stored_string(&std::fs::canonicalize(&source_path).map_err(|error| {
                AppError::Validation(format!(
                    "manual import file is no longer accessible: {} ({error})",
                    source_path.display()
                ))
            })?);
        let candidate_id = prior_candidate_ids
            .get(&canonical_path)
            .cloned()
            .unwrap_or_else(|| scryer_domain::Id::new().0);
        candidates.push(crate::ManualImportSelectionCandidate {
            id: candidate_id.clone(),
            canonical_path: canonical_path.clone(),
        });
        files.push(ManualImportSelectionFilePreview {
            candidate_id,
            file_name: file.file_name,
            size_bytes: file.size_bytes,
            quality: file.quality,
            parsed_season: file.parsed_season,
            parsed_episodes: file.parsed_episodes,
            suggested_episode_id: file.suggested_episode_id,
            suggested_episode_label: file.suggested_episode_label,
            suggested_series_movie_link_id: file.suggested_series_movie_link_id,
        });
    }

    app.services
        .workflow
        .imports
        .replace_manual_import_selection(crate::ManualImportSelection {
            id: selection_id.clone(),
            actor_user_id: actor.id.clone(),
            title_id: title_id.to_string(),
            source_identity,
            release_evidence_json: Some(release_evidence_json),
            candidates,
        })
        .await?;

    Ok(ManualImportSelectionPreview {
        selection_id,
        files,
        available_episodes: all_episodes,
        available_series_movies,
    })
}
/// A user-specified mapping of one file to one manual import target.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportFileMapping {
    pub file_path: String,
    #[serde(default)]
    pub episode_id: Option<String>,
    #[serde(default)]
    pub series_movie_link_id: Option<String>,
}

#[derive(Clone, Copy)]
enum ManualImportMappingTarget<'a> {
    Episode(&'a str),
    SeriesMovie(&'a str),
    /// The title itself. A standalone movie has exactly one destination, so
    /// there is no sub-target to name.
    Movie,
}

fn normalize_manual_import_target(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Resolve which target a mapping addresses, given the facet of the title the
/// selection belongs to.
///
/// The facet matters because a MOVIE has no sub-target to name: its file maps
/// to the title. Requiring an `episode_id` or `series_movie_link_id`
/// unconditionally made movies unimportable through this path — the UI's
/// one-click action sends neither (there is nothing it could send) and the
/// request was rejected as invalid, so a completed movie awaiting manual
/// import had no action that could complete it.
fn manual_import_mapping_target<'a>(
    mapping: &'a ManualImportFileMapping,
    facet: &MediaFacet,
) -> AppResult<ManualImportMappingTarget<'a>> {
    let episode_id = normalize_manual_import_target(mapping.episode_id.as_deref());
    let series_movie_link_id =
        normalize_manual_import_target(mapping.series_movie_link_id.as_deref());

    match (episode_id, series_movie_link_id) {
        (Some(episode_id), None) => Ok(ManualImportMappingTarget::Episode(episode_id)),
        (None, Some(series_movie_link_id)) => {
            Ok(ManualImportMappingTarget::SeriesMovie(series_movie_link_id))
        }
        (None, None) if matches!(facet, MediaFacet::Movie) => Ok(ManualImportMappingTarget::Movie),
        (None, None) => Err(AppError::Validation(
            "manual import mapping requires episode_id or series_movie_link_id".to_string(),
        )),
        (Some(_), Some(_)) => Err(AppError::Validation(
            "manual import mapping cannot include both episode_id and series_movie_link_id"
                .to_string(),
        )),
    }
}

pub(crate) fn validate_manual_import_candidate_mapping_targets(
    files: &[ManualImportCandidateMapping],
    facet: &MediaFacet,
) -> AppResult<()> {
    if files.is_empty() {
        return Err(AppError::Validation(
            "at least one manual import candidate is required".to_string(),
        ));
    }
    let mut candidate_ids = std::collections::HashSet::new();
    for mapping in files {
        let candidate_id = mapping.candidate_id.trim();
        if candidate_id.is_empty() {
            return Err(AppError::Validation(
                "manual import candidate id is required".to_string(),
            ));
        }
        if !candidate_ids.insert(candidate_id) {
            return Err(AppError::Validation(
                "manual import candidate ids must be unique".to_string(),
            ));
        }
        let target = ManualImportFileMapping {
            file_path: String::new(),
            episode_id: mapping.episode_id.clone(),
            series_movie_link_id: mapping.series_movie_link_id.clone(),
        };
        manual_import_mapping_target(&target, facet)?;
    }
    Ok(())
}

pub(crate) async fn validate_manual_import_candidate_mapping_scope(
    app: &AppUseCase,
    title_id: &str,
    files: &[ManualImportCandidateMapping],
) -> AppResult<()> {
    for mapping in files {
        validate_manual_import_target_scope(
            app,
            title_id,
            mapping.episode_id.as_deref(),
            mapping.series_movie_link_id.as_deref(),
        )
        .await?;
    }

    Ok(())
}

async fn validate_manual_import_target_scope(
    app: &AppUseCase,
    title_id: &str,
    episode_id: Option<&str>,
    series_movie_link_id: Option<&str>,
) -> AppResult<()> {
    if let Some(episode_id) = normalize_manual_import_target(episode_id) {
        let episode = app
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "manual import episode target is unavailable: {episode_id}"
                ))
            })?;
        if episode.title_id != title_id {
            return Err(AppError::Validation(format!(
                "manual import episode target {episode_id} does not belong to title {title_id}"
            )));
        }
    }

    if let Some(series_movie_link_id) = normalize_manual_import_target(series_movie_link_id) {
        let link = app
            .services
            .catalog
            .shows
            .get_series_movie_link_by_id(series_movie_link_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "manual import series movie target is unavailable: {series_movie_link_id}"
                ))
            })?;
        if link.series_title_id != title_id {
            return Err(AppError::Validation(format!(
                "manual import series movie target {series_movie_link_id} does not belong to title {title_id}"
            )));
        }
    }

    Ok(())
}

/// Per-file result of a manual import execution.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportFileResult {
    pub file_path: String,
    #[serde(default)]
    pub episode_id: Option<String>,
    #[serde(default)]
    pub series_movie_link_id: Option<String>,
    pub success: bool,
    pub dest_path: Option<String>,
    pub error_code: Option<ImportErrorCode>,
    pub error_message: Option<String>,
}

fn manual_import_file_result(
    mapping: &ManualImportFileMapping,
    success: bool,
    dest_path: Option<String>,
    error_code: Option<ImportErrorCode>,
    error_message: Option<String>,
) -> ManualImportFileResult {
    ManualImportFileResult {
        file_path: mapping.file_path.clone(),
        episode_id: mapping.episode_id.clone(),
        series_movie_link_id: mapping.series_movie_link_id.clone(),
        success,
        dest_path,
        error_code,
        error_message,
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportRequestPayload {
    pub requested_by_user_id: Option<String>,
    pub title_id: Option<String>,
    pub download_client_item_id: String,
    #[serde(default)]
    pub client_id: Option<String>,
    pub client_type: String,
    #[serde(default)]
    pub files: Vec<ManualImportFileMapping>,
    #[serde(default)]
    pub selection_id: Option<String>,
    /// Persisted at queue time so a manual review of a Scryer grab cannot
    /// degrade to downloader display-name or filename evidence.
    #[serde(default)]
    pub(crate) release_evidence: Option<ReleaseEvidence>,
    pub requested_at: String,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportExecutionResult {
    pub import_id: String,
    pub client_type: String,
    pub download_client_item_id: String,
    pub title_id: Option<String>,
    pub status: ImportStatus,
    pub error_code: Option<ImportErrorCode>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub file_results: Vec<ManualImportFileResult>,
    pub completed_at: DateTime<Utc>,
}
fn manual_import_error_from_skip_reason(skip_reason: Option<ImportSkipReason>) -> ImportErrorCode {
    match skip_reason {
        Some(ImportSkipReason::DiskFull) => ImportErrorCode::DiskFull,
        Some(ImportSkipReason::PermissionDenied) => ImportErrorCode::PermissionDenied,
        Some(ImportSkipReason::PolicyMismatch) => ImportErrorCode::PolicyMismatch,
        _ => ImportErrorCode::Unknown,
    }
}
fn classify_manual_import_error_message(message: &str) -> ImportErrorCode {
    let normalized = message.trim().to_ascii_lowercase();
    if normalized.contains("file not found") {
        ImportErrorCode::FileNotFound
    } else if normalized.contains("episode not found") {
        ImportErrorCode::EpisodeNotFound
    } else if normalized.contains("episode lookup failed") {
        ImportErrorCode::EpisodeLookupFailed
    } else if normalized.contains("source_job_failed")
        || normalized.contains("source download failed")
        || normalized.contains("source job failed")
    {
        ImportErrorCode::SourceJobFailed
    } else if normalized.contains("permission denied")
        || normalized.contains("operation not permitted")
    {
        ImportErrorCode::PermissionDenied
    } else if normalized.contains("no space left")
        || normalized.contains("disk full")
        || normalized.contains("insufficient disk space")
    {
        ImportErrorCode::DiskFull
    } else if normalized.is_empty() {
        ImportErrorCode::Unknown
    } else {
        // String matching is only a fallback for unexpected error paths.
        // Known manual-import failures should be classified structurally at
        // the point where the skip reason or domain error is produced.
        ImportErrorCode::IoFailed
    }
}
pub(crate) fn manual_import_result_json(
    import_id: &str,
    payload: &ManualImportRequestPayload,
    status: ImportStatus,
    error_code: Option<ImportErrorCode>,
    error_message: Option<String>,
    file_results: Vec<ManualImportFileResult>,
) -> Option<String> {
    serde_json::to_string(&ManualImportExecutionResult {
        import_id: import_id.to_string(),
        client_type: payload.client_type.clone(),
        download_client_item_id: payload.download_client_item_id.clone(),
        title_id: payload.title_id.clone(),
        status,
        error_code,
        error_message,
        file_results,
        completed_at: Utc::now(),
    })
    .ok()
}
pub(crate) fn manual_import_source_failed_result_json(
    import_id: &str,
    payload: &ManualImportRequestPayload,
    message: String,
) -> Option<String> {
    manual_import_result_json(
        import_id,
        payload,
        ImportStatus::Failed,
        Some(ImportErrorCode::SourceJobFailed),
        Some(message),
        Vec::new(),
    )
}
pub(crate) fn manual_import_request_matches_source(
    payload: &ManualImportRequestPayload,
    client_id: Option<&str>,
    client_type: Option<&str>,
    download_client_item_id: &str,
) -> bool {
    if payload.download_client_item_id != download_client_item_id {
        return false;
    }

    let requested_client_id = client_id.map(str::trim).filter(|value| !value.is_empty());
    let payload_client_id = payload
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested_client_id != payload_client_id {
        return false;
    }

    let requested_client_type = client_type.map(str::trim).filter(|value| !value.is_empty());
    requested_client_type
        .is_none_or(|client_type| payload.client_type.eq_ignore_ascii_case(client_type))
}
pub(crate) async fn find_active_manual_import_for_source(
    app: &AppUseCase,
    client_id: Option<&str>,
    client_type: &str,
    download_client_item_id: &str,
) -> AppResult<Option<ImportRecord>> {
    let normalized_client_type = client_type.trim().to_lowercase();
    let source_ref = download_client_item_id.trim();
    if normalized_client_type.is_empty() || source_ref.is_empty() {
        return Ok(None);
    }

    let records = app
        .services
        .workflow
        .imports
        .list_imports_for_identities(&[DownloadSourceIdentity::new(
            client_id,
            normalized_client_type.as_str(),
            source_ref,
        )])
        .await?;

    Ok(records.into_iter().find(|record| {
        record.import_type == ImportType::ManualImport
            && record.status.is_active()
            && serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json)
                .ok()
                .is_some_and(|payload| {
                    manual_import_request_matches_source(
                        &payload,
                        client_id,
                        Some(normalized_client_type.as_str()),
                        source_ref,
                    )
                })
    }))
}
pub(crate) async fn fail_active_manual_import_for_source(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    failure_reason: &str,
) {
    let record = match find_active_manual_import_for_source(
        app,
        Some(tracked.client_id.as_str()),
        tracked.client_type.as_str(),
        tracked.client_item.download_client_item_id.as_str(),
    )
    .await
    {
        Ok(Some(record)) => record,
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(
                error = %error,
                item_id = %tracked.client_item.download_client_item_id,
                "failed to inspect manual import request for failed source"
            );
            return;
        }
    };

    let payload = serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json)
        .unwrap_or_else(|_| ManualImportRequestPayload {
            requested_by_user_id: None,
            title_id: tracked.title_id.clone(),
            download_client_item_id: tracked.client_item.download_client_item_id.clone(),
            client_id: Some(tracked.client_id.clone()).filter(|value| !value.is_empty()),
            client_type: tracked.client_type.clone(),
            files: Vec::new(),
            selection_id: None,
            release_evidence: None,
            requested_at: record.created_at.clone(),
        });
    let message = format!("source download failed before import: {failure_reason}");
    let result_json = manual_import_source_failed_result_json(&record.id, &payload, message);

    if let Err(error) = app
        .update_import_status_and_notify(&record.id, ImportStatus::Failed, result_json)
        .await
    {
        tracing::warn!(
            error = %error,
            import_id = %record.id,
            item_id = %tracked.client_item.download_client_item_id,
            "failed to terminate manual import request for failed source"
        );
    }
}
fn manual_import_terminal_status_and_error(
    results: &[ManualImportFileResult],
) -> (ImportStatus, Option<ImportErrorCode>, Option<String>) {
    let failures = results
        .iter()
        .filter(|result| !result.success)
        .collect::<Vec<_>>();
    if failures.is_empty() {
        return (ImportStatus::Completed, None, None);
    }

    let primary = failures[0];
    (
        ImportStatus::Failed,
        primary.error_code.or(Some(ImportErrorCode::Unknown)),
        primary.error_message.clone(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "manual series-movie import needs source, title, and resolved path context"
)]
async fn execute_manual_series_movie_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    title: &scryer_domain::Title,
    completed: Option<&CompletedDownload>,
    release_evidence: &ReleaseEvidence,
    source: &Path,
    mapping: &ManualImportFileMapping,
    series_movie_link_id: &str,
    full_folder_path: &Path,
    season_folder_template: &str,
    specials_folder_template: &str,
    rename_enabled: bool,
) -> AppResult<ManualImportFileResult> {
    let link = match app
        .services
        .catalog
        .shows
        .get_series_movie_link_by_id(series_movie_link_id)
        .await?
    {
        Some(link) if link.series_title_id == title.id => link,
        Some(_) => {
            return Ok(manual_import_file_result(
                mapping,
                false,
                None,
                Some(ImportErrorCode::Unknown),
                Some(format!(
                    "series movie link {series_movie_link_id} does not belong to title {}",
                    title.id
                )),
            ));
        }
        None => {
            return Ok(manual_import_file_result(
                mapping,
                false,
                None,
                Some(ImportErrorCode::Unknown),
                Some(format!(
                    "series movie link {series_movie_link_id} not found"
                )),
            ));
        }
    };

    let parsed = build_augmented_movie_import_metadata(source, release_evidence);
    let ext = scryer_domain::canonical_video_extension(source)
        .unwrap_or("mkv")
        .to_string();
    let linked_episode = if let Some(linked_episode_id) = link.linked_episode_id.as_deref() {
        app.services
            .catalog
            .shows
            .get_episode_by_id(linked_episode_id)
            .await?
    } else {
        None
    };
    let season_episode = linked_episode
        .as_ref()
        .and_then(|episode| {
            let season = episode.season_number.as_deref()?.parse::<i32>().ok()?;
            let episode_number = episode.episode_number.as_deref()?.parse::<i32>().ok()?;
            Some(format!("S{season:02}E{episode_number:02}"))
        })
        .unwrap_or_else(|| "S00E00".to_string());
    let rendered_filename = if rename_enabled {
        sanitize_filesystem_component(&format!(
            "{} - {} - {}.{}",
            title.name, season_episode, link.movie.title, ext
        ))
    } else {
        preserved_import_filename(source)
    };
    let dest_path = episodic_import_parent_path(
        title,
        full_folder_path,
        season_folder_template,
        specials_folder_template,
        0,
    )
    .join(rendered_filename);
    persist_title_folder_path_if_missing(app, title, full_folder_path).await?;
    if let Some(parent) = dest_path.parent()
        && let Err(error) = tokio::fs::create_dir_all(parent).await
    {
        return Ok(manual_import_file_result(
            mapping,
            false,
            None,
            Some(classify_manual_import_error_message(&error.to_string())),
            Some(format!(
                "failed to create destination directory {}: {error}",
                parent.display()
            )),
        ));
    }

    let import_mode = app
        .resolve_import_mode(Some(&title.library_id), &title.facet)
        .await?;
    let file_result = match import_file_with_record_progress(
        app,
        import_id,
        &title.library_id,
        &title.facet,
        source,
        &dest_path,
        import_mode,
        None,
    )
    .await
    {
        Ok(file_result) => file_result,
        Err(error) => {
            let message = error.to_string();
            return Ok(manual_import_file_result(
                mapping,
                false,
                None,
                Some(classify_manual_import_error_message(&message)),
                Some(message),
            ));
        }
    };
    let quality_label = parsed.quality.clone();
    let started_at = Utc::now();
    let imported_media_file_id = match app
        .services
        .library
        .media_files
        .insert_media_file(&crate::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: path_to_stored_string(&dest_path),
            size_bytes: file_result.size_bytes as i64,
            role: crate::MediaFileRole::Primary,
            quality_label: quality_label.clone(),
            scene_name: Some(parsed.raw_title.clone()),
            release_group: parsed.release_group.clone(),
            source_type: crate::release_parser::parsed_release_source_type(&parsed),
            resolution: quality_label,
            video_codec_parsed: parsed.video_codec,
            audio_codec_parsed: parsed.audio.as_ref().map(ToString::to_string),
            audio_channels_parsed: parsed.audio_channels.clone(),
            original_file_path: Some(path_to_stored_string(source)),
            grabbed_release_title: release_evidence.release_title(Some(source)),
            grabbed_at: Some(started_at.to_rfc3339()),
            edition: parsed.edition.clone(),
            ..Default::default()
        })
        .await
    {
        Ok(file_id) => file_id,
        Err(error) => {
            let message = error.to_string();
            return Ok(manual_import_file_result(
                mapping,
                false,
                Some(path_to_stored_string(&dest_path)),
                Some(classify_manual_import_error_message(&message)),
                Some(message),
            ));
        }
    };

    app.services
        .library
        .media_files
        .link_file_to_series_movie(&imported_media_file_id, series_movie_link_id)
        .await?;
    if let Some(linked_episode_id) = link.linked_episode_id.as_deref() {
        app.services
            .library
            .media_files
            .link_file_to_episode(&imported_media_file_id, linked_episode_id)
            .await?;
        app.services
            .library
            .media_files
            .set_media_file_roles_for_episode(
                &title.id,
                linked_episode_id,
                &imported_media_file_id,
                &[],
            )
            .await?;
    }

    analyze_and_persist_imported_media_file(app, &title.id, &imported_media_file_id, &dest_path)
        .await;
    if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
        app,
        &title.id,
        &imported_media_file_id,
        None,
        &dest_path,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            title_id = %title.id,
            file_id = %imported_media_file_id,
            dest_path = %dest_path.display(),
            "failed to reconcile external subtitles after manual series movie import"
        );
    }
    maybe_trigger_subtitle_search(app, &title.id, &imported_media_file_id);
    if let Err(error) =
        finalize_import_source_cleanup(app, import_mode, &file_result, &dest_path).await
    {
        let message = error.to_string();
        return Ok(manual_import_file_result(
            mapping,
            false,
            Some(path_to_stored_string(&dest_path)),
            Some(classify_manual_import_error_message(&message)),
            Some(message),
        ));
    }

    if let Some(completed) = completed {
        let linked_episode_artifacts = linked_episode.iter().cloned().collect::<Vec<_>>();
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            source,
            "movie",
            "imported",
            None,
            Some(imported_media_file_id.as_str()),
            &linked_episode_artifacts,
        )
        .await;
    }

    let nfo_enabled = app
        .resolve_nfo_write_on_import(Some(&title.library_id), &title.facet)
        .await?;
    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        let nfo_content = crate::nfo::render_series_movie_episode_nfo(
            &link.movie,
            &season_episode,
            link.after_season,
        );
        if let Err(error) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
            tracing::warn!(
                error = %error,
                path = %nfo_path.display(),
                "failed to write manual series movie NFO sidecar"
            );
        }
    }

    mark_wanted_completed_for_series_movie_link(app, &title.id, series_movie_link_id, None).await;
    spawn_post_processing(PostProcessingContext {
        app: app.clone(),
        actor: crate::domain_events::DomainEventActor::from(actor),
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        dest_path: dest_path.clone(),
        year: title.year,
        imdb_id: title
            .external_ids
            .iter()
            .find(|external_id| external_id.source == "imdb")
            .map(|external_id| external_id.value.clone()),
        tvdb_id: title
            .external_ids
            .iter()
            .find(|external_id| external_id.source == "tvdb")
            .map(|external_id| external_id.value.clone()),
        season: None,
        episode: None,
        quality: parsed.quality.clone(),
    });

    Ok(manual_import_file_result(
        mapping,
        true,
        Some(path_to_stored_string(&dest_path)),
        None,
        None,
    ))
}

/// Execute a manual import: import each file with user-specified episode mappings.
pub async fn execute_manual_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    title_id: &str,
    completed: Option<&CompletedDownload>,
    files: Vec<ManualImportFileMapping>,
    trusted_source_root: Option<PathBuf>,
) -> AppResult<Vec<ManualImportFileResult>> {
    let release_evidence = match completed {
        Some(completed) => {
            resolve_release_evidence_for_completed_download(app, completed, None).await?
        }
        None => ReleaseEvidence::DownloaderObservation { nzb_name: None },
    };
    execute_manual_import_with_release_evidence(
        app,
        actor,
        import_id,
        title_id,
        completed,
        &release_evidence,
        files,
        trusted_source_root,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "manual execution carries explicit user mappings, trusted root, and durable release evidence"
)]
pub(crate) async fn execute_manual_import_with_release_evidence(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    title_id: &str,
    completed: Option<&CompletedDownload>,
    release_evidence: &ReleaseEvidence,
    files: Vec<ManualImportFileMapping>,
    trusted_source_root: Option<PathBuf>,
) -> AppResult<Vec<ManualImportFileResult>> {
    if let Some(submission_title_id) = release_evidence.title_id()
        && submission_title_id != title_id
    {
        return Err(AppError::Validation(
            "manual import title does not match the Scryer submission that grabbed this download"
                .to_string(),
        ));
    }
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title not found: {}", title_id)))?;
    app.require_library_permission(
        actor,
        &title.library_id,
        scryer_domain::LibraryPermission::ResolveImports,
    )
    .await?;
    for mapping in &files {
        validate_manual_import_target_scope(
            app,
            &title.id,
            mapping.episode_id.as_deref(),
            mapping.series_movie_link_id.as_deref(),
        )
        .await?;
    }
    let trusted_source_root = trusted_source_root
        .as_deref()
        .ok_or_else(|| AppError::Validation("manual import source root is required".to_string()))?;
    let trusted_source_root = std::fs::canonicalize(trusted_source_root).map_err(|error| {
        AppError::Validation(format!(
            "manual import source root is not accessible: {} ({error})",
            trusted_source_root.display()
        ))
    })?;

    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
        specials_folder_template,
    } = resolve_import_paths(app, &title).await?;
    let full_folder_path = effective_title_folder_path(&media_root, &title, &folder_template, None);
    ensure_import_title_folder_available(app, &title, &full_folder_path).await?;
    let quality_profile = resolve_import_quality_profile(app, &title).await?;

    let mut results = Vec::new();

    for mapping in &files {
        let source = stored_path_to_path_buf(&mapping.file_path);
        if let Err(err) =
            validate_manual_import_source_under_trusted_root(&source, &trusted_source_root)
        {
            results.push(manual_import_file_result(
                mapping,
                false,
                None,
                Some(classify_manual_import_error_message(&err.to_string())),
                Some(err.to_string()),
            ));
            continue;
        }

        // Validate file exists
        if !source.exists() || !source.is_file() {
            results.push(manual_import_file_result(
                mapping,
                false,
                None,
                Some(ImportErrorCode::FileNotFound),
                Some(format!("file not found: {}", mapping.file_path)),
            ));
            continue;
        }

        let target = match manual_import_mapping_target(mapping, &title.facet) {
            Ok(target) => target,
            Err(err) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(ImportErrorCode::Unknown),
                    Some(err.to_string()),
                ));
                continue;
            }
        };

        let episode_id = match target {
            ManualImportMappingTarget::Episode(episode_id) => episode_id,
            ManualImportMappingTarget::Movie => {
                // Reuse the canonical movie import rather than re-deriving
                // destination and naming here: a manually chosen file must land
                // exactly where the automatic path would have put it, or the
                // same movie ends up named two different ways depending on how
                // it was imported.
                // The canonical movie import derives naming and metadata from
                // the completed download, so it needs one. Every path that can
                // produce a Movie target today resolves the source download
                // first; report rather than unwrap, so a future caller without
                // one gets a message instead of a panic.
                let Some(completed) = completed else {
                    results.push(manual_import_file_result(
                        mapping,
                        false,
                        None,
                        Some(ImportErrorCode::Unknown),
                        Some(
                            "manual movie import requires the completed download context"
                                .to_string(),
                        ),
                    ));
                    continue;
                };
                let result = import_movie_download(
                    app,
                    actor,
                    &title,
                    import_id,
                    completed,
                    release_evidence,
                    std::slice::from_ref(&source),
                    Utc::now(),
                )
                .await;
                let file_result = match result {
                    Ok(import_result) => {
                        let success = import_result.dest_path.is_some()
                            && import_result.error_message.is_none();
                        manual_import_file_result(
                            mapping,
                            success,
                            import_result.dest_path,
                            (!success).then_some(ImportErrorCode::Unknown),
                            import_result.error_message,
                        )
                    }
                    Err(error) => {
                        let message = error.to_string();
                        manual_import_file_result(
                            mapping,
                            false,
                            None,
                            Some(classify_manual_import_error_message(&message)),
                            Some(message),
                        )
                    }
                };
                results.push(file_result);
                continue;
            }
            ManualImportMappingTarget::SeriesMovie(series_movie_link_id) => {
                let result = execute_manual_series_movie_import(
                    app,
                    actor,
                    import_id,
                    &title,
                    completed,
                    release_evidence,
                    &source,
                    mapping,
                    series_movie_link_id,
                    &full_folder_path,
                    &season_folder_template,
                    &specials_folder_template,
                    rename_enabled,
                )
                .await?;
                results.push(result);
                continue;
            }
        };

        // Look up episode
        let episode = match app
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await
        {
            Ok(Some(ep)) => ep,
            Ok(None) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(ImportErrorCode::EpisodeNotFound),
                    Some(format!("episode not found: {episode_id}")),
                ));
                continue;
            }
            Err(err) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(ImportErrorCode::EpisodeLookupFailed),
                    Some(format!("episode lookup failed: {}", err)),
                ));
                continue;
            }
        };
        if episode.title_id != title.id {
            results.push(manual_import_file_result(
                mapping,
                false,
                None,
                Some(ImportErrorCode::EpisodeNotFound),
                Some(format!(
                    "episode {episode_id} does not belong to title {}",
                    title.id
                )),
            ));
            continue;
        }

        // Parse release metadata for quality/codec tokens
        let parsed = build_augmented_episode_import_metadata(&source, release_evidence);

        let season_num: u32 = episode
            .season_number
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let ep_num_str = episode.episode_number.clone().unwrap_or_default();
        match execute_resolved_episode_import(
            app,
            actor,
            &title,
            import_id,
            rename_enabled,
            &rename_template,
            &season_folder_template,
            &specials_folder_template,
            &full_folder_path,
            &source,
            &parsed,
            std::slice::from_ref(&episode),
            std::slice::from_ref(&episode),
            season_num,
            &ep_num_str,
            episode.absolute_number.as_deref(),
            episode.title.as_deref(),
            &quality_profile,
            None,
            crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck,
            false,
        )
        .await
        {
            Ok(EpisodeImportOutcome::Imported {
                dest_path,
                imported_media_file_id,
                reason_code,
                ..
            }) => {
                if let Some(completed) = completed {
                    persist_file_import_artifact(
                        app,
                        import_id,
                        completed,
                        title.id.as_str(),
                        &source,
                        "episode",
                        "imported",
                        reason_code.as_deref(),
                        imported_media_file_id.as_deref(),
                        std::slice::from_ref(&episode),
                    )
                    .await;
                }
                results.push(manual_import_file_result(
                    mapping,
                    true,
                    Some(dest_path),
                    None,
                    None,
                ));
            }
            Ok(EpisodeImportOutcome::Skipped {
                message,
                skip_reason,
                ..
            }) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(manual_import_error_from_skip_reason(skip_reason.clone())),
                    Some(message),
                ));
            }
            Ok(EpisodeImportOutcome::Rejected { rejection, .. }) => {
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(manual_import_error_from_skip_reason(
                        rejection.skip_reason.clone(),
                    )),
                    Some(rejection.message),
                ));
            }
            Err(err) => {
                let error_message = err.to_string();
                results.push(manual_import_file_result(
                    mapping,
                    false,
                    None,
                    Some(classify_manual_import_error_message(&error_message)),
                    Some(error_message),
                ));
            }
        }
    }

    let imported_updates: Vec<NotificationMediaUpdate> = results
        .iter()
        .filter(|result| result.success)
        .filter_map(|result| {
            result
                .dest_path
                .as_ref()
                .map(|path| NotificationMediaUpdate::created(path.clone()))
        })
        .collect();

    let success_count = results.iter().filter(|r| r.success).count();
    let (terminal_status, _, _) = manual_import_terminal_status_and_error(&results);
    if success_count > 0 && terminal_status == ImportStatus::Completed {
        let episode_ids = results
            .iter()
            .filter(|result| result.success)
            .filter_map(|result| result.episode_id.clone())
            .collect::<Vec<_>>();
        app.append_domain_event(new_title_domain_event(
            actor,
            &title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(&title),
                media_updates: imported_updates
                    .into_iter()
                    .map(|update| created_media_update(update.path))
                    .collect(),
                imported_count: success_count as i32,
                import_id: None,
                source_system: completed.map(|download| download.client_type.clone()),
                source_ref: completed.map(|download| download.download_client_item_id.clone()),
                source_title: release_evidence.release_title(
                    files
                        .first()
                        .map(|mapping| Path::new(mapping.file_path.as_str())),
                ),
                source_path: (files.len() == 1).then(|| files[0].file_path.clone()),
                dest_path: results
                    .iter()
                    .find(|result| result.success)
                    .and_then(|result| result.dest_path.clone()),
                quality: None,
                episode_ids,
            }),
        ))
        .await?;
    }

    Ok(results)
}

async fn recover_completed_manual_imports(app: &AppUseCase, worker: &PollingWorker) {
    let records = match app
        .services
        .workflow
        .imports
        .list_completed_manual_imports(MANUAL_IMPORT_RECOVERY_BATCH_SIZE)
        .await
    {
        Ok(records) => records,
        Err(error) => {
            worker.warn_error("list_completed_manual_imports", &error);
            return;
        }
    };
    let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() else {
        return;
    };

    for record in records {
        let Some(recovery) = completed_manual_import_recovery(&record) else {
            continue;
        };
        let source_identity = recovery.source_identity;
        match handle
            .mark_imported_if_nonterminal(source_identity.clone())
            .await
        {
            Ok(true) => {
                if let Err(error) = app
                    .services
                    .workflow
                    .imports
                    .delete_manual_import_selections_for_source(&source_identity)
                    .await
                {
                    worker.warn_error("cleanup_recovered_manual_import_selection", &error);
                }
            }
            Ok(false) => {}
            Err(error) => worker.warn_error("recover_completed_manual_import", &error),
        }
    }
}

struct CompletedManualImportRecovery {
    source_identity: DownloadSourceIdentity,
}

fn completed_manual_import_recovery(
    record: &ImportRecord,
) -> Option<CompletedManualImportRecovery> {
    if record.import_type != ImportType::ManualImport || record.status != ImportStatus::Completed {
        return None;
    }
    let result =
        serde_json::from_str::<ManualImportExecutionResult>(record.result_json.as_deref()?).ok()?;
    if result.import_id != record.id
        || result.status != ImportStatus::Completed
        || result.file_results.is_empty()
        || result.file_results.iter().any(|file| !file.success)
        || !result
            .client_type
            .eq_ignore_ascii_case(&record.source_system)
        || result.download_client_item_id != record.source_ref
        || record.source_system.trim().is_empty()
        || record.source_ref.trim().is_empty()
    {
        return None;
    }
    let client_id = record
        .source_client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(CompletedManualImportRecovery {
        source_identity: DownloadSourceIdentity::new(
            Some(client_id),
            &record.source_system,
            &record.source_ref,
        ),
    })
}

struct QueuedManualImportOutcome {
    status: ImportStatus,
    result_json: Option<String>,
    files_imported_this_pass: usize,
    completed: Option<CompletedDownload>,
    title_id: Option<String>,
    expected_mapping_count: Option<usize>,
    prior_import_proven: bool,
}

impl QueuedManualImportOutcome {
    fn source_unavailable(import_id: &str, payload: &ManualImportRequestPayload) -> Self {
        Self {
            status: ImportStatus::Failed,
            result_json: manual_import_source_failed_result_json(
                import_id,
                payload,
                MANUAL_IMPORT_SOURCE_UNAVAILABLE.to_string(),
            ),
            files_imported_this_pass: 0,
            completed: None,
            title_id: payload.title_id.clone(),
            expected_mapping_count: None,
            prior_import_proven: false,
        }
    }

    fn already_imported(import_id: &str, payload: &ManualImportRequestPayload) -> Self {
        let now = Utc::now();
        let result = scryer_domain::ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::AlreadyImported),
            title_id: payload.title_id.clone(),
            source_system: Some(payload.client_type.clone()),
            source_ref: Some(payload.download_client_item_id.clone()),
            source_title: None,
            source_path: String::new(),
            dest_path: None,
            quality: None,
            episode_ids: Vec::new(),
            file_size_bytes: None,
            link_type: None,
            error_message: None,
            started_at: now,
            completed_at: now,
        };
        Self {
            status: ImportStatus::Skipped,
            result_json: serde_json::to_string(&result).ok(),
            files_imported_this_pass: 0,
            completed: None,
            title_id: payload.title_id.clone(),
            expected_mapping_count: None,
            prior_import_proven: true,
        }
    }
}

async fn execute_queued_manual_import_with_outcome(
    app: &AppUseCase,
    import_id: &str,
    payload: &ManualImportRequestPayload,
) -> AppResult<QueuedManualImportOutcome> {
    let user_id = payload
        .requested_by_user_id
        .as_deref()
        .ok_or_else(|| AppError::Validation("manual import request is missing an actor".into()))?;
    let actor = app
        .services
        .identity
        .users
        .get_by_id(user_id)
        .await?
        .ok_or_else(|| {
            AppError::Validation("manual import request actor no longer exists".into())
        })?;

    app.update_import_status_and_notify(import_id, ImportStatus::Processing, None)
        .await?;

    let Some(title_id) = payload
        .title_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(QueuedManualImportOutcome::source_unavailable(
            import_id, payload,
        ));
    };
    let Some(client_id) = payload
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(QueuedManualImportOutcome::source_unavailable(
            import_id, payload,
        ));
    };
    let client_type = payload.client_type.trim();
    let download_client_item_id = payload.download_client_item_id.trim();
    let authorized_source = match authorize_manual_import_source(
        app,
        &actor,
        client_id,
        client_type,
        download_client_item_id,
        title_id,
    )
    .await
    {
        Ok(source) => source,
        Err(_) => {
            return Ok(QueuedManualImportOutcome::source_unavailable(
                import_id, payload,
            ));
        }
    };
    if manual_import_source_was_already_imported(app, &authorized_source, import_id).await? {
        return Ok(QueuedManualImportOutcome::already_imported(
            import_id, payload,
        ));
    }
    let source_identity = authorized_source.identity;
    let completed = match resolve_authorized_manual_import_source(app, &source_identity).await {
        Ok(completed) => completed,
        Err(_) => {
            return Ok(QueuedManualImportOutcome::source_unavailable(
                import_id, payload,
            ));
        }
    };
    let release_evidence = match payload.release_evidence.clone() {
        Some(release_evidence) => release_evidence,
        None => resolve_release_evidence_for_completed_download(app, &completed, None).await?,
    };
    if let Some(submission_title_id) = release_evidence.title_id()
        && submission_title_id != title_id
    {
        return Err(AppError::Validation(
            "manual import title does not match the Scryer submission that grabbed this download"
                .to_string(),
        ));
    }
    let trusted_source_root = match std::fs::canonicalize(&completed.dest_dir) {
        Ok(root) => root,
        Err(_) => {
            return Ok(QueuedManualImportOutcome::source_unavailable(
                import_id, payload,
            ));
        }
    };
    let expected_mapping_count =
        (!payload.files.is_empty()).then_some(payload.files.len());

    if payload.files.is_empty() {
        let result = import_completed_download_for_manual_review_with_title_override(
            app,
            &actor,
            &completed,
            title_id,
            true,
            Some(&release_evidence),
        )
        .await?;
        let imported = matches!(result.decision, ImportDecision::Imported);

        let (status, error_code, error_message) =
            if imported || matches!(result.skip_reason, Some(ImportSkipReason::AlreadyImported)) {
                (ImportStatus::Completed, None, None)
            } else {
                (
                    ImportStatus::Failed,
                    Some(manual_import_error_from_skip_reason(
                        result.skip_reason.clone(),
                    )),
                    result.error_message.clone(),
                )
            };

        let result_json = if status == ImportStatus::Completed && error_code.is_none() {
            serde_json::to_string(&result).ok()
        } else {
            manual_import_result_json(
                import_id,
                payload,
                status,
                error_code,
                error_message,
                Vec::new(),
            )
        };

        return Ok(QueuedManualImportOutcome {
            status,
            result_json,
            files_imported_this_pass: usize::from(imported),
            completed: Some(completed),
            title_id: result.title_id.or_else(|| Some(title_id.to_string())),
            expected_mapping_count,
            prior_import_proven: false,
        });
    }

    let results = execute_manual_import_with_release_evidence(
        app,
        &actor,
        import_id,
        title_id,
        Some(&completed),
        &release_evidence,
        payload.files.clone(),
        Some(trusted_source_root),
    )
    .await?;
    let files_imported_this_pass = results.iter().filter(|result| result.success).count();
    let (status, error_code, error_message) = manual_import_terminal_status_and_error(&results);

    if status == ImportStatus::Completed
        && let Err(error) = app
            .services
            .workflow
            .imports
            .delete_manual_import_selections_for_source(&source_identity)
            .await
    {
        tracing::warn!(
            error = %error,
            item_id = %source_identity.item_id,
            "failed to clean up terminal manual-import selections"
        );
    }

    let result_json = manual_import_result_json(
        import_id,
        payload,
        status,
        error_code,
        error_message,
        results,
    );

    Ok(QueuedManualImportOutcome {
        status,
        result_json,
        files_imported_this_pass,
        completed: Some(completed),
        title_id: Some(title_id.to_string()),
        expected_mapping_count,
        prior_import_proven: false,
    })
}

pub async fn execute_queued_manual_import(
    app: &AppUseCase,
    import_id: &str,
    payload: &ManualImportRequestPayload,
) -> AppResult<(ImportStatus, Option<String>)> {
    let _import_permit = app.runtime.imports.execution_coordinator.acquire().await;
    let outcome = execute_queued_manual_import_with_outcome(app, import_id, payload).await?;
    Ok((outcome.status, outcome.result_json))
}

#[cfg(test)]
mod manual_import_recovery_tests {
    use super::*;

    fn completed_manual_import_record(client_type: &str, file_success: bool) -> ImportRecord {
        let id = "manual-import-1";
        let result = ManualImportExecutionResult {
            import_id: id.to_string(),
            client_type: client_type.to_string(),
            download_client_item_id: "download-1".to_string(),
            title_id: Some("title-1".to_string()),
            status: ImportStatus::Completed,
            error_code: None,
            error_message: None,
            file_results: vec![ManualImportFileResult {
                file_path: "/downloads/episode.mkv".to_string(),
                episode_id: Some("episode-1".to_string()),
                series_movie_link_id: None,
                success: file_success,
                dest_path: file_success.then(|| "/library/episode.mkv".to_string()),
                error_code: None,
                error_message: None,
            }],
            completed_at: Utc::now(),
        };
        ImportRecord {
            id: id.to_string(),
            source_client_id: Some("client-1".to_string()),
            source_system: client_type.to_string(),
            source_ref: "download-1".to_string(),
            import_type: ImportType::ManualImport,
            status: ImportStatus::Completed,
            payload_json: "{}".to_string(),
            result_json: Some(serde_json::to_string(&result).expect("result JSON")),
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            started_at: None,
            finished_at: None,
            created_at: "2026-08-17T00:00:00Z".to_string(),
            updated_at: "2026-08-17T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn completed_manual_import_recovery_uses_generic_client_identity() {
        for client_type in ["nzbget", "qbittorrent"] {
            let record = completed_manual_import_record(client_type, true);

            let recovery = completed_manual_import_recovery(&record)
                .expect("completed all-success record should recover");
            let identity = recovery.source_identity;

            assert_eq!(identity.client_id.as_deref(), Some("client-1"));
            assert_eq!(identity.client_type, client_type);
            assert_eq!(identity.item_id, "download-1");
        }
    }

    #[test]
    fn completed_manual_import_recovery_rejects_partial_failed_or_malformed_records() {
        let failed = completed_manual_import_record("nzbget", false);
        assert!(completed_manual_import_recovery(&failed).is_none());

        let mut empty = completed_manual_import_record("nzbget", true);
        let mut result = serde_json::from_str::<ManualImportExecutionResult>(
            empty.result_json.as_deref().expect("result JSON"),
        )
        .expect("parse result JSON");
        result.file_results.clear();
        empty.result_json = Some(serde_json::to_string(&result).expect("result JSON"));
        assert!(completed_manual_import_recovery(&empty).is_none());

        let mut malformed = completed_manual_import_record("nzbget", true);
        malformed.result_json = Some("not JSON".to_string());
        assert!(completed_manual_import_recovery(&malformed).is_none());

        let mut stale = completed_manual_import_record("nzbget", true);
        stale.source_client_id = Some(" ".to_string());
        assert!(completed_manual_import_recovery(&stale).is_none());
    }

    #[test]
    fn completed_manual_import_recovery_only_requires_the_queued_mappings() {
        let record = completed_manual_import_record("nzbget", true);
        assert!(completed_manual_import_recovery(&record).is_some());
    }
}

#[cfg(all(test, unix))]
mod manual_source_validation_tests {
    use super::*;

    #[test]
    fn manual_import_source_validation_accepts_symlink_with_path_and_target_under_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("downloads");
        std::fs::create_dir_all(&root).expect("create root");
        let target = root.join("movie.mkv");
        std::fs::write(&target, b"video").expect("write target");
        let link = root.join("linked.mkv");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let trusted_root = std::fs::canonicalize(&root).expect("canonical root");

        validate_manual_import_source_under_trusted_root(&link, &trusted_root)
            .expect("symlink inside root should validate");
    }

    #[test]
    fn manual_import_source_validation_rejects_symlink_path_outside_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("downloads");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&outside).expect("create outside");
        let target = root.join("movie.mkv");
        std::fs::write(&target, b"video").expect("write target");
        let link = outside.join("linked.mkv");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let trusted_root = std::fs::canonicalize(&root).expect("canonical root");

        let error = validate_manual_import_source_under_trusted_root(&link, &trusted_root)
            .expect_err("symlink path outside root should be rejected");

        assert!(
            error
                .to_string()
                .contains("file path is outside the trusted source root")
        );
    }

    #[test]
    fn manual_import_source_validation_rejects_symlink_target_outside_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("downloads");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&outside).expect("create outside");
        let target = outside.join("movie.mkv");
        std::fs::write(&target, b"video").expect("write target");
        let link = root.join("linked.mkv");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let trusted_root = std::fs::canonicalize(&root).expect("canonical root");

        let error = validate_manual_import_source_under_trusted_root(&link, &trusted_root)
            .expect_err("symlink target outside root should be rejected");

        assert!(
            error
                .to_string()
                .contains("file is outside the trusted source root")
        );
    }
}
