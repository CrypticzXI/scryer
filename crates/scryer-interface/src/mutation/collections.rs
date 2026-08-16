use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{from_episode, from_series_movie_link};
use crate::types::*;
use async_graphql::{Context, Object, Result as GqlResult};

#[derive(Default)]
pub(crate) struct CollectionMutations;

#[Object]
impl CollectionMutations {
    /// Set collection monitoring and return the affected collection episodes.
    async fn set_collection_monitored(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Collection identity and desired monitored state.")]
        input: SetCollectionMonitoredInput,
    ) -> GqlResult<SetCollectionMonitoredPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection_id = input.collection_id.to_string();
        let collection = app
            .set_collection_monitored(&actor, &collection_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        let episodes = app
            .list_episodes(&actor, &collection_id)
            .await
            .map_err(to_gql_error)?;
        Ok(SetCollectionMonitoredPayload {
            id: collection.id.into(),
            monitored: collection.monitored,
            episodes: episodes
                .into_iter()
                .map(|episode| from_episode(&app, episode))
                .collect(),
        })
    }

    /// Set episode monitoring and return the updated episode.
    async fn set_episode_monitored(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Episode identity and desired monitored state.")]
        input: SetEpisodeMonitoredInput,
    ) -> GqlResult<EpisodePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode_id = input.episode_id.to_string();
        let episode = app
            .set_episode_monitored(&actor, &episode_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_episode(&app, episode))
    }

    /// Set monitoring for a series-movie link and return the updated link.
    async fn set_series_movie_monitored(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Series-movie link identity and desired monitored state.")]
        input: SetSeriesMovieMonitoredInput,
    ) -> GqlResult<SeriesMovieLinkPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let series_movie_link_id = input.series_movie_link_id.to_string();
        let link = app
            .set_series_movie_monitored(&actor, &series_movie_link_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_series_movie_link(&app, link))
    }
}
