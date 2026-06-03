use scryer_domain::{MediaFacet, Title};

use crate::ports::{EpisodeImageUrlUpdate, TitleArtworkUrlUpdate};
use crate::{AppError, AppResult, AppUseCase, User};

const TITLE_IMAGE_CACHE_REFRESH_BATCH_SIZE: usize = 100;

struct TitleImageCacheClearScheduledGuard {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for TitleImageCacheClearScheduledGuard {
    fn drop(&mut self) {
        self.flag.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TitleImageCacheRefreshSummary {
    pub titles_scanned: u64,
    pub titles_linked: u64,
    pub title_urls_updated: u64,
    pub episode_urls_updated: u64,
    pub missing_artwork_results: u64,
    pub cache_cleared: bool,
}

impl AppUseCase {
    pub async fn clear_title_image_cache(&self, actor: &User) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let app = self.clone();
        tokio::spawn(async move {
            match app.run_title_image_cache_refresh().await {
                Ok(summary) => {
                    info!(
                        titles_scanned = summary.titles_scanned,
                        title_urls_updated = summary.title_urls_updated,
                        episode_urls_updated = summary.episode_urls_updated,
                        missing_artwork_results = summary.missing_artwork_results,
                        "title image cache refresh completed"
                    );
                }
                Err(error) => {
                    warn!(error = %error, "title image cache refresh failed");
                }
            }
        });

        Ok(true)
    }

    pub async fn run_title_image_cache_refresh(&self) -> AppResult<TitleImageCacheRefreshSummary> {
        let scheduled = self
            .runtime
            .catalog
            .title_image_cache_clear_scheduled
            .clone();
        if scheduled.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(AppError::Validation(
                "title image cache refresh is already running".to_string(),
            ));
        }
        let _scheduled_guard = TitleImageCacheClearScheduledGuard { flag: scheduled };

        let _maintenance_guard = loop {
            let active_scans = self.runtime.library.library_scan_tracker.list_active().await;
            if !active_scans.is_empty() {
                info!(
                    active_scans = active_scans.len(),
                    "title image cache refresh pausing while library scan is active"
                );
                self.runtime
                    .library
                    .library_scan_tracker
                    .wait_until_idle()
                    .await;
                info!("title image cache refresh resuming after library scan");
            }
            let guard = self
                .runtime
                .catalog
                .title_image_maintenance_lock
                .write()
                .await;
            if self
                .runtime
                .library
                .library_scan_tracker
                .list_active()
                .await
                .is_empty()
            {
                break guard;
            }
        };

        let mut summary = self.rehydrate_remote_artwork_urls().await?;
        self.services
            .library
            .title_images
            .clear_title_image_cache()
            .await?;
        summary.cache_cleared = true;
        info!(
            titles_scanned = summary.titles_scanned,
            titles_linked = summary.titles_linked,
            title_urls_updated = summary.title_urls_updated,
            episode_urls_updated = summary.episode_urls_updated,
            missing_artwork_results = summary.missing_artwork_results,
            "title image cache reset completed"
        );
        self.wake_title_image_loops();
        Ok(summary)
    }

    async fn rehydrate_remote_artwork_urls(&self) -> AppResult<TitleImageCacheRefreshSummary> {
        let language = self.metadata_language().await;
        let mut after_id = None;
        let mut summary = TitleImageCacheRefreshSummary::default();

        loop {
            let titles = self
                .services
                .catalog
                .titles
                .list_page_after_id(after_id.clone(), TITLE_IMAGE_CACHE_REFRESH_BATCH_SIZE)
                .await?;
            if titles.is_empty() {
                break;
            }

            after_id = titles.last().map(|title| title.id.clone());
            let batch_summary = self
                .rehydrate_remote_artwork_urls_for_title_batch(&titles, &language)
                .await?;
            summary.titles_scanned += batch_summary.titles_scanned;
            summary.titles_linked += batch_summary.titles_linked;
            summary.title_urls_updated += batch_summary.title_urls_updated;
            summary.episode_urls_updated += batch_summary.episode_urls_updated;
            summary.missing_artwork_results += batch_summary.missing_artwork_results;

            info!(
                titles_scanned = summary.titles_scanned,
                title_urls_updated = summary.title_urls_updated,
                episode_urls_updated = summary.episode_urls_updated,
                "title image cache refresh rehydrated artwork url batch"
            );
        }

        Ok(summary)
    }

    async fn rehydrate_remote_artwork_urls_for_title_batch(
        &self,
        titles: &[Title],
        language: &str,
    ) -> AppResult<TitleImageCacheRefreshSummary> {
        let mut summary = TitleImageCacheRefreshSummary {
            titles_scanned: titles.len() as u64,
            ..Default::default()
        };
        let mut movie_ids = Vec::new();
        let mut series_ids = Vec::new();
        let mut movie_title_by_tvdb = HashMap::<i64, &Title>::new();
        let mut series_title_by_tvdb = HashMap::<i64, &Title>::new();

        for title in titles {
            let Some(tvdb_id) = title_tvdb_id(title) else {
                continue;
            };
            summary.titles_linked += 1;
            match title.facet {
                MediaFacet::Movie => {
                    movie_ids.push(tvdb_id);
                    movie_title_by_tvdb.insert(tvdb_id, title);
                }
                MediaFacet::Series | MediaFacet::Anime => {
                    series_ids.push(tvdb_id);
                    series_title_by_tvdb.insert(tvdb_id, title);
                }
            }
        }

        if movie_ids.is_empty() && series_ids.is_empty() {
            return Ok(summary);
        }

        let artwork = self
            .services
            .library
            .metadata_gateway
            .get_artwork_urls_bulk(&movie_ids, &series_ids, language)
            .await?;
        let mut title_updates = Vec::new();
        let mut episode_updates = Vec::new();

        for tvdb_id in movie_ids {
            let Some(title) = movie_title_by_tvdb.get(&tvdb_id) else {
                continue;
            };
            let Some(urls) = artwork.movies.get(&tvdb_id) else {
                summary.missing_artwork_results += 1;
                continue;
            };
            if let Some(update) = title_artwork_update(title, urls.poster_url.as_ref(), urls.background_url.as_ref()) {
                title_updates.push(update);
            }
        }

        for tvdb_id in series_ids {
            let Some(title) = series_title_by_tvdb.get(&tvdb_id) else {
                continue;
            };
            let Some(urls) = artwork.series.get(&tvdb_id) else {
                summary.missing_artwork_results += 1;
                continue;
            };
            if let Some(update) = title_artwork_update(title, urls.poster_url.as_ref(), urls.background_url.as_ref()) {
                title_updates.push(update);
            }

            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await?;
            let mut episode_by_tvdb = HashMap::<i64, &scryer_domain::Episode>::new();
            let mut episode_by_numbers = HashMap::<(String, String), &scryer_domain::Episode>::new();
            for episode in &episodes {
                if let Some(tvdb_id) = episode
                    .tvdb_id
                    .as_deref()
                    .and_then(|value| value.trim().parse::<i64>().ok())
                {
                    episode_by_tvdb.insert(tvdb_id, episode);
                }
                if let (Some(season), Some(number)) = (
                    episode.season_number.as_deref(),
                    episode.episode_number.as_deref(),
                ) {
                    episode_by_numbers.insert((season.to_string(), number.to_string()), episode);
                }
            }

            for incoming in &urls.episodes {
                let existing = episode_by_tvdb
                    .get(&incoming.tvdb_id)
                    .copied()
                    .or_else(|| {
                        episode_by_numbers
                            .get(&(
                                incoming.season_number.to_string(),
                                incoming.episode_number.to_string(),
                            ))
                            .copied()
                    });
                let Some(existing) = existing else {
                    summary.missing_artwork_results += 1;
                    continue;
                };
                let Some(image_url) = incoming.image_url.as_ref() else {
                    continue;
                };
                if existing.image_url.as_deref() != Some(image_url.as_str()) {
                    episode_updates.push(EpisodeImageUrlUpdate {
                        episode_id: existing.id.clone(),
                        image_url: Some(image_url.clone()),
                    });
                }
            }
        }

        summary.title_urls_updated = self
            .services
            .catalog
            .titles
            .update_title_artwork_urls(&title_updates)
            .await?;
        summary.episode_urls_updated = self
            .services
            .catalog
            .shows
            .update_episode_image_urls(&episode_updates)
            .await?;
        Ok(summary)
    }
}

fn title_tvdb_id(title: &Title) -> Option<i64> {
    title
        .external_ids
        .iter()
        .find(|external_id| external_id.source.trim().eq_ignore_ascii_case("tvdb"))
        .and_then(|external_id| external_id.value.trim().parse::<i64>().ok())
}

fn title_artwork_update(
    title: &Title,
    incoming_poster_url: Option<&String>,
    incoming_background_url: Option<&String>,
) -> Option<TitleArtworkUrlUpdate> {
    let current_poster = title
        .poster_source_url
        .as_ref()
        .or(title.poster_url.as_ref())
        .cloned();
    let current_background = title
        .background_source_url
        .as_ref()
        .or(title.background_url.as_ref())
        .cloned();
    let next_poster = incoming_poster_url.cloned().or(current_poster.clone());
    let next_background = incoming_background_url.cloned().or(current_background.clone());

    if next_poster == current_poster && next_background == current_background {
        return None;
    }

    Some(TitleArtworkUrlUpdate {
        title_id: title.id.clone(),
        poster_url: next_poster,
        background_url: next_background,
    })
}
