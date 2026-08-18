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

#[derive(Clone, Debug)]
enum CompletedDownloadOriginResolution {
    Ready(Box<CompletedDownload>),
    NoScryerOrigin,
}

fn resolve_completed_download_origin(
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
) -> CompletedDownloadOriginResolution {
    match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched)
            if submission_has_scryer_origin(&matched.submission) =>
        {
            let mut resolved = completed.clone();
            resolved.parameters = authoritative_scryer_origin_parameters(
                &completed.parameters,
                &matched.submission,
            );
            // The persisted indexer release title is THE name for a Scryer
            // grab. A submission recorded without one must not blank a real
            // client-reported name; the completed download keeps it.
            resolved.release_name = submission_source_title(&matched.submission)
                .or_else(|| completed_observed_release_name(completed));
            CompletedDownloadOriginResolution::Ready(Box::new(resolved))
        }
        _ => CompletedDownloadOriginResolution::NoScryerOrigin,
    }
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
async fn persist_completed_download_tracked_state(
    app: &AppUseCase,
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
    state: TrackedDownloadState,
) {
    if !state.is_terminal() {
        return;
    }
    let state_identity = match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched) => {
            submission_source_identity(&matched.submission)
        }
        _ => completed_download_identity(completed),
    };

    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .update_tracked_state(&state_identity, state.as_str())
        .await
    {
        tracing::warn!(
            error = %error,
            client_id = completed.client_id.as_str(),
            client_type = completed.client_type.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            tracked_state_client_item_id = state_identity.item_id.as_str(),
            state = state.as_str(),
            "failed to persist completed download terminal state"
        );
    }

    let observed_identity = completed_download_observed_identity(completed);
    let download_identity = match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched) => matched
            .identity
            .clone()
            .filter(|identity| !download_submission_identity_is_empty(identity))
            .or_else(|| {
                (!download_submission_identity_is_empty(&observed_identity))
                    .then_some(observed_identity.clone())
            }),
        CompletedDownloadSubmissionResolution::MissingDownloadId { identity } => {
            Some(identity.clone())
        }
        _ => (!download_submission_identity_is_empty(&observed_identity))
            .then_some(observed_identity.clone()),
    };

    if let Some(download_identity) = download_identity
        && let Err(error) = app
            .services
            .workflow
            .download_submissions
            .record_identity_tracked_state(
                &download_identity,
                Some(&completed_download_identity(completed)),
                state.as_str(),
                None,
                None,
            )
            .await
    {
        tracing::warn!(
            error = %error,
            client_id = completed.client_id.as_str(),
            client_type = completed.client_type.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            state = state.as_str(),
            "failed to persist durable completed download terminal state"
        );
    }
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
}
pub(crate) fn terminal_download_cleanup_is_complete(
    outcome: TerminalDownloadCleanupOutcome,
) -> bool {
    matches!(
        outcome,
        TerminalDownloadCleanupOutcome::NotConfigured
            | TerminalDownloadCleanupOutcome::Removed
            | TerminalDownloadCleanupOutcome::AlreadyGone
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
pub(crate) async fn reconcile_terminal_download_cleanup_for_completed(
    app: &AppUseCase,
    completed: &CompletedDownload,
    state: TrackedDownloadState,
) -> TerminalDownloadCleanupOutcome {
    let title_id = extract_parameter(&completed.parameters, "*scryer_title_id").unwrap_or_default();
    let (library_id, resolved_facet) =
        cleanup_routing_scope_for_title_id(app, Some(title_id.as_str())).await;
    let facet = resolved_facet.or_else(|| facet_for_completed_download(completed));
    reconcile_terminal_download_cleanup(
        app,
        &completed.client_id,
        &completed.client_type,
        &completed.download_client_item_id,
        library_id.as_deref(),
        facet.as_ref(),
        state,
    )
    .await
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
    )
    .await
}
fn media_file_score(file: &crate::TitleMediaFile) -> i32 {
    file.acquisition_score.unwrap_or(0)
}
/// Whether a completed-import failure is a transient condition worth retrying
/// automatically instead of blocking the download until an operator retries.
///
/// Sonarr has no allowlist here: `ImportApprovedEpisodes` turns every
/// transfer-time exception (locked file, permissions, root folder missing,
/// destination exists, any IO error) into a `Skipped` result and re-attempts on
/// the next refresh; `IsFileLocked` and `NotUnpackingSpecification` are its
/// only transient-specific detectors. Scryer's importer stringifies IO errors
/// (`AppError::Repository(format!("… {io_error}"))`), so the OS `Display` text
/// — "`… (os error N)`" — is what is matchable here. The phrases below are the
/// glibc/BSD `strerror` and Windows `FormatMessage` (English) texts for the
/// conditions Sonarr retries; the numeric fallback covers localized Windows
/// messages.
fn completed_import_error_message_is_retryable(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    // Scryer's own transient signals and Sonarr's locked / still-unpacking detectors.
    const SCRYER_TRANSIENT_PHRASES: &[&str] = &[
        "active-download marker",
        "still being unpacked",
        "still_unpacking",
        "source changed",
        "locked",
        "temporarily",
        "not found or inaccessible",
    ];
    // OS-level transient IO conditions (io::Error Display text).
    const IO_TRANSIENT_PHRASES: &[&str] = &[
        // Windows FormatMessage
        "being used by another process",       // 32 ERROR_SHARING_VIOLATION
        "has locked a portion of the file",     // 33 ERROR_LOCK_VIOLATION
        "user-mapped section open",            // 1224 ERROR_USER_MAPPED_FILE (Plex/indexer mmap)
        "requested resource is in use",        // 170 ERROR_BUSY
        "network name is no longer available", // 64 ERROR_NETNAME_DELETED (SMB hiccup)
        "unexpected network error",            // 59 ERROR_UNEXP_NET_ERR
        "semaphore timeout period has expired", // 121 ERROR_SEM_TIMEOUT
        "network location cannot be reached",  // 1231/1232 network unreachable
        "not enough space on the disk",        // 112 ERROR_DISK_FULL
        "access is denied",                    // 5 ERROR_ACCESS_DENIED (AV/indexer holds; Sonarr retries)
        // glibc / BSD strerror
        "device or resource busy",             // EBUSY (Linux)
        "resource busy",                       // EBUSY (BSD/macOS)
        "text file busy",                      // ETXTBSY
        "interrupted system call",             // EINTR
        "input/output error",                  // EIO (NFS/USB hiccup)
        "stale file handle",                   // ESTALE (Linux)
        "stale nfs file handle",               // ESTALE (BSD/macOS)
        "transport endpoint is not connected", // ENOTCONN (Linux)
        "socket is not connected",             // ENOTCONN (BSD/macOS)
        "connection timed out",                // ETIMEDOUT (Linux)
        "operation timed out",                 // ETIMEDOUT (BSD/macOS)
        "host is down",                        // EHOSTDOWN
        "no route to host",                    // EHOSTUNREACH
        "software caused connection abort",    // ECONNABORTED
        "no space left on device",             // ENOSPC
        "disk quota exceeded",                 // EDQUOT
        "too many open files",                 // EMFILE
        "permission denied",                   // EACCES (Sonarr retries UnauthorizedAccess)
        "no such file or directory",           // ENOENT mid-transfer (unmounted root / unpacker rename)
    ];
    if SCRYER_TRANSIENT_PHRASES
        .iter()
        .chain(IO_TRANSIENT_PHRASES)
        .any(|needle| normalized.contains(needle))
    {
        return true;
    }
    // Numeric fallback for localized Windows messages, keyed on the exact
    // "(os error N)" token Rust appends to raw OS errors.
    const WINDOWS_TRANSIENT_OS_ERRORS: &[u32] = &[5, 32, 33, 59, 64, 112, 121, 170, 1224, 1231, 1232];
    cfg!(windows)
        && raw_os_error_code(&normalized)
            .is_some_and(|code| WINDOWS_TRANSIENT_OS_ERRORS.contains(&code))
}

/// Extracts `N` from the first "(os error N)" token in an IO error message.
fn raw_os_error_code(normalized_message: &str) -> Option<u32> {
    let rest = normalized_message.split("(os error ").nth(1)?;
    let digits = rest.chars().take_while(char::is_ascii_digit).collect::<String>();
    (!digits.is_empty() && rest[digits.len()..].starts_with(')'))
        .then(|| digits.parse().ok())
        .flatten()
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
