use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{from_episode, from_series_movie_link};
use crate::types::*;
use async_graphql::{Context, Object, Result as GqlResult};

#[derive(Default)]
pub(crate) struct CollectionMutations;

#[Object]
impl CollectionMutations {
    async fn set_collection_monitored(
        &self,
        ctx: &Context<'_>,
        input: SetCollectionMonitoredInput,
    ) -> GqlResult<SetCollectionMonitoredPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .set_collection_monitored(&actor, &input.collection_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        let episodes = app
            .list_episodes(&actor, &input.collection_id)
            .await
            .map_err(to_gql_error)?;
        Ok(SetCollectionMonitoredPayload {
            id: collection.id,
            monitored: collection.monitored,
            episodes: episodes.into_iter().map(from_episode).collect(),
        })
    }

    async fn set_episode_monitored(
        &self,
        ctx: &Context<'_>,
        input: SetEpisodeMonitoredInput,
    ) -> GqlResult<EpisodePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .set_episode_monitored(&actor, &input.episode_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_episode(episode))
    }

    async fn set_series_movie_monitored(
        &self,
        ctx: &Context<'_>,
        input: SetSeriesMovieMonitoredInput,
    ) -> GqlResult<SeriesMovieLinkPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let link = app
            .set_series_movie_monitored(&actor, &input.series_movie_link_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_series_movie_link(link))
    }
}
