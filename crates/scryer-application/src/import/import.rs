use crate::{
    AppError, AppResult, AppUseCase, CollectionUpdate, DownloadSourceIdentity, ImportArtifact,
    ParsedEpisodeMetadata, ParsedReleaseMetadata, WantedCompleteTransition, WantedItemsQuery,
    activity::NotificationMediaUpdate,
    app_usecase_post_processing::{PostProcessingContext, spawn_post_processing},
    domain_events::{
        created_media_update, deleted_media_update, new_title_domain_event, title_context_snapshot,
    },
    import_parameters::{extract_parameter, has_scryer_origin, submission_has_scryer_origin},
    import_title_resolution::normalize_imdb_id,
    nfo::{render_episode_nfo, render_movie_nfo, render_plexmatch, render_tvshow_nfo},
    parse_release_metadata,
    polling_worker::PollingWorker,
    render_rename_template, require, sanitize_filesystem_component,
};
use chrono::{DateTime, Utc};
use scryer_domain::{
    Collection, CollectionType, CompletedDownload, DomainEventPayload, DownloadQueueItem,
    DownloadQueueState, Entitlement, Id, ImportCompletedEventData, ImportDecision, ImportErrorCode,
    ImportRecord, ImportResult, ImportSkipReason, ImportStatus, ImportType, MediaFacet, User,
    is_video_file,
};
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// If subtitles.auto_download_on_import is enabled, spawn a background subtitle search.
fn maybe_trigger_subtitle_search(app: &AppUseCase, title_id: &str, media_file_id: &str) {
    let app = app.clone();
    let title_id = title_id.to_string();
    let media_file_id = media_file_id.to_string();
    tokio::spawn(async move {
        let auto = app
            .subtitle_settings()
            .await
            .ok()
            .map(|settings| settings.auto_download_on_import)
            .unwrap_or(false);
        if auto {
            crate::spawn_subtitle_search_for_file(app, title_id, media_file_id);
        }
    });
}

const MANUAL_IMPORT_POLLER_INTERVAL_SECONDS: u64 = 2;
const MANUAL_IMPORT_STALE_RECOVERY_SECONDS: i64 = 120;
const SERIES_PATH_KEY: &str = "series.path";

fn base_completed_import_result(
    import_id: &str,
    completed: &CompletedDownload,
    started_at: DateTime<Utc>,
) -> ImportResult {
    ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: None,
        title_id: None,
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: None,
        episode_ids: Vec::new(),
        file_size_bytes: None,
        link_type: None,
        error_message: None,
        started_at,
        completed_at: Utc::now(),
    }
}

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
                MANUAL_IMPORT_STALE_RECOVERY_SECONDS,
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

            let (status, result_json) =
                match execute_queued_manual_import(&app, &record.id, &payload).await {
                    Ok(result) => result,
                    Err(error) => (
                        ImportStatus::Failed,
                        manual_import_result_json(
                            &record.id,
                            &payload,
                            ImportStatus::Failed,
                            Some(classify_manual_import_error_message(&error.to_string())),
                            Some(error.to_string()),
                            Vec::new(),
                        ),
                    ),
                };

            if let Err(error) = app
                .update_import_status_and_notify(&record.id, status, result_json)
                .await
            {
                worker.warn_error("finalize_manual_import_request", &error);
                continue;
            }

            if status == ImportStatus::Completed
                && let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref()
            {
                let _ = handle
                    .mark_imported(crate::tracked_downloads::tracked_download_id(
                        payload.client_id.as_deref(),
                        &payload.client_type,
                        &payload.download_client_item_id,
                    ))
                    .await;
            }
        }
    }
}

/// Retry a previously failed import, optionally with an archive password.
pub async fn retry_failed_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    password: Option<&str>,
) -> AppResult<ImportResult> {
    crate::require(actor, &Entitlement::ManageTitle)?;

    let record = app
        .services
        .workflow
        .imports
        .get_import_by_id(import_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("import {import_id}")))?;

    if record.status != ImportStatus::Failed {
        return Err(AppError::Validation(format!(
            "import {} has status '{}', only failed imports can be retried",
            import_id,
            record.status.as_str()
        )));
    }

    let completed: CompletedDownload = serde_json::from_str(&record.payload_json)
        .map_err(|e| AppError::Repository(format!("failed to deserialize import payload: {e}")))?;

    app.update_import_status_and_notify(import_id, ImportStatus::Processing, None)
        .await?;

    let started_at = Utc::now();
    match run_import(app, actor, import_id, &completed, started_at, password).await {
        Ok(result) => Ok(result),
        Err(error) => {
            let skip_reason = if crate::archive_extractor::is_password_required_error(&error) {
                Some(ImportSkipReason::PasswordRequired)
            } else {
                None
            };
            let result = ImportResult {
                decision: ImportDecision::Failed,
                skip_reason,
                error_message: Some(error.to_string()),
                ..base_completed_import_result(import_id, &completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Failed, result_json)
                .await?;
            Ok(result)
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

    for item in completed_items {
        let source_ref = &item.download_client_item_id;
        match app
            .services
            .workflow
            .imports
            .is_already_imported(&item.client_type, source_ref)
            .await
        {
            Ok(true) => {
                tracing::debug!(
                    source_ref = %source_ref,
                    title = %item.title_name,
                    "import: skipping already-imported download"
                );
                processed_ids.insert(source_ref.clone());
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(error = %error, source_ref = %source_ref, "import dedup check failed");
                continue;
            }
        }

        // Find the matching CompletedDownload
        let completed = match completed_downloads
            .iter()
            .find(|cd| cd.download_client_item_id == item.download_client_item_id)
        {
            Some(cd) => cd,
            None => {
                tracing::debug!(
                    source_ref = %source_ref,
                    title = %item.title_name,
                    "import: no matching CompletedDownload from client history (item may still be processing or status != Completed)"
                );
                continue;
            }
        };

        // Skip if dest_dir is empty
        if completed.dest_dir.is_empty() {
            tracing::info!(
                source_ref = %source_ref,
                title = %item.title_name,
                "import: skipping download with empty dest_dir"
            );
            continue;
        }

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
                    patched.parameters = vec![
                        ("*scryer_title_id".to_string(), submission.title_id),
                        ("*scryer_facet".to_string(), submission.facet),
                    ];
                    if let Some(coll_id) = collection_id {
                        patched
                            .parameters
                            .push(("*scryer_collection_id".to_string(), coll_id));
                    }
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

        let facet_label = extract_parameter(&completed.parameters, "*scryer_facet")
            .unwrap_or_else(|| "unknown".to_string());
        tracing::info!(
            source_ref = %source_ref,
            title = %item.title_name,
            dest_dir = %completed.dest_dir,
            facet = %facet_label,
            "import: triggering import for completed download"
        );
        processed_ids.insert(source_ref.clone());
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
                let completed_facet = facet_for_completed_download(&completed);
                let should_remove_completed = if matches!(result.decision, ImportDecision::Imported)
                {
                    match completed_facet.as_ref() {
                        Some(facet) => {
                            app.should_remove_completed_download(facet, &completed.client_id)
                                .await
                        }
                        None => false,
                    }
                } else {
                    false
                };
                let should_remove_failed = if matches!(
                    result.decision,
                    ImportDecision::Failed | ImportDecision::Rejected
                ) {
                    match completed_facet.as_ref() {
                        Some(facet) => {
                            app.should_remove_failed_download(facet, &completed.client_id)
                                .await
                        }
                        None => false,
                    }
                } else {
                    false
                };
                metrics::counter!("scryer_imports_total", "decision" => result.decision.as_str(), "facet" => facet_label.clone()).increment(1);
                metrics::histogram!("scryer_import_duration_seconds", "facet" => facet_label)
                    .record(import_start.elapsed().as_secs_f64());
                if should_remove_completed {
                    remove_download_history_item(app, &completed, "completed").await;
                } else if should_remove_failed {
                    remove_download_history_item(app, &completed, "failed").await;
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
            }
        }
    }

    processed_ids
}

fn facet_for_completed_download(completed: &CompletedDownload) -> Option<MediaFacet> {
    match extract_parameter(&completed.parameters, "*scryer_facet")
        .as_deref()
        .map(str::trim)
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("movie") => Some(MediaFacet::Movie),
        Some("series") => Some(MediaFacet::Series),
        Some("anime") => Some(MediaFacet::Anime),
        _ => None,
    }
}

async fn remove_download_history_item(
    app: &AppUseCase,
    completed: &CompletedDownload,
    outcome_label: &str,
) {
    if let Err(error) = app
        .services
        .integrations
        .download_client
        .delete_queue_item_for_client(
            &completed.client_type,
            &completed.download_client_item_id,
            true,
        )
        .await
    {
        tracing::warn!(
            client_id = completed.client_id.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            outcome = outcome_label,
            error = %error,
            "failed to delete completed download from client history"
        );
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

    let facet = match title_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(title_id) => match app.services.catalog.titles.get_by_id(title_id).await {
            Ok(Some(title)) => Some(title.facet),
            Ok(None) | Err(_) => facet_for_completed_download(completed),
        },
        None => facet_for_completed_download(completed),
    };

    let Some(facet) = facet else {
        return;
    };

    if app
        .should_remove_completed_download(&facet, &completed.client_id)
        .await
    {
        remove_download_history_item(app, completed, "manual_import_completed").await;
    }
}

pub async fn import_completed_download(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    let started_at = Utc::now();
    let source_ref = &completed.download_client_item_id;

    // 1. DEDUP CHECK
    if app
        .services
        .workflow
        .imports
        .is_already_imported(&completed.client_type, source_ref)
        .await?
    {
        return Ok(ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::AlreadyImported),
            ..base_completed_import_result("", completed, started_at)
        });
    }

    // Queue the import request for tracking
    let import_type = {
        let facet_str = extract_parameter(&completed.parameters, "*scryer_facet");
        let is_episode = facet_str
            .as_deref()
            .and_then(|f| app.facet_registry.all().find(|h| h.facet_id() == f))
            .is_some_and(|h| h.has_episodes());
        if is_episode {
            ImportType::SeriesDownload
        } else {
            ImportType::MovieDownload
        }
    };
    let import_id = app
        .services
        .workflow
        .imports
        .queue_import_request(
            completed.client_type.clone(),
            source_ref.clone(),
            import_type.as_str().to_string(),
            serde_json::to_string(completed).unwrap_or_default(),
        )
        .await?;

    // If the source directory no longer exists, the files were already moved
    // by a previous import (possibly under a different source_ref). Mark as
    // skipped so the poller never retries this entry.
    let source_path = std::path::Path::new(&completed.dest_dir);
    if !source_path.exists() {
        tracing::debug!(
            source_ref,
            dest_dir = %completed.dest_dir,
            "import: source directory no longer exists, no files to import"
        );
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            ..base_completed_import_result(&import_id, completed, started_at)
        };
        let result_json = serde_json::to_string(&result).ok();
        let _ = app
            .update_import_status_and_notify(&import_id, ImportStatus::Skipped, result_json)
            .await;
        return Ok(result);
    }

    // Mark as processing
    app.update_import_status_and_notify(&import_id, ImportStatus::Processing, None)
        .await?;

    // From here on, any error must update the import record to "failed" rather than
    // propagating via `?`. Otherwise the record stays "processing" indefinitely.
    match run_import(app, actor, &import_id, completed, started_at, None).await {
        Ok(result) => Ok(result),
        Err(error) => {
            let skip_reason = if crate::archive_extractor::is_password_required_error(&error) {
                Some(ImportSkipReason::PasswordRequired)
            } else {
                None
            };
            let result = ImportResult {
                decision: ImportDecision::Failed,
                skip_reason,
                error_message: Some(error.to_string()),
                ..base_completed_import_result(&import_id, completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            let _ = app
                .update_import_status_and_notify(&import_id, ImportStatus::Failed, result_json)
                .await;
            Ok(result)
        }
    }
}

async fn run_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    completed: &CompletedDownload,
    started_at: chrono::DateTime<Utc>,
    archive_password: Option<&str>,
) -> AppResult<ImportResult> {
    // 2. TITLE MATCHING
    let mut title = None;
    let parsed_completed_name = parse_release_metadata(&completed.name);
    let parsed_completed_folder = parsed_release_from_folder_name(Path::new(&completed.dest_dir));
    if let Some(title_id) = extract_parameter(&completed.parameters, "*scryer_title_id") {
        let title_id = title_id.trim();
        if !title_id.is_empty() {
            title = app.services.catalog.titles.get_by_id(title_id).await?;
        }
    }

    // fallback to IMDb ID if needed
    if title.is_none() {
        let imdb_id = extract_parameter(&completed.parameters, "*scryer_imdb_id")
            .and_then(|value| normalize_imdb_id(&value));

        title = match imdb_id {
            Some(target_imdb_id) => {
                let titles = app
                    .services
                    .catalog
                    .titles
                    .list_for_matching(None, None)
                    .await?;
                let mut matches = titles
                    .into_iter()
                    .filter(|title| {
                        title.external_ids.iter().any(|external_id| {
                            external_id.source.eq_ignore_ascii_case("imdb")
                                && normalize_imdb_id(&external_id.value).as_deref()
                                    == Some(target_imdb_id.as_str())
                        })
                    })
                    .collect::<Vec<_>>();

                if matches.len() == 1 {
                    matches.pop()
                } else {
                    None
                }
            }
            None => None,
        };
    }

    if title.is_none() {
        let titles = app
            .services
            .catalog
            .titles
            .list_for_matching(None, None)
            .await?;
        let facet_hint = extract_parameter(&completed.parameters, "*scryer_facet")
            .or_else(|| completed.category.clone());

        title = if parsed_completed_name.episode.is_some() {
            crate::import_title_resolution::resolve_monitored_episode_title_from_release(
                &titles,
                &parsed_completed_name,
                facet_hint.as_deref(),
            )
            .map(|resolved| resolved.title.clone())
        } else {
            crate::import_title_resolution::resolve_monitored_movie_title_from_release(
                &titles,
                &parsed_completed_name,
            )
            .map(|resolved| resolved.title.clone())
        };

        if title.is_none()
            && let Some(parsed_completed_folder) = parsed_completed_folder.as_ref()
        {
            title = if parsed_completed_folder.episode.is_some() {
                crate::import_title_resolution::resolve_monitored_episode_title_from_release(
                    &titles,
                    parsed_completed_folder,
                    facet_hint.as_deref(),
                )
                .map(|resolved| resolved.title.clone())
            } else {
                crate::import_title_resolution::resolve_monitored_movie_title_from_release(
                    &titles,
                    parsed_completed_folder,
                )
                .map(|resolved| resolved.title.clone())
            };
        }
    }

    let title = match title {
        Some(t) => t,
        None => {
            let result = ImportResult {
                decision: ImportDecision::Unmatched,
                skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
                error_message: Some(format!(
                    "could not match download '{}' to any monitored title",
                    completed.name
                )),
                ..base_completed_import_result(import_id, completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;

            return Ok(result);
        }
    };

    // Validate supported facets
    if !matches!(
        title.facet,
        MediaFacet::Movie | MediaFacet::Series | MediaFacet::Anime
    ) {
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            title_id: Some(title.id.clone()),
            error_message: Some(format!(
                "title '{}' has unsupported facet '{:?}', skipping import",
                title.name, title.facet
            )),
            ..base_completed_import_result(import_id, completed, started_at)
        };
        let result_json = serde_json::to_string(&result).ok();
        app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
            .await?;
        return Ok(result);
    }

    // 3. FIND VIDEO FILES (extract archives first if needed)
    let dest_dir = Path::new(&completed.dest_dir);
    let is_series = matches!(title.facet, MediaFacet::Series | MediaFacet::Anime);
    let extracted_dir =
        crate::archive_extractor::extract_archives_if_needed(dest_dir, archive_password).await?;
    let effective_dir = extracted_dir.as_deref().unwrap_or(dest_dir);
    let video_files = find_video_files(effective_dir, is_series)?;

    if video_files.is_empty() {
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            title_id: Some(title.id.clone()),
            error_message: Some(format!("no video files found in {}", completed.dest_dir)),
            ..base_completed_import_result(import_id, completed, started_at)
        };
        let result_json = serde_json::to_string(&result).ok();
        app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
            .await?;
        return Ok(result);
    }

    // Check if this is an interstitial movie import (anime franchise movie → Season 00)
    let interstitial_collection_id =
        extract_parameter(&completed.parameters, "*scryer_collection_id");

    // Branch on facet: movies import the single largest file, series import all episode files
    let result = if let Some(ref coll_id) = interstitial_collection_id {
        import_interstitial_movie_download(
            app,
            actor,
            &title,
            import_id,
            completed,
            &video_files,
            started_at,
            coll_id,
        )
        .await
    } else if is_series {
        import_series_download(
            app,
            actor,
            &title,
            import_id,
            completed,
            &video_files,
            started_at,
        )
        .await
    } else {
        import_movie_download(
            app,
            actor,
            &title,
            import_id,
            completed,
            &video_files,
            started_at,
        )
        .await
    };

    // Clean up extracted archive directory if we created one
    if let Some(ref dir) = extracted_dir {
        crate::archive_extractor::cleanup_extracted_dir(dir).await;
    }

    result
}

// ---------------------------------------------------------------------------
// Movie import: pick largest file, single import
// ---------------------------------------------------------------------------

async fn import_movie_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let source_video = pick_largest_file(video_files)?;
    let source_size = std::fs::metadata(&source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let (media_root, rename_template) = resolve_import_paths(app, title).await?;

    let parsed = build_augmented_movie_import_metadata(&source_video, completed);
    let existing_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default();
    let quality_profile = resolve_import_quality_profile(app, title).await;
    let existing_score = existing_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    let prepared = match crate::post_download_gate::prepare_import_candidate(
        app,
        title,
        &parsed,
        &quality_profile,
        &source_video,
        source_size,
        !existing_files.is_empty(),
        existing_score,
        false,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(rejection) => {
            crate::post_download_gate::reject_source_file_before_import(
                app,
                Some(&actor.id),
                title,
                &completed.name,
                &source_video,
                &[],
                &rejection,
            )
            .await;
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                &source_video,
                "movie",
                "rejected",
                rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                None,
                &[],
            )
            .await;
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Rejected,
                skip_reason: rejection.skip_reason.clone(),
                title_id: Some(title.id.clone()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: source_video.to_string_lossy().to_string(),
                dest_path: None,
                quality: parsed.quality.clone(),
                episode_ids: Vec::new(),
                file_size_bytes: Some(source_size),
                link_type: None,
                error_message: Some(rejection.message),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
    };

    let ext = source_video
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mkv")
        .to_string();
    let tokens = build_rename_tokens(title, &prepared.parsed, &ext);
    let rendered_filename = render_rename_template(&rename_template, &tokens);

    let year_str = prepared
        .parsed
        .year
        .or(title.year)
        .map(|y| format!(" ({})", y))
        .unwrap_or_default();
    let title_folder = sanitize_filesystem_component(&format!("{}{}", title.name, year_str));
    let full_folder_path = PathBuf::from(&media_root).join(&title_folder);

    if title.folder_path.is_none() {
        let _ = app
            .services
            .catalog
            .titles
            .set_folder_path(&title.id, &full_folder_path.to_string_lossy())
            .await;
    }

    let dest_path = full_folder_path.join(&rendered_filename);
    let check_ctx = crate::import_checks::ImportCheckContext {
        source_path: &source_video,
        dest_path: &dest_path,
        source_size: source_size as u64,
        parsed: &prepared.parsed,
        existing_files: &existing_files,
    };
    if let crate::import_checks::ImportVerdict::Reject { reason, code } =
        crate::import_checks::run_import_checks(&check_ctx)
    {
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            &source_video,
            "movie",
            "rejected",
            Some(code),
            None,
            &[],
        )
        .await;
        let skip_reason = Some(match code {
            "duplicate_file" => ImportSkipReason::DuplicateFile,
            "insufficient_disk_space" => ImportSkipReason::DiskFull,
            "invalid_extension" | "sample_file" | "sample_directory" => {
                ImportSkipReason::NoVideoFiles
            }
            _ => ImportSkipReason::PolicyMismatch,
        });
        let result = ImportResult {
            import_id: import_id.to_string(),
            decision: ImportDecision::Skipped,
            skip_reason,
            title_id: Some(title.id.clone()),
            source_system: Some(completed.client_type.clone()),
            source_ref: Some(completed.download_client_item_id.clone()),
            source_title: Some(completed.name.clone()),
            source_path: source_video.to_string_lossy().to_string(),
            dest_path: Some(dest_path.to_string_lossy().to_string()),
            quality: prepared.parsed.quality.clone(),
            episode_ids: Vec::new(),
            file_size_bytes: Some(source_size),
            link_type: None,
            error_message: Some(reason),
            started_at,
            completed_at: Utc::now(),
        };
        let result_json = serde_json::to_string(&result).ok();
        app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
            .await?;
        return Ok(result);
    }

    if !existing_files.is_empty() {
        let (required_audio_languages, persona) = resolve_import_audio_persona(app, title).await;
        let new_decision = crate::post_download_gate::build_import_profile_decision(
            &quality_profile,
            &required_audio_languages,
            &persona,
            &prepared.parsed,
            crate::post_download_gate::facet_to_category_hint(&title.facet),
            title.runtime_minutes,
            Some(source_size),
            true,
        );
        let new_score = new_decision.preference_score;

        if let Some(existing_file) = existing_files
            .iter()
            .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        {
            let old_score = existing_file.acquisition_score.unwrap_or(0);
            if new_score > old_score {
                let media_root_opt = crate::recycle_bin::media_root_for_title(app, title).await;
                let recycle_config =
                    crate::recycle_bin::resolve_recycle_config(app, media_root_opt.as_deref())
                        .await;

                match crate::upgrade::execute_upgrade(
                    app,
                    actor,
                    title,
                    existing_file,
                    &source_video,
                    &dest_path,
                    &prepared,
                    prepared.parsed.quality.as_deref(),
                    new_score,
                    old_score,
                    &[],
                    &recycle_config,
                )
                .await
                {
                    Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "imported",
                            Some("upgrade"),
                            None,
                            &[],
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Imported,
                            skip_reason: None,
                            title_id: Some(title.id.clone()),
                            source_system: Some(completed.client_type.clone()),
                            source_ref: Some(completed.download_client_item_id.clone()),
                            source_title: Some(completed.name.clone()),
                            source_path: source_video.to_string_lossy().to_string(),
                            dest_path: Some(dest_path.to_string_lossy().to_string()),
                            quality: prepared.parsed.quality.clone(),
                            episode_ids: Vec::new(),
                            file_size_bytes: Some(source_size),
                            link_type: None,
                            error_message: None,
                            started_at,
                            completed_at: Utc::now(),
                        };
                        tracing::info!(
                            title = %title.name,
                            old_score = outcome.old_score,
                            new_score = outcome.new_score,
                            "movie file upgraded"
                        );
                        mark_wanted_completed(app, &title.id, None, None).await;
                        let result_json = serde_json::to_string(&result).ok();
                        app.update_import_status_and_notify(
                            import_id,
                            ImportStatus::Completed,
                            result_json,
                        )
                        .await?;
                        return Ok(result);
                    }
                    Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "already_present",
                            rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                            None,
                            &[],
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Rejected,
                            skip_reason: rejection.skip_reason.clone(),
                            title_id: Some(title.id.clone()),
                            source_system: Some(completed.client_type.clone()),
                            source_ref: Some(completed.download_client_item_id.clone()),
                            source_title: Some(completed.name.clone()),
                            source_path: source_video.to_string_lossy().to_string(),
                            dest_path: Some(dest_path.to_string_lossy().to_string()),
                            quality: prepared.parsed.quality.clone(),
                            episode_ids: Vec::new(),
                            file_size_bytes: Some(source_size),
                            link_type: None,
                            error_message: Some(rejection.message),
                            started_at,
                            completed_at: Utc::now(),
                        };
                        let result_json = serde_json::to_string(&result).ok();
                        app.update_import_status_and_notify(
                            import_id,
                            ImportStatus::Skipped,
                            result_json,
                        )
                        .await?;
                        return Ok(result);
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "upgrade failed, falling through to normal import"
                        );
                    }
                }
            }
        }
    }

    let file_result = app
        .services
        .workflow
        .file_importer
        .import_file(&source_video, &dest_path)
        .await?;

    let nfo_enabled = app
        .read_setting_string_value("nfo.write_on_import.movie", None)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true");
    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        let nfo_content = render_movie_nfo(title);
        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
            tracing::warn!(
                error = %err,
                path = %nfo_path.display(),
                "failed to write movie NFO sidecar"
            );
        }
    }

    let acq_score = crate::post_download_gate::compute_acquisition_score(
        app,
        &prepared.parsed,
        prepared.accepted.as_ref(),
        &quality_profile,
        title,
        file_result.size_bytes as i64,
        !existing_files.is_empty(),
    )
    .await;

    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: dest_path.to_string_lossy().to_string(),
        size_bytes: file_result.size_bytes as i64,
        quality_label: prepared.parsed.quality.clone(),
        scene_name: Some(prepared.parsed.raw_title.clone()),
        release_group: prepared.parsed.release_group.clone(),
        source_type: prepared.parsed.source.clone(),
        resolution: prepared.parsed.quality.clone(),
        video_codec_parsed: prepared.parsed.video_codec.clone(),
        audio_codec_parsed: prepared.parsed.audio.clone(),
        audio_channels_parsed: prepared.parsed.audio_channels.clone(),
        original_file_path: Some(source_video.to_string_lossy().to_string()),
        acquisition_score: Some(acq_score),
        ..Default::default()
    };
    let imported_media_file_id = match app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await
    {
        Ok(file_id) => {
            crate::post_download_gate::persist_media_analysis_result(
                &app.services.library.media_files,
                &file_id,
                prepared.accepted.as_ref(),
            )
            .await;
            if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
                app, &title.id, &file_id, None, &dest_path,
            )
            .await
            {
                tracing::warn!(
                    error = %error,
                    title_id = %title.id,
                    file_id = %file_id,
                    dest_path = %dest_path.display(),
                    "failed to reconcile external subtitles after import"
                );
            }
            maybe_trigger_subtitle_search(app, &title.id, &file_id);
            Some(file_id)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = %title.id,
                dest_path = %dest_path.display(),
                "failed to insert media_files record (import will still succeed)"
            );
            None
        }
    };

    persist_file_import_artifact(
        app,
        import_id,
        completed,
        title.id.as_str(),
        &source_video,
        "movie",
        "imported",
        None,
        imported_media_file_id.as_deref(),
        &[],
    )
    .await;

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: CollectionType::Movie,
        collection_index: "1".to_string(),
        label: prepared.parsed.quality.clone(),
        ordered_path: Some(dest_path.to_string_lossy().to_string()),
        narrative_order: None,
        first_episode_number: None,
        last_episode_number: None,
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: true,
        created_at: Utc::now(),
    };
    if let Err(err) = app
        .services
        .catalog
        .shows
        .create_collection(collection)
        .await
    {
        tracing::warn!(
            error = %err,
            title_id = %title.id,
            "failed to create collection record"
        );
    }

    spawn_post_processing(PostProcessingContext {
        app: app.clone(),
        actor_id: Some(actor.id.clone()),
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        dest_path: dest_path.clone(),
        year: title.year,
        imdb_id: title
            .external_ids
            .iter()
            .find(|e| e.source == "imdb")
            .map(|e| e.value.clone()),
        tvdb_id: title
            .external_ids
            .iter()
            .find(|e| e.source == "tvdb")
            .map(|e| e.value.clone()),
        season: None,
        episode: None,
        quality: prepared.parsed.quality.clone(),
    });

    mark_wanted_completed(app, &title.id, None, None).await;

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: source_video.to_string_lossy().to_string(),
        dest_path: Some(dest_path.to_string_lossy().to_string()),
        quality: prepared.parsed.quality.clone(),
        episode_ids: Vec::new(),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(file_result.strategy),
        error_message: None,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.update_import_status_and_notify(import_id, ImportStatus::Completed, result_json)
        .await?;

    let _ = app
        .append_domain_event(new_title_domain_event(
            Some(actor.id.clone()),
            title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(title),
                media_updates: vec![created_media_update(
                    dest_path.to_string_lossy().to_string(),
                )],
                imported_count: 1,
                import_id: Some(import_id.to_string()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: Some(source_video.to_string_lossy().to_string()),
                dest_path: Some(dest_path.to_string_lossy().to_string()),
                quality: prepared.parsed.quality.clone(),
                episode_ids: Vec::new(),
            }),
        ))
        .await;

    Ok(result)
}

// ---------------------------------------------------------------------------
// Interstitial movie import: anime franchise movie → Season 00 of the series
// ---------------------------------------------------------------------------

async fn import_interstitial_movie_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
    collection_id: &str,
) -> AppResult<ImportResult> {
    // Load the interstitial collection
    let collection = match app
        .services
        .catalog
        .shows
        .get_collection_by_id(collection_id)
        .await?
    {
        Some(c) => c,
        None => {
            let result = ImportResult {
                decision: ImportDecision::Failed,
                skip_reason: None,
                title_id: Some(title.id.clone()),
                error_message: Some(format!("interstitial collection {collection_id} not found")),
                ..base_completed_import_result(import_id, completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
    };

    let movie = match collection.interstitial_movie.as_ref() {
        Some(m) => m,
        None => {
            let result = ImportResult {
                decision: ImportDecision::Failed,
                skip_reason: None,
                title_id: Some(title.id.clone()),
                error_message: Some("interstitial collection has no movie metadata".to_string()),
                ..base_completed_import_result(import_id, completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
    };

    let source_video = pick_largest_file(video_files)?;
    let source_size = std::fs::metadata(&source_video)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let (media_root, _rename_template) = resolve_import_paths(app, title).await?;

    let parsed = build_augmented_movie_import_metadata(&source_video, completed);

    let ext = source_video
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mkv")
        .to_string();

    // Interstitial imports intentionally bypass facet rename templates today and always land as
    // <series>/<Season 00>/<series - S00E## - interstitial movie.ext>; if this ever becomes
    // token-driven, move dest_path construction below prepare_import_candidate per the import/rename lesson.
    let season_episode = collection
        .interstitial_season_episode
        .as_deref()
        .unwrap_or("S00E01");
    let rendered_filename = sanitize_filesystem_component(&format!(
        "{} - {} - {}.{}",
        title.name, season_episode, movie.name, ext
    ));

    // Build destination: <media_root>/<title folder>/Season 00/<filename>
    let year_str = title.year.map(|y| format!(" ({})", y)).unwrap_or_default();
    let title_folder = sanitize_filesystem_component(&format!("{}{}", title.name, year_str));
    let dest_path = PathBuf::from(&media_root)
        .join(&title_folder)
        .join("Season 00")
        .join(&rendered_filename);

    // Pre-import checks (same as movie import)
    let existing_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default();
    // Filter to files in this collection's Season 00 path
    let collection_files: Vec<_> = existing_files
        .iter()
        .filter(|f| {
            collection
                .ordered_path
                .as_deref()
                .is_some_and(|p| f.file_path == p)
        })
        .cloned()
        .collect();
    let quality_profile = resolve_import_quality_profile(app, title).await;
    let existing_score = collection_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    let prepared = match crate::post_download_gate::prepare_import_candidate(
        app,
        title,
        &parsed,
        &quality_profile,
        &source_video,
        source_size,
        !collection_files.is_empty(),
        existing_score,
        false,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(rejection) => {
            crate::post_download_gate::reject_source_file_before_import(
                app,
                Some(&actor.id),
                title,
                &completed.name,
                &source_video,
                &[],
                &rejection,
            )
            .await;
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                &source_video,
                "movie",
                "rejected",
                rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                None,
                &[],
            )
            .await;
            let result = ImportResult {
                import_id: import_id.to_string(),
                decision: ImportDecision::Rejected,
                skip_reason: rejection.skip_reason.clone(),
                title_id: Some(title.id.clone()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: source_video.to_string_lossy().to_string(),
                dest_path: Some(dest_path.to_string_lossy().to_string()),
                quality: parsed.quality.clone(),
                episode_ids: Vec::new(),
                file_size_bytes: Some(source_size),
                link_type: None,
                error_message: Some(rejection.message),
                started_at,
                completed_at: Utc::now(),
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                .await?;
            return Ok(result);
        }
    };

    // Upgrade check: if there's an existing file for this interstitial, score and compare
    if !collection_files.is_empty() {
        let (required_audio_languages, persona) = resolve_import_audio_persona(app, title).await;
        let new_decision = crate::post_download_gate::build_import_profile_decision(
            &quality_profile,
            &required_audio_languages,
            &persona,
            &prepared.parsed,
            crate::post_download_gate::facet_to_category_hint(&title.facet),
            Some(movie.runtime_minutes),
            Some(source_size),
            true,
        );
        let new_score = new_decision.preference_score;

        if let Some(existing_file) = collection_files
            .iter()
            .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        {
            let old_score = existing_file.acquisition_score.unwrap_or(0);
            if new_score > old_score {
                let media_root_opt = crate::recycle_bin::media_root_for_title(app, title).await;
                let recycle_config =
                    crate::recycle_bin::resolve_recycle_config(app, media_root_opt.as_deref())
                        .await;

                match crate::upgrade::execute_upgrade(
                    app,
                    actor,
                    title,
                    existing_file,
                    &source_video,
                    &dest_path,
                    &prepared,
                    prepared.parsed.quality.as_deref(),
                    new_score,
                    old_score,
                    &[],
                    &recycle_config,
                )
                .await
                {
                    Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "imported",
                            Some("upgrade"),
                            None,
                            &[],
                        )
                        .await;
                        tracing::info!(
                            title = %title.name,
                            movie = %movie.name,
                            old_score = outcome.old_score,
                            new_score = outcome.new_score,
                            "interstitial movie file upgraded"
                        );
                        mark_wanted_completed_for_collection(app, &title.id, collection_id).await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Imported,
                            skip_reason: None,
                            title_id: Some(title.id.clone()),
                            source_system: Some(completed.client_type.clone()),
                            source_ref: Some(completed.download_client_item_id.clone()),
                            source_title: Some(completed.name.clone()),
                            source_path: source_video.to_string_lossy().to_string(),
                            dest_path: Some(dest_path.to_string_lossy().to_string()),
                            quality: prepared.parsed.quality.clone(),
                            episode_ids: Vec::new(),
                            file_size_bytes: Some(source_size),
                            link_type: None,
                            error_message: None,
                            started_at,
                            completed_at: Utc::now(),
                        };
                        let result_json = serde_json::to_string(&result).ok();
                        app.update_import_status_and_notify(
                            import_id,
                            ImportStatus::Completed,
                            result_json,
                        )
                        .await?;
                        return Ok(result);
                    }
                    Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                        persist_file_import_artifact(
                            app,
                            import_id,
                            completed,
                            title.id.as_str(),
                            &source_video,
                            "movie",
                            "already_present",
                            rejection.skip_reason.as_ref().map(ImportSkipReason::as_str),
                            None,
                            &[],
                        )
                        .await;
                        let result = ImportResult {
                            import_id: import_id.to_string(),
                            decision: ImportDecision::Rejected,
                            skip_reason: rejection.skip_reason.clone(),
                            title_id: Some(title.id.clone()),
                            source_system: Some(completed.client_type.clone()),
                            source_ref: Some(completed.download_client_item_id.clone()),
                            source_title: Some(completed.name.clone()),
                            source_path: source_video.to_string_lossy().to_string(),
                            dest_path: Some(dest_path.to_string_lossy().to_string()),
                            quality: prepared.parsed.quality.clone(),
                            episode_ids: Vec::new(),
                            file_size_bytes: Some(source_size),
                            link_type: None,
                            error_message: Some(rejection.message),
                            started_at,
                            completed_at: Utc::now(),
                        };
                        let result_json = serde_json::to_string(&result).ok();
                        app.update_import_status_and_notify(
                            import_id,
                            ImportStatus::Skipped,
                            result_json,
                        )
                        .await?;
                        return Ok(result);
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "interstitial upgrade failed, falling through to normal import"
                        );
                    }
                }
            } else {
                // New file is not better — skip
                persist_file_import_artifact(
                    app,
                    import_id,
                    completed,
                    title.id.as_str(),
                    &source_video,
                    "movie",
                    "already_present",
                    Some("existing_better_or_equal"),
                    None,
                    &[],
                )
                .await;
                let result = ImportResult {
                    import_id: import_id.to_string(),
                    decision: ImportDecision::Skipped,
                    skip_reason: Some(ImportSkipReason::PolicyMismatch),
                    title_id: Some(title.id.clone()),
                    source_system: Some(completed.client_type.clone()),
                    source_ref: Some(completed.download_client_item_id.clone()),
                    source_title: Some(completed.name.clone()),
                    source_path: source_video.to_string_lossy().to_string(),
                    dest_path: Some(dest_path.to_string_lossy().to_string()),
                    quality: prepared.parsed.quality.clone(),
                    episode_ids: Vec::new(),
                    file_size_bytes: Some(source_size),
                    link_type: None,
                    error_message: Some(format!(
                        "new score {new_score} not better than existing {old_score}"
                    )),
                    started_at,
                    completed_at: Utc::now(),
                };
                let result_json = serde_json::to_string(&result).ok();
                app.update_import_status_and_notify(import_id, ImportStatus::Skipped, result_json)
                    .await?;
                return Ok(result);
            }
        }
    }

    // Ensure Season 00 directory exists
    if let Some(parent) = dest_path.parent()
        && let Err(err) = tokio::fs::create_dir_all(parent).await
    {
        tracing::warn!(error = %err, path = %parent.display(), "failed to create Season 00 directory");
    }

    // Import file (hardlink or copy)
    let file_result = app
        .services
        .workflow
        .file_importer
        .import_file(&source_video, &dest_path)
        .await?;

    let acq_score = crate::post_download_gate::compute_acquisition_score(
        app,
        &prepared.parsed,
        prepared.accepted.as_ref(),
        &quality_profile,
        title,
        file_result.size_bytes as i64,
        !collection_files.is_empty(),
    )
    .await;

    let imported_media_file_id = if let Ok(file_id) = app
        .services
        .library
        .media_files
        .insert_media_file(&crate::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: dest_path.to_string_lossy().to_string(),
            size_bytes: file_result.size_bytes as i64,
            quality_label: prepared.parsed.quality.clone(),
            scene_name: Some(prepared.parsed.raw_title.clone()),
            release_group: prepared.parsed.release_group.clone(),
            source_type: prepared.parsed.source.clone(),
            resolution: prepared.parsed.quality.clone(),
            video_codec_parsed: prepared.parsed.video_codec.clone(),
            audio_codec_parsed: prepared.parsed.audio.clone(),
            audio_channels_parsed: prepared.parsed.audio_channels.clone(),
            original_file_path: Some(source_video.to_string_lossy().to_string()),
            acquisition_score: Some(acq_score),
            ..Default::default()
        })
        .await
    {
        crate::post_download_gate::persist_media_analysis_result(
            &app.services.library.media_files,
            &file_id,
            prepared.accepted.as_ref(),
        )
        .await;
        if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
            app, &title.id, &file_id, None, &dest_path,
        )
        .await
        {
            tracing::warn!(
                error = %error,
                title_id = %title.id,
                file_id = %file_id,
                dest_path = %dest_path.display(),
                "failed to reconcile external subtitles after import"
            );
        }
        maybe_trigger_subtitle_search(app, &title.id, &file_id);
        Some(file_id)
    } else {
        None
    };

    persist_file_import_artifact(
        app,
        import_id,
        completed,
        title.id.as_str(),
        &source_video,
        "movie",
        "imported",
        None,
        imported_media_file_id.as_deref(),
        &[],
    )
    .await;

    // Update the interstitial collection with the file path
    if let Err(err) = app
        .services
        .catalog
        .shows
        .update_collection(
            collection_id,
            CollectionUpdate {
                ordered_path: Some(dest_path.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .await
    {
        tracing::warn!(
            error = %err,
            collection_id = collection_id,
            "failed to update interstitial collection ordered_path"
        );
    }

    // Write Jellyfin-compatible NFO with airsbefore_season
    let nfo_enabled = app
        .read_setting_string_value("nfo.write_on_import.anime", None)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true");
    if nfo_enabled {
        let nfo_path = dest_path.with_extension("nfo");
        let nfo_content = crate::nfo::render_interstitial_movie_nfo(
            movie,
            season_episode,
            &collection.collection_index,
        );
        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await {
            tracing::warn!(
                error = %err,
                path = %nfo_path.display(),
                "failed to write interstitial movie NFO sidecar"
            );
        }
    }

    // Mark wanted item as completed (by collection_id)
    mark_wanted_completed_for_collection(app, &title.id, collection_id).await;

    // Spawn post-processing
    spawn_post_processing(PostProcessingContext {
        app: app.clone(),
        actor_id: Some(actor.id.clone()),
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        dest_path: dest_path.clone(),
        year: title.year,
        imdb_id: title
            .external_ids
            .iter()
            .find(|e| e.source == "imdb")
            .map(|e| e.value.clone()),
        tvdb_id: title
            .external_ids
            .iter()
            .find(|e| e.source == "tvdb")
            .map(|e| e.value.clone()),
        season: None,
        episode: None,
        quality: prepared.parsed.quality.clone(),
    });

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: source_video.to_string_lossy().to_string(),
        dest_path: Some(dest_path.to_string_lossy().to_string()),
        quality: prepared.parsed.quality.clone(),
        episode_ids: Vec::new(),
        file_size_bytes: Some(file_result.size_bytes as i64),
        link_type: Some(file_result.strategy),
        error_message: None,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.update_import_status_and_notify(import_id, ImportStatus::Completed, result_json)
        .await?;

    app.append_domain_event(new_title_domain_event(
        Some(actor.id.clone()),
        title,
        DomainEventPayload::ImportCompleted(ImportCompletedEventData {
            title: title_context_snapshot(title),
            media_updates: vec![created_media_update(
                dest_path.to_string_lossy().to_string(),
            )],
            imported_count: 1,
            import_id: Some(import_id.to_string()),
            source_system: Some(completed.client_type.clone()),
            source_ref: Some(completed.download_client_item_id.clone()),
            source_title: Some(completed.name.clone()),
            source_path: Some(source_video.to_string_lossy().to_string()),
            dest_path: Some(dest_path.to_string_lossy().to_string()),
            quality: prepared.parsed.quality.clone(),
            episode_ids: Vec::new(),
        }),
    ))
    .await?;

    Ok(result)
}

/// Mark a wanted item as completed by collection_id (for interstitial movies).
async fn mark_wanted_completed_for_collection(
    app: &AppUseCase,
    title_id: &str,
    collection_id: &str,
) {
    // Find the wanted item by iterating (since we don't have a direct lookup by collection_id)
    match app
        .services
        .workflow
        .wanted_items
        .list_wanted_items(WantedItemsQuery {
            status: Some("wanted".into()),
            media_type: Some("interstitial_movie".into()),
            title_id: Some(title_id.to_string()),
            limit: 100,
            ..WantedItemsQuery::default()
        })
        .await
    {
        Ok(items) => {
            for item in items {
                if item.collection_id.as_deref() == Some(collection_id) {
                    let now = Utc::now().to_rfc3339();
                    let _ = app
                        .services
                        .workflow
                        .wanted_items
                        .transition_wanted_to_completed(&WantedCompleteTransition {
                            id: item.id.clone(),
                            last_search_at: Some(now),
                            search_count: item.search_count,
                            current_score: item.current_score,
                            grabbed_release: item.grabbed_release.clone(),
                        })
                        .await;
                    return;
                }
            }
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = title_id,
                collection_id = collection_id,
                "failed to look up wanted item for interstitial movie"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Series import: process ALL video files, link each to its episode
// ---------------------------------------------------------------------------

async fn import_series_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let (media_root, rename_template) = resolve_import_paths(app, title).await?;
    let title_folder = sanitize_filesystem_component(&title.name);
    let full_folder_path = PathBuf::from(&media_root).join(&title_folder);

    if title.folder_path.is_none() {
        let _ = app
            .services
            .catalog
            .titles
            .set_folder_path(&title.id, &full_folder_path.to_string_lossy())
            .await;
    }

    let quality_profile = resolve_import_quality_profile(app, title).await;

    // Check NFO write setting (non-fatal, opt-in)
    let nfo_key = match title.facet {
        scryer_domain::MediaFacet::Anime => "nfo.write_on_import.anime",
        _ => "nfo.write_on_import.series",
    };
    let nfo_enabled = app
        .read_setting_string_value(nfo_key, None)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true");

    let mut imported_count: usize = 0;
    let mut skipped_count: usize = 0;
    let mut rejected_count: usize = 0;
    let mut failed_count: usize = 0;
    let mut last_error: Option<String> = None;
    let mut last_rejection_skip_reason: Option<ImportSkipReason> = None;
    let mut imported_updates: Vec<NotificationMediaUpdate> = Vec::new();
    let mut imported_episode_ids: Vec<String> = Vec::new();

    for source_video in video_files {
        match import_single_episode_file(
            app,
            actor,
            title,
            import_id,
            &media_root,
            &rename_template,
            &title_folder,
            completed,
            source_video,
            video_files.len() > 1,
            &quality_profile,
            nfo_enabled,
        )
        .await
        {
            Ok(EpisodeImportOutcome::Imported {
                dest_path,
                episode_ids,
                ..
            }) => {
                imported_count += 1;
                imported_updates.push(NotificationMediaUpdate::created(dest_path));
                imported_episode_ids.extend(episode_ids);
            }
            Ok(EpisodeImportOutcome::Skipped { .. }) => skipped_count += 1,
            Ok(EpisodeImportOutcome::Rejected { rejection, .. }) => {
                rejected_count += 1;
                last_error = Some(rejection.message.clone());
                last_rejection_skip_reason = rejection.skip_reason.clone();
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    file = %source_video.display(),
                    title = %title.name,
                    "failed to import episode file"
                );
                last_error = Some(err.to_string());
                failed_count += 1;
            }
        }
    }

    if imported_count > 0 {
        write_series_sidecars(app, title, &media_root, &title_folder, nfo_enabled).await;
    }

    let (decision, status, skip_reason) = if imported_count > 0 {
        (ImportDecision::Imported, ImportStatus::Completed, None)
    } else if failed_count > 0 {
        (ImportDecision::Failed, ImportStatus::Failed, None)
    } else if rejected_count > 0 {
        (
            ImportDecision::Rejected,
            ImportStatus::Failed,
            last_rejection_skip_reason,
        )
    } else {
        // All files skipped (no parseable episode info, already imported, etc.)
        // — this is a permanent condition, not worth retrying.
        (ImportDecision::Skipped, ImportStatus::Skipped, None)
    };

    let error_message = if failed_count > 0 || skipped_count > 0 || rejected_count > 0 {
        Some(format!(
            "{imported_count} imported, {skipped_count} skipped, {rejected_count} rejected, {failed_count} failed{}",
            last_error
                .as_ref()
                .map(|e| format!(". Last error: {e}"))
                .unwrap_or_default()
        ))
    } else {
        None
    };

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision,
        skip_reason,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: Some(completed.name.clone()),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: None,
        episode_ids: imported_episode_ids.clone(),
        file_size_bytes: None,
        link_type: None,
        error_message,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    app.update_import_status_and_notify(import_id, status, result_json)
        .await?;

    if imported_count > 0 {
        app.append_domain_event(new_title_domain_event(
            Some(actor.id.clone()),
            title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(title),
                media_updates: imported_updates
                    .into_iter()
                    .map(|update| created_media_update(update.path))
                    .collect(),
                imported_count: imported_count as i32,
                import_id: Some(import_id.to_string()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: Some(completed.name.clone()),
                source_path: Some(completed.dest_dir.clone()),
                dest_path: None,
                quality: None,
                episode_ids: imported_episode_ids,
            }),
        ))
        .await?;
    }

    Ok(result)
}

enum EpisodeImportOutcome {
    Imported {
        dest_path: String,
        episode_ids: Vec<String>,
        imported_media_file_id: Option<String>,
        reason_code: Option<String>,
    },
    Skipped {
        message: String,
        reason_code: Option<String>,
        skip_reason: Option<ImportSkipReason>,
    },
    Rejected {
        rejection: crate::post_download_gate::ImportedFileRejection,
        finalize_before_import: bool,
        reason_code: Option<String>,
    },
}

#[derive(Clone, Debug)]
struct EpisodeUpgradePlan {
    primary_incumbent: crate::EpisodeScopedMediaFile,
    additional_superseded: Vec<crate::EpisodeScopedMediaFile>,
    previous_best_score: i32,
}

fn media_file_score(file: &crate::TitleMediaFile) -> i32 {
    file.acquisition_score.unwrap_or(0)
}

fn skip_reason_for_import_check_code(code: &str) -> ImportSkipReason {
    match code {
        "duplicate_file" => ImportSkipReason::DuplicateFile,
        "insufficient_disk_space" => ImportSkipReason::DiskFull,
        "invalid_extension" | "sample_file" | "sample_directory" => ImportSkipReason::NoVideoFiles,
        _ => ImportSkipReason::PolicyMismatch,
    }
}

fn reject_broader_episode_incumbent(
    incumbent: &crate::EpisodeScopedMediaFile,
) -> crate::post_download_gate::ImportedFileRejection {
    crate::post_download_gate::ImportedFileRejection {
        message: format!(
            "existing episode file {} spans a broader episode set and cannot be replaced by this import",
            incumbent.media_file.file_path
        ),
        recycle_reason: "policy_mismatch",
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        blocking_rule_codes: Vec::new(),
    }
}

fn reject_non_upgrade_episode_incumbent(
    incumbent: &crate::EpisodeScopedMediaFile,
    new_score: i32,
) -> crate::post_download_gate::ImportedFileRejection {
    let old_score = media_file_score(&incumbent.media_file);
    crate::post_download_gate::ImportedFileRejection {
        message: format!(
            "existing episode file {} is equal or better (score {} >= {})",
            incumbent.media_file.file_path, old_score, new_score
        ),
        recycle_reason: "already_imported",
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        blocking_rule_codes: Vec::new(),
    }
}

fn build_episode_upgrade_plan(
    incumbents: &[crate::EpisodeScopedMediaFile],
    target_episode_ids: &[String],
    new_score: i32,
) -> Result<EpisodeUpgradePlan, crate::post_download_gate::ImportedFileRejection> {
    let target_episode_ids = target_episode_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut sorted_incumbents = incumbents.to_vec();
    sorted_incumbents.sort_by(|left, right| {
        media_file_score(&right.media_file)
            .cmp(&media_file_score(&left.media_file))
            .then_with(|| right.media_file.created_at.cmp(&left.media_file.created_at))
            .then_with(|| right.media_file.id.cmp(&left.media_file.id))
    });

    for incumbent in &sorted_incumbents {
        let incumbent_episode_ids = incumbent
            .episode_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if !incumbent_episode_ids.is_subset(&target_episode_ids) {
            return Err(reject_broader_episode_incumbent(incumbent));
        }

        if new_score <= media_file_score(&incumbent.media_file) {
            return Err(reject_non_upgrade_episode_incumbent(incumbent, new_score));
        }
    }

    let previous_best_score = sorted_incumbents
        .iter()
        .map(|incumbent| media_file_score(&incumbent.media_file))
        .max()
        .unwrap_or(0);
    let primary_incumbent = sorted_incumbents.remove(0);

    Ok(EpisodeUpgradePlan {
        primary_incumbent,
        additional_superseded: sorted_incumbents,
        previous_best_score,
    })
}

async fn cleanup_superseded_episode_incumbents(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    superseded: &[crate::EpisodeScopedMediaFile],
    recycle_config: &crate::recycle_bin::RecycleBinConfig,
) {
    for incumbent in superseded {
        let old_path = PathBuf::from(&incumbent.media_file.file_path);
        if old_path.exists() {
            let manifest = crate::recycle_bin::RecycleManifest {
                recycled_at: chrono::Utc::now().to_rfc3339(),
                original_path: incumbent.media_file.file_path.clone(),
                size_bytes: incumbent.media_file.size_bytes as u64,
                title_id: Some(title.id.clone()),
                reason: "upgrade_replaced".to_string(),
            };

            if let Err(error) =
                crate::recycle_bin::recycle_file(recycle_config, &old_path, manifest).await
            {
                tracing::warn!(
                    error = %error,
                    path = %old_path.display(),
                    file_id = %incumbent.media_file.id,
                    "failed to recycle superseded episode incumbent; deleting stale database record anyway"
                );
            }
        }

        if let Err(error) = app
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::MediaFileDeleted(scryer_domain::MediaFileDeletedEventData {
                    title: title_context_snapshot(title),
                    media_updates: vec![deleted_media_update(
                        incumbent.media_file.file_path.clone(),
                    )],
                    file_id: Some(incumbent.media_file.id.clone()),
                    reason: scryer_domain::MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: incumbent.episode_ids.clone(),
                }),
            ))
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %incumbent.media_file.id,
                "failed to emit superseded episode cleanup event"
            );
        }

        if let Err(error) = app
            .services
            .library
            .media_files
            .delete_media_file(&incumbent.media_file.id)
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %incumbent.media_file.id,
                "failed to delete superseded episode media file record"
            );
        }
    }
}

/// Import a single episode video file: parse, gate, import, and link.
async fn import_single_episode_file(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    media_root: &str,
    rename_template: &str,
    title_folder: &str,
    completed: &CompletedDownload,
    source_video: &Path,
    other_video_files: bool,
    quality_profile: &crate::QualityProfile,
    nfo_enabled: bool,
) -> AppResult<EpisodeImportOutcome> {
    let parsed =
        build_augmented_episode_import_metadata(source_video, completed, other_video_files);

    // Must have episode info to proceed
    let ep_meta = match parsed.episode.as_ref() {
        Some(ep) if !ep.episode_numbers.is_empty() => ep,
        Some(ep)
            if ep.absolute_episode.is_some() && title.facet == scryer_domain::MediaFacet::Anime =>
        {
            ep
        }
        Some(ep) if ep.air_date.is_some() => ep,
        Some(ep) if ep.release_type == crate::ParsedEpisodeReleaseType::SeasonPack => ep,
        _ => {
            tracing::debug!(
                file = %source_video.display(),
                "skipping file with no parseable episode info"
            );
            return Ok(EpisodeImportOutcome::Skipped {
                message: "file has no parseable episode info".to_string(),
                reason_code: None,
                skip_reason: Some(ImportSkipReason::NoVideoFiles),
            });
        }
    };

    let season = ep_meta.season.unwrap_or(1);
    let season_str = season.to_string();

    // Resolve target episodes early so we can enrich rename tokens with DB
    // metadata (e.g. absolute_number from TVDB).
    let target_episodes = resolve_target_episodes(app, title, ep_meta, &season_str).await;
    let target_episode_ids: Vec<String> = target_episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect();
    let ep_num_str = ep_meta
        .episode_numbers
        .first()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let abs_str = ep_meta.absolute_episode.map(|n| n.to_string()).or_else(|| {
        target_episodes
            .first()
            .and_then(|ep| ep.absolute_number.clone())
    });
    let episode_title = target_episodes.first().and_then(|ep| ep.title.as_deref());
    let outcome = execute_resolved_episode_import(
        app,
        actor,
        title,
        media_root,
        rename_template,
        title_folder,
        source_video,
        &parsed,
        &target_episodes,
        &target_episodes,
        season as u32,
        &ep_num_str,
        abs_str.as_deref(),
        episode_title,
        quality_profile,
        None,
    )
    .await?;

    match &outcome {
        EpisodeImportOutcome::Imported {
            dest_path,
            imported_media_file_id,
            reason_code,
            ..
        } => {
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "imported",
                reason_code.as_deref(),
                imported_media_file_id.as_deref(),
                &target_episodes,
            )
            .await;

            if imported_media_file_id.is_some() {
                if nfo_enabled {
                    let nfo_path = std::path::Path::new(dest_path).with_extension("nfo");
                    if let Some(episode) = target_episodes.first() {
                        let nfo_content = render_episode_nfo(title, episode);
                        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await
                        {
                            tracing::warn!(
                                error = %err,
                                path = %nfo_path.display(),
                                "failed to write episode NFO sidecar"
                            );
                        }
                    }
                }

                spawn_post_processing(PostProcessingContext {
                    app: app.clone(),
                    actor_id: Some(actor.id.clone()),
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    facet: title.facet.clone(),
                    dest_path: PathBuf::from(dest_path),
                    year: title.year,
                    imdb_id: title
                        .external_ids
                        .iter()
                        .find(|e| e.source == "imdb")
                        .map(|e| e.value.clone()),
                    tvdb_id: title
                        .external_ids
                        .iter()
                        .find(|e| e.source == "tvdb")
                        .map(|e| e.value.clone()),
                    season: Some(season),
                    episode: ep_meta.episode_numbers.first().copied(),
                    quality: parsed.quality.clone(),
                });
            }
        }
        EpisodeImportOutcome::Skipped { reason_code, .. } => {
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "rejected",
                reason_code.as_deref(),
                None,
                &target_episodes,
            )
            .await;
        }
        EpisodeImportOutcome::Rejected {
            rejection,
            finalize_before_import,
            reason_code,
        } => {
            if *finalize_before_import {
                crate::post_download_gate::reject_source_file_before_import(
                    app,
                    Some(&actor.id),
                    title,
                    &completed.name,
                    source_video,
                    &target_episode_ids,
                    rejection,
                )
                .await;
            }

            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "rejected",
                reason_code
                    .as_deref()
                    .or_else(|| rejection.skip_reason.as_ref().map(ImportSkipReason::as_str)),
                None,
                &target_episodes,
            )
            .await;
        }
    }

    Ok(outcome)
}

async fn execute_resolved_episode_import(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    media_root: &str,
    rename_template: &str,
    title_folder: &str,
    source_video: &Path,
    parsed: &crate::ParsedReleaseMetadata,
    target_episodes: &[scryer_domain::Episode],
    coverage_episodes: &[scryer_domain::Episode],
    rename_season: u32,
    rename_episode_number: &str,
    rename_absolute_number: Option<&str>,
    rename_episode_title: Option<&str>,
    quality_profile: &crate::QualityProfile,
    quality_override: Option<String>,
) -> AppResult<EpisodeImportOutcome> {
    let source_size = std::fs::metadata(source_video)
        .map(|metadata| metadata.len() as i64)
        .unwrap_or(0);
    let target_episode_ids = target_episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect::<Vec<_>>();
    let is_filler = target_episodes.iter().any(|episode| episode.is_filler);
    let existing_incumbents = app
        .services
        .library
        .media_files
        .list_live_media_files_for_episode_ids(&title.id, &target_episode_ids)
        .await
        .unwrap_or_default();
    let existing_files = existing_incumbents
        .iter()
        .map(|incumbent| incumbent.media_file.clone())
        .collect::<Vec<_>>();
    let existing_score = existing_files
        .iter()
        .max_by_key(|file| file.acquisition_score.unwrap_or(0))
        .and_then(|file| file.acquisition_score);
    let prepared = match crate::post_download_gate::prepare_import_candidate(
        app,
        title,
        parsed,
        quality_profile,
        source_video,
        source_size,
        !existing_files.is_empty(),
        existing_score,
        is_filler,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(rejection) => {
            return Ok(EpisodeImportOutcome::Rejected {
                rejection,
                finalize_before_import: true,
                reason_code: None,
            });
        }
    };

    if let Err(issue) = super::coverage_validation::validate_broad_episode_coverage(
        title,
        &prepared.parsed,
        coverage_episodes,
        prepared.accepted.as_ref(),
    ) {
        tracing::info!(
            code = issue.code,
            expected_runtime_minutes = issue.expected_runtime_minutes,
            actual_runtime_minutes = issue.actual_runtime_minutes,
            covered_episode_count = issue.covered_episode_count,
            real_runtime_coverage_count = issue.real_runtime_coverage_count,
            file = %source_video.display(),
            "rejecting implausible episode coverage during import"
        );
        return Ok(EpisodeImportOutcome::Rejected {
            rejection: crate::post_download_gate::ImportedFileRejection {
                message: issue.message,
                recycle_reason: super::coverage_validation::COVERAGE_RUNTIME_MISMATCH_CODE,
                skip_reason: Some(ImportSkipReason::PolicyMismatch),
                blocking_rule_codes: Vec::new(),
            },
            finalize_before_import: true,
            reason_code: Some(
                super::coverage_validation::COVERAGE_RUNTIME_MISMATCH_CODE.to_string(),
            ),
        });
    }

    let ext = source_video
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("mkv")
        .to_string();
    let effective_quality_label = quality_override
        .as_deref()
        .and_then(|value| non_empty_string(Some(value.to_string())))
        .or_else(|| prepared.parsed.quality.clone());
    let effective_parsed =
        parsed_with_quality_override(&prepared.parsed, effective_quality_label.as_deref());
    let dest_path = episode_import_dest_path(
        title,
        &effective_parsed,
        &ext,
        media_root,
        title_folder,
        rename_template,
        rename_season,
        rename_episode_number,
        rename_absolute_number,
        rename_episode_title,
        effective_quality_label.as_deref(),
    );

    let check_ctx = crate::import_checks::ImportCheckContext {
        source_path: source_video,
        dest_path: &dest_path,
        source_size: source_size as u64,
        parsed: &prepared.parsed,
        existing_files: &existing_files,
    };
    if let crate::import_checks::ImportVerdict::Reject { reason, code } =
        crate::import_checks::run_import_checks(&check_ctx)
    {
        tracing::debug!(file = %dest_path.display(), %code, %reason, "skipping episode file");
        return Ok(EpisodeImportOutcome::Skipped {
            message: reason,
            reason_code: Some(code.to_string()),
            skip_reason: Some(skip_reason_for_import_check_code(code)),
        });
    }

    if !existing_incumbents.is_empty() {
        let (required_audio_languages, persona) = resolve_import_audio_persona(app, title).await;
        let new_decision = crate::post_download_gate::build_import_profile_decision(
            quality_profile,
            &required_audio_languages,
            &persona,
            &effective_parsed,
            crate::post_download_gate::facet_to_category_hint(&title.facet),
            title.runtime_minutes,
            Some(source_size),
            true,
        );
        let new_score = new_decision.preference_score;
        let upgrade_plan = match build_episode_upgrade_plan(
            &existing_incumbents,
            &target_episode_ids,
            new_score,
        ) {
            Ok(plan) => plan,
            Err(rejection) => {
                return Ok(EpisodeImportOutcome::Rejected {
                    rejection,
                    finalize_before_import: true,
                    reason_code: None,
                });
            }
        };
        let recycle_config =
            crate::recycle_bin::resolve_recycle_config(app, Some(media_root)).await;

        match crate::upgrade::execute_upgrade(
            app,
            actor,
            title,
            &upgrade_plan.primary_incumbent.media_file,
            source_video,
            &dest_path,
            &prepared,
            effective_quality_label.as_deref(),
            new_score,
            upgrade_plan.previous_best_score,
            &target_episode_ids,
            &recycle_config,
        )
        .await
        {
            Ok(crate::upgrade::UpgradeResult::Upgraded(outcome)) => {
                cleanup_superseded_episode_incumbents(
                    app,
                    title,
                    &upgrade_plan.additional_superseded,
                    &recycle_config,
                )
                .await;
                tracing::info!(
                    title = %title.name,
                    old_score = outcome.old_score,
                    new_score = outcome.new_score,
                    superseded_files = upgrade_plan.additional_superseded.len() + 1,
                    "episode file upgraded"
                );
                for episode_id in &target_episode_ids {
                    mark_wanted_completed(app, &title.id, Some(episode_id), None).await;
                }
                return Ok(EpisodeImportOutcome::Imported {
                    dest_path: dest_path.to_string_lossy().to_string(),
                    episode_ids: target_episode_ids,
                    imported_media_file_id: None,
                    reason_code: Some("upgrade".to_string()),
                });
            }
            Ok(crate::upgrade::UpgradeResult::Rejected(rejection)) => {
                return Ok(EpisodeImportOutcome::Rejected {
                    rejection,
                    finalize_before_import: false,
                    reason_code: None,
                });
            }
            Err(err) => {
                tracing::error!(error = %err, "episode upgrade failed");
                return Err(err);
            }
        }
    }

    let file_result = app
        .services
        .workflow
        .file_importer
        .import_file(source_video, &dest_path)
        .await?;

    let has_existing = existing_files
        .iter()
        .any(|file| file.file_path == dest_path.to_string_lossy().as_ref() as &str);
    let acq_score = crate::post_download_gate::compute_acquisition_score(
        app,
        &effective_parsed,
        prepared.accepted.as_ref(),
        quality_profile,
        title,
        file_result.size_bytes as i64,
        has_existing,
    )
    .await;

    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: dest_path.to_string_lossy().to_string(),
        size_bytes: file_result.size_bytes as i64,
        quality_label: effective_quality_label.clone(),
        scene_name: Some(prepared.parsed.raw_title.clone()),
        release_group: prepared.parsed.release_group.clone(),
        source_type: prepared.parsed.source.clone(),
        resolution: effective_quality_label,
        video_codec_parsed: prepared.parsed.video_codec.clone(),
        audio_codec_parsed: prepared.parsed.audio.clone(),
        audio_channels_parsed: prepared.parsed.audio_channels.clone(),
        original_file_path: Some(source_video.to_string_lossy().to_string()),
        acquisition_score: Some(acq_score),
        ..Default::default()
    };
    let media_file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await?;
    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        &media_file_id,
        prepared.accepted.as_ref(),
    )
    .await;
    if let Err(error) = crate::subtitles::reconcile_external_subtitles_for_media_file(
        app,
        &title.id,
        &media_file_id,
        if target_episodes.len() == 1 {
            target_episodes.first().map(|episode| episode.id.as_str())
        } else {
            None
        },
        &dest_path,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            title_id = %title.id,
            file_id = %media_file_id,
            dest_path = %dest_path.display(),
            "failed to reconcile external subtitles after import"
        );
    }
    maybe_trigger_subtitle_search(app, &title.id, &media_file_id);

    for episode in target_episodes {
        if let Err(err) = app
            .services
            .library
            .media_files
            .link_file_to_episode(&media_file_id, &episode.id)
            .await
        {
            tracing::warn!(error = %err, episode_id = %episode.id, "failed to link file to episode");
        }
        mark_wanted_completed(app, &title.id, Some(&episode.id), None).await;
    }

    Ok(EpisodeImportOutcome::Imported {
        dest_path: dest_path.to_string_lossy().to_string(),
        episode_ids: target_episode_ids,
        imported_media_file_id: Some(media_file_id),
        reason_code: None,
    })
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve media root path and rename template for a title's facet.
pub(crate) async fn resolve_import_paths(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> AppResult<(String, String)> {
    let handler = app.facet_registry.get(&title.facet);
    let rename_settings = crate::facet_handler::rename_facet_settings(&title.facet);
    let media_root_key = handler
        .map(|h| h.library_path_key())
        .unwrap_or(SERIES_PATH_KEY);
    let media_root_default = handler
        .map(|h| h.default_library_path())
        .unwrap_or("/data/series");

    let media_root = {
        let default_root = app
            .read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, media_root_key, None)
            .await?
            .unwrap_or_else(|| media_root_default.to_string());

        title
            .tags
            .iter()
            .find(|t| t.starts_with("scryer:root-folder:"))
            .map(|t| t.trim_start_matches("scryer:root-folder:").to_string())
            .unwrap_or(default_root)
    };

    let rename_template = app
        .read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            rename_settings.template_key,
            None,
        )
        .await?
        .unwrap_or_else(|| rename_settings.default_template.to_string());

    Ok((media_root, rename_template))
}

/// Compute the destination path for an episode import using the canonical
/// token set: base tokens from parsed release metadata, overridden by the
/// explicit episode values supplied by the caller.
///
/// `ep_num_str` may be empty to leave `{episode}` blank (anime absolute-only
/// files where no per-season episode number is known).
/// `quality_override` replaces the filename-parsed quality token when the
/// caller supplies an explicit label (e.g. manual import).
pub(crate) fn episode_import_dest_path(
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
    media_root: &str,
    title_folder: &str,
    rename_template: &str,
    season_num: u32,
    ep_num_str: &str,
    absolute_number: Option<&str>,
    episode_title: Option<&str>,
    quality_override: Option<&str>,
) -> PathBuf {
    let mut tokens = build_rename_tokens(title, parsed, ext);
    tokens.insert("season".to_string(), season_num.to_string());
    tokens.insert("season_order".to_string(), season_num.to_string());
    tokens.insert("episode".to_string(), ep_num_str.to_string());
    tokens.insert(
        "absolute_episode".to_string(),
        absolute_number.unwrap_or("").to_string(),
    );
    tokens.insert(
        "episode_title".to_string(),
        episode_title.unwrap_or("").to_string(),
    );
    if let Some(q) = quality_override {
        tokens.insert("quality".to_string(), q.to_string());
    }
    let rendered = render_rename_template(rename_template, &tokens);
    if use_season_folders(title) {
        let season_folder = format!("Season {:02}", season_num);
        PathBuf::from(media_root)
            .join(title_folder)
            .join(&season_folder)
            .join(&rendered)
    } else {
        PathBuf::from(media_root).join(title_folder).join(&rendered)
    }
}

/// Check whether the title's tags request season-folder organisation.
/// Defaults to `true` (use season folders) when the tag is absent.
pub(crate) fn use_season_folders(title: &scryer_domain::Title) -> bool {
    title
        .tags
        .iter()
        .find(|t| t.starts_with("scryer:season-folder:"))
        .map(|t| {
            !t.trim_start_matches("scryer:season-folder:")
                .eq_ignore_ascii_case("disabled")
        })
        .unwrap_or(true)
}

/// Build the common rename token map from parsed release metadata.
pub(crate) fn build_rename_tokens(
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    let fallback_title_year = title.year;
    let resolved_year = parsed.year.or(fallback_title_year);
    tokens.insert("title".to_string(), title.name.clone());
    tokens.insert(
        "year".to_string(),
        resolved_year.map(|y| y.to_string()).unwrap_or_default(),
    );
    tokens.insert(
        "quality".to_string(),
        parsed
            .quality
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
    );
    tokens.insert(
        "source".to_string(),
        parsed.source.clone().unwrap_or_default(),
    );
    tokens.insert(
        "video_codec".to_string(),
        parsed.video_codec.clone().unwrap_or_default(),
    );
    tokens.insert(
        "audio".to_string(),
        parsed.audio.clone().unwrap_or_default(),
    );
    tokens.insert(
        "release_group".to_string(),
        parsed.release_group.clone().unwrap_or_default(),
    );
    tokens.insert(
        "season".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.season)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.episode_numbers.first().copied())
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "absolute_episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.absolute_episode)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert("episode_title".to_string(), String::new());
    tokens.insert("ext".to_string(), ext.to_string());
    tokens
}

/// Mark a wanted item as completed for a title (and optionally a specific episode).
/// If `imported_score` is provided, it becomes the new `current_score`.
/// If the quality profile allows upgrades, the item re-enters "wanted" status
/// with a recomputed schedule (the 24h cooldown in `evaluate_upgrade` prevents churn).
pub(crate) async fn mark_wanted_completed(
    app: &AppUseCase,
    title_id: &str,
    episode_id: Option<&str>,
    imported_score: Option<i32>,
) {
    let now = Utc::now().to_rfc3339();

    match app
        .services
        .workflow
        .wanted_items
        .complete_wanted_item_for_title(title_id, episode_id, Some(&now), imported_score)
        .await
    {
        Ok(true) => {}
        Ok(false) => {}
        Err(err) => {
            tracing::warn!(error = %err, title_id = %title_id, "failed to mark wanted item completed");
        }
    }
}

async fn resolve_import_quality_profile(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> crate::QualityProfile {
    let tvdb_id = title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == "tvdb")
        .map(|external_id| external_id.value.as_str());
    let category_hint = crate::post_download_gate::facet_to_category_hint(&title.facet);
    match app
        .resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
            title_tags: &title.tags,
            imdb_id: title.imdb_id.as_deref(),
            tvdb_id,
            category_hint: Some(category_hint),
        })
        .await
    {
        Ok(profile) => profile,
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = %title.id,
                "failed to resolve quality profile, using default"
            );
            crate::default_quality_profile_for_search()
        }
    }
}

async fn resolve_import_audio_persona(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> (Vec<String>, crate::ScoringPersona) {
    let category_hint = crate::post_download_gate::facet_to_category_hint(&title.facet);
    let required_audio_languages = app
        .resolve_required_audio_languages(Some(&title.id), Some(category_hint))
        .await
        .unwrap_or_default();
    let persona = app
        .resolve_scoring_persona(Some(category_hint))
        .await
        .unwrap_or_default();

    (required_audio_languages, persona)
}

pub(crate) async fn resolve_target_episodes(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
) -> Vec<scryer_domain::Episode> {
    let mut episodes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let target_season = crate::parsed_episode_lookup_season(ep_meta, season_str);

    if let Some(air_date) = ep_meta.air_date {
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        match app
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(collections) => {
                let mut matches = Vec::new();
                for collection in collections {
                    match app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(&collection.id)
                        .await
                    {
                        Ok(collection_episodes) => {
                            matches.extend(collection_episodes.into_iter().filter(|episode| {
                                episode.title_id == title.id
                                    && episode.air_date.as_deref() == Some(air_date_str.as_str())
                            }));
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "daily episode lookup failed during import")
                        }
                    }
                }

                matches.sort_by_key(|episode| {
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                });

                if let Some(part) = ep_meta.daily_part {
                    let part_index = part.saturating_sub(1) as usize;
                    if let Some(episode) = matches.into_iter().nth(part_index)
                        && seen.insert(episode.id.clone())
                    {
                        episodes.push(episode);
                    }
                } else {
                    for episode in matches {
                        if seen.insert(episode.id.clone()) {
                            episodes.push(episode);
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "daily collection lookup failed during import")
            }
        }
    }

    for episode_number in &ep_meta.episode_numbers {
        let episode_str = episode_number.to_string();
        match app
            .services
            .catalog
            .shows
            .find_episode_by_title_and_numbers(&title.id, &target_season, &episode_str)
            .await
        {
            Ok(Some(episode)) => {
                if seen.insert(episode.id.clone()) {
                    episodes.push(episode);
                }
            }
            Ok(None) => {
                tracing::debug!(
                    title_id = %title.id,
                    season = %season_str,
                    episode = %episode_str,
                    "no matching episode found for imported file"
                );
            }
            Err(err) => tracing::warn!(error = %err, "episode lookup failed during import"),
        }
    }

    if episodes.is_empty()
        && ep_meta.season.is_some()
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    {
        match app
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(collections) => {
                for collection in collections
                    .into_iter()
                    .filter(|collection| collection.collection_index == target_season)
                {
                    match app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(&collection.id)
                        .await
                    {
                        Ok(collection_episodes) => {
                            let mut collection_episodes: Vec<_> = collection_episodes
                                .into_iter()
                                .filter(|episode| {
                                    episode.title_id == title.id
                                        && episode.season_number.as_deref()
                                            == Some(target_season.as_str())
                                })
                                .collect();
                            collection_episodes.sort_by_key(|episode| {
                                episode
                                    .episode_number
                                    .as_deref()
                                    .and_then(|value| value.parse::<u32>().ok())
                                    .unwrap_or(u32::MAX)
                            });
                            for episode in collection_episodes {
                                if seen.insert(episode.id.clone()) {
                                    episodes.push(episode);
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "season episode lookup failed during import")
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "season collection lookup failed during import")
            }
        }
    }

    if episodes.is_empty() && !ep_meta.special_absolute_episode_numbers.is_empty() {
        for special_number in &ep_meta.special_absolute_episode_numbers {
            let episode_str = special_number.to_string();
            match app
                .services
                .catalog
                .shows
                .find_episode_by_title_and_numbers(&title.id, "0", &episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        special = %episode_str,
                        "no matching special episode found during import"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "special episode lookup failed during import")
                }
            }
        }
    }

    if episodes.is_empty()
        && (ep_meta.absolute_episode.is_some() || !ep_meta.absolute_episode_numbers.is_empty())
    {
        let absolute_numbers: Vec<u32> = if !ep_meta.absolute_episode_numbers.is_empty() {
            ep_meta.absolute_episode_numbers.clone()
        } else if ep_meta.episode_numbers.is_empty() {
            vec![ep_meta.absolute_episode.unwrap_or_default()]
        } else {
            ep_meta.episode_numbers.clone()
        };

        for absolute_number in absolute_numbers {
            let absolute_episode_str = absolute_number.to_string();
            match app
                .services
                .catalog
                .shows
                .find_episode_by_title_and_absolute_number(&title.id, &absolute_episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        absolute = absolute_number,
                        "no matching episode found by absolute number"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "episode absolute lookup failed during import")
                }
            }
        }
    }

    episodes
}

fn prefer_broader_coverage_episodes(
    target_episodes: &[scryer_domain::Episode],
    claimed_episodes: Vec<scryer_domain::Episode>,
) -> Vec<scryer_domain::Episode> {
    if claimed_episodes.len() > target_episodes.len() {
        claimed_episodes
    } else {
        target_episodes.to_vec()
    }
}

fn parsed_with_quality_override(
    parsed: &crate::ParsedReleaseMetadata,
    quality_label: Option<&str>,
) -> crate::ParsedReleaseMetadata {
    let mut effective = parsed.clone();
    if let Some(quality_label) = quality_label {
        effective.quality = Some(quality_label.to_string());
    }
    effective
}

async fn resolve_manual_import_coverage_episodes(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    fallback_season: u32,
    target_episodes: &[scryer_domain::Episode],
) -> Vec<scryer_domain::Episode> {
    let Some(ep_meta) = parsed.episode.as_ref() else {
        return target_episodes.to_vec();
    };

    let claimed_episodes =
        resolve_target_episodes(app, title, ep_meta, &fallback_season.to_string()).await;
    prefer_broader_coverage_episodes(target_episodes, claimed_episodes)
}

async fn write_series_sidecars(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    media_root: &str,
    title_folder: &str,
    nfo_enabled: bool,
) {
    if nfo_enabled {
        let tvshow_nfo_path = PathBuf::from(media_root)
            .join(title_folder)
            .join("tvshow.nfo");
        if !tvshow_nfo_path.exists() {
            if let Some(parent) = tvshow_nfo_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let nfo_content = render_tvshow_nfo(title);
            if let Err(err) = tokio::fs::write(&tvshow_nfo_path, nfo_content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %tvshow_nfo_path.display(),
                    "failed to write tvshow NFO sidecar"
                );
            }
        }
    }

    let plexmatch_key = match title.facet {
        scryer_domain::MediaFacet::Anime => "plexmatch.write_on_import.anime",
        _ => "plexmatch.write_on_import.series",
    };
    let plexmatch_enabled = app
        .read_setting_string_value(plexmatch_key, None)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true");
    if plexmatch_enabled {
        let plexmatch_path = PathBuf::from(media_root)
            .join(title_folder)
            .join(".plexmatch");
        if !plexmatch_path.exists() {
            if let Some(parent) = plexmatch_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let content = render_plexmatch(title);
            if let Err(err) = tokio::fs::write(&plexmatch_path, content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %plexmatch_path.display(),
                    "failed to write .plexmatch hint file"
                );
            }
        }
    }
}

async fn persist_file_import_artifact(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    title_id: &str,
    source_path: &Path,
    media_kind: &str,
    result: &str,
    reason_code: Option<&str>,
    imported_media_file_id: Option<&str>,
    episodes: &[scryer_domain::Episode],
) {
    let relative_path = source_path
        .strip_prefix(&completed.dest_dir)
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .filter(|path| !path.is_empty());
    let normalized_file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .unwrap_or_else(|| source_path.to_string_lossy().to_ascii_lowercase());

    let episode_rows: Vec<(Option<String>, Option<i32>, Option<i32>)> = if episodes.is_empty() {
        vec![(None, None, None)]
    } else {
        episodes
            .iter()
            .map(|episode| {
                (
                    Some(episode.id.clone()),
                    episode
                        .season_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                )
            })
            .collect()
    };

    for (episode_id, season_number, episode_number) in episode_rows {
        let artifact = ImportArtifact {
            id: Id::new().0,
            source_system: completed.client_type.clone(),
            source_ref: completed.download_client_item_id.clone(),
            import_id: Some(import_id.to_string()),
            relative_path: relative_path.clone(),
            normalized_file_name: normalized_file_name.clone(),
            media_kind: media_kind.to_string(),
            title_id: Some(title_id.to_string()),
            episode_id,
            season_number,
            episode_number,
            result: result.to_string(),
            reason_code: reason_code.map(str::to_string),
            imported_media_file_id: imported_media_file_id.map(str::to_string),
            created_at: Utc::now(),
        };
        if let Err(error) = app
            .services
            .workflow
            .import_artifacts
            .insert_artifact(artifact)
            .await
        {
            tracing::warn!(
                error = %error,
                import_id,
                source_ref = %completed.download_client_item_id,
                file = %source_path.display(),
                "failed to persist import artifact"
            );
        }
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

const SAMPLE_SIZE_THRESHOLD: u64 = 50 * 1024 * 1024; // 50 MB

pub(crate) fn is_sample_file(path: &Path) -> bool {
    let filename = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if filename.contains("sample") {
        return true;
    }

    // Small files in multi-episode directories are almost certainly samples/promos
    std::fs::metadata(path)
        .map(|m| m.len() < SAMPLE_SIZE_THRESHOLD)
        .unwrap_or(false)
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
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(fallback);
    parse_release_metadata(stem)
}

fn parsed_release_from_folder_name(path: &Path) -> Option<ParsedReleaseMetadata> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_release_metadata)
}

fn fill_missing_release_metadata(
    target: &mut ParsedReleaseMetadata,
    fallback: &ParsedReleaseMetadata,
    prefer_episode: bool,
) {
    if prefer_episode
        && target.episode.as_ref().is_none_or(|file_episode| {
            fallback.episode.as_ref().is_some_and(|other_episode| {
                prefer_other_episode_info(Some(file_episode), other_episode)
            })
        })
    {
        if fallback.episode.is_some() {
            target.episode = fallback.episode.clone();
        }
    } else if target.episode.is_none() && fallback.episode.is_some() {
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
        target.source = fallback.source.clone();
    }
    if target.video_codec.is_none() {
        target.video_codec = fallback.video_codec.clone();
    }
    if target.video_encoding.is_none() {
        target.video_encoding = fallback.video_encoding.clone();
    }
    if target.audio.is_none() {
        target.audio = fallback.audio.clone();
    }
    if target.audio_channels.is_none() {
        target.audio_channels = fallback.audio_channels.clone();
    }
    if target.release_group.is_none() {
        target.release_group = fallback.release_group.clone();
    }
    if target.streaming_service.is_none() {
        target.streaming_service = fallback.streaming_service.clone();
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

fn prefer_other_episode_info(
    file_episode_info: Option<&ParsedEpisodeMetadata>,
    other_episode_info: &ParsedEpisodeMetadata,
) -> bool {
    let Some(file_episode_info) = file_episode_info else {
        return true;
    };

    if file_episode_info.absolute_episode.is_none() && other_episode_info.absolute_episode.is_some()
    {
        return false;
    }

    true
}

fn build_augmented_episode_import_metadata(
    source_video: &Path,
    completed: &CompletedDownload,
    other_video_files: bool,
) -> ParsedReleaseMetadata {
    let mut parsed = parsed_release_from_file_stem(source_video);
    let file_episode = parsed.episode.clone();
    let download_client_info = parse_release_metadata(&completed.name);
    let folder_info = parsed_release_from_folder_name(Path::new(&completed.dest_dir));

    if !other_video_files {
        if let Some(other_episode_info) = download_client_info.episode.as_ref()
            && !other_episode_info.full_season
            && prefer_other_episode_info(parsed.episode.as_ref(), other_episode_info)
        {
            fill_missing_release_metadata(&mut parsed, &download_client_info, true);
            return parsed;
        }

        if let Some(folder_info) = folder_info.as_ref()
            && let Some(other_episode_info) = folder_info.episode.as_ref()
            && !other_episode_info.full_season
            && prefer_other_episode_info(parsed.episode.as_ref(), other_episode_info)
        {
            fill_missing_release_metadata(&mut parsed, folder_info, true);
            return parsed;
        }
    }

    fill_missing_release_metadata(&mut parsed, &download_client_info, false);
    if let Some(folder_info) = folder_info.as_ref() {
        fill_missing_release_metadata(&mut parsed, folder_info, false);
    }
    if other_video_files {
        parsed.episode = file_episode;
    }
    parsed
}

fn build_augmented_movie_import_metadata(
    source_video: &Path,
    completed: &CompletedDownload,
) -> ParsedReleaseMetadata {
    let mut parsed = parsed_release_from_file_stem(source_video);
    let download_client_info = parse_release_metadata(&completed.name);
    fill_missing_release_metadata(&mut parsed, &download_client_info, false);
    if let Some(folder_info) = parsed_release_from_folder_name(Path::new(&completed.dest_dir)) {
        fill_missing_release_metadata(&mut parsed, &folder_info, false);
    }
    parsed
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        if value.trim().is_empty() {
            None
        } else {
            Some(value)
        }
    })
}

// ---------------------------------------------------------------------------
// Manual import: preview & execute
// ---------------------------------------------------------------------------

/// A single file in a manual import preview with auto-detected episode info.
pub struct ManualImportFilePreview {
    pub file_path: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub quality: Option<String>,
    pub parsed_season: Option<u32>,
    pub parsed_episodes: Vec<u32>,
    pub suggested_episode_id: Option<String>,
    pub suggested_episode_label: Option<String>,
}

/// Result of previewing a manual import: file list + available episodes for matching.
pub struct ManualImportPreview {
    pub files: Vec<ManualImportFilePreview>,
    pub available_episodes: Vec<scryer_domain::Episode>,
}

/// Scan a completed download's directory and attempt to auto-match files to episodes.
pub async fn preview_manual_import(
    app: &AppUseCase,
    client_id: Option<&str>,
    download_client_item_id: &str,
    title_id: &str,
) -> AppResult<ManualImportPreview> {
    let completed = match app
        .resolve_manual_import_source(client_id, None, download_client_item_id)
        .await?
    {
        crate::ManualImportSourceResolution::Eligible {
            completed: Some(completed),
        } => completed,
        crate::ManualImportSourceResolution::Eligible { completed: None } => {
            return Err(AppError::NotFound(format!(
                "completed download not found: {}",
                download_client_item_id
            )));
        }
        crate::ManualImportSourceResolution::SourceFailed { message } => {
            return Err(AppError::Validation(format!(
                "source_job_failed: {message}"
            )));
        }
        crate::ManualImportSourceResolution::NotEligible { message } => {
            return Err(AppError::Validation(message));
        }
    };

    // Scan for video files (recursive, no sample filtering - let user see everything)
    let dest_dir = Path::new(&completed.dest_dir);
    let video_files = find_video_files(dest_dir, false)?;

    // Get all episodes for this title across all seasons
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

    // For each file, parse and attempt auto-match
    let mut previews = Vec::new();
    let other_video_files = video_files.len() > 1;
    for path in &video_files {
        let file_name = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown")
            .to_string();
        let size = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);

        let parsed = build_augmented_episode_import_metadata(path, &completed, other_video_files);

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

        previews.push(ManualImportFilePreview {
            file_path: path.to_string_lossy().to_string(),
            file_name,
            size_bytes: size,
            quality: parsed.quality.clone(),
            parsed_season,
            parsed_episodes,
            suggested_episode_id,
            suggested_episode_label,
        });
    }

    Ok(ManualImportPreview {
        files: previews,
        available_episodes: all_episodes,
    })
}

/// A user-specified mapping of one file to one episode.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportFileMapping {
    pub file_path: String,
    pub episode_id: String,
    pub quality: Option<String>,
}

/// Per-file result of a manual import execution.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualImportFileResult {
    pub file_path: String,
    pub episode_id: String,
    pub success: bool,
    pub dest_path: Option<String>,
    pub error_code: Option<ImportErrorCode>,
    pub error_message: Option<String>,
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
        .list_imports_for_sources(&[(normalized_client_type.clone(), source_ref.to_string())])
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

fn resolve_queued_manual_import_completed_source(
    import_id: &str,
    payload: &ManualImportRequestPayload,
    source_resolution: crate::ManualImportSourceResolution,
) -> Result<Option<CompletedDownload>, (ImportStatus, Option<String>)> {
    match source_resolution {
        crate::ManualImportSourceResolution::Eligible { completed } => Ok(completed),
        crate::ManualImportSourceResolution::SourceFailed { message }
        | crate::ManualImportSourceResolution::NotEligible { message } => {
            if payload.files.is_empty() {
                Err((
                    ImportStatus::Failed,
                    manual_import_source_failed_result_json(import_id, payload, message),
                ))
            } else {
                Ok(None)
            }
        }
    }
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

/// Execute a manual import: import each file with user-specified episode mappings.
pub async fn execute_manual_import(
    app: &AppUseCase,
    actor: &User,
    title_id: &str,
    completed: Option<&CompletedDownload>,
    files: Vec<ManualImportFileMapping>,
) -> AppResult<Vec<ManualImportFileResult>> {
    require(actor, &Entitlement::ManageTitle)?;
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title not found: {}", title_id)))?;

    let (media_root, rename_template) = resolve_import_paths(app, &title).await?;
    let title_folder = sanitize_filesystem_component(&title.name);
    let quality_profile = resolve_import_quality_profile(app, &title).await;

    let mut results = Vec::new();

    for mapping in &files {
        let source = Path::new(&mapping.file_path);

        // Validate file exists
        if !source.exists() || !source.is_file() {
            results.push(ManualImportFileResult {
                file_path: mapping.file_path.clone(),
                episode_id: mapping.episode_id.clone(),
                success: false,
                dest_path: None,
                error_code: Some(ImportErrorCode::FileNotFound),
                error_message: Some(format!("file not found: {}", mapping.file_path)),
            });
            continue;
        }

        // Look up episode
        let episode = match app
            .services
            .catalog
            .shows
            .get_episode_by_id(&mapping.episode_id)
            .await
        {
            Ok(Some(ep)) => ep,
            Ok(None) => {
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: false,
                    dest_path: None,
                    error_code: Some(ImportErrorCode::EpisodeNotFound),
                    error_message: Some(format!("episode not found: {}", mapping.episode_id)),
                });
                continue;
            }
            Err(err) => {
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: false,
                    dest_path: None,
                    error_code: Some(ImportErrorCode::EpisodeLookupFailed),
                    error_message: Some(format!("episode lookup failed: {}", err)),
                });
                continue;
            }
        };

        // Parse release metadata for quality/codec tokens
        let parsed = completed
            .map(|completed| {
                build_augmented_episode_import_metadata(source, completed, files.len() > 1)
            })
            .unwrap_or_else(|| parsed_release_from_file_stem(source));

        let season_num: u32 = episode
            .season_number
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let ep_num_str = episode.episode_number.clone().unwrap_or_default();
        let coverage_episodes = resolve_manual_import_coverage_episodes(
            app,
            &title,
            &parsed,
            season_num,
            std::slice::from_ref(&episode),
        )
        .await;
        match execute_resolved_episode_import(
            app,
            actor,
            &title,
            &media_root,
            &rename_template,
            &title_folder,
            source,
            &parsed,
            std::slice::from_ref(&episode),
            &coverage_episodes,
            season_num,
            &ep_num_str,
            episode.absolute_number.as_deref(),
            episode.title.as_deref(),
            &quality_profile,
            mapping.quality.clone(),
        )
        .await
        {
            Ok(EpisodeImportOutcome::Imported { dest_path, .. }) => {
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: true,
                    dest_path: Some(dest_path),
                    error_code: None,
                    error_message: None,
                });
            }
            Ok(EpisodeImportOutcome::Skipped {
                message,
                skip_reason,
                ..
            }) => {
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: false,
                    dest_path: None,
                    error_code: Some(manual_import_error_from_skip_reason(skip_reason.clone())),
                    error_message: Some(message),
                });
            }
            Ok(EpisodeImportOutcome::Rejected { rejection, .. }) => {
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: false,
                    dest_path: None,
                    error_code: Some(manual_import_error_from_skip_reason(
                        rejection.skip_reason.clone(),
                    )),
                    error_message: Some(rejection.message),
                });
            }
            Err(err) => {
                let error_message = err.to_string();
                results.push(ManualImportFileResult {
                    file_path: mapping.file_path.clone(),
                    episode_id: mapping.episode_id.clone(),
                    success: false,
                    dest_path: None,
                    error_code: Some(classify_manual_import_error_message(&error_message)),
                    error_message: Some(error_message),
                });
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
    let episode_ids = results
        .iter()
        .filter(|result| result.success)
        .map(|result| result.episode_id.clone())
        .collect::<Vec<_>>();
    app.append_domain_event(new_title_domain_event(
        Some(actor.id.clone()),
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
            source_title: completed
                .map(|download| download.name.clone())
                .or_else(|| (files.len() == 1).then(|| files[0].file_path.clone())),
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

    Ok(results)
}

fn completed_download_matches_manual_import(
    download: &CompletedDownload,
    payload: &ManualImportRequestPayload,
) -> bool {
    if !download
        .client_type
        .eq_ignore_ascii_case(&payload.client_type)
        || download.download_client_item_id != payload.download_client_item_id
    {
        return false;
    }

    let client_id = payload.client_id.as_deref().unwrap_or("").trim();
    client_id.is_empty() || download.client_id == client_id
}

pub async fn execute_queued_manual_import(
    app: &AppUseCase,
    import_id: &str,
    payload: &ManualImportRequestPayload,
) -> AppResult<(ImportStatus, Option<String>)> {
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

    let source_resolution = app
        .resolve_manual_import_source(
            payload.client_id.as_deref(),
            Some(payload.client_type.as_str()),
            &payload.download_client_item_id,
        )
        .await?;
    let completed_source = match resolve_queued_manual_import_completed_source(
        import_id,
        payload,
        source_resolution,
    ) {
        Ok(completed) => completed,
        Err(result) => return Ok(result),
    };

    if payload.files.is_empty() {
        let completed = completed_source.ok_or_else(|| {
            AppError::NotFound(format!(
                "completed download {}",
                payload.download_client_item_id
            ))
        })?;

        let result = app
            .trigger_manual_import(&actor, &completed, payload.title_id.as_deref())
            .await?;

        let (status, error_code, error_message) =
            if matches!(result.decision, ImportDecision::Imported)
                || matches!(result.skip_reason, Some(ImportSkipReason::AlreadyImported))
            {
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

        maybe_remove_completed_manual_import_download(
            app,
            Some(&completed),
            result.title_id.as_deref().or(payload.title_id.as_deref()),
            matches!(result.decision, ImportDecision::Imported),
        )
        .await;

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

        return Ok((status, result_json));
    }

    let title_id = payload.title_id.as_deref().ok_or_else(|| {
        AppError::Validation("title id is required for mapped manual import".into())
    })?;
    let completed = completed_source
        .filter(|download| completed_download_matches_manual_import(download, payload));
    let results = execute_manual_import(
        app,
        &actor,
        title_id,
        completed.as_ref(),
        payload.files.clone(),
    )
    .await?;
    let (status, error_code, error_message) = manual_import_terminal_status_and_error(&results);

    maybe_remove_completed_manual_import_download(
        app,
        completed.as_ref(),
        Some(title_id),
        status == ImportStatus::Completed,
    )
    .await;

    let result_json = manual_import_result_json(
        import_id,
        payload,
        status,
        error_code,
        error_message,
        results,
    );

    Ok((status, result_json))
}

#[cfg(test)]
#[path = "app_usecase_import_tests.rs"]
mod app_usecase_import_tests;
