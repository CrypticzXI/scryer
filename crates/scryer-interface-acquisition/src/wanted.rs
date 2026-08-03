use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::WantedSearchOutcome;

use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct WantedMutations;

/// Shared builder for the media-request approval search outcome. The convergence
/// cutover removed the per-item `trigger*WantedSearch` mutations (the convergence
/// cursor and `triggerAcquisitionSearch` own search now), but the media-request
/// approval flow still reports its post-approval search via this payload.
pub(crate) fn wanted_search_payload(outcome: WantedSearchOutcome) -> WantedSearchPayload {
    WantedSearchPayload {
        queued_count: outcome.queued_count as i32,
        skipped_in_progress_count: outcome.skipped_in_progress_count as i32,
    }
}

#[Object]
impl WantedMutations {
    /// Re-open the title's title-mismatch scopes for search after a rematch
    ///. Retained: a rematch changes the scope fingerprint, so the
    /// re-open prunes coverage and the cursor re-converges.
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

    /// Pause acquisition for a scope. `id` is a state-row id or a convergence scope
    /// key — a scope key with no row yet materializes one.
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
        Ok(PauseWantedItemPayload { id: ID::from(id) })
    }

    /// Resume acquisition for a paused scope. `id` is a state-row id or a
    /// convergence scope key.
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
        Ok(ResumeWantedItemPayload { id: ID::from(id) })
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
        app.dismiss_pending_release(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DismissPendingReleasePayload { id: ID::from(id) })
    }

    /// Start an interactive acquisition-search job over the selected wanted/upgrade
    /// scopes. Replaces the retired per-item trigger
    /// mutations: one server job (progress/cancel survive refresh) runs the
    /// interactive best-release search+grab for every requested scope.
    async fn trigger_acquisition_search(
        &self,
        ctx: &Context<'_>,
        input: TriggerAcquisitionSearchInput,
    ) -> GqlResult<AcquisitionSearchJobPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let request = scryer_application::AcquisitionSearchRequest {
            wanted_kind: input
                .wanted_kind
                .map(|kind| match kind {
                    WantedKindValue::Missing => scryer_application::WantedKind::Missing,
                    WantedKindValue::CutoffUpgrade => scryer_application::WantedKind::CutoffUpgrade,
                })
                .unwrap_or(scryer_application::WantedKind::Missing),
            facet: input.facet.map(MediaFacetValue::into_domain),
            library_ids: input
                .library_ids
                .map(|ids| ids.into_iter().map(String::from).collect())
                .unwrap_or_default(),
            title_id: input.title_id.map(String::from),
            season_number: input.season_number,
            wanted_item_id: input.wanted_item_id.map(String::from),
        };
        let run = app
            .start_acquisition_search_job(&actor, request)
            .await
            .map_err(to_gql_error)?;
        // The run is freshly started; reflect its initial (running) snapshot.
        Ok(AcquisitionSearchJobPayload {
            id: run.id.into(),
            state: AcquisitionSearchJobStateValue::Running,
            total: 0,
            processed: 0,
            grabbed_count: 0,
            failed_count: 0,
            current_title: None,
            started_at: run.started_at,
            finished_at: run.completed_at,
        })
    }

    /// Cancel a running interactive acquisition-search job.
    async fn cancel_acquisition_search(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<CancelAcquisitionSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .cancel_acquisition_search(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(CancelAcquisitionSearchPayload { id, accepted })
    }
}
