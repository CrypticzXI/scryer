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

async fn analyze_and_persist_imported_media_file(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
    file_path: &std::path::Path,
) {
    let acceptance = match app
        .services
        .library
        .media_analyzer
        .analyze_file(file_path.to_path_buf())
        .await
    {
        Ok(crate::MediaAnalysisOutcome::Valid(analysis)) => {
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: Some(*analysis),
                scan_error: None,
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
        Ok(crate::MediaAnalysisOutcome::Invalid(error)) => {
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error),
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id,
                file_id = %media_file_id,
                file_path = %file_path.display(),
                "failed to analyze imported media file"
            );
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error.to_string()),
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
    };

    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        media_file_id,
        &acceptance,
    )
    .await;
}

fn completed_download_identity(completed: &CompletedDownload) -> DownloadSourceIdentity {
    DownloadSourceIdentity::new(
        Some(completed.client_id.as_str()),
        &completed.client_type,
        &completed.download_client_item_id,
    )
}
fn additional_import_dest_path(
    canonical_dest_path: &Path,
    parsed: &ParsedReleaseMetadata,
) -> PathBuf {
    let parent = canonical_dest_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = canonical_dest_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("additional");
    let extension = canonical_dest_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mkv");
    let raw_label = parsed
        .edition
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(parsed.raw_title.as_str());
    let sanitized_label = sanitize_filesystem_component(raw_label)
        .trim()
        .chars()
        .take(48)
        .collect::<String>();
    let label = if sanitized_label.is_empty() {
        "additional".to_string()
    } else {
        sanitized_label
    };
    let hash = blake3::hash(parsed.raw_title.as_bytes()).to_hex();
    let hash = &hash.as_str()[..8];
    let base_name = sanitize_filesystem_component(&format!("{stem} - {label} {hash}.{extension}"));
    let mut candidate = parent.join(&base_name);
    if !candidate.exists() {
        return candidate;
    }

    for suffix in 2..=999 {
        let name =
            sanitize_filesystem_component(&format!("{stem} - {label} {hash} ({suffix}).{extension}"));
        candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(sanitize_filesystem_component(&format!(
        "{stem} - {label} {hash} {}.{extension}",
        Id::new().0
    )))
}
const SCRYER_TITLE_ID_PARAM: &str = "*scryer_title_id";
const SCRYER_FACET_PARAM: &str = "*scryer_facet";
const SCRYER_COLLECTION_ID_PARAM: &str = "*scryer_collection_id";
const SCRYER_SERIES_MOVIE_LINK_ID_PARAM: &str = "*scryer_series_movie_link_id";

/// The pure "stamp" step of provenance resolution: a completed download whose
/// live submission is a Scryer grab carries that grab's identity parameters
/// (authoritative over whatever the client echoed) and its persisted indexer
/// release title as `release_name`. A submission recorded without a release
/// title must not blank a real client-reported name; the completed download
/// keeps it.
fn stamp_scryer_submission_origin(
    completed: &CompletedDownload,
    submission: &DownloadSubmission,
) -> CompletedDownload {
    let mut resolved = completed.clone();
    resolved.parameters =
        authoritative_scryer_origin_parameters(&completed.parameters, submission);
    resolved.release_name = submission_source_title(submission)
        .or_else(|| completed_observed_release_name(completed));
    resolved
}

fn authoritative_scryer_origin_parameters(
    parameters: &[(String, String)],
    submission: &DownloadSubmission,
) -> Vec<(String, String)> {
    let mut resolved = parameters
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                SCRYER_TITLE_ID_PARAM
                    | SCRYER_FACET_PARAM
                    | SCRYER_COLLECTION_ID_PARAM
                    | SCRYER_SERIES_MOVIE_LINK_ID_PARAM
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    if !submission.title_id.trim().is_empty() {
        resolved.push((
            SCRYER_TITLE_ID_PARAM.to_string(),
            submission.title_id.clone(),
        ));
    }
    if !submission.facet.trim().is_empty() {
        resolved.push((SCRYER_FACET_PARAM.to_string(), submission.facet.clone()));
    }
    match &submission.scope {
        SubmissionScope::Collection { collection_id } => {
            resolved.push((
                SCRYER_COLLECTION_ID_PARAM.to_string(),
                collection_id.clone(),
            ));
        }
        SubmissionScope::SeriesMovie { series_movie_link_id } => {
            resolved.push((
                SCRYER_SERIES_MOVIE_LINK_ID_PARAM.to_string(),
                series_movie_link_id.clone(),
            ));
        }
        SubmissionScope::Episode { .. }
        | SubmissionScope::EpisodeSet { .. }
        | SubmissionScope::Title
        | SubmissionScope::Orphan => {}
    }
    resolved
}
async fn terminal_download_item_is_still_visible(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    is_history: bool,
) -> bool {
    let lookup = if is_history {
        app.services
            .integrations
            .download_client
            .list_history()
            .await
    } else {
        app.services.integrations.download_client.list_queue().await
    };

    match lookup {
        Ok(items) => items.iter().any(|item| {
            item.download_client_item_id == download_client_item_id
                && item.client_type.eq_ignore_ascii_case(client_type)
                && (client_id.is_empty() || item.client_id.trim() == client_id)
        }),
        Err(error) => {
            tracing::warn!(
                error = %error,
                client_id,
                client_type,
                download_client_item_id,
                is_history,
                "failed to confirm download item visibility after delete error"
            );
            true
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalDownloadCleanupOutcome {
    NotConfigured,
    Removed,
    AlreadyGone,
    RetryableFailure,
    /// The torrent is imported but has not discharged its seeding obligation.
    /// The tracked download stays visible in `ImportedSeeding` and re-enters
    /// the gate on the next poll.
    HeldForSeeding,
    /// The seeding obligation is discharged but the profile (or the client's
    /// own nature, as with `torrent-blackhole`) says the entry stays. Nothing
    /// further to reconcile.
    SeedingEntryKept,
}
pub(crate) fn terminal_download_cleanup_is_complete(
    outcome: TerminalDownloadCleanupOutcome,
) -> bool {
    matches!(
        outcome,
        TerminalDownloadCleanupOutcome::NotConfigured
            | TerminalDownloadCleanupOutcome::Removed
            | TerminalDownloadCleanupOutcome::AlreadyGone
            | TerminalDownloadCleanupOutcome::SeedingEntryKept
    )
}
async fn cleanup_routing_scope_for_title_id(
    app: &AppUseCase,
    title_id: Option<&str>,
) -> (Option<String>, Option<MediaFacet>) {
    let Some(title_id) = title_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };

    match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => (Some(title.library_id), Some(title.facet)),
        Ok(None) | Err(_) => (None, None),
    }
}
pub(crate) async fn reconcile_terminal_download_cleanup_for_tracked(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    state: TrackedDownloadState,
) -> TerminalDownloadCleanupOutcome {
    let (library_id, resolved_facet) =
        cleanup_routing_scope_for_title_id(app, tracked.title_id.as_deref()).await;
    let facet = resolved_facet.or_else(|| facet_from_tracked_label(tracked.facet.as_deref()));
    reconcile_terminal_download_cleanup(
        app,
        &tracked.client_id,
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
        library_id.as_deref(),
        facet.as_ref(),
        state,
        // The tracker already answers "is this still in the client?": a row
        // absent from the client's snapshot past the grace window is marked
        // untrackable. Reusing that avoids a per-item listing call every tick.
        tracked.is_trackable,
        // The live tracked row was refreshed from the client earlier in this
        // same tick, so its observation is fresher than the published snapshot
        // (which is only republished *after* reconcile runs). Passing it in is
        // what makes each cycle re-evaluate against current ratio/seed time
        // rather than the answer that first parked the row.
        crate::seeding_gate::observation_from_queue_item(&tracked.client_item),
    )
    .await
}
fn media_file_score(file: &crate::TitleMediaFile) -> i32 {
    file.acquisition_score.unwrap_or(0)
}
/// Scryer's own transient markers on a *non-`Failed`* import result.
///
/// Execution-phase failures never come through here: they arrive as
/// `ImportDecision::Failed` and are retried by the phase rule in
/// `completed_import_result_is_retryable` regardless of the message (Sonarr's
/// model — no error-string catalogue). This list only recognises the transient
/// conditions Scryer itself reports as a `Skipped`/`Rejected` result: a source
/// still being unpacked or changing under the importer, or an active-download
/// marker.
fn completed_import_error_message_is_retryable(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    const SCRYER_TRANSIENT_PHRASES: &[&str] = &[
        "active-download marker",
        "still being unpacked",
        "still_unpacking",
        "source changed",
        "locked",
        "temporarily",
        "not found or inaccessible",
    ];
    SCRYER_TRANSIENT_PHRASES
        .iter()
        .any(|needle| normalized.contains(needle))
}
async fn resolve_import_quality_profile(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> crate::AppResult<crate::QualityProfile> {
    let tvdb_id = title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == "tvdb")
        .map(|external_id| external_id.value.as_str());
    let category_hint = crate::post_download_gate::facet_to_category_hint(&title.facet);
    // Resolution failures propagate: gating an import against a substitute
    // profile silently applies the wrong quality rules, which is the failure
    // mode the strict resolver exists to prevent. A validation failure (e.g. a
    // dangling profile reference) needs operator action and surfaces as a
    // blocked import; any other failure is treated as transient and worded so
    // `completed_import_error_message_is_retryable` re-attempts it.
    app.resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
        title_tags: &title.tags,
        library_id: Some(title.library_id.as_str()),
        imdb_id: title.imdb_id.as_deref(),
        tvdb_id,
        category_hint: Some(category_hint),
    })
    .await
    .map_err(|error| match error {
        crate::AppError::Validation(_) => error,
        other => crate::AppError::Repository(format!(
            "quality profile resolution temporarily unavailable: {other}"
        )),
    })
}
const SAMPLE_SIZE_THRESHOLD: u64 = 50 * 1024 * 1024;
fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}
