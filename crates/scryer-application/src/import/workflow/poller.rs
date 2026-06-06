pub async fn import_completed_download(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    let request = match prepare_completed_import_request(app, completed).await? {
        CompletedImportProgress::Ready(request) => request,
        CompletedImportProgress::Finished(result) => return Ok(result),
    };
    let request = match validate_completed_import_source_and_mark_processing(app, request).await? {
        CompletedImportProgress::Ready(request) => request,
        CompletedImportProgress::Finished(result) => return Ok(result),
    };

    execute_completed_import(app, actor, request).await
}

struct CompletedImportRequest {
    completed: CompletedDownload,
    import_id: String,
    started_at: DateTime<Utc>,
}

enum CompletedImportProgress {
    Ready(CompletedImportRequest),
    Finished(ImportResult),
}

async fn prepare_completed_import_request(
    app: &AppUseCase,
    completed: &CompletedDownload,
) -> AppResult<CompletedImportProgress> {
    let mut completed = completed.clone();
    remap_completed_download_for_client(app, &mut completed).await;
    let started_at = Utc::now();
    let source_identity = completed_download_identity(&completed);
    let submission_resolution =
        resolve_completed_download_submission(app, &completed, None).await?;

    if let CompletedDownloadSubmissionResolution::AmbiguousDownloadId {
        download_id,
        matches,
    } = &submission_resolution
    {
        block_completed_download_identity_for_manual_review(
            app,
            &completed,
            "ambiguous_download_id",
            &format!("download id matched {matches} submissions: {download_id}"),
        )
        .await;
        let result = ImportResult {
            decision: ImportDecision::Rejected,
            skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
            error_message: Some(format!(
                "DownloadId matched {matches} submissions; manual review is required: {download_id}"
            )),
            ..base_completed_import_result("", &completed, started_at)
        };
        return Ok(CompletedImportProgress::Finished(result));
    }
    if let CompletedDownloadSubmissionResolution::MissingDownloadId { identity } =
        &submission_resolution
    {
        block_completed_download_identity_for_manual_review(
            app,
            &completed,
            "missing_download_id",
            &format!("download_id={:?}", identity.download_id),
        )
        .await;
        let result = ImportResult {
            decision: ImportDecision::Rejected,
            skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
            error_message: Some(format!(
                "Download has a DownloadId but no matching Scryer submission; manual review is required: download_id={:?}",
                identity.download_id
            )),
            ..base_completed_import_result("", &completed, started_at)
        };
        return Ok(CompletedImportProgress::Finished(result));
    }

    // 1. DEDUP CHECK
    if completed_download_already_imported_for_current_attempt(app, &submission_resolution).await?
    {
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::AlreadyImported),
            ..base_completed_import_result("", &completed, started_at)
        };
        return Ok(CompletedImportProgress::Finished(result));
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
        .queue_import_request_with_identity(
            source_identity,
            import_type.as_str().to_string(),
            serde_json::to_string(&completed).unwrap_or_default(),
            completed_download_import_identity_for_resolution(&completed, &submission_resolution),
        )
        .await?;

    Ok(CompletedImportProgress::Ready(CompletedImportRequest {
        completed,
        import_id,
        started_at,
    }))
}

async fn validate_completed_import_source_and_mark_processing(
    app: &AppUseCase,
    request: CompletedImportRequest,
) -> AppResult<CompletedImportProgress> {
    // If the source directory no longer exists, the files were already moved
    // by a previous import (possibly under a different source_ref). Mark as
    // skipped so the poller never retries this entry.
    let source_ref = &request.completed.download_client_item_id;
    let source_path = std::path::Path::new(&request.completed.dest_dir);
    if !source_path.exists() {
        tracing::debug!(
            source_ref,
            dest_dir = %request.completed.dest_dir,
            "import: source directory no longer exists, no files to import"
        );
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            ..base_completed_import_result(
                &request.import_id,
                &request.completed,
                request.started_at,
            )
        };
        let result_json = serde_json::to_string(&result).ok();
        let _ = app
            .update_import_status_and_notify(&request.import_id, ImportStatus::Skipped, result_json)
            .await;
        return Ok(CompletedImportProgress::Finished(result));
    }

    // Mark as processing
    app.update_import_status_and_notify(&request.import_id, ImportStatus::Processing, None)
        .await?;

    Ok(CompletedImportProgress::Ready(request))
}

async fn execute_completed_import(
    app: &AppUseCase,
    actor: &User,
    request: CompletedImportRequest,
) -> AppResult<ImportResult> {
    // From here on, any error must update the import record to "failed" rather than
    // propagating via `?`. Otherwise the record stays "processing" indefinitely.
    match Box::pin(run_import(
        app,
        actor,
        &request.import_id,
        &request.completed,
        request.started_at,
        None,
    ))
    .await
    {
        Ok(result) => Ok(result),
        Err(error) => finalize_completed_import_error(app, &request, error).await,
    }
}

async fn finalize_completed_import_error(
    app: &AppUseCase,
    request: &CompletedImportRequest,
    error: AppError,
) -> AppResult<ImportResult> {
    let skip_reason = if crate::archive_extractor::is_password_required_error(&error) {
        Some(ImportSkipReason::PasswordRequired)
    } else {
        None
    };
    let result = ImportResult {
        decision: ImportDecision::Failed,
        skip_reason,
        error_message: Some(error.to_string()),
        ..base_completed_import_result(&request.import_id, &request.completed, request.started_at)
    };
    let result_json = serde_json::to_string(&result).ok();
    let _ = app
        .update_import_status_and_notify(&request.import_id, ImportStatus::Failed, result_json)
        .await;
    Ok(result)
}
