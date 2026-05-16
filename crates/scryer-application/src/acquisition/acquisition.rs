use super::*;
#[cfg(test)]
use crate::acquisition_decision_helpers::is_old_failed_grab_title;
use crate::acquisition_decision_helpers::{
    FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES, extract_grabbed_release_title,
    is_all_clients_failed_error, should_research_failed_grab,
};
use crate::acquisition_policy::{
    SearchPhase, compute_search_schedule, episode_search_window_is_open,
    parse_schedule_baseline_date,
};
use crate::acquisition_release_search::{
    ReleaseAutoDecisionCode, annotate_auto_decision, interstitial_movie_search_title,
    serialize_decision_explanation,
};
use crate::contracts::{SubmissionConflictPolicy, SubmissionScopeConflict, WantedSearchOutcome};
use crate::domain_events::{
    new_global_domain_event, new_title_domain_event, title_context_snapshot,
};
use crate::types::{
    DecisionCodeCount, PendingReleaseStatus, PendingReleaseStatusCount,
    TitleAcquisitionDiagnostics, WantedStatusCount,
};
use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    DomainEventPayload, DomainEventStream, DownloadFailedEventData, Id, NewDomainEvent,
    ReleaseBlocklistedEventData, ReleaseGrabbedEventData,
};
use std::collections::{HashMap, HashSet};
use tracing::{debug, info, trace, warn};

use crate::{JobKey, JobTriggerSource};

const MAX_STANDBY_CANDIDATES_PER_WANTED_ITEM: usize = 5;
const STANDBY_RETENTION_HOURS: i64 = 24;
const ACQUISITION_SCAN_QUIET_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

fn parsed_release_season_pack_season(parsed: &crate::ParsedReleaseMetadata) -> Option<u32> {
    parsed.episode.as_ref().and_then(|episode| {
        (episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
            && !episode.is_season_extra)
            .then_some(episode.season)
            .flatten()
    })
}

fn active_scan_facet_labels(facets: &[MediaFacet]) -> Vec<&'static str> {
    facets.iter().map(MediaFacet::as_str).collect()
}

fn episode_wanted_schedule_fields(
    baseline_date: Option<&str>,
    now: &DateTime<Utc>,
    immediate: bool,
) -> (Option<String>, String, Option<String>) {
    let normalized_baseline_date = baseline_date
        .filter(|value| parse_schedule_baseline_date(Some(value)).is_some())
        .map(str::to_string);

    let Some(valid_baseline_date) = normalized_baseline_date.clone() else {
        return (None, SearchPhase::PreAir.to_string(), None);
    };

    let schedule = compute_search_schedule(
        "episode",
        Some(valid_baseline_date.as_str()),
        "primary",
        now,
    );
    let next_search_at =
        if immediate && episode_search_window_is_open(Some(valid_baseline_date.as_str()), now) {
            Some(now.to_rfc3339())
        } else {
            Some(schedule.next_search_at)
        };

    (
        Some(valid_baseline_date),
        schedule.search_phase.to_string(),
        next_search_at,
    )
}

async fn blocked_acquisition_facets_after_quiet_wait(app: &AppUseCase) -> Vec<MediaFacet> {
    let blocked_facets = app
        .runtime
        .library
        .library_scan_tracker
        .active_facets()
        .await;
    if blocked_facets.is_empty() {
        return Vec::new();
    }

    metrics::counter!("scryer_background_acquisition_scan_owned_yields_total").increment(1);
    debug!(
        blocked_facets = ?active_scan_facet_labels(&blocked_facets),
        wait_secs = ACQUISITION_SCAN_QUIET_WAIT.as_secs(),
        "background acquisition: yielding while library scan owns active facet"
    );

    let _ = tokio::time::timeout(
        ACQUISITION_SCAN_QUIET_WAIT,
        app.runtime
            .library
            .library_scan_tracker
            .wait_for_active_facets_change(&blocked_facets),
    )
    .await;

    let blocked_facets = app
        .runtime
        .library
        .library_scan_tracker
        .active_facets()
        .await;

    if !blocked_facets.is_empty() {
        debug!(
            blocked_facets = ?active_scan_facet_labels(&blocked_facets),
            "background acquisition: deferring due wanted items for actively scanning facets"
        );
    }

    blocked_facets
}

fn candidate_is_season_pack_for_season(candidate: &IndexerSearchResult, season_num: u32) -> bool {
    let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
        return false;
    };

    parsed_release_season_pack_season(parsed) == Some(season_num)
}

fn annotated_auto_decision_code(candidate: &IndexerSearchResult) -> ReleaseAutoDecisionCode {
    candidate
        .auto_decision_code
        .as_deref()
        .and_then(ReleaseAutoDecisionCode::parse)
        .unwrap_or_else(|| {
            warn!(
                release_title = candidate.title.as_str(),
                "candidate missing auto decision annotation; defaulting to quality_blocked"
            );
            ReleaseAutoDecisionCode::QualityBlocked
        })
}

fn effective_auto_decision_code(
    candidate: &IndexerSearchResult,
    failed_source_kinds: &[DownloadSourceKind],
) -> ReleaseAutoDecisionCode {
    if let Some(source_kind) = candidate.source_kind
        && failed_source_kinds.contains(&source_kind)
    {
        return ReleaseAutoDecisionCode::DownloadClientUnavailable;
    }

    annotated_auto_decision_code(candidate)
}

async fn record_release_decision(
    app: &AppUseCase,
    item: &WantedItem,
    title: &Title,
    candidate: &IndexerSearchResult,
    decision_code: ReleaseAutoDecisionCode,
    now: &DateTime<Utc>,
) {
    let candidate_score = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.preference_score)
        .unwrap_or(0);
    let mut decision_candidate = candidate.clone();
    annotate_auto_decision(&mut decision_candidate, decision_code);
    let decision_record = ReleaseDecision {
        id: Id::new().0,
        wanted_item_id: item.id.clone(),
        title_id: title.id.clone(),
        release_title: decision_candidate.title.clone(),
        release_url: decision_candidate
            .download_url
            .clone()
            .or_else(|| decision_candidate.link.clone()),
        release_size_bytes: decision_candidate.size_bytes,
        decision_code: decision_code.as_str().to_string(),
        candidate_score,
        current_score: item.current_score,
        score_delta: item
            .current_score
            .map(|current_score| candidate_score - current_score),
        explanation_json: serialize_decision_explanation(&decision_candidate),
        created_at: now.to_rfc3339(),
    };

    let _ = app
        .services
        .workflow
        .wanted_items
        .insert_release_decision(&decision_record)
        .await;
}

impl AppUseCase {
    /// Sync the wanted_items table with current monitored state.
    /// Creates entries for monitored media without files, removes stale entries.
    pub(crate) async fn sync_wanted_state(&self) -> AppResult<()> {
        let titles = self
            .services
            .catalog
            .titles
            .list_for_matching(None, None)
            .await?;
        let now = Utc::now();

        for title in &titles {
            if !title.monitored {
                // Clean up wanted items for unmonitored titles
                if let Err(err) = self
                    .services
                    .workflow
                    .wanted_items
                    .delete_wanted_items_for_title(&title.id)
                    .await
                {
                    warn!(title_id = title.id.as_str(), error = %err, "failed to clean wanted items for unmonitored title");
                }
                continue;
            }

            if let Some(handler) = self.facet_registry.get(&title.facet) {
                if handler.has_episodes() {
                    self.sync_wanted_series(title, &now).await;
                } else {
                    self.sync_wanted_movie(title, &now).await;
                }
            }
        }

        Ok(())
    }

    async fn sync_wanted_movie(&self, title: &Title, now: &DateTime<Utc>) {
        self.sync_wanted_movie_inner(title, now, false).await;
    }

    pub(crate) async fn sync_wanted_movie_inner(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
        immediate: bool,
    ) {
        // Check if movie already has a media file
        let has_file = match self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
        {
            Ok(files) => !files.is_empty(),
            Err(_) => false,
        };

        if has_file {
            return;
        }

        // Minimum availability gate: skip search if the movie hasn't reached the
        // configured availability threshold yet.
        let availability = title.min_availability.as_deref().unwrap_or("announced");
        if !is_movie_available_for_acquisition(title, availability, now) {
            info!(
                title_id = title.id.as_str(),
                min_availability = availability,
                "skipping movie: availability threshold not reached"
            );
            return;
        }

        // Determine baseline date for search scheduling
        let baseline_date = title.first_aired.clone();

        let schedule = compute_search_schedule("movie", baseline_date.as_deref(), "primary", now);

        // When immediate=true (called from add_title), set next_search_at to now
        // so the background poller picks it up on the next 60-second tick.
        let next_search_at = if immediate {
            now.to_rfc3339()
        } else {
            schedule.next_search_at
        };

        let item = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: None,
            title_slug: None,
            title_facet: None,
            library_id: Some(title.library_id.clone()),
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: schedule.search_phase.to_string(),
            next_search_at: Some(next_search_at),
            last_search_at: None,
            search_count: 0,
            baseline_date,
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        };

        match self
            .services
            .workflow
            .wanted_items
            .ensure_wanted_item_seeded(&item)
            .await
        {
            Ok(_) => {
                info!(
                    title_id = title.id.as_str(),
                    title_name = title.name.as_str(),
                    next_search_at = item.next_search_at.as_deref().unwrap_or("none"),
                    search_phase = item.search_phase.as_str(),
                    immediate = immediate,
                    "created wanted item for movie"
                );
            }
            Err(err) => {
                warn!(title_id = title.id.as_str(), error = %err, "failed to upsert wanted item for movie");
            }
        }
    }

    async fn sync_wanted_series(&self, title: &Title, now: &DateTime<Utc>) {
        self.sync_wanted_series_inner(title, now, false).await;
    }

    /// Sync wanted items for a series. When `immediate` is true, episodes that are already
    /// inside the active search window are queued immediately; episodes without a usable
    /// air date remain unscheduled until metadata provides one.
    pub(crate) async fn sync_wanted_series_inner(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
        immediate: bool,
    ) {
        let collections = match self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(c) => c,
            Err(err) => {
                warn!(title_id = title.id.as_str(), error = %err, "failed to list collections for wanted sync");
                return;
            }
        };

        // Get existing files for the title to know which episodes already have files
        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let episodes_with_files: std::collections::HashSet<String> = existing_files
            .iter()
            .filter_map(|f| f.episode_id.clone())
            .collect();
        let mut eligible_episode_ids = HashSet::new();
        let mut eligible_interstitial_collection_ids = HashSet::new();

        for collection in &collections {
            if !collection.monitored {
                continue;
            }

            let episodes = match self
                .services
                .catalog
                .shows
                .list_episodes_for_collection(&collection.id)
                .await
            {
                Ok(eps) => eps,
                Err(_) => continue,
            };

            for episode in &episodes {
                if !episode.monitored || episodes_with_files.contains(&episode.id) {
                    continue;
                }
                eligible_episode_ids.insert(episode.id.clone());

                let (baseline_date, search_phase, next_search_at) =
                    episode_wanted_schedule_fields(episode.air_date.as_deref(), now, immediate);

                let item = WantedItem {
                    id: Id::new().0,
                    title_id: title.id.clone(),
                    title_name: None,
                    title_slug: None,
                    title_facet: None,
                    library_id: Some(title.library_id.clone()),
                    library_name: None,
                    library_slug: None,
                    episode_id: Some(episode.id.clone()),
                    collection_id: None,
                    season_number: episode.season_number.clone(),
                    episode_number: None,
                    media_type: "episode".to_string(),
                    search_phase,
                    next_search_at,
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    latest_release_decision: None,
                    mismatch_recovery_eligible: false,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                if let Err(err) = self
                    .services
                    .workflow
                    .wanted_items
                    .ensure_wanted_item_seeded(&item)
                    .await
                {
                    warn!(
                        title_id = title.id.as_str(),
                        episode_id = episode.id.as_str(),
                        error = %err,
                        "failed to upsert wanted item for episode"
                    );
                }
            }
        }

        // Generate wanted items for interstitial anime movies (franchise movies stored in Season 00)
        if title.facet == scryer_domain::MediaFacet::Anime {
            for collection in &collections {
                if collection.collection_type != CollectionType::Interstitial
                    || !collection.monitored
                {
                    continue;
                }
                // Skip if already has a file on disk
                if collection.ordered_path.is_some() {
                    continue;
                }
                let Some(ref movie) = collection.interstitial_movie else {
                    continue;
                };
                // Skip filler movies unless the user opted in
                if movie.continuity_status.as_deref() == Some("filler") {
                    let monitor_filler = self
                        .resolve_library_bool_setting(
                            "anime.monitor_filler_movies",
                            Some(&title.library_id),
                            Some(title.facet.as_str()),
                            false,
                        )
                        .await
                        .unwrap_or(false);
                    if !monitor_filler {
                        continue;
                    }
                }

                // Skip if the movie already exists as a separate Movie facet title
                // (prevents downloading the same movie twice)
                if (!movie.imdb_id.is_empty() || movie.movie_tmdb_id.is_some())
                    && let Ok(all_titles) = self
                        .services
                        .catalog
                        .titles
                        .list_for_matching(None, None)
                        .await
                {
                    let already_exists = all_titles.iter().any(|t| {
                        t.facet == scryer_domain::MediaFacet::Movie
                            && ((!movie.imdb_id.is_empty()
                                && t.imdb_id.as_deref() == Some(&movie.imdb_id))
                                || movie.movie_tmdb_id.as_deref().is_some_and(|tmdb| {
                                    t.external_ids
                                        .iter()
                                        .any(|eid| eid.source == "tmdb" && eid.value == tmdb)
                                }))
                    });
                    if already_exists {
                        trace!(
                            movie_name = movie.name.as_str(),
                            "skipping interstitial wanted item: movie exists as separate title"
                        );
                        continue;
                    }
                }
                eligible_interstitial_collection_ids.insert(collection.id.clone());

                let baseline_date = movie.digital_release_date.clone();
                let schedule =
                    compute_search_schedule("movie", baseline_date.as_deref(), "primary", now);

                let next_search_at = if immediate {
                    now.to_rfc3339()
                } else {
                    schedule.next_search_at
                };

                let item = WantedItem {
                    id: Id::new().0,
                    title_id: title.id.clone(),
                    title_name: None,
                    title_slug: None,
                    title_facet: None,
                    library_id: Some(title.library_id.clone()),
                    library_name: None,
                    library_slug: None,
                    episode_id: None,
                    collection_id: Some(collection.id.clone()),
                    season_number: Some("0".to_string()),
                    episode_number: None,
                    media_type: "interstitial_movie".to_string(),
                    search_phase: schedule.search_phase.to_string(),
                    next_search_at: Some(next_search_at),
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    latest_release_decision: None,
                    mismatch_recovery_eligible: false,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                if let Err(err) = self
                    .services
                    .workflow
                    .wanted_items
                    .ensure_wanted_item_seeded(&item)
                    .await
                {
                    warn!(
                        title_id = title.id.as_str(),
                        collection_id = collection.id.as_str(),
                        movie_name = movie.name.as_str(),
                        error = %err,
                        "failed to upsert wanted item for interstitial movie"
                    );
                }
            }
        }

        self.reconcile_series_wanted_scope(
            title,
            &eligible_episode_ids,
            &eligible_interstitial_collection_ids,
        )
        .await;
    }

    async fn reconcile_series_wanted_scope(
        &self,
        title: &Title,
        eligible_episode_ids: &HashSet<String>,
        eligible_interstitial_collection_ids: &HashSet<String>,
    ) {
        let existing_items = match self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                title_id: Some(title.id.clone()),
                limit: 5000,
                ..WantedItemsQuery::default()
            })
            .await
        {
            Ok(items) => items,
            Err(err) => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %err,
                    "failed to list existing wanted items for reconciliation"
                );
                return;
            }
        };

        let stale_episode_ids: HashSet<String> = existing_items
            .iter()
            .filter(|item| item.media_type == "episode")
            .filter_map(|item| item.episode_id.clone())
            .filter(|episode_id| !eligible_episode_ids.contains(episode_id))
            .collect();
        for episode_id in stale_episode_ids {
            if let Err(err) = self
                .services
                .workflow
                .wanted_items
                .delete_wanted_items_for_episode(&episode_id)
                .await
            {
                warn!(
                    title_id = title.id.as_str(),
                    episode_id,
                    error = %err,
                    "failed to delete stale episode wanted items during reconciliation"
                );
            }
        }

        let stale_interstitial_collection_ids: HashSet<String> = existing_items
            .iter()
            .filter(|item| item.media_type == "interstitial_movie")
            .filter_map(|item| item.collection_id.clone())
            .filter(|collection_id| !eligible_interstitial_collection_ids.contains(collection_id))
            .collect();
        for collection_id in stale_interstitial_collection_ids {
            if let Err(err) = self
                .services
                .workflow
                .wanted_items
                .delete_wanted_items_for_collection(&collection_id)
                .await
            {
                warn!(
                    title_id = title.id.as_str(),
                    collection_id,
                    error = %err,
                    "failed to delete stale interstitial wanted items during reconciliation"
                );
            }
        }
    }
}

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
    pub wanted_item: Option<WantedItem>,
    pub title_id: Option<String>,
    pub client_id: String,
    pub client_type: String,
    pub client_name: Option<String>,
    pub client_item_id: String,
    pub release_title: String,
    pub reason: String,
    pub remove_from_client_if_configured: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureHandlingOutcome {
    RecoveredFromStandby,
    RequeuedFreshSearch,
    RequeuedDeferred,
    RecordedOnly,
    AlreadyHandled,
}

#[derive(Clone, Debug, Default)]
struct FailedReleaseAttribution {
    title: Option<Title>,
    episode_ids: Vec<String>,
    collection_id: Option<String>,
}

fn push_unique_episode_id(ids: &mut Vec<String>, episode_id: Option<&str>) {
    let Some(episode_id) = episode_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };

    if !ids.iter().any(|existing| existing == episode_id) {
        ids.push(episode_id.to_string());
    }
}

fn normalized_non_empty_owned(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn release_quality_hint(source_title: Option<&str>) -> Option<String> {
    source_title.and_then(|title| crate::parse_release_metadata(title).quality)
}

async fn resolve_failed_release_attribution(
    app: &AppUseCase,
    title_id: Option<&str>,
    failed_submission: Option<&DownloadSubmission>,
    wanted_item: Option<&WantedItem>,
    failed_collection_items: Option<&[WantedItem]>,
) -> FailedReleaseAttribution {
    let title = match title_id {
        Some(title_id) => app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .ok()
            .flatten(),
        None => None,
    };

    let mut attribution = FailedReleaseAttribution {
        title,
        ..Default::default()
    };

    if let Some(submission) = failed_submission {
        if let Some(episode_ids) = submission.scope.episode_ids() {
            for episode_id in episode_ids {
                push_unique_episode_id(&mut attribution.episode_ids, Some(episode_id));
            }
        }
        attribution.collection_id = submission.scope.collection_id().map(str::to_string);
    }

    if let Some(item) = wanted_item {
        push_unique_episode_id(&mut attribution.episode_ids, item.episode_id.as_deref());
        if attribution.collection_id.is_none() {
            attribution.collection_id = item.collection_id.clone();
        }
    }

    if let Some(items) = failed_collection_items {
        for item in items {
            push_unique_episode_id(&mut attribution.episode_ids, item.episode_id.as_deref());
            if attribution.collection_id.is_none() {
                attribution.collection_id = item.collection_id.clone();
            }
        }
    }

    attribution
}

fn title_scoped_domain_event(
    title_id: Option<&str>,
    title: Option<&Title>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    if let Some(title) = title {
        return new_title_domain_event(None, title, payload);
    }

    if let Some(title_id) = title_id {
        return NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: Some(title_id.to_string()),
            facet: None,
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title_id.to_string(),
            },
            payload,
        };
    }

    new_global_domain_event(None, payload)
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
                .add(&NewBlocklistEntry {
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
                Ok(_) => {
                    blocklist_persisted = true;
                }
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

        // Fetch current queue
        if let Ok(queue) = app.services.integrations.download_client.list_queue().await {
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

        // Fetch recent history — key by download client job ID (works across all
        // clients: NZBGet, SABnzbd, Weaver).
        if let Ok(history) = app
            .services
            .integrations
            .download_client
            .list_history()
            .await
        {
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
                info!(
                    failed_count = failed_by_download_id.len(),
                    "download client snapshot: failed history items"
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
        }
    }

    /// Returns true if a release with this title is currently queued/downloading.
    pub(crate) fn is_active(&self, release_title: &str) -> bool {
        self.active_titles
            .contains(&release_title.to_ascii_lowercase())
    }

    /// If a download with this job ID failed in history with a blocklist-worthy
    /// reason, returns the failure snapshot.
    pub(crate) fn failed_item(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> Option<&FailedDownloadSnapshot> {
        self.failed_by_download_id
            .get(&download_client_item_identity(
                client_id,
                download_client_item_id,
            ))
            .or_else(|| self.failed_by_download_id.get(download_client_item_id))
    }

    fn has_active_or_completed_client_item(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> bool {
        let exact_key = download_client_item_identity(client_id, download_client_item_id);
        self.active_client_ids.contains(&exact_key)
            || self.completed_client_ids.contains(&exact_key)
            || self.active_raw_item_id_counts.get(download_client_item_id) == Some(&1)
            || self
                .completed_raw_item_id_counts
                .get(download_client_item_id)
                == Some(&1)
    }
}

fn episode_collection_id_for_wanted_item(
    item: &WantedItem,
    episode: Option<&Episode>,
) -> Option<String> {
    episode
        .and_then(|episode| episode.collection_id.clone())
        .or_else(|| item.collection_id.clone())
}

fn episode_submission_scope(episode_id: Option<String>) -> SubmissionScope {
    episode_id
        .map(|episode_id| SubmissionScope::Episode { episode_id })
        .unwrap_or(SubmissionScope::Title)
}

fn collection_submission_scope(collection_id: Option<String>) -> SubmissionScope {
    collection_id
        .map(|collection_id| SubmissionScope::Collection { collection_id })
        .unwrap_or(SubmissionScope::Title)
}

pub(crate) fn direct_download_submission_scope_for_wanted_item(
    item: &WantedItem,
    _episode: Option<&Episode>,
) -> SubmissionScope {
    match item.media_type.as_str() {
        "episode" => episode_submission_scope(item.episode_id.clone()),
        "interstitial_movie" => collection_submission_scope(item.collection_id.clone()),
        _ => SubmissionScope::Title,
    }
}

pub(crate) fn download_submission_scope_for_release_title(
    item: &WantedItem,
    episode: Option<&Episode>,
    release_title: &str,
) -> SubmissionScope {
    if item.media_type == "episode" {
        let parsed = crate::parse_release_metadata(release_title);
        if parsed.episode.as_ref().is_some_and(|episode| {
            episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        }) {
            return collection_download_submission_scope_for_wanted_item(item, episode);
        }
    }

    direct_download_submission_scope_for_wanted_item(item, episode)
}

pub(crate) fn collection_download_submission_scope_for_wanted_item(
    item: &WantedItem,
    episode: Option<&Episode>,
) -> SubmissionScope {
    match item.media_type.as_str() {
        "episode" => {
            collection_submission_scope(episode_collection_id_for_wanted_item(item, episode))
        }
        "interstitial_movie" => collection_submission_scope(item.collection_id.clone()),
        _ => SubmissionScope::Title,
    }
}

fn submission_is_active_or_completed(
    submission: &DownloadSubmission,
    dl_snapshot: &DownloadClientSnapshot,
) -> bool {
    dl_snapshot.has_active_or_completed_client_item(
        submission.download_client_id.as_deref(),
        &submission.download_client_item_id,
    )
}

pub(crate) fn submission_blocks_wanted_item(
    submission: &DownloadSubmission,
    item: &WantedItem,
    episode_collection_id: Option<&str>,
) -> bool {
    match &submission.scope {
        SubmissionScope::Orphan => false,
        SubmissionScope::Title => true,
        SubmissionScope::Episode { episode_id } => {
            item.media_type == "episode" && item.episode_id.as_deref() == Some(episode_id.as_str())
        }
        SubmissionScope::EpisodeSet { episode_ids } => {
            item.media_type == "episode"
                && item.episode_id.as_ref().is_some_and(|episode_id| {
                    episode_ids.iter().any(|candidate| candidate == episode_id)
                })
        }
        SubmissionScope::Collection { collection_id } => match item.media_type.as_str() {
            "episode" => episode_collection_id == Some(collection_id.as_str()),
            "interstitial_movie" => item.collection_id.as_deref() == Some(collection_id.as_str()),
            _ => false,
        },
    }
}

/// Check grabbed wanted items against the download client. If a grabbed
/// release has failed in the download client, blocklist it and re-queue the
/// wanted item for immediate re-search.
async fn check_grabbed_for_failures(app: &AppUseCase, dl_snapshot: &DownloadClientSnapshot) {
    let grabbed_items = match app
        .services
        .workflow
        .wanted_items
        .list_wanted_items(WantedItemsQuery {
            statuses: vec!["grabbed".into()],
            limit: 200,
            ..WantedItemsQuery::default()
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

    info!(
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

            info!(
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

fn resolved_failed_release_hint(failed_submission: Option<&DownloadSubmission>) -> Option<String> {
    failed_submission
        .and_then(|submission| normalize_release_attempt_hint(submission.source_hint.as_deref()))
}

async fn resolve_failed_collection_episode_wanted_items(
    app: &AppUseCase,
    submission: &DownloadSubmission,
) -> AppResult<Vec<WantedItem>> {
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
        .wanted_items
        .list_wanted_items(WantedItemsQuery {
            media_types: vec!["episode".into()],
            title_id: Some(submission.title_id.clone()),
            limit: 500,
            ..WantedItemsQuery::default()
        })
        .await?;

    Ok(wanted_items
        .into_iter()
        .filter(|item| {
            matches!(item.status, WantedStatus::Wanted | WantedStatus::Grabbed)
                && item
                    .episode_id
                    .as_ref()
                    .is_some_and(|episode_id| episode_ids.contains(episode_id))
        })
        .collect())
}

async fn load_recent_failed_season_pack_seasons_for_title(
    app: &AppUseCase,
    title_id: &str,
    now: &DateTime<Utc>,
) -> HashSet<u32> {
    let cutoff = *now - Duration::minutes(FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES);

    match app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(title_id, 200)
        .await
    {
        Ok(entries) => entries
            .into_iter()
            .filter_map(|entry| {
                let source_title = entry.source_title?;
                let attempted_at = crate::quality_profile::parse_published_at(&entry.attempted_at)?;
                (attempted_at >= cutoff)
                    .then(|| crate::parse_release_metadata(&source_title))
                    .and_then(|parsed| parsed_release_season_pack_season(&parsed))
            })
            .collect(),
        Err(err) => {
            warn!(
                title_id,
                error = %err,
                "failed to load recent failed season pack attempts"
            );
            HashSet::new()
        }
    }
}

pub(crate) async fn process_download_failure(
    app: &AppUseCase,
    context: DownloadFailureContext,
    snapshot: Option<&DownloadClientSnapshot>,
) -> FailureHandlingOutcome {
    let failed_submission = find_failed_submission(app, &context).await;
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
        .acquire(
            resolved_title_id.as_deref(),
            &context.client_id,
            &context.client_type,
            &context.client_item_id,
        )
        .await;

    if let Some(title_id) = resolved_title_id.as_deref() {
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
                return FailureHandlingOutcome::AlreadyHandled;
            }
            Ok(false) => {}
            Err(error) => {
                warn!(
                    title_id,
                    client_id = context.client_id.as_str(),
                    client_type = context.client_type.as_str(),
                    download_client_item_id = context.client_item_id.as_str(),
                    error = %error,
                    "failed to check for duplicate failed download blocklist entry"
                );
            }
        }
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
        None if failed_collection_items.is_none() => {
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

    let (outcome, failure_reason) = if let Some(items) = failed_collection_items.as_ref() {
        let now = Utc::now();
        let next_search_at = now.to_rfc3339();

        for item in items {
            let _ = app
                .services
                .workflow
                .wanted_items
                .schedule_wanted_item_search(&WantedSearchTransition {
                    id: item.id.clone(),
                    next_search_at: Some(next_search_at.clone()),
                    last_search_at: item.last_search_at.clone(),
                    search_count: item.search_count,
                    current_score: item.current_score,
                    grabbed_release: None,
                })
                .await;
        }

        let message = format!(
            "season pack download failed for '{}': {}; re-queuing season episodes for individual search",
            release_title_for_matching, context.reason
        );

        info!(
            title_id = resolved_title_id.as_deref().unwrap_or(""),
            affected_wanted_items = items.len(),
            release_title = release_title_for_matching,
            "re-queued season episodes after failed season-pack download"
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
            if recover_from_standby_candidates(
                app,
                item,
                release_title_for_matching,
                active_snapshot,
                &now,
            )
            .await
            {
                (
                    FailureHandlingOutcome::RecoveredFromStandby,
                    format!(
                        "download failed for '{}': {}; recovered from standby candidate",
                        release_title_for_matching, context.reason
                    ),
                )
            } else {
                let immediate_research = should_research_failed_grab(item, &now);
                let next_search_at = if immediate_research {
                    now.to_rfc3339()
                } else {
                    (now + Duration::minutes(FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES)).to_rfc3339()
                };

                let _ = app
                    .services
                    .workflow
                    .wanted_items
                    .schedule_wanted_item_search(&WantedSearchTransition {
                        id: item.id.clone(),
                        next_search_at: Some(next_search_at),
                        last_search_at: item.last_search_at.clone(),
                        search_count: item.search_count,
                        current_score: item.current_score,
                        grabbed_release: None,
                    })
                    .await;

                let message = if immediate_research {
                    format!(
                        "download failed for '{}': {}; standby exhausted, re-queuing for fresh search",
                        release_title_for_matching, context.reason
                    )
                } else {
                    format!(
                        "download failed for '{}': {}; standby exhausted, deferring reacquisition",
                        release_title_for_matching, context.reason
                    )
                };

                if immediate_research {
                    (FailureHandlingOutcome::RequeuedFreshSearch, message)
                } else {
                    (FailureHandlingOutcome::RequeuedDeferred, message)
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
) -> Option<WantedItem> {
    let title_id = title_id?.trim();
    if title_id.is_empty() {
        return None;
    }

    let grabbed_items = app
        .services
        .workflow
        .wanted_items
        .list_wanted_items(WantedItemsQuery {
            statuses: vec!["grabbed".into()],
            title_id: Some(title_id.to_string()),
            limit: 25,
            ..WantedItemsQuery::default()
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

/// Process due wanted items: search indexers and auto-grab best releases.
async fn process_due_wanted_items(app: &AppUseCase) {
    let blocked_facets = blocked_acquisition_facets_after_quiet_wait(app).await;
    process_due_wanted_items_with_blocked_facets(app, &blocked_facets).await;
}

pub(crate) async fn process_due_wanted_items_with_blocked_facets(
    app: &AppUseCase,
    blocked_facets: &[MediaFacet],
) {
    prune_standby_candidates(app).await;

    // Check for download failures first — re-queues failed items with
    // next_search_at=NOW so they appear in the due list below.
    let dl_snapshot = DownloadClientSnapshot::fetch(app).await;
    check_grabbed_for_failures(app, &dl_snapshot).await;

    // Capture `now` AFTER failure check so that items just re-queued
    // are guaranteed to satisfy `next_search_at <= now`.
    let now = Utc::now();
    let now_str = now.to_rfc3339();

    let batch_size = match app.acquisition_settings().await {
        Ok(settings) => settings.batch_size.clamp(1, 500) as i64,
        Err(err) => {
            warn!(error = %err, "failed to load acquisition settings, using default batch size");
            50
        }
    };

    let due_items = match app
        .services
        .workflow
        .wanted_items
        .list_due_wanted_items(&now_str, batch_size, blocked_facets)
        .await
    {
        Ok(items) => {
            if !items.is_empty() {
                info!(
                    count = items.len(),
                    now = now_str.as_str(),
                    "background acquisition: found due wanted items"
                );
            }
            items
        }
        Err(err) => {
            warn!(error = %err, "failed to list due wanted items");
            return;
        }
    };

    if due_items.is_empty() {
        return;
    }

    info!(count = due_items.len(), "processing due wanted items");

    // Track URLs already submitted this cycle to avoid sending the same NZB
    // multiple times (e.g. a season pack matching several episode wanted items).
    let mut grabbed_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Track (title_id, season_num) for which a season pack search was attempted this cycle.
    let mut season_pack_attempted: std::collections::HashSet<(String, u32)> =
        std::collections::HashSet::new();
    // Track (title_id, season_num) for which a season pack was successfully grabbed this cycle.
    let mut season_pack_grabbed: std::collections::HashSet<(String, u32)> =
        std::collections::HashSet::new();
    let mut recent_failed_season_packs_by_title: std::collections::HashMap<String, HashSet<u32>> =
        std::collections::HashMap::new();

    // Count due episode items per (title_id, season_num). Season pack search is only
    // worthwhile when >= 2 episodes from the same season are due this cycle — mirroring
    // Sonarr's rule of "count > 1 missing" before issuing a SeasonSearchCriteria.
    let mut season_due_counts: std::collections::HashMap<(String, u32), usize> =
        std::collections::HashMap::new();
    for item in &due_items {
        if item.media_type == "episode"
            && let Some(sn) = item.season_number.as_deref()
            && let Ok(n) = sn.parse::<u32>()
            && n > 0
        {
            *season_due_counts
                .entry((item.title_id.clone(), n))
                .or_insert(0) += 1;
        }
    }

    for item in &due_items {
        if let Err(err) = process_single_wanted_item(
            app,
            item,
            &now,
            &mut grabbed_urls,
            &mut season_pack_attempted,
            &mut season_pack_grabbed,
            &mut recent_failed_season_packs_by_title,
            &season_due_counts,
            &dl_snapshot,
        )
        .await
        {
            warn!(
                wanted_item_id = item.id.as_str(),
                title_id = item.title_id.as_str(),
                error = %err,
                "failed to process wanted item"
            );
        }

        // Re-read the wanted item status after processing.  If the item was
        // successfully grabbed inside process_single_wanted_item (status changed
        // to "grabbed"), we must NOT overwrite it with a search schedule — doing
        // so would reset it to "wanted" and prevent check_grabbed_for_failures
        // from ever detecting download failures.
        let current = app
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(&item.id)
            .await
            .ok()
            .flatten();

        if let Some(ref wi) = current
            && wi.status == WantedStatus::Grabbed
        {
            // Item was grabbed — don't touch it.  The download failure
            // detector will handle re-queuing if the download fails.
            continue;
        }

        // Item is still "wanted" (no grab succeeded, or all candidates were
        // exhausted).  Update the search schedule with backoff.
        let schedule = compute_search_schedule(
            &item.media_type,
            item.baseline_date.as_deref(),
            &item.search_phase,
            &now,
        );

        let _ = app
            .services
            .workflow
            .wanted_items
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(schedule.next_search_at),
                last_search_at: Some(now.to_rfc3339()),
                search_count: item.search_count + 1,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
            .await;
    }
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
            .wanted_items
            .get_wanted_item_by_id(&wanted_item_id)
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

        if wanted.status != WantedStatus::Grabbed {
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

impl AppUseCase {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn run_acquisition_cycle_once(&self) {
        process_due_wanted_items(self).await;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "wanted-item processing coordinates shared acquisition state across a single loop iteration"
)]
async fn process_single_wanted_item(
    app: &AppUseCase,
    item: &WantedItem,
    now: &DateTime<Utc>,
    grabbed_urls: &mut std::collections::HashSet<String>,
    season_pack_attempted: &mut std::collections::HashSet<(String, u32)>,
    season_pack_grabbed: &mut std::collections::HashSet<(String, u32)>,
    recent_failed_season_packs_by_title: &mut std::collections::HashMap<String, HashSet<u32>>,
    season_due_counts: &std::collections::HashMap<(String, u32), usize>,
    dl_snapshot: &DownloadClientSnapshot,
) -> AppResult<()> {
    // Load the title to get search context
    let title = match app
        .services
        .catalog
        .titles
        .get_by_id(&item.title_id)
        .await?
    {
        Some(t) => t,
        None => {
            warn!(
                title_id = item.title_id.as_str(),
                "wanted item references missing title"
            );
            return Ok(());
        }
    };

    // Load episode data for episode-type wanted items
    let episode = if item.media_type == "episode" {
        if let Some(ep_id) = item.episode_id.as_deref() {
            match app.services.catalog.shows.get_episode_by_id(ep_id).await {
                Ok(ep) => ep,
                Err(err) => {
                    warn!(episode_id = ep_id, error = %err, "failed to load episode for wanted item");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Item-aware gate: skip only when an active/recent submission blocks this
    // wanted item, not every sibling episode on the same title.
    let submissions = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&item.title_id)
        .await
        .unwrap_or_default();
    let episode_collection_id = episode_collection_id_for_wanted_item(item, episode.as_ref());

    let has_blocking_active_or_completed_submission = submissions.iter().any(|submission| {
        submission_is_active_or_completed(submission, dl_snapshot)
            && submission_blocks_wanted_item(submission, item, episode_collection_id.as_deref())
    });

    if has_blocking_active_or_completed_submission {
        info!(
            title = title.name.as_str(),
            media_type = item.media_type.as_str(),
            episode_id = item.episode_id.as_deref(),
            collection_id = episode_collection_id
                .as_deref()
                .or(item.collection_id.as_deref()),
            "skipping search — download for this wanted item is already active or completed"
        );
        return Ok(());
    }

    // For interstitial movies, build a synthetic title from the collection's movie metadata
    // so the search uses the movie's name/year/IMDB ID instead of the parent series'
    let search_title = if item.media_type == "interstitial_movie" {
        if let Some(ref coll_id) = item.collection_id
            && let Ok(Some(collection)) = app
                .services
                .catalog
                .shows
                .get_collection_by_id(coll_id)
                .await
        {
            interstitial_movie_search_title(&title, &collection)
        } else {
            title.clone()
        }
    } else {
        title.clone()
    };

    let search_title = if item.media_type == "episode" {
        if let Some(anidb_id) = app
            .local_scoped_anidb_id_for_episode(episode.as_ref())
            .await
        {
            let mut title = search_title;
            title.external_ids.retain(|id| {
                !matches!(
                    id.source.trim().to_ascii_lowercase().as_str(),
                    "anidb" | "anidb_id"
                )
            });
            title.external_ids.push(scryer_domain::ExternalId {
                source: "anidb".into(),
                value: anidb_id,
            });
            title
        } else {
            search_title
        }
    } else {
        search_title
    };

    let subject = app
        .resolve_release_search_subject_for_wanted_item(&search_title, item, episode.as_ref())
        .await;
    let search_season = subject.season;

    // Derive the download client category separately — search_category ("series")
    // is for Newznab query type, download_category ("series") is for NZBGet routing.
    //
    // ── Season pack priority ──────────────────────────────────────────────────
    // For episode wanted items, try a season pack search first. Season packs are
    // a first-class release type on Usenet and are more efficient than individual
    // episodes. Individual episode searches only run if no season pack was found
    // this cycle for this (title, season).
    if item.media_type == "episode"
        && let Some(season_num) = search_season
    {
        let season_key = (title.id.clone(), season_num);

        // Only attempt a season pack search when >= 2 episodes from this season
        // are due this cycle (mirrors Sonarr: count > 1 missing → SeasonSearchCriteria).
        let due_count = season_due_counts.get(&season_key).copied().unwrap_or(0);

        if due_count >= 2 && !season_pack_attempted.contains(&season_key) {
            season_pack_attempted.insert(season_key.clone());

            let recent_failed_seasons =
                if let Some(cached) = recent_failed_season_packs_by_title.get(&title.id) {
                    cached.clone()
                } else {
                    let loaded =
                        load_recent_failed_season_pack_seasons_for_title(app, &title.id, now).await;
                    recent_failed_season_packs_by_title.insert(title.id.clone(), loaded.clone());
                    loaded
                };

            if recent_failed_seasons.contains(&season_num) {
                info!(
                    title = title.name.as_str(),
                    season = season_num,
                    cooldown_minutes = FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES,
                    "skipping season-pack search after recent failed season-pack attempt"
                );
            } else {
                // Load season episodes for runtime scoring and upgrade checking.
                let season_episodes = if let Some(ref coll_id) = item.collection_id {
                    app.services
                        .catalog
                        .shows
                        .list_episodes_for_collection(coll_id)
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };

                // Calculate total season runtime for accurate size scoring.
                // A 10-episode × 24-min season should expect ~10× a single episode's size.
                let pack_runtime = if !season_episodes.is_empty() {
                    let ep_count = season_episodes.len().max(1) as i32;
                    let per_ep = title.runtime_minutes.unwrap_or(24);
                    Some(per_ep * ep_count)
                } else {
                    title.runtime_minutes
                };

                let pack_subject = app
                    .resolve_release_search_subject_for_season_pack(
                        &search_title,
                        item,
                        episode.as_ref(),
                        season_num,
                        pack_runtime,
                    )
                    .await?;

                let pack_results = match app
                    .search_and_evaluate_subject(
                        &search_title,
                        &pack_subject,
                        "background_acquisition_season_pack",
                        SearchMode::Auto,
                    )
                    .await
                {
                    Ok(results) => results,
                    Err(err) => {
                        warn!(
                            title_id = title.id.as_str(),
                            season = season_num,
                            error = %err,
                            "season pack search failed"
                        );
                        Vec::new()
                    }
                };

                for candidate in pack_results
                    .iter()
                    .filter(|candidate| candidate_is_season_pack_for_season(candidate, season_num))
                {
                    let decision_code = annotated_auto_decision_code(candidate);
                    record_release_decision(app, item, &title, candidate, decision_code, now).await;
                }

                if let Some(best_pack) = pack_results.iter().find(|candidate| {
                    candidate_is_season_pack_for_season(candidate, season_num)
                        && candidate.auto_eligible == Some(true)
                }) {
                    // ── Season pack upgrade guard ───────────────────────────────
                    // Check whether grabbing this pack benefits at least 1 episode.
                    // If every episode already has a file with an equal or better
                    // score, the pack is pure waste — skip it and fall through to
                    // individual episode searches (which will also be skipped by
                    // the per-episode cutoff/upgrade checks).
                    //
                    // TODO: make this user-configurable via quality profile. Some
                    // users may want a stricter threshold (e.g. "only grab season
                    // packs if ≥50% of episodes benefit") to reduce download
                    // bandwidth, rather than the current "any 1 episode" policy.
                    let pack_dominated = if !season_episodes.is_empty() {
                        let pack_score = best_pack
                            .quality_profile_decision
                            .as_ref()
                            .map(|d| d.preference_score)
                            .unwrap_or(0);

                        let existing_files = app
                            .services
                            .library
                            .media_files
                            .list_media_files_for_title(&title.id)
                            .await
                            .unwrap_or_default();

                        let episode_file_scores: std::collections::HashMap<String, i32> =
                            existing_files
                                .iter()
                                .filter_map(|f| {
                                    f.episode_id
                                        .as_ref()
                                        .zip(f.acquisition_score)
                                        .map(|(eid, score)| (eid.clone(), score))
                                })
                                .collect();

                        // Pack is dominated (no benefit) when every episode in the
                        // season already has a file with score >= pack_score.
                        !season_episodes.iter().any(|ep| {
                            episode_file_scores
                                .get(&ep.id)
                                .map(|&existing| pack_score > existing)
                                .unwrap_or(true) // no file → episode benefits
                        })
                    } else {
                        false // can't determine episodes → allow grab
                    };

                    if pack_dominated {
                        info!(
                            title = title.name.as_str(),
                            season = season_num,
                            release = best_pack.title.as_str(),
                            "season pack skipped: all episodes already have equal or better files"
                        );
                        // Don't grab — fall through to individual episode search
                    } else {
                        // ── End season pack upgrade guard ────────────────────────────

                        let pack_url = best_pack
                            .download_url
                            .clone()
                            .or_else(|| best_pack.link.clone());
                        let url_str = pack_url.as_deref().unwrap_or("").to_string();

                        if !url_str.is_empty() && grabbed_urls.insert(url_str.clone()) {
                            let download_cat = app.derive_download_category(&title.facet).await;
                            let is_recent = app.is_recent_for_queue_priority(
                                best_pack
                                    .published_at
                                    .as_deref()
                                    .or(episode.as_ref().and_then(|item| item.air_date.as_deref()))
                                    .or(title.first_aired.as_deref())
                                    .or(title.digital_release_date.as_deref()),
                            );
                            let pack_title = Some(best_pack.title.clone());
                            let pack_hint = normalize_release_attempt_hint(pack_url.as_deref());
                            let pack_title_norm =
                                normalize_release_attempt_title(pack_title.as_deref());
                            let pack_password =
                                normalize_release_password(best_pack.password_hint.as_deref());
                            let request_signature = normalize_release_selection_signature(
                                pack_url.as_deref(),
                                pack_title.as_deref(),
                                best_pack.source_kind,
                            );

                            let grab_result = app
                                .services
                                .integrations
                                .download_client
                                .submit_download(&DownloadClientAddRequest {
                                    title: title.clone(),
                                    source_hint: pack_url.clone(),
                                    staged_nzb: None,
                                    source_kind: best_pack.source_kind,
                                    source_title: pack_title.clone(),
                                    source_password: pack_password.clone(),
                                    category: Some(download_cat),
                                    queue_priority: None,
                                    download_directory: None,
                                    release_title: Some(best_pack.title.clone()),
                                    indexer_name: Some(best_pack.source.clone()),
                                    info_hash_hint: best_pack
                                        .extra
                                        .get("info_hash")
                                        .and_then(|value| value.as_str())
                                        .map(str::to_string),
                                    seed_goal_ratio: None,
                                    seed_goal_seconds: None,
                                    is_recent,
                                    season_pack: Some(true),
                                })
                                .await;

                            match grab_result {
                                Ok(grab) => {
                                    let download_job_id = grab.job_id.clone();
                                    let facet_label = serde_json::to_string(&title.facet)
                                        .unwrap_or_else(|_| "\"other\"".to_string())
                                        .trim_matches('"')
                                        .to_string();
                                    metrics::counter!("scryer_grabs_total", "indexer" => best_pack.source.clone(), "facet" => facet_label).increment(1);
                                    season_pack_grabbed.insert(season_key.clone());
                                    let _ = app
                                        .services
                                        .workflow
                                        .release_attempts
                                        .record_release_attempt(
                                            Some(title.id.clone()),
                                            pack_hint,
                                            pack_title_norm,
                                            ReleaseDownloadAttemptOutcome::Success,
                                            None,
                                            pack_password,
                                        )
                                        .await;
                                    let facet_str = serde_json::to_string(&title.facet)
                                        .unwrap_or_else(|_| "\"other\"".to_string());
                                    let submission_scope =
                                        collection_download_submission_scope_for_wanted_item(
                                            item,
                                            episode.as_ref(),
                                        );
                                    let covered_wanted_item_ids = app
                                        .covered_wanted_item_ids_for_submission_scope(
                                            &title.id,
                                            &submission_scope,
                                            &item.id,
                                        )
                                        .await?;
                                    let grabbed_json = serde_json::json!({
                                        "title": best_pack.title,
                                        "score": best_pack
                                            .quality_profile_decision
                                            .as_ref()
                                            .map(|decision| decision.preference_score)
                                            .unwrap_or(0),
                                        "grabbed_at": now.to_rfc3339(),
                                        "season_pack": true,
                                    })
                                    .to_string();
                                    app.services
                                        .workflow
                                        .acquisition_state
                                        .commit_successful_grab(&SuccessfulGrabCommit {
                                            wanted_item_id: item.id.clone(),
                                            covered_wanted_item_ids,
                                            search_count: item.search_count + 1,
                                            current_score: item.current_score,
                                            grabbed_release: grabbed_json,
                                            last_search_at: Some(now.to_rfc3339()),
                                            download_submission: DownloadSubmission {
                                                title_id: title.id.clone(),
                                                facet: facet_str.trim_matches('"').to_string(),
                                                download_client_id: grab.client_id,
                                                download_client_type: grab.client_type,
                                                download_client_item_id: grab.job_id,
                                                source_hint: None,
                                                source_kind: None,
                                                source_title: Some(best_pack.title.clone()),
                                                request_signature: request_signature.clone(),
                                                scope: submission_scope,
                                            },
                                            grabbed_pending_release_id: None,
                                            grabbed_at: Some(now.to_rfc3339()),
                                        })
                                        .await?;
                                    let pack_score = best_pack
                                        .quality_profile_decision
                                        .as_ref()
                                        .map(|d| d.preference_score)
                                        .unwrap_or(0);
                                    let mut grab_meta = HashMap::new();
                                    grab_meta.insert(
                                        "title_name".to_string(),
                                        serde_json::json!(title.name),
                                    );
                                    grab_meta.insert(
                                        "release_title".to_string(),
                                        serde_json::json!(best_pack.title),
                                    );
                                    grab_meta.insert(
                                        "indexer".to_string(),
                                        serde_json::json!(best_pack.source),
                                    );
                                    grab_meta
                                        .insert("score".to_string(), serde_json::json!(pack_score));
                                    let _ = app
                                        .append_domain_event(new_title_domain_event(
                                            None,
                                            &title,
                                            DomainEventPayload::ReleaseGrabbed(
                                                ReleaseGrabbedEventData {
                                                    title: title_context_snapshot(&title),
                                                    source_title: Some(best_pack.title.clone()),
                                                    source_hint: Some(best_pack.source.clone()),
                                                    download_id: Some(download_job_id),
                                                    episode_ids: item
                                                        .episode_id
                                                        .iter()
                                                        .cloned()
                                                        .collect(),
                                                },
                                            ),
                                        ))
                                        .await;
                                    info!(
                                        title = title.name.as_str(),
                                        season = season_num,
                                        release = best_pack.title.as_str(),
                                        "season pack grabbed; skipping individual episode searches for this season"
                                    );
                                }
                                Err(err) => {
                                    warn!(
                                        title = title.name.as_str(),
                                        season = season_num,
                                        error = %err,
                                        "season pack grab failed, will fall back to individual episode search"
                                    );
                                    let _ = app
                                        .services
                                        .workflow
                                        .release_attempts
                                        .record_release_attempt(
                                            Some(title.id.clone()),
                                            pack_hint,
                                            pack_title_norm,
                                            ReleaseDownloadAttemptOutcome::Failed,
                                            Some(err.to_string()),
                                            pack_password,
                                        )
                                        .await;
                                }
                            }
                        }
                    } // close else (pack not dominated)
                }
            }
        }

        // If a season pack was grabbed this cycle (by this item or an earlier
        // item for the same season), skip the individual episode search.
        if season_pack_grabbed.contains(&season_key) {
            return Ok(());
        }
    }
    // ── End season pack priority ──────────────────────────────────────────────
    // Uses the per-facet default download category; the selected client's
    // explicit routing category overrides this inside the router.
    let download_cat = app.derive_download_category(&title.facet).await;

    if subject.queries.is_empty() {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            media_type = item.media_type.as_str(),
            "background acquisition: no search queries built, skipping"
        );
        return Ok(());
    }

    debug!(
        title_id = title.id.as_str(),
        title_name = title.name.as_str(),
        queries = ?subject.queries,
        imdb_id = subject.imdb_id.as_deref().unwrap_or(""),
        tvdb_id = subject.tvdb_id.as_deref().unwrap_or(""),
        category = subject.category.as_str(),
        "background acquisition: searching indexers"
    );

    // Search and score releases
    let results = match app
        .search_and_evaluate_subject(
            &search_title,
            &subject,
            "background_acquisition",
            SearchMode::Auto,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => {
            warn!(
                title_id = title.id.as_str(),
                error = %err,
                "background search failed"
            );
            return Ok(());
        }
    };

    app.emit_acquisition_search_completed_event(None, &title, results.len() as i64)
        .await;

    if results.is_empty() {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: search returned 0 results"
        );
        return Ok(());
    }

    info!(
        title_id = title.id.as_str(),
        title_name = title.name.as_str(),
        result_count = results.len(),
        "background acquisition: evaluating candidates"
    );

    // Load DB-level blocklist (covers post-import failures like fake/non-video files,
    // in addition to the download-client snapshot checked below).
    let _db_blocklist: std::collections::HashSet<String> = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 200)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| e.source_title)
        .map(|t| t.to_ascii_lowercase())
        .collect();

    let upgrade_context = app
        .resolve_upgrade_context_for_title_with_category(
            &search_title,
            item.grabbed_release.as_deref(),
            Some(subject.category.as_str()),
        )
        .await;
    let profile = &upgrade_context.profile;

    // Cutoff tier check — skip upgrades if the existing file meets the cutoff quality.
    // This is independent of any candidate and can short-circuit before the loop.
    if upgrade_context.cutoff_reached {
        tracing::debug!(
            title_id = title.id.as_str(),
            cutoff = profile.criteria.cutoff_tier.as_deref().unwrap_or(""),
            "cutoff quality reached, skipping upgrade"
        );
        return Ok(());
    }
    let delay_profiles = app.load_delay_profiles().await;

    // ── Candidate fallthrough loop ──────────────────────────────────────────
    // Iterate ranked candidates (sorted by preference_score DESC).  If a grab
    // fails, try the next candidate instead of re-searching from scratch next
    // cycle.  Mirrors Sonarr's ProcessDownloadDecisions loop.
    let mut had_allowed_candidate = false;
    let mut had_quality_allowed_candidate = false;
    let mut skipped_for_failed = false;
    let mut skipped_for_title_mismatch = false;
    let mut grab_attempts: usize = 0;
    // Track source kinds where ALL download clients failed.  Avoids hammering
    // dead clients with more candidates of the same protocol.
    let mut failed_source_kinds: Vec<DownloadSourceKind> = Vec::new();

    for (candidate_index, candidate) in results.iter().enumerate() {
        let is_allowed = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.allowed)
            .unwrap_or(false);
        if !is_allowed {
            continue;
        }

        had_quality_allowed_candidate = true;

        let decision_code = effective_auto_decision_code(candidate, &failed_source_kinds);

        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.preference_score)
            .unwrap_or(0);

        if !matches!(decision_code, ReleaseAutoDecisionCode::TitleMismatch) {
            had_allowed_candidate = true;
        }
        if matches!(decision_code, ReleaseAutoDecisionCode::TitleMismatch) {
            skipped_for_title_mismatch = true;
        }
        if matches!(decision_code, ReleaseAutoDecisionCode::DbBlocklisted) {
            skipped_for_failed = true;
        }

        record_release_decision(app, item, &title, candidate, decision_code, now).await;

        if !decision_code.is_eligible() {
            app.emit_acquisition_candidate_rejected_event(
                None,
                &title,
                candidate.title.clone(),
                decision_code.as_str().to_string(),
            )
            .await;
            if matches!(
                decision_code,
                ReleaseAutoDecisionCode::NegativeScore
                    | ReleaseAutoDecisionCode::UpgradeRejected
                    | ReleaseAutoDecisionCode::CutoffReached
            ) {
                break;
            }
            if matches!(decision_code, ReleaseAutoDecisionCode::PendingDelay) {
                let scoring_json = candidate.quality_profile_decision.as_ref().map(|decision| {
                    serde_json::to_string(
                        &decision
                            .scoring_log
                            .iter()
                            .map(|entry| serde_json::json!({"code": entry.code, "delta": entry.delta}))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap_or_default()
                });

                app.insert_pending_release(
                    item,
                    &title,
                    &candidate.title,
                    candidate
                        .download_url
                        .as_deref()
                        .or(candidate.link.as_deref()),
                    candidate.source_kind,
                    candidate.size_bytes,
                    candidate_score,
                    scoring_json,
                    Some(candidate.source.as_str()),
                    candidate.guid.as_deref(),
                    crate::delay_profile::resolve_delay_decision(
                        &delay_profiles,
                        &search_title.tags,
                        &search_title.facet,
                        candidate.source_kind,
                        candidate
                            .published_at
                            .as_deref()
                            .and_then(crate::quality_profile::parse_published_at),
                        candidate_score,
                        now,
                    )
                    .map(|delay| delay.effective_delay_minutes)
                    .unwrap_or_default(),
                    candidate.password_hint.as_deref(),
                    candidate.published_at.as_deref(),
                    candidate
                        .extra
                        .get("info_hash")
                        .and_then(|value| value.as_str()),
                )
                .await;
                return Ok(());
            }
            continue;
        }

        // ── Grab attempt ────────────────────────────────────────────────────
        grab_attempts += 1;
        if grab_attempts > 10 {
            warn!(
                title = title.name.as_str(),
                "reached max grab attempts (10), deferring to next cycle"
            );
            break;
        }

        // Submit to download client
        let source_hint = candidate
            .download_url
            .clone()
            .or_else(|| candidate.link.clone());

        // Deduplicate: skip if this exact URL was already submitted this cycle.
        if let Some(url) = source_hint.as_deref()
            && !grabbed_urls.insert(url.to_string())
        {
            info!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                "skipping duplicate release already submitted this cycle"
            );
            // Mark this wanted item as grabbed too since the release covers it
            let grabbed_json = serde_json::json!({
                "title": candidate.title,
                "score": candidate_score,
                "grabbed_at": now.to_rfc3339(),
                "deduplicated": true,
            })
            .to_string();
            let _ = app
                .services
                .workflow
                .wanted_items
                .transition_wanted_to_grabbed(&WantedGrabTransition {
                    id: item.id.clone(),
                    last_search_at: Some(now.to_rfc3339()),
                    search_count: item.search_count + 1,
                    current_score: item.current_score,
                    grabbed_release: grabbed_json,
                })
                .await;
            return Ok(());
        }

        let source_title = Some(candidate.title.clone());
        let source_hint_for_attempt = normalize_release_attempt_hint(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_title(source_title.as_deref());
        let source_password = normalize_release_password(candidate.password_hint.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint.as_deref(),
            source_title.as_deref(),
            candidate.source_kind,
        );

        let _ = app
            .services
            .workflow
            .release_attempts
            .record_release_attempt(
                Some(title.id.clone()),
                source_hint_for_attempt.clone(),
                source_title_for_attempt.clone(),
                ReleaseDownloadAttemptOutcome::Pending,
                None,
                source_password.clone(),
            )
            .await;

        let is_recent = app.is_recent_for_queue_priority(
            candidate
                .published_at
                .as_deref()
                .or(episode.as_ref().and_then(|item| item.air_date.as_deref()))
                .or(item.baseline_date.as_deref())
                .or(title.first_aired.as_deref())
                .or(title.digital_release_date.as_deref()),
        );

        info!(
            title = title.name.as_str(),
            release = candidate.title.as_str(),
            score = candidate_score,
            decision = decision_code.as_str(),
            attempt = grab_attempts,
            "auto-grabbing release"
        );

        let grab_result = app
            .services
            .integrations
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                source_hint: source_hint.clone(),
                staged_nzb: None,
                source_kind: candidate.source_kind,
                source_title: source_title.clone(),
                source_password: source_password.clone(),
                category: Some(download_cat.clone()),
                queue_priority: None,
                download_directory: None,
                release_title: Some(candidate.title.clone()),
                indexer_name: Some(candidate.source.clone()),
                info_hash_hint: candidate
                    .extra
                    .get("info_hash")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: Some(false),
            })
            .await;

        match grab_result {
            Ok(grab) => {
                // ── Success ─────────────────────────────────────────────────
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => candidate.source.clone(), "facet" => facet_label).increment(1);
                }

                let _ = app
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        source_password.clone(),
                    )
                    .await;

                // Record title history: Grabbed
                // Record download submission for auto-import matching
                let facet_str =
                    serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
                let grabbed_json = serde_json::json!({
                    "title": candidate.title,
                    "score": candidate_score,
                    "grabbed_at": now.to_rfc3339(),
                })
                .to_string();
                let download_job_id = grab.job_id.clone();
                let submission_scope = if let Some(parsed) =
                    candidate.parsed_release_metadata.as_ref()
                {
                    let catalog_episodes = app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_title(&title.id)
                        .await
                        .unwrap_or_default();
                    let catalog_collections = app
                        .services
                        .catalog
                        .shows
                        .list_collections_for_title(&title.id)
                        .await
                        .unwrap_or_default();
                    crate::acquisition_coverage::resolve_release_coverage(
                        parsed,
                        &catalog_episodes,
                        &catalog_collections,
                        episode.as_ref(),
                    )
                    .submission_scope_or(
                        &direct_download_submission_scope_for_wanted_item(item, episode.as_ref()),
                    )
                } else {
                    direct_download_submission_scope_for_wanted_item(item, episode.as_ref())
                };
                let covered_wanted_item_ids = app
                    .covered_wanted_item_ids_for_submission_scope(
                        &title.id,
                        &submission_scope,
                        &item.id,
                    )
                    .await?;

                app.services
                    .workflow
                    .acquisition_state
                    .commit_successful_grab(&SuccessfulGrabCommit {
                        wanted_item_id: item.id.clone(),
                        covered_wanted_item_ids,
                        search_count: item.search_count + 1,
                        current_score: item.current_score,
                        grabbed_release: grabbed_json,
                        last_search_at: Some(now.to_rfc3339()),
                        download_submission: DownloadSubmission {
                            title_id: title.id.clone(),
                            facet: facet_str.trim_matches('"').to_string(),
                            download_client_id: grab.client_id,
                            download_client_type: grab.client_type,
                            download_client_item_id: grab.job_id,
                            source_hint: None,
                            source_kind: None,
                            source_title: source_title.clone(),
                            request_signature: request_signature.clone(),
                            scope: submission_scope,
                        },
                        grabbed_pending_release_id: None,
                        grabbed_at: Some(now.to_rfc3339()),
                    })
                    .await?;

                persist_standby_candidates(
                    app,
                    item,
                    &title,
                    &results,
                    candidate_index + 1,
                    now,
                    &failed_source_kinds,
                )
                .await;

                let _ = app
                    .append_domain_event(new_title_domain_event(
                        None,
                        &title,
                        DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                            title: title_context_snapshot(&title),
                            source_title: Some(candidate.title.clone()),
                            source_hint: Some(candidate.source.clone()),
                            download_id: Some(download_job_id),
                            episode_ids: item.episode_id.iter().cloned().collect(),
                        }),
                    ))
                    .await;

                return Ok(());
            }
            Err(err) => {
                // ── Grab failed — try next candidate ────────────────────────
                warn!(
                    title = title.name.as_str(),
                    release = candidate.title.as_str(),
                    attempt = grab_attempts,
                    error = %err,
                    "grab failed, trying next candidate"
                );

                let attribution = FailedReleaseAttribution {
                    title: Some(title.clone()),
                    episode_ids: item.episode_id.iter().cloned().collect(),
                    collection_id: item.collection_id.clone(),
                };
                let failure_reason = format!(
                    "grab failed for '{}' (attempt {}/10, trying next): {}",
                    candidate.title, grab_attempts, err
                );
                let candidate_source_hint = candidate
                    .download_url
                    .clone()
                    .or_else(|| candidate.link.clone())
                    .unwrap_or_else(|| candidate.source.clone());
                let quality = candidate
                    .parsed_release_metadata
                    .as_ref()
                    .and_then(|parsed| parsed.quality.clone())
                    .or_else(|| release_quality_hint(Some(candidate.title.as_str())));

                record_failed_release_outcome(
                    app,
                    Some(title.id.as_str()),
                    &attribution,
                    Some(candidate.title.clone()),
                    Some(candidate_source_hint),
                    None,
                    None,
                    None,
                    None,
                    quality,
                    Some(failure_reason),
                    None,
                    source_password,
                )
                .await;

                // If ALL download clients for this source kind are down, mark it
                // so we skip remaining candidates with the same protocol.
                if is_all_clients_failed_error(&err)
                    && let Some(sk) = candidate.source_kind
                {
                    if !failed_source_kinds.contains(&sk) {
                        failed_source_kinds.push(sk);
                    }
                    info!(
                        source_kind = ?sk,
                        "all download clients failed for source kind, skipping remaining candidates with same protocol"
                    );
                }

                // Add URL to exclusion set so we don't re-select this exact
                // release if the same URL appears from a different indexer.
                if let Some(url) = source_hint.as_deref() {
                    grabbed_urls.insert(url.to_string());
                }

                // CONTINUE — try the next candidate
            }
        }
    }
    // ── End candidate fallthrough loop ───────────────────────────────────────

    // All candidates exhausted without a successful grab.
    if grab_attempts > 0 {
        warn!(
            title = title.name.as_str(),
            attempts = grab_attempts,
            "all grab attempts failed, re-queuing for next cycle"
        );
    } else if had_allowed_candidate && skipped_for_failed {
        warn!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: no suitable candidates found after skipping blocklisted or active releases"
        );
    } else if had_allowed_candidate {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: all allowed candidates were already active or had negative scores"
        );
    } else if had_quality_allowed_candidate && skipped_for_title_mismatch {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            result_count = results.len(),
            "background acquisition: quality-allowed candidates were rejected by title matching"
        );
    } else {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            result_count = results.len(),
            "background acquisition: no allowed candidates found (all blocked by quality profile)"
        );
    }

    // Re-queue for next cycle
    let _ = app
        .services
        .workflow
        .wanted_items
        .schedule_wanted_item_search(&WantedSearchTransition {
            id: item.id.clone(),
            next_search_at: Some(now.to_rfc3339()),
            last_search_at: Some(now.to_rfc3339()),
            search_count: item.search_count + 1,
            current_score: item.current_score,
            grabbed_release: item.grabbed_release.clone(),
        })
        .await;

    Ok(())
}

async fn recover_from_standby_candidates(
    app: &AppUseCase,
    item: &WantedItem,
    failed_release_title: &str,
    dl_snapshot: &DownloadClientSnapshot,
    now: &DateTime<Utc>,
) -> bool {
    let standby_releases = app
        .services
        .workflow
        .pending_releases
        .list_standby_pending_releases_for_wanted_item(&item.id)
        .await
        .unwrap_or_default();

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
            Ok(true) => {
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

                return true;
            }
            Ok(false) | Err(_) => {
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(&standby.id, PendingReleaseStatus::Expired, None)
                    .await;
            }
        }
    }

    false
}

async fn persist_standby_candidates(
    app: &AppUseCase,
    item: &WantedItem,
    title: &Title,
    results: &[IndexerSearchResult],
    start_index: usize,
    now: &DateTime<Utc>,
    failed_source_kinds: &[DownloadSourceKind],
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

        let decision_code = effective_auto_decision_code(candidate, failed_source_kinds);
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
            source_password: candidate.password_hint.clone(),
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

// --- Public use-case methods for the wanted items API ---

impl AppUseCase {
    pub async fn get_wanted_item(&self, actor: &User, id: &str) -> AppResult<Option<WantedItem>> {
        let Some(item) = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(id)
            .await?
        else {
            return Ok(None);
        };

        let library_id = match item.library_id.clone() {
            Some(library_id) => library_id,
            None => self
                .services
                .catalog
                .titles
                .get_by_id(&item.title_id)
                .await?
                .map(|title| title.library_id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?,
        };
        self.require_library_permission(actor, &library_id, scryer_domain::LibraryPermission::View)
            .await?;
        Ok(Some(item))
    }

    pub async fn list_wanted_items(
        &self,
        actor: &User,
        query: WantedItemsQuery,
    ) -> AppResult<(Vec<WantedItem>, i64)> {
        let requested_library_ids = query.library_ids.clone();
        let mut library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        if !requested_library_ids.is_empty() {
            let authorized = library_ids.into_iter().collect::<HashSet<_>>();
            library_ids = requested_library_ids
                .into_iter()
                .filter(|library_id| authorized.contains(library_id))
                .collect();
        }
        self.list_wanted_items_for_libraries(query, library_ids)
            .await
    }

    async fn list_wanted_items_for_libraries(
        &self,
        query: WantedItemsQuery,
        library_ids: Vec<String>,
    ) -> AppResult<(Vec<WantedItem>, i64)> {
        let WantedItemsQuery {
            statuses,
            media_types,
            title_id,
            library_ids: _,
            title_search,
            latest_decision_codes,
            limit,
            offset,
        } = query;
        let title_search = title_search.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        if library_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let items = self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                statuses: statuses.clone(),
                media_types: media_types.clone(),
                title_id: title_id.clone(),
                library_ids: library_ids.clone(),
                title_search: title_search.clone(),
                latest_decision_codes: latest_decision_codes.clone(),
                limit,
                offset,
            })
            .await?;
        let total = self
            .services
            .workflow
            .wanted_items
            .count_wanted_items(WantedItemsQuery {
                statuses,
                media_types,
                title_id,
                library_ids,
                title_search,
                latest_decision_codes,
                ..WantedItemsQuery::default()
            })
            .await?;
        Ok((items, total))
    }

    pub async fn list_release_decisions(
        &self,
        actor: &User,
        query: ReleaseDecisionsQuery,
    ) -> AppResult<Vec<ReleaseDecision>> {
        if let Some(wid) = query.wanted_item_id.as_deref() {
            let wanted = self
                .services
                .workflow
                .wanted_items
                .get_wanted_item_by_id(wid)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("wanted item {wid}")))?;
            let library_id = if let Some(library_id) = wanted.library_id.as_deref() {
                library_id.to_string()
            } else {
                self.services
                    .catalog
                    .titles
                    .get_by_id(&wanted.title_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("title {}", wanted.title_id)))?
                    .library_id
            };
            self.require_library_permission(
                actor,
                &library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
            return self
                .services
                .workflow
                .wanted_items
                .list_release_decisions_for_wanted_item(wid, query.limit)
                .await;
        }
        if let Some(tid) = query.title_id.as_deref() {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(tid)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {tid}")))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
            return self
                .services
                .workflow
                .wanted_items
                .list_release_decisions_for_title(tid, query.limit)
                .await;
        }
        Ok(vec![])
    }

    pub async fn trigger_title_wanted_search(
        &self,
        actor: &User,
        title_id: &str,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let now = Utc::now();
        let outcome = if let Some(handler) = self.facet_registry.get(&title.facet) {
            if handler.has_episodes() {
                self.queue_monitored_series_items_for_search(&title, &now)
                    .await?
            } else if title.monitored {
                self.queue_monitored_movie_for_search(&title, &now, conflict_policy)
                    .await?
            } else {
                WantedSearchOutcome::default()
            }
        } else {
            WantedSearchOutcome::default()
        };

        if outcome.queued_count > 0 {
            self.runtime.acquisition.acquisition_wake.notify_one();
        }

        Ok(outcome)
    }

    async fn wanted_item_is_mismatch_recovery_candidate(
        &self,
        item: &WantedItem,
    ) -> AppResult<bool> {
        let decisions = self
            .services
            .workflow
            .wanted_items
            .list_release_decisions_for_wanted_item(&item.id, 10)
            .await?;
        Ok(!decisions.is_empty()
            && decisions
                .iter()
                .all(|decision| decision.decision_code == "title_mismatch"))
    }

    pub async fn wanted_item_mismatch_recovery_eligible(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<bool> {
        let Some(item) = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
        else {
            return Ok(false);
        };

        self.wanted_item_is_mismatch_recovery_candidate(&item).await
    }

    pub async fn title_acquisition_diagnostics(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<TitleAcquisitionDiagnostics> {
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
            scryer_domain::LibraryPermission::View,
        )
        .await?;

        let recent_decisions = self
            .services
            .workflow
            .wanted_items
            .list_release_decisions_for_title(title_id, 25)
            .await?;
        let wanted_items = self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                title_id: Some(title_id.to_string()),
                limit: 500,
                ..WantedItemsQuery::default()
            })
            .await?;
        let pending_releases = self
            .services
            .workflow
            .pending_releases
            .list_pending_releases_for_title(title_id)
            .await?;

        let mut decision_counts = HashMap::<String, i64>::new();
        for decision in &recent_decisions {
            *decision_counts
                .entry(decision.decision_code.clone())
                .or_insert(0) += 1;
        }
        let mut wanted_status_counts = HashMap::<String, i64>::new();
        for item in &wanted_items {
            *wanted_status_counts
                .entry(item.status.as_str().to_string())
                .or_insert(0) += 1;
        }
        let mut pending_release_counts = HashMap::<String, i64>::new();
        for release in &pending_releases {
            *pending_release_counts
                .entry(release.status.as_str().to_string())
                .or_insert(0) += 1;
        }

        let mismatch_recovery_eligible_count = wanted_items
            .iter()
            .filter(|item| item.status == WantedStatus::Wanted && item.mismatch_recovery_eligible)
            .count() as i64;

        let mut decision_counts = decision_counts
            .into_iter()
            .map(|(code, count)| DecisionCodeCount { code, count })
            .collect::<Vec<_>>();
        decision_counts.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.code.cmp(&right.code))
        });

        let mut wanted_status_counts = wanted_status_counts
            .into_iter()
            .map(|(status, count)| WantedStatusCount { status, count })
            .collect::<Vec<_>>();
        wanted_status_counts.sort_by(|left, right| left.status.cmp(&right.status));

        let mut pending_release_counts = pending_release_counts
            .into_iter()
            .map(|(status, count)| PendingReleaseStatusCount { status, count })
            .collect::<Vec<_>>();
        pending_release_counts.sort_by(|left, right| left.status.cmp(&right.status));

        let latest_wanted_search_at = wanted_items
            .iter()
            .filter_map(|item| item.last_search_at.clone())
            .max();

        Ok(TitleAcquisitionDiagnostics {
            latest_decision_at: recent_decisions
                .first()
                .map(|decision| decision.created_at.clone()),
            latest_wanted_search_at,
            recent_decisions,
            decision_counts,
            wanted_status_counts,
            pending_release_counts,
            mismatch_recovery_eligible_count,
        })
    }

    pub async fn trigger_title_mismatch_recovery_search(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<usize> {
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
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let items = self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                statuses: vec!["wanted".into()],
                title_id: Some(title_id.to_string()),
                limit: 500,
                ..WantedItemsQuery::default()
            })
            .await?;

        let now = Utc::now();
        let mut queued = 0usize;
        for item in &items {
            if !self
                .wanted_item_is_mismatch_recovery_candidate(item)
                .await?
            {
                continue;
            }

            self.services
                .workflow
                .wanted_items
                .schedule_wanted_item_search(&WantedSearchTransition {
                    id: item.id.clone(),
                    next_search_at: Some(now.to_rfc3339()),
                    last_search_at: item.last_search_at.clone(),
                    search_count: item.search_count,
                    current_score: item.current_score,
                    grabbed_release: item.grabbed_release.clone(),
                })
                .await?;
            queued += 1;
        }

        if queued > 0 {
            self.runtime.acquisition.acquisition_wake.notify_one();
        }

        Ok(queued)
    }

    pub async fn trigger_season_wanted_search(
        &self,
        actor: &User,
        title_id: &str,
        season_number: u32,
    ) -> AppResult<WantedSearchOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let season_str = season_number.to_string();
        let items = self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                statuses: vec!["wanted".into()],
                media_types: vec!["episode".into()],
                title_id: Some(title_id.to_string()),
                limit: 500,
                ..WantedItemsQuery::default()
            })
            .await?;

        let now = Utc::now();
        let next_search_at = now.to_rfc3339();
        let mut outcome = WantedSearchOutcome::default();
        for item in &items {
            if item.season_number.as_deref() == Some(season_str.as_str()) {
                let scheduled = self
                    .schedule_wanted_item_search_if_unblocked(&title, item, &next_search_at)
                    .await?;
                outcome.queued_count += scheduled.queued_count;
                outcome.skipped_in_progress_count += scheduled.skipped_in_progress_count;
                if outcome.conflict.is_none() {
                    outcome.conflict = scheduled.conflict;
                }
            }
        }

        if outcome.queued_count > 0 {
            self.runtime.acquisition.acquisition_wake.notify_one();
        }

        Ok(outcome)
    }

    pub async fn trigger_wanted_item_search(
        &self,
        actor: &User,
        wanted_item_id: &str,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        let item = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if let Some(outcome) = self
            .handle_wanted_item_conflict(&title, &item, conflict_policy)
            .await?
        {
            return Ok(outcome);
        }

        let now = Utc::now();
        self.services
            .workflow
            .wanted_items
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(now.to_rfc3339()),
                last_search_at: item.last_search_at.clone(),
                search_count: item.search_count,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
            .await?;
        self.runtime.acquisition.acquisition_wake.notify_one();
        Ok(WantedSearchOutcome {
            queued_count: 1,
            skipped_in_progress_count: 0,
            conflict: None,
        })
    }

    pub async fn pause_wanted_item(&self, actor: &User, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .workflow
            .wanted_items
            .transition_wanted_to_paused(&WantedPauseTransition {
                id: item.id.clone(),
                last_search_at: item.last_search_at.clone(),
                search_count: item.search_count,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
            .await
    }

    pub async fn resume_wanted_item(&self, actor: &User, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let now = Utc::now();
        let schedule = compute_search_schedule(
            &item.media_type,
            item.baseline_date.as_deref(),
            &item.search_phase,
            &now,
        );

        self.services
            .workflow
            .wanted_items
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(schedule.next_search_at),
                last_search_at: item.last_search_at.clone(),
                search_count: item.search_count,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
            .await
    }

    pub async fn reset_wanted_item(&self, actor: &User, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let now = Utc::now();
        let schedule = compute_search_schedule(
            &item.media_type,
            item.baseline_date.as_deref(),
            "primary",
            &now,
        );

        self.services
            .workflow
            .wanted_items
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(schedule.next_search_at),
                last_search_at: None,
                search_count: 0,
                current_score: None,
                grabbed_release: None,
            })
            .await
    }
}

impl AppUseCase {
    async fn wanted_item_submission_scope(&self, item: &WantedItem) -> AppResult<SubmissionScope> {
        let episode = if let Some(episode_id) = item.episode_id.as_deref() {
            self.services
                .catalog
                .shows
                .get_episode_by_id(episode_id)
                .await?
        } else {
            None
        };
        Ok(direct_download_submission_scope_for_wanted_item(
            item,
            episode.as_ref(),
        ))
    }

    async fn wanted_item_blocking_submissions(
        &self,
        title: &Title,
        item: &WantedItem,
    ) -> AppResult<Vec<SubmissionScopeConflict>> {
        let scope = self.wanted_item_submission_scope(item).await?;
        self.find_blocking_download_submissions(title, &scope).await
    }

    async fn handle_wanted_item_conflict(
        &self,
        title: &Title,
        item: &WantedItem,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<Option<WantedSearchOutcome>> {
        let conflicts = self.wanted_item_blocking_submissions(title, item).await?;
        if conflicts.is_empty() {
            return Ok(None);
        }

        match conflict_policy {
            SubmissionConflictPolicy::ReplaceEarly
                if conflicts.iter().all(|conflict| conflict.replaceable) =>
            {
                self.replace_blocking_download_submissions(&conflicts)
                    .await?;
                Ok(None)
            }
            SubmissionConflictPolicy::ReplaceEarly | SubmissionConflictPolicy::Abort => {
                let conflict = conflicts
                    .iter()
                    .find(|conflict| !conflict.replaceable)
                    .cloned()
                    .unwrap_or_else(|| conflicts[0].clone());
                Ok(Some(WantedSearchOutcome {
                    queued_count: 0,
                    skipped_in_progress_count: 0,
                    conflict: Some(conflict),
                }))
            }
            SubmissionConflictPolicy::Skip => Ok(Some(WantedSearchOutcome {
                queued_count: 0,
                skipped_in_progress_count: 1,
                conflict: Some(conflicts[0].clone()),
            })),
        }
    }

    pub(crate) async fn covered_wanted_item_ids_for_submission_scope(
        &self,
        title_id: &str,
        scope: &SubmissionScope,
        fallback_wanted_item_id: &str,
    ) -> AppResult<Vec<String>> {
        let items = self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                title_id: Some(title_id.to_string()),
                limit: 1000,
                ..WantedItemsQuery::default()
            })
            .await?;
        if items.is_empty() {
            return Ok(if fallback_wanted_item_id.is_empty() {
                Vec::new()
            } else {
                vec![fallback_wanted_item_id.to_string()]
            });
        }

        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(title_id)
            .await?;
        let fake_submission = DownloadSubmission {
            title_id: title_id.to_string(),
            facet: String::new(),
            download_client_id: None,
            download_client_type: String::new(),
            download_client_item_id: String::new(),
            source_hint: None,
            source_kind: None,
            source_title: None,
            request_signature: None,
            scope: scope.clone(),
        };

        let mut covered = items
            .iter()
            .filter(|item| {
                let episode_collection_id = item.episode_id.as_ref().and_then(|episode_id| {
                    episodes
                        .iter()
                        .find(|episode| &episode.id == episode_id)
                        .and_then(|episode| episode.collection_id.as_deref())
                });
                item.id == fallback_wanted_item_id
                    || submission_blocks_wanted_item(&fake_submission, item, episode_collection_id)
            })
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        covered.sort();
        covered.dedup();
        if covered.is_empty() && !fallback_wanted_item_id.is_empty() {
            covered.push(fallback_wanted_item_id.to_string());
        }
        Ok(covered)
    }

    pub(crate) async fn reset_wanted_items_for_submission_scope(
        &self,
        title_id: &str,
        scope: &SubmissionScope,
    ) -> AppResult<()> {
        let wanted_item_ids = self
            .covered_wanted_item_ids_for_submission_scope(title_id, scope, "")
            .await?;
        for wanted_item_id in wanted_item_ids {
            if let Some(item) = self
                .services
                .workflow
                .wanted_items
                .get_wanted_item_by_id(&wanted_item_id)
                .await?
            {
                self.services
                    .workflow
                    .wanted_items
                    .schedule_wanted_item_search(&WantedSearchTransition {
                        id: item.id,
                        next_search_at: Some(Utc::now().to_rfc3339()),
                        last_search_at: None,
                        search_count: 0,
                        current_score: None,
                        grabbed_release: None,
                    })
                    .await?;
            }
        }
        Ok(())
    }

    async fn schedule_wanted_item_search_if_unblocked(
        &self,
        title: &Title,
        item: &WantedItem,
        next_search_at: &str,
    ) -> AppResult<WantedSearchOutcome> {
        self.schedule_wanted_item_search_with_policy(
            title,
            item,
            next_search_at,
            SubmissionConflictPolicy::Skip,
        )
        .await
    }

    async fn schedule_wanted_item_search_with_policy(
        &self,
        title: &Title,
        item: &WantedItem,
        next_search_at: &str,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        if let Some(outcome) = self
            .handle_wanted_item_conflict(title, item, conflict_policy)
            .await?
        {
            return Ok(outcome);
        }

        self.services
            .workflow
            .wanted_items
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(next_search_at.to_string()),
                last_search_at: item.last_search_at.clone(),
                search_count: item.search_count,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
            .await?;

        Ok(WantedSearchOutcome {
            queued_count: 1,
            skipped_in_progress_count: 0,
            conflict: None,
        })
    }

    async fn ensure_wanted_item_seeded_with_policy(
        &self,
        title: &Title,
        item: &WantedItem,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        if let Some(outcome) = self
            .handle_wanted_item_conflict(title, item, conflict_policy)
            .await?
        {
            return Ok(outcome);
        }

        self.services
            .workflow
            .wanted_items
            .ensure_wanted_item_seeded(item)
            .await?;

        Ok(WantedSearchOutcome {
            queued_count: 1,
            skipped_in_progress_count: 0,
            conflict: None,
        })
    }

    async fn queue_monitored_movie_for_search(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        let has_file = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .map(|files| !files.is_empty())
            .unwrap_or(false);

        if has_file {
            return Ok(WantedSearchOutcome::default());
        }

        let next_search_at = now.to_rfc3339();
        if let Some(item) = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_for_title(&title.id, None)
            .await?
        {
            if item.status == WantedStatus::Grabbed {
                return Ok(WantedSearchOutcome::default());
            }

            return self
                .schedule_wanted_item_search_with_policy(
                    title,
                    &item,
                    &next_search_at,
                    conflict_policy,
                )
                .await;
        }

        let baseline_date = title.first_aired.clone();
        let schedule = compute_search_schedule("movie", baseline_date.as_deref(), "primary", now);
        let item = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: None,
            title_slug: None,
            title_facet: None,
            library_id: Some(title.library_id.clone()),
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: schedule.search_phase.to_string(),
            next_search_at: Some(next_search_at),
            last_search_at: None,
            search_count: 0,
            baseline_date,
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        };

        self.ensure_wanted_item_seeded_with_policy(title, &item, conflict_policy)
            .await
    }

    async fn queue_monitored_series_items_for_search(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
    ) -> AppResult<WantedSearchOutcome> {
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;

        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let episodes_with_files: std::collections::HashSet<String> = existing_files
            .iter()
            .filter_map(|file| file.episode_id.clone())
            .collect();
        let next_search_at = now.to_rfc3339();
        let mut outcome = WantedSearchOutcome::default();

        for collection in &collections {
            if !collection.monitored {
                continue;
            }

            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_collection(&collection.id)
                .await?;

            for episode in &episodes {
                if !episode.monitored || episodes_with_files.contains(&episode.id) {
                    continue;
                }

                if let Some(item) = self
                    .services
                    .workflow
                    .wanted_items
                    .get_wanted_item_for_title(&title.id, Some(&episode.id))
                    .await?
                {
                    if item.status == WantedStatus::Grabbed {
                        continue;
                    }

                    let scheduled = self
                        .schedule_wanted_item_search_if_unblocked(title, &item, &next_search_at)
                        .await?;
                    outcome.queued_count += scheduled.queued_count;
                    outcome.skipped_in_progress_count += scheduled.skipped_in_progress_count;
                    if outcome.conflict.is_none() {
                        outcome.conflict = scheduled.conflict;
                    }
                    continue;
                }

                let baseline_date = episode.air_date.clone();
                let schedule =
                    compute_search_schedule("episode", baseline_date.as_deref(), "primary", now);
                let item = WantedItem {
                    id: Id::new().0,
                    title_id: title.id.clone(),
                    title_name: None,
                    title_slug: None,
                    title_facet: None,
                    library_id: Some(title.library_id.clone()),
                    library_name: None,
                    library_slug: None,
                    episode_id: Some(episode.id.clone()),
                    collection_id: None,
                    season_number: episode.season_number.clone(),
                    episode_number: None,
                    media_type: "episode".to_string(),
                    search_phase: schedule.search_phase.to_string(),
                    next_search_at: Some(next_search_at.clone()),
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    latest_release_decision: None,
                    mismatch_recovery_eligible: false,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                let scheduled = self
                    .ensure_wanted_item_seeded_with_policy(
                        title,
                        &item,
                        SubmissionConflictPolicy::Skip,
                    )
                    .await?;
                outcome.queued_count += scheduled.queued_count;
                outcome.skipped_in_progress_count += scheduled.skipped_in_progress_count;
                if outcome.conflict.is_none() {
                    outcome.conflict = scheduled.conflict;
                }
            }
        }

        Ok(outcome)
    }
}

pub async fn start_background_acquisition_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    // Check feature flag
    let enabled = std::env::var("SCRYER_BACKGROUND_ACQUISITION")
        .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no" | "off"))
        .unwrap_or(true);

    if !enabled {
        info!("background acquisition poller is disabled (SCRYER_BACKGROUND_ACQUISITION=false)");
        return;
    }

    let settings = match app.acquisition_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!(error = %err, "failed to load acquisition settings, using defaults");
            crate::AcquisitionSettings {
                enabled: true,
                upgrade_cooldown_hours: 24,
                same_tier_min_delta: 120,
                cross_tier_min_delta: 30,
                forced_upgrade_delta_bypass: 400,
                poll_interval_seconds: 60,
                sync_interval_seconds: 3600,
                batch_size: 50,
            }
        }
    };

    if !settings.enabled {
        info!("background acquisition poller is disabled (acquisition.enabled != true)");
        return;
    }

    info!("background acquisition poller started");

    // Initial wanted state sync
    if let Err(err) = app
        .run_scheduled_job_now(JobKey::WantedSync, JobTriggerSource::SystemInternal)
        .await
    {
        warn!(error = %err, "initial wanted state sync failed");
    }

    // Reset items that were searched but never found anything. This recovers
    // from scenarios where a bug (e.g. broken capability filter) caused searches
    // to return 0 results and items got rescheduled far into the future.
    let now_str = Utc::now().to_rfc3339();
    match app
        .services
        .workflow
        .wanted_items
        .reset_fruitless_wanted_items(&now_str)
        .await
    {
        Ok(count) if count > 0 => {
            info!(count, "reset fruitless wanted items to search immediately");
        }
        Err(err) => {
            warn!(error = %err, "failed to reset fruitless wanted items");
        }
        _ => {}
    }

    // Run initial health checks after a short delay to let services initialize
    {
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if let Err(error) = app
                .run_scheduled_job_now(JobKey::HealthChecks, JobTriggerSource::ScheduledStartup)
                .await
            {
                warn!(error = %error, "initial health checks failed");
            }
        });
    }

    app.set_job_next_run_at(
        JobKey::WantedSync,
        Utc::now() + chrono::Duration::seconds(settings.sync_interval_seconds.max(1) as i64),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::MetadataRefresh,
        Utc::now() + chrono::Duration::hours(12),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::PluginRegistryRefresh,
        Utc::now() + chrono::Duration::hours(24),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::HealthChecks,
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::StagedNzbPrune,
        Utc::now() + chrono::Duration::hours(1),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::Housekeeping,
        Utc::now() + chrono::Duration::hours(24),
    )
    .await;
    app.set_job_next_run_at(JobKey::RssSync, Utc::now() + chrono::Duration::minutes(15))
        .await;
    app.set_job_next_run_at(
        JobKey::PendingReleaseProcessing,
        Utc::now() + chrono::Duration::minutes(1),
    )
    .await;

    let mut poll_interval = new_skip_interval(std::time::Duration::from_secs(
        settings.poll_interval_seconds.max(1) as u64,
    ));
    let mut sync_interval = tokio::time::interval(std::time::Duration::from_secs(
        settings.sync_interval_seconds.max(1) as u64,
    ));
    let mut metadata_refresh_interval = tokio::time::interval(std::time::Duration::from_hours(12));
    let mut registry_refresh_interval = tokio::time::interval(std::time::Duration::from_hours(24));
    let mut health_check_interval = tokio::time::interval(std::time::Duration::from_hours(6));
    let mut staged_nzb_prune_interval = tokio::time::interval(std::time::Duration::from_hours(1));
    let mut housekeeping_interval = tokio::time::interval(std::time::Duration::from_hours(24));
    let mut rss_sync_interval = tokio::time::interval(std::time::Duration::from_mins(15));
    let mut pending_release_interval = tokio::time::interval(std::time::Duration::from_mins(1));

    // Consume the first tick immediately
    poll_interval.tick().await;
    sync_interval.tick().await;
    metadata_refresh_interval.tick().await;
    registry_refresh_interval.tick().await;
    health_check_interval.tick().await;
    staged_nzb_prune_interval.tick().await;
    housekeeping_interval.tick().await;
    rss_sync_interval.tick().await;
    pending_release_interval.tick().await;

    let wake = app.runtime.acquisition.acquisition_wake.clone();

    /// Run a scheduled task inside a spawned task to isolate panics.
    /// If the task panics, the error is logged and the scheduler loop continues.
    async fn run_task(
        task_name: &'static str,
        fut: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let t = std::time::Instant::now();
        match tokio::spawn(fut).await {
            Ok(()) => {}
            Err(e) => {
                tracing::error!(
                    task = task_name,
                    error = %e,
                    "CRITICAL: scheduled task panicked — scheduler continues but this task failed"
                );
                metrics::counter!("scryer_task_panics_total", "task" => task_name).increment(1);
            }
        }
        metrics::counter!("scryer_task_runs_total", "task" => task_name).increment(1);
        metrics::histogram!("scryer_task_duration_seconds", "task" => task_name)
            .record(t.elapsed().as_secs_f64());
    }

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("background acquisition poller shutting down");
                break;
            }
            _ = wake.notified() => {
                let app = app.clone();
                run_task("wanted_items", async move {
                    process_due_wanted_items(&app).await;
                }).await;
            }
            _ = poll_interval.tick() => {
                let app = app.clone();
                run_task("wanted_items", async move {
                    process_due_wanted_items(&app).await;
                }).await;
            }
            _ = sync_interval.tick() => {
                let app = app.clone();
                run_task("sync_state", async move {
                    let sync_interval_seconds = app
                        .acquisition_settings()
                        .await
                        .map(|settings| settings.sync_interval_seconds.max(1) as i64)
                        .unwrap_or(60);
                    app.set_job_next_run_at(
                        JobKey::WantedSync,
                        Utc::now() + chrono::Duration::seconds(sync_interval_seconds),
                    ).await;
                    if let Err(err) = app.run_scheduled_job_now(JobKey::WantedSync, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %err, "periodic wanted state sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "sync_state").increment(1);
                    }
                }).await;
            }
            _ = metadata_refresh_interval.tick() => {
                let app = app.clone();
                run_task("metadata_refresh", async move {
                    app.set_job_next_run_at(
                        JobKey::MetadataRefresh,
                        Utc::now() + chrono::Duration::hours(12),
                    ).await;
                    if let Err(err) = app.run_scheduled_job_now(JobKey::MetadataRefresh, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %err, "periodic metadata refresh failed");
                    }
                }).await;
            }
            _ = registry_refresh_interval.tick() => {
                let app = app.clone();
                run_task("registry_refresh", async move {
                    app.set_job_next_run_at(
                        JobKey::PluginRegistryRefresh,
                        Utc::now() + chrono::Duration::hours(24),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::PluginRegistryRefresh, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic plugin registry refresh failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "registry_refresh").increment(1);
                    }
                }).await;
            }
            _ = health_check_interval.tick() => {
                let app = app.clone();
                run_task("health_check", async move {
                    app.set_job_next_run_at(
                        JobKey::HealthChecks,
                        Utc::now() + chrono::Duration::hours(6),
                    ).await;
                    if let Err(err) = app.run_scheduled_job_now(JobKey::HealthChecks, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %err, "periodic health checks failed");
                    }
                }).await;
            }
            _ = staged_nzb_prune_interval.tick() => {
                let app = app.clone();
                run_task("staged_nzb_prune", async move {
                    app.set_job_next_run_at(
                        JobKey::StagedNzbPrune,
                        Utc::now() + chrono::Duration::hours(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::StagedNzbPrune, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic staged nzb prune failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "staged_nzb_prune").increment(1);
                    }
                }).await;
            }
            _ = housekeeping_interval.tick() => {
                let app = app.clone();
                run_task("housekeeping", async move {
                    app.set_job_next_run_at(
                        JobKey::Housekeeping,
                        Utc::now() + chrono::Duration::hours(24),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::Housekeeping, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic housekeeping failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "housekeeping").increment(1);
                    }
                }).await;
            }
            _ = pending_release_interval.tick() => {
                let app = app.clone();
                run_task("pending_releases", async move {
                    app.set_job_next_run_at(
                        JobKey::PendingReleaseProcessing,
                        Utc::now() + chrono::Duration::minutes(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::PendingReleaseProcessing, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "pending release processor failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "pending_releases").increment(1);
                    }
                }).await;
            }
            _ = rss_sync_interval.tick() => {
                let app = app.clone();
                run_task("rss_sync", async move {
                    app.set_job_next_run_at(
                        JobKey::RssSync,
                        Utc::now() + chrono::Duration::minutes(15),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::RssSync, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic RSS sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "rss_sync").increment(1);
                    }
                }).await;
            }
        }
    }
}

fn new_skip_interval(period: std::time::Duration) -> tokio::time::Interval {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

/// Determine whether a movie has reached its configured availability threshold.
///
/// Returns `true` if the movie should be included in acquisition searches,
/// `false` if it should be skipped because its release dates haven't passed yet.
pub(crate) fn is_movie_available_for_acquisition(
    title: &Title,
    availability: &str,
    now: &DateTime<Utc>,
) -> bool {
    match availability {
        "in_cinemas" => title
            .first_aired
            .as_deref()
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|date| date <= now.date_naive())
            .unwrap_or(false),
        "released" => {
            if let Some(ref digital) = title.digital_release_date {
                chrono::NaiveDate::parse_from_str(digital, "%Y-%m-%d")
                    .map(|d| d <= now.date_naive())
                    .unwrap_or(false)
            } else if let Some(ref first_aired) = title.first_aired {
                // Fallback: first_aired + 90 days
                chrono::NaiveDate::parse_from_str(first_aired, "%Y-%m-%d")
                    .map(|d| d + chrono::Duration::days(90) <= now.date_naive())
                    .unwrap_or(false)
            } else {
                false
            }
        }
        // "announced" or anything else: always search
        _ => true,
    }
}

#[cfg(test)]
#[path = "app_usecase_acquisition_tests.rs"]
mod app_usecase_acquisition_tests;
