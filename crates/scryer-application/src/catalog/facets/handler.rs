use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{ExternalId, MediaFacet};

use crate::{
    AnimeMapping, AnimeMovie, AppResult, DiscoveryTitle, EpisodeMetadata, MetadataGateway,
    MovieMetadata, SeasonMetadata, SeriesMetadata, TitleMetadataUpdate,
};

/// Result of hydrating a title's metadata from a metadata gateway.
/// Movies return empty seasons/episodes. Series include full season/episode data.
pub struct HydrationResult {
    pub metadata_update: TitleMetadataUpdate,
    pub seasons: Vec<SeasonMetadata>,
    pub episodes: Vec<EpisodeMetadata>,
    pub anime_mappings: Vec<AnimeMapping>,
    pub anime_movies: Vec<AnimeMovie>,
    pub more_like_this: Vec<DiscoveryTitle>,
}

pub(crate) fn external_ids_from_hydration_metadata(
    mut external_ids: Vec<ExternalId>,
    metadata_update: &TitleMetadataUpdate,
) -> Vec<ExternalId> {
    if let Some(imdb_id) = metadata_update
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
    {
        external_ids.push(ExternalId {
            source: "imdb".to_string(),
            value: imdb_id,
        });
    }
    external_ids.extend(metadata_update.extra_external_ids.iter().cloned());
    external_ids
}

#[derive(Clone, Copy)]
pub struct RenameFacetSettings {
    pub scope_id: &'static str,
    pub template_key: &'static str,
    pub collision_policy_key: &'static str,
    pub missing_metadata_policy_key: &'static str,
    pub default_template: &'static str,
}

pub fn rename_facet_settings(facet: &MediaFacet) -> RenameFacetSettings {
    match facet {
        MediaFacet::Movie => RenameFacetSettings {
            scope_id: "movie",
            template_key: "rename.template.movie.global",
            collision_policy_key: "rename.collision_policy.movie.global",
            missing_metadata_policy_key: "rename.missing_metadata_policy.movie.global",
            default_template: "{title} ({year}) - {quality}.{ext}",
        },
        MediaFacet::Series => RenameFacetSettings {
            scope_id: "series",
            template_key: "rename.template.series.global",
            collision_policy_key: "rename.collision_policy.series.global",
            missing_metadata_policy_key: "rename.missing_metadata_policy.series.global",
            default_template: "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
        },
        MediaFacet::Anime => RenameFacetSettings {
            scope_id: "anime",
            template_key: "rename.template.anime.global",
            collision_policy_key: "rename.collision_policy.anime.global",
            missing_metadata_policy_key: "rename.missing_metadata_policy.anime.global",
            default_template: "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}",
        },
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

pub(crate) fn primary_anime_mapping(anime_mappings: &[AnimeMapping]) -> Option<&AnimeMapping> {
    anime_mappings
        .iter()
        .find(|mapping| mapping.mapping_type != "S")
        .or(anime_mappings.first())
}

fn primary_anime_mapping_extra_external_ids(anime_mappings: &[AnimeMapping]) -> Vec<ExternalId> {
    let Some(mapping) = primary_anime_mapping(anime_mappings) else {
        return Vec::new();
    };

    let mut external_ids = Vec::new();
    push_positive_external_id(&mut external_ids, "mal", mapping.mal_id);
    push_positive_external_id(&mut external_ids, "anilist", mapping.anilist_id);
    push_positive_external_id(&mut external_ids, "anidb", mapping.anidb_id);
    push_positive_external_id(&mut external_ids, "kitsu", mapping.kitsu_id);
    push_positive_external_id(&mut external_ids, "simkl", mapping.simkl_id);
    push_positive_external_id(&mut external_ids, "tvdb", mapping.thetvdb_id);
    push_positive_external_id(&mut external_ids, "tmdb", mapping.themoviedb_id);
    push_positive_imdb_external_id(&mut external_ids, mapping.imdb_id);
    push_positive_external_id(&mut external_ids, "trakt", mapping.trakt_id);
    external_ids
}

fn push_positive_external_id(external_ids: &mut Vec<ExternalId>, source: &str, value: Option<i64>) {
    if let Some(value) = value.filter(|value| *value > 0) {
        external_ids.push(ExternalId {
            source: source.to_string(),
            value: value.to_string(),
        });
    }
}

fn push_positive_imdb_external_id(external_ids: &mut Vec<ExternalId>, value: Option<i64>) {
    let Some(imdb_id) = value
        .filter(|value| *value > 0)
        .and_then(|value| crate::normalize::normalize_imdb_id(&value.to_string()))
    else {
        return;
    };
    external_ids.push(ExternalId {
        source: "imdb".to_string(),
        value: imdb_id,
    });
}

/// Build a [`HydrationResult`] from an already-fetched [`MovieMetadata`].
///
/// Shared by the single-title facet handler path and the bulk hydration loop.
pub fn movie_to_hydration_result(movie: MovieMetadata, language: &str) -> HydrationResult {
    let mut extra_external_ids = Vec::new();
    if let Some(imdb_id) = crate::normalize::normalize_imdb_id(movie.imdb_id.as_str()) {
        extra_external_ids.push(scryer_domain::ExternalId {
            source: "imdb".into(),
            value: imdb_id,
        });
    }
    if let Some(anidb_id) = movie.anidb_id {
        extra_external_ids.push(scryer_domain::ExternalId {
            source: "anidb".into(),
            value: anidb_id.to_string(),
        });
    }
    if let Some(tmdb_id) = movie.tmdb_id {
        extra_external_ids.push(scryer_domain::ExternalId {
            source: "tmdb".into(),
            value: tmdb_id.to_string(),
        });
    }

    let update = TitleMetadataUpdate {
        canonical_subject_key: movie.target_key.and_then(non_empty),
        name: non_empty(movie.name),
        year: movie.year.filter(|&y| y > 0),
        overview: non_empty(movie.overview),
        poster_url: non_empty(movie.poster_url),
        background_url: movie.background_url.and_then(non_empty),
        sort_title: non_empty(movie.sort_title),
        slug: non_empty(movie.slug),
        imdb_id: non_empty(movie.imdb_id),
        runtime_minutes: if movie.runtime_minutes > 0 {
            Some(movie.runtime_minutes)
        } else {
            None
        },
        popularity: movie.popularity.filter(|value| value.is_finite()),
        genres: movie.genres,
        canonical_tags: movie.canonical_tags,
        content_status: non_empty(movie.content_status),
        language: non_empty(movie.language),
        first_aired: None,
        network: None,
        studio: non_empty(movie.studio),
        country: None,
        aliases: vec![],
        metadata_language: Some(language.to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        digital_release_date: movie.tmdb_release_date,
        ratings: Some(movie.ratings),
        extra_external_ids,
        ..Default::default()
    };
    HydrationResult {
        metadata_update: update,
        seasons: vec![],
        episodes: vec![],
        anime_mappings: vec![],
        anime_movies: vec![],
        more_like_this: vec![],
    }
}

/// Build a [`HydrationResult`] from an already-fetched [`SeriesMetadata`].
pub fn series_to_hydration_result(series: SeriesMetadata, language: &str) -> HydrationResult {
    let extra_external_ids = primary_anime_mapping_extra_external_ids(&series.anime_mappings);
    let update = TitleMetadataUpdate {
        canonical_subject_key: series.target_key.and_then(non_empty),
        name: non_empty(series.name),
        year: series.year.filter(|&y| y > 0),
        overview: non_empty(series.overview),
        poster_url: non_empty(series.poster_url),
        background_url: series.background_url.and_then(non_empty),
        sort_title: non_empty(series.sort_name),
        slug: non_empty(series.slug),
        imdb_id: None,
        runtime_minutes: if series.runtime_minutes > 0 {
            Some(series.runtime_minutes)
        } else {
            None
        },
        genres: series.genres,
        canonical_tags: series.canonical_tags,
        content_status: non_empty(series.content_status),
        language: None,
        first_aired: non_empty(series.first_aired),
        network: non_empty(series.network),
        studio: None,
        country: non_empty(series.country),
        aliases: series.aliases,
        tagged_aliases: series.tagged_aliases,
        metadata_language: Some(language.to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        ratings: Some(series.ratings),
        extra_external_ids,
        ..Default::default()
    };
    HydrationResult {
        metadata_update: update,
        seasons: series.seasons,
        episodes: series.episodes,
        anime_mappings: series.anime_mappings,
        anime_movies: series.anime_movies,
        more_like_this: vec![],
    }
}

/// Configuration and strategies for a specific media facet.
/// Each facet (movie, series, anime) implements this trait to define
/// its metadata hydration, rename strategy, import routing, and
/// acquisition behavior.
#[async_trait]
pub trait FacetHandler: Send + Sync {
    /// The domain enum variant this handler covers.
    fn facet(&self) -> MediaFacet;

    /// String ID used in settings keys, database columns, audit logs.
    /// e.g. "movie", "series", "anime"
    fn facet_id(&self) -> &str;

    /// Download client category string.
    fn download_category(&self) -> &str;

    /// Settings key for the library root path (e.g. "movies.path").
    fn library_path_key(&self) -> &str;

    /// Settings key for the root folders JSON array (e.g. "movies.root_folders").
    fn root_folders_key(&self) -> &str;

    /// Default library root path.
    fn default_library_path(&self) -> &str;

    /// Whether this facet has episode-level structure.
    fn has_episodes(&self) -> bool;

    /// Indexer search category (e.g. "movie", "series", "anime").
    fn search_category(&self) -> &str;

    /// Hydrate a title's metadata by calling the metadata gateway.
    async fn hydrate_metadata(
        &self,
        gateway: &dyn MetadataGateway,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<HydrationResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anime_mapping(mapping_type: &str, anidb_id: Option<i64>) -> AnimeMapping {
        AnimeMapping {
            mal_id: None,
            mal_dub_id: None,
            anilist_id: None,
            anidb_id,
            kitsu_id: None,
            simkl_id: None,
            thetvdb_id: None,
            themoviedb_id: None,
            imdb_id: None,
            trakt_id: None,
            alt_tvdb_id: None,
            thetvdb_season: Some(1),
            thetvdb_part: None,
            score: None,
            anime_media_type: String::new(),
            global_media_type: String::new(),
            status: String::new(),
            mapping_type: mapping_type.to_string(),
            episode_mappings: vec![],
        }
    }

    fn test_series(anime_mappings: Vec<AnimeMapping>) -> SeriesMetadata {
        SeriesMetadata {
            target_key: None,
            tvdb_id: 12345,
            name: "Sword Art Online".to_string(),
            sort_name: "Sword Art Online".to_string(),
            slug: "sword-art-online".to_string(),
            year: Some(2012),
            content_status: String::new(),
            first_aired: String::new(),
            overview: String::new(),
            network: String::new(),
            runtime_minutes: 24,
            poster_url: String::new(),
            background_url: None,
            country: String::new(),
            genres: vec![],
            canonical_tags: vec![],
            aliases: vec![],
            tagged_aliases: vec![],
            seasons: vec![],
            episodes: vec![],
            anime_mappings,
            anime_movies: vec![],
            ratings: Default::default(),
        }
    }

    #[test]
    fn series_hydration_uses_primary_anime_mapping_for_title_level_external_ids() {
        let mut secondary_mapping = anime_mapping("S", Some(9999));
        secondary_mapping.mal_id = Some(999);
        let mut primary_mapping = anime_mapping("R", Some(15146));
        primary_mapping.mal_id = Some(111_001);
        primary_mapping.anilist_id = Some(222_002);
        primary_mapping.kitsu_id = Some(444_004);
        primary_mapping.simkl_id = Some(555_005);
        primary_mapping.thetvdb_id = Some(12345);
        primary_mapping.themoviedb_id = Some(666_006);
        primary_mapping.imdb_id = Some(777_007);
        primary_mapping.trakt_id = Some(888_008);

        let result = series_to_hydration_result(
            test_series(vec![secondary_mapping, primary_mapping]),
            "eng",
        );

        assert_eq!(
            result.metadata_update.extra_external_ids,
            vec![
                ExternalId {
                    source: "mal".to_string(),
                    value: "111001".to_string(),
                },
                ExternalId {
                    source: "anilist".to_string(),
                    value: "222002".to_string(),
                },
                ExternalId {
                    source: "anidb".to_string(),
                    value: "15146".to_string(),
                },
                ExternalId {
                    source: "kitsu".to_string(),
                    value: "444004".to_string(),
                },
                ExternalId {
                    source: "simkl".to_string(),
                    value: "555005".to_string(),
                },
                ExternalId {
                    source: "tvdb".to_string(),
                    value: "12345".to_string(),
                },
                ExternalId {
                    source: "tmdb".to_string(),
                    value: "666006".to_string(),
                },
                ExternalId {
                    source: "imdb".to_string(),
                    value: "tt777007".to_string(),
                },
                ExternalId {
                    source: "trakt".to_string(),
                    value: "888008".to_string(),
                },
            ]
        );
    }

    #[test]
    fn rename_facet_settings_for_anime_use_anime_scope_and_keys() {
        let settings = rename_facet_settings(&MediaFacet::Anime);
        assert_eq!(settings.scope_id, "anime");
        assert_eq!(settings.template_key, "rename.template.anime.global");
        assert_eq!(
            settings.collision_policy_key,
            "rename.collision_policy.anime.global"
        );
        assert_eq!(
            settings.missing_metadata_policy_key,
            "rename.missing_metadata_policy.anime.global"
        );
    }

    #[test]
    fn rename_facet_settings_for_series_remain_series_owned() {
        let settings = rename_facet_settings(&MediaFacet::Series);
        assert_eq!(settings.scope_id, "series");
        assert_eq!(settings.template_key, "rename.template.series.global");
        assert_eq!(
            settings.collision_policy_key,
            "rename.collision_policy.series.global"
        );
        assert_eq!(
            settings.missing_metadata_policy_key,
            "rename.missing_metadata_policy.series.global"
        );
    }
}
