use async_graphql::{Context, Object, Result as GqlResult};
use scryer_application::{
    DeleteExecutionConfirmation, DeleteTitlesJobItem, DeleteTitlesJobRequest,
    QueuedReleaseSelection,
};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{from_job_run, from_library_scan_summary, from_title};
use crate::types::*;
use crate::utils::{
    map_add_input, merge_title_option_tags, normalize_title_tags, parse_download_source_kind,
};

#[derive(Default)]
pub(crate) struct TitleMutations;

fn queued_download_payload(
    title: &scryer_domain::Title,
    job_id: String,
    source_title: Option<String>,
    source_kind: Option<scryer_application::DownloadSourceKind>,
) -> QueueDownloadPayload {
    QueueDownloadPayload {
        status: QueueDownloadResultStatusValue::Queued,
        job_id: Some(job_id),
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        source_title,
        source_kind: source_kind.map(DownloadSourceKindValue::from_application),
        conflict: None,
    }
}

#[Object]
impl TitleMutations {
    async fn add_title(
        &self,
        ctx: &Context<'_>,
        input: AddTitleInput,
    ) -> GqlResult<AddTitleResult> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let library_id = input.library_id.clone();
        let request = map_add_input(input)?;
        let result = if let Some(library_id) = library_id {
            app.add_title_with_outcome_in_library(&actor, request, library_id)
                .await
        } else {
            app.add_title_with_outcome(&actor, request).await
        }
        .map_err(to_gql_error)?;

        Ok(AddTitleResult {
            title: from_title(result.title),
            metadata_hydration_state: AddTitleHydrationStateValue::from_application(
                result.metadata_hydration_state,
            ),
            reused_existing_title: result.reused_existing_title,
            reused_queued_download: false,
            download_job_id: None,
            queued_download: None,
        })
    }

    async fn add_title_and_queue_download(
        &self,
        ctx: &Context<'_>,
        input: AddTitleInput,
    ) -> GqlResult<AddTitleResult> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let source_hint = input.source_hint.clone();
        let source_kind = parse_download_source_kind(input.source_kind);
        let source_title = input.source_title.clone();
        let library_id = input.library_id.clone();
        let request = map_add_input(input)?;
        let queued_release = QueuedReleaseSelection {
            source_hint,
            source_kind,
            source_title: source_title.clone(),
        };
        let result = if let Some(library_id) = library_id {
            app.add_title_and_queue_download_with_outcome_in_library(
                &actor,
                request,
                library_id,
                queued_release,
            )
            .await
        } else {
            app.add_title_and_queue_download_with_outcome(&actor, request, queued_release)
                .await
        }
        .map_err(to_gql_error)?;
        let queued_download = queued_download_payload(
            &result.title,
            result.download_job_id.clone(),
            source_title,
            source_kind,
        );

        Ok(AddTitleResult {
            title: from_title(result.title),
            metadata_hydration_state: AddTitleHydrationStateValue::from_application(
                result.metadata_hydration_state,
            ),
            reused_existing_title: result.reused_existing_title,
            reused_queued_download: result.reused_queued_download,
            download_job_id: Some(result.download_job_id),
            queued_download: Some(queued_download),
        })
    }

    async fn update_title(
        &self,
        ctx: &Context<'_>,
        input: UpdateTitleInput,
    ) -> GqlResult<TitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let UpdateTitleInput {
            title_id,
            name,
            facet,
            tags,
            options,
        } = input;
        let facet = facet.map(MediaFacetValue::into_domain);
        let mut tags = tags.map(normalize_title_tags);

        if let Some(options) = options {
            let base_tags = match tags.take() {
                Some(tags) => tags,
                None => app
                    .get_title_tags_for_update(&actor, &title_id)
                    .await
                    .map_err(to_gql_error)?,
            };
            tags = Some(merge_title_option_tags(base_tags, options));
        }

        let title = app
            .update_title_metadata(&actor, &title_id, name, facet, tags)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title(title))
    }

    async fn fix_title_match(
        &self,
        ctx: &Context<'_>,
        input: FixTitleMatchInput,
    ) -> GqlResult<FixTitleMatchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .fix_title_match(&actor, &input.title_id, &input.tvdb_id)
            .await
            .map_err(to_gql_error)?;

        Ok(FixTitleMatchPayload {
            title: from_title(result.title),
            hydrated: result.hydrated,
            library_scan: result.library_scan.map(from_library_scan_summary),
            warnings: result.warnings,
        })
    }

    async fn delete_title(&self, ctx: &Context<'_>, input: DeleteTitleInput) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_title(
            &actor,
            &input.title_id,
            input.delete_files_on_disk.unwrap_or(false),
            input
                .preview_fingerprint
                .map(|preview_fingerprint| DeleteExecutionConfirmation {
                    preview_fingerprint,
                    typed_confirmation: input.typed_confirmation,
                }),
        )
        .await
        .map(|_| true)
        .map_err(to_gql_error)
    }

    async fn delete_titles(
        &self,
        ctx: &Context<'_>,
        input: DeleteTitlesInput,
    ) -> GqlResult<DeleteTitlesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .start_delete_titles_job(
                &actor,
                DeleteTitlesJobRequest {
                    items: input
                        .items
                        .into_iter()
                        .map(|item| DeleteTitlesJobItem {
                            title_id: item.title_id,
                            preview_fingerprint: item.preview_fingerprint,
                        })
                        .collect(),
                    delete_files_on_disk: input.delete_files_on_disk.unwrap_or(false),
                    typed_confirmation: input.typed_confirmation,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteTitlesPayload {
            job_run: from_job_run(accepted.job_run),
            accepted_title_ids: accepted.accepted_title_ids,
        })
    }

    async fn clear_title_release_blocklist_entry(
        &self,
        ctx: &Context<'_>,
        input: ClearTitleReleaseBlocklistEntryInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.clear_title_release_blocklist_entry(&actor, &input.id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }

    async fn set_title_monitored(
        &self,
        ctx: &Context<'_>,
        input: SetTitleMonitoredInput,
    ) -> GqlResult<TitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .set_title_monitored(&actor, &input.title_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title(title))
    }
}
