pub(crate) const HYDRATION_BULK_BATCH_SIZE: usize = 20;
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
}
impl AppUseCase {
    async fn emit_hydration_completed(&self, title: &Title) {
        self.emit_metadata_hydration_updated_event(title, MetadataHydrationState::Completed, None)
            .await;
    }
}
impl AppUseCase {
    async fn emit_hydration_failed(&self, title: &Title, reason: &str) {
        self.emit_metadata_hydration_updated_event(
            title,
            MetadataHydrationState::Failed,
            Some(reason.to_string()),
        )
        .await;
    }
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
    pub(crate) async fn hydrate_titles_bulk(
        &self,
        targets: Vec<HydrationTarget>,
    ) -> AppResult<HydrationBatchOutcome> {
        self.hydrate_titles_bulk_cancellable(targets, None).await
    }
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
/// After successful hydration, sync wanted items for monitored titles.
async fn sync_wanted_after_hydration(app: &AppUseCase, title: &scryer_domain::Title) {
    if title.monitored && title.metadata_fetched_at.is_some() {
        app.sync_title_for_immediate_acquisition(title).await;
    }
}
