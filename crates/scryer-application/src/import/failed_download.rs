//! FailedDownloadHandler — failure detection and processing (plan 055).
//!
//! check(): detects downloads that failed in the client or are encrypted.
//! process_failed(): records the failure, emits events, and optionally reacquires.

use scryer_domain::{DownloadQueueState, TrackedDownloadState, TrackedDownloadStatus};

use crate::AppUseCase;
use crate::acquisition_workflow::{DownloadFailureContext, FailureHandlingOutcome};
use crate::tracked_downloads::TrackedDownload;

/// Detect failed downloads during the poll cycle.
///
/// Called for downloads that have not reached a terminal tracked state. If the
/// client reports failure, transitions to FailedPending before import can run.
pub fn check(td: &mut TrackedDownload) {
    // Only process if in a check-eligible state.
    if !matches!(
        td.state,
        TrackedDownloadState::Downloading
            | TrackedDownloadState::ImportPending
            | TrackedDownloadState::ImportBlocked
    ) {
        return;
    }

    if td.client_item.state != DownloadQueueState::Failed {
        return;
    }

    // Check if scryer has context to handle this failure.
    if td.title_id.is_none() || td.title_id.as_deref() == Some("") {
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages.clear();
        td.warn("Download failed but isn't linked to a scryer title. Skipping automatic failure handling.");
        return;
    }

    td.state = TrackedDownloadState::FailedPending;
    td.status = TrackedDownloadStatus::Error;
    td.status_messages.clear();
}

/// Process a download in FailedPending state.
///
/// Records the failure, emits activity events, and optionally triggers
/// a re-search for the same title.
pub async fn process_failed(app: &AppUseCase, td: &mut TrackedDownload) {
    if td.state != TrackedDownloadState::FailedPending {
        return;
    }

    let failure_reason = td
        .client_item
        .attention_reason
        .as_deref()
        .unwrap_or("Failed download detected");

    tracing::warn!(
        id = %td.id,
        title_id = ?td.title_id,
        reason = failure_reason,
        "download failed - processing failure"
    );

    let outcome = crate::acquisition_workflow::process_download_failure(
        app,
        DownloadFailureContext {
            wanted_item: None,
            title_id: td.title_id.clone(),
            client_id: td.client_id.clone(),
            client_type: td.client_type.clone(),
            client_name: Some(td.client_item.client_name.clone()),
            client_item_id: td.client_item.download_client_item_id.clone(),
            release_title: td
                .source_title
                .clone()
                .unwrap_or_else(|| td.client_item.title_name.clone()),
            reason: failure_reason.to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: td.skip_reacquire_on_failure,
        },
        None,
    )
    .await;
    if td.skip_reacquire_on_failure
        && !matches!(outcome, FailureHandlingOutcome::RecordedNoReacquire)
    {
        td.status_messages.push(
            "Failure was recorded, but Scryer could not confirm that reacquisition was disabled."
                .to_string(),
        );
    }
    crate::fail_active_manual_import_for_source(app, td, failure_reason).await;

    td.state = TrackedDownloadState::Failed;
}
