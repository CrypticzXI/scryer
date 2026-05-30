async fn remap_completed_download_for_client(app: &AppUseCase, completed: &mut CompletedDownload) {
    let client_id = completed.client_id.trim();
    if client_id.is_empty() {
        return;
    }

    let config = match app
        .services
        .integrations
        .download_client_configs
        .get_by_id(client_id)
        .await
    {
        Ok(Some(config)) => config,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                client_id,
                error = %error,
                "import: failed to load download client config for remote path mapping"
            );
            return;
        }
    };

    match parse_download_client_remote_path_mappings(&config.config_json) {
        Ok(mappings) => apply_remote_path_mappings_to_completed_download(completed, &mappings),
        Err(error) => {
            tracing::warn!(
                client_id,
                error = %error,
                "import: failed to parse remote path mappings"
            );
        }
    }
}
/// Attempts to import completed items from the current queue/history snapshot.
/// Returns the set of `download_client_item_id`s that were conclusively processed
/// (imported, failed permanently, or intentionally ignored). Temporary defer
/// conditions (e.g. no matching CompletedDownload yet, empty dest_dir) are NOT
/// included so they can be retried on the next snapshot.
pub async fn try_import_completed_downloads(
    app: &AppUseCase,
    actor: &User,
    items: &[DownloadQueueItem],
) -> HashSet<String> {
    // TODO: increase to 600 (10 minutes) for production — large NAS copies can take a while
    match app
        .services
        .workflow
        .imports
        .recover_stale_processing_imports(120)
        .await
    {
        Ok(recovered) if recovered > 0 => {
            tracing::warn!(recovered, "recovered stale processing imports → failed");
            app.emit_import_recovery_completed_event(
                Some(actor.id.clone()),
                i64::try_from(recovered).unwrap_or(i64::MAX),
            )
            .await;
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to recover stale processing imports");
        }
        _ => {}
    }

    let completed_items: Vec<&DownloadQueueItem> = items
        .iter()
        .filter(|item| item.state == DownloadQueueState::Completed)
        .filter(|item| {
            item.import_status.is_none() || item.import_status == Some(ImportStatus::Failed)
        })
        .collect();

    if completed_items.is_empty() {
        return HashSet::new();
    }

    let mut processed_ids: HashSet<String> = HashSet::new();

    tracing::info!(
        count = completed_items.len(),
        items = %completed_items.iter().map(|i| format!("{}({})", i.title_name, i.download_client_item_id)).collect::<Vec<_>>().join(", "),
        "import: found completed items to evaluate"
    );

    let completed_downloads = match app
        .services
        .integrations
        .download_client
        .list_completed_downloads()
        .await
    {
        Ok(downloads) => {
            tracing::debug!(
                count = downloads.len(),
                ids = %downloads.iter().map(|d| d.download_client_item_id.as_str()).collect::<Vec<_>>().join(", "),
                "import: fetched completed downloads from client"
            );
            downloads
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to fetch completed downloads for import");
            return HashSet::new();
        }
    };

    let completed_downloads_by_identity = completed_downloads
        .iter()
        .map(|completed| (completed_download_identity(completed), completed))
        .collect::<HashMap<_, _>>();

    for item in completed_items {
        let source_ref = &item.download_client_item_id;
        let item_identity = DownloadSourceIdentity::new(
            Some(item.client_id.as_str()),
            &item.client_type,
            &item.download_client_item_id,
        );
        let already_imported = match app
            .services
            .workflow
            .imports
            .is_already_imported(&item_identity)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(error = %error, source_ref = %source_ref, "import dedup check failed");
                continue;
            }
        };

        // Find the matching CompletedDownload
        let completed = match completed_downloads_by_identity.get(&item_identity).copied() {
            Some(completed) => completed,
            None => {
                tracing::debug!(
                    source_ref = %source_ref,
                    title = %item.title_name,
                    "import: no matching CompletedDownload from client history (item may still be processing or status != Completed)"
                );
                continue;
            }
        };

        // Only auto-import downloads that originated from scryer.
        // NZBGet embeds *scryer_title_id via PPParameters. SABnzbd has no
        // equivalent, so we fall back to the download_submissions table which
        // records the (title_id, facet) at grab time.
        let completed = if has_scryer_origin(&completed.parameters) {
            completed.clone()
        } else {
            // Fallback: look up the download_submissions table
            match app
                .services
                .workflow
                .download_submissions
                .find_by_client_item_id(&DownloadSourceIdentity::new(
                    Some(completed.client_id.as_str()),
                    &completed.client_type,
                    &completed.download_client_item_id,
                ))
                .await
            {
                Ok(Some(submission)) if submission_has_scryer_origin(&submission) => {
                    let collection_id = submission.scope.collection_id().map(str::to_string);
                    let mut patched = completed.clone();
                    merge_scryer_origin_parameters(
                        &mut patched.parameters,
                        submission.title_id,
                        submission.facet,
                        collection_id,
                    );
                    patched
                }
                Ok(Some(_)) => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        client_type = %completed.client_type,
                        "import: ignoring stub download_submissions row without scryer origin metadata"
                    );
                    processed_ids.insert(source_ref.clone());
                    continue;
                }
                Ok(None) => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        client_type = %completed.client_type,
                        "import: no scryer origin — not in parameters or download_submissions table"
                    );
                    processed_ids.insert(source_ref.clone());
                    continue;
                }
                Err(error) => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        error = %error,
                        "import: download_submissions lookup failed"
                    );
                    continue;
                }
            }
        };

        if already_imported {
            tracing::debug!(
                source_ref = %source_ref,
                title = %item.title_name,
                "import: treating already-imported download as terminal imported for cleanup"
            );
            let cleanup = reconcile_terminal_download_cleanup_for_completed(
                app,
                &completed,
                TrackedDownloadState::Imported,
            )
            .await;
            if terminal_download_cleanup_is_complete(cleanup) {
                processed_ids.insert(source_ref.clone());
            }
            continue;
        }

        if let Some(state) = completed_download_tracked_state(app, &completed).await
            && matches!(
                state,
                TrackedDownloadState::Imported | TrackedDownloadState::Failed
            )
        {
            tracing::debug!(
                source_ref = %source_ref,
                title = %item.title_name,
                state = state.as_str(),
                "import: retrying terminal cleanup from persisted tracked state"
            );
            let cleanup =
                reconcile_terminal_download_cleanup_for_completed(app, &completed, state).await;
            if terminal_download_cleanup_is_complete(cleanup) {
                processed_ids.insert(source_ref.clone());
            }
            continue;
        }

        // Skip if dest_dir is empty for fresh import attempts.
        if completed.dest_dir.is_empty() {
            tracing::info!(
                source_ref = %source_ref,
                title = %item.title_name,
                "import: skipping download with empty dest_dir"
            );
            continue;
        }

        let facet_label = extract_parameter(&completed.parameters, "*scryer_facet")
            .unwrap_or_else(|| "unknown".to_string());
        tracing::info!(
            source_ref = %source_ref,
            title = %item.title_name,
            dest_dir = %completed.dest_dir,
            facet = %facet_label,
            "import: triggering import for completed download"
        );
        let import_start = std::time::Instant::now();
        match import_completed_download(app, actor, &completed).await {
            Ok(result) => {
                if matches!(
                    result.decision,
                    ImportDecision::Failed | ImportDecision::Rejected
                ) {
                    tracing::warn!(
                        decision = ?result.decision,
                        title_id = ?result.title_id,
                        error_message = ?result.error_message,
                        source_path = %result.source_path,
                        "import failed for {}",
                        completed.name
                    );
                } else if matches!(result.decision, ImportDecision::Unmatched) {
                    tracing::debug!(
                        decision = ?result.decision,
                        error_message = ?result.error_message,
                        source_path = %result.source_path,
                        "import unmatched for {}",
                        completed.name
                    );
                } else {
                    tracing::info!(
                        decision = ?result.decision,
                        title_id = ?result.title_id,
                        dest_path = ?result.dest_path,
                        "import completed for {}",
                        completed.name
                    );
                }
                metrics::counter!("scryer_imports_total", "decision" => result.decision.as_str(), "facet" => facet_label.clone()).increment(1);
                metrics::histogram!("scryer_import_duration_seconds", "facet" => facet_label)
                    .record(import_start.elapsed().as_secs_f64());

                if let Some(state) = terminal_tracked_state_for_import_result(&result) {
                    persist_completed_download_tracked_state(app, &completed, state).await;
                    let cleanup =
                        reconcile_terminal_download_cleanup_for_completed(app, &completed, state)
                            .await;
                    if terminal_download_cleanup_is_complete(cleanup) {
                        processed_ids.insert(source_ref.clone());
                    }
                } else {
                    processed_ids.insert(source_ref.clone());
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    name = %completed.name,
                    "import failed for completed download"
                );
                metrics::counter!("scryer_imports_total", "decision" => "error", "facet" => facet_label.clone()).increment(1);
                metrics::histogram!("scryer_import_duration_seconds", "facet" => facet_label)
                    .record(import_start.elapsed().as_secs_f64());
                processed_ids.insert(source_ref.clone());
            }
        }
    }

    processed_ids
}
fn completed_import_result_is_retryable(result: &ImportResult) -> bool {
    if matches!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles))
        && Path::new(&result.source_path).exists()
    {
        return true;
    }

    result
        .error_message
        .as_deref()
        .is_some_and(completed_import_error_message_is_retryable)
}
// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(crate) struct ImportPathSettings {
    pub(crate) media_root: String,
    pub(crate) rename_template: String,
    pub(crate) folder_template: String,
}
async fn persist_title_folder_path_if_missing(app: &AppUseCase, title: &Title, folder_path: &Path) {
    let has_folder_path = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if has_folder_path {
        return;
    }
    let _ = app
        .services
        .catalog
        .titles
        .set_folder_path(&title.id, &path_to_stored_string(folder_path))
        .await;
}
#[cfg(test)]
fn sanitized_title_folder_component(raw: &str) -> String {
    let sanitized = sanitize_filesystem_component(raw);
    if sanitized.is_empty() {
        "untitled".to_string()
    } else {
        sanitized
    }
}
/// Recursively find all video files under `dir`, optionally filtering out samples.
///
/// `dir` is usually a directory, but SABnzbd sometimes reports the file path
/// itself as the completed download's `storage` field. If the path has a video
/// extension and cannot be opened as a directory, we treat it as a single-file
/// result.
pub(crate) fn find_video_files(dir: &Path, filter_samples: bool) -> AppResult<Vec<PathBuf>> {
    if std::fs::read_dir(dir).is_err() && is_video_file(dir) {
        tracing::info!(
            path = %dir.display(),
            "download path is a video file, not a directory"
        );
        return Ok((!filter_samples || !is_sample_file(dir))
            .then_some(dir.to_path_buf())
            .into_iter()
            .collect());
    }

    let walked = crate::filesystem_walk::FilesystemWalker::new()
        .skip_unreadable_subdirectories()
        .walk(dir)?;

    Ok(walked
        .into_iter()
        .flat_map(|entry| entry.files.into_iter())
        .filter(|path| is_video_file(path))
        .filter(|path| !filter_samples || !is_sample_file(path))
        .collect())
}
pub(crate) fn pick_largest_file(files: &[PathBuf]) -> AppResult<PathBuf> {
    files
        .iter()
        .max_by_key(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0))
        .cloned()
        .ok_or_else(|| AppError::Repository("no files to pick from".to_string()))
}
fn parsed_release_from_file_stem(path: &Path) -> ParsedReleaseMetadata {
    let fallback = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or(fallback);
    normalize_release_title_signal(parse_release_metadata(stem.as_str()))
}
fn parsed_usable_release_from_file_stem(path: &Path) -> Option<ParsedReleaseMetadata> {
    let fallback = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or(fallback);
    parse_usable_release_title(stem.as_str())
}
fn parsed_release_from_folder_name(path: &Path) -> Option<ParsedReleaseMetadata> {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| parse_release_metadata(value.as_str()))
        .map(normalize_release_title_signal)
}
fn parsed_release_from_parent_folder(path: &Path) -> Option<ParsedReleaseMetadata> {
    path.parent().and_then(parsed_release_from_folder_name)
}
fn parsed_usable_release_from_parent_folder(path: &Path) -> Option<ParsedReleaseMetadata> {
    parsed_release_from_parent_folder(path).filter(has_usable_release_title_signal)
}
fn title_evidence_candidates_from_video_files(
    video_files: &[PathBuf],
) -> Vec<ParsedReleaseMetadata> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for video_file in video_files {
        let candidate = parsed_usable_release_from_file_stem(video_file)
            .or_else(|| parsed_usable_release_from_parent_folder(video_file));

        if let Some(candidate) = candidate {
            let key = candidate.raw_title.to_ascii_uppercase();
            if seen.insert(key) {
                candidates.push(candidate);
            }
        }
    }

    candidates
}
