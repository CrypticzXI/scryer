use crate::{FacetRegistry, WantedItem};
use scryer_domain::{Episode, EpisodeType, ExternalId, Title};

pub(crate) struct SearchQueryResult {
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
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

    let category = facet_registry
        .get(&title.facet)
        .map(|handler| handler.search_category().to_string())
        .unwrap_or_else(|| "series".to_string());

    match item.media_type.as_str() {
        "movie" => {
            let mut queries = Vec::new();
            if !title.name.is_empty() {
                let query = if let Some(year) = title.year {
                    format!("{} {}", title.name, year)
                } else {
                    title.name.clone()
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
                category,
                season: None,
                episode: None,
            }
        }
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
                category,
                season: season_param,
                episode: episode_param,
            }
        }
        "interstitial_movie" => {
            let mut queries = Vec::new();
            if !title.name.is_empty() {
                let query = if let Some(year) = title.year {
                    format!("{} {}", title.name, year)
                } else {
                    title.name.clone()
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
                category: "movies".to_string(),
                season: None,
                episode: None,
            }
        }
        _ => SearchQueryResult {
            queries: vec![],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            category,
            season: None,
            episode: None,
        },
    }
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
