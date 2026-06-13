use crate::{FacetRegistry, WantedItem};
use scryer_domain::{Episode, EpisodeType, ExternalId, Title};

pub(crate) struct SearchQueryResult {
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
    pub(crate) mal_id: Option<String>,
    pub(crate) category: String,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
}

pub(crate) fn build_search_queries(
    title: &Title,
    item: &WantedItem,
    episode: Option<&Episode>,
    facet_registry: &FacetRegistry,
) -> SearchQueryResult {
    let imdb_id = imdb_id_from_title(title);
    let tmdb_id = tmdb_id_from_external_ids(&title.external_ids);
    let tvdb_id = tvdb_id_from_external_ids(&title.external_ids);
    let anidb_id = anidb_id_from_external_ids(&title.external_ids);
    let mal_id = mal_id_from_external_ids(&title.external_ids);

    let category = facet_registry
        .get(&title.facet)
        .map(|handler| handler.search_category().to_string())
        .unwrap_or_else(|| "series".to_string());

    match item.media_type.as_str() {
        "movie" | "series_movie" => build_movie_search_queries(title, &item.media_type, category),
        "episode" => {
            let mut queries = Vec::new();
            let mut season_param: Option<u32> = None;
            let mut episode_param: Option<u32> = None;

            if let Some(episode) = episode {
                let season_num: usize = episode
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                let episode_num: usize = episode
                    .episode_number
                    .as_deref()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);

                if season_num > 0 {
                    season_param = Some(season_num as u32);
                }
                if episode_num > 0 {
                    episode_param = Some(episode_num as u32);
                }

                if season_num > 0 && episode_num > 0 {
                    queries.push(format!(
                        "{} S{:0>2}E{:0>2}",
                        title.name, season_num, episode_num
                    ));
                    queries.push(format!("{} S{:0>2}", title.name, season_num));
                }

                if season_num == 0 && title.facet == scryer_domain::MediaFacet::Anime {
                    if let Some(label) = episode
                        .episode_label
                        .as_deref()
                        .filter(|label| !label.is_empty())
                    {
                        queries.push(format!("{} {}", title.name, label));
                    }
                    if episode_num > 0 {
                        if episode.episode_type == EpisodeType::Ova {
                            queries.push(format!("{} OVA {:0>2}", title.name, episode_num));
                        } else {
                            queries.push(format!("{} Special {:0>2}", title.name, episode_num));
                        }
                    }
                }

                if title.facet == scryer_domain::MediaFacet::Anime
                    && let Some(absolute) = episode
                        .absolute_number
                        .as_deref()
                        .and_then(|value| value.parse::<usize>().ok())
                        .filter(|&value| value > 0 && value != episode_num)
                {
                    queries.insert(0, format!("{} {:0>3}", title.name, absolute));
                }

                if title.facet == scryer_domain::MediaFacet::Anime && !title.name.is_empty() {
                    queries.push(title.name.clone());
                }

                if !queries.is_empty() {
                    let mut seen = std::collections::HashSet::new();
                    queries.retain(|query| seen.insert(query.to_ascii_lowercase()));
                }
            }

            if queries.is_empty() {
                queries.push(title.name.clone());
            }

            SearchQueryResult {
                queries,
                imdb_id,
                tmdb_id,
                tvdb_id,
                anidb_id,
                mal_id,
                category,
                season: season_param,
                episode: episode_param,
            }
        }
        _ => SearchQueryResult {
            queries: vec![],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category,
            season: None,
            episode: None,
        },
    }
}

pub(crate) fn build_movie_search_queries(
    title: &Title,
    media_type: &str,
    category: String,
) -> SearchQueryResult {
    let imdb_id = imdb_id_from_title(title);
    let tmdb_id = tmdb_id_from_external_ids(&title.external_ids);
    let tvdb_id = tvdb_id_from_external_ids(&title.external_ids);
    let anidb_id = anidb_id_from_external_ids(&title.external_ids);
    let mal_id = mal_id_from_external_ids(&title.external_ids);
    let mut queries = Vec::new();
    let query_title = if media_type == "series_movie" {
        series_movie_query_title(title)
    } else {
        title.name.trim().to_string()
    };
    if !query_title.is_empty() {
        let query = if let Some(year) = title.year {
            format!("{query_title} {year}")
        } else {
            query_title
        };
        queries.push(query);
    }
    let mut seen = std::collections::HashSet::new();
    queries.retain(|query| seen.insert(query.to_ascii_lowercase()));
    if queries.is_empty() && imdb_id.is_some() {
        queries.push(String::new());
    }
    SearchQueryResult {
        queries,
        imdb_id,
        tmdb_id,
        tvdb_id,
        anidb_id,
        mal_id,
        category,
        season: None,
        episode: None,
    }
}

fn series_movie_query_title(title: &Title) -> String {
    let terminal_token = crate::title_matching::reduced_comparison_key(
        &title.name,
        crate::title_matching::TitleMatchProfile::Movie,
    )
    .split_whitespace()
    .last()
    .map(str::to_string);
    let candidates = title
        .aliases
        .iter()
        .filter_map(|alias| {
            let key = crate::title_matching::canonical_lookup_key(alias);
            crate::title_matching::has_usable_reduced_key(&key).then_some((alias.clone(), key))
        })
        .collect::<Vec<_>>();
    let preferred_candidates = candidates
        .iter()
        .filter(|(_, key)| {
            terminal_token
                .as_deref()
                .is_none_or(|tail| key.split_whitespace().any(|word| word == tail))
        })
        .collect::<Vec<_>>();
    let candidate_pool = if preferred_candidates.is_empty() {
        candidates.iter().collect::<Vec<_>>()
    } else {
        preferred_candidates
    };

    candidate_pool
        .iter()
        .filter(|(_, key)| key.split_whitespace().count() >= 3)
        .min_by_key(|(_, key)| key.split_whitespace().count())
        .or_else(|| {
            candidate_pool
                .iter()
                .min_by_key(|(_, key)| key.split_whitespace().count())
        })
        .map(|(alias, _)| alias.trim().to_string())
        .filter(|alias| !alias.is_empty())
        .unwrap_or_else(|| title.name.trim().to_string())
}

pub(crate) fn tmdb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("tmdb"))
        .map(|id| id.value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn tvdb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("tvdb"))
        .map(|id| id.value.clone())
}

pub(crate) fn anidb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("anidb"))
        .map(|id| id.value.clone())
}

pub(crate) fn mal_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("mal"))
        .map(|id| id.value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn imdb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("imdb"))
        .and_then(|id| crate::normalize::normalize_imdb_id(&id.value))
}

pub(crate) fn imdb_id_from_title(title: &Title) -> Option<String> {
    title
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
        .or_else(|| imdb_id_from_external_ids(&title.external_ids))
}
