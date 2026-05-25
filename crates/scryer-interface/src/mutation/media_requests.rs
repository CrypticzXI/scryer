use async_graphql::{Context, Object, Result as GqlResult};
use scryer_domain::ExternalId;

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::{SubmitMediaRequestInput, SubmitMediaRequestPayload};

#[derive(Default)]
pub(crate) struct MediaRequestMutations;

#[Object]
impl MediaRequestMutations {
    async fn submit_media_request(
        &self,
        ctx: &Context<'_>,
        input: SubmitMediaRequestInput,
    ) -> GqlResult<SubmitMediaRequestPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let outcome = app
            .submit_media_request(
                &actor,
                scryer_application::SubmitMediaRequestInput {
                    library_id: input.library_id,
                    facet: input.facet.into_domain(),
                    title: input.title,
                    sort_title: input.sort_title,
                    slug: input.slug,
                    poster_url: input.poster_url,
                    year: input.year,
                    overview: input.overview,
                    runtime_minutes: input.runtime_minutes,
                    language: input.language,
                    content_status: input.content_status,
                    external_ids: input
                        .external_ids
                        .into_iter()
                        .map(|external_id| ExternalId {
                            source: external_id.source,
                            value: external_id.value,
                        })
                        .collect(),
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(SubmitMediaRequestPayload {
            accepted: outcome.accepted,
        })
    }
}
