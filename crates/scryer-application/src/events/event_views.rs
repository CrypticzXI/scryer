use crate::library_scan_progress::{
    reduce_library_scan_projection_event, replay_library_scan_projection,
};
use crate::{
    ActivityChannel, ActivityEvent, ActivityKind, ActivitySeverity, DownloadQueueItem, JobKey,
    JobRun, JobRunStatus, JobTriggerSource, LibraryScanSession, LibraryScanStatus,
};
use chrono::{DateTime, Utc};
use scryer_domain::{
    ConfigurationChangeAction, DomainEvent, DomainEventPayload, DownloadQueueItemRemovedEventData,
    DownloadQueueItemUpsertedEventData, DownloadQueueState, EventType, HistoryEvent,
    ImportRejectedEventData, ImportStatus, JobNextRunUpdatedEventData, MediaFacet,
    MediaFileDeletedReason, MetadataHydrationState, PostProcessingResult, TitleHistoryEventType,
    TitleHistoryRecord,
};
use std::{cmp::Ordering, collections::HashMap};

fn default_activity_channels() -> Vec<ActivityChannel> {
    vec![ActivityChannel::WebUi]
}

pub(crate) fn activity_event_from_domain_event(event: &DomainEvent) -> Option<ActivityEvent> {
    let (kind, severity, message) = match &event.payload {
        DomainEventPayload::TitleAdded(data) => (
            ActivityKind::TitleAdded,
            ActivitySeverity::Success,
            format!("Added '{}' to Scryer.", data.title.title_name),
        ),
        DomainEventPayload::TitleUpdated(data) => (
            ActivityKind::TitleUpdated,
            ActivitySeverity::Info,
            format!("Updated '{}'.", data.title.title_name),
        ),
        DomainEventPayload::TitleRematched(data) => (
            ActivityKind::TitleUpdated,
            ActivitySeverity::Info,
            format!(
                "Rematched '{}' to TVDB {}.",
                data.title.title_name, data.new_tvdb_id
            ),
        ),
        DomainEventPayload::TitleDeleted(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!("Deleted '{}' from Scryer.", data.title.title_name),
        ),
        DomainEventPayload::ConfigurationChanged(data) => (
            ActivityKind::SettingSaved,
            ActivitySeverity::Success,
            configuration_changed_message(
                data.resource_type.as_str(),
                data.resource_id.as_deref(),
                data.action,
            ),
        ),
        DomainEventPayload::DiscoverySearchCompleted(data) => (
            ActivityKind::MovieFetched,
            ActivitySeverity::Info,
            discovery_search_completed_message(
                data.search_type.as_str(),
                data.query.as_deref(),
                data.result_count,
            ),
        ),
        DomainEventPayload::MetadataHydrationUpdated(data) => metadata_hydration_activity(
            data.state,
            data.title.title_name.as_str(),
            data.reason.as_deref(),
        ),
        DomainEventPayload::ReleaseGrabbed(data) => (
            ActivityKind::AcquisitionCandidateAccepted,
            ActivitySeverity::Success,
            data.source_title
                .as_ref()
                .map(|source_title| {
                    format!(
                        "Grabbed '{}' for '{}'.",
                        source_title, data.title.title_name
                    )
                })
                .unwrap_or_else(|| format!("Grabbed a release for '{}'.", data.title.title_name)),
        ),
        DomainEventPayload::DownloadFailed(data) => (
            ActivityKind::AcquisitionDownloadFailed,
            ActivitySeverity::Warning,
            data.source_title
                .as_ref()
                .map(|source_title| format!("Download failed for '{}'.", source_title))
                .or_else(|| {
                    data.title
                        .as_ref()
                        .map(|title| format!("Download failed for '{}'.", title.title_name))
                })
                .unwrap_or_else(|| "Download failed.".to_string()),
        ),
        DomainEventPayload::ImportCompleted(data) => (
            if data.title.facet == MediaFacet::Movie {
                ActivityKind::MovieDownloaded
            } else {
                ActivityKind::SeriesEpisodeImported
            },
            ActivitySeverity::Success,
            format!(
                "Imported {} file{} for '{}'.",
                data.imported_count,
                if data.imported_count == 1 { "" } else { "s" },
                data.title.title_name
            ),
        ),
        DomainEventPayload::ImportRejected(data) => (
            ActivityKind::ImportRejected,
            ActivitySeverity::Warning,
            import_rejected_message(data),
        ),
        DomainEventPayload::MediaFileImported(data) => (
            if data.title.facet == MediaFacet::Movie {
                ActivityKind::MovieDownloaded
            } else {
                ActivityKind::SeriesEpisodeImported
            },
            ActivitySeverity::Success,
            format!("Imported media file for '{}'.", data.title.title_name),
        ),
        DomainEventPayload::MediaFileRenamed(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!(
                "Renamed {} file(s) for '{}'.",
                data.renamed_count, data.title.title_name
            ),
        ),
        DomainEventPayload::MediaFileDeleted(data) => (
            ActivityKind::SystemNotice,
            if matches!(data.reason, MediaFileDeletedReason::UpgradeCleanup) {
                ActivitySeverity::Info
            } else {
                ActivitySeverity::Warning
            },
            data.media_updates
                .first()
                .map(|update| {
                    if matches!(data.reason, MediaFileDeletedReason::UpgradeCleanup) {
                        format!("Removed old media file during upgrade: {}", update.path)
                    } else {
                        format!("Deleted media file from disk: {}", update.path)
                    }
                })
                .unwrap_or_else(|| format!("Deleted media file for '{}'.", data.title.title_name)),
        ),
        DomainEventPayload::MediaFileUpgraded(data) => (
            ActivityKind::FileUpgraded,
            ActivitySeverity::Success,
            match (data.old_score, data.new_score) {
                (Some(old_score), Some(new_score)) => format!(
                    "Upgraded file for '{}': score {} → {} (delta +{})",
                    data.title.title_name,
                    old_score,
                    new_score,
                    new_score - old_score
                ),
                _ => format!("Upgraded file for '{}'.", data.title.title_name),
            },
        ),
        DomainEventPayload::AcquisitionSearchCompleted(data) => (
            ActivityKind::AcquisitionSearchCompleted,
            ActivitySeverity::Info,
            format!(
                "{} results for '{}'",
                data.result_count, data.title.title_name
            ),
        ),
        DomainEventPayload::AcquisitionCandidateRejected(data) => (
            ActivityKind::AcquisitionCandidateRejected,
            ActivitySeverity::Info,
            format!(
                "{}: '{}' ({})",
                data.reason_code, data.source_title, data.title.title_name
            ),
        ),
        DomainEventPayload::ImportRequested(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            import_requested_message(data.client_type.as_str(), data.source_ref.as_str()),
        ),
        DomainEventPayload::ImportRecoveryCompleted(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Warning,
            format!(
                "{} stale import(s) recovered as failed — check import history",
                data.recovered_count
            ),
        ),
        DomainEventPayload::DownloadQueueItemCommandIssued(data) => (
            ActivityKind::SystemNotice,
            ActivitySeverity::Info,
            format!(
                "download {}: {}",
                download_queue_command_label(data.action),
                data.item_id
            ),
        ),
        DomainEventPayload::PostProcessingCompleted(data) => (
            ActivityKind::PostProcessingCompleted,
            post_processing_severity(data.result),
            post_processing_message(
                data.script_name.as_str(),
                data.title.title_name.as_str(),
                data.result,
                data.exit_code,
            ),
        ),
        DomainEventPayload::SubtitleDownloaded(data) => (
            ActivityKind::SubtitleDownloaded,
            ActivitySeverity::Success,
            format!("Downloaded subtitle for '{}'.", data.title.title_name),
        ),
        DomainEventPayload::SubtitleSearchFailed(data) => (
            ActivityKind::SubtitleSearchFailed,
            ActivitySeverity::Warning,
            format!("Subtitle search failed for '{}'.", data.title.title_name),
        ),
        _ => return None,
    };

    Some(ActivityEvent {
        id: event.event_id.clone(),
        kind,
        severity,
        channels: default_activity_channels(),
        actor_user_id: event.actor_user_id.clone(),
        title_id: event.title_id.clone(),
        facet: event.facet.as_ref().map(|facet| facet.as_str().to_string()),
        message,
        occurred_at: event.occurred_at,
    })
}

pub(crate) fn title_history_records_from_domain_event(
    event: &DomainEvent,
) -> Vec<TitleHistoryRecord> {
    let Some(title_id) = event.title_id.clone() else {
        return Vec::new();
    };

    let (event_type, source_title, quality, download_id) = match &event.payload {
        DomainEventPayload::ReleaseGrabbed(data) => (
            TitleHistoryEventType::Grabbed,
            data.source_title.clone(),
            None,
            data.download_id.clone(),
        ),
        DomainEventPayload::ImportCompleted(data) => (
            TitleHistoryEventType::Imported,
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            None,
            None,
        ),
        DomainEventPayload::ImportRejected(data) => (
            match data.status {
                ImportStatus::Failed => TitleHistoryEventType::ImportFailed,
                ImportStatus::Skipped => TitleHistoryEventType::ImportSkipped,
                _ => return Vec::new(),
            },
            data.source_path.clone(),
            None,
            None,
        ),
        DomainEventPayload::MediaFileDeleted(data) => (
            TitleHistoryEventType::FileDeleted,
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            None,
            None,
        ),
        DomainEventPayload::MediaFileRenamed(data) => (
            TitleHistoryEventType::FileRenamed,
            (data.media_updates.len() == 1)
                .then(|| data.media_updates.first().map(|update| update.path.clone()))
                .flatten(),
            None,
            None,
        ),
        DomainEventPayload::TitleRematched(_) => {
            (TitleHistoryEventType::Rematched, None, None, None)
        }
        DomainEventPayload::TitleUpdated(_) => {
            return Vec::new();
        }
        _ => return Vec::new(),
    };

    let data_json = match &event.payload {
        DomainEventPayload::TitleRematched(data) => serde_json::to_string(data).ok(),
        _ => serde_json::to_string(&event.payload).ok(),
    };
    let episode_ids = event_episode_ids(event);
    if episode_ids.is_empty() {
        return vec![TitleHistoryRecord {
            id: event.event_id.clone(),
            title_id,
            episode_id: None,
            collection_id: None,
            event_type,
            source_title,
            quality,
            download_id,
            data_json,
            occurred_at: event.occurred_at.to_rfc3339(),
            created_at: event.occurred_at.to_rfc3339(),
        }];
    }

    episode_ids
        .into_iter()
        .map(|episode_id| TitleHistoryRecord {
            id: event.event_id.clone(),
            title_id: title_id.clone(),
            episode_id: Some(episode_id),
            collection_id: None,
            event_type,
            source_title: source_title.clone(),
            quality: quality.clone(),
            download_id: download_id.clone(),
            data_json: data_json.clone(),
            occurred_at: event.occurred_at.to_rfc3339(),
            created_at: event.occurred_at.to_rfc3339(),
        })
        .collect()
}

pub(crate) fn history_event_from_domain_event(event: &DomainEvent) -> Option<HistoryEvent> {
    let activity = activity_event_from_domain_event(event)?;
    let event_type = match &event.payload {
        DomainEventPayload::TitleAdded(_) => EventType::TitleAdded,
        DomainEventPayload::TitleUpdated(_) => EventType::TitleUpdated,
        DomainEventPayload::TitleRematched(_) => EventType::TitleUpdated,
        DomainEventPayload::MediaFileUpgraded(_) => EventType::FileUpgraded,
        DomainEventPayload::DownloadFailed(_)
        | DomainEventPayload::ImportRejected(_)
        | DomainEventPayload::SubtitleSearchFailed(_) => EventType::Error,
        _ => EventType::ActionCompleted,
    };

    Some(HistoryEvent {
        id: event.event_id.clone(),
        event_type,
        actor_user_id: event.actor_user_id.clone(),
        title_id: event.title_id.clone(),
        message: activity.message,
        occurred_at: event.occurred_at,
    })
}

#[cfg(test)]
fn title_history_records_for_episode_from_domain_events(
    events: &[DomainEvent],
    episode_id: &str,
    limit: usize,
) -> Vec<TitleHistoryRecord> {
    let mut records = events
        .iter()
        .flat_map(title_history_records_from_domain_event)
        .filter(|record| record.episode_id.as_deref() == Some(episode_id))
        .collect::<Vec<_>>();
    records.sort_by(|left, right| right.occurred_at.cmp(&left.occurred_at));
    records.truncate(limit);
    records
}

pub fn replay_library_scan_state(events: &[DomainEvent]) -> HashMap<String, LibraryScanSession> {
    replay_library_scan_projection(events)
}

pub fn replay_active_job_runs(events: &[DomainEvent]) -> HashMap<String, JobRun> {
    let mut runs = HashMap::new();
    let mut scans = HashMap::new();
    for event in events {
        apply_library_scan_event(&mut scans, event);
        apply_job_run_event(&mut runs, &scans, event);
    }
    runs
}

pub fn replay_download_queue_state(events: &[DomainEvent]) -> HashMap<String, DownloadQueueItem> {
    let mut items = HashMap::new();
    for event in events {
        apply_download_queue_event(&mut items, event);
    }
    items
}

fn import_rejected_message(data: &ImportRejectedEventData) -> String {
    match data.status {
        ImportStatus::Skipped => data
            .reason
            .clone()
            .unwrap_or_else(|| "Import skipped.".to_string()),
        ImportStatus::Failed => data
            .reason
            .clone()
            .unwrap_or_else(|| "Import failed.".to_string()),
        _ => data
            .reason
            .clone()
            .unwrap_or_else(|| "Import rejected.".to_string()),
    }
}

fn event_episode_ids(event: &DomainEvent) -> Vec<String> {
    let mut ids = Vec::new();
    let iter = match &event.payload {
        DomainEventPayload::ReleaseGrabbed(data) => data.episode_ids.iter(),
        DomainEventPayload::ImportCompleted(data) => data.episode_ids.iter(),
        DomainEventPayload::ImportRejected(data) => data.episode_ids.iter(),
        DomainEventPayload::MediaFileRenamed(data) => data.episode_ids.iter(),
        DomainEventPayload::MediaFileDeleted(data) => data.episode_ids.iter(),
        _ => return ids,
    };

    for episode_id in iter {
        if !ids.contains(episode_id) {
            ids.push(episode_id.clone());
        }
    }
    ids
}

fn configuration_changed_message(
    resource_type: &str,
    resource_id: Option<&str>,
    action: ConfigurationChangeAction,
) -> String {
    let target = resource_id.unwrap_or(resource_type);
    match action {
        ConfigurationChangeAction::Saved => format!("{target} saved"),
        ConfigurationChangeAction::Updated => format!("{target} updated"),
        ConfigurationChangeAction::Deleted => format!("{target} deleted"),
        ConfigurationChangeAction::Reordered => format!("{target} reordered"),
    }
}

fn discovery_search_completed_message(
    search_type: &str,
    query: Option<&str>,
    result_count: i64,
) -> String {
    match query.filter(|value| !value.trim().is_empty()) {
        Some(query) => format!("{search_type} searched: {query} ({result_count} results)"),
        None => format!("{search_type} search completed ({result_count} results)"),
    }
}

fn metadata_hydration_activity(
    state: MetadataHydrationState,
    title_name: &str,
    reason: Option<&str>,
) -> (ActivityKind, ActivitySeverity, String) {
    match state {
        MetadataHydrationState::Started => (
            ActivityKind::MetadataHydrationStarted,
            ActivitySeverity::Info,
            format!("hydrating metadata for {title_name}"),
        ),
        MetadataHydrationState::Completed => (
            ActivityKind::MetadataHydrationCompleted,
            ActivitySeverity::Success,
            format!("metadata hydrated for {title_name}"),
        ),
        MetadataHydrationState::Failed => (
            ActivityKind::MetadataHydrationFailed,
            ActivitySeverity::Warning,
            match reason.filter(|value| !value.trim().is_empty()) {
                Some(reason) => format!("metadata hydration failed for {title_name}: {reason}"),
                None => format!("metadata hydration failed for {title_name}"),
            },
        ),
    }
}

fn import_requested_message(client_type: &str, source_ref: &str) -> String {
    format!("manual import queued for {client_type} ({source_ref})")
}

fn post_processing_severity(result: PostProcessingResult) -> ActivitySeverity {
    match result {
        PostProcessingResult::Succeeded => ActivitySeverity::Success,
        PostProcessingResult::TimedOut | PostProcessingResult::Failed => ActivitySeverity::Warning,
    }
}

fn post_processing_message(
    script_name: &str,
    title_name: &str,
    result: PostProcessingResult,
    exit_code: Option<i32>,
) -> String {
    match result {
        PostProcessingResult::Succeeded => {
            format!("Post-processing '{script_name}' succeeded for '{title_name}'")
        }
        PostProcessingResult::TimedOut => {
            format!("Post-processing '{script_name}' timed out for '{title_name}'")
        }
        PostProcessingResult::Failed => format!(
            "Post-processing '{script_name}' failed (exit {}) for '{title_name}'",
            exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "n/a".to_string())
        ),
    }
}

fn download_queue_command_label(action: scryer_domain::DownloadQueueCommandAction) -> &'static str {
    match action {
        scryer_domain::DownloadQueueCommandAction::Pause => "paused",
        scryer_domain::DownloadQueueCommandAction::Resume => "resumed",
        scryer_domain::DownloadQueueCommandAction::Delete => "delete queued",
    }
}

fn apply_library_scan_event(
    sessions: &mut HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<LibraryScanSession> {
    reduce_library_scan_projection_event(sessions, event)
}

pub fn apply_library_scan_projection_event(
    sessions: &mut HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<LibraryScanSession> {
    apply_library_scan_event(sessions, event)
}

fn apply_job_run_event(
    runs: &mut HashMap<String, JobRun>,
    scans: &HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<JobRun> {
    fn merge_scan_status(run: &mut JobRun, session: LibraryScanSession) {
        run.library_scan_progress = Some(session.clone());
        run.status = match session.status {
            LibraryScanStatus::Discovering => JobRunStatus::Discovering,
            LibraryScanStatus::Running => JobRunStatus::Running,
            LibraryScanStatus::Completed => JobRunStatus::Completed,
            LibraryScanStatus::Canceled => JobRunStatus::Warning,
            LibraryScanStatus::Warning => JobRunStatus::Warning,
            LibraryScanStatus::Failed => JobRunStatus::Failed,
        };
        if run.status.is_terminal() {
            run.completed_at = Some(session.updated_at);
        }
    }

    match &event.payload {
        DomainEventPayload::JobRunStarted(data) => {
            let job_key = JobKey::parse(&data.job_key)?;
            let run = JobRun {
                id: data.run_id.clone(),
                job_key,
                display_name: job_key.display_name().to_string(),
                category: job_key.category(),
                section: job_key.section(),
                status: if job_key.uses_library_scan_progress() {
                    JobRunStatus::Discovering
                } else {
                    JobRunStatus::Running
                },
                trigger_source: JobTriggerSource::parse(&data.trigger_source)
                    .unwrap_or(JobTriggerSource::SystemInternal),
                started_at: event.occurred_at,
                completed_at: None,
                summary_json: None,
                summary_text: None,
                error_text: None,
                progress_json: None,
                library_scan_progress: scans.get(&data.run_id).cloned(),
            };
            runs.insert(data.run_id.clone(), run.clone());
            Some(run)
        }
        DomainEventPayload::JobRunCompleted(data) => {
            let mut run = runs.remove(&data.run_id)?;
            run.summary_text = data.summary_text.clone();
            run.completed_at = Some(event.occurred_at);
            run.status = JobRunStatus::Completed;
            Some(run)
        }
        DomainEventPayload::JobRunFailed(data) => {
            let mut run = runs.remove(&data.run_id)?;
            run.error_text = data.error_text.clone();
            run.summary_text = data
                .error_text
                .clone()
                .map(|error| format!("Failed: {error}"));
            run.completed_at = Some(event.occurred_at);
            run.status = JobRunStatus::Failed;
            Some(run)
        }
        DomainEventPayload::LibraryScanStarted(_)
        | DomainEventPayload::LibraryScanProgressed(_)
        | DomainEventPayload::LibraryScanCompleted(_)
        | DomainEventPayload::LibraryScanCanceled(_)
        | DomainEventPayload::LibraryScanFailed(_) => {
            let session_id = match &event.payload {
                DomainEventPayload::LibraryScanStarted(data) => data.session_id.as_str(),
                DomainEventPayload::LibraryScanProgressed(data) => data.session_id.as_str(),
                DomainEventPayload::LibraryScanCompleted(data) => data.session_id.as_str(),
                DomainEventPayload::LibraryScanCanceled(data) => data.session_id.as_str(),
                DomainEventPayload::LibraryScanFailed(data) => data.session_id.as_str(),
                _ => unreachable!(),
            };
            if let Some(run) = runs.get_mut(session_id) {
                if let Some(scan) = scans.get(session_id).cloned() {
                    merge_scan_status(run, scan);
                    return Some(run.clone());
                }
                let mut projected_scans = HashMap::new();
                if let Some(scan) = run.library_scan_progress.clone() {
                    projected_scans.insert(session_id.to_string(), scan);
                }
                if let Some(scan) =
                    reduce_library_scan_projection_event(&mut projected_scans, event)
                {
                    merge_scan_status(run, scan);
                }
                Some(run.clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn apply_job_run_projection_event(
    runs: &mut HashMap<String, JobRun>,
    scans: &HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<JobRun> {
    apply_job_run_event(runs, scans, event)
}

pub fn replay_job_next_runs(events: &[DomainEvent]) -> HashMap<JobKey, DateTime<Utc>> {
    let mut next_runs = HashMap::new();
    for event in events {
        apply_job_next_run_event(&mut next_runs, event);
    }
    next_runs
}

pub fn apply_job_next_run_projection_event(
    next_runs: &mut HashMap<JobKey, DateTime<Utc>>,
    event: &DomainEvent,
) -> Option<(JobKey, Option<DateTime<Utc>>)> {
    apply_job_next_run_event(next_runs, event)
}

fn apply_job_next_run_event(
    next_runs: &mut HashMap<JobKey, DateTime<Utc>>,
    event: &DomainEvent,
) -> Option<(JobKey, Option<DateTime<Utc>>)> {
    let DomainEventPayload::JobNextRunUpdated(JobNextRunUpdatedEventData {
        job_key,
        next_run_at,
    }) = &event.payload
    else {
        return None;
    };

    let job_key = JobKey::parse(job_key)?;
    let next_run_at = next_run_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));

    match next_run_at {
        Some(next_run_at) => {
            next_runs.insert(job_key, next_run_at);
            Some((job_key, Some(next_run_at)))
        }
        None => {
            next_runs.remove(&job_key);
            Some((job_key, None))
        }
    }
}

fn download_queue_item_key(item: &DownloadQueueItem) -> String {
    if item.client_id.trim().is_empty() {
        return format!("{}::{}", item.client_type, item.download_client_item_id);
    }

    format!("{}::{}", item.client_id, item.download_client_item_id)
}

fn apply_download_queue_event(
    items: &mut HashMap<String, DownloadQueueItem>,
    event: &DomainEvent,
) -> Option<Vec<DownloadQueueItem>> {
    match &event.payload {
        DomainEventPayload::DownloadQueueItemUpserted(DownloadQueueItemUpsertedEventData {
            item,
        }) => {
            items.insert(download_queue_item_key(item), item.clone());
            Some(sorted_download_queue_items(items))
        }
        DomainEventPayload::DownloadQueueItemRemoved(DownloadQueueItemRemovedEventData {
            download_client_item_id,
            client_id,
            client_type,
        }) => {
            if let Some(client_id) = client_id.as_ref().filter(|value| !value.trim().is_empty()) {
                items.remove(&format!("{client_id}::{download_client_item_id}"));
            } else if let Some(client_type) = client_type.as_ref() {
                items.remove(&format!("{client_type}::{download_client_item_id}"));
            } else {
                items.retain(|_, item| item.download_client_item_id != *download_client_item_id);
            }
            Some(sorted_download_queue_items(items))
        }
        _ => None,
    }
}

pub fn apply_download_queue_projection_event(
    items: &mut HashMap<String, DownloadQueueItem>,
    event: &DomainEvent,
) -> Option<Vec<DownloadQueueItem>> {
    apply_download_queue_event(items, event)
}

pub fn sorted_download_queue_items(
    items: &HashMap<String, DownloadQueueItem>,
) -> Vec<DownloadQueueItem> {
    let mut values = items.values().cloned().collect::<Vec<_>>();
    sort_download_queue_items(&mut values);
    values
}

pub fn sort_download_queue_items(items: &mut [DownloadQueueItem]) {
    items.sort_by(compare_download_queue_items);
}

pub fn compare_download_queue_items(
    left: &DownloadQueueItem,
    right: &DownloadQueueItem,
) -> Ordering {
    let left_rank = queue_state_sort_rank(&left.state);
    let right_rank = queue_state_sort_rank(&right.state);
    if left_rank != right_rank {
        return left_rank.cmp(&right_rank);
    }

    match left.state {
        DownloadQueueState::Downloading
        | DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => right
            .progress_percent
            .cmp(&left.progress_percent)
            .then_with(|| left.id.cmp(&right.id)),
        DownloadQueueState::Queued | DownloadQueueState::Paused => {
            compare_queue_sort_values(left.queued_at.as_deref(), right.queued_at.as_deref())
                .then_with(|| left.id.cmp(&right.id))
        }
        _ => compare_queue_sort_values(
            right.last_updated_at.as_deref(),
            left.last_updated_at.as_deref(),
        )
        .then_with(|| left.id.cmp(&right.id)),
    }
}

fn compare_queue_sort_values(left: Option<&str>, right: Option<&str>) -> Ordering {
    fn parse(value: Option<&str>) -> i64 {
        value
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
    }

    parse(left).cmp(&parse(right))
}

fn queue_state_sort_rank(state: &DownloadQueueState) -> u8 {
    match state {
        DownloadQueueState::Downloading
        | DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => 0,
        DownloadQueueState::Queued => 1,
        DownloadQueueState::Paused => 2,
        DownloadQueueState::ImportPending | DownloadQueueState::Completed => 3,
        DownloadQueueState::Failed => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use scryer_domain::{
        DomainEventStream, DomainExternalIds, DownloadQueueState, ImportCompletedEventData,
        JobRunStartedEventData, LibraryScanCompletedEventData, LibraryScanProgressedEventData,
        MediaFacet, MediaPathUpdate, MediaUpdateType, TitleContextSnapshot,
    };

    fn title_snapshot(name: &str, facet: MediaFacet) -> TitleContextSnapshot {
        TitleContextSnapshot {
            title_name: name.to_string(),
            facet,
            external_ids: DomainExternalIds::default(),
            poster_url: None,
            year: Some(2024),
        }
    }

    fn event(
        sequence: i64,
        occurred_at: DateTime<Utc>,
        payload: DomainEventPayload,
    ) -> DomainEvent {
        DomainEvent {
            sequence,
            event_id: format!("evt-{sequence}"),
            occurred_at,
            actor_user_id: None,
            title_id: Some("title-1".to_string()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Global,
            payload,
        }
    }

    fn queue_item(
        id: &str,
        state: DownloadQueueState,
        progress_percent: u8,
        queued_at: i64,
        last_updated_at: i64,
    ) -> DownloadQueueItem {
        DownloadQueueItem {
            id: id.to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Example".to_string(),
            facet: None,
            client_id: "client-1".to_string(),
            client_name: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            state,
            progress_percent,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: Some(queued_at.to_string()),
            last_updated_at: Some(last_updated_at.to_string()),
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        }
    }

    #[test]
    fn episode_history_returns_most_recent_records_first() {
        let now = Utc::now();
        let events = vec![
            event(
                1,
                now,
                DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                    title: title_snapshot("Example", MediaFacet::Series),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/old.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    imported_count: 1,
                    episode_ids: vec!["ep-1".to_string()],
                }),
            ),
            event(
                2,
                now + Duration::seconds(60),
                DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                    title: title_snapshot("Example", MediaFacet::Series),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/new.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    imported_count: 1,
                    episode_ids: vec!["ep-1".to_string()],
                }),
            ),
        ];

        let records = title_history_records_for_episode_from_domain_events(&events, "ep-1", 10);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].source_title.as_deref(), Some("/data/new.mkv"));
        assert_eq!(records[1].source_title.as_deref(), Some("/data/old.mkv"));
    }

    #[test]
    fn sorted_download_queue_items_matches_query_ordering_contract() {
        let items = HashMap::from([
            (
                "queued".to_string(),
                queue_item("queued", DownloadQueueState::Queued, 0, 10, 10),
            ),
            (
                "failed".to_string(),
                queue_item("failed", DownloadQueueState::Failed, 0, 10, 50),
            ),
            (
                "completed-newer".to_string(),
                queue_item("completed-newer", DownloadQueueState::Completed, 0, 10, 30),
            ),
            (
                "downloading-fast".to_string(),
                queue_item(
                    "downloading-fast",
                    DownloadQueueState::Downloading,
                    80,
                    10,
                    10,
                ),
            ),
            (
                "downloading-slower".to_string(),
                queue_item(
                    "downloading-slower",
                    DownloadQueueState::Downloading,
                    35,
                    10,
                    10,
                ),
            ),
            (
                "completed-older".to_string(),
                queue_item("completed-older", DownloadQueueState::Completed, 0, 10, 20),
            ),
        ]);

        let ordered = sorted_download_queue_items(&items)
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ordered,
            vec![
                "downloading-fast".to_string(),
                "downloading-slower".to_string(),
                "queued".to_string(),
                "completed-newer".to_string(),
                "completed-older".to_string(),
                "failed".to_string(),
            ]
        );
    }

    #[test]
    fn terminal_library_scan_event_updates_job_run_projection() {
        let now = Utc::now();
        let run_id = "run-1";
        let mut runs = HashMap::new();
        let mut scans = HashMap::new();

        let started = event(
            1,
            now,
            DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                run_id: run_id.to_string(),
                job_key: JobKey::BackgroundLibraryRefreshSeries.as_str().to_string(),
                operation_type: "library_scan".to_string(),
                trigger_source: JobTriggerSource::Manual.as_str().to_string(),
            }),
        );
        let progress = event(
            2,
            now + Duration::seconds(5),
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: run_id.to_string(),
                status: "running".to_string(),
                found_titles: 3,
                title_match_completed: 2,
                title_match_total_known: false,
                titles_completed: 2,
                titles_total: Some(5),
                files_completed: 4,
                files_total: Some(9),
                warning_message: None,
            }),
        );
        let completed = event(
            3,
            now + Duration::seconds(10),
            DomainEventPayload::LibraryScanCompleted(LibraryScanCompletedEventData {
                session_id: run_id.to_string(),
                status: "completed".to_string(),
                found_titles: 5,
                title_match_completed: 5,
                title_match_total_known: true,
                titles_completed: 5,
                titles_total: Some(5),
                files_completed: 9,
                files_total: Some(9),
                summary: None,
                warning_message: None,
            }),
        );

        let run = apply_job_run_projection_event(&mut runs, &scans, &started)
            .expect("job start should create a run");
        assert_eq!(run.status, JobRunStatus::Discovering);

        let _ = apply_library_scan_projection_event(&mut scans, &progress);
        let running = apply_job_run_projection_event(&mut runs, &scans, &progress)
            .expect("progress should update the run");
        assert_eq!(running.status, JobRunStatus::Running);

        let _ = apply_library_scan_projection_event(&mut scans, &completed);
        let terminal = apply_job_run_projection_event(&mut runs, &scans, &completed)
            .expect("terminal scan event should update the run");
        assert_eq!(terminal.status, JobRunStatus::Completed);
        assert!(
            terminal
                .library_scan_progress
                .as_ref()
                .is_some_and(|scan| scan.status == LibraryScanStatus::Completed)
        );
        assert_eq!(terminal.completed_at, Some(completed.occurred_at));
    }

    #[test]
    fn completed_library_scan_event_replays_summary_counts() {
        let now = Utc::now();
        let completed = event(
            1,
            now,
            DomainEventPayload::LibraryScanCompleted(LibraryScanCompletedEventData {
                session_id: "session-1".to_string(),
                status: "completed".to_string(),
                found_titles: 3,
                title_match_completed: 3,
                title_match_total_known: true,
                titles_completed: 2,
                titles_total: Some(2),
                files_completed: 3,
                files_total: Some(3),
                summary: Some(scryer_domain::LibraryScanSummaryEventData {
                    scanned: 3,
                    matched: 2,
                    imported: 2,
                    skipped: 1,
                    unmatched: 0,
                }),
                warning_message: None,
            }),
        );

        let mut scans = HashMap::new();
        let session = apply_library_scan_projection_event(&mut scans, &completed)
            .expect("completed event should project a terminal session");

        assert_eq!(session.status, LibraryScanStatus::Completed);
        assert_eq!(
            session.summary.as_ref().map(|summary| summary.imported),
            Some(2)
        );
        assert_eq!(
            session.summary.as_ref().map(|summary| summary.skipped),
            Some(1)
        );
    }
}
