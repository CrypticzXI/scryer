use std::path::{Path, PathBuf};

use scryer_domain::{Entitlement, ExternalId, MediaFacet, NewTitle};

use super::*;

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
        display_name: item.display_name,
        path: item.item_path,
        folder_path,
        query: item.query,
        year_hint: item.year_hint,
        reason: item.reason_code,
        search_attempts,
    }
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

    pub async fn pending_import_counts(&self, actor: &User) -> AppResult<PendingImportCounts> {
        require(actor, &Entitlement::ManageTitle)?;

        let repository = self.services.library.library_scan_unmatched_items.clone();
        let movie_repo = repository.clone();
        let series_repo = repository.clone();
        let anime_repo = repository;
        let (movie, series, anime) = tokio::try_join!(
            movie_repo.count_library_scan_unmatched_items(Some(MediaFacet::Movie), None),
            series_repo.count_library_scan_unmatched_items(Some(MediaFacet::Series), None),
            anime_repo.count_library_scan_unmatched_items(Some(MediaFacet::Anime), None),
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
            .count_library_scan_unmatched_items(Some(facet.clone()), None)
            .await?;
        let items = self
            .services
            .library
            .library_scan_unmatched_items
            .list_library_scan_unmatched_items(Some(facet), None, limit, offset)
            .await?
            .into_iter()
            .map(pending_import_item_from_unmatched)
            .collect();

        Ok(PendingImportConnection { total, items })
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

        let previous_folder_path = title.folder_path.clone();
        self.services
            .catalog
            .titles
            .set_folder_path(&title.id, &scan_path_string)
            .await?;

        let summary = match self.scan_title_library(actor, &title.id).await {
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
}
