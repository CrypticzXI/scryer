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

pub(crate) async fn import_completed_download_with_release_evidence(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
) -> AppResult<ImportResult> {
    let _import_permit = app.runtime.imports.execution_coordinator.acquire().await;
    import_completed_download_with_identity_policy(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::RequireSubmission,
        None,
        Some(release_evidence),
    )
    .await
}

/// Import a downloader observation into the title the tracked download already
/// validated for it (a parse match the completed-check proved, or an operator
/// assignment). Without this the import would re-derive the title from a
/// context-free parse of the release name and could land elsewhere or fail to
/// match at all. The target is persisted with the request so retries honor it;
/// a durable Scryer submission found for the download still wins over it.
pub(crate) async fn import_completed_download_with_target_title(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    target_title_id: &str,
) -> AppResult<ImportResult> {
    let _import_permit = app.runtime.imports.execution_coordinator.acquire().await;
    import_completed_download_with_identity_policy(
        app,
        actor,
        completed,
        CompletedImportIdentityPolicy::RequireSubmission,
        Some(target_title_id),
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

/// Who chose the requested target title, which decides how a disagreement with a
/// durable Scryer submission is settled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletedImportIdentityPolicy {
    /// Automatic import: the target (if any) is the tracked download's
    /// validated title. A Scryer submission is authoritative over it — the
    /// submission wins and the disagreement is logged, never an error.
    RequireSubmission,
    /// Manual review: the target is an operator's explicit choice. It is
    /// passed through as-is so `resolve_completed_import_target` rejects a
    /// choice outside the durable Scryer submission title.
    AllowUnresolved,
}

async fn import_completed_download_with_identity_policy(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
    identity_policy: CompletedImportIdentityPolicy,
    target_title_id: Option<&str>,
    release_evidence: Option<&ReleaseEvidence>,
) -> AppResult<ImportResult> {
    let request = match prepare_completed_import_request(
        app,
        completed,
        identity_policy,
        target_title_id,
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
    target_title_id: Option<String>,
    import_id: String,
    started_at: DateTime<Utc>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompletedImportRequestPayload {
    completed: CompletedDownload,
    release_evidence: ReleaseEvidence,
    /// The title the import must land in when the release evidence carries no
    /// Scryer identity: the tracked download's validated title for automatic
    /// imports, or the operator's chosen title for manual review. Persisted so
    /// a retry after the tracked download is gone still lands in it.
    #[serde(default, alias = "manual_title_id")]
    target_title_id: Option<String>,
}

/// Where the release evidence for an import attempt came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletedImportEvidenceSource {
    /// The caller resolved it itself (tracked auto-import, manual selection).
    Override,
    /// A live `download_submissions` row for the completed download.
    FreshRow,
    /// No row exists any more; the evidence an earlier attempt persisted.
    Persisted,
    /// Nothing durable: the client-reported observation.
    FreshObservation,
}

struct CompletedImportEvidenceInputs<'a> {
    identity_policy: CompletedImportIdentityPolicy,
    /// The live submission lookup, or `None` when it failed transiently.
    fresh_resolution: Option<&'a CompletedDownloadSubmissionResolution>,
    release_evidence_override: Option<&'a ReleaseEvidence>,
    persisted_release_evidence: Option<&'a ReleaseEvidence>,
    persisted_target_title_id: Option<&'a str>,
    requested_target_title_id: Option<&'a str>,
    completed: &'a CompletedDownload,
}

struct SelectedCompletedImportEvidence {
    release_evidence: ReleaseEvidence,
    target_title_id: Option<String>,
    source: CompletedImportEvidenceSource,
}

fn non_empty_title_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Chooses the release evidence and target title for an import attempt.
///
/// A live `download_submissions` row is authoritative over anything an earlier
/// attempt persisted: after an operator reassignment the row is rewritten (to
/// an orphan row naming the new title), and a retry that replayed the stale
/// persisted `ScryerSubmission` would import into the old title. Persisted
/// evidence is used only when the row is gone (lost) or the live lookup failed
/// transiently. The persisted target follows the same rule: a live row that
/// names a title (a Scryer submission or an operator assignment) supersedes it.
fn select_completed_import_evidence(
    inputs: CompletedImportEvidenceInputs<'_>,
) -> SelectedCompletedImportEvidence {
    let fresh_match = inputs
        .fresh_resolution
        .and_then(|resolution| match resolution {
            CompletedDownloadSubmissionResolution::Matched(matched) => Some(matched.as_ref()),
            _ => None,
        });

    let (release_evidence, source) = if let Some(evidence) = inputs.release_evidence_override {
        (evidence.clone(), CompletedImportEvidenceSource::Override)
    } else if let (Some(resolution), Some(_)) = (inputs.fresh_resolution, fresh_match) {
        (
            release_evidence_for_resolution(inputs.completed, resolution),
            CompletedImportEvidenceSource::FreshRow,
        )
    } else if let Some(persisted) = inputs.persisted_release_evidence {
        (persisted.clone(), CompletedImportEvidenceSource::Persisted)
    } else {
        (
            ReleaseEvidence::from_completed_observation(inputs.completed),
            CompletedImportEvidenceSource::FreshObservation,
        )
    };

    let requested_target_title_id = non_empty_title_id(inputs.requested_target_title_id);
    let target_title_id = match release_evidence.title_id() {
        Some(submission_title_id) => match (inputs.identity_policy, requested_target_title_id) {
            (_, None) => None,
            (CompletedImportIdentityPolicy::AllowUnresolved, Some(requested)) => {
                Some(requested.to_string())
            }
            (CompletedImportIdentityPolicy::RequireSubmission, Some(requested)) => {
                if requested != submission_title_id {
                    tracing::warn!(
                        client_id = %inputs.completed.client_id,
                        client_type = %inputs.completed.client_type,
                        download_client_item_id = %inputs.completed.download_client_item_id,
                        requested_title_id = requested,
                        submission_title_id,
                        "import: requested target title disagrees with the durable Scryer submission; importing into the submission title"
                    );
                }
                None
            }
        },
        None => requested_target_title_id
            .map(str::to_string)
            .or_else(|| {
                // An operator assignment rewrites the row as an orphan naming
                // the chosen title; that live choice outranks a stale target.
                fresh_match
                    .and_then(|matched| non_empty_title_id(Some(&matched.submission.title_id)))
                    .map(str::to_string)
            })
            .or_else(|| {
                non_empty_title_id(inputs.persisted_target_title_id).map(str::to_string)
            })
            .or_else(|| {
                // Last resort, when both the submission row and the tracked
                // state are gone: `*scryer_title_id` is Scryer's own stamp,
                // written only at Scryer add time (NZBGet PP parameter) or by
                // `with_tracked_metadata`, and 0.18.12 honored it directly.
                // It never outranks a Scryer submission title (handled above)
                // or any of the earlier target sources.
                extract_parameter(&inputs.completed.parameters, SCRYER_TITLE_ID_PARAM)
                    .as_deref()
                    .and_then(|value| non_empty_title_id(Some(value)))
                    .map(str::to_string)
            }),
    };

    SelectedCompletedImportEvidence {
        release_evidence,
        target_title_id,
        source,
    }
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
    identity_policy: CompletedImportIdentityPolicy,
    target_title_id: Option<&str>,
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
    let SelectedCompletedImportEvidence {
        release_evidence,
        target_title_id: resolved_target_title_id,
        source: evidence_source,
    } = select_completed_import_evidence(CompletedImportEvidenceInputs {
        identity_policy,
        fresh_resolution: Some(&submission_resolution),
        release_evidence_override,
        persisted_release_evidence: persisted_request
            .as_ref()
            .map(|request| &request.release_evidence),
        persisted_target_title_id: persisted_request
            .as_ref()
            .and_then(|request| request.target_title_id.as_deref()),
        requested_target_title_id: target_title_id,
        completed: &completed,
    });
    // The Scryer origin parameters follow the evidence: a live Scryer submission
    // stamps its own (authoritative, idempotent with what the tracked and
    // untracked callers already applied); evidence replayed from an earlier
    // attempt because the row is gone brings that attempt's parameters along.
    if let CompletedDownloadSubmissionResolution::Matched(matched) = &submission_resolution
        && submission_has_scryer_origin(&matched.submission)
    {
        completed.parameters =
            authoritative_scryer_origin_parameters(&completed.parameters, &matched.submission);
    } else if let Some(request) = persisted_request.as_ref()
        && evidence_source == CompletedImportEvidenceSource::Persisted
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
                target_title_id: resolved_target_title_id.clone(),
            })
            .unwrap_or_default(),
            completed_download_import_identity_for_resolution(&completed, &submission_resolution),
        )
        .await?;

    Ok(CompletedImportProgress::Ready(CompletedImportRequest {
        completed,
        release_evidence,
        target_title_id: resolved_target_title_id,
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
        request.target_title_id.as_deref(),
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
