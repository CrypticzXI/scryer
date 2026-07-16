/// Snapshot of the download client's current queue and recent history,
/// fetched once per polling cycle to avoid repeated API calls.
pub(crate) struct DownloadClientSnapshot {
    /// Lowercase title names of items currently queued or downloading.
    active_titles: std::collections::HashSet<String>,
    /// Download client item IDs of items currently queued/downloading.
    /// Used for episode-level dedup (check by submission ID, not title name).
    active_client_ids: std::collections::HashSet<String>,
    /// Raw native item ID counts for legacy rows that predate configured
    /// client IDs. Used only when the raw ID is unique in the snapshot.
    active_raw_item_id_counts: std::collections::HashMap<String, usize>,
    /// Download client item IDs of items that completed successfully.
    completed_client_ids: std::collections::HashSet<String>,
    completed_raw_item_id_counts: std::collections::HashMap<String, usize>,
    /// Failed history items keyed by download client job ID (NZBGet NZBID,
    /// SABnzbd nzo_id, Weaver job UUID). Matched against `download_submissions`
    /// table to find which scryer title a failed download belongs to.
    failed_by_download_id: std::collections::HashMap<String, FailedDownloadSnapshot>,
    /// True when `list_queue()` errored while building this snapshot. An
    /// unobservable queue must be treated as "possibly active" for automatic
    /// grabs so a transient client outage cannot cause a blind double-submit
    /// (the Scryer-shaped analogue of Sonarr's download-client backoff).
    queue_listing_failed: bool,
    /// True when `list_history()` errored while building this snapshot. Failure
    /// detection reads only history, so an unobservable history simply yields
    /// no failures rather than acting on an empty map.
    history_listing_failed: bool,
}
fn download_client_item_identity(client_id: Option<&str>, item_id: &str) -> String {
    let client_id = client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    if client_id.is_empty() {
        return item_id.to_string();
    }

    format!("{client_id}:{item_id}")
}
#[derive(Clone, Debug)]
pub(crate) struct FailedDownloadSnapshot {
    reason: String,
    download_client_item_id: String,
    client_id: String,
    client_name: Option<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct DownloadFailureContext {
    pub wanted_item: Option<AcquisitionScopeState>,
    pub title_id: Option<String>,
    pub client_id: String,
    pub client_type: String,
    pub client_name: Option<String>,
    pub client_item_id: String,
    pub release_title: String,
    pub reason: String,
    pub remove_from_client_if_configured: bool,
    pub skip_reacquire: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureHandlingOutcome {
    RecoveredFromStandby,
    RequeuedFreshSearch,
    RequeuedDeferred,
    RecordedOnly,
    RecordedNoReacquire,
    AlreadyHandled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StandbyRecoveryOutcome {
    Recovered,
    Deferred,
    Exhausted,
}
// Canonical owner for all title-affecting failed release / blocklist side effects.
#[expect(
    clippy::too_many_arguments,
    reason = "failure recording persists the full release-attribution envelope for auditability"
)]
async fn record_failed_release_outcome(
    app: &AppUseCase,
    title_id: Option<&str>,
    attribution: &FailedReleaseAttribution,
    source_title: Option<String>,
    source_hint: Option<String>,
    download_id: Option<String>,
    client_id: Option<String>,
    client_name: Option<String>,
    client_type: Option<String>,
    quality: Option<String>,
    failure_reason: Option<String>,
    blocklist_reason: Option<String>,
    source_password: Option<String>,
) {
    let normalized_source_title = normalize_release_attempt_title(source_title.as_deref());
    let normalized_source_hint = normalize_release_attempt_hint(source_hint.as_deref());
    let normalized_client_id = normalized_non_empty_owned(client_id);
    let normalized_client_name = normalized_non_empty_owned(client_name);
    let normalized_client_type = normalized_non_empty_owned(client_type);

    let mut blocklist_persisted = false;
    if let Some(title_id) = title_id {
        let _ = app
            .services
            .workflow
            .release_attempts
            .record_release_attempt(
                Some(title_id.to_string()),
                normalized_source_hint.clone(),
                normalized_source_title.clone(),
                ReleaseDownloadAttemptOutcome::Failed,
                failure_reason.clone(),
                source_password,
            )
            .await;

        if let Some(reason) = blocklist_reason.clone() {
            let mut blocklist_data = HashMap::new();
            if !attribution.episode_ids.is_empty() {
                blocklist_data.insert(
                    "episode_ids".to_string(),
                    serde_json::json!(attribution.episode_ids),
                );
            }
            if let Some(collection_id) = attribution.collection_id.as_deref() {
                blocklist_data.insert(
                    "collection_id".to_string(),
                    serde_json::json!(collection_id),
                );
            }
            match app
                .services
                .workflow
                .blocklist_repo
                .add_failed_download_if_absent(&NewBlocklistEntry {
                    title_id: title_id.to_string(),
                    source_title: normalized_source_title.clone(),
                    source_hint: normalized_source_hint.clone(),
                    quality: quality.clone(),
                    download_id: download_id.clone(),
                    reason: Some(reason),
                    data: blocklist_data,
                })
                .await
            {
                Ok(true) => {
                    blocklist_persisted = true;
                }
                Ok(false) => {}
                Err(error) => {
                    warn!(
                        title_id,
                        source_title = normalized_source_title.as_deref().unwrap_or(""),
                        error = %error,
                        "failed to persist blocklist entry for failed download"
                    );
                }
            }
        }
    }

    let title = attribution.title.as_ref();
    let title_snapshot = title.map(title_context_snapshot);
    let payload = DomainEventPayload::DownloadFailed(DownloadFailedEventData {
        title: title_snapshot.clone(),
        source_title: normalized_source_title.clone(),
        source_hint: normalized_source_hint.clone(),
        download_id: download_id.clone(),
        client_id: normalized_client_id.clone(),
        client_name: normalized_client_name.clone(),
        client_type: normalized_client_type.clone(),
        quality: quality.clone(),
        reason: failure_reason,
        episode_ids: attribution.episode_ids.clone(),
        collection_id: attribution.collection_id.clone(),
    });
    let _ = app
        .append_domain_event(title_scoped_domain_event(title_id, title, payload))
        .await;

    if blocklist_persisted && let Some(reason) = blocklist_reason {
        let payload = DomainEventPayload::ReleaseBlocklisted(ReleaseBlocklistedEventData {
            title: title_snapshot,
            source_title: normalized_source_title,
            source_hint: normalized_source_hint,
            download_id,
            client_id: normalized_client_id,
            client_name: normalized_client_name,
            client_type: normalized_client_type,
            quality,
            reason: Some(reason),
            episode_ids: attribution.episode_ids.clone(),
            collection_id: attribution.collection_id.clone(),
        });
        let _ = app
            .append_domain_event(title_scoped_domain_event(title_id, title, payload))
            .await;
    }
}
impl DownloadClientSnapshot {
    pub(crate) async fn fetch(app: &AppUseCase) -> Self {
        let mut active_titles = std::collections::HashSet::new();
        let mut active_client_ids = std::collections::HashSet::new();
        let mut active_raw_item_id_counts = std::collections::HashMap::new();
        let mut completed_client_ids = std::collections::HashSet::new();
        let mut completed_raw_item_id_counts = std::collections::HashMap::new();
        let mut failed_by_download_id = std::collections::HashMap::new();
        let mut queue_listing_failed = false;
        let mut history_listing_failed = false;

        // Fetch current queue
        match app.services.integrations.download_client.list_queue().await {
            Ok(queue) => {
                for item in &queue {
                    match item.state {
                        DownloadQueueState::Queued
                        | DownloadQueueState::Downloading
                        | DownloadQueueState::Paused => {
                            active_titles.insert(item.title_name.to_ascii_lowercase());
                            active_client_ids.insert(download_client_item_identity(
                                Some(item.client_id.as_str()),
                                &item.download_client_item_id,
                            ));
                            *active_raw_item_id_counts
                                .entry(item.download_client_item_id.clone())
                                .or_insert(0) += 1;
                        }
                        _ => {}
                    }
                }
                if !active_titles.is_empty() {
                    info!(
                        active_count = active_titles.len(),
                        "download client snapshot: active queue items"
                    );
                }
            }
            Err(error) => {
                queue_listing_failed = true;
                warn!(
                    error = %error,
                    "download client snapshot: queue listing failed; treating queue as possibly-active to avoid blind double-submits"
                );
            }
        }

        // Fetch recent history — key by download client job ID (works across all
        // clients: NZBGet, SABnzbd, Weaver).
        match app
            .services
            .integrations
            .download_client
            .list_history()
            .await
        {
            Ok(history) => {
                for item in &history {
                if item.state == DownloadQueueState::Completed {
                    completed_client_ids.insert(download_client_item_identity(
                        Some(item.client_id.as_str()),
                        &item.download_client_item_id,
                    ));
                    *completed_raw_item_id_counts
                        .entry(item.download_client_item_id.clone())
                        .or_insert(0) += 1;
                } else if item.state == DownloadQueueState::Failed {
                    let reason = item
                        .attention_reason
                        .as_deref()
                        .unwrap_or("unknown")
                        .to_ascii_uppercase();
                    failed_by_download_id.insert(
                        download_client_item_identity(
                            Some(item.client_id.as_str()),
                            &item.download_client_item_id,
                        ),
                        FailedDownloadSnapshot {
                            reason,
                            download_client_item_id: item.download_client_item_id.clone(),
                            client_id: item.client_id.clone(),
                            client_name: normalized_non_empty_owned(Some(item.client_name.clone())),
                        },
                    );
                }
            }
                if !failed_by_download_id.is_empty() {
                    debug!(
                        failed_count = failed_by_download_id.len(),
                        "download client snapshot: failed history items"
                    );
                }
            }
            Err(error) => {
                history_listing_failed = true;
                warn!(
                    error = %error,
                    "download client snapshot: history listing failed; failure detection is skipped this cycle"
                );
            }
        }

        Self {
            active_titles,
            active_client_ids,
            active_raw_item_id_counts,
            completed_client_ids,
            completed_raw_item_id_counts,
            failed_by_download_id,
            queue_listing_failed,
            history_listing_failed,
        }
    }

    /// Returns true if a release with this title is currently
    /// queued/downloading, or if the queue could not be observed this cycle (an
    /// unknown queue is treated as possibly-active so automatic grabs skip/defer
    /// instead of double-submitting blind).
    pub(crate) fn is_active(&self, release_title: &str) -> bool {
        self.queue_listing_failed
            || self
                .active_titles
                .contains(&release_title.to_ascii_lowercase())
    }

    /// Whether the queue could not be listed while building this snapshot.
    /// Callers that would otherwise expire a release on an "already active"
    /// signal must instead defer, since the signal here is "unknown", not
    /// "confirmed active".
    pub(crate) fn queue_listing_failed(&self) -> bool {
        self.queue_listing_failed
    }

    /// If a download with this job ID failed in history with a blocklist-worthy
    /// reason, returns the failure snapshot.
    pub(crate) fn failed_item(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> Option<&FailedDownloadSnapshot> {
        // Failure detection reads only history; if it could not be observed we
        // report no failures rather than acting on an incomplete map.
        if self.history_listing_failed {
            return None;
        }
        self.failed_by_download_id
            .get(&download_client_item_identity(
                client_id,
                download_client_item_id,
            ))
            .or_else(|| self.failed_by_download_id.get(download_client_item_id))
    }

    fn has_active_client_item(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> bool {
        if self.queue_listing_failed {
            return true;
        }
        let exact_key = download_client_item_identity(client_id, download_client_item_id);
        self.active_client_ids.contains(&exact_key)
            || self.active_raw_item_id_counts.get(download_client_item_id) == Some(&1)
    }

    fn has_completed_client_item(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> bool {
        let exact_key = download_client_item_identity(client_id, download_client_item_id);
        self.completed_client_ids.contains(&exact_key)
            || self
                .completed_raw_item_id_counts
                .get(download_client_item_id)
                == Some(&1)
    }
}
fn submission_is_active(
    submission: &DownloadSubmission,
    dl_snapshot: &DownloadClientSnapshot,
) -> bool {
    dl_snapshot.has_active_client_item(
        submission.download_client_id.as_deref(),
        &submission.download_client_item_id,
    )
}
fn submission_is_completed(
    submission: &DownloadSubmission,
    dl_snapshot: &DownloadClientSnapshot,
) -> bool {
    dl_snapshot.has_completed_client_item(
        submission.download_client_id.as_deref(),
        &submission.download_client_item_id,
    )
}
/// Check grabbed wanted items against the download client. If a grabbed
/// release has failed in the download client, blocklist it and re-queue the
/// wanted item for immediate re-search.
async fn check_grabbed_for_failures(app: &AppUseCase, dl_snapshot: &DownloadClientSnapshot) {
    let grabbed_items = match app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            statuses: vec!["grabbed".into()],
            limit: 200,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
    {
        Ok(items) => items,
        Err(err) => {
            warn!(error = %err, "failed to list grabbed wanted items for failure check");
            return;
        }
    };

    if grabbed_items.is_empty() {
        debug!("check_grabbed_for_failures: no grabbed wanted items");
        return;
    }

    debug!(
        count = grabbed_items.len(),
        "check_grabbed_for_failures: checking grabbed wanted items against download client"
    );

    let mut submissions_by_title = HashMap::new();
    let mut processed_failed_submissions = HashSet::new();

    for item in &grabbed_items {
        // Extract the grabbed release title from the stored JSON (for logging/blocklist)
        let release_title = item
            .grabbed_release
            .as_deref()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .and_then(|v| v.get("title").and_then(|t| t.as_str().map(String::from)))
            .unwrap_or_default();

        // Look up the download submission to find the download client job ID.
        // Match by job ID (works across all clients) instead of title name
        // (which gets sanitized differently by each client).
        let submissions = if let Some(cached) = submissions_by_title.get(&item.title_id) {
            cached
        } else {
            let fetched = match app
                .services
                .workflow
                .download_submissions
                .list_for_title(&item.title_id)
                .await
            {
                Ok(submissions) => submissions,
                Err(err) => {
                    warn!(
                        error = %err,
                        title_id = item.title_id.as_str(),
                        "failed to list submissions for grabbed wanted item title"
                    );
                    Vec::new()
                }
            };

            trace!(
                title_id = item.title_id.as_str(),
                release = release_title.as_str(),
                submission_count = fetched.len(),
                submission_ids = ?fetched.iter().map(|s| s.download_client_item_id.as_str()).collect::<Vec<_>>(),
                "check_grabbed_for_failures: looking up submissions for grabbed title"
            );

            submissions_by_title.insert(item.title_id.clone(), fetched);
            submissions_by_title
                .get(&item.title_id)
                .expect("title submissions cache entry should exist")
        };

        let failed = submissions.iter().find_map(|sub| {
            dl_snapshot
                .failed_item(
                    sub.download_client_id.as_deref(),
                    &sub.download_client_item_id,
                )
                .map(|f| (f, sub))
        });

        if let Some((failed_item, submission)) = failed {
            let failure_key = format!(
                "{}:{}:{}",
                submission.download_client_id.as_deref().unwrap_or(""),
                submission.download_client_type,
                submission.download_client_item_id
            );
            if !processed_failed_submissions.insert(failure_key.clone()) {
                debug!(
                    title_id = item.title_id.as_str(),
                    failure_key = failure_key.as_str(),
                    "skipping duplicate failed submission for covered grabbed set"
                );
                continue;
            }

            let release_title = submission
                .source_title
                .clone()
                .unwrap_or_else(|| release_title.clone());
            warn!(
                title_id = item.title_id.as_str(),
                release = release_title.as_str(),
                reason = failed_item.reason.as_str(),
                "grabbed release failed in download client"
            );

            let _ = process_download_failure(
                app,
                DownloadFailureContext {
                    wanted_item: Some(item.clone()),
                    title_id: Some(item.title_id.clone()),
                    client_id: failed_item.client_id.clone(),
                    client_type: submission.download_client_type.clone(),
                    client_name: failed_item.client_name.clone(),
                    client_item_id: failed_item.download_client_item_id.clone(),
                    release_title: release_title.clone(),
                    reason: failed_item.reason.clone(),
                    remove_from_client_if_configured: true,
                    skip_reacquire: false,
                },
                Some(dl_snapshot),
            )
            .await;
        }
    }
}
async fn find_failed_submission(
    app: &AppUseCase,
    context: &DownloadFailureContext,
) -> Option<DownloadSubmission> {
    app.services
        .workflow
        .download_submissions
        .find_by_client_item_id(&DownloadSourceIdentity::new(
            Some(context.client_id.as_str()),
            &context.client_type,
            &context.client_item_id,
        ))
        .await
        .ok()
        .flatten()
}
fn preferred_failed_release_title(
    context: &DownloadFailureContext,
    failed_submission: Option<&DownloadSubmission>,
) -> Option<String> {
    failed_submission
        .and_then(|submission| normalized_non_empty_owned(submission.source_title.clone()))
        .or_else(|| normalized_non_empty_owned(Some(context.release_title.clone())))
}
async fn resolve_failed_collection_episode_wanted_items(
    app: &AppUseCase,
    submission: &DownloadSubmission,
) -> AppResult<Vec<AcquisitionScopeState>> {
    let SubmissionScope::Collection { collection_id } = &submission.scope else {
        return Ok(Vec::new());
    };

    let episode_ids: HashSet<String> = app
        .services
        .catalog
        .shows
        .list_episodes_for_collection(collection_id)
        .await?
        .into_iter()
        .map(|episode| episode.id)
        .collect();

    if episode_ids.is_empty() {
        return Ok(Vec::new());
    }

    let wanted_items = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            media_types: vec!["episode".into()],
            title_id: Some(submission.title_id.clone()),
            limit: 500,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await?;

    Ok(wanted_items
        .into_iter()
        .filter(|item| {
            matches!(item.status, AcquisitionScopeStatus::Wanted | AcquisitionScopeStatus::Grabbed)
                && item
                    .episode_id
                    .as_ref()
                    .is_some_and(|episode_id| episode_ids.contains(episode_id))
        })
        .collect())
}
pub(crate) async fn process_download_failure(
    app: &AppUseCase,
    context: DownloadFailureContext,
    snapshot: Option<&DownloadClientSnapshot>,
) -> FailureHandlingOutcome {
    let failed_submission = find_failed_submission(app, &context).await;
    if context.wanted_item.is_none() && failed_submission.is_none() {
        info!(
            client_id = context.client_id.as_str(),
            client_type = context.client_type.as_str(),
            download_client_item_id = context.client_item_id.as_str(),
            release_title = context.release_title.as_str(),
            "skipping automatic failed download handling without scryer grab history"
        );
        return FailureHandlingOutcome::RecordedOnly;
    }

    let resolved_title_id = context
        .wanted_item
        .as_ref()
        .map(|item| item.title_id.clone())
        .or(context.title_id.clone())
        .or_else(|| {
            failed_submission
                .as_ref()
                .map(|submission| submission.title_id.clone())
        });
    let download_id = normalized_non_empty_owned(Some(context.client_item_id.clone()));
    let preferred_source_title =
        preferred_failed_release_title(&context, failed_submission.as_ref());
    let normalized_source_title =
        normalize_release_attempt_title(preferred_source_title.as_deref());
    let normalized_source_hint = resolved_failed_release_hint(failed_submission.as_ref());
    let quality = failed_submission
        .as_ref()
        .and_then(|submission| release_quality_hint(submission.source_title.as_deref()))
        .or_else(|| release_quality_hint(Some(context.release_title.as_str())));
    let release_title_for_matching = preferred_source_title
        .as_deref()
        .unwrap_or(context.release_title.as_str());
    let _failure_guard = app
        .runtime
        .acquisition
        .download_failure_guards
        .acquire_release_or_client_item(
            resolved_title_id.as_deref(),
            normalized_source_title.as_deref(),
            &context.client_id,
            &context.client_type,
            &context.client_item_id,
        )
        .await;

    let failure_already_recorded = if let Some(title_id) = resolved_title_id.as_deref() {
        match app
            .services
            .workflow
            .blocklist_repo
            .has_recorded_download_failure(title_id, normalized_source_title.as_deref())
            .await
        {
            Ok(true) => {
                info!(
                    title_id,
                    client_id = context.client_id.as_str(),
                    client_type = context.client_type.as_str(),
                    download_client_item_id = context.client_item_id.as_str(),
                    release_title = release_title_for_matching,
                    "skipping duplicate failed download handling; failure already recorded"
                );
                true
            }
            Ok(false) => false,
            Err(error) => {
                warn!(
                    title_id,
                    client_id = context.client_id.as_str(),
                    client_type = context.client_type.as_str(),
                    download_client_item_id = context.client_item_id.as_str(),
                    error = %error,
                    "failed to check for duplicate failed download blocklist entry"
                );
                false
            }
        }
    } else {
        false
    };

    if failure_already_recorded && !context.skip_reacquire {
        return FailureHandlingOutcome::AlreadyHandled;
    }

    let failed_collection_items = if let Some(submission) = failed_submission.as_ref() {
        match resolve_failed_collection_episode_wanted_items(app, submission).await {
            Ok(items) if !items.is_empty() => Some(items),
            Ok(_) => None,
            Err(err) => {
                warn!(
                    title_id = submission.title_id.as_str(),
                    download_client_item_id = context.client_item_id.as_str(),
                    error = %err,
                    "failed to resolve wanted items for collection-scoped download failure"
                );
                None
            }
        }
    } else {
        None
    };

    let wanted_item = match context.wanted_item.clone() {
        Some(item) => Some(item),
        None if failed_collection_items.is_none() && failed_submission.is_some() => {
            resolve_failure_wanted_item(
                app,
                resolved_title_id.as_deref(),
                release_title_for_matching,
            )
            .await
        }
        None => None,
    };
    let attribution = resolve_failed_release_attribution(
        app,
        resolved_title_id.as_deref(),
        failed_submission.as_ref(),
        wanted_item.as_ref(),
        failed_collection_items.as_deref(),
    )
    .await;

    let (outcome, failure_reason) = if context.skip_reacquire {
        if let Some(items) = failed_collection_items.as_ref() {
            let mut update_error = None;
            for item in items {
                if let Err(err) = mark_wanted_item_failed_without_reacquire(app, item).await {
                    update_error.get_or_insert_with(|| err.to_string());
                }
            }
            if let Some(err) = update_error {
                (
                    FailureHandlingOutcome::RecordedOnly,
                    format!(
                        "season pack download failed for '{}': {}; failed to disable reacquisition: {}",
                        release_title_for_matching, context.reason, err
                    ),
                )
            } else {
                (
                    FailureHandlingOutcome::RecordedNoReacquire,
                    format!(
                        "season pack download failed for '{}': {}; recorded failure without reacquisition",
                        release_title_for_matching, context.reason
                    ),
                )
            }
        } else if let Some(item) = wanted_item.as_ref() {
            match mark_wanted_item_failed_without_reacquire(app, item).await {
                Ok(()) => (
                    FailureHandlingOutcome::RecordedNoReacquire,
                    format!(
                        "download failed for '{}': {}; recorded failure without reacquisition",
                        release_title_for_matching, context.reason
                    ),
                ),
                Err(err) => (
                    FailureHandlingOutcome::RecordedOnly,
                    format!(
                        "download failed for '{}': {}; failed to disable reacquisition: {}",
                        release_title_for_matching, context.reason, err
                    ),
                ),
            }
        } else {
            (
                FailureHandlingOutcome::RecordedNoReacquire,
                format!(
                    "download failed: {} — {}; recorded failure without reacquisition",
                    release_title_for_matching, context.reason
                ),
            )
        }
    } else if let Some(items) = failed_collection_items.as_ref() {
        // A failed season pack re-opens every covered episode scope: coverage
        // pruned, state reset, acquisition woken. The cursor re-converges them
        // individually (RFC 119 §11 #8 — never a cadence write).
        for item in items {
            app.reopen_wanted_scope_for_acquisition(item).await;
        }

        let message = format!(
            "season pack download failed for '{}': {}; re-opened season episodes for individual search",
            release_title_for_matching, context.reason
        );

        info!(
            title_id = resolved_title_id.as_deref().unwrap_or(""),
            affected_wanted_items = items.len(),
            release_title = release_title_for_matching,
            "re-opened season episode scopes after failed season-pack download"
        );

        (FailureHandlingOutcome::RequeuedFreshSearch, message)
    } else if let Some(item) = wanted_item.as_ref() {
        let now = Utc::now();
        let owned_snapshot = if snapshot.is_none() {
            Some(DownloadClientSnapshot::fetch(app).await)
        } else {
            None
        };
        let active_snapshot = snapshot.or(owned_snapshot.as_ref());

        if let Some(active_snapshot) = active_snapshot {
            match recover_from_standby_candidates(
                app,
                item,
                release_title_for_matching,
                active_snapshot,
                &now,
            )
            .await
            {
                StandbyRecoveryOutcome::Recovered => (
                    FailureHandlingOutcome::RecoveredFromStandby,
                    format!(
                        "download failed for '{}': {}; recovered from standby candidate",
                        release_title_for_matching, context.reason
                    ),
                ),
                StandbyRecoveryOutcome::Deferred => (
                    FailureHandlingOutcome::RequeuedDeferred,
                    format!(
                        "download failed for '{}': {}; standby candidate kept pending until download client recovers",
                        release_title_for_matching, context.reason
                    ),
                ),
                StandbyRecoveryOutcome::Exhausted => {
                    // No standby candidate left: re-open the scope's convergence
                    // (coverage pruned, state reset) so the cursor re-searches it.
                    // The failed release is blocklisted below, and standby-first +
                    // scheduler pacing keep this from tight-looping — never a
                    // cadence write (RFC 119 §11 #8).
                    app.reopen_wanted_scope_for_acquisition(item).await;

                    (
                        FailureHandlingOutcome::RequeuedFreshSearch,
                        format!(
                            "download failed for '{}': {}; standby exhausted, re-opened scope for fresh search",
                            release_title_for_matching, context.reason
                        ),
                    )
                }
            }
        } else {
            (
                FailureHandlingOutcome::RecordedOnly,
                format!(
                    "download failed for '{}': {}; download client snapshot unavailable",
                    context.release_title, context.reason
                ),
            )
        }
    } else {
        (
            FailureHandlingOutcome::RecordedOnly,
            format!(
                "download failed: {} — {}",
                release_title_for_matching, context.reason
            ),
        )
    };

    let blocklist_reason = format!("download client failure: {}", context.reason);

    if !failure_already_recorded {
        record_failed_release_outcome(
            app,
            resolved_title_id.as_deref(),
            &attribution,
            normalized_source_title.clone(),
            normalized_source_hint.clone(),
            download_id.clone(),
            Some(context.client_id.clone()),
            context.client_name.clone(),
            Some(context.client_type.clone()),
            quality,
            Some(failure_reason),
            Some(blocklist_reason),
            None,
        )
        .await;
    }

    if context.remove_from_client_if_configured
        && let Some(title) = attribution.title.as_ref()
        && app
            .should_remove_failed_download(
                Some(title.library_id.as_str()),
                &title.facet,
                &context.client_id,
            )
            .await
        && let Err(error) = app
            .services
            .integrations
            .download_client
            .delete_queue_item_for_client_id(&context.client_id, &context.client_item_id, true)
            .await
    {
        warn!(
            title_id = resolved_title_id.as_deref().unwrap_or(""),
            client_id = context.client_id.as_str(),
            download_client_item_id = context.client_item_id.as_str(),
            error = %error,
            "failed to delete failed download from client history"
        );
    }

    let _ = app
        .services
        .workflow
        .download_submissions
        .update_tracked_state(
            &DownloadSourceIdentity::new(
                Some(context.client_id.as_str()),
                &context.client_type,
                &context.client_item_id,
            ),
            scryer_domain::TrackedDownloadState::Failed.as_str(),
        )
        .await;

    outcome
}
async fn resolve_failure_wanted_item(
    app: &AppUseCase,
    title_id: Option<&str>,
    release_title: &str,
) -> Option<AcquisitionScopeState> {
    let title_id = title_id?.trim();
    if title_id.is_empty() {
        return None;
    }

    let grabbed_items = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            statuses: vec!["grabbed".into()],
            title_id: Some(title_id.to_string()),
            limit: 25,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .ok()?;

    if grabbed_items.len() == 1 {
        return grabbed_items.into_iter().next();
    }

    grabbed_items.into_iter().find(|item| {
        extract_grabbed_release_title(item.grabbed_release.as_deref())
            .is_some_and(|title| title.eq_ignore_ascii_case(release_title))
    })
}
async fn prune_standby_candidates(app: &AppUseCase) {
    let all_standby = app
        .services
        .workflow
        .pending_releases
        .list_all_standby_pending_releases()
        .await
        .unwrap_or_default();

    if all_standby.is_empty() {
        return;
    }

    let now = Utc::now();
    let cutoff = now - Duration::hours(STANDBY_RETENTION_HOURS);
    let mut grouped: std::collections::HashMap<String, Vec<PendingRelease>> =
        std::collections::HashMap::new();
    for release in all_standby {
        grouped
            .entry(release.wanted_item_id.clone())
            .or_default()
            .push(release);
    }

    for (wanted_item_id, mut releases) in grouped {
        let wanted = app
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(&wanted_item_id)
            .await
            .ok()
            .flatten();

        let Some(wanted) = wanted else {
            let _ = app
                .services
                .workflow
                .pending_releases
                .delete_standby_pending_releases_for_wanted_item(&wanted_item_id)
                .await;
            continue;
        };

        if wanted.status != AcquisitionScopeStatus::Grabbed {
            let _ = app
                .services
                .workflow
                .pending_releases
                .delete_standby_pending_releases_for_wanted_item(&wanted_item_id)
                .await;
            continue;
        }

        releases.sort_by(|left, right| right.added_at.cmp(&left.added_at));
        for (index, release) in releases.iter().enumerate() {
            let added_at = crate::quality_profile::parse_published_at(&release.added_at);
            let is_stale = added_at.is_none_or(|added_at| added_at < cutoff);
            let is_overflow = index >= MAX_STANDBY_CANDIDATES_PER_WANTED_ITEM;
            if is_stale || is_overflow {
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(&release.id, PendingReleaseStatus::Expired, None)
                    .await;
            }
        }
    }
}
async fn recover_from_standby_candidates(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    failed_release_title: &str,
    dl_snapshot: &DownloadClientSnapshot,
    now: &DateTime<Utc>,
) -> StandbyRecoveryOutcome {
    let standby_releases = app
        .services
        .workflow
        .pending_releases
        .list_standby_pending_releases_for_wanted_item(&item.id)
        .await
        .unwrap_or_default();
    let db_blocklist: std::collections::HashSet<String> = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&item.title_id, 200)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.source_title)
        .map(|title| title.to_ascii_lowercase())
        .collect();

    for standby in standby_releases {
        let mut effective_wanted = item.clone();
        effective_wanted.grabbed_release = None;
        effective_wanted.last_search_at = None;

        let claimed = app
            .services
            .workflow
            .pending_releases
            .compare_and_set_pending_release_status(
                &standby.id,
                PendingReleaseStatus::Standby,
                PendingReleaseStatus::Processing,
                None,
            )
            .await
            .unwrap_or(false);
        if !claimed {
            continue;
        }

        if db_blocklist.contains(&standby.release_title.to_ascii_lowercase()) {
            let _ = app
                .services
                .workflow
                .pending_releases
                .update_pending_release_status(&standby.id, PendingReleaseStatus::Expired, None)
                .await;
            continue;
        }

        if dl_snapshot.queue_listing_failed() {
            // Cannot confirm the release isn't already active; keep the standby
            // for a later cycle rather than expiring it on an unknown signal.
            info!(
                title_id = item.title_id.as_str(),
                standby_release = standby.release_title.as_str(),
                "standby reacquisition: queue listing failed, keeping release pending"
            );
            let _ = app
                .services
                .workflow
                .pending_releases
                .update_pending_release_status(&standby.id, PendingReleaseStatus::Standby, None)
                .await;
            return StandbyRecoveryOutcome::Deferred;
        }

        if dl_snapshot.is_active(&standby.release_title) {
            let _ = app
                .services
                .workflow
                .pending_releases
                .update_pending_release_status(&standby.id, PendingReleaseStatus::Expired, None)
                .await;
            continue;
        }

        info!(
            title_id = item.title_id.as_str(),
            failed_release = failed_release_title,
            standby_release = standby.release_title.as_str(),
            "attempting standby reacquisition"
        );

        match app
            .try_grab_pending_release(&effective_wanted, &standby, now)
            .await
        {
            Ok(super::pending::PendingGrabOutcome::Grabbed) => {
                let grabbed_at = now.to_rfc3339();
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(
                        &standby.id,
                        PendingReleaseStatus::Grabbed,
                        Some(&grabbed_at),
                    )
                    .await;

                let siblings = app
                    .services
                    .workflow
                    .pending_releases
                    .list_standby_pending_releases_for_wanted_item(&item.id)
                    .await
                    .unwrap_or_default();
                for sibling in siblings {
                    if sibling.id == standby.id {
                        continue;
                    }
                    let _ = app
                        .services
                        .workflow
                        .pending_releases
                        .update_pending_release_status(
                            &sibling.id,
                            PendingReleaseStatus::Superseded,
                            None,
                        )
                        .await;
                }

                if let Ok(Some(title)) = app.services.catalog.titles.get_by_id(&item.title_id).await
                {
                    let _ = app
                        .append_domain_event(new_title_domain_event(
                            None,
                            &title,
                            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                                title: title_context_snapshot(&title),
                                source_title: Some(standby.release_title.clone()),
                                source_hint: None,
                                download_id: None,
                                episode_ids: item.episode_id.iter().cloned().collect(),
                            }),
                        ))
                        .await;
                }

                return StandbyRecoveryOutcome::Recovered;
            }
            Ok(super::pending::PendingGrabOutcome::Deferred) => {
                info!(
                    release = standby.release_title.as_str(),
                    "standby reacquisition: download client unavailable, keeping release pending"
                );
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(
                        &standby.id,
                        PendingReleaseStatus::Standby,
                        None,
                    )
                    .await;
                return StandbyRecoveryOutcome::Deferred;
            }
            Ok(super::pending::PendingGrabOutcome::Rejected) | Err(_) => {
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(&standby.id, PendingReleaseStatus::Expired, None)
                    .await;
            }
        }
    }

    StandbyRecoveryOutcome::Exhausted
}

#[expect(
    clippy::too_many_arguments,
    reason = "standby candidate persistence carries the search context explicitly"
)]
async fn persist_standby_candidates(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    title: &Title,
    results: &[IndexerSearchResult],
    start_index: usize,
    now: &DateTime<Utc>,
    failed_source_kinds: &[DownloadSourceKind],
    db_blocklist: &std::collections::HashSet<String>,
) {
    let _ = app
        .services
        .workflow
        .pending_releases
        .delete_standby_pending_releases_for_wanted_item(&item.id)
        .await;

    let mut persisted = 0usize;
    let mut seen_source_hints = std::collections::HashSet::new();

    for candidate in results.iter().skip(start_index) {
        if persisted >= MAX_STANDBY_CANDIDATES_PER_WANTED_ITEM {
            break;
        }

        let decision_code = effective_auto_decision_code(candidate, failed_source_kinds, db_blocklist);
        if !decision_code.is_eligible() {
            if matches!(
                decision_code,
                ReleaseAutoDecisionCode::NegativeScore
                    | ReleaseAutoDecisionCode::UpgradeRejected
                    | ReleaseAutoDecisionCode::CutoffReached
            ) {
                break;
            }
            continue;
        }

        let source_hint = candidate
            .download_url
            .clone()
            .or_else(|| candidate.link.clone());
        let Some(source_hint_value) = source_hint else {
            continue;
        };
        if !seen_source_hints.insert(source_hint_value.clone()) {
            continue;
        }

        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|decision| decision.preference_score)
            .unwrap_or(0);
        let scoring_log_json = candidate
            .quality_profile_decision
            .as_ref()
            .and_then(|decision| {
                serde_json::to_string(
                    &decision
                        .scoring_log
                        .iter()
                        .map(|entry| serde_json::json!({"code": entry.code, "delta": entry.delta}))
                        .collect::<Vec<_>>(),
                )
                .ok()
            });

        let standby = PendingRelease {
            id: Id::new().0,
            wanted_item_id: item.id.clone(),
            title_id: title.id.clone(),
            release_title: candidate.title.clone(),
            release_url: Some(source_hint_value),
            source_kind: candidate.source_kind,
            release_size_bytes: candidate.size_bytes,
            release_score: candidate_score,
            scoring_log_json,
            indexer_source: Some(candidate.source.clone()),
            release_guid: candidate.guid.clone(),
            added_at: now.to_rfc3339(),
            delay_until: now.to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: crate::normalize_release_password(candidate.password_hint.as_deref()),
            published_at: candidate.published_at.clone(),
            info_hash: candidate
                .extra
                .get("info_hash")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        };

        if app
            .services
            .workflow
            .pending_releases
            .insert_pending_release(&standby)
            .await
            .is_ok()
        {
            persisted += 1;
        }
    }

    if persisted > 0 {
        info!(
            wanted_item_id = item.id.as_str(),
            title_id = title.id.as_str(),
            standby_candidates = persisted,
            "persisted standby candidates for failed-download recovery"
        );
    }
}

#[cfg(test)]
mod client_snapshot_tests {
    use super::*;

    fn snapshot(queue_listing_failed: bool, history_listing_failed: bool) -> DownloadClientSnapshot {
        DownloadClientSnapshot {
            active_titles: std::collections::HashSet::new(),
            active_client_ids: std::collections::HashSet::new(),
            active_raw_item_id_counts: std::collections::HashMap::new(),
            completed_client_ids: std::collections::HashSet::new(),
            completed_raw_item_id_counts: std::collections::HashMap::new(),
            failed_by_download_id: std::collections::HashMap::new(),
            queue_listing_failed,
            history_listing_failed,
        }
    }

    #[test]
    fn queue_listing_failure_treats_everything_as_active() {
        let snap = snapshot(true, false);
        assert!(snap.queue_listing_failed());
        // Any title / client item is treated as possibly-active so automatic
        // grabs skip/defer instead of double-submitting blind.
        assert!(snap.is_active("Some.Release.That.Is.Not.In.Any.Queue"));
        assert!(snap.has_active_client_item(Some("client-1"), "nzo_missing"));
        assert!(snap.has_active_client_item(None, "nzo_missing"));
    }

    #[test]
    fn observable_empty_queue_reports_nothing_active() {
        let snap = snapshot(false, false);
        assert!(!snap.queue_listing_failed());
        assert!(!snap.is_active("Some.Release"));
        assert!(!snap.has_active_client_item(Some("client-1"), "nzo_missing"));
    }

    #[test]
    fn history_listing_failure_reports_no_failures() {
        let mut snap = snapshot(false, true);
        snap.failed_by_download_id.insert(
            "client-1:nzo_1".to_string(),
            FailedDownloadSnapshot {
                reason: "MISSING ARTICLES".to_string(),
                download_client_item_id: "nzo_1".to_string(),
                client_id: "client-1".to_string(),
                client_name: None,
            },
        );
        // Even with a populated map, an unobservable history must not surface
        // failures (failure detection is skipped this cycle).
        assert!(snap.failed_item(Some("client-1"), "nzo_1").is_none());

        // With history observable, the same entry is reported.
        snap.history_listing_failed = false;
        assert!(snap.failed_item(Some("client-1"), "nzo_1").is_some());
    }
}
