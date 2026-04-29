use crate::{
    AppUseCase, NotificationAppPayload, NotificationDownloadPayload, NotificationEpisodePayload,
    NotificationExternalIdsPayload, NotificationFilePayload, NotificationImportPayload,
    NotificationMediaUpdatePayload, NotificationMediaUpdateTypePayload, NotificationPayload,
    NotificationReleasePayload, NotificationTitlePayload,
};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventType, DownloadFailedEventData,
    ImportCompletedEventData, ImportRejectedEventData, MediaFileDeletedEventData,
    MediaFileDeletedReason, MediaFileRenamedEventData, MediaFileUpgradedEventData, MediaPathUpdate,
    MediaUpdateType,
    NotificationEventType, PostProcessingCompletedEventData, PostProcessingResult,
    ReleaseGrabbedEventData, SubtitleDownloadedEventData, SubtitleSearchFailedEventData,
    TitleAddedEventData, TitleContextSnapshot, TitleDeletedEventData,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

const NOTIFICATION_SUBSCRIBER: &str = "notification_dispatcher";
const NOTIFICATION_BATCH_LIMIT: usize = 100;

macro_rules! notification_event_mappings {
    ($macro:ident $(, $extra:expr)*) => {
        $macro! {
            $($extra,)*
            title_added => DomainEventPayload::TitleAdded(_) => DomainEventPayload::TitleAdded(data) => DomainEventType::TitleAdded => NotificationEventType::TitleAdded => build_title_added_notification(data),
            title_deleted => DomainEventPayload::TitleDeleted(_) => DomainEventPayload::TitleDeleted(data) => DomainEventType::TitleDeleted => NotificationEventType::TitleDeleted => build_title_deleted_notification(data),
            release_grabbed => DomainEventPayload::ReleaseGrabbed(_) => DomainEventPayload::ReleaseGrabbed(data) => DomainEventType::ReleaseGrabbed => NotificationEventType::Grab => build_release_grabbed_notification(data),
            download_failed => DomainEventPayload::DownloadFailed(_) => DomainEventPayload::DownloadFailed(data) => DomainEventType::DownloadFailed => NotificationEventType::Download => build_download_failed_notification(data),
            import_completed => DomainEventPayload::ImportCompleted(_) => DomainEventPayload::ImportCompleted(data) => DomainEventType::ImportCompleted => NotificationEventType::ImportComplete => build_import_completed_notification(data),
            import_rejected => DomainEventPayload::ImportRejected(_) => DomainEventPayload::ImportRejected(data) => DomainEventType::ImportRejected => NotificationEventType::ImportRejected => build_import_rejected_notification(data),
            media_file_upgraded => DomainEventPayload::MediaFileUpgraded(_) => DomainEventPayload::MediaFileUpgraded(data) => DomainEventType::MediaFileUpgraded => NotificationEventType::Upgrade => build_media_file_upgraded_notification(data),
            media_file_renamed => DomainEventPayload::MediaFileRenamed(_) => DomainEventPayload::MediaFileRenamed(data) => DomainEventType::MediaFileRenamed => NotificationEventType::Rename => build_media_file_renamed_notification(data),
            media_file_deleted_upgrade => DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData { reason: MediaFileDeletedReason::UpgradeCleanup, .. }) => DomainEventPayload::MediaFileDeleted(data @ MediaFileDeletedEventData { reason: MediaFileDeletedReason::UpgradeCleanup, .. }) => DomainEventType::MediaFileDeleted => NotificationEventType::FileDeletedForUpgrade => build_media_file_deleted_notification(data, NotificationEventType::FileDeletedForUpgrade),
            media_file_deleted => DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData { reason: MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk, .. }) => DomainEventPayload::MediaFileDeleted(data @ MediaFileDeletedEventData { reason: MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk, .. }) => DomainEventType::MediaFileDeleted => NotificationEventType::FileDeleted => build_media_file_deleted_notification(data, NotificationEventType::FileDeleted),
            post_processing_completed => DomainEventPayload::PostProcessingCompleted(_) => DomainEventPayload::PostProcessingCompleted(data) => DomainEventType::PostProcessingCompleted => NotificationEventType::PostProcessingCompleted => build_post_processing_completed_notification(data),
            subtitle_downloaded => DomainEventPayload::SubtitleDownloaded(_) => DomainEventPayload::SubtitleDownloaded(data) => DomainEventType::SubtitleDownloaded => NotificationEventType::SubtitleDownloaded => build_subtitle_downloaded_notification(data),
            subtitle_search_failed => DomainEventPayload::SubtitleSearchFailed(_) => DomainEventPayload::SubtitleSearchFailed(data) => DomainEventType::SubtitleSearchFailed => NotificationEventType::SubtitleSearchFailed => build_subtitle_search_failed_notification(data),
        }
    };
}

macro_rules! notification_domain_event_type_list {
    ($( $name:ident => $type_pattern:pat => $build_pattern:pat => $domain_event_type:expr => $notification_event_type:expr => $builder:expr, )*) => {
        const NOTIFICATION_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
            $( $domain_event_type, )*
        ];
    };
}

notification_event_mappings!(notification_domain_event_type_list);

macro_rules! notification_event_type_match {
    ($payload:expr, $( $name:ident => $type_pattern:pat => $build_pattern:pat => $domain_event_type:expr => $notification_event_type:expr => $builder:expr, )*) => {
        match $payload {
            $( $type_pattern => Some($notification_event_type), )*
            _ => None,
        }
    };
}

macro_rules! notification_build_match {
    ($payload:expr, $( $name:ident => $type_pattern:pat => $build_pattern:pat => $domain_event_type:expr => $notification_event_type:expr => $builder:expr, )*) => {
        match $payload {
            $( $build_pattern => Some($builder), )*
            _ => None,
        }
    };
}

pub async fn start_notification_dispatcher(app: AppUseCase, cancel: CancellationToken) {
    info!("notification dispatcher started");
    let repo = app.services.events.domain_events.clone();
    let mut rx = app.runtime.events.notification_event_broadcast.subscribe();
    let mut last_sequence = match repo.get_subscriber_offset(NOTIFICATION_SUBSCRIBER).await {
        Ok(sequence) => sequence,
        Err(error) => {
            warn!(error = %error, "failed to load notification subscriber offset; starting at 0");
            0
        }
    };
    // Send-side filtering keeps operational bursts from waking this dispatcher, but persisted
    // filtered replay stays authoritative. The broadcast payload is only a high-water hint used
    // to avoid needless catch-up queries when we already processed that range.
    let mut should_poll = true;

    loop {
        if should_poll {
            match dispatch_pending_events(&app, last_sequence).await {
                Ok(sequence) => last_sequence = sequence,
                Err(error) => {
                    warn!(error = %error, "notification dispatcher failed to process pending events")
                }
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("notification dispatcher shutting down");
                break;
            }
            result = rx.recv() => {
                match result {
                    Ok(high_water_sequence) => {
                        should_poll = high_water_sequence > last_sequence;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "notification dispatcher lagged, resyncing from persisted domain events");
                        should_poll = true;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("notification event broadcast closed, notification dispatcher exiting");
                        break;
                    }
                }
            }
        }
    }
}

async fn dispatch_pending_events(
    app: &AppUseCase,
    mut after_sequence: i64,
) -> crate::AppResult<i64> {
    let repo = app.services.events.domain_events.clone();

    loop {
        let events = repo
            .list(&DomainEventFilter {
                event_types: Some(NOTIFICATION_DOMAIN_EVENT_TYPES.to_vec()),
                after_sequence: Some(after_sequence),
                limit: NOTIFICATION_BATCH_LIMIT,
                ..DomainEventFilter::default()
            })
            .await?;
        if events.is_empty() {
            break;
        }

        for event in events {
            dispatch_event(app, &event).await;
            after_sequence = event.sequence;
            repo.set_subscriber_offset(NOTIFICATION_SUBSCRIBER, after_sequence)
                .await?;
        }
    }

    Ok(after_sequence)
}

pub(crate) fn notification_event_type(
    payload: &DomainEventPayload,
) -> Option<NotificationEventType> {
    notification_event_mappings!(notification_event_type_match, payload)
}

async fn dispatch_event(app: &AppUseCase, event: &DomainEvent) {
    let Some(notification) = build_notification(event) else {
        return;
    };

    let sub_repo = match app.notification_subscriptions_repo() {
        Ok(repo) => repo,
        Err(_) => return,
    };
    let ch_repo = match app.notification_channels_repo() {
        Ok(repo) => repo,
        Err(_) => return,
    };
    let Some(provider) = app.services.notifications.notification_provider() else {
        return;
    };

    let event_type = event.payload.event_type();
    debug!(
        event_type = event_type.as_str(),
        title_id = ?event.title_id,
        sequence = event.sequence,
        "dispatching domain-event-backed notification"
    );

    let mut subscriptions = Vec::new();
    for subscription_event_type in subscription_event_types(notification.payload.event_type) {
        match sub_repo
            .list_subscriptions_for_event(subscription_event_type)
            .await
        {
            Ok(mut matching) => subscriptions.append(&mut matching),
            Err(error) => {
                warn!(
                    error = %error,
                    event_type = subscription_event_type.as_str(),
                    "failed to list notification subscriptions"
                );
                return;
            }
        }
    }
    subscriptions.sort_by(|left, right| left.id.cmp(&right.id));
    subscriptions.dedup_by(|left, right| left.id == right.id);

    for subscription in subscriptions {
        if !subscription.is_enabled {
            continue;
        }

        if !matches_scope(
            &subscription.scope,
            subscription.scope_id.as_deref(),
            event.title_id.as_deref(),
            event.facet.as_ref().map(|facet| facet.as_str()),
        ) {
            continue;
        }

        let channel = match ch_repo.get_channel(&subscription.channel_id).await {
            Ok(Some(channel)) if channel.is_enabled => channel,
            _ => continue,
        };

        let client = match provider.client_for_channel(&channel) {
            Some(client) => client,
            None => {
                warn!(
                    channel_type = channel.channel_type.as_str(),
                    channel_name = channel.name.as_str(),
                    "no notification plugin available for channel type"
                );
                continue;
            }
        };

        match client.send_notification(&notification.payload).await {
            Ok(()) => {
                info!(
                    event_type = event_type.as_str(),
                    plugin_event_type = notification.payload.event_type.as_str(),
                    channel = channel.name.as_str(),
                    "notification dispatched"
                );
            }
            Err(error) => {
                warn!(
                    event_type = event_type.as_str(),
                    plugin_event_type = notification.payload.event_type.as_str(),
                    channel = channel.name.as_str(),
                    error = %error,
                    "notification dispatch failed"
                );
            }
        }
    }
}

struct BuiltNotification {
    payload: NotificationPayload,
}

fn build_notification(event: &DomainEvent) -> Option<BuiltNotification> {
    notification_event_mappings!(notification_build_match, &event.payload)
}

fn build_title_added_notification(data: &TitleAddedEventData) -> BuiltNotification {
    BuiltNotification {
        payload: base_notification_payload(
            NotificationEventType::TitleAdded,
            format!("Added: {}", data.title.title_name),
            format!("Added '{}' to Scryer.", data.title.title_name),
            Some(&data.title),
            &[],
            &[],
        ),
    }
}

fn build_title_deleted_notification(data: &TitleDeletedEventData) -> BuiltNotification {
    BuiltNotification {
        payload: base_notification_payload(
            NotificationEventType::TitleDeleted,
            format!("Deleted: {}", data.title.title_name),
            format!("Deleted '{}' from Scryer.", data.title.title_name),
            Some(&data.title),
            &[],
            &[],
        ),
    }
}

fn build_release_grabbed_notification(data: &ReleaseGrabbedEventData) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::Grab,
        format!("Grabbed: {}", data.title.title_name),
        data
            .source_title
            .as_ref()
            .map(|source_title| {
                format!(
                    "Grabbed '{}' for '{}'.",
                    source_title, data.title.title_name
                )
            })
            .unwrap_or_else(|| format!("Grabbed a release for '{}'.", data.title.title_name)),
        Some(&data.title),
        &data.episode_ids,
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        source_title: data.source_title.clone(),
        source_hint: data.source_hint.clone(),
        ..Default::default()
    });
    payload.download = Some(NotificationDownloadPayload {
        download_id: data.download_id.clone(),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_download_failed_notification(data: &DownloadFailedEventData) -> BuiltNotification {
    let title = data
        .title
        .as_ref()
        .map(|title| title.title_name.as_str())
        .unwrap_or("Unknown title");
    let mut payload = base_notification_payload(
        NotificationEventType::Download,
        format!("Download failed: {title}"),
        data
            .reason
            .clone()
            .unwrap_or_else(|| "Download failed.".to_string()),
        data.title.as_ref(),
        &data.episode_ids,
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        source_title: data.source_title.clone(),
        source_hint: data.source_hint.clone(),
        quality: data.quality.clone(),
        ..Default::default()
    });
    payload.download = Some(NotificationDownloadPayload {
        download_id: data.download_id.clone(),
        client_id: data.client_id.clone(),
        client_name: data.client_name.clone(),
        client_type: data.client_type.clone(),
    });
    BuiltNotification { payload }
}

fn build_import_completed_notification(data: &ImportCompletedEventData) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::ImportComplete,
        format!("Import complete: {}", data.title.title_name),
        format!(
            "Imported {} file{} for '{}'.",
            data.imported_count,
            if data.imported_count == 1 { "" } else { "s" },
            data.title.title_name
        ),
        Some(&data.title),
        &data.episode_ids,
        &data.media_updates,
    );
    payload.release = Some(NotificationReleasePayload {
        source_title: data.source_title.clone(),
        quality: data.quality.clone(),
        ..Default::default()
    });
    payload.download = Some(NotificationDownloadPayload {
        client_name: data.source_system.clone(),
        ..Default::default()
    });
    payload.import = Some(NotificationImportPayload {
        import_id: data.import_id.clone(),
        source_system: data.source_system.clone(),
        source_ref: data.source_ref.clone(),
        source_title: data.source_title.clone(),
        source_path: data.source_path.clone(),
        dest_path: data.dest_path.clone(),
        imported_count: Some(data.imported_count),
        status: Some("completed".to_string()),
    });
    BuiltNotification { payload }
}

fn build_import_rejected_notification(data: &ImportRejectedEventData) -> BuiltNotification {
    let title = data
        .title
        .as_ref()
        .map(|title| title.title_name.as_str())
        .unwrap_or("Unknown title");
    let mut payload = base_notification_payload(
        NotificationEventType::ImportRejected,
        format!("Import rejected: {title}"),
        data
            .reason
            .clone()
            .unwrap_or_else(|| "Import was rejected.".to_string()),
        data.title.as_ref(),
        &data.episode_ids,
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        source_title: data.source_title.clone(),
        quality: data.quality.clone(),
        ..Default::default()
    });
    payload.import = Some(NotificationImportPayload {
        import_id: data.import_id.clone(),
        source_system: data.source_system.clone(),
        source_ref: data.source_ref.clone(),
        source_title: data.source_title.clone(),
        source_path: data.source_path.clone(),
        dest_path: data.dest_path.clone(),
        status: Some(data.status.as_str().to_string()),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_media_file_upgraded_notification(data: &MediaFileUpgradedEventData) -> BuiltNotification {
    BuiltNotification {
        payload: base_notification_payload(
            NotificationEventType::Upgrade,
            format!("Upgraded: {}", data.title.title_name),
            format!("Upgraded file for '{}'.", data.title.title_name),
            Some(&data.title),
            &[],
            &data.media_updates,
        ),
    }
}

fn build_media_file_renamed_notification(data: &MediaFileRenamedEventData) -> BuiltNotification {
    BuiltNotification {
        payload: base_notification_payload(
            NotificationEventType::Rename,
            format!("Renamed: {}", data.title.title_name),
            format!(
            "Renamed {} file(s) for '{}'.",
            data.renamed_count, data.title.title_name
        ),
            Some(&data.title),
            &data.episode_ids,
            &data.media_updates,
        ),
    }
}

fn build_media_file_deleted_notification(
    data: &MediaFileDeletedEventData,
    event_type: NotificationEventType,
) -> BuiltNotification {
    let first_path = data
        .media_updates
        .first()
        .map(|update| update.path.as_str());
    let title = match data.reason {
        MediaFileDeletedReason::UpgradeCleanup => {
            format!("Deleted for upgrade: {}", data.title.title_name)
        }
        MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk => {
            format!("File deleted: {}", data.title.title_name)
        }
    };
    let body = match data.reason {
        MediaFileDeletedReason::UpgradeCleanup => format!(
            "Removed old media file during upgrade: {}",
            first_path.unwrap_or("(path unavailable)")
        ),
        MediaFileDeletedReason::Deleted | MediaFileDeletedReason::MissingOnDisk => {
            format!(
                "Deleted media file from disk: {}",
                first_path.unwrap_or("(path unavailable)")
            )
        }
    };

    BuiltNotification {
        payload: base_notification_payload(
            event_type,
            title,
            body,
            Some(&data.title),
            &data.episode_ids,
            &data.media_updates,
        ),
    }
}

fn build_post_processing_completed_notification(
    data: &PostProcessingCompletedEventData,
) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::PostProcessingCompleted,
        format!("Post-processing: {}", data.title.title_name),
        match data.result {
            PostProcessingResult::Succeeded => format!(
                "Post-processing '{}' succeeded for '{}'.",
                data.script_name, data.title.title_name
            ),
            PostProcessingResult::TimedOut => format!(
                "Post-processing '{}' timed out for '{}'.",
                data.script_name, data.title.title_name
            ),
            PostProcessingResult::Failed => format!(
                "Post-processing '{}' failed for '{}'.",
                data.script_name, data.title.title_name
            ),
        },
        Some(&data.title),
        &[],
        &[],
    );
    payload.import = Some(NotificationImportPayload {
        status: Some(
            match data.result {
                PostProcessingResult::Succeeded => "succeeded",
                PostProcessingResult::TimedOut => "timed_out",
                PostProcessingResult::Failed => "failed",
            }
            .to_string(),
        ),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn build_subtitle_downloaded_notification(data: &SubtitleDownloadedEventData) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::SubtitleDownloaded,
        format!("Subtitle downloaded: {}", data.title.title_name),
        data.language.as_deref().map_or_else(
            || format!("Downloaded subtitle for '{}'.", data.title.title_name),
            |language| {
                format!(
                    "Downloaded {language} subtitle for '{}'.",
                    data.title.title_name
                )
            },
        ),
        Some(&data.title),
        &[],
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        provider: data.provider.clone(),
        language: data.language.clone(),
        ..Default::default()
    });
    payload.file = Some(NotificationFilePayload {
        primary_path: data.subtitle_path.clone(),
        media_updates: Vec::new(),
    });
    BuiltNotification { payload }
}

fn build_subtitle_search_failed_notification(
    data: &SubtitleSearchFailedEventData,
) -> BuiltNotification {
    let mut payload = base_notification_payload(
        NotificationEventType::SubtitleSearchFailed,
        format!("Subtitle search failed: {}", data.title.title_name),
        data
            .reason
            .clone()
            .unwrap_or_else(|| format!("Subtitle search failed for '{}'.", data.title.title_name)),
        Some(&data.title),
        &[],
        &[],
    );
    payload.release = Some(NotificationReleasePayload {
        language: data.language.clone(),
        ..Default::default()
    });
    BuiltNotification { payload }
}

fn base_notification_payload(
    event_type: NotificationEventType,
    summary_title: String,
    summary_message: String,
    title: Option<&TitleContextSnapshot>,
    episode_ids: &[String],
    updates: &[MediaPathUpdate],
) -> NotificationPayload {
    NotificationPayload {
        event_type,
        summary_title,
        summary_message,
        app: NotificationAppPayload {
            name: "Scryer".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        title: title.map(title_payload),
        episode: episode_payload(episode_ids),
        release: None,
        download: None,
        import: None,
        health: None,
        file: file_payload(updates),
    }
}

fn title_payload(title: &TitleContextSnapshot) -> NotificationTitlePayload {
    NotificationTitlePayload {
        name: title.title_name.clone(),
        facet: title.facet.as_str().to_string(),
        year: title.year,
        poster_url: title.poster_url.clone(),
        external_ids: NotificationExternalIdsPayload {
            tmdb_id: title.external_ids.tmdb_id.clone(),
            imdb_id: title.external_ids.imdb_id.clone(),
            tvdb_id: title.external_ids.tvdb_id.clone(),
            anidb_id: title.external_ids.anidb_id.clone(),
        },
    }
}

fn episode_payload(episode_ids: &[String]) -> Option<NotificationEpisodePayload> {
    (!episode_ids.is_empty()).then(|| NotificationEpisodePayload {
        episode_ids: episode_ids.to_vec(),
        display: None,
    })
}

fn file_payload(updates: &[MediaPathUpdate]) -> Option<NotificationFilePayload> {
    if updates.is_empty() {
        return None;
    }

    Some(NotificationFilePayload {
        primary_path: updates.first().map(|update| update.path.clone()),
        media_updates: updates
            .iter()
            .map(|update| NotificationMediaUpdatePayload {
                path: update.path.clone(),
                update_type: match update.update_type {
                    MediaUpdateType::Created => NotificationMediaUpdateTypePayload::Created,
                    MediaUpdateType::Modified => NotificationMediaUpdateTypePayload::Modified,
                    MediaUpdateType::Deleted => NotificationMediaUpdateTypePayload::Deleted,
                },
            })
            .collect(),
    })
}

fn subscription_event_types(event_type: NotificationEventType) -> Vec<NotificationEventType> {
    match event_type {
        NotificationEventType::FileDeletedForUpgrade => vec![
            NotificationEventType::FileDeletedForUpgrade,
            NotificationEventType::FileDeleted,
        ],
        _ => vec![event_type],
    }
}

fn matches_scope(
    scope: &str,
    scope_id: Option<&str>,
    event_title_id: Option<&str>,
    event_facet: Option<&str>,
) -> bool {
    match scope {
        "global" => true,
        "facet" => match (scope_id, event_facet) {
            (Some(scope_id), Some(facet)) => scope_id == facet,
            _ => false,
        },
        "title" => match (scope_id, event_title_id) {
            (Some(scope_id), Some(title_id)) => scope_id == title_id,
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_events::new_global_domain_event;
    use crate::lib_tests::bootstrap;
    use chrono::Utc;
    use scryer_domain::{
        DomainExternalIds, DownloadFailedEventData, ImportCompletedEventData,
        ImportRejectedEventData, ImportStatus, LibraryScanProgressedEventData, MediaFacet,
        MediaFileDeletedEventData, MediaFileRenamedEventData, MediaFileUpgradedEventData,
        MediaUpdateType, PostProcessingCompletedEventData, ReleaseGrabbedEventData,
        SubtitleDownloadedEventData, SubtitleSearchFailedEventData, TitleAddedEventData,
        TitleDeletedEventData,
    };

    fn title_context(name: &str, facet: MediaFacet) -> TitleContextSnapshot {
        TitleContextSnapshot {
            title_name: name.to_string(),
            facet,
            external_ids: DomainExternalIds {
                imdb_id: Some("tt1234567".to_string()),
                tmdb_id: Some("987".to_string()),
                tvdb_id: Some("123".to_string()),
                anidb_id: None,
            },
            poster_url: Some("https://example.invalid/poster.jpg".to_string()),
            year: Some(2024),
        }
    }

    fn notification_sample_events() -> Vec<DomainEvent> {
        vec![
            DomainEvent {
                sequence: 1,
                event_id: "evt-title-added".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::TitleAdded(TitleAddedEventData {
                    title: title_context("Example Movie", MediaFacet::Movie),
                }),
            },
            DomainEvent {
                sequence: 2,
                event_id: "evt-title-deleted".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::TitleDeleted(TitleDeletedEventData {
                    title: title_context("Deleted Movie", MediaFacet::Movie),
                }),
            },
            DomainEvent {
                sequence: 3,
                event_id: "evt-release-grabbed".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                    title: title_context("Example Show", MediaFacet::Series),
                    source_title: Some("Example.Show.S01E01.1080p".to_string()),
                    source_hint: Some("rss".to_string()),
                    download_id: Some("grab-1".to_string()),
                    episode_ids: vec!["episode-1".to_string()],
                }),
            },
            DomainEvent {
                sequence: 4,
                event_id: "evt-download-failed".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: None,
                facet: None,
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                    title: Some(title_context("Broken Download", MediaFacet::Movie)),
                    source_title: Some("Broken.Download.2024".to_string()),
                    source_hint: Some("manual".to_string()),
                    download_id: None,
                    client_id: None,
                    client_name: None,
                    client_type: None,
                    quality: None,
                    reason: Some("archive corrupt".to_string()),
                    episode_ids: Vec::new(),
                    collection_id: None,
                }),
            },
            DomainEvent {
                sequence: 5,
                event_id: "evt-import-completed".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                    title: title_context("Imported Show", MediaFacet::Series),
                    media_updates: vec![MediaPathUpdate {
                        path: "/library/Imported Show/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    imported_count: 1,
                    import_id: None,
                    source_system: Some("download_client".to_string()),
                    source_ref: Some("queue-1".to_string()),
                    source_title: Some("Imported.Show.S01E01.1080p".to_string()),
                    source_path: Some("/downloads/Imported.Show.S01E01.1080p.mkv".to_string()),
                    dest_path: Some("/library/Imported Show/S01E01.mkv".to_string()),
                    quality: Some("1080p".to_string()),
                    episode_ids: vec!["episode-1".to_string()],
                }),
            },
            DomainEvent {
                sequence: 6,
                event_id: "evt-import-rejected".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::ImportRejected(ImportRejectedEventData {
                    title: Some(title_context("Rejected Movie", MediaFacet::Movie)),
                    status: ImportStatus::Failed,
                    import_id: None,
                    source_system: Some("download_client".to_string()),
                    source_ref: Some("queue-2".to_string()),
                    source_title: Some("Rejected.Movie.1080p".to_string()),
                    source_path: Some("/downloads/rejected.mkv".to_string()),
                    dest_path: None,
                    quality: Some("1080p".to_string()),
                    reason: Some("not parsable".to_string()),
                    skip_reason: None,
                    episode_ids: Vec::new(),
                }),
            },
            DomainEvent {
                sequence: 7,
                event_id: "evt-media-upgraded".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
                    title: title_context("Upgraded Movie", MediaFacet::Movie),
                    media_updates: vec![MediaPathUpdate {
                        path: "/library/Upgraded Movie/Upgraded Movie.mkv".to_string(),
                        update_type: MediaUpdateType::Modified,
                    }],
                    previous_file_id: Some("file-old".to_string()),
                    current_file_id: Some("file-new".to_string()),
                    old_score: Some(10),
                    new_score: Some(15),
                }),
            },
            DomainEvent {
                sequence: 8,
                event_id: "evt-media-renamed".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaFileRenamed(MediaFileRenamedEventData {
                    title: title_context("Renamed Show", MediaFacet::Series),
                    media_updates: vec![
                        MediaPathUpdate {
                            path: "/library/Renamed Show/Old.mkv".to_string(),
                            update_type: MediaUpdateType::Deleted,
                        },
                        MediaPathUpdate {
                            path: "/library/Renamed Show/New.mkv".to_string(),
                            update_type: MediaUpdateType::Created,
                        },
                    ],
                    renamed_count: 1,
                    episode_ids: vec!["episode-1".to_string()],
                }),
            },
            DomainEvent {
                sequence: 9,
                event_id: "evt-media-deleted".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                    title: title_context("Deleted Movie", MediaFacet::Movie),
                    media_updates: vec![MediaPathUpdate {
                        path: "/library/Deleted Movie/Deleted Movie.old.mkv".to_string(),
                        update_type: MediaUpdateType::Deleted,
                    }],
                    file_id: Some("file-old".to_string()),
                    reason: MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: Vec::new(),
                }),
            },
            DomainEvent {
                sequence: 10,
                event_id: "evt-post-processing".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Movie),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::PostProcessingCompleted(
                    PostProcessingCompletedEventData {
                        title: title_context("Post Processed Movie", MediaFacet::Movie),
                        script_name: "notify.sh".to_string(),
                        result: PostProcessingResult::Succeeded,
                        exit_code: Some(0),
                    },
                ),
            },
            DomainEvent {
                sequence: 11,
                event_id: "evt-subtitle-downloaded".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::SubtitleDownloaded(SubtitleDownloadedEventData {
                    title: title_context("Subtitle Show", MediaFacet::Series),
                    subtitle_path: Some("/library/Subtitle Show/S01E01.en.srt".to_string()),
                    language: Some("English".to_string()),
                    provider: Some("opensubtitles".to_string()),
                }),
            },
            DomainEvent {
                sequence: 12,
                event_id: "evt-subtitle-search-failed".to_string(),
                occurred_at: Utc::now(),
                actor_user_id: None,
                title_id: Some("title-1".to_string()),
                facet: Some(MediaFacet::Series),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: scryer_domain::DomainEventStream::Global,
                payload: DomainEventPayload::SubtitleSearchFailed(SubtitleSearchFailedEventData {
                    title: title_context("Subtitle Failure", MediaFacet::Series),
                    language: Some("English".to_string()),
                    reason: Some("provider timeout".to_string()),
                }),
            },
        ]
    }

    #[tokio::test]
    async fn dispatch_pending_events_replays_only_notification_events() {
        let (app, _) = bootstrap();
        let operational = new_global_domain_event(
            None,
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: "scan-1".to_string(),
                status: "running".to_string(),
                found_titles: 1,
                title_match_completed: 0,
                title_match_total_known: false,
                titles_completed: 1,
                titles_total: Some(10),
                files_completed: 1,
                files_total: Some(10),
                warning_message: None,
            }),
        );
        let notification = new_global_domain_event(
            None,
            DomainEventPayload::TitleAdded(TitleAddedEventData {
                title: title_context("Replay Fixture", MediaFacet::Movie),
            }),
        );

        app.append_domain_events(vec![operational, notification])
            .await
            .expect("events should append");

        let last_sequence = dispatch_pending_events(&app, 0)
            .await
            .expect("dispatch should replay");

        assert_eq!(last_sequence, 2);
        let offset = app
            .services
            .events
            .domain_events
            .get_subscriber_offset(NOTIFICATION_SUBSCRIBER)
            .await
            .expect("offset should load");
        assert_eq!(offset, 2);
    }

    #[test]
    fn notification_filter_list_matches_buildable_payloads() {
        let supported_events = notification_sample_events();
        let configured_event_types = NOTIFICATION_DOMAIN_EVENT_TYPES
            .iter()
            .map(|event_type| event_type.as_str().to_string())
            .collect::<std::collections::HashSet<_>>();
        let buildable_event_types = supported_events
            .iter()
            .map(|event| {
                let built = build_notification(event)
                    .expect("supported payload should build a notification");
                assert!(
                    notification_event_type(&event.payload) == Some(built.payload.event_type),
                    "notification type helper should mirror notification classification"
                );
                event.payload.event_type().as_str().to_string()
            })
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(buildable_event_types, configured_event_types);

        let unsupported = DomainEvent {
            sequence: 99,
            event_id: "evt-scan".to_string(),
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: None,
            facet: Some(MediaFacet::Movie),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: scryer_domain::DomainEventStream::Global,
            payload: DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: "scan-unsupported".to_string(),
                status: "running".to_string(),
                found_titles: 1,
                title_match_completed: 0,
                title_match_total_known: false,
                titles_completed: 1,
                titles_total: Some(5),
                files_completed: 1,
                files_total: Some(5),
                warning_message: None,
            }),
        };
        assert!(notification_event_type(&unsupported.payload).is_none());
        assert!(build_notification(&unsupported).is_none());
    }
}
