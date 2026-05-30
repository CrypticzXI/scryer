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
impl AppUseCase {
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
                let new_image_url = normalize_episode_image_url(&ep.image_url);
                let tvdb_id_changed = new_tvdb_id.as_deref() != existing.tvdb_id.as_deref();
                let image_url_changed = new_image_url.as_deref() != existing.image_url.as_deref();
                if title_changed || overview_changed || tvdb_id_changed || image_url_changed {
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
                                image_url: if image_url_changed {
                                    new_image_url.clone()
                                } else {
                                    None
                                },
                                clear_image_url: image_url_changed && new_image_url.is_none(),
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
                image_url: normalize_episode_image_url(&ep.image_url),
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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

}
impl AppUseCase {
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

}
impl AppUseCase {
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
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };

        let episode = self.services.catalog.shows.create_episode(episode).await?;
        Ok(episode)
    }

}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
fn normalize_episode_image_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = url::Url::parse(trimmed).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        return None;
    }

    Some(parsed.to_string())
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
