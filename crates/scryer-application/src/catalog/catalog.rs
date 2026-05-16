use super::*;
use crate::acquisition::acquisition::submission_blocks_wanted_item;
use crate::catalog_helpers::{
    DownloadClientRoutingEntry, anime_mapping_identity_keys, anime_movie_after_season,
    anime_movie_identity_keys, anime_movie_release_sort_key, build_rematched_external_ids,
    default_download_client_routing_entry, interstitial_movie_from_anime_movie,
    is_logical_specials_collection, parse_download_client_routing_entry,
    parse_download_client_routing_map, release_is_recent_for_queue_priority,
    strip_derived_match_tags,
};
use crate::contracts::{
    QueueDownloadOutcome, QueuedDownloadResult, SubmissionConflictPolicy, SubmissionScopeConflict,
};
use crate::domain_events::{deleted_media_update, new_title_domain_event, title_context_snapshot};
use crate::settings::settings::root_folder_entries_from_library_roots;
use scryer_domain::{
    DomainEventPayload, InterstitialMovieMetadata, MediaFileDeletedEventData,
    MediaFileDeletedReason, MetadataHydrationState, ReleaseGrabbedEventData, TitleAddedEventData,
    TitleDeletedEventData, TitleRematchedEventData,
};
use std::collections::HashMap;
use std::collections::HashSet;
use tracing::{debug, info, warn};

const RECENT_QUEUE_PRIORITY_WINDOW_DAYS: i64 = 14;
const REMATCH_REPLACED_EXTERNAL_ID_SOURCES: &[&str] =
    &["tvdb", "imdb", "tmdb", "mal", "anilist", "anidb", "kitsu"];
const REMATCH_DERIVED_TAG_PREFIXES: &[&str] = &[
    "scryer:mal-score:",
    "scryer:anime-media-type:",
    "scryer:anime-status:",
];
pub(crate) const HYDRATION_BULK_BATCH_SIZE: usize = 20;

fn blocklist_episode_ids(data_json: Option<&str>) -> Vec<String> {
    let Some(raw) = data_json else {
        return Vec::new();
    };

    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };

    let mut ids = Vec::new();

    if let Some(episode_id) = value.get("episode_id").and_then(serde_json::Value::as_str) {
        let trimmed = episode_id.trim();
        if !trimmed.is_empty() {
            ids.push(trimmed.to_string());
        }
    }

    if let Some(episode_ids) = value
        .get("episode_ids")
        .and_then(serde_json::Value::as_array)
    {
        for episode_id in episode_ids
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !ids.iter().any(|existing| existing == episode_id) {
                ids.push(episode_id.to_string());
            }
        }
    }

    ids
}

fn title_external_id_value(title: &Title, source: &str) -> Option<String> {
    if source == "imdb"
        && let Some(imdb_id) = title.imdb_id.as_deref()
        && !imdb_id.trim().is_empty()
    {
        return Some(imdb_id.trim().to_string());
    }

    title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == source && !external_id.value.trim().is_empty())
        .map(|external_id| external_id.value.trim().to_string())
}

fn push_title_external_id_index(
    map: &mut HashMap<String, Vec<Title>>,
    key: Option<String>,
    title: &Title,
) {
    let Some(key) = key else { return };
    map.entry(key).or_default().push(title.clone());
}

fn unique_title_match(map: &HashMap<String, Vec<Title>>, key: Option<&str>) -> Option<Title> {
    let key = key?.trim();
    if key.is_empty() {
        return None;
    }

    let matches = map.get(key)?;
    (matches.len() == 1).then(|| matches[0].clone())
}

fn unique_episode_match(
    episodes_by_tvdb: &HashMap<String, Vec<Episode>>,
    episodes_by_number: &HashMap<(String, String), Vec<Episode>>,
    tvdb_id: Option<&str>,
    season_number: i32,
    episode_number: i32,
) -> Option<Episode> {
    let tvdb_match = tvdb_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| episodes_by_tvdb.get(value))
        .and_then(|matches| (matches.len() == 1).then(|| matches[0].clone()));

    tvdb_match.or_else(|| {
        let key = (season_number.to_string(), episode_number.to_string());
        episodes_by_number
            .get(&key)
            .and_then(|matches| (matches.len() == 1).then(|| matches[0].clone()))
    })
}

fn wanted_item_candidates_for_submission_scope(
    title_id: &str,
    scope: &SubmissionScope,
    episodes: &[Episode],
) -> Vec<(WantedItem, Option<String>)> {
    match scope {
        SubmissionScope::Orphan => Vec::new(),
        SubmissionScope::Title => vec![(
            WantedItem {
                id: String::new(),
                title_id: title_id.to_string(),
                title_name: None,
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: None,
                collection_id: None,
                season_number: None,
                episode_number: None,
                media_type: "movie".to_string(),
                search_phase: String::new(),
                next_search_at: None,
                last_search_at: None,
                search_count: 0,
                baseline_date: None,
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: String::new(),
                updated_at: String::new(),
            },
            None,
        )],
        SubmissionScope::Episode { episode_id } => {
            let candidate = episodes
                .iter()
                .find(|episode| episode.id == *episode_id)
                .map(|episode| {
                    (
                        wanted_item_candidate_for_episode(title_id, episode),
                        episode.collection_id.clone(),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        wanted_item_candidate_for_episode_id(title_id, episode_id, None, None),
                        None,
                    )
                });
            vec![candidate]
        }
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids
            .iter()
            .map(|episode_id| {
                episodes
                    .iter()
                    .find(|episode| episode.id == *episode_id)
                    .map(|episode| {
                        (
                            wanted_item_candidate_for_episode(title_id, episode),
                            episode.collection_id.clone(),
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            wanted_item_candidate_for_episode_id(title_id, episode_id, None, None),
                            None,
                        )
                    })
            })
            .collect(),
        SubmissionScope::Collection { collection_id } => {
            let mut candidates = episodes
                .iter()
                .filter(|episode| episode.collection_id.as_deref() == Some(collection_id.as_str()))
                .map(|episode| {
                    (
                        wanted_item_candidate_for_episode(title_id, episode),
                        episode.collection_id.clone(),
                    )
                })
                .collect::<Vec<_>>();
            candidates.push((
                WantedItem {
                    id: String::new(),
                    title_id: title_id.to_string(),
                    title_name: None,
                    title_slug: None,
                    title_facet: None,
                    library_id: None,
                    library_name: None,
                    library_slug: None,
                    episode_id: None,
                    collection_id: Some(collection_id.clone()),
                    season_number: None,
                    episode_number: None,
                    media_type: "interstitial_movie".to_string(),
                    search_phase: String::new(),
                    next_search_at: None,
                    last_search_at: None,
                    search_count: 0,
                    baseline_date: None,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    latest_release_decision: None,
                    mismatch_recovery_eligible: false,
                    created_at: String::new(),
                    updated_at: String::new(),
                },
                Some(collection_id.clone()),
            ));
            candidates
        }
    }
}

fn wanted_item_candidate_for_episode(title_id: &str, episode: &Episode) -> WantedItem {
    wanted_item_candidate_for_episode_id(
        title_id,
        &episode.id,
        episode.collection_id.clone(),
        episode.season_number.clone(),
    )
}

fn wanted_item_candidate_for_episode_id(
    title_id: &str,
    episode_id: &str,
    collection_id: Option<String>,
    season_number: Option<String>,
) -> WantedItem {
    WantedItem {
        id: String::new(),
        title_id: title_id.to_string(),
        title_name: None,
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some(episode_id.to_string()),
        collection_id,
        season_number,
        episode_number: None,
        media_type: "episode".to_string(),
        search_phase: String::new(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn submission_for_scope(title_id: &str, scope: &SubmissionScope) -> DownloadSubmission {
    DownloadSubmission {
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
    }
}

fn submission_scopes_overlap(
    title_id: &str,
    existing: &SubmissionScope,
    requested: &SubmissionScope,
    episodes: &[Episode],
) -> bool {
    let existing_submission = submission_for_scope(title_id, existing);
    if wanted_item_candidates_for_submission_scope(title_id, requested, episodes)
        .iter()
        .any(|(item, collection_id)| {
            submission_blocks_wanted_item(&existing_submission, item, collection_id.as_deref())
        })
    {
        return true;
    }

    let requested_submission = submission_for_scope(title_id, requested);
    wanted_item_candidates_for_submission_scope(title_id, existing, episodes)
        .iter()
        .any(|(item, collection_id)| {
            submission_blocks_wanted_item(&requested_submission, item, collection_id.as_deref())
        })
}

fn queue_state_blocks_submission(state: DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Queued
            | DownloadQueueState::Downloading
            | DownloadQueueState::Paused
            | DownloadQueueState::Verifying
            | DownloadQueueState::Repairing
            | DownloadQueueState::Extracting
            | DownloadQueueState::ImportPending
    )
}

fn queue_state_is_replaceable(state: DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Queued | DownloadQueueState::Downloading | DownloadQueueState::Paused
    )
}

fn queue_item_matches_submission(
    item: &DownloadQueueItem,
    submission: &DownloadSubmission,
) -> bool {
    item.download_client_item_id == submission.download_client_item_id
        && submission
            .download_client_id
            .as_deref()
            .map(|client_id| client_id == item.client_id)
            .unwrap_or(true)
}

fn blocking_queue_item_for_submission<'a>(
    queue: &'a [DownloadQueueItem],
    submission: &DownloadSubmission,
) -> Option<&'a DownloadQueueItem> {
    queue.iter().find(|item| {
        queue_item_matches_submission(item, submission) && queue_state_blocks_submission(item.state)
    })
}

fn anibridge_scoped_external_ids_from_mappings(
    anime_mappings: &[AnimeMapping],
    season_number_to_collection: &HashMap<i32, String>,
    episodes_by_number: &HashMap<(i32, i32), Episode>,
) -> (Vec<ScopedExternalId>, Vec<ScopedExternalId>) {
    let known_episodes_by_season = known_episode_numbers_by_season(episodes_by_number);
    let mut collection_ids = Vec::new();
    let mut episode_ids = Vec::new();
    let mut seen_collections = HashSet::new();
    let mut seen_episodes = HashSet::new();

    for mapping in anime_mappings {
        let external_ids = anime_mapping_external_ids(mapping);
        if external_ids.is_empty() {
            continue;
        }
        let source_scope = non_empty_scope(mapping.mapping_type.as_str());

        if mapping.episode_mappings.is_empty() {
            if let Some(season) = mapping.thetvdb_season
                && let Some(collection_id) = season_number_to_collection.get(&season)
            {
                push_scoped_external_ids(
                    &mut collection_ids,
                    &mut seen_collections,
                    collection_id,
                    &external_ids,
                    source_scope.as_deref(),
                );
            }
            continue;
        }

        let mut covered_by_season = HashMap::<i32, std::collections::BTreeSet<i32>>::new();
        for episode_mapping in &mapping.episode_mappings {
            if episode_mapping.episode_start > episode_mapping.episode_end {
                continue;
            }
            let Some(known_episode_numbers) =
                known_episodes_by_season.get(&episode_mapping.tvdb_season)
            else {
                continue;
            };
            for episode_number in known_episode_numbers
                .range(episode_mapping.episode_start..=episode_mapping.episode_end)
                .copied()
            {
                let Some(episode) =
                    episodes_by_number.get(&(episode_mapping.tvdb_season, episode_number))
                else {
                    continue;
                };
                push_scoped_external_ids(
                    &mut episode_ids,
                    &mut seen_episodes,
                    &episode.id,
                    &external_ids,
                    source_scope.as_deref(),
                );
                covered_by_season
                    .entry(episode_mapping.tvdb_season)
                    .or_default()
                    .insert(episode_number);
            }
        }

        for (season, covered) in covered_by_season {
            let Some(known) = known_episodes_by_season.get(&season) else {
                continue;
            };
            let Some(collection_id) = season_number_to_collection.get(&season) else {
                continue;
            };
            if !known.is_empty() && known.iter().all(|episode| covered.contains(episode)) {
                push_scoped_external_ids(
                    &mut collection_ids,
                    &mut seen_collections,
                    collection_id,
                    &external_ids,
                    source_scope.as_deref(),
                );
            }
        }
    }

    (collection_ids, episode_ids)
}

fn known_episode_numbers_by_season(
    episodes_by_number: &HashMap<(i32, i32), Episode>,
) -> HashMap<i32, std::collections::BTreeSet<i32>> {
    let mut known = HashMap::<i32, std::collections::BTreeSet<i32>>::new();
    for (season, episode_number) in episodes_by_number.keys().copied() {
        known.entry(season).or_default().insert(episode_number);
    }
    known
}

fn anime_mapping_external_ids(mapping: &AnimeMapping) -> Vec<(&'static str, String)> {
    let mut ids = Vec::new();
    push_optional_mapping_id(&mut ids, "mal", mapping.mal_id);
    push_optional_mapping_id(&mut ids, "mal_dub", mapping.mal_dub_id);
    push_optional_mapping_id(&mut ids, "anilist", mapping.anilist_id);
    push_optional_mapping_id(&mut ids, "anidb", mapping.anidb_id);
    push_optional_mapping_id(&mut ids, "kitsu", mapping.kitsu_id);
    push_optional_mapping_id(&mut ids, "simkl", mapping.simkl_id);
    push_optional_mapping_id(&mut ids, "tvdb", mapping.thetvdb_id);
    push_optional_mapping_id(&mut ids, "tmdb", mapping.themoviedb_id);
    push_optional_mapping_id(&mut ids, "imdb", mapping.imdb_id);
    push_optional_mapping_id(&mut ids, "trakt", mapping.trakt_id);
    push_optional_mapping_id(&mut ids, "alt_tvdb", mapping.alt_tvdb_id);
    ids
}

fn push_optional_mapping_id(
    ids: &mut Vec<(&'static str, String)>,
    source: &'static str,
    value: Option<i64>,
) {
    if let Some(value) = value
        && value > 0
    {
        ids.push((source, value.to_string()));
    }
}

fn push_scoped_external_ids(
    out: &mut Vec<ScopedExternalId>,
    seen: &mut HashSet<(String, String, String, String)>,
    scope_id: &str,
    external_ids: &[(&'static str, String)],
    source_scope: Option<&str>,
) {
    let scope_id = scope_id.trim();
    if scope_id.is_empty() {
        return;
    }
    let source_scope = source_scope.unwrap_or_default().trim();
    for (source, external_id) in external_ids {
        let external_id = external_id.trim();
        if external_id.is_empty() {
            continue;
        }
        let key = (
            scope_id.to_string(),
            (*source).to_string(),
            external_id.to_string(),
            source_scope.to_string(),
        );
        if seen.insert(key) {
            out.push(ScopedExternalId {
                scope_id: scope_id.to_string(),
                source: (*source).to_string(),
                external_id: external_id.to_string(),
                provenance: "anibridge".to_string(),
                source_scope: if source_scope.is_empty() {
                    None
                } else {
                    Some(source_scope.to_string())
                },
            });
        }
    }
}

fn non_empty_scope(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HydrationCompletionOptions {
    sync_wanted_after_completion: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HydrationSource {
    BackgroundDue,
    LibraryScanFull,
    LibraryScanAdditive,
    Interactive,
    Maintenance,
}

impl HydrationSource {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::BackgroundDue => "background_due",
            Self::LibraryScanFull => "library_scan_full",
            Self::LibraryScanAdditive => "library_scan_additive",
            Self::Interactive => "interactive",
            Self::Maintenance => "maintenance",
        }
    }
}

#[derive(Clone)]
pub(crate) struct HydrationTarget {
    pub(crate) title: Title,
    pub(crate) requested_tvdb_id: Option<i64>,
    pub(crate) sync_wanted_after_completion: bool,
    pub(crate) source: HydrationSource,
}

#[derive(Default)]
pub(crate) struct HydrationBatchOutcome {
    pub(crate) hydrated_titles: HashMap<String, Title>,
    pub(crate) failed_titles: HashMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TitleLogicalDeleteOptions {
    pub(crate) purge_recycle_bin_entries: bool,
    pub(crate) append_title_deleted_event: bool,
}

impl AppUseCase {
    async fn emit_hydration_started(&self, title: &Title) {
        self.emit_metadata_hydration_updated_event(title, MetadataHydrationState::Started, None)
            .await;
    }

    async fn emit_hydration_completed(&self, title: &Title) {
        self.emit_metadata_hydration_updated_event(title, MetadataHydrationState::Completed, None)
            .await;
    }

    async fn emit_hydration_failed(&self, title: &Title, reason: &str) {
        self.emit_metadata_hydration_updated_event(
            title,
            MetadataHydrationState::Failed,
            Some(reason.to_string()),
        )
        .await;
    }

    async fn read_download_client_routing_value(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(value) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await?
        {
            return Ok(Some(value));
        }

        self.read_setting_string_value(LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
    }

    async fn read_explicit_download_client_routing_value(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(value) = self
            .read_setting_string_value_explicit(
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id),
            )
            .await?
        {
            return Ok(Some(value));
        }

        self.read_setting_string_value_explicit(
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope_id),
        )
        .await
    }

    /// Returns `Some(entry)` when the persisted JSON has an entry for this
    /// client in this scope, else `None`. Callers are responsible for applying
    /// the canonical default — the explicit fallback site — so the read path
    /// stays a thin lookup over normalized data. Legacy installs converge via
    /// the startup `normalize_routing_settings` pass.
    async fn read_download_client_routing_entry(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientRoutingEntry>> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Ok(None);
        }

        if let Some(library_id) = library_id.map(str::trim).filter(|value| !value.is_empty())
            && let Some(raw_json) = self
                .read_explicit_download_client_routing_value(library_id)
                .await?
            && let Some(routing_map) = parse_download_client_routing_map(&raw_json)
        {
            if let Some(config) = routing_map.get(client_id) {
                return Ok(Some(parse_download_client_routing_entry(config)));
            }

            let mut disabled_entry = default_download_client_routing_entry();
            disabled_entry.enabled = false;
            return Ok(Some(disabled_entry));
        }

        let scope_id = facet.as_str();

        let Some(raw_json) = self.read_download_client_routing_value(scope_id).await? else {
            return Ok(None);
        };

        let Some(routing_map) = parse_download_client_routing_map(&raw_json) else {
            return Ok(None);
        };

        Ok(routing_map
            .get(client_id)
            .map(parse_download_client_routing_entry))
    }

    pub(crate) async fn should_remove_completed_download(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        match self
            .read_download_client_routing_entry(library_id, facet, client_id)
            .await
            .ok()
            .flatten()
        {
            Some(entry) => entry.remove_completed,
            None => default_download_client_routing_entry().remove_completed,
        }
    }

    pub(crate) async fn should_remove_failed_download(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        match self
            .read_download_client_routing_entry(library_id, facet, client_id)
            .await
            .ok()
            .flatten()
        {
            Some(entry) => entry.remove_failed,
            None => default_download_client_routing_entry().remove_failed,
        }
    }

    pub(crate) fn is_recent_for_queue_priority(&self, baseline_date: Option<&str>) -> Option<bool> {
        baseline_date.map(|_| {
            release_is_recent_for_queue_priority(baseline_date, RECENT_QUEUE_PRIORITY_WINDOW_DAYS)
        })
    }

    pub(crate) async fn metadata_language(&self) -> String {
        self.read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, METADATA_LANGUAGE_KEY, None)
            .await
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "eng".to_string())
    }

    pub async fn list_titles(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let mut library_ids = self
            .authorized_library_ids(actor, facet.clone(), scryer_domain::LibraryPermission::View)
            .await?;
        let requested_library_ids = requested_library_ids
            .as_ref()
            .map(|requested| {
                requested
                    .iter()
                    .map(|library_id| library_id.trim())
                    .filter(|library_id| !library_id.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if !requested_library_ids.is_empty() {
            library_ids.retain(|library_id| requested_library_ids.contains(library_id));
        }
        self.services
            .catalog
            .titles
            .list_for_libraries(facet, &library_ids, query)
            .await
    }

    pub async fn list_titles_without_external_ids(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let mut library_ids = self
            .authorized_library_ids(actor, facet.clone(), scryer_domain::LibraryPermission::View)
            .await?;
        let requested_library_ids = requested_library_ids
            .as_ref()
            .map(|requested| {
                requested
                    .iter()
                    .map(|library_id| library_id.trim())
                    .filter(|library_id| !library_id.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if !requested_library_ids.is_empty() {
            library_ids.retain(|library_id| requested_library_ids.contains(library_id));
        }
        self.services
            .catalog
            .titles
            .list_for_libraries_without_external_ids(facet, &library_ids, query)
            .await
    }

    pub async fn list_titles_by_external_ids(
        &self,
        actor: &User,
        source: &str,
        values: &[String],
    ) -> AppResult<Vec<Title>> {
        let normalized_source = source.trim();
        if normalized_source.is_empty() {
            return Ok(Vec::new());
        }

        let mut seen = HashSet::new();
        let mut normalized_values = Vec::new();
        for value in values {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                normalized_values.push(trimmed.to_string());
            }
        }

        if normalized_values.is_empty() {
            return Ok(Vec::new());
        }

        let titles = self
            .services
            .catalog
            .titles
            .list_by_external_ids(normalized_source, &normalized_values)
            .await?;
        let library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        Ok(titles
            .into_iter()
            .filter(|title| library_ids.contains(&title.library_id))
            .collect())
    }

    pub async fn list_cutoff_unmet_titles(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
    ) -> AppResult<Vec<CutoffUnmetItem>> {
        let authorized_libraries = self
            .list_libraries_for_permission(
                actor,
                facet.clone(),
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        let library_name_by_id = authorized_libraries
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect::<HashMap<_, _>>();
        let library_slug_by_id = authorized_libraries
            .iter()
            .map(|library| (library.id.clone(), library.slug.clone()))
            .collect::<HashMap<_, _>>();
        let mut library_ids = authorized_libraries
            .iter()
            .map(|library| library.id.clone())
            .collect::<Vec<_>>();
        let requested_library_ids = requested_library_ids
            .as_ref()
            .map(|requested| {
                requested
                    .iter()
                    .map(|library_id| library_id.trim())
                    .filter(|library_id| !library_id.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if !requested_library_ids.is_empty() {
            library_ids.retain(|library_id| requested_library_ids.contains(library_id));
        }
        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(facet, &library_ids, None)
            .await?;
        let monitored_titles = titles
            .into_iter()
            .filter(|title| title.monitored)
            .collect::<Vec<_>>();
        if monitored_titles.is_empty() {
            return Ok(Vec::new());
        }

        let title_ids = monitored_titles
            .iter()
            .map(|title| title.id.clone())
            .collect::<Vec<_>>();
        let quality_summaries = self
            .services
            .library
            .media_files
            .list_cutoff_unmet_quality_summaries(&title_ids)
            .await?;

        let profile_settings = self.load_quality_profile_settings().await?;
        let global_profile_id = Some(profile_settings.global_profile_id.as_str());
        let profile_map: HashMap<&str, &QualityProfile> = profile_settings
            .profiles
            .iter()
            .map(|profile| (profile.id.as_str(), profile))
            .collect();
        let default_profile = crate::default_quality_profile_for_search();

        let mut title_map = HashMap::new();
        let mut cutoff_profile_map = HashMap::new();
        for title in monitored_titles {
            let title_profile_id = extract_tag_string(&title.tags, "scryer:quality-profile:")
                .map(str::trim)
                .filter(|value| {
                    !value.is_empty() && *value != crate::QUALITY_PROFILE_INHERIT_VALUE
                });
            let category_profile_id = profile_settings
                .category_selections
                .iter()
                .find(|selection| selection.facet == title.facet)
                .and_then(|selection| selection.override_profile_id.as_deref());

            let resolved_profile_id = crate::resolve_profile_id_for_title(
                title_profile_id,
                None,
                category_profile_id,
                global_profile_id,
            );
            let profile = resolved_profile_id
                .as_deref()
                .and_then(|profile_id| profile_map.get(profile_id).copied())
                .unwrap_or(&default_profile);

            if !profile.criteria.allow_upgrades {
                continue;
            }

            let Some(cutoff_tier) = profile
                .criteria
                .cutoff_tier
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };

            let Some(normalized_cutoff_tier) =
                crate::quality_profile::normalize_quality_tier(Some(cutoff_tier))
            else {
                continue;
            };

            if !profile
                .criteria
                .quality_tiers
                .iter()
                .any(|tier| tier == &normalized_cutoff_tier)
            {
                continue;
            }

            cutoff_profile_map.insert(
                title.id.clone(),
                (
                    profile.criteria.quality_tiers.clone(),
                    normalized_cutoff_tier,
                ),
            );
            title_map.insert(title.id.clone(), title);
        }

        let mut items = Vec::new();
        for summary in quality_summaries {
            let Some(title) = title_map.get(summary.title_id.as_str()) else {
                continue;
            };
            let Some((quality_tiers, normalized_cutoff_tier)) =
                cutoff_profile_map.get(summary.title_id.as_str())
            else {
                continue;
            };

            if summary.episode_id.is_none() && title.facet != MediaFacet::Movie {
                continue;
            }

            let Some(normalized_current_tier) =
                crate::quality_profile::normalize_quality_tier(Some(summary.quality_tier.as_str()))
            else {
                continue;
            };

            if !quality_tiers
                .iter()
                .any(|tier| tier == &normalized_current_tier)
            {
                continue;
            }

            if crate::quality_profile::quality_meets_or_exceeds_cutoff(
                normalized_current_tier.as_str(),
                normalized_cutoff_tier.as_str(),
                quality_tiers,
            ) {
                continue;
            }

            items.push(CutoffUnmetItem {
                title_id: title.id.clone(),
                title_name: title.name.clone(),
                title_slug: title.slug.clone(),
                title_facet: title.facet.clone(),
                library_id: title.library_id.clone(),
                library_name: library_name_by_id.get(&title.library_id).cloned(),
                library_slug: library_slug_by_id.get(&title.library_id).cloned(),
                episode_id: summary.episode_id,
                season_number: summary.season_number,
                episode_number: summary.episode_number,
                current_tier: normalized_current_tier,
                target_tier: normalized_cutoff_tier.clone(),
            });
        }

        fn parse_episode_sort_number(value: Option<&str>) -> i64 {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|value| {
                    let digits = value
                        .chars()
                        .filter(|ch| ch.is_ascii_digit())
                        .collect::<String>();
                    if digits.is_empty() {
                        None
                    } else {
                        digits.parse::<i64>().ok()
                    }
                })
                .unwrap_or(i64::MAX)
        }

        items.sort_by(|left, right| {
            left.title_name
                .to_ascii_lowercase()
                .cmp(&right.title_name.to_ascii_lowercase())
                .then_with(|| {
                    parse_episode_sort_number(left.season_number.as_deref())
                        .cmp(&parse_episode_sort_number(right.season_number.as_deref()))
                })
                .then_with(|| {
                    parse_episode_sort_number(left.episode_number.as_deref())
                        .cmp(&parse_episode_sort_number(right.episode_number.as_deref()))
                })
        });

        Ok(items)
    }

    pub async fn list_title_release_blocklist(
        &self,
        actor: &User,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        let bounded_limit = limit.clamp(1, 1_000);
        let submissions = self
            .services
            .workflow
            .download_submissions
            .list_for_title(title_id)
            .await
            .unwrap_or_default();
        let episode_ids_by_download_id: HashMap<String, Vec<String>> = submissions
            .into_iter()
            .filter_map(|submission| {
                let episode_ids = submission.scope.episode_ids()?.to_vec();
                if episode_ids.is_empty() {
                    None
                } else {
                    Some((submission.download_client_item_id, episode_ids))
                }
            })
            .collect();
        let entries = self
            .services
            .workflow
            .blocklist_repo
            .list_for_title(title_id, bounded_limit)
            .await?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let mut episode_ids = blocklist_episode_ids(entry.data_json.as_deref());
                if episode_ids.is_empty()
                    && let Some(download_id) = entry.download_id.as_deref()
                    && let Some(submission_episode_ids) =
                        episode_ids_by_download_id.get(download_id)
                {
                    episode_ids = submission_episode_ids.clone();
                }

                TitleReleaseBlocklistEntry {
                    id: entry.id,
                    source_hint: entry.source_hint,
                    source_title: entry.source_title,
                    error_message: entry.reason,
                    attempted_at: entry.created_at,
                    episode_ids,
                }
            })
            .collect())
    }

    pub async fn clear_title_release_blocklist_entry(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<()> {
        let (entries, _) = self
            .services
            .workflow
            .blocklist_repo
            .list_all(500, 0)
            .await?;
        let entry = entries
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("blocklist entry {id}")))?;
        self.require_title_permission(
            actor,
            &entry.title_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.services.workflow.blocklist_repo.remove(id).await
    }

    /// Return the configured root folders for a facet.
    ///
    /// Reads canonical roots from the facet's default library. Legacy
    /// `<facet>.root_folders` and `<facet>.path` settings are maintained only
    /// as compatibility mirrors and are reconciled during startup.
    pub async fn root_folders_for_facet(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<scryer_domain::RootFolderEntry>> {
        let handler = self.facet_registry.get(facet);
        let default_path = handler.map(|h| h.default_library_path()).unwrap_or("/data");

        if let Some(library) = self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
        {
            let entries = root_folder_entries_from_library_roots(&library.roots);

            if !entries.is_empty() {
                return Ok(entries);
            }
        }

        Ok(vec![scryer_domain::RootFolderEntry {
            path: default_path.to_string(),
            is_default: true,
        }])
    }

    /// Return the configured root folders for a concrete library.
    ///
    /// If a stale title points at a missing or empty library, fall back to the
    /// facet default roots so existing data remains importable.
    pub(crate) async fn root_folders_for_library(
        &self,
        library_id: &str,
        fallback_facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<scryer_domain::RootFolderEntry>> {
        if let Some(library) = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
        {
            if library.facet != *fallback_facet {
                warn!(
                    library_id = %library.id,
                    library_facet = library.facet.as_str(),
                    title_facet = fallback_facet.as_str(),
                    "library facet does not match title facet; falling back to facet default roots"
                );
                return self.root_folders_for_facet(fallback_facet).await;
            }

            let entries = root_folder_entries_from_library_roots(&library.roots);
            if !entries.is_empty() {
                return Ok(entries);
            }
            warn!(
                library_id = %library.id,
                facet = library.facet.as_str(),
                "library has no roots; falling back to facet default roots"
            );
        } else {
            warn!(
                library_id,
                facet = fallback_facet.as_str(),
                "library is missing; falling back to facet default roots"
            );
        }

        self.root_folders_for_facet(fallback_facet).await
    }

    pub(crate) async fn default_media_root_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<String> {
        let handler = self.facet_registry.get(&title.facet);
        let default_path = handler.map(|h| h.default_library_path()).unwrap_or("/data");
        let root_folders = self
            .root_folders_for_library(&title.library_id, &title.facet)
            .await?;

        Ok(root_folders
            .iter()
            .find(|entry| entry.is_default)
            .or_else(|| root_folders.first())
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| default_path.to_string()))
    }

    pub async fn add_title_with_outcome(
        &self,
        actor: &User,
        request: NewTitle,
    ) -> AppResult<AddTitleOutcome> {
        let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
        self.add_title_with_outcome_in_library(actor, request, library_id)
            .await
    }

    pub async fn add_title_with_outcome_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
    ) -> AppResult<AddTitleOutcome> {
        let created = self
            .create_title_without_hydration_in_library(actor, request, library_id)
            .await?;
        self.notify_title_image_wakes(&created.title);

        let metadata_hydration_state = if created.title.metadata_fetched_at.is_some() {
            AddTitleHydrationState::Complete
        } else if extract_tvdb_id(&created.title).is_some() {
            if created.reused_existing {
                self.services
                    .catalog
                    .titles
                    .mark_title_metadata_hydration_due_now(&created.title.id)
                    .await?;
            }
            self.runtime.catalog.title_hydration_wake.notify_one();
            AddTitleHydrationState::Pending
        } else {
            self.services
                .catalog
                .titles
                .clear_title_metadata_hydration_retry_state(&created.title.id)
                .await?;
            AddTitleHydrationState::NotRequired
        };

        Ok(AddTitleOutcome {
            title: created.title,
            metadata_hydration_state,
            reused_existing_title: created.reused_existing,
        })
    }

    pub async fn add_title(&self, actor: &User, request: NewTitle) -> AppResult<Title> {
        Ok(self.add_title_with_outcome(actor, request).await?.title)
    }

    #[cfg(test)]
    pub(crate) async fn create_title_without_hydration(
        &self,
        actor: &User,
        request: NewTitle,
    ) -> AppResult<CreateTitleOutcome> {
        let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
        self.create_title_without_hydration_in_library(actor, request, library_id)
            .await
    }

    pub(crate) async fn create_title_without_hydration_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
    ) -> AppResult<CreateTitleOutcome> {
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.create_title_without_hydration_after_library_authorization(actor, request, library_id)
            .await
    }

    pub(crate) async fn create_title_without_hydration_after_library_authorization(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
    ) -> AppResult<CreateTitleOutcome> {
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("title name is required".into()));
        }

        let title = Title {
            id: Id::new().0,
            library_id,
            name: request.name.trim().to_string(),
            facet: request.facet,
            monitored: request.monitored,
            tags: normalize_tags(&request.tags),
            external_ids: sanitize_ids(request.external_ids),
            created_by: Some(actor.id.clone()),
            created_at: Utc::now(),
            year: request.year,
            overview: request.overview,
            poster_url: request.poster_url,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: request.sort_title,
            slug: request.slug,
            imdb_id: None,
            runtime_minutes: request.runtime_minutes,
            genres: vec![],
            content_status: request.content_status,
            language: request.language,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: request.min_availability,
            digital_release_date: None,
            folder_path: None,
        };

        let created = self
            .services
            .catalog
            .titles
            .create_or_get_existing(title)
            .await?;
        if !created.reused_existing {
            self.append_domain_event(new_title_domain_event(
                Some(actor.id.clone()),
                &created.title,
                DomainEventPayload::TitleAdded(TitleAddedEventData {
                    title: title_context_snapshot(&created.title),
                }),
            ))
            .await?;
        }

        Ok(created)
    }

    fn notify_title_image_wakes(&self, title: &Title) {
        if title
            .poster_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.poster_wake.notify_one();
        }
        if title
            .banner_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.banner_wake.notify_one();
        }
        if title
            .background_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.fanart_wake.notify_one();
        }
    }

    async fn lock_download_submission_signature(
        &self,
        title_id: &str,
        request_signature: Option<&str>,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.runtime
            .acquisition
            .download_submission_guards
            .acquire(title_id, request_signature)
            .await
    }

    async fn lock_download_submission_scope(
        &self,
        title_id: &str,
        scope: &SubmissionScope,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        self.runtime
            .acquisition
            .download_submission_guards
            .acquire_scope(title_id, scope)
            .await
    }

    pub(crate) async fn find_blocking_download_submissions(
        &self,
        title: &Title,
        scope: &SubmissionScope,
    ) -> AppResult<Vec<SubmissionScopeConflict>> {
        let submissions = self
            .services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await?;
        if submissions.is_empty() {
            return Ok(Vec::new());
        }

        let queue = self
            .services
            .integrations
            .download_client
            .list_queue()
            .await?;
        if queue.is_empty() {
            return Ok(Vec::new());
        }

        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;

        let mut conflicts = Vec::new();
        for submission in submissions {
            if !submission_scopes_overlap(&title.id, &submission.scope, scope, &episodes) {
                continue;
            }

            let Some(queue_item) = blocking_queue_item_for_submission(&queue, &submission) else {
                continue;
            };

            conflicts.push(SubmissionScopeConflict {
                title_id: title.id.clone(),
                title_name: title.name.clone(),
                download_client_id: submission.download_client_id.clone(),
                download_client_type: submission.download_client_type.clone(),
                download_client_item_id: submission.download_client_item_id.clone(),
                source_title: submission.source_title.clone(),
                source_kind: submission.source_kind,
                scope: submission.scope,
                state: Some(queue_item.state),
                replaceable: queue_state_is_replaceable(queue_item.state),
            });
        }

        Ok(conflicts)
    }

    pub(crate) async fn replace_blocking_download_submission(
        &self,
        conflict: &SubmissionScopeConflict,
    ) -> AppResult<()> {
        if !conflict.replaceable {
            return Err(AppError::Validation(
                "the existing download is no longer safe to replace".into(),
            ));
        }

        if let Some(client_id) = conflict.download_client_id.as_deref() {
            self.services
                .integrations
                .download_client
                .delete_queue_item_for_client_id(
                    client_id,
                    &conflict.download_client_item_id,
                    false,
                )
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .delete_queue_item_for_client(
                    &conflict.download_client_type,
                    &conflict.download_client_item_id,
                    false,
                )
                .await?;
        }

        self.services
            .workflow
            .download_submissions
            .delete_by_client_item_id(&DownloadSourceIdentity::new(
                conflict.download_client_id.as_deref(),
                &conflict.download_client_type,
                &conflict.download_client_item_id,
            ))
            .await?;
        self.reset_wanted_items_for_submission_scope(&conflict.title_id, &conflict.scope)
            .await?;

        Ok(())
    }

    pub(crate) async fn replace_blocking_download_submissions(
        &self,
        conflicts: &[SubmissionScopeConflict],
    ) -> AppResult<()> {
        for conflict in conflicts {
            self.replace_blocking_download_submission(conflict).await?;
        }

        Ok(())
    }

    async fn complete_title_hydration(&self, title: &Title, options: HydrationCompletionOptions) {
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            facet = %title.facet.as_str(),
            metadata_fetched = title.metadata_fetched_at.is_some(),
            sync_wanted_after_completion = options.sync_wanted_after_completion,
            "complete_title_hydration invoked"
        );

        if title.metadata_fetched_at.is_some() {
            self.notify_title_image_wakes(title);
            self.emit_hydration_completed(title).await;
            self.emit_title_updated_activity(None, title).await;
            if options.sync_wanted_after_completion {
                sync_wanted_after_hydration(self, title).await;
            }
        } else {
            debug!(
                title_id = %title.id,
                title_name = %title.name,
                "complete_title_hydration missing persisted metadata"
            );
            self.emit_hydration_failed(title, "metadata could not be persisted")
                .await;
        }
    }

    pub(crate) async fn hydrate_titles_bulk_cancellable(
        &self,
        targets: Vec<HydrationTarget>,
        cancel_token: Option<&tokio_util::sync::CancellationToken>,
    ) -> AppResult<HydrationBatchOutcome> {
        let language = self.metadata_language().await;
        let mut outcome = HydrationBatchOutcome::default();

        'chunks: for chunk in targets.chunks(HYDRATION_BULK_BATCH_SIZE) {
            if crate::library::library::library_scan_cancel_requested(cancel_token) {
                break;
            }
            let mut movie_targets = Vec::new();
            let mut series_targets = Vec::new();

            for target in chunk.iter().cloned() {
                self.emit_hydration_started(&target.title).await;

                let Some(tvdb_id) = target
                    .requested_tvdb_id
                    .or_else(|| extract_tvdb_id(&target.title))
                else {
                    warn!(
                        hydration_source = target.source.as_str(),
                        facet = target.title.facet.as_str(),
                        title_id = %target.title.id,
                        "title hydration failed: no tvdb external id found"
                    );
                    self.emit_hydration_failed(&target.title, "no tvdb external id found")
                        .await;
                    outcome.failed_titles.insert(
                        target.title.id.clone(),
                        "no tvdb external id found".to_string(),
                    );
                    continue;
                };

                match target.title.facet {
                    MediaFacet::Movie => movie_targets.push((target, tvdb_id)),
                    MediaFacet::Series | MediaFacet::Anime => {
                        series_targets.push((target, tvdb_id))
                    }
                }
            }

            if movie_targets.is_empty() && series_targets.is_empty() {
                continue;
            }

            let movie_ids = movie_targets
                .iter()
                .map(|(_, tvdb_id)| *tvdb_id)
                .collect::<Vec<_>>();
            let series_ids = series_targets
                .iter()
                .map(|(_, tvdb_id)| *tvdb_id)
                .collect::<Vec<_>>();

            let bulk_result = await_cancellable(
                cancel_token,
                self.services.library.metadata_gateway.get_metadata_bulk(
                    &movie_ids,
                    &series_ids,
                    &language,
                ),
            )
            .await;

            let Some(bulk_result) = bulk_result else {
                break;
            };

            let bulk_result = match bulk_result {
                Ok(result) => result,
                Err(error) => {
                    let reason = error.to_string();
                    for (target, _) in movie_targets.iter().chain(series_targets.iter()) {
                        warn!(
                            hydration_source = target.source.as_str(),
                            facet = target.title.facet.as_str(),
                            title_id = %target.title.id,
                            error = %error,
                            "title hydration bulk metadata request failed"
                        );
                        self.emit_hydration_failed(&target.title, &reason).await;
                        outcome
                            .failed_titles
                            .insert(target.title.id.clone(), reason.clone());
                    }
                    continue;
                }
            };

            for (target, tvdb_id) in movie_targets {
                if crate::library::library::library_scan_cancel_requested(cancel_token) {
                    break 'chunks;
                }
                let title_id = target.title.id.clone();
                let title_facet = target.title.facet.clone();
                let title_source = target.source;
                if let Some(movie) = bulk_result.movies.get(&tvdb_id) {
                    let result = super::movie_to_hydration_result(movie.clone(), &language);
                    let hydrated = self
                        .apply_hydration_result(target.title, result, title_source)
                        .await;
                    self.complete_title_hydration(
                        &hydrated,
                        HydrationCompletionOptions {
                            sync_wanted_after_completion: target.sync_wanted_after_completion,
                        },
                    )
                    .await;
                    let refreshed = self
                        .services
                        .catalog
                        .titles
                        .get_by_id(&hydrated.id)
                        .await?
                        .unwrap_or(hydrated);
                    if refreshed.metadata_fetched_at.is_some() {
                        outcome
                            .hydrated_titles
                            .insert(refreshed.id.clone(), refreshed);
                    } else {
                        warn!(
                            hydration_source = title_source.as_str(),
                            facet = title_facet.as_str(),
                            title_id = %title_id,
                            "title hydration failed: metadata could not be persisted"
                        );
                        outcome
                            .failed_titles
                            .insert(title_id, "metadata could not be persisted".to_string());
                    }
                } else {
                    warn!(
                        hydration_source = title_source.as_str(),
                        facet = title_facet.as_str(),
                        title_id = %title_id,
                        "title hydration failed: bulk metadata response missing movie title"
                    );
                    self.emit_hydration_failed(
                        &target.title,
                        "bulk metadata response missing title",
                    )
                    .await;
                    outcome
                        .failed_titles
                        .insert(title_id, "bulk metadata response missing title".to_string());
                }
            }

            for (target, tvdb_id) in series_targets {
                if crate::library::library::library_scan_cancel_requested(cancel_token) {
                    break 'chunks;
                }
                let title_id = target.title.id.clone();
                let title_facet = target.title.facet.clone();
                let title_source = target.source;
                if let Some(series) = bulk_result.series.get(&tvdb_id) {
                    let result = super::series_to_hydration_result(series.clone(), &language);
                    let hydrated = self
                        .apply_hydration_result(target.title, result, title_source)
                        .await;
                    self.complete_title_hydration(
                        &hydrated,
                        HydrationCompletionOptions {
                            sync_wanted_after_completion: target.sync_wanted_after_completion,
                        },
                    )
                    .await;
                    let refreshed = self
                        .services
                        .catalog
                        .titles
                        .get_by_id(&hydrated.id)
                        .await?
                        .unwrap_or(hydrated);
                    if refreshed.metadata_fetched_at.is_some() {
                        outcome
                            .hydrated_titles
                            .insert(refreshed.id.clone(), refreshed);
                    } else {
                        warn!(
                            hydration_source = title_source.as_str(),
                            facet = title_facet.as_str(),
                            title_id = %title_id,
                            "title hydration failed: metadata could not be persisted"
                        );
                        outcome
                            .failed_titles
                            .insert(title_id, "metadata could not be persisted".to_string());
                    }
                } else {
                    warn!(
                        hydration_source = title_source.as_str(),
                        facet = title_facet.as_str(),
                        title_id = %title_id,
                        "title hydration failed: bulk metadata response missing series title"
                    );
                    self.emit_hydration_failed(
                        &target.title,
                        "bulk metadata response missing title",
                    )
                    .await;
                    outcome
                        .failed_titles
                        .insert(title_id, "bulk metadata response missing title".to_string());
                }
            }
        }

        Ok(outcome)
    }

    pub(crate) async fn hydrate_titles_bulk(
        &self,
        targets: Vec<HydrationTarget>,
    ) -> AppResult<HydrationBatchOutcome> {
        self.hydrate_titles_bulk_cancellable(targets, None).await
    }

    /// Apply a [`HydrationResult`] to a title: persist metadata, create
    /// seasons/episodes, and enrich with anime mapping data.
    async fn apply_hydration_result(
        &self,
        title: Title,
        result: super::HydrationResult,
        source: HydrationSource,
    ) -> Title {
        let has_episodes = self
            .facet_registry
            .get(&title.facet)
            .is_some_and(|h| h.has_episodes());

        if has_episodes {
            debug!(
                hydration_source = source.as_str(),
                facet = title.facet.as_str(),
                title_id = %title.id,
                seasons = result.seasons.len(),
                episodes = result.episodes.len(),
                "received series metadata from gateway"
            );
        }

        let mut metadata_update = result.metadata_update;

        // Store anime-specific metadata as tags on the title
        if let Some(primary) =
            crate::catalog::facets::handler::primary_anime_mapping(&result.anime_mappings)
        {
            if let Some(score) = primary.score {
                metadata_update
                    .extra_tags
                    .push(format!("scryer:mal-score:{score}"));
            }
            if !primary.anime_media_type.is_empty() {
                metadata_update.extra_tags.push(format!(
                    "scryer:anime-media-type:{}",
                    primary.anime_media_type
                ));
            }
            if !primary.status.is_empty() {
                metadata_update
                    .extra_tags
                    .push(format!("scryer:anime-status:{}", primary.status));
            }
        }

        let title = match self
            .services
            .catalog
            .titles
            .update_title_hydrated_metadata(&title.id, metadata_update)
            .await
        {
            Ok(updated) => updated,
            Err(err) => {
                warn!(
                    hydration_source = source.as_str(),
                    facet = title.facet.as_str(),
                    title_id = %title.id,
                    error = %err,
                    "failed to persist metadata"
                );
                title
            }
        };

        if !result.seasons.is_empty() || !result.episodes.is_empty() {
            self.create_series_seasons_and_episodes(
                &title,
                &result.seasons,
                &result.episodes,
                &result.anime_mappings,
                &result.anime_movies,
            )
            .await;
        }

        if title
            .poster_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.poster_wake.notify_one();
        }
        if title
            .banner_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.banner_wake.notify_one();
        }
        if title
            .background_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.fanart_wake.notify_one();
        }

        title
    }

    pub(crate) async fn create_series_seasons_and_episodes(
        &self,
        title: &Title,
        seasons: &[SeasonMetadata],
        episodes: &[EpisodeMetadata],
        anime_mappings: &[AnimeMapping],
        anime_movies: &[AnimeMovie],
    ) {
        let monitor_type = if title.monitored {
            extract_monitor_type(&title.tags)
        } else {
            "none".to_string()
        };
        info!(
            title_id = %title.id,
            monitor_type = %monitor_type,
            tags = ?title.tags,
            episode_count = episodes.len(),
            "creating series seasons and episodes"
        );

        // Fetch existing collections so we can reuse them instead of creating
        // duplicates on every metadata refresh cycle.
        let existing_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let mut existing_collections_by_id: std::collections::HashMap<String, Collection> =
            existing_collections
                .iter()
                .map(|collection| (collection.id.clone(), collection.clone()))
                .collect();
        let mut existing_collection_map: std::collections::HashMap<
            (CollectionType, String),
            String,
        > = existing_collections
            .iter()
            .map(|c| {
                (
                    (c.collection_type, c.collection_index.clone()),
                    c.id.clone(),
                )
            })
            .collect();
        if !existing_collection_map.contains_key(&(CollectionType::Specials, "0".to_string()))
            && let Some(legacy_specials_id) = existing_collections
                .iter()
                .find(|collection| is_logical_specials_collection(collection))
                .map(|collection| collection.id.clone())
        {
            existing_collection_map.insert(
                (CollectionType::Specials, "0".to_string()),
                legacy_specials_id,
            );
        }
        let mut existing_episode_lookup: std::collections::HashMap<(String, String), Episode> =
            self.services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter_map(|episode| {
                    let season_number = episode.season_number.clone()?;
                    let episode_number = episode.episode_number.clone()?;
                    Some(((season_number, episode_number), episode))
                })
                .collect();

        // Build a map from season number -> collection_id for episode assignment.
        // Only create one collection per season number, preferring "official" episode_type.
        let mut best_season_by_number: std::collections::HashMap<i32, &SeasonMetadata> =
            std::collections::HashMap::new();
        for season in seasons {
            let existing = best_season_by_number.get(&season.number);
            if existing.is_none() || season.episode_type == "official" {
                best_season_by_number.insert(season.number, season);
            }
        }

        let monitor_specials = if title.facet == MediaFacet::Anime {
            // Per-title tag overrides global setting
            if let Some(per_title) = extract_tag_bool(&title.tags, "scryer:monitor-specials:") {
                per_title
            } else {
                self.resolve_library_bool_setting(
                    "anime.monitor_specials",
                    Some(&title.library_id),
                    Some(title.facet.as_str()),
                    false,
                )
                .await
                .unwrap_or(false)
            }
        } else {
            false
        };

        let inter_season_movies = if title.facet == MediaFacet::Anime {
            if let Some(per_title) = extract_tag_bool(&title.tags, "scryer:inter-season-movies:") {
                per_title
            } else {
                self.resolve_library_bool_setting(
                    "anime.inter_season_movies",
                    Some(&title.library_id),
                    Some(title.facet.as_str()),
                    true,
                )
                .await
                .unwrap_or(true)
            }
        } else {
            false
        };

        // Regular seasons should auto-monitor on creation even before SMG has
        // episode rows. Specials still require episode data so empty season-0
        // shells do not become monitored unless they are backed by episodes.
        let seasons_with_episodes: std::collections::HashSet<i32> =
            episodes.iter().map(|ep| ep.season_number).collect();

        let derived_anime_movies: Vec<&AnimeMovie> =
            if title.facet == MediaFacet::Anime && inter_season_movies {
                anime_movies
                    .iter()
                    .filter(|movie| {
                        !movie.name.trim().is_empty()
                            && matches!(movie.association_confidence.as_str(), "medium" | "high")
                    })
                    .collect()
            } else {
                vec![]
            };
        let specials_movies: Vec<InterstitialMovieMetadata> = derived_anime_movies
            .iter()
            .copied()
            .filter(|movie| movie.placement == "specials")
            .map(interstitial_movie_from_anime_movie)
            .collect();
        let ordered_movies: Vec<&AnimeMovie> = derived_anime_movies
            .iter()
            .copied()
            .filter(|movie| movie.placement != "specials")
            .collect();

        let mut season_number_to_collection: std::collections::HashMap<i32, String> =
            std::collections::HashMap::new();

        for season in best_season_by_number.values() {
            let season_should_monitor =
                should_monitor_season(&monitor_type, season.number, monitor_specials);
            let season_monitored = if season.number == 0 {
                seasons_with_episodes.contains(&season.number) && season_should_monitor
            } else {
                season_should_monitor
            };
            let collection_type = if season.number == 0 {
                CollectionType::Specials
            } else {
                CollectionType::Season
            };
            let collection_index = season.number.to_string();
            if let Some(existing_id) =
                existing_collection_map.get(&(collection_type, collection_index.clone()))
            {
                // Update language-sensitive label if it changed
                if !season.label.is_empty()
                    && let Some(existing) = existing_collections_by_id.get(existing_id)
                    && existing.label.as_deref() != Some(&season.label)
                {
                    let _ = self
                        .services
                        .catalog
                        .shows
                        .update_collection(
                            existing_id,
                            CollectionUpdate {
                                label: Some(season.label.clone()),
                                ..Default::default()
                            },
                        )
                        .await;
                    if let Some(existing) = existing_collections_by_id.get_mut(existing_id) {
                        existing.label = Some(season.label.clone());
                    }
                }
                if season.number == 0
                    && title.facet == MediaFacet::Anime
                    && let Some(existing) = existing_collections_by_id.get(existing_id)
                    && existing.specials_movies != specials_movies
                {
                    match self
                        .services
                        .catalog
                        .shows
                        .update_collection_specials_movies(existing_id, specials_movies.clone())
                        .await
                    {
                        Ok(updated) => {
                            existing_collections_by_id.insert(existing_id.clone(), updated);
                        }
                        Err(err) => {
                            warn!(
                                title_id = %title.id,
                                collection_id = %existing_id,
                                error = %err,
                                "failed to update specials movie metadata"
                            );
                        }
                    }
                }
                season_number_to_collection.insert(season.number, existing_id.clone());
                continue;
            }

            let collection = Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type,
                collection_index,
                label: Some(season.label.clone()),
                ordered_path: None,
                narrative_order: Some(season.number.to_string()),
                first_episode_number: None,
                last_episode_number: None,
                interstitial_movie: None,
                specials_movies: if season.number == 0 && title.facet == MediaFacet::Anime {
                    specials_movies.clone()
                } else {
                    vec![]
                },
                interstitial_season_episode: None,
                monitored: season_monitored,
                created_at: Utc::now(),
            };

            match self
                .services
                .catalog
                .shows
                .create_collection(collection.clone())
                .await
            {
                Ok(created) => {
                    existing_collections_by_id.insert(created.id.clone(), created.clone());
                    season_number_to_collection.insert(season.number, created.id);
                }
                Err(err) => {
                    warn!(
                        title_id = %title.id,
                        season_number = season.number,
                        error = %err,
                        "failed to create season collection"
                    );
                }
            }
        }

        // Build last-aired date per regular season from the episode data so
        // we can determine where each interstitial movie falls narratively.
        let mut season_last_aired: std::collections::BTreeMap<i32, String> =
            std::collections::BTreeMap::new();
        for ep in episodes.iter() {
            if ep.season_number > 0 && !ep.aired.is_empty() {
                season_last_aired
                    .entry(ep.season_number)
                    .and_modify(|d| {
                        if ep.aired > *d {
                            *d = ep.aired.clone();
                        }
                    })
                    .or_insert_with(|| ep.aired.clone());
            }
        }

        // Create interstitial movie collections for anime titles using the
        // derived anime_movies payload from SMG. Episode mappings are only used
        // to route any linked season-0 episode records into the movie collection
        // when a matching mapping still exists.
        let mut interstitial_episode_lookup: std::collections::HashMap<(i32, i32), String> =
            std::collections::HashMap::new();

        if title.facet == MediaFacet::Anime && inter_season_movies && !ordered_movies.is_empty() {
            let mut mapping_episode_links: HashMap<String, Vec<(i32, i32)>> = HashMap::new();
            for mapping in anime_mappings {
                let identity_keys = anime_mapping_identity_keys(mapping);
                if identity_keys.is_empty() || mapping.episode_mappings.is_empty() {
                    continue;
                }
                let mut linked_episodes = Vec::new();
                for em in &mapping.episode_mappings {
                    for ep_num in em.episode_start..=em.episode_end {
                        linked_episodes.push((em.tvdb_season, ep_num));
                    }
                }
                for key in identity_keys {
                    mapping_episode_links
                        .entry(key)
                        .or_default()
                        .extend(linked_episodes.iter().copied());
                }
            }

            let mut movies_by_position: std::collections::BTreeMap<i32, Vec<&AnimeMovie>> =
                std::collections::BTreeMap::new();
            for movie in &ordered_movies {
                let after_season = anime_movie_after_season(movie, &season_last_aired);
                movies_by_position
                    .entry(after_season)
                    .or_default()
                    .push(*movie);
            }

            for (after_season, movies) in &mut movies_by_position {
                movies.sort_by(|left, right| {
                    anime_movie_release_sort_key(left)
                        .cmp(&anime_movie_release_sort_key(right))
                        .then_with(|| left.name.cmp(&right.name))
                });

                for (seq, movie) in movies.iter().enumerate() {
                    let narrative_order = format!("{}.{}", after_season, seq + 1);
                    let label = if movie.continuity_status == "canon" {
                        movie.name.clone()
                    } else {
                        format!("Movie {}", seq + 1)
                    };
                    let interstitial_movie = interstitial_movie_from_anime_movie(movie);

                    // Reuse existing interstitial collection if one already exists.
                    if let Some(existing_id) = existing_collection_map
                        .get(&(CollectionType::Interstitial, narrative_order.clone()))
                    {
                        // Update language-sensitive label if it changed
                        if !label.is_empty()
                            && let Some(existing_coll) = existing_collections_by_id.get(existing_id)
                            && existing_coll.label.as_deref() != Some(&label)
                        {
                            let _ = self
                                .services
                                .catalog
                                .shows
                                .update_collection(
                                    existing_id,
                                    CollectionUpdate {
                                        label: Some(label.clone()),
                                        ..Default::default()
                                    },
                                )
                                .await;
                            if let Some(existing_coll) =
                                existing_collections_by_id.get_mut(existing_id)
                            {
                                existing_coll.label = Some(label.clone());
                            }
                        }
                        if let Some(existing_coll) = existing_collections_by_id.get(existing_id)
                            && existing_coll.interstitial_movie.as_ref()
                                != Some(&interstitial_movie)
                        {
                            match self
                                .services
                                .catalog
                                .shows
                                .update_collection_interstitial_movie(
                                    existing_id,
                                    interstitial_movie.clone(),
                                )
                                .await
                            {
                                Ok(updated) => {
                                    existing_collections_by_id.insert(existing_id.clone(), updated);
                                }
                                Err(err) => {
                                    warn!(
                                        title_id = %title.id,
                                        collection_id = %existing_id,
                                        error = %err,
                                        "failed to update interstitial movie metadata"
                                    );
                                }
                            }
                        }

                        // Update interstitial_season_episode if it changed or was missing
                        let new_season_episode = anime_movie_identity_keys(movie)
                            .iter()
                            .filter_map(|key| mapping_episode_links.get(key.as_str()))
                            .flatten()
                            .find(|(s, _)| *s == 0)
                            .map(|(_, ep)| format!("S00E{:0>2}", ep));
                        if let Some(ref se) = new_season_episode
                            && let Some(existing_coll) = existing_collections_by_id.get(existing_id)
                            && existing_coll.interstitial_season_episode.as_deref()
                                != Some(se.as_str())
                        {
                            let _ = self
                                .services
                                .catalog
                                .shows
                                .update_interstitial_season_episode(existing_id, Some(se.clone()))
                                .await;
                            if let Some(existing_coll) =
                                existing_collections_by_id.get_mut(existing_id)
                            {
                                existing_coll.interstitial_season_episode = Some(se.clone());
                            }
                        }

                        for key in anime_movie_identity_keys(movie) {
                            if let Some(linked_episodes) = mapping_episode_links.get(&key) {
                                for (season_num, episode_num) in linked_episodes {
                                    interstitial_episode_lookup
                                        .insert((*season_num, *episode_num), existing_id.clone());
                                }
                            }
                        }
                        continue;
                    }

                    // Compute the S00Exx episode number from the linked episode data
                    let season_episode = anime_movie_identity_keys(movie)
                        .iter()
                        .filter_map(|key| mapping_episode_links.get(key.as_str()))
                        .flatten()
                        .find(|(s, _)| *s == 0)
                        .map(|(_, ep)| format!("S00E{:0>2}", ep));

                    let collection = Collection {
                        id: Id::new().0,
                        title_id: title.id.clone(),
                        collection_type: CollectionType::Interstitial,
                        collection_index: narrative_order.clone(),
                        label: Some(label.clone()),
                        ordered_path: None,
                        narrative_order: Some(narrative_order.clone()),
                        first_episode_number: None,
                        last_episode_number: None,
                        interstitial_movie: Some(interstitial_movie),
                        specials_movies: vec![],
                        interstitial_season_episode: season_episode,
                        monitored: false,
                        created_at: Utc::now(),
                    };

                    match self
                        .services
                        .catalog
                        .shows
                        .create_collection(collection)
                        .await
                    {
                        Ok(created) => {
                            existing_collections_by_id.insert(created.id.clone(), created.clone());
                            debug!(
                                title_id = %title.id,
                                label = %label,
                                narrative_order = %narrative_order,
                                placement = %movie.placement,
                                "created interstitial movie collection"
                            );
                            for key in anime_movie_identity_keys(movie) {
                                if let Some(linked_episodes) = mapping_episode_links.get(&key) {
                                    for (season_num, episode_num) in linked_episodes {
                                        interstitial_episode_lookup.insert(
                                            (*season_num, *episode_num),
                                            created.id.clone(),
                                        );
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            warn!(
                                title_id = %title.id,
                                label = %label,
                                error = %err,
                                "failed to create interstitial movie collection"
                            );
                        }
                    }
                }
            }
        }

        // Build a lookup from season number → season episode_type for deriving episode type.
        let season_episode_types: std::collections::HashMap<i32, &str> = best_season_by_number
            .iter()
            .map(|(&num, s)| (num, s.episode_type.as_str()))
            .collect();

        let today = Utc::now().format("%Y-%m-%d").to_string();

        let skip_filler = if title.facet == MediaFacet::Anime {
            let effective = match extract_tag_string(&title.tags, "scryer:filler-policy:") {
                Some(v) => v.to_string(),
                None => self
                    .resolve_library_string_setting(
                        "anime.filler_policy",
                        Some(&title.library_id),
                        Some(title.facet.as_str()),
                        "download_all",
                    )
                    .await
                    .unwrap_or_else(|_| "download_all".to_string()),
            };
            effective == "skip_filler"
        } else {
            false
        };
        let skip_recap = if title.facet == MediaFacet::Anime {
            let effective = match extract_tag_string(&title.tags, "scryer:recap-policy:") {
                Some(v) => v.to_string(),
                None => self
                    .resolve_library_string_setting(
                        "anime.recap_policy",
                        Some(&title.library_id),
                        Some(title.facet.as_str()),
                        "download_all",
                    )
                    .await
                    .unwrap_or_else(|_| "download_all".to_string()),
            };
            effective == "skip_recap"
        } else {
            false
        };

        // Track which interstitial collections have had their label updated
        // to the first episode's name (e.g. "Movie 1" → "Mugen Train").
        let mut labeled_collections: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for ep in episodes {
            let season_number_key = ep.season_number.to_string();
            let episode_number_key = ep.episode_number.to_string();

            // Check interstitial episode lookup first (routes movie episodes to their
            // interstitial collections), then fall back to the season-based lookup.
            let collection_id = interstitial_episode_lookup
                .get(&(ep.season_number, ep.episode_number))
                .cloned()
                .or_else(|| season_number_to_collection.get(&ep.season_number).cloned());

            // If this episode is routed to an interstitial collection and the
            // collection is still using a generic placeholder label, update it
            // to the episode's name (once per collection).
            if let Some(ref cid) = collection_id
                && interstitial_episode_lookup.contains_key(&(ep.season_number, ep.episode_number))
                && !ep.name.is_empty()
                && labeled_collections.insert(cid.clone())
                && existing_collections_by_id
                    .get(cid)
                    .is_some_and(|collection| {
                        collection
                            .label
                            .as_deref()
                            .is_none_or(|label| label.is_empty() || label.starts_with("Movie "))
                    })
                && let Err(err) = self
                    .services
                    .catalog
                    .shows
                    .update_collection(
                        cid,
                        CollectionUpdate {
                            label: Some(ep.name.clone()),
                            ..Default::default()
                        },
                    )
                    .await
            {
                warn!(
                    collection_id = %cid,
                    error = %err,
                    "failed to update interstitial collection label"
                );
            }

            let air_date = if ep.aired.is_empty() {
                None
            } else {
                Some(ep.aired.clone())
            };
            let episode_monitored = if (skip_filler && ep.is_filler) || (skip_recap && ep.is_recap)
            {
                false
            } else {
                should_monitor_episode(
                    &monitor_type,
                    ep.season_number,
                    air_date.as_deref(),
                    &today,
                    monitor_specials,
                )
            };

            let anime_media_type = if title.facet == MediaFacet::Anime {
                anime_mappings
                    .iter()
                    .find(|m| m.thetvdb_season == Some(ep.season_number))
                    .map(|m| m.anime_media_type.as_str())
            } else {
                None
            };

            let episode_type = derive_episode_type(
                ep.season_number,
                season_episode_types.get(&ep.season_number).copied(),
                anime_media_type,
            );

            // If episode already exists, update language-sensitive fields instead of skipping.
            if let Some(existing) = existing_episode_lookup
                .get(&(season_number_key.clone(), episode_number_key.clone()))
                .cloned()
            {
                let new_title = if ep.name.is_empty() {
                    None
                } else {
                    Some(ep.name.clone())
                };
                let new_overview = if ep.overview.trim().is_empty() {
                    None
                } else {
                    Some(ep.overview.clone())
                };
                // Only update if the new data differs from existing
                let title_changed = new_title.as_deref() != existing.title.as_deref();
                let overview_changed = new_overview.as_deref() != existing.overview.as_deref();
                let new_tvdb_id = if ep.tvdb_id > 0 {
                    Some(ep.tvdb_id.to_string())
                } else {
                    None
                };
                let tvdb_id_changed = new_tvdb_id.as_deref() != existing.tvdb_id.as_deref();
                if title_changed || overview_changed || tvdb_id_changed {
                    let _ = self
                        .services
                        .catalog
                        .shows
                        .update_episode(
                            &existing.id,
                            EpisodeUpdate {
                                episode_label: if title_changed {
                                    new_title.clone()
                                } else {
                                    None
                                },
                                title: if title_changed { new_title } else { None },
                                overview: if overview_changed { new_overview } else { None },
                                tvdb_id: if tvdb_id_changed { new_tvdb_id } else { None },
                                ..Default::default()
                            },
                        )
                        .await;
                }
                continue;
            }

            let episode = Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id,
                episode_type,
                episode_number: Some(episode_number_key.clone()),
                season_number: Some(season_number_key.clone()),
                episode_label: Some(ep.name.clone()),
                title: Some(ep.name.clone()),
                air_date,
                duration_seconds: if ep.runtime_minutes > 0 {
                    Some(i64::from(ep.runtime_minutes) * 60)
                } else {
                    None
                },
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: ep.is_filler,
                is_recap: ep.is_recap,
                absolute_number: if ep.absolute_number.is_empty() {
                    None
                } else {
                    Some(ep.absolute_number.clone())
                },
                overview: if ep.overview.trim().is_empty() {
                    None
                } else {
                    Some(ep.overview.clone())
                },
                tvdb_id: if ep.tvdb_id > 0 {
                    Some(ep.tvdb_id.to_string())
                } else {
                    None
                },
                monitored: episode_monitored,
                created_at: Utc::now(),
            };

            match self.services.catalog.shows.create_episode(episode).await {
                Ok(created) => {
                    existing_episode_lookup
                        .insert((season_number_key, episode_number_key), created);
                }
                Err(err) => {
                    warn!(
                        title_id = %title.id,
                        episode_number = ep.episode_number,
                        error = %err,
                        "failed to create episode"
                    );
                }
            }
        }

        if title.facet == MediaFacet::Anime {
            let episode_lookup_by_number: HashMap<(i32, i32), Episode> = existing_episode_lookup
                .values()
                .filter_map(|episode| {
                    let season = episode.season_number.as_deref()?.parse::<i32>().ok()?;
                    let episode_number = episode.episode_number.as_deref()?.parse::<i32>().ok()?;
                    Some(((season, episode_number), episode.clone()))
                })
                .collect();
            let (collection_external_ids, episode_external_ids) =
                anibridge_scoped_external_ids_from_mappings(
                    anime_mappings,
                    &season_number_to_collection,
                    &episode_lookup_by_number,
                );
            if let Err(err) = self
                .services
                .catalog
                .shows
                .replace_anibridge_scoped_external_ids_for_title(
                    &title.id,
                    collection_external_ids,
                    episode_external_ids,
                )
                .await
            {
                warn!(
                    title_id = %title.id,
                    error = %err,
                    "failed to persist scoped anibridge external IDs"
                );
            }
        }
    }

    async fn queue_manual_release_for_title(
        &self,
        actor: &User,
        title: &Title,
        queued_release: QueuedReleaseSelection,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let QueuedReleaseSelection {
            source_hint,
            source_kind,
            source_title,
        } = queued_release;
        let source_hint_for_attempt = normalize_release_attempt_value(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_value(source_title.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint_for_attempt.as_deref(),
            source_title_for_attempt.as_deref(),
            source_kind,
        );
        let source_password: Option<String> = None;
        let scope_guard = self.lock_download_submission_scope(&title.id, &scope).await;
        let dedupe_guard = self
            .lock_download_submission_signature(&title.id, request_signature.as_deref())
            .await;

        if let Some(signature) = request_signature.as_deref()
            && let Some(existing) = self
                .services
                .workflow
                .download_submissions
                .find_by_title_and_request_signature(&title.id, signature)
                .await?
        {
            let queue = self
                .services
                .integrations
                .download_client
                .list_queue()
                .await?;
            if blocking_queue_item_for_submission(&queue, &existing).is_some() {
                drop(dedupe_guard);
                drop(scope_guard);
                return Ok(QueueDownloadOutcome::Queued(QueuedDownloadResult {
                    job_id: existing.download_client_item_id,
                    queued_release: QueuedReleaseSelection {
                        source_hint,
                        source_kind,
                        source_title,
                    },
                    reused_existing: true,
                }));
            }
        }

        let conflicts = self
            .find_blocking_download_submissions(title, &scope)
            .await?;
        if !conflicts.is_empty() {
            match conflict_policy {
                SubmissionConflictPolicy::Abort | SubmissionConflictPolicy::Skip => {
                    drop(dedupe_guard);
                    drop(scope_guard);
                    return Ok(QueueDownloadOutcome::Conflict(conflicts[0].clone()));
                }
                SubmissionConflictPolicy::ReplaceEarly
                    if conflicts.iter().all(|conflict| conflict.replaceable) =>
                {
                    self.replace_blocking_download_submissions(&conflicts)
                        .await?;
                }
                SubmissionConflictPolicy::ReplaceEarly => {
                    let conflict = conflicts
                        .into_iter()
                        .find(|conflict| !conflict.replaceable)
                        .expect("non-empty conflicts should contain a non-replaceable item");
                    drop(dedupe_guard);
                    drop(scope_guard);
                    return Ok(QueueDownloadOutcome::Conflict(conflict));
                }
            }
        }

        let _ = self
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

        let category = self.derive_download_category(&title.facet).await;
        let is_recent = self.is_recent_for_queue_priority(
            title
                .first_aired
                .as_deref()
                .or(title.digital_release_date.as_deref()),
        );
        let job_result = self
            .services
            .integrations
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                source_hint,
                staged_nzb: None,
                source_kind,
                source_title,
                source_password: source_password.clone(),
                category: Some(category),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: None,
            })
            .await;

        let grab = match job_result {
            Ok(grab) => {
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => "manual", "facet" => facet_label).increment(1);
                }
                let _ = self
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
                let facet_str =
                    serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
                let _ = self
                    .services
                    .workflow
                    .download_submissions
                    .record_submission(DownloadSubmission {
                        title_id: title.id.clone(),
                        facet: facet_str.trim_matches('"').to_string(),
                        download_client_id: grab.client_id.clone(),
                        download_client_type: grab.client_type.clone(),
                        download_client_item_id: grab.job_id.clone(),
                        source_hint: source_hint_for_attempt.clone(),
                        source_kind,
                        source_title: source_title_for_attempt.clone(),
                        request_signature: request_signature.clone(),
                        scope,
                    })
                    .await;
                grab
            }
            Err(error) => {
                let error_message = error.to_string();
                let _ = self
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        ReleaseDownloadAttemptOutcome::Failed,
                        Some(error_message.clone()),
                        source_password,
                    )
                    .await;
                let blocklist_episode_ids = match &scope {
                    SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
                    SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
                    SubmissionScope::Collection { collection_id } => self
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(collection_id)
                        .await
                        .map(|episodes| episodes.into_iter().map(|episode| episode.id).collect())
                        .unwrap_or_default(),
                    SubmissionScope::Title | SubmissionScope::Orphan => Vec::new(),
                };
                let mut blocklist_data = HashMap::new();
                if !blocklist_episode_ids.is_empty() {
                    blocklist_data.insert(
                        "episode_ids".to_string(),
                        serde_json::json!(blocklist_episode_ids),
                    );
                }
                if let SubmissionScope::Collection { collection_id } = &scope {
                    blocklist_data.insert(
                        "collection_id".to_string(),
                        serde_json::json!(collection_id),
                    );
                }
                let _ = self
                    .services
                    .workflow
                    .blocklist_repo
                    .add(&NewBlocklistEntry {
                        title_id: title.id.clone(),
                        source_title: source_title_for_attempt.clone(),
                        source_hint: source_hint_for_attempt.clone(),
                        quality: None,
                        download_id: None,
                        reason: Some(error_message.clone()),
                        data: blocklist_data,
                    })
                    .await;
                drop(dedupe_guard);
                drop(scope_guard);
                return Err(error);
            }
        };

        drop(dedupe_guard);
        drop(scope_guard);

        self.append_domain_event(new_title_domain_event(
            Some(actor.id.clone()),
            title,
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: title_context_snapshot(title),
                source_title: None,
                source_hint: None,
                download_id: Some(grab.job_id.clone()),
                episode_ids: Vec::new(),
            }),
        ))
        .await?;

        Ok(QueueDownloadOutcome::Queued(QueuedDownloadResult {
            job_id: grab.job_id,
            queued_release: QueuedReleaseSelection {
                source_hint: source_hint_for_attempt,
                source_kind,
                source_title: source_title_for_attempt,
            },
            reused_existing: false,
        }))
    }

    pub async fn add_title_and_queue_download_with_outcome(
        &self,
        actor: &User,
        request: NewTitle,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<AddTitleAndQueueDownloadOutcome> {
        let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
        self.add_title_and_queue_download_with_outcome_in_library(
            actor,
            request,
            library_id,
            queued_release,
        )
        .await
    }

    pub async fn add_title_and_queue_download_with_outcome_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<AddTitleAndQueueDownloadOutcome> {
        let add_outcome = self
            .add_title_with_outcome_in_library(actor, request, library_id)
            .await?;
        let title = add_outcome.title.clone();
        let queued = self
            .queue_manual_release_for_title(
                actor,
                &title,
                queued_release,
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await?;
        let QueueDownloadOutcome::Queued(queued) = queued else {
            return Err(AppError::Validation(
                "a download is already queued for this title".into(),
            ));
        };

        Ok(AddTitleAndQueueDownloadOutcome {
            title,
            metadata_hydration_state: add_outcome.metadata_hydration_state,
            reused_existing_title: add_outcome.reused_existing_title,
            download_job_id: queued.job_id,
            reused_queued_download: queued.reused_existing,
        })
    }

    pub async fn add_title_and_queue_download(
        &self,
        actor: &User,
        request: NewTitle,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<(Title, String)> {
        let outcome = self
            .add_title_and_queue_download_with_outcome(actor, request, queued_release)
            .await?;
        Ok((outcome.title, outcome.download_job_id))
    }

    pub async fn queue_existing_title_download(
        &self,
        actor: &User,
        title_id: &str,
        queued_release: QueuedReleaseSelection,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.queue_manual_release_for_title(actor, &title, queued_release, scope, conflict_policy)
            .await
    }

    pub async fn queue_existing_title_download_from_candidate_token(
        &self,
        actor: &User,
        title_id: &str,
        candidate_token: &str,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let (queued_release, signed_scope) = self
            .verify_release_candidate_token_for_signed_scope(actor, title_id, candidate_token)
            .await?;
        let outcome = self
            .queue_existing_title_download(
                actor,
                title_id,
                queued_release.clone(),
                signed_scope,
                conflict_policy,
            )
            .await?;
        let _ = scope;
        Ok(match outcome {
            QueueDownloadOutcome::Queued(mut queued) => {
                queued.queued_release = queued_release;
                QueueDownloadOutcome::Queued(queued)
            }
            QueueDownloadOutcome::Conflict(conflict) => QueueDownloadOutcome::Conflict(conflict),
        })
    }

    pub async fn queue_best_release(
        &self,
        actor: &User,
        title_id: &str,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let (search_title, subject) = match &scope {
            SubmissionScope::Title | SubmissionScope::Orphan => (
                title.clone(),
                self.resolve_release_search_subject_for_title(&title)
                    .await?,
            ),
            SubmissionScope::Episode { episode_id } => {
                let episode = self
                    .services
                    .catalog
                    .shows
                    .get_episode_by_id(episode_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
                let season = episode.season_number.clone().ok_or_else(|| {
                    AppError::Validation("episode is missing season number".into())
                })?;
                let episode_number = episode.episode_number.clone().ok_or_else(|| {
                    AppError::Validation("episode is missing episode number".into())
                })?;
                (
                    title.clone(),
                    self.resolve_release_search_subject_for_episode(
                        &title,
                        &season,
                        &episode_number,
                    )
                    .await?,
                )
            }
            SubmissionScope::EpisodeSet { .. } => {
                return Err(AppError::Validation(
                    "best-release search is not supported for multi-episode scopes".into(),
                ));
            }
            SubmissionScope::Collection { collection_id } => {
                let collection = self
                    .services
                    .catalog
                    .shows
                    .get_collection_by_id(collection_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
                self.resolve_release_search_subject_for_collection(&title, &collection)
                    .await?
            }
        };

        let results = self
            .search_and_evaluate_subject(&search_title, &subject, &actor.id, SearchMode::Auto)
            .await?;
        let best = results
            .into_iter()
            .find(|candidate| candidate.auto_eligible == Some(true))
            .ok_or_else(|| AppError::Validation("no auto-eligible release found".into()))?;
        let queue_scope = if matches!(&scope, SubmissionScope::Collection { .. }) {
            scope
        } else if let Some(parsed) = best.parsed_release_metadata.as_ref() {
            let catalog_episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await
                .unwrap_or_default();
            let catalog_collections = self
                .services
                .catalog
                .shows
                .list_collections_for_title(&title.id)
                .await
                .unwrap_or_default();
            let requested_episode = match &scope {
                SubmissionScope::Episode { episode_id } => catalog_episodes
                    .iter()
                    .find(|episode| episode.id == *episode_id),
                _ => None,
            };
            crate::acquisition_coverage::resolve_release_coverage(
                parsed,
                &catalog_episodes,
                &catalog_collections,
                requested_episode,
            )
            .submission_scope_or(&scope)
        } else {
            scope
        };

        self.queue_existing_title_download(
            actor,
            title_id,
            QueuedReleaseSelection {
                source_hint: best.download_url.clone().or(best.link.clone()),
                source_kind: best.source_kind,
                source_title: Some(best.title.clone()),
            },
            queue_scope,
            conflict_policy,
        )
        .await
    }

    /// Resolve the per-facet fallback category used when the selected client
    /// does not declare an explicit routing category.
    pub(crate) async fn derive_download_category(&self, facet: &MediaFacet) -> String {
        let scope_id = facet.as_str();

        if let Ok(Some(configured)) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY, Some(scope_id))
            .await
        {
            let trimmed = configured.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        if let Ok(Some(configured)) = self
            .read_setting_string_value(LEGACY_NZBGET_CATEGORY_SETTING_KEY, Some(scope_id))
            .await
        {
            let trimmed = configured.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        self.facet_registry
            .get(facet)
            .map(|h| h.download_category().to_string())
            .unwrap_or_else(|| "other".to_string())
    }

    /// Canonical owner for the "this title should be actionable right now"
    /// orchestration. Callers must route immediate acquisition seeding through
    /// this helper instead of open-coding facet splits or wake-ups.
    async fn sync_title_for_immediate_acquisition(&self, title: &Title) {
        if !title.monitored {
            return;
        }

        let now = Utc::now();
        if let Some(handler) = self.facet_registry.get(&title.facet) {
            if handler.has_episodes() {
                self.sync_wanted_series_inner(title, &now, true).await;
            } else {
                self.sync_wanted_movie_inner(title, &now, true).await;
            }
            self.runtime.acquisition.acquisition_wake.notify_one();
        }
    }

    /// Low-level title monitoring persistence and side effects. This helper
    /// intentionally does not emit domain events; canonical apply helpers do.
    async fn persist_title_monitoring(&self, title_id: &str, monitored: bool) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .update_monitored(title_id, monitored)
            .await?;

        if title.monitored {
            self.sync_title_for_immediate_acquisition(&title).await;
        } else if let Err(err) = self
            .services
            .workflow
            .wanted_items
            .delete_wanted_items_for_title(&title.id)
            .await
        {
            warn!(
                title_id = title.id.as_str(),
                error = %err,
                "failed to delete wanted items after disabling monitoring"
            );
        }

        Ok(title)
    }

    /// Canonical owner for direct title monitoring changes.
    async fn apply_title_monitoring_change(
        &self,
        actor_user_id: Option<String>,
        title_id: &str,
        monitored: bool,
    ) -> AppResult<Title> {
        let current_title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        if current_title.monitored == monitored {
            return Ok(current_title);
        }

        let title = self.persist_title_monitoring(title_id, monitored).await?;
        self.emit_title_updated_activity(actor_user_id, &title)
            .await;
        Ok(title)
    }

    /// Low-level collection monitoring persistence. This helper intentionally
    /// does not emit domain events; canonical apply helpers do.
    async fn persist_collection_monitoring(
        &self,
        collection_id: &str,
        monitored: bool,
        propagate_to_episodes: bool,
    ) -> AppResult<Collection> {
        let collection = self
            .services
            .catalog
            .shows
            .update_collection(
                collection_id,
                CollectionUpdate {
                    monitored: Some(monitored),
                    ..Default::default()
                },
            )
            .await?;

        if propagate_to_episodes {
            self.services
                .catalog
                .shows
                .set_collection_episodes_monitored(collection_id, monitored)
                .await?;
        }

        if !monitored
            && let Err(err) = self
                .services
                .workflow
                .wanted_items
                .delete_wanted_items_for_collection(collection_id)
                .await
        {
            warn!(
                collection_id,
                error = %err,
                "failed to delete wanted items after disabling collection monitoring"
            );
        }

        Ok(collection)
    }

    /// Low-level episode monitoring persistence. This helper intentionally
    /// does not emit domain events; canonical apply helpers do.
    async fn persist_episode_monitoring(
        &self,
        episode_id: &str,
        monitored: bool,
    ) -> AppResult<Episode> {
        let episode = self
            .services
            .catalog
            .shows
            .update_episode(
                episode_id,
                EpisodeUpdate {
                    monitored: Some(monitored),
                    ..Default::default()
                },
            )
            .await?;

        if !monitored
            && let Err(err) = self
                .services
                .workflow
                .wanted_items
                .delete_wanted_items_for_episode(episode_id)
                .await
        {
            warn!(
                episode_id,
                error = %err,
                "failed to delete wanted items after disabling episode monitoring"
            );
        }

        Ok(episode)
    }

    async fn apply_movie_monitor_snapshot_entries(
        &self,
        entries: &[ExternalImportMonitorMovieEntry],
        now: &DateTime<Utc>,
    ) -> AppResult<()> {
        let titles = self
            .services
            .catalog
            .titles
            .list(Some(MediaFacet::Movie), None)
            .await?;
        let mut titles_by_tmdb = HashMap::<String, Vec<Title>>::new();
        let mut titles_by_imdb = HashMap::<String, Vec<Title>>::new();

        for title in &titles {
            push_title_external_id_index(
                &mut titles_by_tmdb,
                title_external_id_value(title, "tmdb"),
                title,
            );
            push_title_external_id_index(
                &mut titles_by_imdb,
                title_external_id_value(title, "imdb"),
                title,
            );
        }

        let mut touched_title_ids = HashSet::new();
        for entry in entries {
            let matched_title = unique_title_match(&titles_by_tmdb, entry.tmdb_id.as_deref())
                .or_else(|| unique_title_match(&titles_by_imdb, entry.imdb_id.as_deref()));
            let Some(title) = matched_title else { continue };

            let updated = self
                .apply_title_monitoring_change(None, &title.id, entry.monitored)
                .await?;
            touched_title_ids.insert(updated.id);
        }

        for title_id in touched_title_ids {
            let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
                continue;
            };

            if title.monitored {
                self.sync_wanted_movie_inner(&title, now, true).await;
            } else {
                self.services
                    .workflow
                    .wanted_items
                    .delete_wanted_items_for_title(&title.id)
                    .await?;
            }
        }

        Ok(())
    }

    async fn apply_series_monitor_snapshot_entries(
        &self,
        facet: &MediaFacet,
        entries: &[ExternalImportMonitorSeriesEntry],
        now: &DateTime<Utc>,
    ) -> AppResult<()> {
        let titles = self
            .services
            .catalog
            .titles
            .list(Some(facet.clone()), None)
            .await?;
        let mut titles_by_tvdb = HashMap::<String, Vec<Title>>::new();

        for title in &titles {
            push_title_external_id_index(
                &mut titles_by_tvdb,
                title_external_id_value(title, "tvdb"),
                title,
            );
        }

        let mut touched_title_ids = HashSet::new();
        for entry in entries {
            let Some(title) = unique_title_match(&titles_by_tvdb, entry.tvdb_id.as_deref()) else {
                continue;
            };

            let updated_title = self
                .apply_title_monitoring_change(None, &title.id, entry.monitored)
                .await?;
            touched_title_ids.insert(updated_title.id.clone());

            let collections = self
                .services
                .catalog
                .shows
                .list_collections_for_title(&updated_title.id)
                .await?;
            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&updated_title.id)
                .await?;

            let mut collections_by_season = HashMap::<String, Collection>::new();
            let mut episodes_by_tvdb = HashMap::<String, Vec<Episode>>::new();
            let mut episodes_by_number = HashMap::<(String, String), Vec<Episode>>::new();

            for collection in &collections {
                collections_by_season
                    .entry(collection.collection_index.clone())
                    .or_insert_with(|| collection.clone());
            }

            for episode in &episodes {
                if let Some(tvdb_id) = episode.tvdb_id.as_deref().filter(|value| !value.is_empty())
                {
                    episodes_by_tvdb
                        .entry(tvdb_id.to_string())
                        .or_default()
                        .push(episode.clone());
                }
                if let (Some(season_number), Some(episode_number)) = (
                    episode.season_number.as_deref(),
                    episode.episode_number.as_deref(),
                ) {
                    episodes_by_number
                        .entry((season_number.to_string(), episode_number.to_string()))
                        .or_default()
                        .push(episode.clone());
                }
            }

            for collection in &collections {
                self.apply_collection_monitoring_change(None, &collection.id, false, false, false)
                    .await?;
            }
            for episode in &episodes {
                self.apply_episode_monitoring_change(None, &episode.id, false, false)
                    .await?;
            }

            if updated_title.monitored {
                for season in entry.seasons.iter().filter(|season| season.monitored) {
                    if let Some(collection) =
                        collections_by_season.get(&season.season_number.to_string())
                    {
                        self.apply_collection_monitoring_change(
                            None,
                            &collection.id,
                            true,
                            false,
                            false,
                        )
                        .await?;
                    }
                }

                for episode in entry.episodes.iter().filter(|episode| episode.monitored) {
                    if let Some(matched_episode) = unique_episode_match(
                        &episodes_by_tvdb,
                        &episodes_by_number,
                        episode.tvdb_id.as_deref(),
                        episode.season_number,
                        episode.episode_number,
                    ) {
                        self.apply_episode_monitoring_change(
                            None,
                            &matched_episode.id,
                            true,
                            false,
                        )
                        .await?;
                    }
                }
            }
        }

        for title_id in touched_title_ids {
            let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
                continue;
            };

            if title.monitored {
                self.sync_wanted_series_inner(&title, now, true).await;
            } else {
                self.services
                    .workflow
                    .wanted_items
                    .delete_wanted_items_for_title(&title.id)
                    .await?;
            }
        }

        Ok(())
    }

    pub(crate) async fn apply_pending_external_import_monitor_snapshot_for_facet(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<bool> {
        let Some(snapshot) = self.pending_external_import_monitor_snapshot(facet).await? else {
            return Ok(false);
        };

        let now = Utc::now();
        match (&snapshot.facet, &snapshot.payload) {
            (MediaFacet::Movie, ExternalImportMonitorSnapshotPayload::Movie { entries }) => {
                self.apply_movie_monitor_snapshot_entries(entries, &now)
                    .await?;
            }
            (
                MediaFacet::Series | MediaFacet::Anime,
                ExternalImportMonitorSnapshotPayload::Series { entries },
            ) => {
                self.apply_series_monitor_snapshot_entries(&snapshot.facet, entries, &now)
                    .await?;
            }
            (snapshot_facet, _) => {
                return Err(AppError::Validation(format!(
                    "monitor snapshot payload did not match facet {}",
                    snapshot_facet.as_str()
                )));
            }
        }

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot(facet)
            .await?;

        Ok(true)
    }

    /// Canonical owner for collection monitoring orchestration. Dedicated
    /// monitor mutations and generic collection updates must both delegate here
    /// so propagation and immediate acquisition behavior cannot drift.
    async fn apply_collection_monitoring_change(
        &self,
        actor_user_id: Option<String>,
        collection_id: &str,
        monitored: bool,
        propagate_to_episodes: bool,
        sync_title_if_already_monitored: bool,
    ) -> AppResult<Collection> {
        let current_collection = self
            .services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
        let collection_changed = current_collection.monitored != monitored;
        let episode_propagation_changed = if propagate_to_episodes {
            self.services
                .catalog
                .shows
                .list_episodes_for_collection(collection_id)
                .await?
                .iter()
                .any(|episode| episode.monitored != monitored)
        } else {
            false
        };
        let effective_collection_change = collection_changed || episode_propagation_changed;
        let collection = if effective_collection_change {
            self.persist_collection_monitoring(collection_id, monitored, propagate_to_episodes)
                .await?
        } else {
            current_collection
        };
        let mut title_changed = false;
        let mut final_title = None;

        if monitored {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&collection.title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", collection.title_id)))?;

            if !title.monitored {
                final_title = Some(self.persist_title_monitoring(&title.id, true).await?);
                title_changed = true;
                tracing::info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    "auto-monitored title because a collection was monitored"
                );
            } else {
                if effective_collection_change && sync_title_if_already_monitored {
                    self.sync_title_for_immediate_acquisition(&title).await;
                }
                final_title = Some(title);
            }
        }

        if (effective_collection_change || title_changed) && final_title.is_none() {
            final_title = self
                .services
                .catalog
                .titles
                .get_by_id(&collection.title_id)
                .await?;
        }

        if let Some(title) = final_title {
            self.emit_title_updated_activity(actor_user_id, &title)
                .await;
        }

        Ok(collection)
    }

    /// Canonical owner for episode monitoring orchestration. Dedicated monitor
    /// mutations and generic episode updates must both delegate here so parent
    /// propagation and immediate acquisition behavior stay single-sourced.
    async fn apply_episode_monitoring_change(
        &self,
        actor_user_id: Option<String>,
        episode_id: &str,
        monitored: bool,
        sync_title_if_already_monitored: bool,
    ) -> AppResult<Episode> {
        let current_episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
        let episode_changed = current_episode.monitored != monitored;
        let episode = if episode_changed {
            self.persist_episode_monitoring(episode_id, monitored)
                .await?
        } else {
            current_episode
        };
        let mut collection_changed = false;
        let mut title_changed = false;
        let mut final_title = None;

        if monitored {
            if let Some(collection_id) = episode.collection_id.as_deref() {
                let collection = self
                    .services
                    .catalog
                    .shows
                    .get_collection_by_id(collection_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;

                if !collection.monitored {
                    self.persist_collection_monitoring(collection_id, true, false)
                        .await?;
                    collection_changed = true;
                    tracing::info!(
                        collection_id = %collection_id,
                        "auto-monitored collection because an episode was monitored"
                    );
                }
            }

            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&episode.title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", episode.title_id)))?;

            if !title.monitored {
                final_title = Some(self.persist_title_monitoring(&title.id, true).await?);
                title_changed = true;
                tracing::info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    "auto-monitored title because an episode was monitored"
                );
            } else {
                if (episode_changed || collection_changed) && sync_title_if_already_monitored {
                    self.sync_title_for_immediate_acquisition(&title).await;
                }
                final_title = Some(title);
            }
        }

        if (episode_changed || collection_changed || title_changed) && final_title.is_none() {
            final_title = self
                .services
                .catalog
                .titles
                .get_by_id(&episode.title_id)
                .await?;
        }

        if let Some(title) = final_title {
            self.emit_title_updated_activity(actor_user_id, &title)
                .await;
        }

        Ok(episode)
    }

    pub async fn set_title_monitored(
        &self,
        actor: &User,
        id: &str,
        monitored: bool,
    ) -> AppResult<Title> {
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {id}")))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.apply_title_monitoring_change(Some(actor.id.clone()), id, monitored)
            .await
    }

    pub async fn set_collection_monitored(
        &self,
        actor: &User,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<Collection> {
        let collection = self
            .services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&collection.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", collection.title_id)))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let collection = self
            .apply_collection_monitoring_change(
                Some(actor.id.clone()),
                collection_id,
                monitored,
                true,
                true,
            )
            .await?;
        Ok(collection)
    }

    pub async fn set_episode_monitored(
        &self,
        actor: &User,
        episode_id: &str,
        monitored: bool,
    ) -> AppResult<Episode> {
        let episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&episode.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", episode.title_id)))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let episode = self
            .apply_episode_monitoring_change(Some(actor.id.clone()), episode_id, monitored, true)
            .await?;
        Ok(episode)
    }

    pub async fn delete_title(
        &self,
        actor: &User,
        id: &str,
        delete_files_on_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if delete_files_on_disk {
            let delete_confirmation = delete_confirmation.ok_or_else(|| {
                AppError::Validation(
                    "delete preview confirmation is required before deleting files on disk".into(),
                )
            })?;
            let DeleteExecutionConfirmation {
                preview_fingerprint,
                typed_confirmation,
            } = delete_confirmation;
            self.execute_delete_title_files(
                id,
                &preview_fingerprint,
                typed_confirmation.as_deref(),
            )
            .await?;
        }

        self.delete_title_logical_cleanup(
            &title,
            Some(actor.id.clone()),
            TitleLogicalDeleteOptions {
                purge_recycle_bin_entries: true,
                append_title_deleted_event: true,
            },
        )
        .await?;

        Ok(())
    }

    pub(crate) async fn delete_title_logical_cleanup(
        &self,
        title: &scryer_domain::Title,
        actor_user_id: Option<String>,
        options: TitleLogicalDeleteOptions,
    ) -> AppResult<()> {
        self.purge_title_logical_dependents(title, options.purge_recycle_bin_entries)
            .await?;
        self.delete_title_row(title, actor_user_id, options.append_title_deleted_event)
            .await
    }

    pub(crate) async fn purge_title_logical_dependents(
        &self,
        title: &scryer_domain::Title,
        purge_recycle_bin_entries: bool,
    ) -> AppResult<()> {
        let title_id = title.id.as_str();

        if purge_recycle_bin_entries
            && let Some(media_root) = crate::recycle_bin::media_root_for_title(self, title).await
        {
            let config = crate::recycle_bin::resolve_recycle_config(self, Some(&media_root)).await;
            match crate::recycle_bin::purge_for_title(&config, title_id).await {
                Ok(n) if n > 0 => info!(
                    purged = n,
                    title_id = %title_id,
                    "purged recycle bin entries for deleted title"
                ),
                Err(e) => warn!(
                    error = %e,
                    title_id = %title_id,
                    "failed to purge recycle entries for deleted title"
                ),
                _ => {}
            }
        }

        let queued_submission_keys = match self
            .services
            .workflow
            .download_submissions
            .list_for_title(title_id)
            .await
        {
            Ok(submissions) => submissions
                .into_iter()
                .map(|submission| {
                    (
                        submission.download_client_type,
                        submission.download_client_item_id,
                    )
                })
                .collect::<HashSet<_>>(),
            Err(err) => {
                warn!(
                    title_id = %title_id,
                    error = %err,
                    "failed to list download submissions while deleting title; falling back to embedded queue metadata only"
                );
                HashSet::new()
            }
        };

        match self
            .services
            .integrations
            .download_client
            .list_queue()
            .await
        {
            Ok(queue_items) => {
                for item in queue_items {
                    let matches_title = item.title_id.as_deref() == Some(title_id)
                        || queued_submission_keys.contains(&(
                            item.client_type.clone(),
                            item.download_client_item_id.clone(),
                        ));
                    if matches_title
                        && let Err(err) = self
                            .services
                            .integrations
                            .download_client
                            .delete_queue_item_for_client(
                                &item.client_type,
                                &item.download_client_item_id,
                                false,
                            )
                            .await
                    {
                        warn!(
                            title_id = %title_id,
                            download_item_id = %item.download_client_item_id,
                            error = %err,
                            "failed to cancel inflight download while deleting title"
                        );
                    }
                }
            }
            Err(err) => {
                warn!(
                    title_id = %title_id,
                    error = %err,
                    "failed to list download queue while deleting title; skipping download cancellation"
                );
            }
        }

        self.services
            .workflow
            .pending_releases
            .delete_pending_releases_for_title(title_id)
            .await?;
        self.services
            .workflow
            .wanted_items
            .delete_wanted_items_for_title(title_id)
            .await?;
        self.services
            .workflow
            .download_submissions
            .delete_for_title(title_id)
            .await?;
        self.services
            .workflow
            .blocklist_repo
            .delete_for_title(title_id)
            .await?;
        self.services
            .library
            .library_probe_signatures
            .delete_probe_signatures_for_title_ids(std::slice::from_ref(&title.id))
            .await?;

        Ok(())
    }

    pub(crate) async fn delete_title_row(
        &self,
        title: &scryer_domain::Title,
        actor_user_id: Option<String>,
        append_title_deleted_event: bool,
    ) -> AppResult<()> {
        let title_id = title.id.as_str();

        self.services.catalog.titles.delete(title_id).await?;

        if append_title_deleted_event {
            let _ = self
                .append_domain_event(new_title_domain_event(
                    actor_user_id,
                    title,
                    DomainEventPayload::TitleDeleted(TitleDeletedEventData {
                        title: title_context_snapshot(title),
                    }),
                ))
                .await;
        }

        Ok(())
    }

    pub async fn delete_media_file(
        &self,
        actor: &User,
        file_id: &str,
        delete_from_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", media_file.title_id)))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let (matching_movie_collection_ids, matching_interstitial_collection_ids) = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&media_file.title_id)
            .await?
            .into_iter()
            .filter(|collection| {
                collection.ordered_path.as_deref() == Some(media_file.file_path.as_str())
            })
            .fold((Vec::new(), Vec::new()), |mut acc, collection| {
                match collection.collection_type {
                    scryer_domain::CollectionType::Movie => acc.0.push(collection.id),
                    scryer_domain::CollectionType::Interstitial => acc.1.push(collection.id),
                    _ => {}
                }
                acc
            });

        if delete_from_disk {
            let delete_confirmation = delete_confirmation.ok_or_else(|| {
                AppError::Validation(
                    "delete preview confirmation is required before deleting files on disk".into(),
                )
            })?;
            let DeleteExecutionConfirmation {
                preview_fingerprint,
                typed_confirmation,
            } = delete_confirmation;
            self.execute_delete_media_file(
                file_id,
                &preview_fingerprint,
                typed_confirmation.as_deref(),
            )
            .await?;
        }

        self.services
            .library
            .media_files
            .delete_media_file(file_id)
            .await?;
        for collection_id in matching_movie_collection_ids {
            if let Err(error) = self
                .services
                .catalog
                .shows
                .delete_collection(&collection_id)
                .await
            {
                tracing::warn!(
                    error = %error,
                    file_id = %file_id,
                    collection_id = %collection_id,
                    file_path = %media_file.file_path,
                    "failed to delete matching movie collection after media file delete"
                );
            }
        }
        for collection_id in matching_interstitial_collection_ids {
            if let Err(error) = self
                .services
                .catalog
                .shows
                .update_collection(
                    &collection_id,
                    CollectionUpdate {
                        clear_ordered_path: true,
                        ..Default::default()
                    },
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    file_id = %file_id,
                    collection_id = %collection_id,
                    file_path = %media_file.file_path,
                    "failed to clear matching interstitial collection ordered_path after media file delete"
                );
            }
        }

        info!(
            file_id = %file_id,
            file_path = %media_file.file_path,
            delete_from_disk = %delete_from_disk,
            "media file deleted"
        );

        if delete_from_disk
            && let Ok(Some(title)) = self
                .services
                .catalog
                .titles
                .get_by_id(&media_file.title_id)
                .await
        {
            let _ = self
                .append_domain_event(new_title_domain_event(
                    Some(actor.id.clone()),
                    &title,
                    DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                        title: title_context_snapshot(&title),
                        media_updates: vec![deleted_media_update(media_file.file_path.clone())],
                        file_id: Some(media_file.id.clone()),
                        reason: MediaFileDeletedReason::Deleted,
                        episode_ids: media_file.episode_id.iter().cloned().collect(),
                    }),
                ))
                .await;
        }

        Ok(())
    }

    pub(crate) async fn apply_title_metadata_update(
        &self,
        actor_user_id: Option<String>,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .update_metadata(id, name, facet, tags)
            .await?;
        self.emit_title_updated_activity(actor_user_id, &title)
            .await;
        Ok(title)
    }

    pub async fn update_title_metadata(
        &self,
        actor: &User,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        if name.is_none() && facet.is_none() && tags.is_none() {
            return Err(AppError::Validation(
                "at least one title field must be provided".into(),
            ));
        }
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {id}")))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.apply_title_metadata_update(Some(actor.id.clone()), id, name, facet, tags)
            .await
    }

    pub async fn fix_title_match(
        &self,
        actor: &User,
        title_id: &str,
        target_tvdb_id: &str,
    ) -> AppResult<FixTitleMatchResult> {
        let target_tvdb_id = target_tvdb_id.trim();
        if target_tvdb_id.is_empty() {
            return Err(AppError::Validation("tvdb id is required".into()));
        }
        let target_tvdb_numeric = target_tvdb_id
            .parse::<i64>()
            .map_err(|_| AppError::Validation("tvdb id must be numeric".into()))?;

        let existing_title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &existing_title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let duplicate = self
            .services
            .catalog
            .titles
            .find_by_external_id_in_facet(existing_title.facet.clone(), "tvdb", target_tvdb_id)
            .await?
            .filter(|title| title.id != existing_title.id);
        if let Some(duplicate) = duplicate {
            return Err(AppError::Validation(format!(
                "tvdb id {target_tvdb_id} is already assigned to title {}",
                duplicate.name
            )));
        }

        let handler = self
            .facet_registry
            .get(&existing_title.facet)
            .ok_or_else(|| AppError::Validation("unsupported title facet".into()))?;
        let has_episodes = handler.has_episodes();

        if has_episodes {
            self.services
                .workflow
                .pending_releases
                .delete_pending_releases_for_title(&existing_title.id)
                .await?;
            self.services
                .workflow
                .wanted_items
                .delete_wanted_items_for_title(&existing_title.id)
                .await?;

            self.services
                .catalog
                .shows
                .delete_episodes_for_title(&existing_title.id)
                .await?;
            self.services
                .catalog
                .shows
                .delete_collections_for_title(&existing_title.id)
                .await?;
        }

        let replacement_external_ids = build_rematched_external_ids(
            &existing_title,
            target_tvdb_id,
            None,
            REMATCH_REPLACED_EXTERNAL_ID_SOURCES,
        );
        let replacement_tags =
            strip_derived_match_tags(&existing_title.tags, REMATCH_DERIVED_TAG_PREFIXES);

        let mut reset_title = self
            .services
            .catalog
            .titles
            .replace_match_state(
                &existing_title.id,
                replacement_external_ids,
                replacement_tags,
            )
            .await?;

        if has_episodes
            && reset_title
                .folder_path
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            let mut legacy_folder_path = existing_title
                .folder_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);

            if legacy_folder_path.is_none() {
                let old_title_name = existing_title.name.trim();
                if !old_title_name.is_empty()
                    && let Ok((media_root, _)) =
                        crate::import_workflow::resolve_import_paths(self, &existing_title).await
                {
                    legacy_folder_path = Some(
                        std::path::PathBuf::from(media_root)
                            .join(old_title_name)
                            .to_string_lossy()
                            .to_string(),
                    );
                }
            }

            if let Some(legacy_folder_path) = legacy_folder_path
                && tokio::fs::metadata(&legacy_folder_path)
                    .await
                    .ok()
                    .is_some_and(|metadata| metadata.is_dir())
            {
                match self
                    .services
                    .catalog
                    .titles
                    .set_folder_path(&existing_title.id, &legacy_folder_path)
                    .await
                {
                    Ok(()) => {
                        reset_title.folder_path = Some(legacy_folder_path);
                    }
                    Err(error) => warn!(
                        error = %error,
                        title_id = %existing_title.id,
                        "failed to persist legacy folder path before title rematch hydration"
                    ),
                }
            }
        }

        let mut hydration_outcome = self
            .hydrate_titles_bulk(vec![HydrationTarget {
                title: reset_title.clone(),
                requested_tvdb_id: Some(target_tvdb_numeric),
                sync_wanted_after_completion: false,
                source: HydrationSource::Interactive,
            }])
            .await?;
        let hydrated_title = hydration_outcome
            .hydrated_titles
            .remove(&reset_title.id)
            .unwrap_or(reset_title);
        let mut warnings = Vec::new();
        if hydrated_title.metadata_fetched_at.is_none() {
            warnings.push(
                hydration_outcome
                    .failed_titles
                    .remove(&existing_title.id)
                    .unwrap_or_else(|| {
                        "Matched title metadata could not be fully refreshed.".to_string()
                    }),
            );
        }

        let mut library_scan = None;
        if has_episodes {
            match self.scan_title_library(actor, &existing_title.id).await {
                Ok(summary) => library_scan = Some(summary),
                Err(err) => warnings.push(format!("Library relink failed: {err}")),
            }
        }

        if hydrated_title.monitored {
            self.sync_title_for_immediate_acquisition(&hydrated_title)
                .await;
        }

        let refreshed_title = self
            .services
            .catalog
            .titles
            .get_by_id(&existing_title.id)
            .await?
            .unwrap_or(hydrated_title);

        let old_tvdb_id = extract_tvdb_id(&existing_title).map(|id| id.to_string());
        self.append_domain_event(new_title_domain_event(
            Some(actor.id.clone()),
            &refreshed_title,
            DomainEventPayload::TitleRematched(TitleRematchedEventData {
                title: title_context_snapshot(&refreshed_title),
                old_tvdb_id,
                new_tvdb_id: target_tvdb_id.to_string(),
                source: "manual".to_string(),
            }),
        ))
        .await?;
        self.emit_title_updated_activity(Some(actor.id.clone()), &refreshed_title)
            .await;

        Ok(FixTitleMatchResult {
            hydrated: refreshed_title.metadata_fetched_at.is_some(),
            title: refreshed_title,
            library_scan,
            warnings,
        })
    }

    pub async fn get_title(&self, actor: &User, id: &str) -> AppResult<Option<Title>> {
        let title = self.services.catalog.titles.get_by_id(id).await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(title)
    }

    pub async fn get_title_without_external_ids(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<Title>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id_without_external_ids(id)
            .await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(title)
    }

    pub async fn get_title_by_slug(
        &self,
        actor: &User,
        facet: MediaFacet,
        library_id: Option<String>,
        library_slug: Option<String>,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let mut authorized_libraries = self
            .list_libraries_for_permission(
                actor,
                Some(facet.clone()),
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        if let Some(requested_library_id) = library_id {
            authorized_libraries.retain(|library| library.id == requested_library_id);
        }
        if let Some(requested_library_slug) = library_slug {
            let normalized_slug = requested_library_slug.trim();
            authorized_libraries
                .retain(|library| library.slug.eq_ignore_ascii_case(normalized_slug));
        }
        let library_ids = authorized_libraries
            .into_iter()
            .map(|library| library.id)
            .collect::<Vec<_>>();
        let title = self
            .services
            .catalog
            .titles
            .get_by_facet_libraries_and_slug(facet, &library_ids, slug)
            .await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(title)
    }

    async fn require_title_permission(
        &self,
        actor: &User,
        title_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(actor, &title.library_id, permission)
            .await?;
        Ok(title)
    }

    async fn filter_title_ids_for_permission(
        &self,
        actor: &User,
        title_ids: &[String],
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<String>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut visible = Vec::with_capacity(title_ids.len());
        for title_id in title_ids {
            if let Some(title) = self.services.catalog.titles.get_by_id(title_id).await?
                && allowed_library_ids.contains(&title.library_id)
            {
                visible.push(title.id);
            }
        }
        Ok(visible)
    }

    async fn require_collection_permission(
        &self,
        actor: &User,
        collection_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Collection> {
        let collection = self
            .services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("collection {collection_id}")))?;
        self.require_title_permission(actor, &collection.title_id, permission)
            .await?;
        Ok(collection)
    }

    async fn require_episode_permission(
        &self,
        actor: &User,
        episode_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Episode> {
        let episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("episode {episode_id}")))?;
        self.require_title_permission(actor, &episode.title_id, permission)
            .await?;
        Ok(episode)
    }

    pub async fn list_primary_collection_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .catalog
            .shows
            .list_primary_collection_summaries(&title_ids)
            .await
    }

    pub async fn list_title_media_size_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_title_media_size_summaries(&title_ids)
            .await
    }

    pub async fn list_title_quality_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_title_quality_summaries(&title_ids)
            .await
    }

    pub async fn list_title_episode_progress_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_title_episode_progress_summaries(&title_ids)
            .await
    }

    pub async fn list_collections(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<Collection>> {
        self.require_title_permission(actor, title_id, scryer_domain::LibraryPermission::View)
            .await?;
        self.services
            .catalog
            .shows
            .list_collections_for_title(title_id)
            .await
    }

    pub async fn get_collection(
        &self,
        actor: &User,
        collection_id: &str,
    ) -> AppResult<Option<Collection>> {
        let collection = self
            .services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await?;
        if let Some(collection) = collection.as_ref() {
            self.require_title_permission(
                actor,
                &collection.title_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(collection)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "collection creation mirrors the editable collection fields at the application boundary"
    )]
    pub async fn create_collection(
        &self,
        actor: &User,
        title_id: String,
        collection_type: String,
        collection_index: String,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
    ) -> AppResult<Collection> {
        self.require_title_permission(
            actor,
            &title_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if collection_type.trim().is_empty() {
            return Err(AppError::Validation("collection type is required".into()));
        }
        let parsed_type = CollectionType::parse(collection_type.trim().to_lowercase().as_str())
            .ok_or_else(|| {
                AppError::Validation(format!("unknown collection type: {}", collection_type))
            })?;
        if collection_index.trim().is_empty() {
            return Err(AppError::Validation("collection index is required".into()));
        }
        let collection = Collection {
            id: Id::new().0,
            title_id,
            collection_type: parsed_type,
            collection_index: collection_index.trim().to_string(),
            label: normalize_show_text_opt(label),
            ordered_path: normalize_show_text_opt(ordered_path),
            narrative_order: None,
            first_episode_number: normalize_show_text_opt(first_episode_number),
            last_episode_number: normalize_show_text_opt(last_episode_number),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        };

        let collection = self
            .services
            .catalog
            .shows
            .create_collection(collection)
            .await?;
        Ok(collection)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "collection updates keep each mutable field explicit for callers and validation"
    )]
    pub async fn update_collection(
        &self,
        actor: &User,
        collection_id: String,
        collection_type: Option<String>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
    ) -> AppResult<Collection> {
        self.require_collection_permission(
            actor,
            &collection_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if let Some(raw) = &collection_type
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation(
                "collection type cannot be empty".into(),
            ));
        }
        let parsed_type = collection_type
            .map(|raw| {
                CollectionType::parse(raw.trim().to_lowercase().as_str()).ok_or_else(|| {
                    AppError::Validation(format!("unknown collection type: {}", raw))
                })
            })
            .transpose()?;

        if let Some(raw) = &collection_index
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation(
                "collection index cannot be empty".into(),
            ));
        }

        let update = CollectionUpdate {
            collection_type: parsed_type,
            collection_index: collection_index.map(|value| value.trim().to_string()),
            label: normalize_show_text_opt(label),
            ordered_path: normalize_show_text_opt(ordered_path),
            clear_ordered_path: false,
            first_episode_number: normalize_show_text_opt(first_episode_number),
            last_episode_number: normalize_show_text_opt(last_episode_number),
            monitored,
        };
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one collection field must be provided".into(),
            ));
        }

        let has_non_monitor_updates = update.has_non_monitor_changes();
        let monitored = update.monitored;

        let mut collection = if has_non_monitor_updates {
            let mut repo_update = update.clone();
            repo_update.monitored = None;
            Some(
                self.services
                    .catalog
                    .shows
                    .update_collection(&collection_id, repo_update)
                    .await?,
            )
        } else {
            None
        };

        if let Some(monitored) = monitored {
            collection = Some(
                self.apply_collection_monitoring_change(
                    Some(actor.id.clone()),
                    &collection_id,
                    monitored,
                    true,
                    true,
                )
                .await?,
            );
        }

        let collection = collection.ok_or_else(|| {
            AppError::Validation("at least one collection field must be provided".into())
        })?;

        Ok(collection)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "episode creation mirrors the full editable episode form at the application boundary"
    )]
    pub async fn create_episode(
        &self,
        actor: &User,
        title_id: String,
        collection_id: Option<String>,
        episode_type: String,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: bool,
        has_subtitle: bool,
    ) -> AppResult<Episode> {
        self.require_title_permission(
            actor,
            &title_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if episode_type.trim().is_empty() {
            return Err(AppError::Validation("episode type is required".into()));
        }

        let parsed_episode_type =
            scryer_domain::EpisodeType::parse(episode_type.trim().to_lowercase().as_str())
                .ok_or_else(|| {
                    AppError::Validation(format!("unknown episode type: {}", episode_type))
                })?;
        let episode = Episode {
            id: Id::new().0,
            title_id,
            collection_id,
            episode_type: parsed_episode_type,
            episode_number: normalize_show_text_opt(episode_number),
            season_number: normalize_show_text_opt(season_number),
            episode_label: normalize_show_text_opt(episode_label),
            title: normalize_show_text_opt(title),
            air_date: normalize_show_text_opt(air_date),
            duration_seconds,
            has_multi_audio,
            has_subtitle,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        };

        let episode = self.services.catalog.shows.create_episode(episode).await?;
        Ok(episode)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "episode updates keep each mutable field explicit for validation and auditing"
    )]
    pub async fn update_episode(
        &self,
        actor: &User,
        episode_id: String,
        episode_type: Option<String>,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: Option<bool>,
        has_subtitle: Option<bool>,
        monitored: Option<bool>,
        collection_id: Option<String>,
        overview: Option<String>,
    ) -> AppResult<Episode> {
        self.require_episode_permission(
            actor,
            &episode_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if let Some(raw) = &episode_type
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation("episode type cannot be empty".into()));
        }

        let parsed_episode_type = episode_type
            .map(|value| {
                scryer_domain::EpisodeType::parse(value.trim().to_lowercase().as_str())
                    .ok_or_else(|| AppError::Validation(format!("unknown episode type: {}", value)))
            })
            .transpose()?;

        let update = EpisodeUpdate {
            episode_type: parsed_episode_type,
            episode_number: normalize_show_text_opt(episode_number),
            season_number: normalize_show_text_opt(season_number),
            episode_label: normalize_show_text_opt(episode_label),
            title: normalize_show_text_opt(title),
            air_date: normalize_show_text_opt(air_date),
            duration_seconds,
            has_multi_audio,
            has_subtitle,
            monitored,
            collection_id,
            overview,
            tvdb_id: None,
        };
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one episode field must be provided".into(),
            ));
        }

        let has_non_monitor_updates = update.has_non_monitor_changes();
        let monitored = update.monitored;

        let mut episode = if has_non_monitor_updates {
            let mut repo_update = update.clone();
            repo_update.monitored = None;
            Some(
                self.services
                    .catalog
                    .shows
                    .update_episode(&episode_id, repo_update)
                    .await?,
            )
        } else {
            None
        };

        if let Some(monitored) = monitored {
            episode = Some(
                self.apply_episode_monitoring_change(
                    Some(actor.id.clone()),
                    &episode_id,
                    monitored,
                    true,
                )
                .await?,
            );
        }

        let episode = episode.ok_or_else(|| {
            AppError::Validation("at least one episode field must be provided".into())
        })?;

        Ok(episode)
    }

    pub async fn delete_collection(&self, actor: &User, collection_id: &str) -> AppResult<()> {
        self.require_collection_permission(
            actor,
            collection_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .catalog
            .shows
            .delete_collection(collection_id)
            .await?;
        Ok(())
    }

    pub async fn delete_episode(&self, actor: &User, episode_id: &str) -> AppResult<()> {
        self.require_episode_permission(
            actor,
            episode_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .catalog
            .shows
            .delete_episode(episode_id)
            .await?;
        Ok(())
    }

    pub async fn list_episodes(
        &self,
        actor: &User,
        collection_id: &str,
    ) -> AppResult<Vec<Episode>> {
        self.require_collection_permission(
            actor,
            collection_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .catalog
            .shows
            .list_episodes_for_collection(collection_id)
            .await
    }

    pub async fn get_episode(&self, actor: &User, episode_id: &str) -> AppResult<Option<Episode>> {
        let episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?;
        if let Some(episode) = episode.as_ref() {
            self.require_title_permission(
                actor,
                &episode.title_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(episode)
    }

    pub async fn list_calendar_episodes(
        &self,
        actor: &User,
        start_date: &str,
        end_date: &str,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<Vec<CalendarEpisode>> {
        let authorized = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let requested_library_ids = library_ids
            .unwrap_or_default()
            .into_iter()
            .map(|library_id| library_id.trim().to_string())
            .filter(|library_id| !library_id.is_empty())
            .collect::<HashSet<_>>();
        let visible_library_ids = if requested_library_ids.is_empty() {
            authorized
        } else {
            authorized
                .intersection(&requested_library_ids)
                .cloned()
                .collect::<HashSet<_>>()
        };
        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_in_date_range(start_date, end_date)
            .await?;
        Ok(episodes
            .into_iter()
            .filter(|episode| visible_library_ids.contains(&episode.library_id))
            .collect())
    }

    /// Re-fetch metadata from SMG for all monitored series/anime titles.
    /// This updates episode air dates (TBA → actual), adds newly announced
    /// episodes, and refreshes other metadata fields.
    pub(crate) async fn run_metadata_refresh_job(&self) -> AppResult<u32> {
        let titles = match self.services.catalog.titles.list(None, None).await {
            Ok(t) => t,
            Err(err) => {
                warn!(error = %err, "metadata refresh: failed to list titles");
                return Err(err);
            }
        };

        let targets = titles
            .into_iter()
            .filter(|title| title.monitored)
            .filter(|title| {
                self.facet_registry
                    .get(&title.facet)
                    .is_some_and(|handler| handler.has_episodes())
            })
            .map(|title| HydrationTarget {
                title,
                requested_tvdb_id: None,
                sync_wanted_after_completion: false,
                source: HydrationSource::Maintenance,
            })
            .collect::<Vec<_>>();

        let refreshed = targets.len() as u32;
        let _ = self.hydrate_titles_bulk(targets).await?;

        if refreshed > 0 {
            info!(count = refreshed, "periodic metadata refresh completed");
        }

        Ok(refreshed)
    }

    pub async fn hydrate_all_titles_for_current_language(&self) -> AppResult<u32> {
        let titles = self.services.catalog.titles.list(None, None).await?;
        let refreshed = titles.len() as u32;
        let targets = titles
            .into_iter()
            .map(|title| HydrationTarget {
                title,
                requested_tvdb_id: None,
                sync_wanted_after_completion: false,
                source: HydrationSource::Maintenance,
            })
            .collect::<Vec<_>>();
        let _ = self.hydrate_titles_bulk(targets).await?;
        Ok(refreshed)
    }

    pub async fn rehydrate_all_metadata(&self, actor: &User, language: &str) -> AppResult<u64> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let language = language.trim().to_ascii_lowercase();
        if language.is_empty() {
            return Err(AppError::Validation("language is required".to_string()));
        }

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                METADATA_LANGUAGE_KEY,
                None,
                serde_json::to_string(&language)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                "rehydrate_metadata",
                Some(actor.id.clone()),
            )
            .await?;

        let cleared = self
            .services
            .catalog
            .titles
            .clear_metadata_language_for_all()
            .await?;
        let app = self.clone();
        tokio::spawn(async move {
            match app.hydrate_all_titles_for_current_language().await {
                Ok(refreshed) => {
                    info!(
                        language = %language,
                        titles_cleared = cleared,
                        titles_refreshed = refreshed,
                        "metadata rehydration completed"
                    );
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        language = %language,
                        titles_cleared = cleared,
                        "metadata rehydration failed"
                    );
                }
            }
        });

        Ok(cleared)
    }
}

fn normalize_release_attempt_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Extract the monitor type from title tags (e.g. "scryer:monitor-type:none").
/// Defaults to "allEpisodes" when no tag is present for backward compatibility.
fn extract_monitor_type(tags: &[String]) -> String {
    // Tags are lowercased by normalize_tag(), so values like "futureEpisodes"
    // become "futureepisodes". We return the lowercased value.
    for tag in tags {
        if let Some(value) = tag.strip_prefix("scryer:monitor-type:") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "allepisodes".to_string()
}

/// Extract a boolean from a `scryer:{prefix}:true/false` tag.
/// Returns `None` when no matching tag exists (caller falls back to global setting).
fn extract_tag_bool(tags: &[String], prefix: &str) -> Option<bool> {
    for tag in tags {
        if let Some(value) = tag.strip_prefix(prefix) {
            return Some(!value.trim().eq_ignore_ascii_case("false"));
        }
    }
    None
}

/// Extract a string value from a `scryer:{prefix}:{value}` tag.
/// Returns `None` when no matching tag exists (caller falls back to global setting).
fn extract_tag_string<'a>(tags: &'a [String], prefix: &str) -> Option<&'a str> {
    for tag in tags {
        if let Some(value) = tag.strip_prefix(prefix) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

/// Determine whether an individual episode should be monitored based on
/// the user's monitor type selection and the episode's air date.
///
/// NOTE: All values are lowercase because tags go through `normalize_tag`
/// which calls `.to_lowercase()`. The frontend sends camelCase values like
/// "futureEpisodes" which become "futureepisodes" after normalization.
fn should_monitor_season(monitor_type: &str, season_number: i32, monitor_specials: bool) -> bool {
    if season_number == 0 {
        return monitor_specials;
    }

    monitor_type != "none" && monitor_type != "unmonitored"
}

fn should_monitor_episode(
    monitor_type: &str,
    season_number: i32,
    air_date: Option<&str>,
    today: &str,
    monitor_specials: bool,
) -> bool {
    if season_number == 0 {
        return monitor_specials;
    }

    match monitor_type {
        "none" | "unmonitored" => false,
        "allepisodes" | "monitored" => true,
        "futureepisodes" => {
            // Monitor only episodes that haven't aired yet
            match air_date {
                Some(date) if !date.is_empty() => date >= today,
                _ => true, // no air date = assume future
            }
        }
        "missingandfutureepisodes" => {
            // Monitor episodes that haven't aired or are missing (not on disk).
            // At add time, no episodes are on disk yet, so all are "missing" — monitor all.
            true
        }
        _ => true,
    }
}

/// Derive the episode type from the season number, season episode_type, and anime media type.
fn derive_episode_type(
    season_number: i32,
    season_episode_type: Option<&str>,
    anime_media_type: Option<&str>,
) -> scryer_domain::EpisodeType {
    use scryer_domain::EpisodeType;
    if season_number == 0 {
        return match anime_media_type {
            Some("OVA") => EpisodeType::Ova,
            Some("ONA") => EpisodeType::Ona,
            _ => EpisodeType::Special,
        };
    }
    match season_episode_type {
        Some("alternate") => EpisodeType::Alternate,
        _ => EpisodeType::Standard,
    }
}

pub(crate) fn extract_tvdb_id(title: &scryer_domain::Title) -> Option<i64> {
    title
        .external_ids
        .iter()
        .find(|eid| eid.source == "tvdb")
        .and_then(|eid| eid.value.parse::<i64>().ok())
}

/// After successful hydration, sync wanted items for monitored titles.
async fn sync_wanted_after_hydration(app: &AppUseCase, title: &scryer_domain::Title) {
    if title.monitored && title.metadata_fetched_at.is_some() {
        app.sync_title_for_immediate_acquisition(title).await;
    }
}
