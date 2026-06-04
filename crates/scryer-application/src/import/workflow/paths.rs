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
                "import: failed to load download client config for remote path mapping"
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
                "import: failed to parse remote path mappings"
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DownloadSubmissionMatchKind {
    RequestId,
    Fingerprint,
    LegacyClientItemId,
}

#[derive(Clone, Debug)]
struct CompletedDownloadSubmissionMatch {
    submission: DownloadSubmission,
    kind: DownloadSubmissionMatchKind,
    identity: Option<DownloadSubmissionIdentity>,
}

#[derive(Clone, Debug)]
enum CompletedDownloadSubmissionResolution {
    Matched(CompletedDownloadSubmissionMatch),
    Foreign,
    MissingDurableIdentity {
        identity: DownloadSubmissionIdentity,
    },
    AmbiguousFingerprint { fingerprint: String, matches: usize },
    ConflictingIdentity {
        request_id: Option<String>,
        fingerprint: Option<String>,
    },
    IncompatibleLegacyClientItem { submission: DownloadSubmission },
}

fn completed_download_observed_identity(completed: &CompletedDownload) -> DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_request_id: completed.download_request_id.as_deref(),
        download_fingerprint: completed.download_fingerprint.as_deref(),
        parameters: &completed.parameters,
        info_hash_hint: None,
    })
}

fn download_queue_item_observed_identity(item: &DownloadQueueItem) -> DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_request_id: item.download_request_id.as_deref(),
        download_fingerprint: item.download_fingerprint.as_deref(),
        parameters: &[],
        info_hash_hint: None,
    })
}

fn identity_field(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn find_completed_download_for_queue_item(
    completed_downloads: &[CompletedDownload],
    item: &DownloadQueueItem,
) -> Option<CompletedDownload> {
    let item_identity = download_queue_item_observed_identity(item);
    if let Some(request_id) = identity_field(item_identity.download_request_id.as_deref()) {
        let matches = completed_downloads
            .iter()
            .filter(|completed| {
                let completed_identity = completed_download_observed_identity(completed);
                identity_field(completed_identity.download_request_id.as_deref())
                    == Some(request_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return None;
        }
        if let Some(completed) =
            crate::download_identity::coalesce_completed_downloads_by_release_observation(&matches)
        {
            return Some(completed);
        }
        if matches.len() > 1 {
            tracing::warn!(
                source_ref = %item.download_client_item_id,
                request_id,
                "import: queue item request id matched multiple completed downloads"
            );
        }
        return None;
    }
    if let Some(fingerprint) = identity_field(item_identity.download_fingerprint.as_deref()) {
        let matches = completed_downloads
            .iter()
            .filter(|completed| {
                let completed_identity = completed_download_observed_identity(completed);
                identity_field(completed_identity.download_fingerprint.as_deref())
                    == Some(fingerprint)
            })
            .cloned()
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return None;
        }
        if let Some(completed) =
            crate::download_identity::coalesce_completed_downloads_by_release_observation(&matches)
        {
            return Some(completed);
        }
        if matches.len() > 1 {
            tracing::warn!(
                source_ref = %item.download_client_item_id,
                fingerprint,
                "import: queue item fingerprint matched multiple completed downloads"
            );
        }
        return None;
    }

    completed_downloads.iter().find(|completed| {
        completed_download_identity(completed)
            == DownloadSourceIdentity::new(
                Some(item.client_id.as_str()),
                &item.client_type,
                &item.download_client_item_id,
            )
    }).cloned()
}

fn download_submission_identity_is_empty(identity: &DownloadSubmissionIdentity) -> bool {
    identity
        .download_request_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
        && identity
            .download_fingerprint
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
}

fn completed_download_has_durable_identity(completed: &CompletedDownload) -> bool {
    !download_submission_identity_is_empty(&completed_download_observed_identity(completed))
}

fn submission_source_identity(submission: &DownloadSubmission) -> DownloadSourceIdentity {
    DownloadSourceIdentity::from_submission(submission)
}

fn submission_matches_completed_download_legacy_evidence(
    submission: &DownloadSubmission,
    completed: &CompletedDownload,
    item: Option<&DownloadQueueItem>,
) -> bool {
    let completed_title_id = extract_parameter(&completed.parameters, "*scryer_title_id");
    let title_id = completed_title_id
        .as_deref()
        .or_else(|| item.and_then(|item| item.title_id.as_deref()));
    let episode_id = item.and_then(|item| item.episode_id.as_deref());
    crate::download_identity::download_submission_is_compatible_with_evidence(
        submission,
        crate::download_identity::DownloadSubmissionCompatibilityEvidence {
            title_id,
            episode_id,
            source_title: Some(completed.name.as_str()),
        },
    )
}

async fn resolve_completed_download_submission(
    app: &AppUseCase,
    completed: &CompletedDownload,
    item: Option<&DownloadQueueItem>,
) -> AppResult<CompletedDownloadSubmissionResolution> {
    let observed_identity = completed_download_observed_identity(completed);
    let request_id = observed_identity.download_request_id.clone();
    let fingerprint = observed_identity.download_fingerprint.clone();

    let request_submissions = if let Some(request_id) = request_id.as_deref() {
        app.services
            .workflow
            .download_submissions
            .list_by_request_id(request_id)
            .await?
    } else {
        Vec::new()
    };

    let fingerprint_submissions = if let Some(fingerprint) = fingerprint.as_deref() {
        app.services
            .workflow
            .download_submissions
            .list_by_fingerprint(fingerprint)
            .await?
    } else {
        Vec::new()
    };

    if !request_submissions.is_empty() {
        let Some(submission) =
            crate::download_identity::coalesce_download_submissions_by_release_attempt(
                &request_submissions,
            )
        else {
            return Ok(CompletedDownloadSubmissionResolution::ConflictingIdentity {
                request_id,
                fingerprint,
            });
        };
        let stored_identity = submission_identity_for_submission(app, &submission).await?;
        if !submission_identity_is_compatible_with_observed(&stored_identity, &observed_identity) {
            return Ok(CompletedDownloadSubmissionResolution::ConflictingIdentity {
                request_id,
                fingerprint,
            });
        }
        if !fingerprint_submissions.is_empty() {
            let all_same_release = fingerprint_submissions
                .iter()
                .all(|candidate| {
                    crate::download_identity::coalesce_download_submissions_by_release_attempt(&[
                        submission.clone(),
                        candidate.clone(),
                    ])
                    .is_some()
                });
            if !all_same_release {
                return Ok(CompletedDownloadSubmissionResolution::ConflictingIdentity {
                    request_id,
                    fingerprint,
                });
            }
        }
        return matched_completed_download_submission(
            app,
            submission,
            DownloadSubmissionMatchKind::RequestId,
            &observed_identity,
        )
        .await;
    }

    if let Some(fingerprint) = fingerprint.as_deref() {
        if fingerprint_submissions.is_empty() {
            return Ok(
                CompletedDownloadSubmissionResolution::MissingDurableIdentity {
                    identity: observed_identity,
                },
            );
        }
        if let Some(submission) =
            crate::download_identity::coalesce_download_submissions_by_release_attempt(
                &fingerprint_submissions,
            )
        {
            return matched_completed_download_submission(
                app,
                submission,
                DownloadSubmissionMatchKind::Fingerprint,
                &observed_identity,
            )
            .await;
        }
        return Ok(
        CompletedDownloadSubmissionResolution::AmbiguousFingerprint {
            fingerprint: fingerprint.to_string(),
            matches: fingerprint_submissions.len(),
        },
        );
    }

    if !download_submission_identity_is_empty(&observed_identity) {
        return Ok(
            CompletedDownloadSubmissionResolution::MissingDurableIdentity {
                identity: observed_identity,
            },
        );
    }

    let submission = app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id(&DownloadSourceIdentity::new(
            Some(completed.client_id.as_str()),
            &completed.client_type,
            &completed.download_client_item_id,
        ))
        .await?;

    let Some(submission) = submission else {
        return Ok(CompletedDownloadSubmissionResolution::Foreign);
    };

    if submission_matches_completed_download_legacy_evidence(&submission, completed, item) {
        return matched_completed_download_submission(
            app,
            submission,
            DownloadSubmissionMatchKind::LegacyClientItemId,
            &observed_identity,
        )
        .await;
    }

    Ok(CompletedDownloadSubmissionResolution::IncompatibleLegacyClientItem { submission })
}

async fn submission_identity_for_submission(
    app: &AppUseCase,
    submission: &DownloadSubmission,
) -> AppResult<DownloadSubmissionIdentity> {
    Ok(app
        .services
        .workflow
        .download_submissions
        .get_submission_identity(&submission_source_identity(submission))
        .await?
        .unwrap_or_default())
}

async fn matched_completed_download_submission(
    app: &AppUseCase,
    submission: DownloadSubmission,
    kind: DownloadSubmissionMatchKind,
    observed_identity: &DownloadSubmissionIdentity,
) -> AppResult<CompletedDownloadSubmissionResolution> {
    let stored_identity = submission_identity_for_submission(app, &submission).await?;
    let identity = if download_submission_identity_is_empty(&stored_identity) {
        (!download_submission_identity_is_empty(observed_identity)).then_some(observed_identity.clone())
    } else {
        Some(stored_identity)
    };
    Ok(CompletedDownloadSubmissionResolution::Matched(
        CompletedDownloadSubmissionMatch {
            submission,
            kind,
            identity,
        },
    ))
}

fn submission_identity_is_compatible_with_observed(
    stored: &DownloadSubmissionIdentity,
    observed: &DownloadSubmissionIdentity,
) -> bool {
    if let Some(observed_request_id) = observed
        .download_request_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && let Some(stored_request_id) = stored
            .download_request_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        && stored_request_id != observed_request_id
    {
        return false;
    }

    if let Some(observed_fingerprint) = observed
        .download_fingerprint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && let Some(stored_fingerprint) = stored
            .download_fingerprint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        && stored_fingerprint != observed_fingerprint
    {
        return false;
    }

    true
}

async fn completed_download_already_imported_for_current_attempt(
    app: &AppUseCase,
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
) -> AppResult<bool> {
    if matches!(
        resolution,
        CompletedDownloadSubmissionResolution::AmbiguousFingerprint { .. }
            | CompletedDownloadSubmissionResolution::ConflictingIdentity { .. }
            | CompletedDownloadSubmissionResolution::MissingDurableIdentity { .. }
            | CompletedDownloadSubmissionResolution::IncompatibleLegacyClientItem { .. }
            | CompletedDownloadSubmissionResolution::Foreign
    ) {
        return Ok(false);
    }

    if completed_download_terminal_state_for_resolution(app, resolution).await?
        == Some(TrackedDownloadState::Imported)
    {
        return Ok(true);
    }

    let CompletedDownloadSubmissionResolution::Matched(matched) = resolution else {
        return Ok(false);
    };

    if let Some(identity) = matched.identity.as_ref()
        && app
            .services
            .workflow
            .imports
            .is_already_imported_by_submission_identity(identity)
            .await?
    {
        return Ok(true);
    }

    if matched.kind == DownloadSubmissionMatchKind::LegacyClientItemId
        && !completed_download_has_durable_identity(completed)
    {
        return app
            .services
            .workflow
            .imports
            .is_already_imported(&submission_source_identity(&matched.submission))
            .await;
    }

    Ok(false)
}

async fn completed_download_terminal_state_for_resolution(
    app: &AppUseCase,
    resolution: &CompletedDownloadSubmissionResolution,
) -> AppResult<Option<TrackedDownloadState>> {
    let CompletedDownloadSubmissionResolution::Matched(matched) = resolution else {
        return Ok(None);
    };

    Ok(app
        .services
        .workflow
        .download_submissions
        .get_tracked_state(&submission_source_identity(&matched.submission))
        .await?
        .and_then(|value| TrackedDownloadState::from_str_opt(&value)))
}

fn completed_download_import_identity_for_resolution(
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
) -> Option<DownloadSubmissionIdentity> {
    let observed_identity = completed_download_observed_identity(completed);
    if !download_submission_identity_is_empty(&observed_identity) {
        return Some(observed_identity);
    }

    match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched) => matched.identity.clone(),
        _ => None,
    }
}

async fn block_completed_download_identity_for_manual_review(
    app: &AppUseCase,
    completed: &CompletedDownload,
    reason: &str,
    detail: &str,
) {
    tracing::warn!(
        client_id = completed.client_id.as_str(),
        client_type = completed.client_type.as_str(),
        download_client_item_id = completed.download_client_item_id.as_str(),
        reason,
        detail,
        "import: download identity is unresolved; blocking import for manual review"
    );
    let observed_identity = completed_download_observed_identity(completed);
    if !download_submission_identity_is_empty(&observed_identity) {
        if let Err(error) = app
            .services
            .workflow
            .download_submissions
            .record_identity_tracked_state(
                &observed_identity,
                Some(&completed_download_identity(completed)),
                TrackedDownloadState::ImportBlocked.as_str(),
                Some(reason),
                Some(detail),
            )
            .await
        {
            tracing::warn!(
                error = %error,
                client_id = completed.client_id.as_str(),
                client_type = completed.client_type.as_str(),
                download_client_item_id = completed.download_client_item_id.as_str(),
                reason,
                "failed to persist durable download identity manual-review state"
            );
        }
        return;
    }

    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .update_tracked_state(
            &completed_download_identity(completed),
            TrackedDownloadState::ImportBlocked.as_str(),
        )
        .await
    {
        tracing::warn!(
            error = %error,
            client_id = completed.client_id.as_str(),
            client_type = completed.client_type.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            reason,
            "failed to persist download identity manual-review state"
        );
    }
}

async fn block_download_queue_item_identity_for_manual_review(
    app: &AppUseCase,
    item: &DownloadQueueItem,
    reason: &str,
    detail: &str,
) {
    let observed_identity = download_queue_item_observed_identity(item);
    if download_submission_identity_is_empty(&observed_identity) {
        return;
    }
    tracing::warn!(
        client_id = item.client_id.as_str(),
        client_type = item.client_type.as_str(),
        download_client_item_id = item.download_client_item_id.as_str(),
        reason,
        detail,
        "import: queue item durable identity is unresolved; blocking import for manual review"
    );
    let source_identity = DownloadSourceIdentity::new(
        Some(item.client_id.as_str()),
        &item.client_type,
        &item.download_client_item_id,
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
            client_id = item.client_id.as_str(),
            client_type = item.client_type.as_str(),
            download_client_item_id = item.download_client_item_id.as_str(),
            reason,
            "failed to persist durable queue item manual-review state"
        );
    }
}

/// Attempts to import completed items from the current queue/history snapshot.
/// Returns the set of `download_client_item_id`s that were conclusively processed
/// (imported, failed permanently, or intentionally ignored). Temporary defer
/// conditions (e.g. no matching CompletedDownload yet, empty dest_dir) are NOT
/// included so they can be retried on the next snapshot.
pub async fn try_import_completed_downloads(
    app: &AppUseCase,
    actor: &User,
    items: &[DownloadQueueItem],
) -> HashSet<String> {
    // TODO: increase to 600 (10 minutes) for production — large NAS copies can take a while
    match app
        .services
        .workflow
        .imports
        .recover_stale_processing_imports(120)
        .await
    {
        Ok(recovered) if recovered > 0 => {
            tracing::warn!(recovered, "recovered stale processing imports → failed");
            app.emit_import_recovery_completed_event(
                Some(actor.id.clone()),
                i64::try_from(recovered).unwrap_or(i64::MAX),
            )
            .await;
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to recover stale processing imports");
        }
        _ => {}
    }

    let completed_items: Vec<&DownloadQueueItem> = items
        .iter()
        .filter(|item| item.state == DownloadQueueState::Completed)
        .filter(|item| {
            item.import_status.is_none() || item.import_status == Some(ImportStatus::Failed)
        })
        .collect();

    if completed_items.is_empty() {
        return HashSet::new();
    }

    let mut processed_ids: HashSet<String> = HashSet::new();

    tracing::info!(
        count = completed_items.len(),
        items = %completed_items.iter().map(|i| format!("{}({})", i.title_name, i.download_client_item_id)).collect::<Vec<_>>().join(", "),
        "import: found completed items to evaluate"
    );

    let completed_downloads = match app
        .services
        .integrations
        .download_client
        .list_completed_downloads()
        .await
    {
        Ok(downloads) => {
            tracing::debug!(
                count = downloads.len(),
                ids = %downloads.iter().map(|d| d.download_client_item_id.as_str()).collect::<Vec<_>>().join(", "),
                "import: fetched completed downloads from client"
            );
            downloads
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to fetch completed downloads for import");
            return HashSet::new();
        }
    };

    for item in completed_items {
        let source_ref = &item.download_client_item_id;

        // Find the matching CompletedDownload
        let completed = match find_completed_download_for_queue_item(&completed_downloads, item) {
            Some(completed) => completed,
            None => {
                tracing::debug!(
                    source_ref = %source_ref,
                    title = %item.title_name,
                    "import: no matching CompletedDownload from client history (item may still be processing or status != Completed)"
                );
                if !download_submission_identity_is_empty(&download_queue_item_observed_identity(item))
                {
                    block_download_queue_item_identity_for_manual_review(
                        app,
                        item,
                        "missing_completed_history_identity",
                        "completed queue item carried durable identity but completed history did not contain a compatible identity",
                    )
                    .await;
                }
                continue;
            }
        };

        // Only auto-import downloads that originated from scryer.
        // NZBGet embeds *scryer_title_id via PPParameters. SABnzbd has no
        // equivalent, so we fall back to the download_submissions table which
        // records the (title_id, facet) at grab time.
        let submission_resolution =
            match resolve_completed_download_submission(app, &completed, Some(item)).await {
                Ok(resolution) => resolution,
                Err(error) => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        error = %error,
                        "import: download_submissions lookup failed"
                    );
                    continue;
                }
            };

        if let CompletedDownloadSubmissionResolution::AmbiguousFingerprint {
            fingerprint,
            matches,
        } = &submission_resolution
        {
            block_completed_download_identity_for_manual_review(
                app,
                &completed,
                "ambiguous_fingerprint",
                &format!(
                    "download fingerprint matched {matches} submissions: {fingerprint}"
                ),
            )
            .await;
            continue;
        }
        if let CompletedDownloadSubmissionResolution::MissingDurableIdentity { identity } =
            &submission_resolution
        {
            block_completed_download_identity_for_manual_review(
                app,
                &completed,
                "missing_durable_identity",
                &format!(
                    "request_id={:?} fingerprint={:?}",
                    identity.download_request_id, identity.download_fingerprint
                ),
            )
            .await;
            continue;
        }
        if let CompletedDownloadSubmissionResolution::ConflictingIdentity {
            request_id,
            fingerprint,
        } = &submission_resolution
        {
            block_completed_download_identity_for_manual_review(
                app,
                &completed,
                "conflicting_durable_identity",
                &format!("request_id={request_id:?} fingerprint={fingerprint:?}"),
            )
            .await;
            continue;
        }

        let completed = if has_scryer_origin(&completed.parameters) {
            completed.clone()
        } else {
            match &submission_resolution {
                CompletedDownloadSubmissionResolution::Matched(matched)
                    if submission_has_scryer_origin(&matched.submission) =>
                {
                    let collection_id =
                        matched.submission.scope.collection_id().map(str::to_string);
                    let mut patched = completed.clone();
                    merge_scryer_origin_parameters(
                        &mut patched.parameters,
                        matched.submission.title_id.clone(),
                        matched.submission.facet.clone(),
                        collection_id,
                    );
                    patched
                }
                CompletedDownloadSubmissionResolution::Matched(_) => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        client_type = %completed.client_type,
                        "import: ignoring stub download_submissions row without scryer origin metadata"
                    );
                    processed_ids.insert(source_ref.clone());
                    continue;
                }
                CompletedDownloadSubmissionResolution::IncompatibleLegacyClientItem {
                    submission,
                } => {
                    tracing::warn!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        client_id = %completed.client_id,
                        client_type = %completed.client_type,
                        download_client_item_id = %completed.download_client_item_id,
                        submission_title_id = %submission.title_id,
                        "download_client_item_id_reused"
                    );
                    continue;
                }
                CompletedDownloadSubmissionResolution::Foreign => {
                    tracing::debug!(
                        source_ref = %source_ref,
                        title = %item.title_name,
                        client_type = %completed.client_type,
                        "import: no scryer origin — not in parameters or download_submissions table"
                    );
                    processed_ids.insert(source_ref.clone());
                    continue;
                }
                CompletedDownloadSubmissionResolution::AmbiguousFingerprint { .. }
                | CompletedDownloadSubmissionResolution::MissingDurableIdentity { .. }
                | CompletedDownloadSubmissionResolution::ConflictingIdentity { .. } => {
                    unreachable!()
                }
            }
        };

        let already_imported = match completed_download_already_imported_for_current_attempt(
            app,
            &completed,
            &submission_resolution,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(error = %error, source_ref = %source_ref, "import dedup check failed");
                continue;
            }
        };

        if already_imported {
            tracing::debug!(
                source_ref = %source_ref,
                title = %item.title_name,
                "import: treating already-imported download as terminal imported for cleanup"
            );
            let cleanup = reconcile_terminal_download_cleanup_for_completed(
                app,
                &completed,
                TrackedDownloadState::Imported,
            )
            .await;
            if terminal_download_cleanup_is_complete(cleanup) {
                processed_ids.insert(source_ref.clone());
            }
            continue;
        }

        if let Ok(Some(state)) =
            completed_download_terminal_state_for_resolution(app, &submission_resolution).await
            && matches!(
                state,
                TrackedDownloadState::Imported | TrackedDownloadState::Failed
            )
        {
            tracing::debug!(
                source_ref = %source_ref,
                title = %item.title_name,
                state = state.as_str(),
                "import: retrying terminal cleanup from persisted tracked state"
            );
            let cleanup =
                reconcile_terminal_download_cleanup_for_completed(app, &completed, state).await;
            if terminal_download_cleanup_is_complete(cleanup) {
                processed_ids.insert(source_ref.clone());
            }
            continue;
        }

        // Skip if dest_dir is empty for fresh import attempts.
        if completed.dest_dir.is_empty() {
            tracing::info!(
                source_ref = %source_ref,
                title = %item.title_name,
                "import: skipping download with empty dest_dir"
            );
            continue;
        }

        let facet_label = extract_parameter(&completed.parameters, "*scryer_facet")
            .unwrap_or_else(|| "unknown".to_string());
        tracing::info!(
            source_ref = %source_ref,
            title = %item.title_name,
            dest_dir = %completed.dest_dir,
            facet = %facet_label,
            "import: triggering import for completed download"
        );
        let import_start = std::time::Instant::now();
        match import_completed_download(app, actor, &completed).await {
            Ok(result) => {
                if matches!(
                    result.decision,
                    ImportDecision::Failed | ImportDecision::Rejected
                ) {
                    tracing::warn!(
                        decision = ?result.decision,
                        title_id = ?result.title_id,
                        error_message = ?result.error_message,
                        source_path = %result.source_path,
                        "import failed for {}",
                        completed.name
                    );
                } else if matches!(result.decision, ImportDecision::Unmatched) {
                    tracing::debug!(
                        decision = ?result.decision,
                        error_message = ?result.error_message,
                        source_path = %result.source_path,
                        "import unmatched for {}",
                        completed.name
                    );
                } else {
                    tracing::info!(
                        decision = ?result.decision,
                        title_id = ?result.title_id,
                        dest_path = ?result.dest_path,
                        "import completed for {}",
                        completed.name
                    );
                }
                metrics::counter!("scryer_imports_total", "decision" => result.decision.as_str(), "facet" => facet_label.clone()).increment(1);
                metrics::histogram!("scryer_import_duration_seconds", "facet" => facet_label)
                    .record(import_start.elapsed().as_secs_f64());

                if let Some(state) = terminal_tracked_state_for_import_result(&result) {
                    persist_completed_download_tracked_state(
                        app,
                        &completed,
                        &submission_resolution,
                        state,
                    )
                    .await;
                    let cleanup =
                        reconcile_terminal_download_cleanup_for_completed(app, &completed, state)
                            .await;
                    if terminal_download_cleanup_is_complete(cleanup) {
                        processed_ids.insert(source_ref.clone());
                    }
                } else {
                    processed_ids.insert(source_ref.clone());
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    name = %completed.name,
                    "import failed for completed download"
                );
                metrics::counter!("scryer_imports_total", "decision" => "error", "facet" => facet_label.clone()).increment(1);
                metrics::histogram!("scryer_import_duration_seconds", "facet" => facet_label)
                    .record(import_start.elapsed().as_secs_f64());
                processed_ids.insert(source_ref.clone());
            }
        }
    }

    processed_ids
}
fn completed_import_result_is_retryable(result: &ImportResult) -> bool {
    if matches!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles))
        && Path::new(&result.source_path).exists()
    {
        return true;
    }

    result
        .error_message
        .as_deref()
        .is_some_and(completed_import_error_message_is_retryable)
}
// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(crate) struct ImportPathSettings {
    pub(crate) media_root: String,
    pub(crate) rename_template: String,
    pub(crate) folder_template: String,
}
async fn persist_title_folder_path_if_missing(app: &AppUseCase, title: &Title, folder_path: &Path) {
    let has_folder_path = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if has_folder_path {
        return;
    }
    let _ = app
        .services
        .catalog
        .titles
        .set_folder_path(&title.id, &path_to_stored_string(folder_path))
        .await;
}
#[cfg(test)]
fn sanitized_title_folder_component(raw: &str) -> String {
    let sanitized = sanitize_filesystem_component(raw);
    if sanitized.is_empty() {
        "untitled".to_string()
    } else {
        sanitized
    }
}
/// Recursively find all video files under `dir`, optionally filtering out samples.
///
/// `dir` is usually a directory, but SABnzbd sometimes reports the file path
/// itself as the completed download's `storage` field. If the path has a video
/// extension and cannot be opened as a directory, we treat it as a single-file
/// result.
pub(crate) fn find_video_files(dir: &Path, filter_samples: bool) -> AppResult<Vec<PathBuf>> {
    if std::fs::read_dir(dir).is_err() && is_video_file(dir) {
        tracing::info!(
            path = %dir.display(),
            "download path is a video file, not a directory"
        );
        return Ok((!filter_samples || !is_sample_file(dir))
            .then_some(dir.to_path_buf())
            .into_iter()
            .collect());
    }

    let walked = crate::filesystem_walk::FilesystemWalker::new()
        .skip_unreadable_subdirectories()
        .walk(dir)?;

    Ok(walked
        .into_iter()
        .flat_map(|entry| entry.files.into_iter())
        .filter(|path| is_video_file(path))
        .filter(|path| !filter_samples || !is_sample_file(path))
        .collect())
}
pub(crate) fn pick_largest_file(files: &[PathBuf]) -> AppResult<PathBuf> {
    files
        .iter()
        .max_by_key(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0))
        .cloned()
        .ok_or_else(|| AppError::Repository("no files to pick from".to_string()))
}
fn parsed_release_from_file_stem(path: &Path) -> ParsedReleaseMetadata {
    let fallback = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or(fallback);
    normalize_release_title_signal(parse_release_metadata(stem.as_str()))
}
fn parsed_usable_release_from_file_stem(path: &Path) -> Option<ParsedReleaseMetadata> {
    let fallback = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or(fallback);
    parse_usable_release_title(stem.as_str())
}
fn parsed_release_from_folder_name(path: &Path) -> Option<ParsedReleaseMetadata> {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| parse_release_metadata(value.as_str()))
        .map(normalize_release_title_signal)
}
fn parsed_release_from_parent_folder(path: &Path) -> Option<ParsedReleaseMetadata> {
    path.parent().and_then(parsed_release_from_folder_name)
}
fn parsed_usable_release_from_parent_folder(path: &Path) -> Option<ParsedReleaseMetadata> {
    parsed_release_from_parent_folder(path).filter(has_usable_release_title_signal)
}
fn title_evidence_candidates_from_video_files(
    video_files: &[PathBuf],
) -> Vec<ParsedReleaseMetadata> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for video_file in video_files {
        let candidate = parsed_usable_release_from_file_stem(video_file)
            .or_else(|| parsed_usable_release_from_parent_folder(video_file));

        if let Some(candidate) = candidate {
            let key = candidate.raw_title.to_ascii_uppercase();
            if seen.insert(key) {
                candidates.push(candidate);
            }
        }
    }

    candidates
}
