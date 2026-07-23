use async_graphql::{Context, ID, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::query::from_interactive_release_search_snapshot;
use crate::types::*;

#[derive(Default)]
pub(crate) struct InteractiveSearchMutations;

#[Object]
impl InteractiveSearchMutations {
    /// Start a server-side interactive release-search job for the same scopes
    /// as `searchReleases`. Results stream into the job snapshot as each
    /// indexer completes; poll it with `interactiveReleaseSearch`. Starting a
    /// new search for a scope cancels the caller's running job for that scope.
    async fn start_interactive_release_search(
        &self,
        ctx: &Context<'_>,
        input: SearchReleasesInput,
    ) -> GqlResult<InteractiveReleaseSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let SearchReleasesInput {
            title_id,
            series_movie_link_id,
            season,
            episode,
            limit,
        } = input;
        let request = scryer_application::InteractiveReleaseSearchRequest {
            title_id: title_id.to_string(),
            series_movie_link_id: series_movie_link_id.map(String::from),
            season,
            episode,
            limit,
        };
        let snapshot = app
            .start_interactive_release_search(&actor, request)
            .await
            .map_err(to_gql_error)?;
        Ok(from_interactive_release_search_snapshot(snapshot))
    }

    /// Cancel a running interactive release-search job.
    async fn cancel_interactive_release_search(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<CancelInteractiveReleaseSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .cancel_interactive_release_search(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(CancelInteractiveReleaseSearchPayload { id, accepted })
    }
}
