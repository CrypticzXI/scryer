const TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET: Duration = Duration::from_millis(25);
const TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT: usize = 1;
const DOWNLOAD_QUEUE_POLL_INTERVAL: Duration = Duration::from_secs(10);
const DOWNLOAD_QUEUE_RECENT_HISTORY_POLL_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct DownloadQueuePollerOptions {
    pub interval: Duration,
    pub excluded_client_types: Vec<String>,
}

impl Default for DownloadQueuePollerOptions {
    fn default() -> Self {
        Self {
            interval: DOWNLOAD_QUEUE_POLL_INTERVAL,
            excluded_client_types: Vec::new(),
        }
    }
}

struct TrackedDownloadWorkDrain {
    pending_ids: std::collections::VecDeque<String>,
    attempted_ids: HashSet<String>,
    completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
}

impl TrackedDownloadWorkDrain {
    fn empty() -> Self {
        Self {
            pending_ids: std::collections::VecDeque::new(),
            attempted_ids: HashSet::new(),
            completed_lookup: crate::completed_download_handler::CompletedDownloadLookup::default(),
        }
    }

    fn new(
        ids: Vec<String>,
        completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
    ) -> Self {
        Self {
            pending_ids: ids.into(),
            attempted_ids: HashSet::new(),
            completed_lookup,
        }
    }

    fn has_pending(&self) -> bool {
        !self.pending_ids.is_empty()
    }
}

fn apply_tracked_download_queue_metadata(
    item: &mut DownloadQueueItem,
    tracked: &TrackedDownloadQueueMetadata,
) {
    item.tracked_state = Some(tracked.state);
    item.tracked_status = Some(tracked.status);
    item.tracked_status_messages
        .clone_from(&tracked.status_messages);
    item.tracked_match_type = Some(tracked.match_type);
    if let Some(source_title) = tracked
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        item.title_name = source_title.to_string();
    }
    if item.title_id.is_none() && tracked.title_id.is_some() {
        item.title_id.clone_from(&tracked.title_id);
    }
    if item.facet.is_none() && tracked.facet.is_some() {
        item.facet.clone_from(&tracked.facet);
    }
}
fn tracked_download_queue_snapshot(item: &TrackedDownload) -> TrackedDownloadQueueMetadata {
    TrackedDownloadQueueMetadata::from(item)
}
impl AppUseCase {
    pub async fn ignore_tracked_download(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .ignore(crate::tracked_downloads::tracked_download_id(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn mark_tracked_download_failed(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        skip_reacquire: bool,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .mark_failed(
                crate::tracked_downloads::tracked_download_id(
                    client_id,
                    client_type,
                    download_client_item_id,
                ),
                skip_reacquire,
            )
            .await?;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn retry_tracked_download_import(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .retry_import(crate::tracked_downloads::tracked_download_id(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn assign_tracked_download_title(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        title_id: &str,
        scope: SubmissionScope,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        self.services
            .workflow
            .download_submissions
            .record_submission(DownloadSubmission {
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: title.facet.as_str().to_string(),
                download_client_id: client_id.map(str::to_string),
                download_client_type: client_type.to_string(),
                download_client_item_id: download_client_item_id.to_string(),
                source_hint: None,
                source_kind: None,
                source_title: Some(title.name.clone()),
                request_signature: None,
                scope,
            })
            .await?;
        let source_identity =
            DownloadSourceIdentity::new(client_id, client_type, download_client_item_id);
        let actor_snapshot = crate::domain_events::DomainEventActor::from(actor)
            .into_download_submission_actor_snapshot();
        if let Err(error) = self
            .services
            .workflow
            .download_submissions
            .record_submission_actor_snapshot(&source_identity, actor_snapshot)
            .await
        {
            tracing::warn!(
                error = %error,
                client_id = ?client_id,
                client_type,
                download_client_item_id,
                "download_submission_actor_snapshot_persistence_failed"
            );
        }
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .assign_title(
                crate::tracked_downloads::tracked_download_id(
                    client_id,
                    client_type,
                    download_client_item_id,
                ),
                title.id,
            )
            .await?;
        Ok(())
    }
}
pub async fn start_download_queue_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    command_rx: tokio::sync::mpsc::Receiver<crate::tracked_downloads::TrackedDownloadCommand>,
) {
    start_download_queue_poller_with_options(
        app,
        token,
        command_rx,
        DownloadQueuePollerOptions::default(),
    )
    .await;
}

pub async fn start_download_queue_poller_with_options(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    mut command_rx: tokio::sync::mpsc::Receiver<crate::tracked_downloads::TrackedDownloadCommand>,
    options: DownloadQueuePollerOptions,
) {
    use crate::tracked_downloads::{
        TrackedDownloadService, publish_runtime_tracked_download_snapshot_cache,
    };
    use scryer_domain::TrackedDownloadState;

    let actor = match app.find_or_create_default_user().await {
        Ok(actor) => actor,
        Err(error) => {
            tracing::warn!(error = %error, "download queue poller failed to resolve actor");
            return;
        }
    };

    let mut tracker = TrackedDownloadService::new();
    let mut previous_items: HashMap<String, DownloadQueueItem> = HashMap::new();
    let (tracked_work_result_tx, mut tracked_work_result_rx) =
        tokio::sync::mpsc::unbounded_channel::<TrackedDownloadBackgroundWorkResult>();
    let mut tracked_work_in_flight = HashSet::new();
    let mut tracked_work_drain = TrackedDownloadWorkDrain::empty();
    let mut last_recent_history_poll: Option<Instant> = None;

    let excluded_client_type_refs = options
        .excluded_client_types
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    tracing::info!(
        interval_secs = options.interval.as_secs(),
        excluded_client_types = ?options.excluded_client_types,
        "download queue poller started (tracked downloads enabled)"
    );
    let mut interval = tokio::time::interval(options.interval);
    let mut commands_open = true;
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                tracing::info!("download queue poller shutting down");
                break;
            }
            maybe_command = command_rx.recv(), if commands_open => {
                match maybe_command {
                    Some(command) => {
                        handle_tracked_download_command(
                            &app,
                            &actor,
                            &mut tracker,
                            &mut tracked_work_in_flight,
                            &tracked_work_result_tx,
                            command,
                        )
                        .await;
                    }
                    None => {
                        commands_open = false;
                    }
                }
            }
            maybe_result = tracked_work_result_rx.recv(), if !tracked_work_in_flight.is_empty() => {
                if let Some(result) = maybe_result {
                    handle_tracked_download_background_work_result(
                        &app,
                        &mut tracker,
                        &mut tracked_work_in_flight,
                        result,
                    )
                    .await;
                    if try_dispatch_next_tracked_download_background_work(
                        &app,
                        &actor,
                        &mut tracker,
                        &mut tracked_work_in_flight,
                        &tracked_work_result_tx,
                        &mut tracked_work_drain,
                    ) {
                        publish_runtime_tracked_download_snapshot_cache(&app, &tracker).await;
                    }
                }
            }
            _ = interval.tick() => {
                let cycle_started_at = Instant::now();
                let include_recent_history = last_recent_history_poll
                    .map(|last| last.elapsed() >= DOWNLOAD_QUEUE_RECENT_HISTORY_POLL_INTERVAL)
                    .unwrap_or(true);
                if include_recent_history {
                    last_recent_history_poll = Some(Instant::now());
                }
                match app
                    .collect_download_snapshot_items_excluding_client_types(
                        true,
                        include_recent_history,
                        false,
                        &excluded_client_type_refs,
                    )
                    .await
                {
                    Ok(mut items) => {
                        let mut seen_ids = HashSet::new();
                        let completed_download_lookup =
                            crate::completed_download_handler::load_completed_download_lookup_for_items_excluding_client_types(
                                &app,
                                &items,
                                DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT,
                                &excluded_client_type_refs,
                            )
                            .await;

                        // Phase 1: Refresh — track each item and run checks.
                        for item in items.iter() {
                            let id = tracked_download_id_for_item(item);
                            seen_ids.insert(id.clone());

                            let is_new = tracker.find(&id).is_none();
                            tracker.track(&app, item.clone()).await;

                            if let Some(td) = tracker.find(&id)
                                && is_new
                            {
                                if td.state.is_terminal()
                                    || is_history_download_state(&td.client_item.state)
                                {
                                    tracing::debug!(
                                        id = %td.id,
                                        state = ?td.state,
                                        client_state = ?td.client_item.state,
                                        match_type = ?td.match_type,
                                        title_id = ?td.title_id,
                                        client_title_name = %td.client_item.title_name,
                                        "tracked: new background download"
                                    )
                                } else {
                                    tracing::info!(
                                        id = %td.id,
                                        state = ?td.state,
                                        client_state = ?td.client_item.state,
                                        match_type = ?td.match_type,
                                        title_id = ?td.title_id,
                                        client_title_name = %td.client_item.title_name,
                                        "tracked: new download"
                                    )
                                }
                            }

                            if let Some(td) = tracker.find_mut(&id)
                                && matches!(
                                    td.state,
                                    TrackedDownloadState::Downloading
                                        | TrackedDownloadState::ImportPending
                                        | TrackedDownloadState::ImportBlocked
                                )
                            {
                                let state_before = td.state;
                                crate::failed_download_handler::check(td);
                                if td.state != TrackedDownloadState::FailedPending {
                                    crate::completed_download_handler::check_with_lookup(
                                        &app,
                                        td,
                                        completed_download_lookup.as_ref(),
                                    )
                                    .await;
                                }
                                if td.state != state_before {
                                    tracing::info!(
                                        id = %id,
                                        from = ?state_before,
                                        to = ?td.state,
                                        "tracked: state transition after check"
                                    );
                                }
                            }
                        }

                        tracker.update_trackable(&seen_ids);
                        reconcile_terminal_tracked_downloads(&app, &mut tracker).await;
                        publish_runtime_tracked_download_snapshot_cache(&app, &tracker).await;

                        // Phase 2: Dispatch — import pending and failed items.
                        let mut published_after_dispatch = false;
                        if tracked_work_in_flight.is_empty() {
                            if !tracked_work_drain.has_pending() {
                                let trackable_ids = tracker.get_trackable_ids();
                                tracked_work_drain = build_tracked_download_work_drain(
                                    &app,
                                    &tracker,
                                    &tracked_work_in_flight,
                                    &trackable_ids,
                                    &excluded_client_type_refs,
                                )
                                .await;
                            }
                            if try_dispatch_next_tracked_download_background_work(
                                &app,
                                &actor,
                                &mut tracker,
                                &mut tracked_work_in_flight,
                                &tracked_work_result_tx,
                                &mut tracked_work_drain,
                            ) {
                                published_after_dispatch = true;
                            }
                        }

                        if published_after_dispatch {
                            publish_runtime_tracked_download_snapshot_cache(&app, &tracker).await;
                        }

                        // Enrich items with tracked state before broadcasting.
                        for item in &mut items {
                            let id = tracked_download_id_for_item(item);
                            if let Some(td) = tracker.find(&id) {
                                let metadata = tracked_download_queue_snapshot(td);
                                apply_tracked_download_queue_metadata(item, &metadata);
                            }
                        }

                        // Emit download queue gauge by state.
                        let mut counts = [0u64; 9];
                        for item in &items {
                            match item.state {
                                scryer_domain::DownloadQueueState::Queued => counts[0] += 1,
                                scryer_domain::DownloadQueueState::Downloading => counts[1] += 1,
                                scryer_domain::DownloadQueueState::Paused => counts[2] += 1,
                                scryer_domain::DownloadQueueState::Completed => counts[3] += 1,
                                scryer_domain::DownloadQueueState::ImportPending => counts[4] += 1,
                                scryer_domain::DownloadQueueState::Failed => counts[5] += 1,
                                scryer_domain::DownloadQueueState::Verifying => counts[6] += 1,
                                scryer_domain::DownloadQueueState::Repairing => counts[7] += 1,
                                scryer_domain::DownloadQueueState::Extracting => counts[8] += 1,
                            }
                        }
                        let labels = ["queued", "downloading", "paused", "completed", "import_pending", "failed", "verifying", "repairing", "extracting"];
                        for (label, &count) in labels.iter().zip(&counts) {
                            metrics::gauge!("scryer_download_queue_items", "state" => *label).set(count as f64);
                        }

                        publish_download_queue_snapshot_events(
                            &app,
                            None,
                            &mut previous_items,
                            &items,
                        )
                        .await;

                        tracing::debug!(
                            elapsed_ms = cycle_started_at.elapsed().as_millis() as u64,
                            item_count = items.len(),
                            tracked_count = tracker.get_all().len(),
                            active_workers = tracked_work_in_flight.len(),
                            "download queue poller cycle completed"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "download queue poll failed");
                    }
                }
            }
        }
    }
}
fn resolve_tracked_command_id(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    requested_id: &str,
) -> String {
    tracker
        .resolve_cached_id(requested_id)
        .unwrap_or_else(|| requested_id.to_string())
}
async fn handle_tracked_download_command(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    tracked_work_result_tx: &tokio::sync::mpsc::UnboundedSender<
        TrackedDownloadBackgroundWorkResult,
    >,
    command: crate::tracked_downloads::TrackedDownloadCommand,
) {
    use crate::tracked_downloads::TrackedDownloadCommand;
    use scryer_domain::{TrackedDownloadState, TrackedDownloadStatus};

    match command {
        TrackedDownloadCommand::MarkImported { id, reply } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::Imported;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Imported)
                    .await;
                finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Imported)
                    .await;
                Ok(())
            } else {
                Err(AppError::NotFound(format!(
                    "tracked download {requested_id}"
                )))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::Ignore { id, reply } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::Ignored;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Ignored)
                    .await;
                finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Ignored)
                    .await;
                Ok(())
            } else {
                Err(AppError::NotFound(format!(
                    "tracked download {requested_id}"
                )))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::MarkFailed {
            id,
            skip_reacquire,
            reply,
        } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let failure_identity = tracker.find(&id).and_then(
                crate::failed_download_handler::tracked_download_failure_submission_identity,
            );
            let has_grabbed_submission = if let Some(identity) = failure_identity.as_ref() {
                crate::failed_download_handler::download_submission_exists(app, identity).await
            } else {
                false
            };
            let result = if let Some(td) = tracker.find_mut(&id) {
                if !has_grabbed_submission {
                    crate::failed_download_handler::warn_download_not_grabbed(td);
                    if td.state == TrackedDownloadState::FailedPending {
                        td.state = TrackedDownloadState::Downloading;
                    }
                    td.skip_reacquire_on_failure = false;
                    Ok(())
                } else {
                    td.state = TrackedDownloadState::FailedPending;
                    td.status = TrackedDownloadStatus::Error;
                    td.status_messages.clear();
                    td.skip_reacquire_on_failure = skip_reacquire;
                    let completed_lookup =
                        crate::completed_download_handler::CompletedDownloadLookup::default();
                    let _ = try_dispatch_tracked_download_background_work(
                        app,
                        actor,
                        tracker,
                        tracked_work_in_flight,
                        tracked_work_result_tx,
                        &id,
                        &completed_lookup,
                    );
                    Ok(())
                }
            } else {
                Err(AppError::NotFound(format!(
                    "tracked download {requested_id}"
                )))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::RetryImport { id, reply } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.reset_for_import_retry();
                Ok(())
            } else {
                Err(AppError::NotFound(format!(
                    "tracked download {requested_id}"
                )))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::AssignTitle {
            id,
            title_id,
            reply,
        } => {
            let requested_id = id;
            let id = resolve_tracked_command_id(tracker, &requested_id);
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {requested_id} is busy processing"
                ))));
                return;
            }
            let title = match app.services.catalog.titles.get_by_id(&title_id).await {
                Ok(Some(title)) => title,
                Ok(None) => {
                    let _ = reply.send(Err(AppError::NotFound(format!("title {title_id}"))));
                    return;
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                    return;
                }
            };

            let result = if let Some(td) = tracker.find_mut(&id) {
                crate::tracked_downloads::assign_title_to_tracked_download(app, td, &title).await;
                Ok(())
            } else {
                Err(AppError::NotFound(format!(
                    "tracked download {requested_id}"
                )))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::Snapshot { ids, reply } => {
            let snapshot = ids
                .into_iter()
                .filter_map(|id| {
                    let resolved_id = resolve_tracked_command_id(tracker, &id);
                    tracker
                        .find(&resolved_id)
                        .map(|tracked| (id, tracked_download_queue_snapshot(tracked)))
                })
                .collect();
            let _ = reply.send(snapshot);
        }
    }
}
fn prepare_tracked_download_background_work_dispatch(
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
) -> Option<(TrackedDownloadBackgroundWorkKind, TrackedDownload)> {
    let td = tracker.find_mut(id)?;
    match td.state {
        TrackedDownloadState::ImportPending => {
            if td
                .no_video_import_retry
                .as_ref()
                .is_some_and(|retry| retry.next_retry_at > chrono::Utc::now())
            {
                return None;
            }
            crate::completed_download_handler::mark_importing(td);
            Some((TrackedDownloadBackgroundWorkKind::Import, td.clone()))
        }
        TrackedDownloadState::FailedPending => {
            Some((TrackedDownloadBackgroundWorkKind::Failed, td.clone()))
        }
        _ => None,
    }
}

fn trackable_import_work_completed_lookup_items(
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    trackable_ids: &[String],
) -> Vec<DownloadQueueItem> {
    let now = chrono::Utc::now();
    trackable_ids
        .iter()
        .filter(|id| !tracked_work_in_flight.contains(*id))
        .filter_map(|id| {
            tracker.find(id).and_then(|td| {
                if td.state == TrackedDownloadState::ImportPending
                    && td
                        .no_video_import_retry
                        .as_ref()
                        .is_none_or(|retry| retry.next_retry_at <= now)
                {
                    Some(td.client_item.clone())
                } else {
                    None
                }
            })
        })
        .collect()
}

async fn build_tracked_download_work_drain(
    app: &AppUseCase,
    tracker: &crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    trackable_ids: &[String],
    excluded_client_type_refs: &[&str],
) -> TrackedDownloadWorkDrain {
    let import_lookup_items = trackable_import_work_completed_lookup_items(
        tracker,
        tracked_work_in_flight,
        trackable_ids,
    );
    let completed_lookup = if !import_lookup_items.is_empty() {
        crate::completed_download_handler::load_completed_download_lookup_for_tracked_client_items_excluding_client_types(
            app,
            &import_lookup_items,
            DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT,
            excluded_client_type_refs,
        )
        .await
        .unwrap_or_default()
    } else {
        crate::completed_download_handler::CompletedDownloadLookup::default()
    };

    TrackedDownloadWorkDrain::new(trackable_ids.to_vec(), completed_lookup)
}

fn prepare_next_tracked_download_background_work_dispatch(
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &HashSet<String>,
    drain: &mut TrackedDownloadWorkDrain,
) -> Option<(String, TrackedDownloadBackgroundWorkKind, TrackedDownload)> {
    if tracked_work_in_flight.len() >= TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT {
        return None;
    }

    while let Some(id) = drain.pending_ids.pop_front() {
        if !drain.attempted_ids.insert(id.clone()) {
            continue;
        }
        if tracked_work_in_flight.contains(&id) {
            continue;
        }
        if let Some((kind, tracked)) =
            prepare_tracked_download_background_work_dispatch(tracker, &id)
        {
            return Some((id, kind, tracked));
        }
    }

    None
}

fn try_dispatch_next_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    drain: &mut TrackedDownloadWorkDrain,
) -> bool {
    let Some((id, kind, tracked)) = prepare_next_tracked_download_background_work_dispatch(
        tracker,
        tracked_work_in_flight,
        drain,
    ) else {
        return false;
    };

    dispatch_prepared_tracked_download_background_work(
        app,
        actor,
        tracked_work_in_flight,
        result_tx,
        &id,
        kind,
        tracked,
        drain.completed_lookup.clone(),
    );
    true
}

fn try_dispatch_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    id: &str,
    completed_lookup: &crate::completed_download_handler::CompletedDownloadLookup,
) -> bool {
    if tracked_work_in_flight.len() >= TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT
        || tracked_work_in_flight.contains(id)
    {
        return false;
    }

    let Some((kind, tracked)) = prepare_tracked_download_background_work_dispatch(tracker, id)
    else {
        return false;
    };

    dispatch_prepared_tracked_download_background_work(
        app,
        actor,
        tracked_work_in_flight,
        result_tx,
        id,
        kind,
        tracked,
        completed_lookup.clone(),
    );
    true
}

#[expect(
    clippy::too_many_arguments,
    reason = "dispatch wiring carries state needed by both manual and drain dispatch paths"
)]
fn dispatch_prepared_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    id: &str,
    kind: TrackedDownloadBackgroundWorkKind,
    tracked: TrackedDownload,
    completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
) {
    tracing::info!(
        id = %id,
        work = kind.as_str(),
        active_workers = tracked_work_in_flight.len() + 1,
        worker_limit = TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT,
        "tracked: dispatched background work"
    );
    tracked_work_in_flight.insert(id.to_string());
    dispatch_tracked_download_background_work(
        app.clone(),
        actor.clone(),
        tracked,
        kind,
        result_tx.clone(),
        completed_lookup,
    );
}
fn dispatch_tracked_download_background_work(
    app: AppUseCase,
    actor: User,
    tracked: crate::tracked_downloads::TrackedDownload,
    kind: TrackedDownloadBackgroundWorkKind,
    result_tx: tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    completed_lookup: crate::completed_download_handler::CompletedDownloadLookup,
) {
    tokio::spawn(async move {
        let started_at = Instant::now();
        let tracked_id = tracked.id.clone();
        let worker = tokio::spawn(async move {
            let mut tracked = tracked;

            match kind {
                TrackedDownloadBackgroundWorkKind::Import => {
                    let _ = crate::completed_download_handler::import_with_lookup(
                        &app,
                        &actor,
                        &mut tracked,
                        &completed_lookup,
                    )
                    .await;
                }
                TrackedDownloadBackgroundWorkKind::Failed => {
                    crate::failed_download_handler::process_failed(&app, &mut tracked).await;
                }
            }

            tracked
        });

        let outcome = match worker.await {
            Ok(tracked) => {
                tracing::info!(
                    id = %tracked.id,
                    work = kind.as_str(),
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    final_state = tracked.state.as_str(),
                    "tracked: background work completed"
                );
                Ok(tracked)
            }
            Err(error) => {
                let message = format!(
                    "tracked {} worker exited before completion: {}",
                    kind.as_str(),
                    error
                );
                tracing::error!(
                    id = %tracked_id,
                    work = kind.as_str(),
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    error = %error,
                    "tracked: background work crashed"
                );
                Err(message)
            }
        };
        let elapsed = started_at.elapsed();
        if result_tx
            .send(TrackedDownloadBackgroundWorkResult {
                id: tracked_id,
                kind,
                outcome,
                elapsed,
            })
            .is_err()
        {
            tracing::debug!(
                work = kind.as_str(),
                "tracked background work result dropped after poller shutdown"
            );
        }
    });
}
async fn handle_tracked_download_background_work_result(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result: TrackedDownloadBackgroundWorkResult,
) {
    tracked_work_in_flight.remove(&result.id);

    let Some(tracked) = tracker.find_mut(&result.id) else {
        tracing::debug!(
            id = %result.id,
            work = result.kind.as_str(),
            elapsed_ms = result.elapsed.as_millis() as u64,
            "tracked background work finished after tracker entry disappeared"
        );
        return;
    };

    let state = match result.outcome {
        Ok(finished) => {
            merge_tracked_download_background_work_state(tracked, finished);
            tracked.state
        }
        Err(message) => {
            tracked.status = TrackedDownloadStatus::Error;
            tracked.status_messages.clear();
            tracked.status_messages.push(message);
            match result.kind {
                TrackedDownloadBackgroundWorkKind::Import => {
                    tracked.import_attempted = true;
                    tracked.state = TrackedDownloadState::ImportBlocked;
                    TrackedDownloadState::ImportBlocked
                }
                TrackedDownloadBackgroundWorkKind::Failed => {
                    tracked.state = TrackedDownloadState::Failed;
                    TrackedDownloadState::Failed
                }
            }
        }
    };

    if state.is_terminal() {
        tracing::info!(
            id = %result.id,
            state = state.as_str(),
            work = result.kind.as_str(),
            "tracked: persisting worker terminal state"
        );
        let persisted = tracker.persist_terminal_state(app, &result.id, state).await;
        if persisted {
            finalize_tracked_terminal_state(app, tracker, &result.id, state).await;
        }
    }

    publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
}
async fn finalize_tracked_terminal_state(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
    state: TrackedDownloadState,
) {
    let Some(td) = tracker.find(id) else {
        return;
    };

    let cleanup =
        crate::import::import::reconcile_terminal_download_cleanup_for_tracked(app, td, state)
            .await;
    if crate::import::import::terminal_download_cleanup_is_complete(cleanup) {
        tracker.stop_tracking(id);
    }
}
async fn reconcile_terminal_tracked_downloads(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
) {
    reconcile_duplicate_terminal_source_states(tracker);

    let terminal_ids: Vec<(String, TrackedDownloadState)> = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| tracked.state.is_terminal())
        .map(|tracked| (tracked.id.clone(), tracked.state))
        .collect();

    for (id, state) in terminal_ids {
        finalize_tracked_terminal_state(app, tracker, &id, state).await;
    }
}

fn reconcile_duplicate_terminal_source_states(
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
) {
    let mut terminal_source_states = HashMap::new();
    for tracked in tracker.get_all() {
        if !tracked.state.is_terminal() {
            continue;
        }
        let Some(source_identity) = tracked_download_source_identity(&tracked) else {
            continue;
        };
        let should_replace = terminal_source_states
            .get(&source_identity)
            .is_none_or(|existing| {
                terminal_state_precedence(tracked.state) > terminal_state_precedence(*existing)
            });
        if should_replace {
            terminal_source_states.insert(source_identity, tracked.state);
        }
    }

    if terminal_source_states.is_empty() {
        return;
    }

    let updates: Vec<(String, crate::DownloadSourceIdentity, TrackedDownloadState)> = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| !tracked.state.is_terminal())
        .filter_map(|tracked| {
            let source_identity = tracked_download_source_identity(&tracked)?;
            terminal_source_states
                .get(&source_identity)
                .copied()
                .map(|state| (tracked.id.clone(), source_identity, state))
        })
        .collect();

    for (id, source_identity, state) in updates {
        let Some(tracked) = tracker.find_mut(&id) else {
            continue;
        };
        tracing::info!(
            id = %tracked.id,
            client_id = tracked.client_id.as_str(),
            client_type = tracked.client_type.as_str(),
            download_client_item_id = source_identity.item_id.as_str(),
            from = ?tracked.state,
            to = ?state,
            "tracked: reconciling duplicate terminal source state"
        );
        apply_reconciled_terminal_state(tracked, state);
    }
}

fn tracked_download_source_identity(
    tracked: &TrackedDownload,
) -> Option<crate::DownloadSourceIdentity> {
    let client_type = tracked.client_type.trim();
    let item_id = tracked.client_item.download_client_item_id.trim();
    if client_type.is_empty() || item_id.is_empty() {
        return None;
    }
    Some(crate::DownloadSourceIdentity::new(
        Some(tracked.client_id.as_str()),
        client_type,
        item_id,
    ))
}

fn terminal_state_precedence(state: TrackedDownloadState) -> u8 {
    match state {
        TrackedDownloadState::Imported => 3,
        TrackedDownloadState::Failed => 2,
        TrackedDownloadState::Ignored => 1,
        _ => 0,
    }
}

fn apply_reconciled_terminal_state(tracked: &mut TrackedDownload, state: TrackedDownloadState) {
    tracked.state = state;
    match state {
        TrackedDownloadState::Imported => {
            tracked.status = TrackedDownloadStatus::Ok;
            tracked.status_messages.clear();
        }
        TrackedDownloadState::Failed => {
            tracked.status = TrackedDownloadStatus::Error;
        }
        _ => {}
    }
}
