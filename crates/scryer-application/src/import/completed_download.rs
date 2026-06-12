//! CompletedDownloadHandler — two-phase import bridge (plan 055).
//!
//! Phase 1 (check): validate completed downloads, resolve title, gate auto-import.
//! Phase 2 (import): run the import pipeline, verify completion across passes.

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    CompletedDownload, DownloadQueueItem, DownloadQueueState, ImportDecision,
    ImportRejectedEventData, ImportResult, ImportSkipReason, ImportStatus, TitleMatchType,
    TrackedDownloadState, TrackedDownloadStatus,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::domain_events::{
    new_global_domain_event, new_title_domain_event, title_context_snapshot,
};
use crate::import_workflow::import_completed_download;
use crate::tracked_downloads::TrackedDownload;
use crate::{AppResult, AppUseCase, DownloadSourceIdentity, User};
use crate::{
    apply_remote_path_mappings_to_completed_download, parse_download_client_remote_path_mappings,
};

const PATH_WAITING_MESSAGE: &str =
    "Completed download path is not available yet. Retrying for up to 10 minutes.";
const PATH_BLOCKED_MESSAGE: &str = "Completed download path is still unavailable. Check remote path mappings, volume mounts, or download paths, then retry manually.";
const PATH_BLOCKED_NZBDAV_SYMLINK_MESSAGE: &str = "Completed download path is still unavailable. Check remote path mappings, confirm the NZBDAV completed-symlinks mount is visible to Scryer, and make sure the rclone mount was started with --links before retrying manually.";
const PATH_URL_UNSUPPORTED_MESSAGE: &str = "Completed download path is a URL, not a local filesystem path. Mount it locally or use remote path mappings before retrying.";
const ID_ONLY_CONFLICT_MESSAGE: &str = "Download name conflicts with the current ID-only title match. Manual confirmation required before import.";
const FOREIGN_CATEGORY_BLOCKED_MESSAGE: &str = "Download wasn't grabbed by Scryer and is not in a Scryer download category. Manual confirmation required before import.";
const IMPORT_RUNNING_MESSAGE: &str = "Moving files to library.";
const COMPLETED_PATH_GRACE_PERIOD_MINUTES: i64 = 10;

#[derive(Default)]
pub(crate) struct CompletedDownloadLookup {
    by_source: HashMap<(String, String, String), CompletedDownload>,
    by_download_id: HashMap<(String, String, String), Vec<CompletedDownload>>,
}

enum ExpectedEpisodeResolution {
    NotApplicable,
    Unresolved,
    Resolved(HashSet<String>),
    AtLeastOne(HashSet<String>),
}

/// Phase 1: evaluate a tracked download whose client reports completion.
///
/// Called every poll cycle for downloads in Downloading or ImportBlocked state.
/// Transitions to ImportPending if all validations pass, or ImportBlocked with
/// warnings if auto-import is not safe.
pub async fn check(app: &AppUseCase, td: &mut TrackedDownload) {
    check_with_lookup(app, td, None).await;
}

pub(crate) async fn check_with_lookup(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
) {
    // Only process if client reports completed.
    if td.client_item.state != DownloadQueueState::Completed {
        return;
    }

    // Only process if still in a check-eligible state.
    if td.state != TrackedDownloadState::Downloading
        && td.state != TrackedDownloadState::ImportBlocked
    {
        return;
    }

    // Don't re-evaluate a post-import block. Import already ran and returned
    // Skipped/Failed — stay blocked until the user explicitly retries.
    if td.state == TrackedDownloadState::ImportBlocked && td.import_attempted {
        return;
    }

    // A blocked download that was explicitly assigned by the user should remain
    // in the manual-import flow until the user queues that import. Assigning a
    // title should not silently convert it back into auto-import.
    if td.state == TrackedDownloadState::ImportBlocked
        && td.match_type == TitleMatchType::Submission
    {
        return;
    }

    if td.state == TrackedDownloadState::ImportBlocked && td.path_missing_since.is_some() {
        return;
    }

    let queue_identity = observed_queue_item_identity(&td.client_item);
    let queue_source_identity = queue_item_source_identity(&td.client_item);
    if let Some(state) =
        download_id_tracked_state(app, &queue_identity, Some(&queue_source_identity)).await
        && state.is_terminal()
    {
        apply_download_id_state(td, state);
        return;
    }

    let Some(completed) = find_completed_download(app, td, completed_lookup).await else {
        if !crate::download_submission_identity_is_empty(&queue_identity) {
            block_tracked_download_identity_for_manual_review(
                app,
                td,
                "missing_completed_history_identity",
                "completed queue item carried DownloadId but completed history did not contain a matching DownloadId",
            )
            .await;
        }
        return;
    };

    let completed_identity = observed_completed_download_identity(&completed);
    let completed_source_identity = completed_download_source_identity(&completed);
    if let Some(state) =
        download_id_tracked_state(app, &completed_identity, Some(&completed_source_identity)).await
        && state.is_terminal()
    {
        apply_download_id_state(td, state);
        return;
    }

    maybe_resolve_title_from_completed_download(app, td, &completed).await;

    match evaluate_completed_download_path(td, &completed, Utc::now()) {
        CompletedDownloadPathState::Ready => {}
        CompletedDownloadPathState::Retry => {
            return;
        }
        CompletedDownloadPathState::Blocked => {
            tracing::warn!(
                id = %td.id,
                dest_dir = %completed.dest_dir,
                empty_dest_dir = completed.dest_dir.trim().is_empty(),
                grace_minutes = COMPLETED_PATH_GRACE_PERIOD_MINUTES,
                "completed download path remained unavailable after grace window; blocking import until manual retry"
            );
            set_state_to_import_blocked(app, td).await;
            return;
        }
    }

    // Auto-import safety gating.
    match td.match_type {
        TitleMatchType::Unmatched => {
            if !td
                .status_messages
                .iter()
                .any(|m| m.contains("couldn't be matched"))
            {
                td.status_messages.clear();
                td.warn("Download couldn't be matched to a library title. Assign a title manually or check the download name.");
            }
            set_state_to_import_blocked(app, td).await;
            return;
        }
        TitleMatchType::IdOnly => {
            // Match Sonarr/Radarr's conservative handling for risky ID-only
            // matches: interactive/Scryer-origin grabs may continue, but
            // foreign downloads that only resolved through embedded IDs still
            // need manual confirmation before import.
            if !td.client_item.is_scryer_origin || has_id_only_conflict(td) {
                if !td.status_messages.iter().any(|m| {
                    m.contains("matched by ID only") || m.contains(ID_ONLY_CONFLICT_MESSAGE)
                }) {
                    td.status_messages.clear();
                    td.warn(
                        "Download was matched to a title by ID only. Manual confirmation required to import.",
                    );
                }
                set_state_to_import_blocked(app, td).await;
                return;
            }
        }
        TitleMatchType::Submission
        | TitleMatchType::ClientParameter
        | TitleMatchType::TitleParse => {
            // High-confidence matches — proceed.
        }
    }

    // Check that the resolved title still exists.
    // (This is a sync check against cached data; the actual title lookup
    //  was done during resolve_title. If the title was deleted since then,
    //  title_id will still be set but import will fail gracefully.)

    if td.title_id.is_none() || td.title_id.as_deref() == Some("") {
        td.warn("No title linked to this download.");
        set_state_to_import_blocked(app, td).await;
        return;
    }

    if !completed_download_allows_automatic_import(app, td, &completed).await {
        td.status_messages.clear();
        td.warn(FOREIGN_CATEGORY_BLOCKED_MESSAGE);
        set_state_to_import_blocked(app, td).await;
        return;
    }

    // All checks passed — queue for import.
    tracing::info!(
        id = %td.id,
        title_id = ?td.title_id,
        match_type = ?td.match_type,
        "check: transitioning to ImportPending"
    );
    td.state = TrackedDownloadState::ImportPending;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
}

async fn completed_download_allows_automatic_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed: &CompletedDownload,
) -> bool {
    if matches!(
        td.match_type,
        TitleMatchType::Submission | TitleMatchType::ClientParameter
    ) || td.client_item.is_scryer_origin
    {
        return true;
    }

    let Some(observed_category) = normalized_download_category(
        completed
            .category
            .as_deref()
            .or(td.client_item.category.as_deref()),
    ) else {
        return false;
    };

    let Some(title_id) = td
        .title_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    let title = match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => title,
        Ok(None) => return false,
        Err(error) => {
            tracing::warn!(
                title_id,
                error = %error,
                "completed download category gate could not load title"
            );
            return false;
        }
    };

    match app
        .effective_download_client_category_for_title(&title, &td.client_id)
        .await
    {
        Ok(Some(expected_category)) => observed_category == expected_category.trim(),
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                title_id,
                client_id = td.client_id.as_str(),
                error = %error,
                "completed download category gate could not resolve effective category"
            );
            false
        }
    }
}

fn normalized_download_category(category: Option<&str>) -> Option<&str> {
    category.map(str::trim).filter(|value| !value.is_empty())
}

/// Phase 2: run the actual import for a download in ImportPending state.
///
/// This is async because it calls the import pipeline. Returns true if the
/// download reached a terminal tracked state and can be persisted/removed.
pub(crate) fn mark_importing(td: &mut TrackedDownload) {
    td.state = TrackedDownloadState::Importing;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages = vec![IMPORT_RUNNING_MESSAGE.to_string()];
}

pub async fn import(app: &AppUseCase, actor: &User, td: &mut TrackedDownload) -> bool {
    if td.state != TrackedDownloadState::ImportPending
        && td.state != TrackedDownloadState::Importing
    {
        return false;
    }

    match app
        .resolve_manual_import_source(
            Some(td.client_id.as_str()),
            Some(td.client_type.as_str()),
            &td.client_item.download_client_item_id,
        )
        .await
    {
        Ok(crate::ManualImportSourceResolution::Eligible { .. }) => {}
        Ok(crate::ManualImportSourceResolution::SourceFailed { message })
        | Ok(crate::ManualImportSourceResolution::NotEligible { message }) => {
            tracing::warn!(
                id = %td.id,
                item_id = %td.client_item.download_client_item_id,
                reason = %message,
                "import: source is no longer eligible; routing to failure handling"
            );
            td.state = TrackedDownloadState::FailedPending;
            td.status = TrackedDownloadStatus::Error;
            td.client_item.attention_reason = Some(message.clone());
            td.status_messages = vec![message];
            return false;
        }
        Err(error) => {
            tracing::warn!(
                id = %td.id,
                error = %error,
                "import: could not revalidate source before import"
            );
            td.state = TrackedDownloadState::ImportPending;
            return false;
        }
    }

    mark_importing(td);
    crate::tracked_downloads::publish_runtime_tracked_download_snapshot(app, td).await;

    let Some(completed) = find_completed_download(app, td, None).await else {
        tracing::debug!(
            id = %td.id,
            item_id = %td.client_item.download_client_item_id,
            "import: completed download not found in client history, will retry"
        );
        td.state = TrackedDownloadState::ImportPending;
        return false;
    };

    tracing::info!(
        id = %td.id,
        dest_dir = %completed.dest_dir,
        title_id = ?td.title_id,
        "import: starting import from completed download"
    );

    let success_before = total_successful_artifacts(app, td).await;
    td.import_attempted = true;

    match import_completed_download(app, actor, &completed).await {
        Ok(result) => {
            let success_after = total_successful_artifacts(app, td).await;
            let files_imported_this_pass = success_after.saturating_sub(success_before) as usize;
            tracing::info!(
                id = %td.id,
                decision = ?result.decision,
                skip_reason = ?result.skip_reason,
                error_message = ?result.error_message,
                files_imported_this_pass,
                "import: pipeline returned result"
            );
            apply_import_result(app, td, result, files_imported_this_pass).await
        }
        Err(error) => {
            tracing::warn!(
                id = %td.id,
                error = %error,
                dest_dir = %completed.dest_dir,
                "import: pipeline returned error"
            );
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Error;
            td.status_messages = vec![format!("Import failed: {error}")];
            false
        }
    }
}

/// Verify whether a download's import is complete by checking cumulative
/// artifact history across all passes.
///
/// Returns true if all expected files are accounted for (imported or already_present).
pub async fn verify_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
) -> bool {
    let source_identity = DownloadSourceIdentity::new(
        Some(td.client_id.as_str()),
        &td.client_type,
        &td.client_item.download_client_item_id,
    );

    let artifacts = match app
        .services
        .workflow
        .import_artifacts
        .list_by_source_identity(&source_identity)
        .await
    {
        Ok(artifacts) => artifacts,
        Err(_) => return false,
    };

    if artifacts.is_empty() {
        return false;
    }

    let current_visible_files = current_visible_video_file_count(app, td).await;
    let mut successful_units = HashSet::new();
    let mut rejected_units = HashSet::new();

    for artifact in artifacts {
        let logical_unit = artifact.episode_id.clone().unwrap_or_else(|| {
            format!("{}:{}", artifact.media_kind, artifact.normalized_file_name)
        });

        match artifact.result.as_str() {
            "imported" | "already_present" => {
                successful_units.insert(logical_unit);
            }
            "rejected" => {
                rejected_units.insert(logical_unit);
            }
            _ => {}
        }
    }

    if successful_units.is_empty() {
        return false;
    }

    if td.facet.as_deref() == Some("movie") {
        return !successful_units.is_empty();
    }

    match expected_episode_units(app, td).await {
        ExpectedEpisodeResolution::Resolved(expected_episode_units) => {
            if expected_episode_units.is_empty() {
                return false;
            }

            return expected_episode_units
                .iter()
                .all(|unit| successful_units.contains(unit));
        }
        ExpectedEpisodeResolution::AtLeastOne(expected_episode_units) => {
            if expected_episode_units.is_empty() {
                return false;
            }

            return expected_episode_units
                .iter()
                .any(|unit| successful_units.contains(unit));
        }
        ExpectedEpisodeResolution::Unresolved => {
            if successful_units_cover_visible_files(successful_units.len(), current_visible_files) {
                return true;
            }

            return files_imported_this_pass > 0 && rejected_units.is_empty();
        }
        ExpectedEpisodeResolution::NotApplicable => {}
    }

    if successful_units_cover_visible_files(successful_units.len(), current_visible_files) {
        return true;
    }

    !successful_units.is_empty()
}

fn successful_units_cover_visible_files(
    successful_unit_count: usize,
    current_visible_files: usize,
) -> bool {
    current_visible_files > 0 && successful_unit_count >= current_visible_files
}

pub(crate) async fn load_completed_download_lookup(
    app: &AppUseCase,
) -> AppResult<CompletedDownloadLookup> {
    let completed_downloads = app
        .services
        .integrations
        .download_client
        .list_completed_downloads()
        .await?;
    Ok(index_completed_downloads(completed_downloads))
}

pub(crate) async fn load_recent_completed_download_lookup(
    app: &AppUseCase,
    limit: usize,
) -> AppResult<CompletedDownloadLookup> {
    let completed_downloads = app
        .services
        .integrations
        .download_client
        .list_recent_completed_downloads(limit)
        .await?;
    Ok(index_completed_downloads(completed_downloads))
}

pub(crate) async fn load_completed_download_lookup_for_items(
    app: &AppUseCase,
    items: &[DownloadQueueItem],
    limit: usize,
) -> Option<CompletedDownloadLookup> {
    if !items
        .iter()
        .any(|item| item.state == DownloadQueueState::Completed)
    {
        return None;
    }

    Some(
        match load_recent_completed_download_lookup(app, limit).await {
            Ok(lookup) => lookup,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "download queue poller: failed to load completed download snapshot for this cycle"
                );
                CompletedDownloadLookup::default()
            }
        },
    )
}

fn index_completed_downloads(downloads: Vec<CompletedDownload>) -> CompletedDownloadLookup {
    let mut lookup = CompletedDownloadLookup::default();
    for completed in downloads {
        let observed_identity = observed_completed_download_identity(&completed);
        if let Some(download_id) = observed_identity
            .download_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            lookup
                .by_download_id
                .entry(completed_download_lookup_key(
                    Some(&completed.client_id),
                    &completed.client_type,
                    download_id,
                ))
                .or_default()
                .push(completed.clone());
        }
        lookup.by_source.insert(
            completed_download_lookup_key(
                Some(&completed.client_id),
                &completed.client_type,
                &completed.download_client_item_id,
            ),
            completed,
        );
    }
    lookup
}

fn observed_queue_item_identity(item: &DownloadQueueItem) -> crate::DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_id: item.download_id.as_deref(),
        parameters: &[],
        info_hash_hint: None,
    })
}

fn queue_item_source_identity(item: &DownloadQueueItem) -> DownloadSourceIdentity {
    DownloadSourceIdentity::new(
        Some(item.client_id.as_str()),
        item.client_type.as_str(),
        item.download_client_item_id.as_str(),
    )
}

fn observed_completed_download_identity(
    completed: &CompletedDownload,
) -> crate::DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_id: completed.download_id.as_deref(),
        parameters: &completed.parameters,
        info_hash_hint: None,
    })
}

fn completed_download_source_identity(completed: &CompletedDownload) -> DownloadSourceIdentity {
    DownloadSourceIdentity::new(
        Some(completed.client_id.as_str()),
        completed.client_type.as_str(),
        completed.download_client_item_id.as_str(),
    )
}

async fn download_id_tracked_state(
    app: &AppUseCase,
    identity: &crate::DownloadSubmissionIdentity,
    source_identity: Option<&DownloadSourceIdentity>,
) -> Option<TrackedDownloadState> {
    if crate::download_submission_identity_is_empty(identity) {
        return None;
    }
    app.services
        .workflow
        .download_submissions
        .get_identity_tracked_state(identity, source_identity)
        .await
        .ok()
        .flatten()
        .and_then(|state| TrackedDownloadState::from_str_opt(&state))
}

fn apply_download_id_state(td: &mut TrackedDownload, state: TrackedDownloadState) {
    td.state = state;
    match state {
        TrackedDownloadState::Imported => {
            td.status = TrackedDownloadStatus::Ok;
            td.status_messages.clear();
        }
        TrackedDownloadState::Failed => {
            td.status = TrackedDownloadStatus::Error;
        }
        _ => {}
    }
}

fn single_completed_download_identity_match(
    matches: Option<&Vec<CompletedDownload>>,
    label: &str,
    value: &str,
) -> Option<CompletedDownload> {
    let matches = matches?;
    if matches.len() == 1 {
        return matches.first().cloned();
    }
    if let Some(completed) =
        crate::download_identity::coalesce_completed_downloads_by_release_observation(matches)
    {
        return Some(completed);
    }
    tracing::warn!(
        identity_kind = label,
        identity_value = value,
        matches = matches.len(),
        "find_completed_download: DownloadId matched multiple completed downloads"
    );
    None
}

fn completed_download_lookup_key(
    client_id: Option<&str>,
    client_type: &str,
    item_id: &str,
) -> (String, String, String) {
    (
        client_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("")
            .to_string(),
        client_type.to_string(),
        item_id.to_string(),
    )
}

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
                "find_completed_download: failed to load download client config for remote path mapping"
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
                "find_completed_download: failed to parse remote path mappings"
            );
        }
    }
}

async fn find_completed_download(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
) -> Option<CompletedDownload> {
    let lookup = match completed_lookup {
        Some(_) => None,
        None => match load_completed_download_lookup(app).await {
            Ok(lookup) => Some(lookup),
            Err(error) => {
                tracing::warn!(error = %error, "find_completed_download: failed to fetch from client");
                return None;
            }
        },
    };
    let completed = match completed_lookup {
        Some(lookup) => find_completed_download_in_lookup(lookup, td),
        None => lookup
            .as_ref()
            .and_then(|indexed| find_completed_download_in_lookup(indexed, td)),
    };
    match completed {
        Some(completed) => {
            let mut completed = with_tracked_metadata(td, completed);
            remap_completed_download_for_client(app, &mut completed).await;
            Some(completed)
        }
        None => {
            tracing::debug!(
                id = %td.id,
                item_id = %td.client_item.download_client_item_id,
                client_type = %td.client_type,
                "find_completed_download: no matching item in client history"
            );
            None
        }
    }
}

fn find_completed_download_in_lookup(
    lookup: &CompletedDownloadLookup,
    td: &TrackedDownload,
) -> Option<CompletedDownload> {
    let observed_identity = observed_queue_item_identity(&td.client_item);
    if let Some(download_id) = observed_identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return single_completed_download_identity_match(
            lookup.by_download_id.get(&completed_download_lookup_key(
                Some(&td.client_id),
                &td.client_type,
                download_id,
            )),
            "download_id",
            download_id,
        );
    }

    let key = completed_download_lookup_key(
        Some(&td.client_id),
        &td.client_type,
        &td.client_item.download_client_item_id,
    );
    if let Some(completed) = lookup.by_source.get(&key) {
        return Some(completed.clone());
    }

    None
}

async fn maybe_resolve_title_from_completed_download(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    completed: &CompletedDownload,
) {
    if !matches!(
        td.match_type,
        TitleMatchType::Unmatched | TitleMatchType::IdOnly
    ) {
        return;
    }

    clear_id_only_conflict(td);

    let Ok(matcher) = app.monitored_title_matcher().await else {
        return;
    };

    let folder_name = Path::new(&completed.dest_dir)
        .file_name()
        .and_then(|value| value.to_str());
    let release_candidates = [Some(completed.name.as_str()), folder_name];

    for release_title in release_candidates.into_iter().flatten() {
        let release_title = release_title.trim();
        if release_title.is_empty() {
            continue;
        }

        let parsed = crate::parse_release_metadata(release_title);
        let resolved = if parsed.episode.is_some() {
            matcher.resolve_episode(
                &parsed,
                td.client_item
                    .facet
                    .as_deref()
                    .or(td.facet.as_deref())
                    .or(completed.category.as_deref()),
            )
        } else {
            matcher.resolve_movie(&parsed)
        };

        if let Some(resolved) = resolved {
            if td.match_type == TitleMatchType::IdOnly
                && let Some(existing_title_id) = td.title_id.as_deref()
                && existing_title_id != resolved.title.id
            {
                td.status = TrackedDownloadStatus::Warning;
                td.status_messages.retain(|message| {
                    !message.contains("matched by ID only")
                        && !message.contains(ID_ONLY_CONFLICT_MESSAGE)
                });
                td.warn(ID_ONLY_CONFLICT_MESSAGE);
                return;
            }

            td.title_id = Some(resolved.title.id.clone());
            td.facet = Some(resolved.title.facet.as_str().to_string());
            td.source_title = Some(release_title.to_string());
            if td.match_type != TitleMatchType::IdOnly {
                td.match_type = resolved.match_type;
            }
            return;
        }
    }
}

fn with_tracked_metadata(
    td: &TrackedDownload,
    mut completed: CompletedDownload,
) -> CompletedDownload {
    upsert_parameter(
        &mut completed.parameters,
        "*scryer_title_id",
        td.title_id.clone().unwrap_or_default(),
    );
    upsert_parameter(
        &mut completed.parameters,
        "*scryer_facet",
        td.facet.clone().unwrap_or_default(),
    );
    completed
}

fn upsert_parameter(params: &mut Vec<(String, String)>, key: &str, value: String) {
    if value.trim().is_empty() {
        return;
    }

    if let Some((_, existing)) = params
        .iter_mut()
        .find(|(existing_key, _)| existing_key == key)
    {
        *existing = value;
    } else {
        params.push((key.to_string(), value));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletedDownloadPathState {
    Ready,
    Retry,
    Blocked,
}

fn evaluate_completed_download_path(
    td: &mut TrackedDownload,
    completed: &CompletedDownload,
    now: DateTime<Utc>,
) -> CompletedDownloadPathState {
    if completed_download_path_is_unsupported_url(&completed.dest_dir) {
        td.path_missing_since = None;
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages = vec![PATH_URL_UNSUPPORTED_MESSAGE.to_string()];
        return CompletedDownloadPathState::Blocked;
    }

    let path_available =
        !completed.dest_dir.trim().is_empty() && Path::new(&completed.dest_dir).exists();

    if path_available {
        td.path_missing_since = None;
        clear_path_warnings(td);
        return CompletedDownloadPathState::Ready;
    }

    let missing_since = td.path_missing_since.get_or_insert(now);
    let elapsed = now.signed_duration_since(*missing_since);
    if elapsed < Duration::minutes(COMPLETED_PATH_GRACE_PERIOD_MINUTES) {
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages = vec![PATH_WAITING_MESSAGE.to_string()];
        return CompletedDownloadPathState::Retry;
    }

    td.status = TrackedDownloadStatus::Warning;
    td.status_messages = vec![missing_completed_download_path_message(completed)];
    CompletedDownloadPathState::Blocked
}

fn missing_completed_download_path_message(completed: &CompletedDownload) -> String {
    if completed.dest_dir.contains("/completed-symlinks/") {
        PATH_BLOCKED_NZBDAV_SYMLINK_MESSAGE.to_string()
    } else {
        PATH_BLOCKED_MESSAGE.to_string()
    }
}

fn completed_download_path_is_unsupported_url(dest_dir: &str) -> bool {
    let normalized = dest_dir.trim().to_ascii_lowercase();
    normalized.contains("://") || normalized.starts_with("webdav:")
}

fn clear_path_warnings(td: &mut TrackedDownload) {
    td.status_messages.retain(|message| {
        message != PATH_WAITING_MESSAGE
            && message != PATH_BLOCKED_MESSAGE
            && message != PATH_BLOCKED_NZBDAV_SYMLINK_MESSAGE
            && message != PATH_URL_UNSUPPORTED_MESSAGE
    });
    if td.status_messages.is_empty() && td.status == TrackedDownloadStatus::Warning {
        td.status = TrackedDownloadStatus::Ok;
    }
}

fn clear_id_only_conflict(td: &mut TrackedDownload) {
    td.status_messages
        .retain(|message| message != ID_ONLY_CONFLICT_MESSAGE);
    if td.status_messages.is_empty() && td.status == TrackedDownloadStatus::Warning {
        td.status = TrackedDownloadStatus::Ok;
    }
}

fn has_id_only_conflict(td: &TrackedDownload) -> bool {
    td.status_messages
        .iter()
        .any(|message| message == ID_ONLY_CONFLICT_MESSAGE)
}

async fn apply_import_result(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    result: ImportResult,
    files_imported_this_pass: usize,
) -> bool {
    let already_imported = result.skip_reason == Some(ImportSkipReason::AlreadyImported);
    if result.decision == ImportDecision::Imported || already_imported {
        if verify_import(app, td, files_imported_this_pass).await {
            td.state = TrackedDownloadState::Imported;
            td.status = TrackedDownloadStatus::Ok;
            td.status_messages.clear();
            return true;
        }

        if already_imported {
            td.state = TrackedDownloadState::Imported;
            td.status = TrackedDownloadStatus::Ok;
            td.status_messages.clear();
            return true;
        }

        td.state = TrackedDownloadState::ImportPending;
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages = vec![
            "Import partially completed; waiting for remaining files or verification.".to_string(),
        ];
        return false;
    }

    if import_result_is_retryable(&result) {
        let result_json = serde_json::to_string(&result).ok();
        if let Err(err) = app
            .update_import_status_and_notify(&result.import_id, ImportStatus::Pending, result_json)
            .await
        {
            tracing::warn!(
                import_id = result.import_id.as_str(),
                error = %err,
                "failed to restore retryable import attempt to pending status"
            );
        }
        td.state = TrackedDownloadState::ImportPending;
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages = vec![retryable_import_result_message(&result)];
        return false;
    }

    match result.decision {
        ImportDecision::Failed => {
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Error;
            td.status_messages = vec![import_result_message(&result, ImportStatus::Failed)];
            false
        }
        _ => {
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Warning;
            td.status_messages = vec![import_result_message(&result, ImportStatus::Skipped)];
            false
        }
    }
}

fn import_result_is_retryable(result: &ImportResult) -> bool {
    if matches!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles))
        && Path::new(&result.source_path).exists()
    {
        return true;
    }

    result
        .error_message
        .as_deref()
        .is_some_and(error_message_is_retryable)
}

fn error_message_is_retryable(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "active-download marker",
        "still being unpacked",
        "still_unpacking",
        "source changed",
        "locked",
        "temporarily",
        "not found or inaccessible",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn retryable_import_result_message(result: &ImportResult) -> String {
    let detail = import_result_message(result, ImportStatus::Skipped);
    format!("{detail} Retrying automatically.")
}

fn import_result_message(result: &ImportResult, fallback_status: ImportStatus) -> String {
    if let Some(message) = result
        .error_message
        .as_ref()
        .filter(|message| !message.trim().is_empty())
    {
        return message.clone();
    }

    if let Some(skip_reason) = result.skip_reason.as_ref() {
        return format!("Import blocked: {}", skip_reason.as_str());
    }

    format!("Import ended with status {}", fallback_status.as_str())
}

async fn expected_episode_units(
    app: &AppUseCase,
    td: &TrackedDownload,
) -> ExpectedEpisodeResolution {
    let Some(title_id) = td.title_id.as_deref() else {
        return ExpectedEpisodeResolution::Unresolved;
    };
    let Some(title) = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await
        .ok()
        .flatten()
    else {
        return ExpectedEpisodeResolution::Unresolved;
    };

    let release_title = td
        .source_title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(td.client_item.title_name.as_str());
    let parse_context = crate::build_release_parse_context(&title, None, None, td.facet.as_deref());
    let parsed = crate::parse_release_metadata_for_target(release_title, &parse_context);
    let Some(ep_meta) = parsed.episode.as_ref() else {
        return ExpectedEpisodeResolution::NotApplicable;
    };
    let season_str = ep_meta.season.unwrap_or(1).to_string();
    let episodes =
        crate::import_workflow::resolve_target_episodes(app, &title, ep_meta, &season_str).await;

    if episodes.is_empty() {
        return ExpectedEpisodeResolution::Unresolved;
    }

    let expected_lookup_count = if ep_meta.season.is_some() && !ep_meta.episode_numbers.is_empty() {
        ep_meta
            .episode_numbers
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
    } else if !ep_meta.absolute_episode_numbers.is_empty() {
        ep_meta
            .absolute_episode_numbers
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
    } else if ep_meta.absolute_episode.is_some() {
        if ep_meta.episode_numbers.is_empty() {
            1
        } else {
            ep_meta
                .episode_numbers
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len()
        }
    } else {
        0
    };

    if expected_lookup_count > 0 && episodes.len() < expected_lookup_count {
        return ExpectedEpisodeResolution::Unresolved;
    }

    let all_expected_episode_ids = episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect::<HashSet<_>>();
    let monitored_expected_episode_ids = episodes
        .into_iter()
        .filter(|episode| episode.monitored)
        .map(|episode| episode.id)
        .collect::<HashSet<_>>();
    let expected_episode_ids = if monitored_expected_episode_ids.is_empty() {
        all_expected_episode_ids
    } else {
        monitored_expected_episode_ids
    };

    if ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        && ep_meta.is_partial_season
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.absolute_episode_numbers.is_empty()
        && ep_meta.special_absolute_episode_numbers.is_empty()
    {
        return ExpectedEpisodeResolution::AtLeastOne(expected_episode_ids);
    }

    ExpectedEpisodeResolution::Resolved(expected_episode_ids)
}

async fn set_state_to_import_blocked(app: &AppUseCase, td: &mut TrackedDownload) {
    td.state = TrackedDownloadState::ImportBlocked;
    td.status = TrackedDownloadStatus::Warning;

    if td.notified_manual_interaction {
        return;
    }

    td.notified_manual_interaction = true;
    let message = td
        .status_messages
        .first()
        .cloned()
        .unwrap_or_else(|| "Manual interaction required for this download.".to_string());

    let event = match td.title_id.as_ref() {
        Some(title_id) => match app.services.catalog.titles.get_by_id(title_id).await {
            Ok(Some(title)) => new_title_domain_event(
                None,
                &title,
                scryer_domain::DomainEventPayload::ImportRejected(ImportRejectedEventData {
                    title: Some(title_context_snapshot(&title)),
                    status: ImportStatus::Skipped,
                    import_id: None,
                    source_system: Some(td.client_type.clone()),
                    source_ref: Some(td.client_item.download_client_item_id.clone()),
                    source_title: td
                        .source_title
                        .clone()
                        .or_else(|| Some(td.client_item.title_name.clone())),
                    source_path: None,
                    dest_path: None,
                    quality: None,
                    reason: Some(message.clone()),
                    skip_reason: None,
                    episode_ids: Vec::new(),
                }),
            ),
            _ => new_global_domain_event(
                None,
                scryer_domain::DomainEventPayload::ImportRejected(ImportRejectedEventData {
                    title: None,
                    status: ImportStatus::Skipped,
                    import_id: None,
                    source_system: Some(td.client_type.clone()),
                    source_ref: Some(td.client_item.download_client_item_id.clone()),
                    source_title: td
                        .source_title
                        .clone()
                        .or_else(|| Some(td.client_item.title_name.clone())),
                    source_path: None,
                    dest_path: None,
                    quality: None,
                    reason: Some(message.clone()),
                    skip_reason: None,
                    episode_ids: Vec::new(),
                }),
            ),
        },
        None => new_global_domain_event(
            None,
            scryer_domain::DomainEventPayload::ImportRejected(ImportRejectedEventData {
                title: None,
                status: ImportStatus::Skipped,
                import_id: None,
                source_system: Some(td.client_type.clone()),
                source_ref: Some(td.client_item.download_client_item_id.clone()),
                source_title: td
                    .source_title
                    .clone()
                    .or_else(|| Some(td.client_item.title_name.clone())),
                source_path: None,
                dest_path: None,
                quality: None,
                reason: Some(message),
                skip_reason: None,
                episode_ids: Vec::new(),
            }),
        ),
    };

    let _ = app.append_domain_event(event).await;
}

async fn block_tracked_download_identity_for_manual_review(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    reason: &str,
    detail: &str,
) {
    let observed_identity = observed_queue_item_identity(&td.client_item);
    if crate::download_submission_identity_is_empty(&observed_identity) {
        return;
    }
    let source_identity = DownloadSourceIdentity::new(
        Some(td.client_id.as_str()),
        &td.client_type,
        &td.client_item.download_client_item_id,
    );
    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .record_identity_tracked_state(
            &observed_identity,
            Some(&source_identity),
            TrackedDownloadState::ImportBlocked.as_str(),
            Some(reason),
            Some(detail),
        )
        .await
    {
        tracing::warn!(
            error = %error,
            client_id = td.client_id.as_str(),
            client_type = td.client_type.as_str(),
            download_client_item_id = td.client_item.download_client_item_id.as_str(),
            reason,
            "failed to persist durable tracked-download manual-review state"
        );
    }
    if !td.status_messages.iter().any(|message| message == detail) {
        td.status_messages.clear();
        td.status_messages.push(detail.to_string());
    }
    set_state_to_import_blocked(app, td).await;
}

async fn total_successful_artifacts(app: &AppUseCase, td: &TrackedDownload) -> u64 {
    let source_identity = DownloadSourceIdentity::new(
        Some(td.client_id.as_str()),
        &td.client_type,
        &td.client_item.download_client_item_id,
    );
    let imported = app
        .services
        .workflow
        .import_artifacts
        .count_by_result_for_source_identity(&source_identity, "imported")
        .await
        .unwrap_or(0);
    let already_present = app
        .services
        .workflow
        .import_artifacts
        .count_by_result_for_source_identity(&source_identity, "already_present")
        .await
        .unwrap_or(0);
    imported + already_present
}

async fn current_visible_video_file_count(app: &AppUseCase, td: &TrackedDownload) -> usize {
    let Some(completed) = find_completed_download(app, td, None).await else {
        return 0;
    };

    let path = std::path::Path::new(&completed.dest_dir);
    let filter_samples = td.facet.as_deref() != Some("movie");
    crate::import_workflow::find_video_files(path, filter_samples)
        .map(|files| files.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullReleaseAttemptRepository, NullUserRepository,
    };
    use crate::{
        ActivityKind, AppError, AppResult, AppServices, AppUseCase, CollectionUpdate,
        CreateTitleOutcome, DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
        DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, DomainEventRepository, DownloadClient,
        DownloadClientAddRequest, DownloadClientConfigRepository, DownloadGrabResult,
        DownloadSourceIdentity, DownloadSubmission, DownloadSubmissionIdentity,
        DownloadSubmissionRepository, EpisodeUpdate, FacetRegistry, ImportArtifact,
        ImportArtifactRepository, IndexerConfigRepository, JwtAuthConfig, PendingTitleHydration,
        QualityProfile, QualityProfileRepository, SETTINGS_SCOPE_SYSTEM, ScopedExternalId,
        SettingsRepository, ShowRepository, SubmissionScope, TitleMetadataUpdate, TitleRepository,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_domain::{
        CalendarEpisode, Collection, CollectionType, DomainEvent, DomainEventFilter,
        DownloadClientConfig, DownloadQueueItem, DownloadQueueState, Episode, EpisodeType, Id,
        MediaFacet, NewDomainEvent, Title, TitleHistoryEventType, TitleMatchType,
        TrackedDownloadState, TrackedDownloadStatus, User,
    };
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestTitleRepo {
        titles: Arc<Mutex<Vec<Title>>>,
    }

    #[async_trait]
    impl TitleRepository for TestTitleRepo {
        async fn list(
            &self,
            facet: Option<MediaFacet>,
            query: Option<String>,
        ) -> AppResult<Vec<Title>> {
            let titles = self.titles.lock().await.clone();
            Ok(titles
                .into_iter()
                .filter(|title| {
                    facet
                        .as_ref()
                        .is_none_or(|expected| &title.facet == expected)
                })
                .filter(|title| {
                    query.as_ref().is_none_or(|value| {
                        title
                            .name
                            .to_ascii_lowercase()
                            .contains(&value.to_ascii_lowercase())
                    })
                })
                .collect())
        }

        async fn list_by_external_ids(
            &self,
            source: &str,
            values: &[String],
        ) -> AppResult<Vec<Title>> {
            let requested: Vec<&str> = values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .collect();
            let titles = self.titles.lock().await;
            let mut matches = Vec::new();
            let mut seen = HashSet::new();
            for value in requested {
                if let Some(title) = titles.iter().find(|title| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
                }) && seen.insert(title.id.clone())
                {
                    matches.push(title.clone());
                }
            }
            Ok(matches)
        }

        async fn list_for_matching(
            &self,
            facet: Option<MediaFacet>,
            query: Option<String>,
        ) -> AppResult<Vec<Title>> {
            self.list(facet, query).await
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
            let titles = self.titles.lock().await;
            Ok(titles.iter().find(|title| title.id == id).cloned())
        }

        async fn get_by_facet_and_slug(
            &self,
            facet: MediaFacet,
            slug: &str,
        ) -> AppResult<Option<Title>> {
            let normalized_slug = slug.trim();
            if normalized_slug.is_empty() {
                return Ok(None);
            }

            let titles = self.titles.lock().await;
            let matches = titles
                .iter()
                .filter(|title| {
                    title.facet == facet
                        && title.slug.as_deref().is_some_and(|candidate| {
                            candidate.trim().eq_ignore_ascii_case(normalized_slug)
                        })
                })
                .cloned()
                .collect::<Vec<_>>();

            match matches.as_slice() {
                [] => Ok(None),
                [title] => Ok(Some(title.clone())),
                _ => Err(AppError::Validation(
                    "multiple titles found for slug lookup".into(),
                )),
            }
        }

        async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
            let titles = self.titles.lock().await;
            Ok(titles
                .iter()
                .find(|title| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
                })
                .cloned())
        }

        async fn find_by_external_id_in_facet(
            &self,
            facet: MediaFacet,
            source: &str,
            value: &str,
        ) -> AppResult<Option<Title>> {
            let titles = self.titles.lock().await;
            Ok(titles
                .iter()
                .find(|title| {
                    title.facet == facet
                        && title.external_ids.iter().any(|external_id| {
                            external_id.source.eq_ignore_ascii_case(source)
                                && external_id.value == value
                        })
                })
                .cloned())
        }

        async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome> {
            Ok(CreateTitleOutcome {
                title: self.create(title).await?,
                reused_existing: false,
            })
        }

        async fn create(&self, title: Title) -> AppResult<Title> {
            self.titles.lock().await.push(title.clone());
            Ok(title)
        }

        async fn list_titles_due_for_hydration(
            &self,
            _: usize,
            _: &[MediaFacet],
        ) -> AppResult<Vec<PendingTitleHydration>> {
            Ok(vec![])
        }

        async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
            &self,
            _: usize,
        ) -> AppResult<Vec<String>> {
            Ok(vec![])
        }

        async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn schedule_title_metadata_hydration_retry(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update_metadata(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<MediaFacet>,
            _: Option<Vec<String>>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_title_hydrated_metadata(
            &self,
            _: &str,
            _: TitleMetadataUpdate,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn replace_match_state(
            &self,
            _: &str,
            _: Vec<scryer_domain::ExternalId>,
            _: Vec<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_folder_path(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct TestShowRepo {
        collections: Arc<Mutex<Vec<Collection>>>,
        episodes: Arc<Mutex<Vec<Episode>>>,
    }

    #[async_trait]
    impl ShowRepository for TestShowRepo {
        async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
            let collections = self.collections.lock().await;
            Ok(collections
                .iter()
                .filter(|collection| collection.title_id == title_id)
                .cloned()
                .collect())
        }

        async fn list_collection_external_ids(&self, _: &str) -> AppResult<Vec<ScopedExternalId>> {
            Ok(vec![])
        }

        async fn list_collections_for_titles(
            &self,
            title_ids: &[String],
        ) -> AppResult<std::collections::HashMap<String, Vec<Collection>>> {
            let collections = self.collections.lock().await;
            let wanted = title_ids.iter().cloned().collect::<HashSet<_>>();
            let mut grouped = std::collections::HashMap::<String, Vec<Collection>>::new();
            for collection in collections.iter() {
                if wanted.contains(&collection.title_id) {
                    grouped
                        .entry(collection.title_id.clone())
                        .or_default()
                        .push(collection.clone());
                }
            }
            Ok(grouped)
        }

        async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
            let collections = self.collections.lock().await;
            Ok(collections
                .iter()
                .find(|collection| collection.id == collection_id)
                .cloned())
        }

        async fn get_collection_by_ordered_path(
            &self,
            ordered_path: &str,
        ) -> AppResult<Option<Collection>> {
            let collections = self.collections.lock().await;
            Ok(collections
                .iter()
                .find(|collection| collection.ordered_path.as_deref() == Some(ordered_path))
                .cloned())
        }

        async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
            self.collections.lock().await.push(collection.clone());
            Ok(collection)
        }

        async fn update_collection(&self, _: &str, _: CollectionUpdate) -> AppResult<Collection> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_collection_interstitial_movie(
            &self,
            _: &str,
            _: scryer_domain::InterstitialMovieMetadata,
        ) -> AppResult<Collection> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_collection_specials_movies(
            &self,
            _: &str,
            _: Vec<scryer_domain::InterstitialMovieMetadata>,
        ) -> AppResult<Collection> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_interstitial_season_episode(
            &self,
            _: &str,
            _: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn set_collection_episodes_monitored(&self, _: &str, _: bool) -> AppResult<()> {
            Ok(())
        }

        async fn set_collections_monitored(
            &self,
            collection_ids: &[String],
            monitored: bool,
        ) -> AppResult<()> {
            let wanted = collection_ids.iter().cloned().collect::<HashSet<_>>();
            let mut collections = self.collections.lock().await;
            for collection in collections.iter_mut() {
                if wanted.contains(&collection.id) {
                    collection.monitored = monitored;
                }
            }
            Ok(())
        }

        async fn delete_collection(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn delete_collections_for_title(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn list_episodes_for_collection(
            &self,
            collection_id: &str,
        ) -> AppResult<Vec<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
                .cloned()
                .collect())
        }

        async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .filter(|episode| episode.title_id == title_id)
                .cloned()
                .collect())
        }

        async fn list_episode_external_ids(&self, _: &str) -> AppResult<Vec<ScopedExternalId>> {
            Ok(vec![])
        }

        async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .find(|episode| episode.id == episode_id)
                .cloned())
        }

        async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
            self.episodes.lock().await.push(episode.clone());
            Ok(episode)
        }

        async fn update_episode(&self, _: &str, _: EpisodeUpdate) -> AppResult<Episode> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn set_episodes_monitored(
            &self,
            episode_ids: &[String],
            monitored: bool,
        ) -> AppResult<()> {
            let wanted = episode_ids.iter().cloned().collect::<HashSet<_>>();
            let mut episodes = self.episodes.lock().await;
            for episode in episodes.iter_mut() {
                if wanted.contains(&episode.id) {
                    episode.monitored = monitored;
                }
            }
            Ok(())
        }

        async fn delete_episode(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn delete_episodes_for_title(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn find_episode_by_title_and_numbers(
            &self,
            title_id: &str,
            season_number: &str,
            episode_number: &str,
        ) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .find(|episode| {
                    episode.title_id == title_id
                        && episode.season_number.as_deref() == Some(season_number)
                        && episode.episode_number.as_deref() == Some(episode_number)
                })
                .cloned())
        }

        async fn find_episode_by_title_and_absolute_number(
            &self,
            title_id: &str,
            absolute_number: &str,
        ) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .find(|episode| {
                    episode.title_id == title_id
                        && episode.absolute_number.as_deref() == Some(absolute_number)
                })
                .cloned())
        }

        async fn list_primary_collection_summaries(
            &self,
            _: &[String],
        ) -> AppResult<Vec<crate::PrimaryCollectionSummary>> {
            Ok(vec![])
        }

        async fn list_episodes_in_date_range(
            &self,
            _: &str,
            _: &str,
        ) -> AppResult<Vec<CalendarEpisode>> {
            Ok(vec![])
        }

        async fn replace_anibridge_scoped_external_ids_for_title(
            &self,
            _: &str,
            _: Vec<ScopedExternalId>,
            _: Vec<ScopedExternalId>,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestImportArtifactRepo {
        artifacts: Arc<Mutex<Vec<ImportArtifact>>>,
    }

    #[async_trait]
    impl ImportArtifactRepository for TestImportArtifactRepo {
        async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
            self.artifacts.lock().await.push(artifact);
            Ok(())
        }

        async fn list_by_source_identity(
            &self,
            identity: &DownloadSourceIdentity,
        ) -> AppResult<Vec<ImportArtifact>> {
            let artifacts = self.artifacts.lock().await;
            Ok(artifacts
                .iter()
                .filter(|artifact| artifact.source_identity() == *identity)
                .cloned()
                .collect())
        }

        async fn count_by_result_for_source_identity(
            &self,
            identity: &DownloadSourceIdentity,
            result: &str,
        ) -> AppResult<u64> {
            let artifacts = self.artifacts.lock().await;
            Ok(artifacts
                .iter()
                .filter(|artifact| {
                    artifact.source_identity() == *identity && artifact.result == result
                })
                .count() as u64)
        }
    }

    #[derive(Default)]
    struct TestDownloadSubmissionRepo {
        rows: Arc<Mutex<Vec<(DownloadSubmission, DownloadSubmissionIdentity)>>>,
        tracked_states: Arc<Mutex<Vec<(DownloadSourceIdentity, String)>>>,
        identity_tracked_states: Arc<Mutex<Vec<(String, String)>>>,
    }

    fn test_download_identity_state_key(
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> Option<String> {
        let download_id = identity
            .download_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        if download_id.starts_with("scryer-download:")
            || (matches!(download_id.len(), 40 | 64)
                && download_id.chars().all(|ch| ch.is_ascii_hexdigit()))
        {
            return Some(format!("download:{download_id}"));
        }

        let source_identity = source_identity?;
        let client_type = source_identity.client_type.trim();
        if client_type.is_empty() {
            return None;
        }
        Some(format!(
            "client:{}:{}:download:{}",
            source_identity
                .client_id
                .as_deref()
                .map(str::trim)
                .unwrap_or_default(),
            client_type.to_ascii_lowercase(),
            download_id
        ))
    }

    #[async_trait]
    impl DownloadSubmissionRepository for TestDownloadSubmissionRepo {
        async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
            self.rows
                .lock()
                .await
                .push((submission, DownloadSubmissionIdentity::default()));
            Ok(())
        }

        async fn record_submission_with_identity(
            &self,
            submission: DownloadSubmission,
            submission_identity: DownloadSubmissionIdentity,
        ) -> AppResult<()> {
            self.rows
                .lock()
                .await
                .push((submission, submission_identity));
            Ok(())
        }

        async fn find_by_client_item_id(
            &self,
            identity: &DownloadSourceIdentity,
        ) -> AppResult<Option<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .find(|(submission, _)| {
                    DownloadSourceIdentity::from_submission(submission) == *identity
                })
                .map(|(submission, _)| submission.clone()))
        }

        async fn list_by_download_id(
            &self,
            client_id: Option<&str>,
            client_type: &str,
            download_id: &str,
        ) -> AppResult<Vec<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .filter(|(submission, identity)| {
                    submission.download_client_id.as_deref().unwrap_or("")
                        == client_id.unwrap_or("")
                        && submission
                            .download_client_type
                            .eq_ignore_ascii_case(client_type)
                        && identity.download_id.as_deref() == Some(download_id)
                })
                .map(|(submission, _)| submission.clone())
                .collect())
        }

        async fn get_submission_identity(
            &self,
            identity: &DownloadSourceIdentity,
        ) -> AppResult<Option<DownloadSubmissionIdentity>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .find(|(submission, _)| {
                    DownloadSourceIdentity::from_submission(submission) == *identity
                })
                .map(|(_, submission_identity)| submission_identity.clone()))
        }

        async fn record_identity_tracked_state(
            &self,
            identity: &DownloadSubmissionIdentity,
            source_identity: Option<&DownloadSourceIdentity>,
            tracked_state: &str,
            _reason: Option<&str>,
            _detail: Option<&str>,
        ) -> AppResult<()> {
            let Some(key) = test_download_identity_state_key(identity, source_identity) else {
                return Ok(());
            };
            let mut states = self.identity_tracked_states.lock().await;
            if let Some((_, state)) = states.iter_mut().find(|(stored_key, _)| stored_key == &key) {
                *state = tracked_state.to_string();
            } else {
                states.push((key, tracked_state.to_string()));
            }
            Ok(())
        }

        async fn get_identity_tracked_state(
            &self,
            identity: &DownloadSubmissionIdentity,
            source_identity: Option<&DownloadSourceIdentity>,
        ) -> AppResult<Option<String>> {
            let Some(key) = test_download_identity_state_key(identity, source_identity) else {
                return Ok(None);
            };
            Ok(self
                .identity_tracked_states
                .lock()
                .await
                .iter()
                .find(|(stored_key, _)| stored_key == &key)
                .map(|(_, state)| state.clone()))
        }

        async fn list_for_client_items(
            &self,
            client_items: &[DownloadSourceIdentity],
        ) -> AppResult<Vec<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .filter(|(submission, _)| {
                    let identity = DownloadSourceIdentity::from_submission(submission);
                    client_items.contains(&identity)
                })
                .map(|(submission, _)| submission.clone())
                .collect())
        }

        async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .filter(|(submission, _)| submission.title_id == title_id)
                .map(|(submission, _)| submission.clone())
                .collect())
        }

        async fn find_by_title_and_request_signature(
            &self,
            title_id: &str,
            request_signature: &str,
        ) -> AppResult<Option<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .find(|(submission, _)| {
                    submission.title_id == title_id
                        && submission.request_signature.as_deref() == Some(request_signature)
                })
                .map(|(submission, _)| submission.clone()))
        }

        async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
            self.rows
                .lock()
                .await
                .retain(|(submission, _)| submission.title_id != title_id);
            Ok(())
        }

        async fn delete_by_client_item_id(
            &self,
            identity: &DownloadSourceIdentity,
        ) -> AppResult<()> {
            self.rows.lock().await.retain(|(submission, _)| {
                DownloadSourceIdentity::from_submission(submission) != *identity
            });
            Ok(())
        }

        async fn update_tracked_state(
            &self,
            identity: &DownloadSourceIdentity,
            tracked_state: &str,
        ) -> AppResult<()> {
            let mut states = self.tracked_states.lock().await;
            if let Some((_, state)) = states
                .iter_mut()
                .find(|(stored_identity, _)| stored_identity == identity)
            {
                *state = tracked_state.to_string();
            } else {
                states.push((identity.clone(), tracked_state.to_string()));
            }
            Ok(())
        }

        async fn get_tracked_state(
            &self,
            identity: &DownloadSourceIdentity,
        ) -> AppResult<Option<String>> {
            Ok(self
                .tracked_states
                .lock()
                .await
                .iter()
                .find(|(stored_identity, _)| stored_identity == identity)
                .map(|(_, state)| state.clone()))
        }
    }

    struct TestDownloadClientConfigRepo {
        configs: Vec<DownloadClientConfig>,
    }

    #[async_trait]
    impl DownloadClientConfigRepository for TestDownloadClientConfigRepo {
        async fn list(
            &self,
            _provider_type: Option<String>,
        ) -> AppResult<Vec<DownloadClientConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
            Ok(self.configs.iter().find(|config| config.id == id).cloned())
        }

        async fn create(&self, _config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update(
            &self,
            _update: crate::DownloadClientConfigUpdate,
        ) -> AppResult<DownloadClientConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn reorder(&self, _ordered_ids: Vec<String>) -> AppResult<()> {
            Err(AppError::Repository("not needed in test".into()))
        }
    }

    #[derive(Default)]
    struct TestIndexerConfigRepo;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepo {
        async fn list(&self, _: Option<String>) -> AppResult<Vec<scryer_domain::IndexerConfig>> {
            Ok(vec![])
        }

        async fn get_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::IndexerConfig>> {
            Ok(None)
        }

        async fn create(
            &self,
            _: scryer_domain::IndexerConfig,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn touch_last_error(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(
            &self,
            _: crate::IndexerConfigUpdate,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestQualityProfileRepo;

    #[async_trait]
    impl QualityProfileRepository for TestQualityProfileRepo {
        async fn list_quality_profiles(
            &self,
            _: &str,
            _: Option<String>,
        ) -> AppResult<Vec<QualityProfile>> {
            Ok(vec![])
        }

        async fn replace_quality_profiles(
            &self,
            _: &str,
            _: Option<String>,
            _: Vec<QualityProfile>,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestDomainEventRepo {
        events: Arc<Mutex<Vec<DomainEvent>>>,
        subscriber_offsets: Arc<Mutex<std::collections::HashMap<String, i64>>>,
    }

    #[derive(Default)]
    struct TestDownloadClient {
        completed_downloads: Arc<Mutex<Vec<CompletedDownload>>>,
        completed_download_calls: Arc<AtomicUsize>,
        recent_completed_download_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DownloadClient for TestDownloadClient {
        async fn submit_download(
            &self,
            _: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
            self.completed_download_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.completed_downloads.lock().await.clone())
        }

        async fn list_recent_completed_downloads(
            &self,
            limit: usize,
        ) -> AppResult<Vec<CompletedDownload>> {
            self.recent_completed_download_calls
                .fetch_add(1, Ordering::SeqCst);
            Ok(self
                .completed_downloads
                .lock()
                .await
                .iter()
                .take(limit)
                .cloned()
                .collect())
        }
    }

    #[async_trait]
    impl DomainEventRepository for TestDomainEventRepo {
        async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
            let mut events = self.events.lock().await;
            let sequence = events
                .last()
                .map(|existing| existing.sequence + 1)
                .unwrap_or(1);
            let stored = DomainEvent {
                sequence,
                event_id: event.event_id,
                occurred_at: event.occurred_at,
                actor_user_id: event.actor_user_id,
                title_id: event.title_id,
                facet: event.facet,
                correlation_id: event.correlation_id,
                causation_id: event.causation_id,
                schema_version: event.schema_version,
                stream: event.stream,
                payload: event.payload,
            };
            events.push(stored.clone());
            Ok(stored)
        }

        async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
            let mut stored = Vec::with_capacity(events.len());
            for event in events {
                stored.push(self.append(event).await?);
            }
            Ok(stored)
        }

        async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
            let events = self.events.lock().await;
            let limit = if filter.limit == 0 {
                usize::MAX
            } else {
                filter.limit
            };
            let iter: Box<dyn Iterator<Item = &DomainEvent>> =
                if filter.after_sequence.is_some() && filter.before_sequence.is_none() {
                    Box::new(events.iter())
                } else {
                    Box::new(events.iter().rev())
                };
            Ok(iter
                .filter(|event| {
                    filter
                        .after_sequence
                        .is_none_or(|after| event.sequence > after)
                        && filter
                            .before_sequence
                            .is_none_or(|before| event.sequence < before)
                        && filter.title_id.as_ref().is_none_or(|title_id| {
                            event.title_id.as_deref() == Some(title_id.as_str())
                        })
                        && filter
                            .facet
                            .as_ref()
                            .is_none_or(|facet| event.facet.as_ref() == Some(facet))
                        && filter.event_types.as_ref().is_none_or(|event_types| {
                            event_types
                                .iter()
                                .any(|event_type| &event.payload.event_type() == event_type)
                        })
                })
                .take(limit)
                .cloned()
                .collect())
        }

        async fn count_title_history_page_events(
            &self,
            event_types: Option<&[TitleHistoryEventType]>,
            title_ids: Option<&[String]>,
            download_id: Option<&str>,
        ) -> AppResult<i64> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .rev()
                .filter_map(crate::event_views::title_history_record_from_domain_event)
                .filter(|record| {
                    event_types.is_none_or(|values| values.contains(&record.event_type))
                        && title_ids.is_none_or(|values| values.contains(&record.title_id))
                        && download_id
                            .is_none_or(|value| record.download_id.as_deref() == Some(value))
                })
                .count() as i64)
        }

        async fn list_title_history_page_events(
            &self,
            event_types: Option<&[TitleHistoryEventType]>,
            title_ids: Option<&[String]>,
            download_id: Option<&str>,
            limit: usize,
            offset: usize,
        ) -> AppResult<Vec<DomainEvent>> {
            let page_size = if limit == 0 { usize::MAX } else { limit };
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .rev()
                .filter(|event| {
                    crate::event_views::title_history_record_from_domain_event(event).is_some_and(
                        |record| {
                            event_types.is_none_or(|values| values.contains(&record.event_type))
                                && title_ids.is_none_or(|values| values.contains(&record.title_id))
                                && download_id.is_none_or(|value| {
                                    record.download_id.as_deref() == Some(value)
                                })
                        },
                    )
                })
                .skip(offset)
                .take(page_size)
                .cloned()
                .collect())
        }

        async fn list_after_sequence(
            &self,
            after_sequence: i64,
            limit: usize,
        ) -> AppResult<Vec<DomainEvent>> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .take(limit)
                .cloned()
                .collect())
        }

        async fn delete_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
            Ok(0)
        }

        async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
            let offsets = self.subscriber_offsets.lock().await;
            Ok(*offsets.get(subscriber).unwrap_or(&0))
        }

        async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
            let mut offsets = self.subscriber_offsets.lock().await;
            offsets.insert(subscriber.to_string(), sequence);
            Ok(())
        }
    }

    type TestSettingsKey = (String, String, Option<String>);
    type TestSettingsValues = Arc<Mutex<HashMap<TestSettingsKey, String>>>;

    #[derive(Default)]
    struct TestSettingsRepo {
        values: TestSettingsValues,
    }

    impl TestSettingsRepo {
        async fn set_scoped_json(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: &str,
            value_json: &str,
        ) {
            self.values.lock().await.insert(
                (
                    scope.to_string(),
                    key_name.to_string(),
                    Some(scope_id.to_string()),
                ),
                value_json.to_string(),
            );
        }
    }

    #[async_trait]
    impl SettingsRepository for TestSettingsRepo {
        async fn get_setting_json(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(self
                .values
                .lock()
                .await
                .get(&(scope.to_string(), key_name.to_string(), scope_id))
                .cloned())
        }

        async fn get_setting_json_explicit(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            self.get_setting_json(scope, key_name, scope_id).await
        }

        async fn upsert_setting_json(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
            value_json: String,
            _source: &str,
            _updated_by_user_id: Option<String>,
        ) -> AppResult<()> {
            self.values.lock().await.insert(
                (scope.to_string(), key_name.to_string(), scope_id),
                value_json,
            );
            Ok(())
        }

        async fn delete_setting_value(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<()> {
            self.values
                .lock()
                .await
                .remove(&(scope.to_string(), key_name.to_string(), scope_id));
            Ok(())
        }

        async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
            let mut values = self.values.lock().await;
            let before = values.len();
            values
                .retain(|(_, _, stored_scope_id), _| stored_scope_id.as_deref() != Some(scope_id));
            Ok((before - values.len()) as u32)
        }
    }

    fn build_app(
        titles: Vec<Title>,
        collections: Vec<Collection>,
        episodes: Vec<Episode>,
        artifacts: Vec<ImportArtifact>,
    ) -> AppUseCase {
        build_app_with_download_client(
            titles,
            collections,
            episodes,
            artifacts,
            Arc::new(NullDownloadClient),
        )
    }

    fn build_app_with_download_client(
        titles: Vec<Title>,
        collections: Vec<Collection>,
        episodes: Vec<Episode>,
        artifacts: Vec<ImportArtifact>,
        download_client: Arc<dyn DownloadClient>,
    ) -> AppUseCase {
        build_app_with_download_client_and_configs(
            titles,
            collections,
            episodes,
            artifacts,
            download_client,
            Arc::new(NullDownloadClientConfigRepository),
        )
    }

    fn build_app_with_download_client_and_configs(
        titles: Vec<Title>,
        collections: Vec<Collection>,
        episodes: Vec<Episode>,
        artifacts: Vec<ImportArtifact>,
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    ) -> AppUseCase {
        build_app_with_download_client_configs_and_submissions(
            titles,
            collections,
            episodes,
            artifacts,
            download_client,
            download_client_configs,
            Arc::new(crate::null_repositories::NullDownloadSubmissionRepository),
        )
    }

    fn build_app_with_download_client_configs_and_submissions(
        titles: Vec<Title>,
        collections: Vec<Collection>,
        episodes: Vec<Episode>,
        artifacts: Vec<ImportArtifact>,
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        download_submissions: Arc<dyn DownloadSubmissionRepository>,
    ) -> AppUseCase {
        build_app_with_download_client_configs_submissions_and_settings(
            titles,
            collections,
            episodes,
            artifacts,
            TestAppRepositories {
                download_client,
                download_client_configs,
                download_submissions,
                settings: Arc::new(crate::null_repositories::NullSettingsRepository),
            },
        )
    }

    struct TestAppRepositories {
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        download_submissions: Arc<dyn DownloadSubmissionRepository>,
        settings: Arc<dyn SettingsRepository>,
    }

    fn build_app_with_download_client_configs_submissions_and_settings(
        titles: Vec<Title>,
        collections: Vec<Collection>,
        episodes: Vec<Episode>,
        artifacts: Vec<ImportArtifact>,
        repositories: TestAppRepositories,
    ) -> AppUseCase {
        let services = AppServices::builder(
            Arc::new(TestTitleRepo {
                titles: Arc::new(Mutex::new(titles)),
            }),
            Arc::new(TestShowRepo {
                collections: Arc::new(Mutex::new(collections)),
                episodes: Arc::new(Mutex::new(episodes)),
            }),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            repositories.download_client,
            repositories.download_client_configs,
            Arc::new(NullReleaseAttemptRepository),
            repositories.settings,
            Arc::new(TestQualityProfileRepo),
            String::new(),
        )
        .with_domain_events(Arc::new(TestDomainEventRepo::default()))
        .with_import_artifacts(Arc::new(TestImportArtifactRepo {
            artifacts: Arc::new(Mutex::new(artifacts)),
        }))
        .with_download_submissions(repositories.download_submissions)
        .build_partial_for_tests();

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn build_title(id: &str, name: &str, facet: MediaFacet) -> Title {
        Title {
            id: id.to_string(),
            name: name.to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
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

    fn build_collection(id: &str, title_id: &str, season: &str) -> Collection {
        Collection {
            id: id.to_string(),
            title_id: title_id.to_string(),
            collection_type: CollectionType::Season,
            collection_index: season.to_string(),
            label: Some(format!("Season {season}")),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn build_episode(
        id: &str,
        title_id: &str,
        collection_id: &str,
        season_number: &str,
        episode_number: &str,
        absolute_number: Option<&str>,
    ) -> Episode {
        build_episode_with_details(
            id,
            title_id,
            collection_id,
            EpisodeType::Standard,
            season_number,
            episode_number,
            None,
            absolute_number,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture helper mirrors the episode fields under test"
    )]
    fn build_episode_with_details(
        id: &str,
        title_id: &str,
        collection_id: &str,
        episode_type: EpisodeType,
        season_number: &str,
        episode_number: &str,
        air_date: Option<&str>,
        absolute_number: Option<&str>,
    ) -> Episode {
        Episode {
            id: id.to_string(),
            title_id: title_id.to_string(),
            collection_id: Some(collection_id.to_string()),
            episode_type,
            episode_number: Some(episode_number.to_string()),
            season_number: Some(season_number.to_string()),
            episode_label: None,
            title: None,
            air_date: air_date.map(str::to_string),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: absolute_number.map(str::to_string),
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn build_artifact(
        source_ref: &str,
        episode_id: &str,
        normalized_file_name: &str,
    ) -> ImportArtifact {
        build_artifact_with_result(
            source_ref,
            Some(episode_id),
            normalized_file_name,
            "imported",
        )
    }

    fn build_artifact_with_result(
        source_ref: &str,
        episode_id: Option<&str>,
        normalized_file_name: &str,
        result: &str,
    ) -> ImportArtifact {
        ImportArtifact {
            id: Id::new().0,
            source_client_id: Some("client-1".to_string()),
            source_system: "nzbget".to_string(),
            source_ref: source_ref.to_string(),
            import_id: None,
            relative_path: None,
            normalized_file_name: normalized_file_name.to_string(),
            media_kind: "episode".to_string(),
            title_id: Some("title-1".to_string()),
            episode_id: episode_id.map(str::to_string),
            season_number: Some(1),
            episode_number: None,
            result: result.to_string(),
            reason_code: None,
            imported_media_file_id: None,
            created_at: Utc::now(),
        }
    }

    fn build_tracked_download(title_id: &str, facet: &str, release_title: &str) -> TrackedDownload {
        TrackedDownload {
            id: format!("nzbget:{release_title}"),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: DownloadQueueItem {
                id: Id::new().0,
                title_id: Some(title_id.to_string()),
                episode_id: None,
                title_name: release_title.to_string(),
                facet: Some(facet.to_string()),
                category: None,
                client_id: "client-1".to_string(),
                client_name: "NZBGet".to_string(),
                client_type: "nzbget".to_string(),
                state: DownloadQueueState::Completed,
                progress_percent: 100,
                size_bytes: None,
                remaining_seconds: None,
                queued_at: None,
                last_updated_at: None,
                attention_required: false,
                attention_reason: None,
                download_client_item_id: "dl-1".to_string(),
                download_id: None,
                import_status: None,
                import_error_code: None,
                import_error_message: None,
                imported_at: None,
                delete_status: None,
                delete_error_message: None,
                is_scryer_origin: true,
                tracked_state: None,
                tracked_status: None,
                tracked_status_messages: vec![],
                tracked_match_type: None,
            },
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: vec![],
            title_id: Some(title_id.to_string()),
            facet: Some(facet.to_string()),
            source_title: Some(release_title.to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
            skip_reacquire_on_failure: false,
        }
    }

    fn build_completed_download(
        name: &str,
        dest_dir: &str,
        category: Option<&str>,
    ) -> CompletedDownload {
        CompletedDownload {
            client_type: "nzbget".to_string(),
            client_id: "client-1".to_string(),
            download_client_item_id: "dl-1".to_string(),
            download_id: None,
            name: name.to_string(),
            dest_dir: dest_dir.to_string(),
            category: category.map(str::to_string),
            size_bytes: None,
            completed_at: None,
            parameters: vec![],
        }
    }

    fn test_download_client_with_completed(
        completed: CompletedDownload,
    ) -> Arc<TestDownloadClient> {
        Arc::new(TestDownloadClient {
            completed_downloads: Arc::new(Mutex::new(vec![completed])),
            completed_download_calls: Arc::new(AtomicUsize::new(0)),
            recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn build_foreign_completed_tracked_download(
        category: Option<&str>,
        match_type: TitleMatchType,
        is_scryer_origin: bool,
    ) -> TrackedDownload {
        let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p.WEB-DL");
        td.client_item.is_scryer_origin = is_scryer_origin;
        td.client_item.category = category.map(str::to_string);
        td.match_type = match_type;
        td
    }

    async fn run_category_gate_check(
        settings: Arc<TestSettingsRepo>,
        completed_category: Option<&str>,
        queue_category: Option<&str>,
        match_type: TitleMatchType,
        is_scryer_origin: bool,
    ) -> TrackedDownload {
        if settings
            .get_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
                Some("movie".to_string()),
            )
            .await
            .expect("read default category")
            .is_none()
        {
            set_scoped_default_category(&settings, "movie", "movie").await;
        }

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p.WEB-DL",
            temp_dir.path().to_string_lossy().as_ref(),
            completed_category,
        );
        let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
        let download_client = test_download_client_with_completed(completed);
        let app = build_app_with_download_client_configs_submissions_and_settings(
            vec![title],
            vec![],
            vec![],
            vec![],
            TestAppRepositories {
                download_client,
                download_client_configs: Arc::new(NullDownloadClientConfigRepository),
                download_submissions: Arc::new(
                    crate::null_repositories::NullDownloadSubmissionRepository,
                ),
                settings,
            },
        );
        let mut td =
            build_foreign_completed_tracked_download(queue_category, match_type, is_scryer_origin);

        check(&app, &mut td).await;
        td
    }

    async fn set_scoped_routing(settings: &TestSettingsRepo, scope_id: &str, routing_json: &str) {
        settings
            .set_scoped_json(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                scope_id,
                routing_json,
            )
            .await;
    }

    async fn set_scoped_default_category(
        settings: &TestSettingsRepo,
        scope_id: &str,
        category: &str,
    ) {
        settings
            .set_scoped_json(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
                scope_id,
                &serde_json::json!(category).to_string(),
            )
            .await;
    }

    #[test]
    fn completed_download_lookup_keeps_same_native_id_from_different_clients() {
        let first =
            build_completed_download("Paper.Lantern.2012.1080p", "/downloads/a", Some("movie"));
        let mut second =
            build_completed_download("Paper.Lantern.2012.1080p", "/downloads/b", Some("movie"));
        second.client_id = "client-2".to_string();

        let lookup = index_completed_downloads(vec![first, second]);

        assert_eq!(lookup.by_source.len(), 2);
        assert!(
            lookup
                .by_source
                .contains_key(&completed_download_lookup_key(
                    Some("client-1"),
                    "nzbget",
                    "dl-1"
                ))
        );
        assert!(
            lookup
                .by_source
                .contains_key(&completed_download_lookup_key(
                    Some("client-2"),
                    "nzbget",
                    "dl-1"
                ))
        );
    }

    #[tokio::test]
    async fn foreign_title_parse_requires_submission_origin_or_scryer_category() {
        for (completed_category, queue_category) in [(None, None), (Some("other"), None)] {
            let td = run_category_gate_check(
                Arc::new(TestSettingsRepo::default()),
                completed_category,
                queue_category,
                TitleMatchType::TitleParse,
                false,
            )
            .await;
            assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
            assert!(
                td.status_messages
                    .iter()
                    .any(|message| message == FOREIGN_CATEGORY_BLOCKED_MESSAGE)
            );
        }

        let td = run_category_gate_check(
            Arc::new(TestSettingsRepo::default()),
            Some("movie"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);

        let default_category_settings = Arc::new(TestSettingsRepo::default());
        set_scoped_default_category(&default_category_settings, "movie", "Configured Movies").await;
        let td = run_category_gate_check(
            default_category_settings,
            Some("Configured Movies"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);
    }

    #[tokio::test]
    async fn foreign_title_parse_with_orphan_submission_still_requires_scryer_category() {
        let settings = Arc::new(TestSettingsRepo::default());
        set_scoped_default_category(&settings, "movie", "movie").await;
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p.WEB-DL",
            temp_dir.path().to_string_lossy().as_ref(),
            None,
        );
        let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
        let download_client = test_download_client_with_completed(completed);
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        download_submissions
            .record_submission(DownloadSubmission {
                title_id: String::new(),
                facet: "movie".to_string(),
                download_client_id: Some("client-1".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "dl-1".to_string(),
                source_hint: None,
                source_kind: None,
                source_title: Some("Paper.Lantern.2012.1080p.WEB-DL".to_string()),
                request_signature: None,
                scope: SubmissionScope::Orphan,
            })
            .await
            .expect("record orphan submission");
        let app = build_app_with_download_client_configs_submissions_and_settings(
            vec![title],
            vec![],
            vec![],
            vec![],
            TestAppRepositories {
                download_client,
                download_client_configs: Arc::new(NullDownloadClientConfigRepository),
                download_submissions,
                settings,
            },
        );
        let mut td =
            build_foreign_completed_tracked_download(None, TitleMatchType::TitleParse, false);

        check(&app, &mut td).await;

        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        assert!(
            td.status_messages
                .iter()
                .any(|message| message == FOREIGN_CATEGORY_BLOCKED_MESSAGE)
        );
    }

    #[tokio::test]
    async fn completed_category_gate_honors_facet_and_library_shadowing() {
        let facet_settings = Arc::new(TestSettingsRepo::default());
        set_scoped_routing(
            &facet_settings,
            "movie",
            r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
        )
        .await;
        let td = run_category_gate_check(
            facet_settings,
            Some("Facet Movies"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);

        let library_settings = Arc::new(TestSettingsRepo::default());
        set_scoped_routing(
            &library_settings,
            "movie",
            r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
        )
        .await;
        set_scoped_routing(
            &library_settings,
            "movie_default_library",
            r#"{"client-1":{"enabled":true,"category":"Library Movies"}}"#,
        )
        .await;
        let td = run_category_gate_check(
            library_settings.clone(),
            Some("Facet Movies"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        let td = run_category_gate_check(
            library_settings,
            Some("Library Movies"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);

        let empty_library_category_settings = Arc::new(TestSettingsRepo::default());
        set_scoped_routing(
            &empty_library_category_settings,
            "movie",
            r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
        )
        .await;
        set_scoped_routing(
            &empty_library_category_settings,
            "movie_default_library",
            r#"{"client-1":{"enabled":true,"category":""}}"#,
        )
        .await;
        let td = run_category_gate_check(
            empty_library_category_settings.clone(),
            Some("Facet Movies"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        let td = run_category_gate_check(
            empty_library_category_settings,
            Some("movie"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);
    }

    #[tokio::test]
    async fn completed_category_gate_honors_missing_disabled_and_invalid_routing() {
        let missing_library_client_settings = Arc::new(TestSettingsRepo::default());
        set_scoped_routing(
            &missing_library_client_settings,
            "movie",
            r#"{"client-1":{"enabled":true,"category":"movie"}}"#,
        )
        .await;
        set_scoped_routing(
            &missing_library_client_settings,
            "movie_default_library",
            r#"{"other-client":{"enabled":true,"category":"movie"}}"#,
        )
        .await;
        let td = run_category_gate_check(
            missing_library_client_settings,
            Some("movie"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);

        let missing_facet_client_settings = Arc::new(TestSettingsRepo::default());
        set_scoped_routing(
            &missing_facet_client_settings,
            "movie",
            r#"{"other-client":{"enabled":true,"category":"other"}}"#,
        )
        .await;
        let td = run_category_gate_check(
            missing_facet_client_settings,
            Some("movie"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);

        for (scope_id, settings) in [
            (
                "movie_default_library",
                Arc::new(TestSettingsRepo::default()),
            ),
            ("movie", Arc::new(TestSettingsRepo::default())),
        ] {
            set_scoped_routing(
                &settings,
                scope_id,
                r#"{"client-1":{"enabled":false,"category":"movie"}}"#,
            )
            .await;
            let td = run_category_gate_check(
                settings,
                Some("movie"),
                None,
                TitleMatchType::TitleParse,
                false,
            )
            .await;
            assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        }

        let invalid_library_settings = Arc::new(TestSettingsRepo::default());
        set_scoped_routing(
            &invalid_library_settings,
            "movie_default_library",
            "not-json",
        )
        .await;
        set_scoped_routing(
            &invalid_library_settings,
            "movie",
            r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
        )
        .await;
        let td = run_category_gate_check(
            invalid_library_settings,
            Some("Facet Movies"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);
    }

    #[tokio::test]
    async fn confirmed_completed_downloads_bypass_category_gate() {
        for (match_type, is_scryer_origin) in [
            (TitleMatchType::Submission, false),
            (TitleMatchType::ClientParameter, false),
            (TitleMatchType::TitleParse, true),
        ] {
            let td = run_category_gate_check(
                Arc::new(TestSettingsRepo::default()),
                None,
                None,
                match_type,
                is_scryer_origin,
            )
            .await;
            assert_eq!(td.state, TrackedDownloadState::ImportPending);
        }
    }

    #[tokio::test]
    async fn manual_assignment_allows_retry_after_category_block() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p.WEB-DL",
            temp_dir.path().to_string_lossy().as_ref(),
            None,
        );
        let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
        let download_client = test_download_client_with_completed(completed);
        let app = build_app_with_download_client(
            vec![title.clone()],
            vec![],
            vec![],
            vec![],
            download_client,
        );
        let mut td =
            build_foreign_completed_tracked_download(None, TitleMatchType::TitleParse, false);

        check(&app, &mut td).await;
        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);

        crate::tracked_downloads::assign_title_to_tracked_download(&app, &mut td, &title).await;
        assert_eq!(td.match_type, TitleMatchType::Submission);
        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);

        td.state = TrackedDownloadState::Downloading;
        check(&app, &mut td).await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);
    }

    #[test]
    fn completed_download_path_retries_during_grace_window_and_blocks_after_deadline() {
        let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
        let missing_dir = std::env::temp_dir().join(format!("scryer-missing-path-{}", Id::new().0));
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p",
            missing_dir.to_string_lossy().as_ref(),
            Some("movie"),
        );
        let now = Utc::now();

        assert_eq!(
            evaluate_completed_download_path(&mut td, &completed, now),
            CompletedDownloadPathState::Retry
        );
        assert_eq!(td.status_messages, vec![PATH_WAITING_MESSAGE.to_string()]);
        assert_eq!(td.path_missing_since, Some(now));

        assert_eq!(
            evaluate_completed_download_path(
                &mut td,
                &completed,
                now + Duration::minutes(COMPLETED_PATH_GRACE_PERIOD_MINUTES + 1),
            ),
            CompletedDownloadPathState::Blocked
        );
        assert_eq!(td.status_messages, vec![PATH_BLOCKED_MESSAGE.to_string()]);
    }

    #[test]
    fn completed_download_path_ready_clears_waiting_state_when_path_appears() {
        let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
        td.path_missing_since = Some(Utc::now() - Duration::minutes(5));
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages = vec![PATH_WAITING_MESSAGE.to_string()];

        let existing_dir = std::env::temp_dir().join(format!("scryer-path-ready-{}", Id::new().0));
        std::fs::create_dir_all(&existing_dir).expect("create temp dir");
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p",
            existing_dir.to_string_lossy().as_ref(),
            Some("movie"),
        );

        assert_eq!(
            evaluate_completed_download_path(&mut td, &completed, Utc::now()),
            CompletedDownloadPathState::Ready
        );
        assert!(td.path_missing_since.is_none());
        assert!(td.status_messages.is_empty());
        assert_eq!(td.status, TrackedDownloadStatus::Ok);

        std::fs::remove_dir_all(&existing_dir).expect("remove temp dir");
    }

    #[test]
    fn completed_download_path_blocks_immediately_for_url_sources() {
        let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p",
            "https://nzbdav.example/remote/completed-symlinks/Paper.Lantern.2012.1080p",
            Some("movie"),
        );

        assert_eq!(
            evaluate_completed_download_path(&mut td, &completed, Utc::now()),
            CompletedDownloadPathState::Blocked
        );
        assert!(td.path_missing_since.is_none());
        assert_eq!(
            td.status_messages,
            vec![PATH_URL_UNSUPPORTED_MESSAGE.to_string()]
        );
    }

    #[tokio::test]
    async fn check_with_lookup_remaps_remote_completed_download_paths_before_readiness_checks() {
        let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
        let local_root =
            std::env::temp_dir().join(format!("scryer-remote-path-map-{}", Id::new().0));
        let local_download_dir = local_root.join("Paper.Lantern.2012.1080p");
        std::fs::create_dir_all(&local_download_dir).expect("create mapped download directory");

        let remote_completed = build_completed_download(
            "Paper.Lantern.2012.1080p",
            "/downloads/Paper.Lantern.2012.1080p",
            Some("movie"),
        );
        let lookup = index_completed_downloads(vec![remote_completed]);
        let download_client = Arc::new(TestDownloadClient::default());
        let app = build_app_with_download_client_and_configs(
            vec![title.clone()],
            vec![],
            vec![],
            vec![],
            download_client,
            Arc::new(TestDownloadClientConfigRepo {
                configs: vec![DownloadClientConfig {
                    id: "client-1".to_string(),
                    name: "qBittorrent".to_string(),
                    client_type: "qbittorrent".to_string(),
                    config_json: format!(
                        r#"{{"remote_path_mappings":"/downloads => {}"}}"#,
                        local_root.to_string_lossy()
                    ),
                    is_enabled: true,
                    status: scryer_domain::DownloadClientStatus::Healthy,
                    last_error: None,
                    last_seen_at: None,
                    client_priority: 0,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }],
            }),
        );
        let mut td = build_tracked_download(&title.id, "movie", "Paper.Lantern.2012.1080p");

        check_with_lookup(&app, &mut td, Some(&lookup)).await;

        assert_eq!(td.state, TrackedDownloadState::ImportPending);
        assert!(td.status_messages.is_empty());

        std::fs::remove_dir_all(&local_root).expect("remove mapped download directory");
    }

    #[tokio::test]
    async fn check_with_lookup_matches_qbit_torrent_hash_download_id() {
        let title = build_title("title-1", "Paperman", MediaFacet::Movie);
        let release_title = "Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb";
        let info_hash = "5b4ba671c3729e34718de86e3372be2ecb527b15";
        let accepted_identity = crate::download_identity::accepted_download_submission_identity(
            crate::download_identity::AcceptedDownloadIdentityInput {
                initial_download_id: Some("scryer-download:test-qbit"),
                source_kind: Some(crate::DownloadSourceKind::TorrentFile),
                source_hint: Some("http://torrent-indexer/download/paperman.torrent"),
                info_hash_hint: None,
                client_type: Some("qbittorrent"),
                client_item_id: Some(info_hash),
                accepted_info_hash: Some(info_hash),
            },
        );
        assert_eq!(accepted_identity.download_id.as_deref(), Some(info_hash));

        let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
        submission_repo
            .record_submission_with_identity(
                DownloadSubmission {
                    title_id: title.id.clone(),
                    facet: title.facet.as_str().to_string(),
                    download_client_id: Some("client-1".to_string()),
                    download_client_type: "qbittorrent".to_string(),
                    download_client_item_id: info_hash.to_string(),
                    source_hint: Some("http://torrent-indexer/download/paperman.torrent".to_string()),
                    source_kind: Some(crate::DownloadSourceKind::TorrentFile),
                    source_title: Some(release_title.to_string()),
                    request_signature: Some(
                        "torrent_file|http://torrent-indexer/download/paperman.torrent|Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb"
                            .to_string(),
                    ),
                    scope: crate::SubmissionScope::Title,
                },
                accepted_identity,
            )
            .await
            .expect("record submission");

        let completed_dir =
            std::env::temp_dir().join(format!("scryer-qbit-completed-{}", Id::new().0));
        std::fs::create_dir_all(&completed_dir).expect("create completed dir");
        let mut completed = build_completed_download(
            release_title,
            completed_dir.to_string_lossy().as_ref(),
            Some("movie"),
        );
        completed.client_type = "qbittorrent".to_string();
        completed.client_id = "client-1".to_string();
        completed.download_client_item_id = info_hash.to_string();
        completed.download_id = Some(info_hash.to_string());
        let lookup = index_completed_downloads(vec![completed]);

        let app = build_app_with_download_client_configs_and_submissions(
            vec![title.clone()],
            vec![],
            vec![],
            vec![],
            Arc::new(TestDownloadClient::default()),
            Arc::new(NullDownloadClientConfigRepository),
            submission_repo,
        );
        let mut td = build_tracked_download(&title.id, "movie", release_title);
        td.id = format!("download:{info_hash}");
        td.client_type = "qbittorrent".to_string();
        td.client_id = "client-1".to_string();
        td.client_item.client_id = "client-1".to_string();
        td.client_item.client_name = "qBittorrent".to_string();
        td.client_item.client_type = "qbittorrent".to_string();
        td.client_item.download_client_item_id = info_hash.to_string();
        td.client_item.download_id = Some(info_hash.to_string());

        check_with_lookup(&app, &mut td, Some(&lookup)).await;

        assert_eq!(td.state, TrackedDownloadState::ImportPending);
        assert!(td.status_messages.is_empty());

        std::fs::remove_dir_all(&completed_dir).expect("remove completed dir");
    }

    #[tokio::test]
    async fn check_with_lookup_uses_durable_terminal_state_before_redispatch() {
        let title = build_title("title-1", "Paperman", MediaFacet::Movie);
        let download_id = "scryer-download:terminal";
        let identity = DownloadSubmissionIdentity {
            download_id: Some(download_id.to_string()),
        };
        let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
        submission_repo
            .record_identity_tracked_state(
                &identity,
                Some(&DownloadSourceIdentity::new(
                    Some("client-1"),
                    "nzbget",
                    "dl-1",
                )),
                TrackedDownloadState::Imported.as_str(),
                None,
                None,
            )
            .await
            .expect("record identity state");

        let app = build_app_with_download_client_configs_and_submissions(
            vec![title.clone()],
            vec![],
            vec![],
            vec![],
            Arc::new(TestDownloadClient::default()),
            Arc::new(NullDownloadClientConfigRepository),
            submission_repo,
        );
        let mut td = build_tracked_download(&title.id, "movie", "Paperman.2012.720p.WEB-DL");
        td.id = format!("download:{download_id}");
        td.client_item.download_id = Some(download_id.to_string());

        check_with_lookup(&app, &mut td, Some(&CompletedDownloadLookup::default())).await;

        assert_eq!(td.state, TrackedDownloadState::Imported);
        assert_eq!(td.status, TrackedDownloadStatus::Ok);
        assert!(td.status_messages.is_empty());
    }

    #[tokio::test]
    async fn check_with_lookup_does_not_apply_client_local_terminal_state_from_other_client() {
        let title = build_title("title-1", "Paperman", MediaFacet::Movie);
        let download_id = "10010";
        let identity = DownloadSubmissionIdentity {
            download_id: Some(download_id.to_string()),
        };
        let other_client_source = DownloadSourceIdentity::new(Some("client-2"), "nzbget", "dl-1");
        let current_client_source = DownloadSourceIdentity::new(Some("client-1"), "nzbget", "dl-1");
        let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
        submission_repo
            .record_identity_tracked_state(
                &identity,
                Some(&other_client_source),
                TrackedDownloadState::Imported.as_str(),
                None,
                None,
            )
            .await
            .expect("record other client identity state");

        let app = build_app_with_download_client_configs_and_submissions(
            vec![title.clone()],
            vec![],
            vec![],
            vec![],
            Arc::new(TestDownloadClient::default()),
            Arc::new(NullDownloadClientConfigRepository),
            submission_repo.clone(),
        );
        let mut td = build_tracked_download(&title.id, "movie", "Paperman.2012.720p.WEB-DL");
        td.id = format!("download:{download_id}");
        td.client_item.download_id = Some(download_id.to_string());

        check_with_lookup(&app, &mut td, Some(&CompletedDownloadLookup::default())).await;

        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        let current_client_state = submission_repo
            .get_identity_tracked_state(&identity, Some(&current_client_source))
            .await
            .expect("current client state lookup");
        let other_client_state = submission_repo
            .get_identity_tracked_state(&identity, Some(&other_client_source))
            .await
            .expect("other client state lookup");
        assert_eq!(
            current_client_state.as_deref(),
            Some(TrackedDownloadState::ImportBlocked.as_str())
        );
        assert_eq!(
            other_client_state.as_deref(),
            Some(TrackedDownloadState::Imported.as_str())
        );
    }

    #[tokio::test]
    async fn completed_download_reresolution_keeps_conflicting_id_only_match() {
        let existing_title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
        let parsed_title = build_title("title-2", "The Other Movie", MediaFacet::Movie);
        let app = build_app(
            vec![existing_title.clone(), parsed_title],
            vec![],
            vec![],
            vec![],
        );
        let mut td =
            build_tracked_download(&existing_title.id, "movie", "Paper.Lantern.2012.1080p");
        td.match_type = TitleMatchType::IdOnly;
        td.source_title = None;
        let completed = build_completed_download(
            "The.Other.Movie.2020.1080p.WEB-DL",
            "/tmp/does-not-matter",
            Some("movie"),
        );

        maybe_resolve_title_from_completed_download(&app, &mut td, &completed).await;

        assert_eq!(td.title_id.as_deref(), Some(existing_title.id.as_str()));
        assert_eq!(td.match_type, TitleMatchType::IdOnly);
        assert!(has_id_only_conflict(&td));
        assert_eq!(
            td.status_messages,
            vec![ID_ONLY_CONFLICT_MESSAGE.to_string()]
        );
    }

    #[tokio::test]
    async fn completed_download_reresolution_enriches_matching_id_only_title() {
        let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
        let app = build_app(vec![title.clone()], vec![], vec![], vec![]);
        let mut td = build_tracked_download(&title.id, "movie", "Paper.Lantern.2012.1080p");
        td.match_type = TitleMatchType::IdOnly;
        td.source_title = None;
        td.facet = None;
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p.WEB-DL",
            "/tmp/does-not-matter",
            Some("movie"),
        );

        maybe_resolve_title_from_completed_download(&app, &mut td, &completed).await;

        assert_eq!(td.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(td.match_type, TitleMatchType::IdOnly);
        assert_eq!(td.facet.as_deref(), Some("movie"));
        assert_eq!(
            td.source_title.as_deref(),
            Some("Paper.Lantern.2012.1080p.WEB-DL")
        );
        assert!(!has_id_only_conflict(&td));
    }

    #[tokio::test]
    async fn check_with_lookup_uses_snapshot_without_fetching_client_history() {
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p",
            std::env::temp_dir().to_string_lossy().as_ref(),
            Some("movie"),
        );
        let download_client = Arc::new(TestDownloadClient {
            completed_downloads: Arc::new(Mutex::new(vec![completed.clone()])),
            completed_download_calls: Arc::new(AtomicUsize::new(0)),
            recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        });
        let app =
            build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
        let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
        let lookup = index_completed_downloads(vec![completed]);

        check_with_lookup(&app, &mut td, Some(&lookup)).await;

        assert_eq!(
            download_client
                .completed_download_calls
                .load(Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn load_completed_download_lookup_for_items_fetches_client_history_once_per_cycle() {
        let completed = build_completed_download(
            "Paper.Lantern.2012.1080p",
            std::env::temp_dir().to_string_lossy().as_ref(),
            Some("movie"),
        );
        let download_client = Arc::new(TestDownloadClient {
            completed_downloads: Arc::new(Mutex::new(vec![completed.clone()])),
            completed_download_calls: Arc::new(AtomicUsize::new(0)),
            recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        });
        let app =
            build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
        let first = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
        let mut second =
            build_tracked_download("title-2", "movie", "Paper.Lantern.2012.1080p.REPACK");
        second.client_item.download_client_item_id = "dl-2".to_string();
        second.client_item.title_id = Some("title-2".to_string());

        let lookup = load_completed_download_lookup_for_items(
            &app,
            &[first.client_item.clone(), second.client_item.clone()],
            100,
        )
        .await
        .expect("completed lookup should load");

        assert_eq!(lookup.by_source.len(), 1);
        assert_eq!(
            download_client
                .completed_download_calls
                .load(Ordering::SeqCst),
            0
        );
        assert_eq!(
            download_client
                .recent_completed_download_calls
                .load(Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn verify_import_requires_full_season_pack_coverage() {
        let title = build_title("title-1", "Star Trek Picard", MediaFacet::Series);
        let collection = build_collection("season-2", "title-1", "2");
        let episodes = vec![
            build_episode("ep-201", "title-1", "season-2", "2", "1", None),
            build_episode("ep-202", "title-1", "season-2", "2", "2", None),
            build_episode("ep-203", "title-1", "season-2", "2", "3", None),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-201", "S02E01.mkv"),
            build_artifact("dl-1", "ep-202", "S02E02.mkv"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        );

        let parsed = crate::parse_release_metadata(
            "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        );
        assert_eq!(
            parsed.episode.as_ref().and_then(|episode| episode.season),
            Some(2)
        );

        match expected_episode_units(&app, &td).await {
            ExpectedEpisodeResolution::Resolved(expected) => assert_eq!(expected.len(), 3),
            _ => panic!("expected a resolved season-pack episode set"),
        }

        assert!(!verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_accepts_full_season_pack_coverage() {
        let title = build_title("title-1", "Star Trek Picard", MediaFacet::Series);
        let collection = build_collection("season-2", "title-1", "2");
        let episodes = vec![
            build_episode("ep-201", "title-1", "season-2", "2", "1", None),
            build_episode("ep-202", "title-1", "season-2", "2", "2", None),
            build_episode("ep-203", "title-1", "season-2", "2", "3", None),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-201", "S02E01.mkv"),
            build_artifact("dl-1", "ep-202", "S02E02.mkv"),
            build_artifact("dl-1", "ep-203", "S02E03.mkv"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_ignores_rejected_extras_when_expected_units_are_satisfied() {
        let title = build_title("title-1", "Star Trek Picard", MediaFacet::Series);
        let collection = build_collection("season-2", "title-1", "2");
        let episodes = vec![
            build_episode("ep-201", "title-1", "season-2", "2", "1", None),
            build_episode("ep-202", "title-1", "season-2", "2", "2", None),
            build_episode("ep-203", "title-1", "season-2", "2", "3", None),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-201", "S02E01.mkv"),
            build_artifact("dl-1", "ep-202", "S02E02.mkv"),
            build_artifact("dl-1", "ep-203", "S02E03.mkv"),
            build_artifact_with_result("dl-1", None, "sample.mkv", "rejected"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_resolves_absolute_episode_ranges() {
        let title = build_title("title-1", "Tidebreaker", MediaFacet::Anime);
        let collection = build_collection("season-22", "title-1", "22");
        let episodes = vec![
            build_episode("ep-1122", "title-1", "season-22", "22", "1", Some("1122")),
            build_episode("ep-1123", "title-1", "season-22", "22", "2", Some("1123")),
            build_episode("ep-1124", "title-1", "season-22", "22", "3", Some("1124")),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-1122", "1122.mkv"),
            build_artifact("dl-1", "ep-1123", "1123.mkv"),
            build_artifact("dl-1", "ep-1124", "1124.mkv"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "anime",
            "[HatSubs] Tidebreaker 1122-1124 (WEB 1080p)",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_absolute_range_requires_all_monitored_episodes_only() {
        let title = build_title("title-1", "Tidebreaker", MediaFacet::Anime);
        let collection = build_collection("season-22", "title-1", "22");
        let mut unmonitored =
            build_episode("ep-1123", "title-1", "season-22", "22", "2", Some("1123"));
        unmonitored.monitored = false;
        let episodes = vec![
            build_episode("ep-1122", "title-1", "season-22", "22", "1", Some("1122")),
            unmonitored,
            build_episode("ep-1124", "title-1", "season-22", "22", "3", Some("1124")),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-1122", "1122.mkv"),
            build_artifact("dl-1", "ep-1124", "1124.mkv"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "anime",
            "[HatSubs] Tidebreaker 1122-1124 (WEB 1080p)",
        );

        match expected_episode_units(&app, &td).await {
            ExpectedEpisodeResolution::Resolved(expected) => {
                assert_eq!(
                    expected,
                    HashSet::from(["ep-1122".to_string(), "ep-1124".to_string()])
                );
            }
            _ => panic!("expected monitored range episode set"),
        }

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_accepts_release_when_title_has_no_monitored_episodes() {
        let title = build_title("title-1", "Harbor Pals", MediaFacet::Series);
        let collection = build_collection("season-1", "title-1", "1");
        let mut first_episode = build_episode("ep-101", "title-1", "season-1", "1", "1", None);
        first_episode.monitored = false;
        let episodes = vec![first_episode];
        let artifacts = vec![build_artifact("dl-1", "ep-101", "S01E01.mkv")];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download("title-1", "series", "Harbor.Pals.S01E01.720p.WEB-DL-NTb");

        match expected_episode_units(&app, &td).await {
            ExpectedEpisodeResolution::Resolved(expected) => {
                assert_eq!(expected, HashSet::from(["ep-101".to_string()]));
            }
            _ => panic!("expected resolved episodes for an explicitly queued release"),
        }

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_absolute_range_blocks_when_monitored_episode_missing() {
        let title = build_title("title-1", "Tidebreaker", MediaFacet::Anime);
        let collection = build_collection("season-22", "title-1", "22");
        let episodes = vec![
            build_episode("ep-1122", "title-1", "season-22", "22", "1", Some("1122")),
            build_episode("ep-1123", "title-1", "season-22", "22", "2", Some("1123")),
            build_episode("ep-1124", "title-1", "season-22", "22", "3", Some("1124")),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-1122", "1122.mkv"),
            build_artifact("dl-1", "ep-1123", "1123.mkv"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "anime",
            "[HatSubs] Tidebreaker 1122-1124 (WEB 1080p)",
        );

        assert!(!verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_partial_pack_accepts_one_monitored_episode() {
        let title = build_title(
            "title-1",
            "Nightfall!! Heavy Metal, Dark Fantasy",
            MediaFacet::Anime,
        );
        let collection = build_collection("season-1", "title-1", "1");
        let episodes = vec![
            build_episode("ep-14", "title-1", "season-1", "1", "14", Some("14")),
            build_episode("ep-15", "title-1", "season-1", "1", "15", Some("15")),
        ];
        let artifacts = vec![build_artifact("dl-1", "ep-14", "S01E14.mkv")];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "anime",
            "[EMBER] NIGHTFALL‼ Heavy Metal, Dark Fantasy (2022) (Season 1 | Part 02) [1080p] [Dual Audio HEVC 10 bits WEBRip AAC] (Batch)",
        );
        match expected_episode_units(&app, &td).await {
            ExpectedEpisodeResolution::AtLeastOne(expected) => {
                assert!(expected.contains("ep-14"));
                assert!(expected.contains("ep-15"));
            }
            _ => panic!("expected partial pack monitored episode set"),
        }

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_resolves_daily_episode_by_air_date() {
        let title = build_title("title-1", "Series Title", MediaFacet::Series);
        let collection = build_collection("season-1", "title-1", "1");
        let episodes = vec![
            build_episode_with_details(
                "ep-101",
                "title-1",
                "season-1",
                EpisodeType::Standard,
                "1",
                "1",
                Some("2015-09-07"),
                None,
            ),
            build_episode_with_details(
                "ep-102",
                "title-1",
                "season-1",
                EpisodeType::Standard,
                "1",
                "2",
                Some("2015-09-08"),
                None,
            ),
        ];
        let artifacts = vec![build_artifact(
            "dl-1",
            "ep-101",
            "Series.Title.2015.09.07.mkv",
        )];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Series.Title.2015.09.07.Part.1.720p.HULU.WEBRip.AAC2.0.H.264-Sonarr",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_resolves_special_by_season_zero_number() {
        let title = build_title("title-1", "Another Anime Show", MediaFacet::Anime);
        let collection = build_collection("season-0", "title-1", "0");
        let episodes = vec![build_episode_with_details(
            "ep-special-1",
            "title-1",
            "season-0",
            EpisodeType::Ova,
            "0",
            "1",
            None,
            None,
        )];
        let artifacts = vec![build_artifact(
            "dl-1",
            "ep-special-1",
            "Another.Anime.Show.S00E01.ova.mkv",
        )];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "anime",
            "[DeadFish] Another Anime Show - 01 - OVA [BD][720p][AAC]",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_unresolved_episode_resolution_falls_back_to_successful_pass() {
        let title = build_title("title-1", "Mystery Show", MediaFacet::Series);
        let artifacts = vec![build_artifact_with_result(
            "dl-1",
            None,
            "Mystery.Show.S01E01.mkv",
            "imported",
        )];
        let app = build_app(vec![title], vec![], vec![], artifacts);
        let td = build_tracked_download("title-1", "series", "Mystery.Show.S01E01.1080p.WEB-DL");

        match expected_episode_units(&app, &td).await {
            ExpectedEpisodeResolution::Unresolved => {}
            _ => panic!("expected unresolved episodic resolution"),
        }

        assert!(verify_import(&app, &td, 1).await);
    }

    #[tokio::test]
    async fn apply_result_marks_already_present_rejection_imported_when_expected_units_are_covered()
    {
        let title = build_title("title-1", "Show", MediaFacet::Series);
        let collection = build_collection("season-1", "title-1", "1");
        let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
        let app = build_app(
            vec![title],
            vec![collection],
            vec![episode],
            vec![build_artifact_with_result(
                "dl-1",
                Some("ep-1"),
                "Show.S01E01.mkv",
                "already_present",
            )],
        );
        let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
        let result = ImportResult {
            import_id: "import-1".to_string(),
            decision: ImportDecision::Rejected,
            skip_reason: Some(ImportSkipReason::AlreadyImported),
            title_id: Some("title-1".to_string()),
            source_system: Some("nzbget".to_string()),
            source_ref: Some("dl-1".to_string()),
            source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
            source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
            dest_path: None,
            quality: None,
            episode_ids: vec![],
            file_size_bytes: None,
            link_type: None,
            error_message: Some("episode already imported".to_string()),
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        assert!(apply_import_result(&app, &mut td, result, 0).await);
        assert_eq!(td.state, TrackedDownloadState::Imported);
        assert_eq!(td.status, TrackedDownloadStatus::Ok);
        assert!(td.status_messages.is_empty());
    }

    #[tokio::test]
    async fn apply_result_does_not_verify_unresolved_identity_rejection_as_imported() {
        let title = build_title("title-1", "Show", MediaFacet::Series);
        let collection = build_collection("season-1", "title-1", "1");
        let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
        let app = build_app(
            vec![title],
            vec![collection],
            vec![episode],
            vec![build_artifact("dl-1", "ep-1", "Show.S01E01.mkv")],
        );
        let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
        let result = ImportResult {
            import_id: "import-1".to_string(),
            decision: ImportDecision::Rejected,
            skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
            title_id: Some("title-1".to_string()),
            source_system: Some("nzbget".to_string()),
            source_ref: Some("dl-1".to_string()),
            source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
            source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
            dest_path: None,
            quality: None,
            episode_ids: vec![],
            file_size_bytes: None,
            link_type: None,
            error_message: Some("download identity is unresolved".to_string()),
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        assert!(!apply_import_result(&app, &mut td, result, 0).await);
        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        assert_eq!(td.status, TrackedDownloadStatus::Warning);
    }

    #[tokio::test]
    async fn apply_result_keeps_retryable_no_video_import_pending() {
        let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let result = ImportResult {
            import_id: "import-1".to_string(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            title_id: Some("title-1".to_string()),
            source_system: Some("nzbget".to_string()),
            source_ref: Some("dl-1".to_string()),
            source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
            source_path: temp_dir.path().to_string_lossy().into_owned(),
            dest_path: None,
            quality: None,
            episode_ids: vec![],
            file_size_bytes: None,
            link_type: None,
            error_message: Some("no eligible video files found".to_string()),
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        assert!(!apply_import_result(&app, &mut td, result, 0).await);
        assert_eq!(td.state, TrackedDownloadState::ImportPending);
        assert_eq!(td.status, TrackedDownloadStatus::Warning);
        assert!(td.status_messages[0].contains("Retrying automatically"));
    }

    #[tokio::test]
    async fn check_emits_manual_interaction_notification_once() {
        let existing_dir =
            std::env::temp_dir().join(format!("scryer-completed-path-{}", Id::new().0));
        std::fs::create_dir_all(&existing_dir).expect("create temp dir");
        let completed = build_completed_download(
            "Unknown.Show.S01.Complete.1080p",
            existing_dir.to_string_lossy().as_ref(),
            Some("series"),
        );
        let download_client = Arc::new(TestDownloadClient {
            completed_downloads: Arc::new(Mutex::new(vec![CompletedDownload {
                download_client_item_id: "dl-2".to_string(),
                ..completed
            }])),
            completed_download_calls: Arc::new(AtomicUsize::new(0)),
            recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        });
        let app = build_app_with_download_client(vec![], vec![], vec![], vec![], download_client);
        let mut actor = User::new_admin("admin");
        actor.authorization = scryer_domain::UserAuthorization {
            app: scryer_domain::AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageSystemSettings,
            ]),
            default_library: scryer_domain::LibraryPermissionMask::from_permissions([
                scryer_domain::LibraryPermission::View,
            ]),
            loaded: true,
            ..Default::default()
        };
        let mut td = TrackedDownload {
            id: "nzbget:unmatched".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: DownloadQueueItem {
                id: Id::new().0,
                title_id: None,
                episode_id: None,
                title_name: "Unknown.Show.S01.Complete.1080p".to_string(),
                facet: Some("series".to_string()),
                category: None,
                client_id: "client-1".to_string(),
                client_name: "NZBGet".to_string(),
                client_type: "nzbget".to_string(),
                state: DownloadQueueState::Completed,
                progress_percent: 100,
                size_bytes: None,
                remaining_seconds: None,
                queued_at: None,
                last_updated_at: None,
                attention_required: false,
                attention_reason: None,
                download_client_item_id: "dl-2".to_string(),
                download_id: None,
                import_status: None,
                import_error_code: None,
                import_error_message: None,
                imported_at: None,
                delete_status: None,
                delete_error_message: None,
                is_scryer_origin: false,
                tracked_state: None,
                tracked_status: None,
                tracked_status_messages: vec![],
                tracked_match_type: None,
            },
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: vec![],
            title_id: None,
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Unmatched,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
            skip_reacquire_on_failure: false,
        };

        check(&app, &mut td).await;
        check(&app, &mut td).await;

        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        assert!(td.notified_manual_interaction);

        let activity = app.recent_activity(&actor, 10, 0).await.unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, ActivityKind::ImportRejected);
        assert!(activity[0].message.contains("couldn't be matched"));

        std::fs::remove_dir_all(&existing_dir).expect("remove temp dir");
    }
}
