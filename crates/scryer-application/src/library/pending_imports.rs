use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration as StdDuration;

use scryer_domain::{Entitlement, ExternalId, MediaFacet, NewTitle};

use chrono::Utc;
use tracing::warn;

use super::*;
use crate::library::library::{
    PlannedTitleScanFile, PlannedTitleScanRecord, file_source_signature_from_metadata,
    file_source_snapshot_from_library_file, finalize_title_scan_file,
};

const MAX_PENDING_IMPORTS_PAGE_SIZE: i64 = 200;

fn build_pending_import_search_attempt(
    attempt: &LibraryScanUnmatchedSearchAttempt,
) -> PendingImportSearchAttempt {
    let top_results = attempt.top_results.clone();
    let top_results_summary = if top_results.is_empty() {
        "no results".to_string()
    } else {
        top_results.join(" | ")
    };

    PendingImportSearchAttempt {
        query: attempt.query.clone(),
        result_count: attempt.result_count,
        top_results,
        summary: format!(
            "{} result{}: {}",
            attempt.result_count,
            if attempt.result_count == 1 { "" } else { "s" },
            top_results_summary
        ),
    }
}

fn pending_import_movie_entry_path(item: &LibraryScanUnmatchedItem) -> PathBuf {
    let item_path = PathBuf::from(item.item_path.trim());
    let scan_root = Path::new(item.scan_root.trim());

    if let Ok(relative) = item_path.strip_prefix(scan_root)
        && let Some(first_component) = relative.components().next()
    {
        return scan_root.join(first_component.as_os_str());
    }

    item_path
}

fn pending_import_folder_path(item: &LibraryScanUnmatchedItem) -> Option<String> {
    match item.facet {
        MediaFacet::Movie => {
            let entry_path = pending_import_movie_entry_path(item);
            let entry_path = entry_path.to_string_lossy().trim().to_string();
            if entry_path.is_empty() || entry_path == item.item_path {
                None
            } else {
                Some(entry_path)
            }
        }
        MediaFacet::Series | MediaFacet::Anime => Some(item.item_path.clone()),
    }
}

fn pending_import_item_from_unmatched(item: LibraryScanUnmatchedItem) -> PendingImportItem {
    let folder_path = pending_import_folder_path(&item);
    let search_attempts = item
        .search_attempts
        .iter()
        .map(build_pending_import_search_attempt)
        .collect();

    PendingImportItem {
        id: item.id,
        facet: item.facet,
        status: item.status,
        title_id: item.title_id,
        display_name: item.display_name,
        path: item.item_path,
        folder_path,
        query: item.query,
        year_hint: item.year_hint,
        reason: item.reason_code,
        search_attempts,
    }
}

async fn build_pending_import_library_file(
    item: &LibraryScanUnmatchedItem,
) -> AppResult<LibraryFile> {
    let item_path = item.item_path.trim();
    if item_path.is_empty() {
        return Err(AppError::Validation(
            "pending import path is missing or invalid".into(),
        ));
    }

    let path = PathBuf::from(item_path);
    let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
        AppError::Validation(format!("pending import file is unavailable: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(AppError::Validation(
            "pending import path is not a file".into(),
        ));
    }

    let signature = file_source_signature_from_metadata(&metadata);
    let display_name = if item.display_name.trim().is_empty() {
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(item_path)
            .to_string()
    } else {
        item.display_name.clone()
    };

    Ok(LibraryFile {
        path: path.to_string_lossy().trim().to_string(),
        display_name,
        nfo_path: None,
        size_bytes: Some(metadata.len() as i64),
        source_signature_scheme: signature.as_ref().map(|value| value.scheme.clone()),
        source_signature_value: signature.map(|value| value.value),
    })
}

async fn list_pending_import_title_episodes(
    app: &AppUseCase,
    title_id: &str,
) -> AppResult<Vec<Episode>> {
    let mut episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(title_id)
        .await?;
    episodes.sort_by(|left, right| {
        let left_season = left
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let right_season = right
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let left_episode = left
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let right_episode = right
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        left_season
            .cmp(&right_season)
            .then(left_episode.cmp(&right_episode))
            .then(left.id.cmp(&right.id))
    });
    Ok(episodes)
}

fn pending_import_parse_raw_name(item: &LibraryScanUnmatchedItem) -> &str {
    Path::new(item.item_path.trim())
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(item.display_name.as_str())
}

fn pending_import_suggested_episode_ids(
    parsed: &ParsedReleaseMetadata,
    available_episodes: &[Episode],
) -> Vec<String> {
    let Some(episode) = parsed.episode.as_ref() else {
        return Vec::new();
    };

    let mut suggested = Vec::new();

    if !episode.episode_numbers.is_empty() {
        let season_number = episode.season.unwrap_or(1).to_string();
        for episode_number in &episode.episode_numbers {
            let episode_number = episode_number.to_string();
            if let Some(matched) = available_episodes.iter().find(|candidate| {
                candidate.season_number.as_deref() == Some(season_number.as_str())
                    && candidate.episode_number.as_deref() == Some(episode_number.as_str())
            }) {
                suggested.push(matched.id.clone());
            }
        }
    }

    if suggested.is_empty()
        && let Some(absolute_episode) = episode.absolute_episode
    {
        let absolute_episode = absolute_episode.to_string();
        if let Some(matched) = available_episodes.iter().find(|candidate| {
            candidate.absolute_number.as_deref() == Some(absolute_episode.as_str())
        }) {
            suggested.push(matched.id.clone());
        }
    }

    if suggested.is_empty() && !episode.special_absolute_episode_numbers.is_empty() {
        for absolute_episode in &episode.special_absolute_episode_numbers {
            let absolute_episode = absolute_episode.to_string();
            if let Some(matched) = available_episodes.iter().find(|candidate| {
                candidate.absolute_number.as_deref() == Some(absolute_episode.as_str())
            }) {
                suggested.push(matched.id.clone());
            }
        }
    }

    if suggested.is_empty()
        && let Some(air_date) = episode.air_date
    {
        let air_date = air_date.to_string();
        suggested.extend(
            available_episodes
                .iter()
                .filter(|candidate| candidate.air_date.as_deref() == Some(air_date.as_str()))
                .map(|candidate| candidate.id.clone()),
        );
    }

    if suggested.is_empty() && episode.full_season {
        let season_number = episode.season.unwrap_or(1).to_string();
        suggested.extend(
            available_episodes
                .iter()
                .filter(|candidate| {
                    candidate.season_number.as_deref() == Some(season_number.as_str())
                })
                .map(|candidate| candidate.id.clone()),
        );
    }

    let mut deduped = Vec::with_capacity(suggested.len());
    let mut seen = HashSet::new();
    for episode_id in suggested {
        if seen.insert(episode_id.clone()) {
            deduped.push(episode_id);
        }
    }
    deduped
}

fn library_scan_summary_has_pending_import_success(summary: &LibraryScanSummary) -> bool {
    summary.imported > 0 || summary.matched > 0
}

struct PendingImportResolutionGuard {
    pending_import_id: String,
    locks: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl Drop for PendingImportResolutionGuard {
    fn drop(&mut self) {
        if let Ok(mut locks) = self.locks.lock() {
            locks.remove(&self.pending_import_id);
        }
    }
}

impl AppUseCase {
    fn acquire_pending_import_resolution_guard(
        &self,
        pending_import_id: &str,
    ) -> AppResult<PendingImportResolutionGuard> {
        let mut locks = self
            .pending_import_resolution_locks
            .lock()
            .map_err(|_| AppError::Repository("pending import resolution lock poisoned".into()))?;
        if !locks.insert(pending_import_id.to_string()) {
            return Err(AppError::Validation(format!(
                "pending import {pending_import_id} is already being resolved"
            )));
        }

        Ok(PendingImportResolutionGuard {
            pending_import_id: pending_import_id.to_string(),
            locks: self.pending_import_resolution_locks.clone(),
        })
    }

    async fn rollback_pending_import_title_binding(
        &self,
        actor: &User,
        title: &Title,
        previous_folder_path: Option<&str>,
        created: bool,
    ) {
        if created {
            let _ = self.delete_title(actor, &title.id, false, None).await;
            return;
        }

        let restore_result = match previous_folder_path {
            Some(folder_path) => {
                self.services
                    .catalog
                    .titles
                    .set_folder_path(&title.id, folder_path)
                    .await
            }
            None => {
                self.services
                    .catalog
                    .titles
                    .clear_folder_path(&title.id)
                    .await
            }
        };

        if let Err(error) = restore_result {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to rollback pending import folder path"
            );
        }
    }

    async fn rollback_created_pending_import_title(
        &self,
        actor: &User,
        title: &Title,
        created: bool,
    ) {
        if created {
            self.rollback_pending_import_title_binding(actor, title, None, true)
                .await;
        }
    }

    pub async fn pending_import_counts(&self, actor: &User) -> AppResult<PendingImportCounts> {
        require(actor, &Entitlement::ManageTitle)?;

        let repository = self.services.library.library_scan_unmatched_items.clone();
        let movie_repo = repository.clone();
        let series_repo = repository.clone();
        let anime_repo = repository;
        let (movie, series, anime) = tokio::try_join!(
            movie_repo.count_library_scan_unmatched_items(
                Some(MediaFacet::Movie),
                None,
                Some(PendingImportStatus::Pending),
            ),
            series_repo.count_library_scan_unmatched_items(
                Some(MediaFacet::Series),
                None,
                Some(PendingImportStatus::Pending),
            ),
            anime_repo.count_library_scan_unmatched_items(
                Some(MediaFacet::Anime),
                None,
                Some(PendingImportStatus::Pending),
            ),
        )?;

        Ok(PendingImportCounts {
            movie,
            series,
            anime,
        })
    }

    pub async fn pending_imports(
        &self,
        actor: &User,
        facet: MediaFacet,
        status: PendingImportStatus,
        limit: i64,
        offset: i64,
    ) -> AppResult<PendingImportConnection> {
        require(actor, &Entitlement::ManageTitle)?;

        let limit = limit.clamp(0, MAX_PENDING_IMPORTS_PAGE_SIZE);
        let offset = offset.max(0);
        let total = self
            .services
            .library
            .library_scan_unmatched_items
            .count_library_scan_unmatched_items(Some(facet.clone()), None, Some(status))
            .await?;
        let items = self
            .services
            .library
            .library_scan_unmatched_items
            .list_library_scan_unmatched_items(Some(facet), None, Some(status), limit, offset)
            .await?
            .into_iter()
            .map(pending_import_item_from_unmatched)
            .collect();

        Ok(PendingImportConnection { total, items })
    }

    pub async fn ignore_pending_import(
        &self,
        actor: &User,
        pending_import_id: &str,
    ) -> AppResult<IgnorePendingImportResult> {
        require(actor, &Entitlement::ManageTitle)?;

        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }
        let _pending_import_resolution_guard =
            self.acquire_pending_import_resolution_guard(pending_import_id)?;

        let mut item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;

        if item.status != PendingImportStatus::Ignored {
            item.status = PendingImportStatus::Ignored;
            item.updated_at = Utc::now().to_rfc3339();
            self.services
                .library
                .library_scan_unmatched_items
                .upsert_library_scan_unmatched_item(&item)
                .await?;
        }

        Ok(IgnorePendingImportResult {
            id: item.id,
            status: item.status,
        })
    }

    pub async fn resolve_pending_import(
        &self,
        actor: &User,
        pending_import_id: &str,
        target_tvdb_id: &str,
    ) -> AppResult<ResolvePendingImportResult> {
        require(actor, &Entitlement::ManageTitle)?;

        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }
        let _pending_import_resolution_guard =
            self.acquire_pending_import_resolution_guard(pending_import_id)?;

        let target_tvdb_id = target_tvdb_id.trim();
        if target_tvdb_id.is_empty() {
            return Err(AppError::Validation("tvdb id is required".into()));
        }
        let target_tvdb_numeric = target_tvdb_id
            .parse::<i64>()
            .map_err(|_| AppError::Validation("tvdb id must be numeric".into()))?;

        let item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;
        if item.title_id.is_some() {
            return Err(AppError::Validation(
                "pending import requires explicit episode binding".into(),
            ));
        }

        let existing_title = self
            .services
            .catalog
            .titles
            .find_by_external_id_in_facet(item.facet.clone(), "tvdb", target_tvdb_id)
            .await?;

        let (title, created) = if let Some(existing_title) = existing_title {
            (existing_title, false)
        } else {
            let metadata_language = self.metadata_language().await;
            let new_title = match item.facet {
                MediaFacet::Movie => {
                    let movie = self
                        .services
                        .library
                        .metadata_gateway
                        .get_movie(target_tvdb_numeric, &metadata_language)
                        .await?;
                    NewTitle {
                        name: movie.name,
                        facet: item.facet.clone(),
                        monitored: false,
                        tags: vec![],
                        external_ids: vec![ExternalId {
                            source: "tvdb".to_string(),
                            value: target_tvdb_id.to_string(),
                        }],
                        min_availability: None,
                        poster_url: Some(movie.poster_url),
                        year: movie.year,
                        overview: Some(movie.overview),
                        sort_title: Some(movie.sort_title),
                        slug: Some(movie.slug),
                        runtime_minutes: Some(movie.runtime_minutes),
                        language: Some(movie.language),
                        content_status: Some(movie.content_status),
                    }
                }
                MediaFacet::Series | MediaFacet::Anime => {
                    let series = self
                        .services
                        .library
                        .metadata_gateway
                        .get_series(target_tvdb_numeric, &metadata_language)
                        .await?;
                    NewTitle {
                        name: series.name,
                        facet: item.facet.clone(),
                        monitored: false,
                        tags: vec![],
                        external_ids: vec![ExternalId {
                            source: "tvdb".to_string(),
                            value: target_tvdb_id.to_string(),
                        }],
                        min_availability: None,
                        poster_url: Some(series.poster_url),
                        year: series.year,
                        overview: Some(series.overview),
                        sort_title: Some(series.sort_name),
                        slug: Some(series.slug),
                        runtime_minutes: Some(series.runtime_minutes),
                        language: None,
                        content_status: Some(series.content_status),
                    }
                }
            };
            let created = self
                .create_title_without_hydration(actor, new_title)
                .await?;
            (created.title, !created.reused_existing)
        };

        let scan_path = match item.facet {
            MediaFacet::Movie => pending_import_movie_entry_path(&item),
            MediaFacet::Series | MediaFacet::Anime => PathBuf::from(item.item_path.trim()),
        };
        let scan_path_string = scan_path.to_string_lossy().trim().to_string();
        if scan_path_string.is_empty() {
            return Err(AppError::Validation(
                "pending import path is missing or invalid".into(),
            ));
        }

        let summary = if matches!(item.facet, MediaFacet::Series | MediaFacet::Anime) {
            let scan_metadata = match tokio::fs::metadata(&scan_path).await {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.rollback_created_pending_import_title(actor, &title, created)
                        .await;
                    return Err(AppError::Validation(format!(
                        "pending import path is unavailable: {error}"
                    )));
                }
            };

            if scan_metadata.is_file() {
                let library_file = match build_pending_import_library_file(&item).await {
                    Ok(file) => file,
                    Err(err) => {
                        self.rollback_created_pending_import_title(actor, &title, created)
                            .await;
                        return Err(err);
                    }
                };
                match self
                    .scan_title_library_with_discovered_files(
                        actor,
                        title.clone(),
                        vec![library_file],
                    )
                    .await
                {
                    Ok(summary) if library_scan_summary_has_pending_import_success(&summary) => {
                        summary
                    }
                    Ok(_) => {
                        self.rollback_created_pending_import_title(actor, &title, created)
                            .await;
                        return Err(AppError::Validation(format!(
                            "no media files were found at {}",
                            scan_path_string
                        )));
                    }
                    Err(err) => {
                        self.rollback_created_pending_import_title(actor, &title, created)
                            .await;
                        return Err(err);
                    }
                }
            } else if scan_metadata.is_dir() {
                let previous_folder_path = title.folder_path.clone();
                self.services
                    .catalog
                    .titles
                    .set_folder_path(&title.id, &scan_path_string)
                    .await?;

                match self.scan_title_library(actor, &title.id).await {
                    Ok(summary) if library_scan_summary_has_pending_import_success(&summary) => {
                        summary
                    }
                    Ok(_) => {
                        self.rollback_pending_import_title_binding(
                            actor,
                            &title,
                            previous_folder_path.as_deref(),
                            created,
                        )
                        .await;
                        return Err(AppError::Validation(format!(
                            "no media files were found at {}",
                            scan_path_string
                        )));
                    }
                    Err(err) => {
                        self.rollback_pending_import_title_binding(
                            actor,
                            &title,
                            previous_folder_path.as_deref(),
                            created,
                        )
                        .await;
                        return Err(err);
                    }
                }
            } else {
                self.rollback_created_pending_import_title(actor, &title, created)
                    .await;
                return Err(AppError::Validation(
                    "pending import path must be a file or directory".into(),
                ));
            }
        } else {
            let previous_folder_path = title.folder_path.clone();
            self.services
                .catalog
                .titles
                .set_folder_path(&title.id, &scan_path_string)
                .await?;

            match self.scan_title_library(actor, &title.id).await {
                Ok(summary) if library_scan_summary_has_pending_import_success(&summary) => summary,
                Ok(_) => {
                    self.rollback_pending_import_title_binding(
                        actor,
                        &title,
                        previous_folder_path.as_deref(),
                        created,
                    )
                    .await;
                    return Err(AppError::Validation(format!(
                        "no media files were found at {}",
                        scan_path_string
                    )));
                }
                Err(err) => {
                    self.rollback_pending_import_title_binding(
                        actor,
                        &title,
                        previous_folder_path.as_deref(),
                        created,
                    )
                    .await;
                    return Err(err);
                }
            }
        };

        self.services
            .library
            .library_scan_unmatched_items
            .delete_library_scan_unmatched_item(item.facet.clone(), &item.item_path)
            .await?;

        let refreshed_title = self
            .services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await?
            .unwrap_or(title);

        Ok(ResolvePendingImportResult {
            title: refreshed_title,
            created,
            library_scan: summary,
        })
    }

    pub async fn preview_title_bound_pending_import(
        &self,
        actor: &User,
        pending_import_id: &str,
    ) -> AppResult<PendingImportBindingPreview> {
        require(actor, &Entitlement::ManageTitle)?;

        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }

        let item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;
        let title_id = item.title_id.as_deref().ok_or_else(|| {
            AppError::Validation("pending import does not have a known title".into())
        })?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;

        let available_episodes = list_pending_import_title_episodes(self, &title.id).await?;
        let parse_context =
            crate::build_release_parse_context_for_title(&title, &available_episodes, None);
        let parsed = crate::parse_release_metadata_for_target(
            pending_import_parse_raw_name(&item),
            &parse_context,
        );
        let suggested_episode_ids =
            pending_import_suggested_episode_ids(&parsed, &available_episodes);
        let file = build_pending_import_library_file(&item).await?;

        Ok(PendingImportBindingPreview {
            title,
            file: PendingImportBindingFilePreview {
                file_path: file.path.clone(),
                file_name: file.display_name.clone(),
                size_bytes: file.size_bytes.unwrap_or_default(),
                parsed_season: parsed.episode.as_ref().and_then(|episode| episode.season),
                parsed_episodes: parsed
                    .episode
                    .as_ref()
                    .map(|episode| episode.episode_numbers.clone())
                    .unwrap_or_default(),
                parsed_absolute_numbers: parsed
                    .episode
                    .as_ref()
                    .map(|episode| {
                        let mut absolute_numbers = episode.special_absolute_episode_numbers.clone();
                        if let Some(value) = episode.absolute_episode {
                            absolute_numbers.push(value);
                        }
                        absolute_numbers
                    })
                    .unwrap_or_default(),
                suggested_episode_ids,
            },
            available_episodes,
        })
    }

    pub async fn bind_title_bound_pending_import(
        &self,
        actor: &User,
        pending_import_id: &str,
        collection_id: Option<&str>,
        episode_ids: &[String],
    ) -> AppResult<ResolvePendingImportResult> {
        require(actor, &Entitlement::ManageTitle)?;

        let pending_import_id = pending_import_id.trim();
        if pending_import_id.is_empty() {
            return Err(AppError::Validation("pending import id is required".into()));
        }
        let _pending_import_resolution_guard =
            self.acquire_pending_import_resolution_guard(pending_import_id)?;

        let item = self
            .services
            .library
            .library_scan_unmatched_items
            .get_library_scan_unmatched_item(pending_import_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("pending import {pending_import_id}")))?;
        let title_id = item.title_id.as_deref().ok_or_else(|| {
            AppError::Validation("pending import does not have a known title".into())
        })?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        let available_episodes = list_pending_import_title_episodes(self, &title.id).await?;

        let target_episodes = if let Some(collection_id) = collection_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let episodes = available_episodes
                .iter()
                .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
                .cloned()
                .collect::<Vec<_>>();
            if episodes.is_empty() {
                return Err(AppError::Validation(format!(
                    "collection {collection_id} does not belong to title {}",
                    title.id
                )));
            }
            episodes
        } else {
            let requested_ids = episode_ids
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .collect::<HashSet<_>>();
            if requested_ids.is_empty() {
                return Err(AppError::Validation(
                    "at least one episode must be selected".into(),
                ));
            }
            let episodes = available_episodes
                .iter()
                .filter(|episode| requested_ids.contains(episode.id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if episodes.len() != requested_ids.len() {
                return Err(AppError::Validation(
                    "one or more selected episodes do not belong to the target title".into(),
                ));
            }
            episodes
        };

        let file = build_pending_import_library_file(&item).await?;
        let parse_context =
            crate::build_release_parse_context_for_title(&title, &available_episodes, None);
        let parsed = crate::parse_release_metadata_for_target(
            pending_import_parse_raw_name(&item),
            &parse_context,
        );
        let snapshot = file_source_snapshot_from_library_file(&file).ok_or_else(|| {
            AppError::Validation("pending import file metadata is incomplete".into())
        })?;
        let analysis_outcome = match self
            .services
            .library
            .media_analyzer
            .analyze_file(PathBuf::from(&file.path))
            .await
        {
            Ok(outcome) => Some(outcome),
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %file.path,
                    "failed to analyze title-bound pending import file"
                );
                None
            }
        };

        let mut episode_links = HashSet::new();
        let mut summary = LibraryScanSummary::default();
        let mut db_elapsed = StdDuration::ZERO;
        finalize_title_scan_file(
            self,
            &title,
            PlannedTitleScanFile {
                file,
                parsed,
                target_episodes,
                snapshot,
                record: PlannedTitleScanRecord::New,
            },
            analysis_outcome,
            LibraryScanMode::Full,
            &mut episode_links,
            &mut summary,
            &mut db_elapsed,
        )
        .await;

        if !library_scan_summary_has_pending_import_success(&summary) {
            return Err(AppError::Validation(
                "failed to bind pending import file to selected episodes".into(),
            ));
        }

        self.services
            .library
            .library_scan_unmatched_items
            .delete_library_scan_unmatched_item(item.facet.clone(), &item.item_path)
            .await?;

        let refreshed_title = self
            .services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await?
            .unwrap_or(title);

        Ok(ResolvePendingImportResult {
            title: refreshed_title,
            created: false,
            library_scan: summary,
        })
    }
}
