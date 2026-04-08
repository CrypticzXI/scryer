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

    find_movie_title_by_external_ids(&monitored_movies, parsed)
        .or_else(|| find_movie_title_by_name(&monitored_movies, parsed))
        .cloned()
}

pub(crate) fn normalize_imdb_id(raw_imdb_id: &str) -> Option<String> {
    crate::normalize::normalize_imdb_id(raw_imdb_id)
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

fn find_movie_title_by_external_ids<'a>(
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
