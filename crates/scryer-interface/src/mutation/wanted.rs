use async_graphql::{Context, Object, Result as GqlResult};
use scryer_application::{SubmissionConflictPolicy, WantedSearchOutcome};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::*;

#[derive(Default)]
pub(crate) struct WantedMutations;

pub(crate) fn wanted_search_payload(outcome: WantedSearchOutcome) -> WantedSearchPayload {
    WantedSearchPayload {
        queued_count: outcome.queued_count as i32,
        skipped_in_progress_count: outcome.skipped_in_progress_count as i32,
        conflict: outcome
            .conflict
            .map(super::downloads::queue_download_conflict_payload),
    }
}

#[Object]
impl WantedMutations {
    async fn trigger_title_wanted_search(
        &self,
        ctx: &Context<'_>,
        input: TriggerTitleWantedSearchInput,
    ) -> GqlResult<WantedSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let queued = app
            .trigger_title_wanted_search(
                &actor,
                &input.title_id,
                SubmissionConflictPolicy::from_replace_flag(
                    input.replace_in_progress.unwrap_or(false),
                ),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(wanted_search_payload(queued))
    }

    async fn trigger_title_mismatch_recovery_search(
        &self,
        ctx: &Context<'_>,
        input: TitleIdInput,
    ) -> GqlResult<i32> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let queued = app
            .trigger_title_mismatch_recovery_search(&actor, &input.title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(queued as i32)
    }

    async fn trigger_season_wanted_search(
        &self,
        ctx: &Context<'_>,
        input: TriggerSeasonWantedSearchInput,
    ) -> GqlResult<WantedSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let queued = app
            .trigger_season_wanted_search(&actor, &input.title_id, input.season_number as u32)
            .await
            .map_err(to_gql_error)?;
        Ok(wanted_search_payload(queued))
    }

    async fn trigger_wanted_search(
        &self,
        ctx: &Context<'_>,
        input: TriggerWantedSearchInput,
    ) -> GqlResult<WantedSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let outcome = app
            .trigger_wanted_item_search(
                &actor,
                &input.wanted_item_id,
                SubmissionConflictPolicy::from_replace_flag(
                    input.replace_in_progress.unwrap_or(false),
                ),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(wanted_search_payload(outcome))
    }

    async fn pause_wanted_item(
        &self,
        ctx: &Context<'_>,
        input: WantedItemIdInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.pause_wanted_item(&actor, &input.wanted_item_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn resume_wanted_item(
        &self,
        ctx: &Context<'_>,
        input: WantedItemIdInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.resume_wanted_item(&actor, &input.wanted_item_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn reset_wanted_item(
        &self,
        ctx: &Context<'_>,
        input: WantedItemIdInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.reset_wanted_item(&actor, &input.wanted_item_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn force_grab_pending_release(
        &self,
        ctx: &Context<'_>,
        input: PendingReleaseActionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.force_grab_pending_release(&actor, &input.id)
            .await
            .map_err(to_gql_error)
    }

    async fn dismiss_pending_release(
        &self,
        ctx: &Context<'_>,
        input: PendingReleaseActionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.dismiss_pending_release(&actor, &input.id)
            .await
            .map_err(to_gql_error)
    }
}
