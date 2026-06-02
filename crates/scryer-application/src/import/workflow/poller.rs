pub async fn import_completed_download(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    let app = app.clone();
    let actor = actor.clone();
    let completed = completed.clone();
    import_completed_download_on_large_stack(app, actor, completed).await
}

const COMPLETED_IMPORT_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

async fn import_completed_download_on_large_stack(
    app: AppUseCase,
    actor: User,
    completed: CompletedDownload,
) -> AppResult<ImportResult> {
    let runtime = tokio::runtime::Handle::current();
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();

    std::thread::Builder::new()
        .name("scryer-completed-import".to_string())
        .stack_size(COMPLETED_IMPORT_THREAD_STACK_BYTES)
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(import_completed_download_inner(
                    &app, &actor, &completed,
                ))
            }))
            .unwrap_or_else(|panic| {
                Err(AppError::Repository(format!(
                    "completed download import worker panicked: {}",
                    panic_payload_message(panic.as_ref())
                )))
            });
            let _ = result_tx.send(result);
        })
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to spawn completed download import worker: {error}"
            ))
        })?;

    result_rx.await.map_err(|error| {
        AppError::Repository(format!(
            "completed download import worker exited before returning a result: {error}"
        ))
    })?
}

fn panic_payload_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    "unknown panic payload".to_string()
}

async fn import_completed_download_inner(
    app: &AppUseCase,
    actor: &User,
    completed: &CompletedDownload,
) -> AppResult<ImportResult> {
    let mut completed = completed.clone();
    remap_completed_download_for_client(app, &mut completed).await;
    let started_at = Utc::now();
    let source_ref = &completed.download_client_item_id;
    let source_identity = completed_download_identity(&completed);

    // 1. DEDUP CHECK
    if app
        .services
        .workflow
        .imports
        .is_already_imported(&source_identity)
        .await?
    {
        return Ok(ImportResult {
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::AlreadyImported),
            ..base_completed_import_result("", &completed, started_at)
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
            source_identity,
            import_type.as_str().to_string(),
            serde_json::to_string(&completed).unwrap_or_default(),
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
            ..base_completed_import_result(&import_id, &completed, started_at)
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
    match run_import(app, actor, &import_id, &completed, started_at, None).await {
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
                ..base_completed_import_result(&import_id, &completed, started_at)
            };
            let result_json = serde_json::to_string(&result).ok();
            let _ = app
                .update_import_status_and_notify(&import_id, ImportStatus::Failed, result_json)
                .await;
            Ok(result)
        }
    }
}
