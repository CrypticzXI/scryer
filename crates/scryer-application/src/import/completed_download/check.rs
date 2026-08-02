use super::lookup::{
    apply_download_id_state, completed_download_source_identity, download_id_tracked_state,
    find_completed_download, maybe_resolve_title_from_completed_download,
    observed_completed_download_identity, observed_queue_item_identity, queue_item_source_identity,
};
use super::path_state::{CompletedDownloadPathState, evaluate_completed_download_path};
use super::*;

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

    let waiting_for_completed_history_retry = td.state == TrackedDownloadState::ImportPending
        && td.waiting_for_completed_history
        && completed_lookup.is_some();

    // Only process if still in a check-eligible state.
    if td.state != TrackedDownloadState::Downloading
        && td.state != TrackedDownloadState::ImportBlocked
        && !waiting_for_completed_history_retry
    {
        return;
    }
    if waiting_for_completed_history_retry {
        td.state = TrackedDownloadState::Downloading;
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
        if completed_lookup.is_some_and(|lookup| !lookup.is_exhaustive()) {
            mark_waiting_for_completed_history(td, completed_lookup);
            return;
        }
        if !crate::download_submission_identity_is_empty(&queue_identity) {
            if missing_completed_history_is_retryable(td, &queue_identity) {
                mark_waiting_for_completed_history(td, completed_lookup);
                return;
            }
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
    td.waiting_for_completed_history = false;

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
    td.waiting_for_completed_history = false;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
}

fn mark_waiting_for_completed_history(
    td: &mut TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
) {
    tracing::warn!(
        id = %td.id,
        item_id = %td.client_item.download_client_item_id,
        download_id = ?td.client_item.download_id,
        match_type = ?td.match_type,
        is_scryer_origin = td.client_item.is_scryer_origin,
        lookup_exhaustive = completed_lookup.is_some_and(CompletedDownloadLookup::is_exhaustive),
        "check: completed download not found in client history, will retry"
    );
    td.state = TrackedDownloadState::ImportPending;
    td.waiting_for_completed_history = true;
    td.status = TrackedDownloadStatus::Warning;
    td.status_messages = vec![
        "Completed download is waiting for client history to expose a matching item; retrying."
            .to_string(),
    ];
}

fn missing_completed_history_is_retryable(
    td: &TrackedDownload,
    identity: &crate::DownloadSubmissionIdentity,
) -> bool {
    durable_global_download_id(identity)
        && (td.client_item.is_scryer_origin
            || matches!(
                td.match_type,
                TitleMatchType::Submission | TitleMatchType::ClientParameter
            ))
}

fn durable_global_download_id(identity: &crate::DownloadSubmissionIdentity) -> bool {
    let Some(download_id) = identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    download_id.starts_with("scryer-download:")
        || (matches!(download_id.len(), 40 | 64)
            && download_id.chars().all(|ch| ch.is_ascii_hexdigit()))
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

pub(super) fn has_id_only_conflict(td: &TrackedDownload) -> bool {
    td.status_messages
        .iter()
        .any(|message| message == ID_ONLY_CONFLICT_MESSAGE)
}

async fn set_state_to_import_blocked(app: &AppUseCase, td: &mut TrackedDownload) {
    let was_blocked = td.state == TrackedDownloadState::ImportBlocked;
    td.state = TrackedDownloadState::ImportBlocked;
    td.waiting_for_completed_history = false;
    td.status = TrackedDownloadStatus::Warning;

    if !was_blocked {
        crate::tracked_downloads::persist_tracked_download_state_marker(
            app,
            td,
            TrackedDownloadState::ImportBlocked,
            Some("import_blocked_pre_import"),
            td.status_messages.first().map(String::as_str),
        )
        .await;
    }

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
    if !td.status_messages.iter().any(|message| message == detail) {
        td.status_messages.clear();
        td.status_messages.push(detail.to_string());
    }
    // set_state_to_import_blocked writes the generic blocked marker; record
    // the specific identity reason afterwards so it wins the upsert.
    set_state_to_import_blocked(app, td).await;
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
}
