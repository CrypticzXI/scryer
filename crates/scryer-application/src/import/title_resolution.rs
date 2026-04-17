use crate::ParsedReleaseMetadata;
use scryer_domain::{MediaFacet, Title, TitleMatchType};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(crate) struct ResolvedMonitoredTitle<'a> {
    pub title: &'a Title,
    pub match_type: TitleMatchType,
}

#[derive(Clone, Default)]
pub(crate) struct MonitoredTitleMatcherCache {
    pub matcher: Option<Arc<MonitoredTitleMatcher>>,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MonitoredTitleMatcher {
    titles: Vec<Title>,
    normalized_title_index: HashMap<String, Vec<usize>>,
    imdb_index: HashMap<String, Vec<usize>>,
    tmdb_index: HashMap<String, Vec<usize>>,
}

impl MonitoredTitleMatcher {
    pub(crate) fn new(titles: Vec<Title>) -> Self {
        let mut matcher = Self::default();

        for title in titles.into_iter().filter(|title| title.monitored) {
            let index = matcher.titles.len();

            for candidate in
                std::iter::once(title.name.as_str()).chain(title.aliases.iter().map(String::as_str))
            {
                let normalized = crate::app_usecase_rss::normalize_for_matching(candidate);
                if normalized.is_empty() {
                    continue;
                }
                matcher
                    .normalized_title_index
                    .entry(normalized)
                    .or_default()
                    .push(index);
            }

            for external_id in &title.external_ids {
                if external_id.source.eq_ignore_ascii_case("imdb") {
                    if let Some(imdb_id) = normalize_imdb_id(&external_id.value) {
                        matcher.imdb_index.entry(imdb_id).or_default().push(index);
                    }
                } else if external_id.source.eq_ignore_ascii_case("tmdb") {
                    let tmdb_id = external_id.value.trim();
                    if !tmdb_id.is_empty() {
                        matcher
                            .tmdb_index
                            .entry(tmdb_id.to_string())
                            .or_default()
                            .push(index);
                    }
                }
            }

            matcher.titles.push(title);
        }

        matcher
    }

    pub(crate) fn resolve_movie<'a>(
        &'a self,
        parsed: &ParsedReleaseMetadata,
    ) -> Option<ResolvedMonitoredTitle<'a>> {
        parsed
            .imdb_id
            .as_deref()
            .and_then(normalize_imdb_id)
            .and_then(|imdb_id| {
                lookup_unique_title(
                    self.imdb_index.get(&imdb_id).map(Vec::as_slice),
                    &self.titles,
                    |title| title.facet == MediaFacet::Movie,
                )
            })
            .map(|title| ResolvedMonitoredTitle {
                title,
                match_type: TitleMatchType::IdOnly,
            })
            .or_else(|| {
                parsed
                    .tmdb_id
                    .map(|id| id.to_string())
                    .and_then(|tmdb_id| {
                        lookup_unique_title(
                            self.tmdb_index.get(&tmdb_id).map(Vec::as_slice),
                            &self.titles,
                            |title| title.facet == MediaFacet::Movie,
                        )
                    })
                    .map(|title| ResolvedMonitoredTitle {
                        title,
                        match_type: TitleMatchType::IdOnly,
                    })
            })
            .or_else(|| {
                let (year_matches, any_matches) =
                    self.collect_name_matches(parsed, |title| title.facet == MediaFacet::Movie);

                if year_matches.len() == 1 {
                    return Some(ResolvedMonitoredTitle {
                        title: year_matches[0],
                        match_type: TitleMatchType::TitleParse,
                    });
                }

                if any_matches.len() == 1 {
                    return Some(ResolvedMonitoredTitle {
                        title: any_matches[0],
                        match_type: TitleMatchType::TitleParse,
                    });
                }

                year_matches
                    .into_iter()
                    .next()
                    .or_else(|| any_matches.into_iter().next())
                    .map(|title| ResolvedMonitoredTitle {
                        title,
                        match_type: TitleMatchType::TitleParse,
                    })
            })
    }

    pub(crate) fn resolve_episode<'a>(
        &'a self,
        parsed: &ParsedReleaseMetadata,
        facet_hint: Option<&str>,
    ) -> Option<ResolvedMonitoredTitle<'a>> {
        let mut external_matches = HashSet::new();

        if let Some(imdb_id) = parsed.imdb_id.as_deref().and_then(normalize_imdb_id) {
            for index in self.imdb_index.get(&imdb_id).into_iter().flatten().copied() {
                if self.titles.get(index).is_some_and(|title| {
                    episodic_facet_matches_hint(title.facet.clone(), facet_hint)
                }) {
                    external_matches.insert(index);
                }
            }
        }

        if let Some(tmdb_id) = parsed.tmdb_id.map(|id| id.to_string()) {
            for index in self.tmdb_index.get(&tmdb_id).into_iter().flatten().copied() {
                if self.titles.get(index).is_some_and(|title| {
                    episodic_facet_matches_hint(title.facet.clone(), facet_hint)
                }) {
                    external_matches.insert(index);
                }
            }
        }

        if external_matches.len() == 1
            && let Some(index) = external_matches.into_iter().next()
            && let Some(title) = self.titles.get(index)
        {
            return Some(ResolvedMonitoredTitle {
                title,
                match_type: TitleMatchType::IdOnly,
            });
        }

        let (year_matches, any_matches) = self.collect_name_matches(parsed, |title| {
            episodic_facet_matches_hint(title.facet.clone(), facet_hint)
        });

        if year_matches.len() == 1 {
            return Some(ResolvedMonitoredTitle {
                title: year_matches[0],
                match_type: TitleMatchType::TitleParse,
            });
        }

        (any_matches.len() == 1).then(|| ResolvedMonitoredTitle {
            title: any_matches[0],
            match_type: TitleMatchType::TitleParse,
        })
    }

    fn collect_name_matches<'a, F>(
        &'a self,
        parsed: &ParsedReleaseMetadata,
        filter: F,
    ) -> (Vec<&'a Title>, Vec<&'a Title>)
    where
        F: Fn(&Title) -> bool,
    {
        let candidates = normalized_release_title_candidates(parsed);
        if candidates.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let mut year_matches = Vec::<&Title>::new();
        let mut any_matches = Vec::<&Title>::new();
        let mut seen = HashSet::<usize>::new();
        let mut seen_year = HashSet::<usize>::new();

        for candidate in candidates {
            for index in self
                .normalized_title_index
                .get(&candidate)
                .into_iter()
                .flatten()
                .copied()
            {
                let Some(title) = self.titles.get(index) else {
                    continue;
                };
                if !filter(title) {
                    continue;
                }

                if seen.insert(index) {
                    any_matches.push(title);
                }

                if let Some(year) = parsed.year
                    && title.year.map(|value| value as u32) == Some(year)
                    && seen_year.insert(index)
                {
                    year_matches.push(title);
                }
            }
        }

        (year_matches, any_matches)
    }
}

fn lookup_unique_title<'a, F>(
    indexes: Option<&[usize]>,
    titles: &'a [Title],
    filter: F,
) -> Option<&'a Title>
where
    F: Fn(&Title) -> bool,
{
    let mut matches = indexes
        .into_iter()
        .flatten()
        .copied()
        .filter_map(|index| titles.get(index))
        .filter(|title| filter(title))
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches.pop()).flatten()
}

#[cfg(test)]
pub(crate) fn find_monitored_movie_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<Title> {
    resolve_monitored_movie_title_from_release(titles, parsed)
        .map(|resolved| resolved.title.clone())
}

#[cfg(test)]
pub(crate) fn find_monitored_episode_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<Title> {
    resolve_monitored_episode_title_from_release(titles, parsed, facet_hint)
        .map(|resolved| resolved.title.clone())
}

pub(crate) fn resolve_monitored_movie_title_from_release<'a>(
    titles: &'a [Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<ResolvedMonitoredTitle<'a>> {
    let monitored_movies = titles
        .iter()
        .filter(|title| title.monitored && title.facet == MediaFacet::Movie)
        .collect::<Vec<_>>();

    find_title_by_external_ids(&monitored_movies, parsed)
        .map(|title| ResolvedMonitoredTitle {
            title,
            match_type: TitleMatchType::IdOnly,
        })
        .or_else(|| {
            find_movie_title_by_name(&monitored_movies, parsed).map(|title| {
                ResolvedMonitoredTitle {
                    title,
                    match_type: TitleMatchType::TitleParse,
                }
            })
        })
}

pub(crate) fn resolve_monitored_episode_title_from_release<'a>(
    titles: &'a [Title],
    parsed: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<ResolvedMonitoredTitle<'a>> {
    let monitored_episodes = titles
        .iter()
        .filter(|title| {
            title.monitored && episodic_facet_matches_hint(title.facet.clone(), facet_hint)
        })
        .collect::<Vec<_>>();

    find_unique_title_by_external_ids(&monitored_episodes, parsed)
        .map(|title| ResolvedMonitoredTitle {
            title,
            match_type: TitleMatchType::IdOnly,
        })
        .or_else(|| {
            find_unique_title_by_name(&monitored_episodes, parsed).map(|title| {
                ResolvedMonitoredTitle {
                    title,
                    match_type: TitleMatchType::TitleParse,
                }
            })
        })
}

pub(crate) fn normalize_imdb_id(raw_imdb_id: &str) -> Option<String> {
    crate::normalize::normalize_imdb_id(raw_imdb_id)
}

fn episodic_facet_matches_hint(facet: MediaFacet, facet_hint: Option<&str>) -> bool {
    match facet_hint.map(|value| value.trim().to_ascii_lowercase()) {
        Some(hint) if hint == "anime" => facet == MediaFacet::Anime,
        Some(hint) if matches!(hint.as_str(), "series" | "tv") => facet == MediaFacet::Series,
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
    use scryer_domain::{ExternalId, Id, TitleMatchType};

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

    #[test]
    fn resolve_monitored_episode_title_marks_external_id_matches_as_id_only() {
        let mut title = test_title("Completely Different Name", MediaFacet::Series, None, &[]);
        title.external_ids.push(ExternalId {
            source: "imdb".to_string(),
            value: "tt0944947".to_string(),
        });
        let titles = vec![title.clone()];
        let parsed = crate::parse_release_metadata("Outlander.S08E05.[tt0944947].1080p.WEB-DL");

        let matched =
            resolve_monitored_episode_title_from_release(&titles, &parsed, Some("series"))
                .expect("matched title by imdb id");

        assert_eq!(matched.title.id, title.id);
        assert_eq!(matched.match_type, TitleMatchType::IdOnly);
    }

    #[test]
    fn resolve_monitored_episode_title_marks_name_matches_as_title_parse() {
        let titles = vec![test_title(
            "YATAGARASU The Raven Does Not Choose Its Master",
            MediaFacet::Anime,
            None,
            &[],
        )];
        let parsed = crate::parse_release_metadata(
            "YATAGARASU.The.Raven.Does.Not.Choose.Its.Master.S01E18.1080p.WEB-DL",
        );

        let matched = resolve_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .expect("matched title by name");

        assert_eq!(matched.title.id, titles[0].id);
        assert_eq!(matched.match_type, TitleMatchType::TitleParse);
    }
}
