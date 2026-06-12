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
}
impl AppUseCase {
    async fn sync_wanted_movie(&self, title: &Title, now: &DateTime<Utc>) {
        self.sync_wanted_movie_inner(title, now, false).await;
    }
}
impl AppUseCase {
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
}
impl AppUseCase {
    async fn sync_wanted_series(&self, title: &Title, now: &DateTime<Utc>) {
        self.sync_wanted_series_inner(title, now, false).await;
    }
}
impl AppUseCase {
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
}
impl AppUseCase {
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
pub(crate) async fn has_enabled_download_clients(app: &AppUseCase) -> bool {
    app.services
        .integrations
        .download_client_configs
        .list(None)
        .await
        .map(|configs| configs.into_iter().any(|config| config.is_enabled))
        .unwrap_or(false)
}
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
