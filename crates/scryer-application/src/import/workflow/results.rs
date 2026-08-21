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

    let payload: StoredCompletedImportRequestPayload = serde_json::from_str(&record.payload_json)
        .map_err(|e| AppError::Repository(format!("failed to deserialize import payload: {e}")))?;
    let (mut completed, persisted) = match payload {
        StoredCompletedImportRequestPayload::Current(payload) => {
            (payload.completed.clone(), Some(payload))
        }
        StoredCompletedImportRequestPayload::Legacy(completed) => (completed, None),
    };
    remap_completed_download_for_client(app, &mut completed).await;

    // A live submission row is authoritative over what the failed attempt
    // persisted (an operator may have reassigned the download since); the
    // persisted evidence is the fallback for a lost row or a transient lookup
    // failure only.
    let ImportProvenance {
        completed,
        release_evidence,
        target_title_id,
        ..
    } = resolve_import_provenance(
        app,
        completed,
        ImportProvenanceRequest {
            identity_policy: CompletedImportIdentityPolicy::RequireSubmission,
            queue_item: None,
            requested_target_title_id: None,
            release_evidence_override: None,
            persisted: persisted.as_ref(),
            tolerate_lookup_failure: true,
        },
    )
    .await?;

    let authorization_title_id = release_evidence
        .title_id()
        .map(str::to_string)
        .or_else(|| target_title_id.clone())
        .or_else(|| {
            extract_parameter(&completed.parameters, "*scryer_title_id")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    if let Some(title_id) = authorization_title_id
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
    match run_import(
        app,
        actor,
        import_id,
        &completed,
        &release_evidence,
        target_title_id.as_deref(),
        started_at,
        password,
    )
    .await
    {
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
                ..base_completed_import_result(
                    import_id,
                    &completed,
                    &release_evidence,
                    started_at,
                )
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Failed, result_json)
                .await?;
            Ok(result)
        }
    }
}
/// Remove a terminal download's entry from its client, subject to the
/// seeding-aware gate.
///
/// Removing a torrent's entry stops it seeding even with `remove_data: false`,
/// so for torrent-protocol items an `Imported` state is no longer sufficient
/// on its own — the gate has to agree that the seeding obligation is
/// discharged. Failed and Ignored downloads are deliberately *not* gated:
/// blocklist and retry must never wait on seeding (Sonarr's rule).
///
/// Once a removal is agreed, a torrent's payload goes with the entry
/// (`remove_data`, Sonarr's `deleteData: true`); see the call site for which
/// states qualify and which keep today's behavior.
#[expect(
    clippy::too_many_arguments,
    reason = "terminal cleanup carries client identity, routing scope, state, and the seeding gate's view of the client entry"
)]
async fn reconcile_terminal_download_cleanup(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    library_id: Option<&str>,
    facet: Option<&MediaFacet>,
    state: TrackedDownloadState,
    present_in_client: bool,
    // The freshest seeding observation the caller holds, or `None` to have the
    // gate look one up from the published tracked-download snapshot.
    observation: Option<crate::seeding_gate::TorrentSeedingObservation>,
    // The reconcile tick's shared reads, or `None` for callers outside a tick
    // (manual import), which take the per-row path.
    cache: Option<&TerminalCleanupTickCache>,
) -> TerminalDownloadCleanup {
    let client_id = client_id.trim();
    let routing_key = if client_id.is_empty() {
        client_type
    } else {
        client_id
    };

    let should_remove = match state {
        TrackedDownloadState::Imported | TrackedDownloadState::ImportedSeeding => match facet {
            Some(facet) => {
                should_remove_completed_download_cached(app, library_id, facet, routing_key, cache)
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
        return TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::NotConfigured);
    }

    // Carried past the gate for the seeding history events: the gate consumes
    // the observation, and the release event has to report the ratio and seed
    // time the decision was actually taken on.
    let observed_ratio = observation
        .as_ref()
        .and_then(|observation| observation.seed_ratio);
    let observed_seed_time_seconds = observation
        .as_ref()
        .and_then(|observation| observation.seed_time_seconds);
    let report = |reason: &'static str, action: Option<SeedingReleaseAction>| SeedingGateReport {
        reason,
        action,
        seed_ratio: observed_ratio,
        seed_time_seconds: observed_seed_time_seconds,
    };

    let mut seeding_report = None;
    if state.counts_as_imported() {
        let key = crate::seeding_gate::SeedGoalLookupKey {
            client_id: client_id.to_string(),
            client_type: client_type.trim().to_string(),
            client_item_id: download_client_item_id.trim().to_string(),
            info_hash: crate::normalize_torrent_info_hash(Some(download_client_item_id)),
        };
        let decision = crate::seeding_gate::evaluate_seeding_gate_with(
            app,
            &key,
            present_in_client,
            observation,
            cache.map(TerminalCleanupTickCache::goal_batch),
        )
        .await;
        match decision.outcome {
            crate::seeding_gate::SeedingGateOutcome::NotApplicable => {}
            crate::seeding_gate::SeedingGateOutcome::Vanished => {
                return TerminalDownloadCleanup::gated(
                    TerminalDownloadCleanupOutcome::AlreadyGone,
                    report(decision.reason, Some(SeedingReleaseAction::Vanished)),
                );
            }
            crate::seeding_gate::SeedingGateOutcome::HandedOff => {
                tracing::info!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    reason = decision.reason,
                    "post-import handoff: leaving the client entry untouched and no longer managing this torrent"
                );
                return TerminalDownloadCleanup::gated(
                    TerminalDownloadCleanupOutcome::HandedOff,
                    report(decision.reason, Some(SeedingReleaseAction::HandedOff)),
                );
            }
            crate::seeding_gate::SeedingGateOutcome::Hold => {
                tracing::debug!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    reason = decision.reason,
                    "seeding gate is holding a torrent entry after import"
                );
                return TerminalDownloadCleanup::gated(
                    TerminalDownloadCleanupOutcome::HeldForSeeding,
                    report(decision.reason, None),
                );
            }
            crate::seeding_gate::SeedingGateOutcome::Released { action } => match action {
                scryer_domain::SeedGoalMetAction::RemoveEntry => {
                    seeding_report =
                        Some(report(decision.reason, Some(SeedingReleaseAction::Removed)));
                }
                scryer_domain::SeedGoalMetAction::StopSeeding => {
                    let stopped = stop_seeding_for_terminal_download(
                        app,
                        client_id,
                        client_type,
                        download_client_item_id,
                        decision.reason,
                    )
                    .await;
                    return TerminalDownloadCleanup::gated(
                        TerminalDownloadCleanupOutcome::SeedingEntryKept,
                        report(decision.reason, Some(stopped)),
                    );
                }
                scryer_domain::SeedGoalMetAction::Keep => {
                    tracing::info!(
                        client_id,
                        client_type,
                        download_client_item_id,
                        reason = decision.reason,
                        "seeding goal met; keeping the client entry per profile policy"
                    );
                    return TerminalDownloadCleanup::gated(
                        TerminalDownloadCleanupOutcome::SeedingEntryKept,
                        report(decision.reason, Some(SeedingReleaseAction::Kept)),
                    );
                }
            },
        }
    }

    let is_history = matches!(
        state,
        TrackedDownloadState::Imported
            | TrackedDownloadState::ImportedSeeding
            | TrackedDownloadState::Failed
            | TrackedDownloadState::Ignored
    );

    // Sonarr removes both a completed-and-imported download and a failed one
    // with `RemoveItem(item, deleteData: true)` (DownloadEventHub
    // .RemoveFromDownloadClient). Match it for torrents: reaching this line in
    // `Imported`/`ImportedSeeding` means the gate released the entry with
    // `RemoveEntry` — the seeding obligation is discharged and nothing is left
    // to protect — and `Failed` is a download nobody will import. A hardlinked
    // import keeps the library file either way; a copy import would otherwise
    // leave the client's copy behind with no owner.
    //
    // `Ignored` keeps today's behavior on purpose: the operator told Scryer to
    // stop tracking the download, not to delete what it downloaded. So do the
    // first-party usenet clients, whose delete semantics are their own — both
    // are documented as deferred.
    let remove_data = matches!(
        state,
        TrackedDownloadState::Imported
            | TrackedDownloadState::ImportedSeeding
            | TrackedDownloadState::Failed
    ) && crate::seeding_gate::client_type_is_torrent(app, client_type);

    let delete_result = if client_id.is_empty() {
        app.services
            .integrations
            .download_client
            .delete_queue_item_for_client(
                client_type,
                download_client_item_id,
                is_history,
                remove_data,
            )
            .await
    } else {
        app.services
            .integrations
            .download_client
            .delete_queue_item_for_client_id(
                client_id,
                download_client_item_id,
                is_history,
                remove_data,
            )
            .await
    };

    let outcome = match delete_result {
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
    };

    // The removal may have failed after the gate released the entry; report
    // what actually happened rather than the intent.
    let seeding = seeding_report.map(|report| SeedingGateReport {
        action: match outcome {
            TerminalDownloadCleanupOutcome::Removed => Some(SeedingReleaseAction::Removed),
            TerminalDownloadCleanupOutcome::AlreadyGone => Some(SeedingReleaseAction::Vanished),
            _ => Some(SeedingReleaseAction::Kept),
        },
        ..report
    });
    TerminalDownloadCleanup { outcome, seeding }
}
/// `SeedGoalMetAction::StopSeeding`: leave the entry in the client but stop it
/// uploading.
///
/// Pause is the only stop control the download-client port exposes
/// (`DownloadControlAction::Pause` in the plugin SDK), and for a torrent that
/// has finished downloading, paused *is* stopped seeding. A client that does
/// not support pause degrades to `Keep`: the entry stays and nothing is
/// removed, which is the safe direction.
async fn stop_seeding_for_terminal_download(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    reason: &'static str,
) -> SeedingReleaseAction {
    let paused = if client_id.is_empty() {
        app.services
            .integrations
            .download_client
            .pause_queue_item(download_client_item_id)
            .await
    } else {
        app.services
            .integrations
            .download_client
            .pause_queue_item_for_client(client_id, download_client_item_id)
            .await
    };

    match paused {
        Ok(()) => {
            tracing::info!(
                client_id,
                client_type,
                download_client_item_id,
                reason,
                "seeding goal met; paused the torrent per profile policy"
            );
            SeedingReleaseAction::Paused
        }
        Err(error) => {
            tracing::warn!(
                client_id,
                client_type,
                download_client_item_id,
                reason,
                error = %error,
                "seeding goal met but this client cannot stop the torrent; keeping the entry untouched"
            );
            SeedingReleaseAction::Kept
        }
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

async fn skip_reason_for_import_check_rejection(
    app: &AppUseCase,
    code: &str,
    dest_path: &Path,
) -> AppResult<ImportSkipReason> {
    if code == "duplicate_file" {
        let stored_dest_path = path_to_stored_string(dest_path);
        let cataloged = app
            .services
            .library
            .media_files
            .get_media_file_by_path(&stored_dest_path)
            .await?
            .is_some();
        if !cataloged {
            return Ok(ImportSkipReason::DuplicateFile);
        }
    }
    Ok(skip_reason_for_import_check_code(code))
}

async fn finalize_import_source_cleanup(
    app: &AppUseCase,
    import_mode: scryer_domain::ImportMode,
    file_result: &scryer_domain::ImportFileResult,
    final_dest_path: &Path,
) -> AppResult<scryer_domain::ImportStrategy> {
    if import_mode != scryer_domain::ImportMode::Move {
        return Ok(file_result.strategy);
    }

    let guard = file_result.source_cleanup.clone().ok_or_else(|| {
        AppError::Repository(format!(
            "move import did not return a source cleanup guard for {}",
            file_result.source_path.display()
        ))
    })?;

    app.services
        .workflow
        .file_importer
        .remove_import_source_after_verified_import(guard, final_dest_path)
        .await?;

    Ok(scryer_domain::ImportStrategy::Move)
}
/// Sonarr's phase rule, not an error-string catalogue: an import that was
/// approved but failed while *executing* (`ImportDecision::Failed` — locked or
/// still-growing files, IO, network shares, DB hiccups) is transient by
/// construction and is re-attempted automatically at a capped cadence.
/// Decision-phase outcomes (rejections, policy skips, unmatched identity) are
/// permanent and stay blocked for review. Two exceptions in each direction:
/// a password-protected archive can never succeed without operator input, and
/// disk-full / permission-denied skips are environmental and clear on their own.
/// The message allowlist remains as belt-and-braces for Scryer's own transient
/// markers that surface on non-`Failed` decisions.
pub(crate) fn completed_import_result_is_retryable(result: &ImportResult) -> bool {
    match result.decision {
        ImportDecision::Failed => {
            result.skip_reason != Some(ImportSkipReason::PasswordRequired)
        }
        _ => {
            matches!(
                result.skip_reason,
                Some(ImportSkipReason::DiskFull | ImportSkipReason::PermissionDenied)
            ) || result
                .error_message
                .as_deref()
                .is_some_and(completed_import_error_message_is_retryable)
        }
    }
}

fn completed_import_status_for_result(
    result: &ImportResult,
    fallback_status: ImportStatus,
) -> ImportStatus {
    if result.skip_reason == Some(ImportSkipReason::NoVideoFiles)
        || completed_import_result_is_retryable(result)
    {
        ImportStatus::Pending
    } else {
        fallback_status
    }
}
