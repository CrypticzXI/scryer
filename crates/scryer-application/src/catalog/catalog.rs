use super::*;
use crate::catalog_helpers::{
    DownloadClientRoutingEntry, anime_mapping_identity_keys, anime_movie_after_season,
    anime_movie_identity_keys, anime_movie_release_sort_key, build_rematched_external_ids,
    interstitial_movie_from_anime_movie, is_logical_specials_collection,
    parse_download_client_routing_entry, parse_download_client_routing_map,
    release_is_recent_for_queue_priority, strip_derived_match_tags,
};
use crate::domain_events::{deleted_media_update, new_title_domain_event, title_context_snapshot};
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

    async fn read_download_client_routing_entry(
        &self,
        facet: &MediaFacet,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientRoutingEntry>> {
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
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        self.read_download_client_routing_entry(facet, client_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|entry| entry.remove_completed)
    }

    pub(crate) async fn should_remove_failed_download(
        &self,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        self.read_download_client_routing_entry(facet, client_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|entry| entry.remove_failed)
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
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services.catalog.titles.list(facet, query).await
    }

    pub async fn list_titles_by_external_ids(
        &self,
        actor: &User,
        source: &str,
        values: &[String],
    ) -> AppResult<Vec<Title>> {
        require(actor, &Entitlement::ViewCatalog)?;

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

        self.services
            .catalog
            .titles
            .list_by_external_ids(normalized_source, &normalized_values)
            .await
    }

    pub async fn list_title_release_blocklist(
        &self,
        actor: &User,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        require(actor, &Entitlement::ViewCatalog)?;
        let bounded_limit = limit.clamp(1, 1_000);
        self.services
            .workflow
            .release_attempts
            .list_failed_release_signatures_for_title(title_id, bounded_limit)
            .await
    }

    /// Return the configured root folders for a facet.
    ///
    /// Reads the `<facet>.root_folders` JSON setting.  When absent or empty,
    /// falls back to the single `<facet>.path` setting and returns it as the
    /// sole default entry.
    pub async fn root_folders_for_facet(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<scryer_domain::RootFolderEntry>> {
        let handler = self.facet_registry.get(facet);
        let root_folders_key = handler.map(|h| h.root_folders_key());
        let library_path_key = handler.map(|h| h.library_path_key());
        let default_path = handler.map(|h| h.default_library_path()).unwrap_or("/data");

        // Try the root_folders JSON array first.
        if let Some(key) = root_folders_key
            && let Some(raw) = self
                .read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, key, None)
                .await?
        {
            let trimmed = raw.trim();
            if !trimmed.is_empty()
                && trimmed != "[]"
                && let Ok(entries) =
                    serde_json::from_str::<Vec<scryer_domain::RootFolderEntry>>(trimmed)
                && !entries.is_empty()
            {
                return Ok(entries);
            }
        }

        // Fall back to the single path setting.
        let path = if let Some(key) = library_path_key {
            self.read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, key, None)
                .await?
                .unwrap_or_else(|| default_path.to_string())
        } else {
            default_path.to_string()
        };

        Ok(vec![scryer_domain::RootFolderEntry {
            path,
            is_default: true,
        }])
    }

    pub async fn add_title_with_outcome(
        &self,
        actor: &User,
        request: NewTitle,
    ) -> AppResult<AddTitleOutcome> {
        require(actor, &Entitlement::ManageTitle)?;

        let created = self.create_title_without_hydration(actor, request).await?;
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

    pub(crate) async fn create_title_without_hydration(
        &self,
        actor: &User,
        request: NewTitle,
    ) -> AppResult<CreateTitleOutcome> {
        require(actor, &Entitlement::ManageTitle)?;

        if request.name.trim().is_empty() {
            return Err(AppError::Validation("title name is required".into()));
        }

        let title = Title {
            id: Id::new().0,
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

        // Build extra external IDs from the primary anime mapping only.
        // Prefer non-special (R/regular) mappings over specials (S) to avoid
        // OVA metadata clobbering the main series (e.g. Bleach anilist 834 vs 269).
        let mut metadata_update = result.metadata_update;
        if let Some(mapping) = result
            .anime_mappings
            .iter()
            .find(|m| m.mapping_type != "S")
            .or(result.anime_mappings.first())
        {
            if let Some(mal_id) = mapping.mal_id {
                metadata_update.extra_external_ids.push(ExternalId {
                    source: "mal".to_string(),
                    value: mal_id.to_string(),
                });
            }
            if let Some(anilist_id) = mapping.anilist_id {
                metadata_update.extra_external_ids.push(ExternalId {
                    source: "anilist".to_string(),
                    value: anilist_id.to_string(),
                });
            }
            if let Some(anidb_id) = mapping.anidb_id {
                metadata_update.extra_external_ids.push(ExternalId {
                    source: "anidb".to_string(),
                    value: anidb_id.to_string(),
                });
            }
            if let Some(kitsu_id) = mapping.kitsu_id {
                metadata_update.extra_external_ids.push(ExternalId {
                    source: "kitsu".to_string(),
                    value: kitsu_id.to_string(),
                });
            }
        }

        // Store anime-specific metadata as tags on the title
        if let Some(primary) = result
            .anime_mappings
            .iter()
            .find(|m| m.mapping_type != "S")
            .or(result.anime_mappings.first())
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
                self.read_setting_string_value("anime.monitor_specials", Some("anime"))
                    .await
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some("true") // Default: false
            }
        } else {
            false
        };

        let inter_season_movies = if title.facet == MediaFacet::Anime {
            if let Some(per_title) = extract_tag_bool(&title.tags, "scryer:inter-season-movies:") {
                per_title
            } else {
                self.read_setting_string_value("anime.inter_season_movies", Some("anime"))
                    .await
                    .ok()
                    .flatten()
                    .as_deref()
                    != Some("false") // Default: true
            }
        } else {
            false
        };

        // Seasons that have no episodes should not be auto-monitored.
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
            let season_monitored = seasons_with_episodes.contains(&season.number)
                && should_monitor_season(&monitor_type, season.number, monitor_specials);
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
                    .read_setting_string_value("anime.filler_policy", Some("anime"))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            };
            effective == "skip_filler"
        } else {
            false
        };
        let skip_recap = if title.facet == MediaFacet::Anime {
            let effective = match extract_tag_string(&title.tags, "scryer:recap-policy:") {
                Some(v) => v.to_string(),
                None => self
                    .read_setting_string_value("anime.recap_policy", Some("anime"))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
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
    }

    pub async fn add_title_and_queue_download_with_outcome(
        &self,
        actor: &User,
        request: NewTitle,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<AddTitleAndQueueDownloadOutcome> {
        let QueuedReleaseSelection {
            source_hint,
            source_kind,
            source_title,
        } = queued_release;
        let add_outcome = self.add_title_with_outcome(actor, request).await?;
        let title = add_outcome.title.clone();
        let source_hint_for_attempt = normalize_release_attempt_value(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_value(source_title.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint_for_attempt.as_deref(),
            source_title_for_attempt.as_deref(),
            source_kind,
        );
        let source_password: Option<String> = None;
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
            drop(dedupe_guard);
            return Ok(AddTitleAndQueueDownloadOutcome {
                title,
                metadata_hydration_state: add_outcome.metadata_hydration_state,
                reused_existing_title: add_outcome.reused_existing_title,
                download_job_id: existing.download_client_item_id,
                reused_queued_download: true,
            });
        }

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
                        download_client_type: grab.client_type.clone(),
                        download_client_item_id: grab.job_id.clone(),
                        source_hint: source_hint_for_attempt.clone(),
                        source_kind,
                        source_title: source_title_for_attempt.clone(),
                        request_signature: request_signature.clone(),
                        scope: SubmissionScope::Title,
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
                        source_hint_for_attempt,
                        source_title_for_attempt,
                        ReleaseDownloadAttemptOutcome::Failed,
                        Some(error_message),
                        source_password,
                    )
                    .await;
                drop(dedupe_guard);
                return Err(error);
            }
        };

        drop(dedupe_guard);

        self.append_domain_event(new_title_domain_event(
            Some(actor.id.clone()),
            &title,
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: title_context_snapshot(&title),
                source_title: None,
                source_hint: None,
                download_id: Some(grab.job_id.clone()),
                episode_ids: Vec::new(),
            }),
        ))
        .await?;

        Ok(AddTitleAndQueueDownloadOutcome {
            title,
            metadata_hydration_state: add_outcome.metadata_hydration_state,
            reused_existing_title: add_outcome.reused_existing_title,
            download_job_id: grab.job_id,
            reused_queued_download: false,
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
    ) -> AppResult<String> {
        require(actor, &Entitlement::TriggerActions)?;

        let QueuedReleaseSelection {
            source_hint,
            source_kind,
            source_title,
        } = queued_release;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;

        let source_hint_for_attempt = normalize_release_attempt_value(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_value(source_title.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint_for_attempt.as_deref(),
            source_title_for_attempt.as_deref(),
            source_kind,
        );
        let source_password: Option<String> = None;
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
            drop(dedupe_guard);
            return Ok(existing.download_client_item_id);
        }

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
                        download_client_type: grab.client_type.clone(),
                        download_client_item_id: grab.job_id.clone(),
                        source_hint: None,
                        source_kind: None,
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
                        source_hint_for_attempt,
                        source_title_for_attempt,
                        ReleaseDownloadAttemptOutcome::Failed,
                        Some(error_message),
                        source_password,
                    )
                    .await;
                drop(dedupe_guard);
                return Err(error);
            }
        };

        drop(dedupe_guard);

        self.append_domain_event(new_title_domain_event(
            Some(actor.id.clone()),
            &title,
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: title_context_snapshot(&title),
                source_title: None,
                source_hint: None,
                download_id: Some(grab.job_id.clone()),
                episode_ids: Vec::new(),
            }),
        ))
        .await?;

        Ok(grab.job_id)
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

    /// Canonical owner for persisted title monitoring changes and title-level
    /// side effects. All title monitoring writes must flow through this helper.
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

    /// Canonical owner for persisted collection monitoring changes. All
    /// collection monitoring writes, including `update_collection(... monitored
    /// ...)`, must flow through this helper.
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

    /// Canonical owner for persisted episode monitoring changes. All episode
    /// monitoring writes, including `update_episode(... monitored ...)`, must
    /// flow through this helper.
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
                .persist_title_monitoring(&title.id, entry.monitored)
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
                .persist_title_monitoring(&title.id, entry.monitored)
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
                self.persist_collection_monitoring(&collection.id, false, false)
                    .await?;
            }
            for episode in &episodes {
                self.persist_episode_monitoring(&episode.id, false).await?;
            }

            if updated_title.monitored {
                for season in entry.seasons.iter().filter(|season| season.monitored) {
                    if let Some(collection) =
                        collections_by_season.get(&season.season_number.to_string())
                    {
                        self.persist_collection_monitoring(&collection.id, true, false)
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
                        self.persist_episode_monitoring(&matched_episode.id, true)
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
        collection_id: &str,
        monitored: bool,
        propagate_to_episodes: bool,
    ) -> AppResult<Collection> {
        let collection = self
            .persist_collection_monitoring(collection_id, monitored, propagate_to_episodes)
            .await?;

        if monitored {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&collection.title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", collection.title_id)))?;

            if !title.monitored {
                self.persist_title_monitoring(&title.id, true).await?;
                tracing::info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    "auto-monitored title because a collection was monitored"
                );
            } else {
                self.sync_title_for_immediate_acquisition(&title).await;
            }
        }

        Ok(collection)
    }

    /// Canonical owner for episode monitoring orchestration. Dedicated monitor
    /// mutations and generic episode updates must both delegate here so parent
    /// propagation and immediate acquisition behavior stay single-sourced.
    async fn apply_episode_monitoring_change(
        &self,
        episode_id: &str,
        monitored: bool,
    ) -> AppResult<Episode> {
        let episode = self
            .persist_episode_monitoring(episode_id, monitored)
            .await?;

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
                self.persist_title_monitoring(&title.id, true).await?;
                tracing::info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    "auto-monitored title because an episode was monitored"
                );
            } else {
                self.sync_title_for_immediate_acquisition(&title).await;
            }
        }

        Ok(episode)
    }

    pub async fn set_title_monitored(
        &self,
        actor: &User,
        id: &str,
        monitored: bool,
    ) -> AppResult<Title> {
        require(actor, &Entitlement::MonitorTitle)?;

        let title = self.persist_title_monitoring(id, monitored).await?;
        self.emit_title_updated_activity(Some(actor.id.clone()), &title)
            .await;
        Ok(title)
    }

    pub async fn set_collection_monitored(
        &self,
        actor: &User,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<Collection> {
        require(actor, &Entitlement::MonitorTitle)?;

        let collection = self
            .apply_collection_monitoring_change(collection_id, monitored, true)
            .await?;
        if let Some(title) = self
            .services
            .catalog
            .titles
            .get_by_id(&collection.title_id)
            .await?
        {
            self.emit_title_updated_activity(Some(actor.id.clone()), &title)
                .await;
        }
        Ok(collection)
    }

    pub async fn set_episode_monitored(
        &self,
        actor: &User,
        episode_id: &str,
        monitored: bool,
    ) -> AppResult<Episode> {
        require(actor, &Entitlement::MonitorTitle)?;

        let episode = self
            .apply_episode_monitoring_change(episode_id, monitored)
            .await?;
        if let Some(title) = self
            .services
            .catalog
            .titles
            .get_by_id(&episode.title_id)
            .await?
        {
            self.emit_title_updated_activity(Some(actor.id.clone()), &title)
                .await;
        }
        Ok(episode)
    }

    pub async fn delete_title(
        &self,
        actor: &User,
        id: &str,
        delete_files_on_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        let title = self
            .services
            .catalog
            .titles
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;

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

        // Purge recycle bin entries that belonged to this title.
        if let Some(media_root) = crate::recycle_bin::media_root_for_title(self, &title).await {
            let config = crate::recycle_bin::resolve_recycle_config(self, Some(&media_root)).await;
            match crate::recycle_bin::purge_for_title(&config, id).await {
                Ok(n) if n > 0 => info!(
                    purged = n,
                    title_id = %id,
                    "purged recycle bin entries for deleted title"
                ),
                Err(e) => warn!(
                    error = %e,
                    title_id = %id,
                    "failed to purge recycle entries for deleted title"
                ),
                _ => {}
            }
        }

        let queued_submission_keys = match self
            .services
            .workflow
            .download_submissions
            .list_for_title(id)
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
                    title_id = %id,
                    error = %err,
                    "failed to list download submissions while deleting title; falling back to embedded queue metadata only"
                );
                HashSet::new()
            }
        };

        // Cancel any inflight downloads for this title
        match self
            .services
            .integrations
            .download_client
            .list_queue()
            .await
        {
            Ok(queue_items) => {
                for item in queue_items {
                    let matches_title = item.title_id.as_deref() == Some(id)
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
                            title_id = %id,
                            download_item_id = %item.download_client_item_id,
                            error = %err,
                            "failed to cancel inflight download while deleting title"
                        );
                    }
                }
            }
            Err(err) => {
                warn!(
                    title_id = %id,
                    error = %err,
                    "failed to list download queue while deleting title; skipping download cancellation"
                );
            }
        }

        if let Err(err) = self
            .services
            .workflow
            .pending_releases
            .delete_pending_releases_for_title(id)
            .await
        {
            warn!(
                title_id = %id,
                error = %err,
                "failed to delete pending releases while deleting title"
            );
        }

        // Clean up wanted items for this title
        if let Err(err) = self
            .services
            .workflow
            .wanted_items
            .delete_wanted_items_for_title(id)
            .await
        {
            warn!(
                title_id = %id,
                error = %err,
                "failed to delete wanted items while deleting title"
            );
        }

        if let Err(err) = self
            .services
            .workflow
            .download_submissions
            .delete_for_title(id)
            .await
        {
            warn!(
                title_id = %id,
                error = %err,
                "failed to delete download submissions while deleting title"
            );
        }

        self.services.catalog.titles.delete(id).await?;

        let _ = self
            .append_domain_event(new_title_domain_event(
                Some(actor.id.clone()),
                &title,
                DomainEventPayload::TitleDeleted(TitleDeletedEventData {
                    title: title_context_snapshot(&title),
                }),
            ))
            .await;

        Ok(())
    }

    pub async fn delete_media_file(
        &self,
        actor: &User,
        file_id: &str,
        delete_from_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;

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
        require(actor, &Entitlement::ManageTitle)?;

        let title = self
            .services
            .catalog
            .titles
            .update_metadata(id, name, facet, tags)
            .await?;
        self.emit_title_updated_activity(Some(actor.id.clone()), &title)
            .await;
        Ok(title)
    }

    pub async fn fix_title_match(
        &self,
        actor: &User,
        title_id: &str,
        target_tvdb_id: &str,
    ) -> AppResult<FixTitleMatchResult> {
        require(actor, &Entitlement::ManageTitle)?;

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
        require(actor, &Entitlement::ViewCatalog)?;
        self.services.catalog.titles.get_by_id(id).await
    }

    pub async fn get_title_by_slug(
        &self,
        actor: &User,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .catalog
            .titles
            .get_by_facet_and_slug(facet, slug)
            .await
    }

    async fn validate_title_exists(&self, title_id: &str) -> AppResult<()> {
        self.services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .map(|_| ())
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))
    }

    pub async fn list_primary_collection_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .catalog
            .shows
            .list_primary_collection_summaries(title_ids)
            .await
    }

    pub async fn list_title_media_size_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .media_files
            .list_title_media_size_summaries(title_ids)
            .await
    }

    pub async fn list_title_quality_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .media_files
            .list_title_quality_summaries(title_ids)
            .await
    }

    pub async fn list_title_episode_progress_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .library
            .media_files
            .list_title_episode_progress_summaries(title_ids)
            .await
    }

    pub async fn list_collections(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<Collection>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.validate_title_exists(title_id).await?;
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
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await
    }

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
        require(actor, &Entitlement::ManageTitle)?;

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

        self.validate_title_exists(&title_id).await?;

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
        require(actor, &Entitlement::ManageTitle)?;

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
                self.apply_collection_monitoring_change(&collection_id, monitored, true)
                    .await?,
            );
        }

        let collection = collection.ok_or_else(|| {
            AppError::Validation("at least one collection field must be provided".into())
        })?;

        Ok(collection)
    }

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
        require(actor, &Entitlement::ManageTitle)?;

        if episode_type.trim().is_empty() {
            return Err(AppError::Validation("episode type is required".into()));
        }

        let parsed_episode_type =
            scryer_domain::EpisodeType::parse(episode_type.trim().to_lowercase().as_str())
                .ok_or_else(|| {
                    AppError::Validation(format!("unknown episode type: {}", episode_type))
                })?;

        self.validate_title_exists(&title_id).await?;

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
        require(actor, &Entitlement::ManageTitle)?;

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
                self.apply_episode_monitoring_change(&episode_id, monitored)
                    .await?,
            );
        }

        let episode = episode.ok_or_else(|| {
            AppError::Validation("at least one episode field must be provided".into())
        })?;

        Ok(episode)
    }

    pub async fn delete_collection(&self, actor: &User, collection_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        self.services
            .catalog
            .shows
            .delete_collection(collection_id)
            .await?;
        Ok(())
    }

    pub async fn delete_episode(&self, actor: &User, episode_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

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
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .catalog
            .shows
            .list_episodes_for_collection(collection_id)
            .await
    }

    pub async fn get_episode(&self, actor: &User, episode_id: &str) -> AppResult<Option<Episode>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await
    }

    pub async fn list_calendar_episodes(
        &self,
        actor: &User,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .catalog
            .shows
            .list_episodes_in_date_range(start_date, end_date)
            .await
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
        require(actor, &Entitlement::ManageConfig)?;

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
