const REMATCH_REPLACED_EXTERNAL_ID_SOURCES: &[&str] =
    &["tvdb", "imdb", "tmdb", "mal", "anilist", "anidb", "kitsu"];
const REMATCH_DERIVED_TAG_PREFIXES: &[&str] = &[
    "scryer:mal-score:",
    "scryer:anime-media-type:",
    "scryer:anime-status:",
];
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
impl AppUseCase {
    pub(crate) async fn metadata_language(&self) -> String {
        self.read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, METADATA_LANGUAGE_KEY, None)
            .await
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "eng".to_string())
    }
}
impl AppUseCase {
    pub(crate) async fn apply_title_metadata_update(
        &self,
        actor: impl Into<DomainEventActor>,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .update_metadata(id, name, facet, tags, None)
            .await?;
        self.emit_title_updated_activity(actor, &title)
            .await;
        Ok(title)
    }
}
impl AppUseCase {
    pub async fn update_title_metadata(
        &self,
        actor: &User,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        self.update_title_metadata_with_root_folder_id(actor, id, name, facet, tags, None)
            .await
    }

    pub async fn update_title_metadata_with_root_folder_id(
        &self,
        actor: &User,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
        root_folder_id: Option<Option<String>>,
    ) -> AppResult<Title> {
        if name.is_none() && facet.is_none() && tags.is_none() && root_folder_id.is_none() {
            return Err(AppError::Validation(
                "at least one title field must be provided".into(),
            ));
        }
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(facet) = facet.as_ref()
            && facet != &title.facet
        {
            return Err(AppError::Validation(
                "changing a title facet is not supported because titles cannot move between libraries"
                    .into(),
            ));
        }
        let resolved_root_folder_id = match root_folder_id {
            Some(Some(root_folder_id)) => Some(
                self.resolve_title_root_folder_id_for_library(
                    &title.library_id,
                    Some(root_folder_id.as_str()),
                )
                .await?,
            ),
            Some(None) => Some(
                self.resolve_title_root_folder_id_for_library(&title.library_id, None)
                    .await?,
            ),
            None => None,
        };

        let title = self
            .services
            .catalog
            .titles
            .update_metadata(id, name, facet, tags, resolved_root_folder_id)
            .await?;

        self.emit_title_updated_activity(actor, &title).await;
        Ok(title)
    }

    pub async fn set_primary_movie_file(
        &self,
        actor: &User,
        title_id: &str,
        file_id: &str,
    ) -> AppResult<Title> {
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

        let media_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await?;
        let selected_file = media_files
            .iter()
            .find(|file| file.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        if title.facet != MediaFacet::Movie {
            let series_movie_link_id = selected_file
                .series_movie_link_ids
                .first()
                .ok_or_else(|| {
                    AppError::Validation(
                        "primary movie file can only be set for movie titles or series movie files"
                            .to_string(),
                    )
                })?;
            let additional_file_ids = media_files
                .iter()
                .filter(|file| file.id != selected_file.id)
                .filter(|file| {
                    file.series_movie_link_ids
                        .iter()
                        .any(|link_id| link_id == series_movie_link_id)
                })
                .map(|file| file.id.clone())
                .collect::<Vec<_>>();

            self.services
                .library
                .media_files
                .set_media_file_roles_for_title(&title.id, &selected_file.id, &additional_file_ids)
                .await?;
            self.emit_title_updated_activity(actor, &title)
                .await;
            return Ok(title);
        }
        let movie_scope =
            crate::library::movie_scan_scope::MovieScanScope::from_title_folder_or_file(
                title.folder_path.as_deref(),
                &selected_file.file_path,
            )
            .ok_or_else(|| {
                AppError::Validation("movie title does not have a canonical folder path".to_string())
            })?;
        if !movie_scope.file_is_inside_canonical_folder(&selected_file.file_path) {
            return Err(AppError::Validation(
                "selected file is outside the title's canonical movie folder".to_string(),
            ));
        }

        let additional_file_ids = media_files
            .iter()
            .filter(|file| file.id != selected_file.id)
            .filter(|file| movie_scope.file_is_inside_canonical_folder(&file.file_path))
            .map(|file| file.id.clone())
            .collect::<Vec<_>>();

        self.services
            .library
            .media_files
            .set_media_file_roles_for_title(&title.id, &selected_file.id, &additional_file_ids)
            .await?;
        self.emit_title_updated_activity(actor, &title)
            .await;
        Ok(title)
    }
}
impl AppUseCase {
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
                    && let Ok(import_paths) =
                        crate::import_workflow::resolve_import_paths(self, &existing_title).await
                {
                    legacy_folder_path = Some(
                        crate::effective_title_folder_path(
                            &import_paths.media_root,
                            &existing_title,
                            &import_paths.folder_template,
                            None,
                        )
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
            actor,
            &refreshed_title,
            DomainEventPayload::TitleRematched(TitleRematchedEventData {
                title: title_context_snapshot(&refreshed_title),
                old_tvdb_id,
                new_tvdb_id: target_tvdb_id.to_string(),
                source: "manual".to_string(),
            }),
        ))
        .await?;
        self.emit_title_updated_activity(actor, &refreshed_title)
            .await;

        Ok(FixTitleMatchResult {
            hydrated: refreshed_title.metadata_fetched_at.is_some(),
            title: refreshed_title,
            library_scan,
            warnings,
        })
    }
}
impl AppUseCase {
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
pub(crate) fn extract_tvdb_id(title: &scryer_domain::Title) -> Option<i64> {
    title
        .external_ids
        .iter()
        .find(|eid| eid.source == "tvdb")
        .and_then(|eid| eid.value.parse::<i64>().ok())
}
