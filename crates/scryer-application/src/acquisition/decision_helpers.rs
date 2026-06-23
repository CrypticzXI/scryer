use crate::acquisition_search_queries::tvdb_id_from_external_ids;
use crate::{
    AcquisitionThresholds, AppError, AppUseCase, QualityProfile, TitleMediaFile, WantedItem,
    app_usecase_discovery::QualityProfileLookup, default_quality_profile_for_search,
};
use chrono::{DateTime, NaiveDate, Utc};
use scryer_domain::Title;

pub(crate) const FAILED_GRAB_OLD_TITLE_DAYS: i64 = 14;
pub(crate) const FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES: i64 = 20;

pub(crate) fn extract_grabbed_release_title(raw: Option<&str>) -> Option<String> {
    raw.and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| {
            value
                .get("title")
                .and_then(|title| title.as_str())
                .map(str::to_string)
        })
}

/// Returns true if the error indicates all prioritized download clients failed.
pub(crate) fn is_all_clients_failed_error(err: &AppError) -> bool {
    matches!(err, AppError::Repository(msg) if msg.contains("all prioritized download clients failed"))
}

pub(crate) fn is_download_submit_unavailable_error(err: &AppError) -> bool {
    err.is_download_submit_unavailable() || is_all_clients_failed_error(err)
}

pub(crate) fn should_research_failed_grab(item: &WantedItem, now: &DateTime<Utc>) -> bool {
    !is_old_failed_grab_title(item, now)
        && is_last_search_stale(item.last_search_at.as_deref(), now)
}

pub(crate) fn is_old_failed_grab_title(item: &WantedItem, now: &DateTime<Utc>) -> bool {
    let Some(baseline_date) = item.baseline_date.as_deref() else {
        return false;
    };
    let Some(parsed_date) = parse_failed_grab_baseline_date(baseline_date) else {
        return false;
    };
    now.date_naive()
        .signed_duration_since(parsed_date)
        .num_days()
        > FAILED_GRAB_OLD_TITLE_DAYS
}

fn is_last_search_stale(last_search_at: Option<&str>, now: &DateTime<Utc>) -> bool {
    let Some(last_search_at) = last_search_at else {
        return true;
    };
    let Some(last_search_at) = crate::quality_profile::parse_published_at(last_search_at) else {
        return true;
    };
    (*now - last_search_at).num_minutes() > FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES
}

fn parse_failed_grab_baseline_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|value| value.date_naive())
        })
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(raw)
                .ok()
                .map(|value| value.date_naive())
        })
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedUpgradeContext {
    pub(crate) profile: QualityProfile,
    pub(crate) thresholds: AcquisitionThresholds,
    pub(crate) cutoff_reached: bool,
}

pub(crate) fn upgrade_context_category<'a>(
    title: &'a Title,
    category_hint: Option<&'a str>,
) -> &'a str {
    category_hint
        .map(str::trim)
        .filter(|category| !category.is_empty())
        .unwrap_or_else(|| title.facet.as_str())
}

pub(crate) fn analyzed_cutoff_quality_for_scope<'a>(
    existing_files: &'a [TitleMediaFile],
    episode_id: Option<&str>,
    series_movie_link_id: Option<&str>,
) -> Option<&'a str> {
    existing_files
        .iter()
        .filter(|file| file.role.is_primary())
        .filter(|file| media_file_matches_cutoff_scope(file, episode_id, series_movie_link_id))
        .filter(|file| {
            file.quality_label
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        })
        .max_by(|left, right| {
            left.acquisition_score
                .unwrap_or(i32::MIN)
                .cmp(&right.acquisition_score.unwrap_or(i32::MIN))
                .then_with(|| left.created_at.cmp(&right.created_at))
        })
        .and_then(|file| file.quality_label.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn media_file_matches_cutoff_scope(
    file: &TitleMediaFile,
    episode_id: Option<&str>,
    series_movie_link_id: Option<&str>,
) -> bool {
    if let Some(episode_id) = episode_id {
        return file.episode_id.as_deref() == Some(episode_id);
    }
    if let Some(series_movie_link_id) = series_movie_link_id {
        return file
            .series_movie_link_ids
            .iter()
            .any(|link_id| link_id == series_movie_link_id);
    }
    file.episode_id.is_none() && file.series_movie_link_ids.is_empty()
}

impl AppUseCase {
    pub(crate) async fn resolve_upgrade_context_for_title_with_category_and_quality(
        &self,
        title: &Title,
        grabbed_release: Option<&str>,
        category_hint: Option<&str>,
        analyzed_quality: Option<&str>,
    ) -> ResolvedUpgradeContext {
        let category = upgrade_context_category(title, category_hint);
        let grabbed_release = if grabbed_release
            .map(str::trim)
            .is_some_and(|value| value.is_empty())
        {
            None
        } else {
            grabbed_release
        };
        let profile = self
            .resolve_quality_profile(QualityProfileLookup {
                title_tags: &title.tags,
                library_id: Some(title.library_id.as_str()),
                imdb_id: title.imdb_id.as_deref(),
                tvdb_id: tvdb_id_from_external_ids(&title.external_ids).as_deref(),
                category_hint: Some(category),
            })
            .await
            .unwrap_or_else(|_| default_quality_profile_for_search());

        let cutoff_reached = crate::quality_profile::has_reached_cutoff_from_quality_or_release(
            analyzed_quality,
            grabbed_release,
            profile.criteria.cutoff_tier.as_deref(),
            &profile.criteria.quality_tiers,
        );

        let persona = self
            .resolve_scoring_persona(Some(title.library_id.as_str()), Some(category))
            .await
            .unwrap_or_default();
        let thresholds = self.acquisition_thresholds(&persona).await;

        ResolvedUpgradeContext {
            profile,
            thresholds,
            cutoff_reached,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{MediaFacet, Title};

    fn make_title(facet: MediaFacet) -> Title {
        Title {
            id: "title-1".to_string(),
            name: "Example".to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    #[test]
    fn upgrade_context_category_prefers_explicit_hint() {
        let title = make_title(MediaFacet::Movie);
        assert_eq!(upgrade_context_category(&title, Some("anime")), "anime");
    }

    #[test]
    fn upgrade_context_category_falls_back_to_facet_for_blank_hint() {
        let title = make_title(MediaFacet::Series);
        assert_eq!(upgrade_context_category(&title, Some("  ")), "series");
    }
}
