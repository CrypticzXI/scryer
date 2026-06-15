use async_graphql::{Context, Object, Result as GqlResult};
use scryer_domain::ExternalId;

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_media_request;
use crate::types::{
    ApproveMediaRequestInput, ApproveMediaRequestPayload, MediaRequestActionInput,
    MediaRequestActionPayload, MediaRequestPayload, SubmitMediaRequestInput,
    SubmitMediaRequestPayload, UpdateMediaRequestInput,
};

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
                    year: input.year,
                    overview: input.overview,
                    runtime_minutes: input.runtime_minutes,
                    language: input.language,
                    content_status: input.content_status,
                    requested_quality_profile_id: input.requested_quality_profile_id,
                    requested_monitor_type: input
                        .requested_monitor_type
                        .map(|value| value.as_tag_value().to_string()),
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

    async fn approve_media_request(
        &self,
        ctx: &Context<'_>,
        input: ApproveMediaRequestInput,
    ) -> GqlResult<ApproveMediaRequestPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let outcome = app
            .approve_media_request(
                &actor,
                &input.request_id,
                &input.quality_profile_id,
                input
                    .monitor_type
                    .map(|value| value.as_tag_value().to_string()),
            )
            .await
            .map_err(to_gql_error)?;

        Ok(ApproveMediaRequestPayload {
            accepted: outcome.accepted,
            title_id: outcome.title_id,
            wanted_search: outcome
                .wanted_search
                .map(super::wanted::wanted_search_payload),
            search_error: outcome.search_error,
        })
    }

    async fn dismiss_media_request(
        &self,
        ctx: &Context<'_>,
        input: MediaRequestActionInput,
    ) -> GqlResult<MediaRequestActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.dismiss_media_request(&actor, &input.request_id)
            .await
            .map_err(to_gql_error)?;

        Ok(MediaRequestActionPayload { accepted: true })
    }

    async fn update_my_media_request(
        &self,
        ctx: &Context<'_>,
        input: UpdateMediaRequestInput,
    ) -> GqlResult<MediaRequestPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let request = app
            .update_my_media_request(
                &actor,
                scryer_application::UpdateMediaRequestInput {
                    request_id: input.request_id,
                    requested_quality_profile_id: input.requested_quality_profile_id,
                    requested_monitor_type: input
                        .requested_monitor_type
                        .map(|value| value.as_tag_value().to_string()),
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_media_request(request))
    }

    async fn cancel_my_media_request(
        &self,
        ctx: &Context<'_>,
        input: MediaRequestActionInput,
    ) -> GqlResult<MediaRequestActionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.cancel_my_media_request(&actor, &input.request_id)
            .await
            .map_err(to_gql_error)?;

        Ok(MediaRequestActionPayload { accepted: true })
    }
}
