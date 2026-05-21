use async_graphql::{Context, Error, Object, Result as GqlResult};
use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::mappers::from_calendar_episode;
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct MetadataQueries;

fn from_metadata_search_item(
    item: scryer_application::RichMetadataSearchItem,
) -> MetadataSearchItemPayload {
    MetadataSearchItemPayload {
        tvdb_id: item.tvdb_id,
        name: item.name,
        imdb_id: item.imdb_id,
        slug: item.slug,
        type_hint: item.type_hint,
        year: item.year,
        status: item.status,
        overview: item.overview,
        popularity: item.popularity,
        poster_url: item.poster_url,
        language: item.language,
        runtime_minutes: item.runtime_minutes,
        sort_title: item.sort_title,
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl MetadataQueries {
    async fn search_metadata(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(name = "type")] type_hint: String,
        #[graphql(default = 25)] limit: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
        year: Option<i32>,
    ) -> GqlResult<Vec<MetadataSearchItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.clamp(1, 100);
        let results = app
            .search_metadata(&actor, &query, &type_hint, limit, &language, year)
            .await
            .map_err(to_gql_error)?;
        Ok(results.into_iter().map(from_metadata_search_item).collect())
    }

    async fn search_metadata_multi(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(default = 25)] limit: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataSearchMultiPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.clamp(1, 100);
        let result = app
            .search_metadata_multi(&actor, &query, limit, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataSearchMultiPayload {
            movies: result
                .movies
                .into_iter()
                .map(from_metadata_search_item)
                .collect(),
            series: result
                .series
                .into_iter()
                .map(from_metadata_search_item)
                .collect(),
            anime: result
                .anime
                .into_iter()
                .map(from_metadata_search_item)
                .collect(),
        })
    }

    async fn metadata_movie(
        &self,
        ctx: &Context<'_>,
        tvdb_id: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataMoviePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let movie = app
            .get_metadata_movie(&actor, tvdb_id as i64, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataMoviePayload {
            tvdb_id: movie.tvdb_id.to_string(),
            name: movie.name,
            slug: movie.slug,
            year: movie.year,
            status: movie.content_status,
            overview: movie.overview,
            poster_url: movie.poster_url,
            language: movie.language,
            runtime_minutes: movie.runtime_minutes,
            sort_title: movie.sort_title,
            imdb_id: movie.imdb_id,
            genres: movie.genres,
            studio: movie.studio,
            tmdb_release_date: movie.tmdb_release_date,
        })
    }

    async fn metadata_series(
        &self,
        ctx: &Context<'_>,
        id: String,
        #[graphql(default = true)] include_episodes: bool,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataSeriesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let tvdb_id: i64 = id.parse().map_err(|_| Error::new("invalid tvdb id"))?;
        let series = app
            .get_metadata_series(&actor, tvdb_id, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataSeriesPayload {
            tvdb_id: series.tvdb_id.to_string(),
            name: series.name,
            sort_name: series.sort_name,
            slug: series.slug,
            year: series.year,
            status: series.content_status,
            first_aired: series.first_aired,
            overview: series.overview,
            network: series.network,
            runtime_minutes: series.runtime_minutes,
            poster_url: series.poster_url,
            country: series.country,
            genres: series.genres,
            aliases: series.aliases,
            seasons: series
                .seasons
                .into_iter()
                .map(|s| MetadataSeasonPayload {
                    tvdb_id: s.tvdb_id.to_string(),
                    number: s.number,
                    label: s.label,
                    episode_type: s.episode_type,
                })
                .collect(),
            episodes: if include_episodes {
                series
                    .episodes
                    .into_iter()
                    .map(|e| MetadataEpisodePayload {
                        tvdb_id: e.tvdb_id.to_string(),
                        episode_number: e.episode_number,
                        season_number: e.season_number,
                        name: e.name,
                        aired: e.aired,
                        runtime_minutes: e.runtime_minutes,
                        is_filler: e.is_filler,
                    })
                    .collect()
            } else {
                vec![]
            },
        })
    }

    async fn calendar_episodes(
        &self,
        ctx: &Context<'_>,
        start_date: String,
        end_date: String,
        library_ids: Option<Vec<String>>,
    ) -> GqlResult<Vec<CalendarEpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episodes = app
            .list_calendar_episodes(&actor, &start_date, &end_date, library_ids)
            .await
            .map_err(to_gql_error)?;
        Ok(episodes.into_iter().map(from_calendar_episode).collect())
    }
}
