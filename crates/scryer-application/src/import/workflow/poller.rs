pub async fn import_completed_download(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    let _import_permit = app.runtime.imports.execution_coordinator.acquire().await;
    import_completed_download_with_identity_policy(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::RequireSubmission,
        None,
        None,
    )
    .await
}

pub async fn import_completed_download_for_manual_review(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    let _import_permit = app.runtime.imports.execution_coordinator.acquire().await;
    import_completed_download_for_manual_review_with_permit(app, actor, completed).await
}

pub(crate) async fn import_completed_download_for_manual_review_with_permit(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    import_completed_download_with_identity_policy(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::AllowUnresolved,
        None,
        None,
    )
    .await
}

pub(crate) async fn import_completed_download_for_manual_review_with_title_override(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    title_id: &str,
    import_permit_held: bool,
    release_evidence: Option<&ReleaseEvidence>,
) -> AppResult<ImportResult> {
    let import = import_completed_download_with_identity_policy(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::AllowUnresolved,
        Some(title_id),
        release_evidence,
    );
    if import_permit_held {
        import.await
    } else {
        let _import_permit = app.runtime.imports.execution_coordinator.acquire().await;
        import.await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletedImportIdentityPolicy {
    RequireSubmission,
    AllowUnresolved,
}

async fn import_completed_download_with_identity_policy(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    identity_policy: CompletedImportIdentityPolicy,
    manual_title_id: Option<&str>,
    release_evidence: Option<&ReleaseEvidence>,
) -> AppResult<ImportResult> {
    let request = match prepare_completed_import_request(
        app,
        completed,
        identity_policy,
        manual_title_id,
        release_evidence,
    )
    .await?
    {
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
    release_evidence: ReleaseEvidence,
    manual_title_id: Option<String>,
    import_id: String,
    started_at: DateTime<Utc>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompletedImportRequestPayload {
    completed: CompletedDownload,
    release_evidence: ReleaseEvidence,
    #[serde(default)]
    manual_title_id: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredCompletedImportRequestPayload {
    Current(CompletedImportRequestPayload),
    Legacy(CompletedDownload),
}

enum CompletedImportProgress {
    Ready(CompletedImportRequest),
    Finished(ImportResult),
}

async fn prepare_completed_import_request(
    app: &AppUseCase,
    completed: &CompletedDownload,
    _identity_policy: CompletedImportIdentityPolicy,
    manual_title_id: Option<&str>,
    release_evidence_override: Option<&ReleaseEvidence>,
) -> AppResult<CompletedImportProgress> {
    let mut completed = completed.clone();
    remap_completed_download_for_client(app, &mut completed).await;
    let started_at = Utc::now();
    let source_identity = completed_download_identity(&completed);
    let persisted_request = app
        .services
        .workflow
        .imports
        .list_imports_for_identities(std::slice::from_ref(&source_identity))
        .await?
        .into_iter()
        .find_map(|record| {
            serde_json::from_str::<StoredCompletedImportRequestPayload>(&record.payload_json)
                .ok()
                .and_then(|payload| match payload {
                    StoredCompletedImportRequestPayload::Current(payload) => Some(payload),
                    StoredCompletedImportRequestPayload::Legacy(completed) => {
                        let _ = completed;
                        None
                    }
                })
        });
    let mut submission_resolution =
        resolve_completed_download_submission(app, &completed, None).await?;

    // A completion that cannot resolve a durable Scryer submission is an
    // observation, not another application's property. It remains eligible
    // for the normal import flow using canonical NZB evidence.
    if matches!(
        submission_resolution,
        CompletedDownloadSubmissionResolution::AmbiguousDownloadId { .. }
            | CompletedDownloadSubmissionResolution::MissingDownloadId { .. }
    ) {
        submission_resolution = CompletedDownloadSubmissionResolution::DownloaderObservation;
    }
    let release_evidence = release_evidence_override
        .cloned()
        .or_else(|| {
            persisted_request
                .as_ref()
                .map(|request| request.release_evidence.clone())
        })
        .map(Ok)
        .unwrap_or_else(|| release_evidence_for_resolution(&completed, &submission_resolution))?;
    if let Some(request) = persisted_request.as_ref()
        && matches!(release_evidence, ReleaseEvidence::ScryerSubmission { .. })
    {
        completed.parameters = request.completed.parameters.clone();
    }
    // 1. DEDUP CHECK
    if completed_download_already_imported_for_current_attempt(app, &completed, &submission_resolution)
        .await?
    {
        let result = ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::AlreadyImported),
            ..base_completed_import_result("", &completed, &release_evidence, started_at)
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
    let resolved_manual_title_id = manual_title_id
        .map(str::to_string)
        .or_else(|| {
            persisted_request
                .as_ref()
                .and_then(|request| request.manual_title_id.clone())
        });
    let import_id = app
        .services
        .workflow
        .imports
        .queue_import_request_with_identity(
            source_identity,
            import_type.as_str().to_string(),
            serde_json::to_string(&CompletedImportRequestPayload {
                completed: completed.clone(),
                release_evidence: release_evidence.clone(),
                manual_title_id: resolved_manual_title_id.clone(),
            })
            .unwrap_or_default(),
            completed_download_import_identity_for_resolution(&completed, &submission_resolution),
        )
        .await?;

    Ok(CompletedImportProgress::Ready(CompletedImportRequest {
        completed,
        release_evidence,
        manual_title_id: resolved_manual_title_id,
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
                &request.release_evidence,
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
        &request.release_evidence,
        request.manual_title_id.as_deref(),
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
        ..base_completed_import_result(
            &request.import_id,
            &request.completed,
            &request.release_evidence,
            request.started_at,
        )
    };
    let result_json = serde_json::to_string(&result).ok();
    let _ = app
        .update_import_status_and_notify(&request.import_id, ImportStatus::Failed, result_json)
        .await;
    Ok(result)
}
