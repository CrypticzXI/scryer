use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseCoverage {
    SingleEpisode(String),
    EpisodeSet(Vec<String>),
    Collection(String),
    Title,
    Unknown,
}

impl ReleaseCoverage {
    pub(crate) fn submission_scope(&self) -> SubmissionScope {
        match self {
            Self::SingleEpisode(episode_id) => SubmissionScope::Episode {
                episode_id: episode_id.clone(),
            },
            Self::EpisodeSet(episode_ids) => SubmissionScope::EpisodeSet {
                episode_ids: episode_ids.clone(),
            },
            Self::Collection(collection_id) => SubmissionScope::Collection {
                collection_id: collection_id.clone(),
            },
            Self::Title => SubmissionScope::Title,
            Self::Unknown => SubmissionScope::Title,
        }
    }

    pub(crate) fn submission_scope_or(&self, fallback: &SubmissionScope) -> SubmissionScope {
        match self {
            Self::Title | Self::Unknown => fallback.clone(),
            _ => self.submission_scope(),
        }
    }

    pub(crate) fn covers_episode(&self, episode: &Episode) -> bool {
        match self {
            Self::SingleEpisode(episode_id) => episode_id == &episode.id,
            Self::EpisodeSet(episode_ids) => episode_ids.iter().any(|id| id == &episode.id),
            Self::Collection(collection_id) => {
                episode.collection_id.as_deref() == Some(collection_id)
            }
            Self::Title => false,
            Self::Unknown => false,
        }
    }

    pub(crate) fn single_episode_preference_penalty(
        &self,
        requested_episode: Option<&Episode>,
    ) -> i32 {
        let Some(episode) = requested_episode else {
            return 0;
        };
        match self {
            Self::SingleEpisode(episode_id) if episode_id == &episode.id => 0,
            Self::EpisodeSet(episode_ids) if episode_ids.iter().any(|id| id == &episode.id) => -6,
            Self::Collection(collection_id)
                if episode.collection_id.as_deref() == Some(collection_id.as_str()) =>
            {
                -12
            }
            _ => 0,
        }
    }
}

pub(crate) fn resolve_release_coverage(
    parsed: &ParsedReleaseMetadata,
    episodes: &[Episode],
    collections: &[Collection],
    requested_episode: Option<&Episode>,
) -> ReleaseCoverage {
    let Some(episode) = parsed.episode.as_ref() else {
        return ReleaseCoverage::Title;
    };

    if episode.release_type == ParsedEpisodeReleaseType::SeasonPack {
        if let Some(season) = episode.season {
            if let Some(collection_id) = collection_id_for_season(collections, season) {
                return ReleaseCoverage::Collection(collection_id);
            }
            if let Some(requested) = requested_episode
                && requested
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                    == Some(season)
                && let Some(collection_id) = requested.collection_id.clone()
            {
                return ReleaseCoverage::Collection(collection_id);
            }
        }
        return requested_episode
            .and_then(|episode| episode.collection_id.clone())
            .map(ReleaseCoverage::Collection)
            .unwrap_or(ReleaseCoverage::Unknown);
    }

    let mut covered = Vec::new();
    if let Some(season) = episode.season
        && !episode.episode_numbers.is_empty()
    {
        let wanted = episode
            .episode_numbers
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        for catalog_episode in episodes {
            let catalog_season = catalog_episode
                .season_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok());
            let catalog_number = catalog_episode
                .episode_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok());
            if catalog_season == Some(season)
                && catalog_number.is_some_and(|number| wanted.contains(&number))
            {
                covered.push(catalog_episode.id.clone());
            }
        }
    }

    if covered.is_empty() && !episode.absolute_episode_numbers.is_empty() {
        let wanted = episode
            .absolute_episode_numbers
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        for catalog_episode in episodes {
            let absolute = catalog_episode
                .absolute_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok());
            if absolute.is_some_and(|number| wanted.contains(&number)) {
                covered.push(catalog_episode.id.clone());
            }
        }
    }

    if covered.is_empty()
        && let Some(absolute_episode) = episode.absolute_episode
    {
        for catalog_episode in episodes {
            let absolute = catalog_episode
                .absolute_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok());
            if absolute == Some(absolute_episode) {
                covered.push(catalog_episode.id.clone());
            }
        }
    }

    coverage_from_episode_ids(covered).unwrap_or(ReleaseCoverage::Unknown)
}

pub(crate) fn coverage_runtime_minutes(
    coverage: &ReleaseCoverage,
    parsed: &ParsedReleaseMetadata,
    episodes: &[Episode],
    default_runtime_minutes: Option<i32>,
) -> Option<i32> {
    match coverage {
        ReleaseCoverage::SingleEpisode(episode_id) => {
            episode_runtime_minutes(episodes, episode_id).or(default_runtime_minutes)
        }
        ReleaseCoverage::EpisodeSet(episode_ids) => {
            let mut total = 0i32;
            let mut missing = 0i32;
            for episode_id in episode_ids {
                if let Some(runtime) = episode_runtime_minutes(episodes, episode_id) {
                    total += runtime;
                } else {
                    missing += 1;
                }
            }
            if missing > 0 {
                total += default_runtime_minutes.unwrap_or(45) * missing;
            }
            (total > 0).then_some(total)
        }
        ReleaseCoverage::Collection(collection_id) => {
            let season_episodes = episodes
                .iter()
                .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
                .collect::<Vec<_>>();
            if season_episodes.is_empty() {
                return default_runtime_minutes;
            }
            let count = i32::try_from(season_episodes.len()).unwrap_or(0);
            if parsed
                .episode
                .as_ref()
                .is_some_and(|episode| episode.is_partial_season)
            {
                return Some(default_runtime_minutes.unwrap_or(45) * (count.max(2) / 2).max(1));
            }
            let total = season_episodes
                .iter()
                .map(|episode| {
                    episode
                        .duration_seconds
                        .map(|seconds| (seconds / 60) as i32)
                })
                .map(|runtime| runtime.unwrap_or(default_runtime_minutes.unwrap_or(45)))
                .sum::<i32>();
            (total > 0).then_some(total)
        }
        ReleaseCoverage::Title | ReleaseCoverage::Unknown => default_runtime_minutes,
    }
}

fn coverage_from_episode_ids(mut episode_ids: Vec<String>) -> Option<ReleaseCoverage> {
    episode_ids.retain(|episode_id| !episode_id.trim().is_empty());
    episode_ids.sort();
    episode_ids.dedup();
    match episode_ids.len() {
        0 => None,
        1 => episode_ids
            .into_iter()
            .next()
            .map(ReleaseCoverage::SingleEpisode),
        _ => Some(ReleaseCoverage::EpisodeSet(episode_ids)),
    }
}

fn collection_id_for_season(collections: &[Collection], season: u32) -> Option<String> {
    collections
        .iter()
        .find(|collection| collection.collection_index.trim().parse::<u32>().ok() == Some(season))
        .map(|collection| collection.id.clone())
}

fn episode_runtime_minutes(episodes: &[Episode], episode_id: &str) -> Option<i32> {
    episodes
        .iter()
        .find(|episode| episode.id == episode_id)
        .and_then(|episode| episode.duration_seconds)
        .map(|seconds| (seconds / 60) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{CollectionType, EpisodeType};

    fn episode(id: &str, season: &str, number: &str, absolute: Option<&str>) -> Episode {
        Episode {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            collection_id: Some(format!("season-{season}")),
            episode_type: EpisodeType::Standard,
            episode_number: Some(number.to_string()),
            season_number: Some(season.to_string()),
            episode_label: None,
            title: None,
            air_date: None,
            duration_seconds: Some(1_500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: absolute.map(str::to_string),
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn collection(id: &str, index: &str) -> Collection {
        Collection {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            collection_type: CollectionType::Season,
            collection_index: index.to_string(),
            label: None,
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: Vec::new(),
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn parsed_with_episode(episode: ParsedEpisodeMetadata) -> ParsedReleaseMetadata {
        let mut parsed = ParsedReleaseMetadata::empty("release", "test");
        parsed.episode = Some(episode);
        parsed
    }

    #[test]
    fn absolute_range_resolves_to_episode_set_scope() {
        let episodes = vec![
            episode("ep-14", "1", "14", Some("14")),
            episode("ep-15", "1", "15", Some("15")),
            episode("ep-16", "1", "16", Some("16")),
        ];
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            absolute_episode: Some(14),
            absolute_episode_numbers: vec![14, 15, 16],
            release_type: ParsedEpisodeReleaseType::RangePack,
            ..Default::default()
        });

        let coverage = resolve_release_coverage(&parsed, &episodes, &[], None);

        assert_eq!(
            coverage,
            ReleaseCoverage::EpisodeSet(vec![
                "ep-14".to_string(),
                "ep-15".to_string(),
                "ep-16".to_string()
            ])
        );
        assert_eq!(
            coverage.submission_scope(),
            SubmissionScope::EpisodeSet {
                episode_ids: vec![
                    "ep-14".to_string(),
                    "ep-15".to_string(),
                    "ep-16".to_string()
                ]
            }
        );
    }

    #[test]
    fn season_pack_resolves_to_collection_scope() {
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            season: Some(1),
            full_season: true,
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            ..Default::default()
        });

        let coverage = resolve_release_coverage(&parsed, &[], &[collection("season-1", "1")], None);

        assert_eq!(
            coverage,
            ReleaseCoverage::Collection("season-1".to_string())
        );
        assert_eq!(
            coverage.submission_scope(),
            SubmissionScope::Collection {
                collection_id: "season-1".to_string()
            }
        );
    }

    #[test]
    fn explicit_range_runtime_uses_covered_episode_total() {
        let episodes = vec![
            episode("ep-14", "1", "14", Some("14")),
            episode("ep-15", "1", "15", Some("15")),
        ];
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            absolute_episode: Some(14),
            absolute_episode_numbers: vec![14, 15],
            release_type: ParsedEpisodeReleaseType::RangePack,
            ..Default::default()
        });
        let coverage = resolve_release_coverage(&parsed, &episodes, &[], None);

        assert_eq!(
            coverage_runtime_minutes(&coverage, &parsed, &episodes, Some(45)),
            Some(50)
        );
    }

    #[test]
    fn title_only_coverage_does_not_cover_requested_episode() {
        let episode = episode("ep-1", "1", "1", Some("1"));

        assert!(!ReleaseCoverage::Title.covers_episode(&episode));
        assert!(!ReleaseCoverage::Unknown.covers_episode(&episode));
    }

    #[test]
    fn unresolved_coverage_uses_requested_scope_instead_of_widening_to_title() {
        let fallback = SubmissionScope::Episode {
            episode_id: "ep-1".to_string(),
        };

        assert_eq!(
            ReleaseCoverage::Unknown.submission_scope_or(&fallback),
            fallback
        );
    }
}
