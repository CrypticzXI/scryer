/// Retry a previously failed import, optionally with an archive password.
pub async fn retry_failed_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    password: Option<&str>,
) -> AppResult<ImportResult> {
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

    let mut completed: CompletedDownload = serde_json::from_str(&record.payload_json)
        .map_err(|e| AppError::Repository(format!("failed to deserialize import payload: {e}")))?;
    remap_completed_download_for_client(app, &mut completed).await;

    if let Some(title_id) = extract_parameter(&completed.parameters, "*scryer_title_id")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        let title = app
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        app.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
    } else if app
        .authorized_library_ids(
            actor,
            None,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?
        .is_empty()
    {
        return Err(AppError::Unauthorized(
            "You do not have access to this library".to_string(),
        ));
    }

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
async fn reconcile_terminal_download_cleanup(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    library_id: Option<&str>,
    facet: Option<&MediaFacet>,
    state: TrackedDownloadState,
) -> TerminalDownloadCleanupOutcome {
    let client_id = client_id.trim();
    let routing_key = if client_id.is_empty() {
        client_type
    } else {
        client_id
    };

    let should_remove = match state {
        TrackedDownloadState::Imported => match facet {
            Some(facet) => {
                app.should_remove_completed_download(library_id, facet, routing_key)
                    .await
            }
            None => false,
        },
        TrackedDownloadState::Failed => match facet {
            Some(facet) => {
                app.should_remove_failed_download(library_id, facet, routing_key)
                    .await
            }
            None => false,
        },
        TrackedDownloadState::Ignored => true,
        _ => false,
    };

    if !should_remove {
        return TerminalDownloadCleanupOutcome::NotConfigured;
    }

    let is_history = matches!(
        state,
        TrackedDownloadState::Imported
            | TrackedDownloadState::Failed
            | TrackedDownloadState::Ignored
    );

    let delete_result = if client_id.is_empty() {
        app.services
            .integrations
            .download_client
            .delete_queue_item_for_client(client_type, download_client_item_id, is_history)
            .await
    } else {
        app.services
            .integrations
            .download_client
            .delete_queue_item_for_client_id(client_id, download_client_item_id, is_history)
            .await
    };

    match delete_result {
        Ok(()) => TerminalDownloadCleanupOutcome::Removed,
        Err(error) => {
            if !terminal_download_item_is_still_visible(
                app,
                client_id,
                client_type,
                download_client_item_id,
                is_history,
            )
            .await
            {
                tracing::debug!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    error = %error,
                    "download item was already absent after delete error"
                );
                TerminalDownloadCleanupOutcome::AlreadyGone
            } else {
                tracing::warn!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    error = %error,
                    "failed to remove terminal download from client"
                );
                TerminalDownloadCleanupOutcome::RetryableFailure
            }
        }
    }
}
fn terminal_tracked_state_for_import_result(result: &ImportResult) -> Option<TrackedDownloadState> {
    match result.decision {
        ImportDecision::Imported => Some(TrackedDownloadState::Imported),
        ImportDecision::Failed | ImportDecision::Rejected => Some(TrackedDownloadState::Failed),
        ImportDecision::Skipped
            if result.skip_reason == Some(ImportSkipReason::AlreadyImported) =>
        {
            Some(TrackedDownloadState::Imported)
        }
        _ => None,
    }
}
fn skip_reason_for_import_check_code(code: &str) -> ImportSkipReason {
    match code {
        "duplicate_file" => ImportSkipReason::AlreadyImported,
        "insufficient_disk_space" => ImportSkipReason::DiskFull,
        "invalid_extension" | "sample_file" | "sample_directory" => {
            ImportSkipReason::PolicyMismatch
        }
        _ => ImportSkipReason::PolicyMismatch,
    }
}
fn completed_import_status_for_result(
    result: &ImportResult,
    fallback_status: ImportStatus,
) -> ImportStatus {
    if completed_import_result_is_retryable(result) {
        ImportStatus::Pending
    } else {
        fallback_status
    }
}
