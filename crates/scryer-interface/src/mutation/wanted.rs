use async_graphql::{Context, ID, Object, Result as GqlResult};
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
        let title_id = input.title_id.to_string();
        let queued = app
            .trigger_title_wanted_search(
                &actor,
                &title_id,
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
        title_id: ID,
    ) -> GqlResult<TriggerTitleMismatchRecoverySearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id_string = title_id.to_string();
        let queued = app
            .trigger_title_mismatch_recovery_search(&actor, &title_id_string)
            .await
            .map_err(to_gql_error)?;
        Ok(TriggerTitleMismatchRecoverySearchPayload {
            title_id,
            queued_count: queued as i32,
        })
    }

    async fn trigger_season_wanted_search(
        &self,
        ctx: &Context<'_>,
        input: TriggerSeasonWantedSearchInput,
    ) -> GqlResult<WantedSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = input.title_id.to_string();
        let queued = app
            .trigger_season_wanted_search(&actor, &title_id, input.season_number as u32)
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
        let wanted_item_id = input.wanted_item_id.to_string();
        let outcome = app
            .trigger_wanted_item_search(
                &actor,
                &wanted_item_id,
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
        id: ID,
    ) -> GqlResult<PauseWantedItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.pause_wanted_item(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(PauseWantedItemPayload {
            id: ID::from(id),
            paused: true,
        })
    }

    async fn resume_wanted_item(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<ResumeWantedItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.resume_wanted_item(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(ResumeWantedItemPayload {
            id: ID::from(id),
            resumed: true,
        })
    }

    async fn reset_wanted_item(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<ResetWantedItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.reset_wanted_item(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(ResetWantedItemPayload {
            id: ID::from(id),
            reset: true,
        })
    }

    async fn force_grab_pending_release(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<ForceGrabPendingReleasePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        let grabbed = app
            .force_grab_pending_release(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(ForceGrabPendingReleasePayload {
            id: ID::from(id),
            grabbed,
        })
    }

    async fn dismiss_pending_release(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<DismissPendingReleasePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        let dismissed = app
            .dismiss_pending_release(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DismissPendingReleasePayload {
            id: ID::from(id),
            dismissed,
        })
    }
}
