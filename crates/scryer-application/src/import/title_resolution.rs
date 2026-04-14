use crate::ParsedReleaseMetadata;
use scryer_domain::{MediaFacet, Title};

pub(crate) fn find_monitored_movie_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<Title> {
    let monitored_movies = titles
        .iter()
        .filter(|title| title.monitored && title.facet == MediaFacet::Movie)
        .collect::<Vec<_>>();

    find_title_by_external_ids(&monitored_movies, parsed)
        .or_else(|| find_movie_title_by_name(&monitored_movies, parsed))
        .cloned()
}

pub(crate) fn find_monitored_episode_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<Title> {
    let monitored_episodes = titles
        .iter()
        .filter(|title| {
            title.monitored && episodic_facet_matches_hint(title.facet.clone(), facet_hint)
        })
        .collect::<Vec<_>>();

    find_unique_title_by_external_ids(&monitored_episodes, parsed)
        .or_else(|| find_unique_title_by_name(&monitored_episodes, parsed))
        .cloned()
}

pub(crate) fn normalize_imdb_id(raw_imdb_id: &str) -> Option<String> {
    crate::normalize::normalize_imdb_id(raw_imdb_id)
}

fn episodic_facet_matches_hint(facet: MediaFacet, facet_hint: Option<&str>) -> bool {
    match facet_hint.map(|value| value.trim().to_ascii_lowercase()) {
        Some(hint) if hint == "anime" => facet == MediaFacet::Anime,
        Some(hint) if hint == "series" || hint == "series" => facet == MediaFacet::Series,
        _ => matches!(facet, MediaFacet::Series | MediaFacet::Anime),
    }
}

fn normalized_release_title_candidates(parsed: &ParsedReleaseMetadata) -> Vec<String> {
    let raw_candidates = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    raw_candidates
        .into_iter()
        .map(|title| crate::app_usecase_rss::normalize_for_matching(&title))
        .filter(|title| !title.is_empty())
        .fold(Vec::<String>::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == &value) {
                acc.push(value);
            }
            acc
        })
}

fn title_matches_normalized_candidate(title: &Title, candidate: &str) -> bool {
    if crate::app_usecase_rss::normalize_for_matching(&title.name) == candidate {
        return true;
    }

    title
        .aliases
        .iter()
        .any(|alias| crate::app_usecase_rss::normalize_for_matching(alias) == candidate)
}

fn find_title_by_external_ids<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<&'a Title> {
    if let Some(parsed_imdb_id) = parsed.imdb_id.as_deref().and_then(normalize_imdb_id) {
        let mut matches = titles
            .iter()
            .copied()
            .filter(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case("imdb")
                        && normalize_imdb_id(&external_id.value).as_deref()
                            == Some(parsed_imdb_id.as_str())
                })
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.pop();
        }
    }

    if let Some(parsed_tmdb_id) = parsed.tmdb_id.map(|id| id.to_string()) {
        let mut matches = titles
            .iter()
            .copied()
            .filter(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case("tmdb")
                        && external_id.value.trim() == parsed_tmdb_id
                })
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.pop();
        }
    }

    None
}

fn find_unique_title_by_external_ids<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<&'a Title> {
    let matches = titles
        .iter()
        .copied()
        .filter(|title| {
            parsed
                .imdb_id
                .as_deref()
                .and_then(normalize_imdb_id)
                .is_some_and(|parsed_imdb_id| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case("imdb")
                            && normalize_imdb_id(&external_id.value).as_deref()
                                == Some(parsed_imdb_id.as_str())
                    })
                })
                || parsed
                    .tmdb_id
                    .map(|id| id.to_string())
                    .is_some_and(|parsed_tmdb_id| {
                        title.external_ids.iter().any(|external_id| {
                            external_id.source.eq_ignore_ascii_case("tmdb")
                                && external_id.value.trim() == parsed_tmdb_id
                        })
                    })
        })
        .collect::<Vec<_>>();

    (matches.len() == 1).then(|| matches[0])
}

fn find_movie_title_by_name<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<&'a Title> {
    let candidates = normalized_release_title_candidates(parsed);
    if candidates.is_empty() {
        return None;
    }

    let mut year_matches = Vec::<&Title>::new();
    let mut any_matches = Vec::<&Title>::new();

    for candidate in candidates {
        for title in titles {
            if !title_matches_normalized_candidate(title, &candidate) {
                continue;
            }

            if !any_matches.iter().any(|existing| existing.id == title.id) {
                any_matches.push(*title);
            }

            if let Some(year) = parsed.year
                && title.year.map(|value| value as u32) == Some(year)
                && !year_matches.iter().any(|existing| existing.id == title.id)
            {
                year_matches.push(*title);
            }
        }
    }

    if year_matches.len() == 1 {
        return year_matches.into_iter().next();
    }

    if any_matches.len() == 1 {
        return any_matches.into_iter().next();
    }

    year_matches
        .into_iter()
        .next()
        .or_else(|| any_matches.into_iter().next())
}

fn find_unique_title_by_name<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<&'a Title> {
    let candidates = normalized_release_title_candidates(parsed);
    if candidates.is_empty() {
        return None;
    }

    let mut year_matches = Vec::<&Title>::new();
    let mut any_matches = Vec::<&Title>::new();

    for candidate in candidates {
        for title in titles {
            if !title_matches_normalized_candidate(title, &candidate) {
                continue;
            }

            if !any_matches.iter().any(|existing| existing.id == title.id) {
                any_matches.push(*title);
            }

            if let Some(year) = parsed.year
                && title.year.map(|value| value as u32) == Some(year)
                && !year_matches.iter().any(|existing| existing.id == title.id)
            {
                year_matches.push(*title);
            }
        }
    }

    if year_matches.len() == 1 {
        return year_matches.into_iter().next();
    }

    (any_matches.len() == 1).then(|| any_matches[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{ExternalId, Id};

    fn test_title(name: &str, facet: MediaFacet, year: Option<i32>, aliases: &[&str]) -> Title {
        Title {
            id: Id::new().0,
            name: name.to_string(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
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
            aliases: aliases.iter().map(|value| value.to_string()).collect(),
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    #[test]
    fn finds_unique_episodic_title_from_release_name() {
        let titles = vec![test_title(
            "YATAGARASU The Raven Does Not Choose Its Master",
            MediaFacet::Anime,
            None,
            &[],
        )];
        let parsed = crate::parse_release_metadata(
            "YATAGARASU.The.Raven.Does.Not.Choose.Its.Master.S01E18.1080p.WEB-DL",
        );

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .expect("matched title");

        assert_eq!(matched.name, titles[0].name);
    }

    #[test]
    fn finds_episodic_title_by_alias() {
        let titles = vec![test_title(
            "Karasu wa Aruji wo Erabanai",
            MediaFacet::Anime,
            None,
            &["YATAGARASU The Raven Does Not Choose Its Master"],
        )];
        let parsed = crate::parse_release_metadata(
            "YATAGARASU.The.Raven.Does.Not.Choose.Its.Master.S01E18.1080p.WEB-DL",
        );

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .expect("matched title by alias");

        assert_eq!(matched.id, titles[0].id);
    }

    #[test]
    fn does_not_match_ambiguous_episodic_titles() {
        let titles = vec![
            test_title("Outlander", MediaFacet::Series, Some(2014), &[]),
            test_title("Outlander", MediaFacet::Anime, Some(2000), &[]),
        ];
        let parsed = crate::parse_release_metadata("Outlander.S08E05.1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, None);

        assert!(matched.is_none());
    }

    #[test]
    fn matches_unique_episodic_title_by_imdb_id() {
        let mut title = test_title("Completely Different Name", MediaFacet::Series, None, &[]);
        title.external_ids.push(ExternalId {
            source: "imdb".to_string(),
            value: "tt0944947".to_string(),
        });
        let titles = vec![title.clone()];
        let parsed = crate::parse_release_metadata("Outlander.S08E05.[tt0944947].1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("series"))
            .expect("matched title by imdb id");

        assert_eq!(matched.id, title.id);
    }
}
