/// If subtitles.auto_download_on_import is enabled, spawn a background subtitle search.
fn maybe_trigger_subtitle_search(app: &AppUseCase, title_id: &str, media_file_id: &str) {
    let app = app.clone();
    let title_id = title_id.to_string();
    let media_file_id = media_file_id.to_string();
    tokio::spawn(async move {
        let auto = app
            .subtitle_settings()
            .await
            .ok()
            .map(|settings| settings.auto_download_on_import)
            .unwrap_or(false);
        if auto {
            crate::spawn_subtitle_search_for_file(app, title_id, media_file_id);
        }
    });
}
fn completed_download_identity(completed: &CompletedDownload) -> DownloadSourceIdentity {
    DownloadSourceIdentity::new(
        Some(completed.client_id.as_str()),
        &completed.client_type,
        &completed.download_client_item_id,
    )
}
fn merge_scryer_origin_parameters(
    parameters: &mut Vec<(String, String)>,
    title_id: String,
    facet: String,
    collection_id: Option<String>,
) {
    upsert_parameter(parameters, "*scryer_title_id", title_id);
    upsert_parameter(parameters, "*scryer_facet", facet);
    if let Some(collection_id) = collection_id {
        upsert_parameter(parameters, "*scryer_collection_id", collection_id);
    }
}
fn upsert_parameter(parameters: &mut Vec<(String, String)>, key: &str, value: String) {
    if let Some((_, existing_value)) = parameters.iter_mut().find(|(name, _)| name == key) {
        *existing_value = value;
    } else {
        parameters.push((key.to_string(), value));
    }
}
async fn persist_completed_download_tracked_state(
    app: &AppUseCase,
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
    state: TrackedDownloadState,
) {
    if !state.is_terminal() {
        return;
    }
    let state_identity = match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched) => {
            submission_source_identity(&matched.submission)
        }
        _ => completed_download_identity(completed),
    };

    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .update_tracked_state(&state_identity, state.as_str())
        .await
    {
        tracing::warn!(
            error = %error,
            client_id = completed.client_id.as_str(),
            client_type = completed.client_type.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            tracked_state_client_item_id = state_identity.item_id.as_str(),
            state = state.as_str(),
            "failed to persist completed download terminal state"
        );
    }

    let observed_identity = completed_download_observed_identity(completed);
    let download_identity = match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched) => matched
            .identity
            .clone()
            .filter(|identity| !download_submission_identity_is_empty(identity))
            .or_else(|| {
                (!download_submission_identity_is_empty(&observed_identity))
                    .then_some(observed_identity.clone())
            }),
        CompletedDownloadSubmissionResolution::MissingDownloadId { identity } => {
            Some(identity.clone())
        }
        _ => (!download_submission_identity_is_empty(&observed_identity))
            .then_some(observed_identity.clone()),
    };

    if let Some(download_identity) = download_identity
        && let Err(error) = app
            .services
            .workflow
            .download_submissions
            .record_identity_tracked_state(
                &download_identity,
                Some(&completed_download_identity(completed)),
                state.as_str(),
                None,
                None,
            )
            .await
    {
        tracing::warn!(
            error = %error,
            client_id = completed.client_id.as_str(),
            client_type = completed.client_type.as_str(),
            download_client_item_id = completed.download_client_item_id.as_str(),
            state = state.as_str(),
            "failed to persist durable completed download terminal state"
        );
    }
}
async fn terminal_download_item_is_still_visible(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    is_history: bool,
) -> bool {
    let lookup = if is_history {
        app.services
            .integrations
            .download_client
            .list_history()
            .await
    } else {
        app.services.integrations.download_client.list_queue().await
    };

    match lookup {
        Ok(items) => items.iter().any(|item| {
            item.download_client_item_id == download_client_item_id
                && item.client_type.eq_ignore_ascii_case(client_type)
                && (client_id.is_empty() || item.client_id.trim() == client_id)
        }),
        Err(error) => {
            tracing::warn!(
                error = %error,
                client_id,
                client_type,
                download_client_item_id,
                is_history,
                "failed to confirm download item visibility after delete error"
            );
            true
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalDownloadCleanupOutcome {
    NotConfigured,
    Removed,
    AlreadyGone,
    RetryableFailure,
}
pub(crate) fn terminal_download_cleanup_is_complete(
    outcome: TerminalDownloadCleanupOutcome,
) -> bool {
    matches!(
        outcome,
        TerminalDownloadCleanupOutcome::NotConfigured
            | TerminalDownloadCleanupOutcome::Removed
            | TerminalDownloadCleanupOutcome::AlreadyGone
    )
}
async fn cleanup_routing_scope_for_title_id(
    app: &AppUseCase,
    title_id: Option<&str>,
) -> (Option<String>, Option<MediaFacet>) {
    let Some(title_id) = title_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };

    match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => (Some(title.library_id), Some(title.facet)),
        Ok(None) | Err(_) => (None, None),
    }
}
pub(crate) async fn reconcile_terminal_download_cleanup_for_completed(
    app: &AppUseCase,
    completed: &CompletedDownload,
    state: TrackedDownloadState,
) -> TerminalDownloadCleanupOutcome {
    let title_id = extract_parameter(&completed.parameters, "*scryer_title_id").unwrap_or_default();
    let (library_id, resolved_facet) =
        cleanup_routing_scope_for_title_id(app, Some(title_id.as_str())).await;
    let facet = resolved_facet.or_else(|| facet_for_completed_download(completed));
    reconcile_terminal_download_cleanup(
        app,
        &completed.client_id,
        &completed.client_type,
        &completed.download_client_item_id,
        library_id.as_deref(),
        facet.as_ref(),
        state,
    )
    .await
}
pub(crate) async fn reconcile_terminal_download_cleanup_for_tracked(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    state: TrackedDownloadState,
) -> TerminalDownloadCleanupOutcome {
    let (library_id, resolved_facet) =
        cleanup_routing_scope_for_title_id(app, tracked.title_id.as_deref()).await;
    let facet = resolved_facet.or_else(|| facet_from_tracked_label(tracked.facet.as_deref()));
    reconcile_terminal_download_cleanup(
        app,
        &tracked.client_id,
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
        library_id.as_deref(),
        facet.as_ref(),
        state,
    )
    .await
}
fn media_file_score(file: &crate::TitleMediaFile) -> i32 {
    file.acquisition_score.unwrap_or(0)
}
fn completed_import_error_message_is_retryable(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "active-download marker",
        "still being unpacked",
        "still_unpacking",
        "source changed",
        "locked",
        "temporarily",
        "not found or inaccessible",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}
async fn resolve_import_quality_profile(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> crate::QualityProfile {
    let tvdb_id = title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == "tvdb")
        .map(|external_id| external_id.value.as_str());
    let category_hint = crate::post_download_gate::facet_to_category_hint(&title.facet);
    match app
        .resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
            title_tags: &title.tags,
            library_id: Some(title.library_id.as_str()),
            imdb_id: title.imdb_id.as_deref(),
            tvdb_id,
            category_hint: Some(category_hint),
        })
        .await
    {
        Ok(profile) => profile,
        Err(err) => {
            tracing::warn!(
                error = %err,
                title_id = %title.id,
                "failed to resolve quality profile, using default"
            );
            crate::default_quality_profile_for_search()
        }
    }
}
async fn resolve_import_audio_persona(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> (Vec<String>, crate::ScoringPersona) {
    let category_hint = crate::post_download_gate::facet_to_category_hint(&title.facet);
    let required_audio_languages = app
        .resolve_required_audio_languages(
            Some(&title.id),
            Some(title.library_id.as_str()),
            Some(category_hint),
        )
        .await
        .unwrap_or_default();
    let persona = app
        .resolve_scoring_persona(Some(title.library_id.as_str()), Some(category_hint))
        .await
        .unwrap_or_default();

    (required_audio_languages, persona)
}
const SAMPLE_SIZE_THRESHOLD: u64 = 50 * 1024 * 1024;
fn non_empty_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        if value.trim().is_empty() {
            None
        } else {
            Some(value)
        }
    })
}
