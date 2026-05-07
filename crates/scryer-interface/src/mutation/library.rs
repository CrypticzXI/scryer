use async_graphql::{Context, Error, Object, Result as GqlResult};
use scryer_application::DeleteExecutionConfirmation;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_cancel_library_scan_result, from_ignore_pending_import_result, from_library,
    from_library_scan_session, from_library_scan_summary, from_media_rename_apply,
    from_resolve_pending_import_result,
};
use crate::types::*;

static RENAME_IDEMPOTENCY_KEYS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn claim_rename_idempotency_key(scope: &str, key: Option<String>) -> GqlResult<Option<String>> {
    let Some(raw_key) = key else {
        return Ok(None);
    };

    let normalized = raw_key.trim();
    if normalized.is_empty() {
        return Err(Error::new("idempotencyKey cannot be empty"));
    }

    let composite = format!("{scope}:{normalized}");
    let store = &*RENAME_IDEMPOTENCY_KEYS;
    let mut guard = store
        .lock()
        .map_err(|_| Error::new("failed to lock rename idempotency key store"))?;
    if !guard.insert(composite.clone()) {
        return Err(Error::new("duplicate idempotencyKey"));
    }

    Ok(Some(composite))
}

fn library_settings_draft(
    input: LibrarySettingsInput,
) -> scryer_application::LibrarySettingsOverrideDraft {
    scryer_application::LibrarySettingsOverrideDraft {
        required_audio_languages: input.required_audio_languages,
        quality_profile_id: input.quality_profile_id,
        scoring_persona: input
            .scoring_persona
            .map(ScoringPersonaValue::into_application),
        filler_policy: input.filler_policy,
        recap_policy: input.recap_policy,
        monitor_specials: input.monitor_specials,
        inter_season_movies: input.inter_season_movies,
        monitor_filler_movies: input.monitor_filler_movies,
        nfo_write_on_import: input.nfo_write_on_import,
        plexmatch_write_on_import: input.plexmatch_write_on_import,
        indexer_routing: input.indexer_routing.map(|entries| {
            entries
                .into_iter()
                .map(|entry| scryer_application::IndexerRoutingSettingsEntry {
                    indexer_id: entry.indexer_id,
                    enabled: entry.enabled,
                    categories: entry.categories,
                    priority: entry.priority,
                })
                .collect()
        }),
        download_client_routing: input.download_client_routing.map(|entries| {
            entries
                .into_iter()
                .map(
                    |entry| scryer_application::DownloadClientRoutingSettingsEntry {
                        client_id: entry.client_id,
                        enabled: entry.enabled,
                        category: entry.category,
                        recent_queue_priority: entry.recent_queue_priority,
                        older_queue_priority: entry.older_queue_priority,
                        remove_completed: entry.remove_completed,
                        remove_failed: entry.remove_failed,
                    },
                )
                .collect()
        }),
    }
}

#[derive(Default)]
pub(crate) struct LibraryMutations;

#[Object]
impl LibraryMutations {
    async fn create_library(
        &self,
        ctx: &Context<'_>,
        input: CreateLibraryInput,
    ) -> GqlResult<LibraryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let roots = input
            .roots
            .into_iter()
            .map(|root| scryer_application::LibraryRootDraft {
                path: root.path,
                is_default: root.is_default,
            })
            .collect();
        let library = app
            .create_library(
                &actor,
                input.facet.into_domain(),
                input.name,
                roots,
                input.settings.map(library_settings_draft),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_library(library))
    }

    async fn update_library(
        &self,
        ctx: &Context<'_>,
        input: UpdateLibraryInput,
    ) -> GqlResult<LibraryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let roots = input.roots.map(|roots| {
            roots
                .into_iter()
                .map(|root| scryer_application::LibraryRootDraft {
                    path: root.path,
                    is_default: root.is_default,
                })
                .collect()
        });
        let library = app
            .update_library(
                &actor,
                &input.library_id,
                input.name,
                roots,
                input.settings.map(library_settings_draft),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_library(library))
    }

    async fn delete_empty_library(
        &self,
        ctx: &Context<'_>,
        input: DeleteLibraryInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_empty_library(&actor, &input.library_id)
            .await
            .map_err(to_gql_error)
    }

    async fn scan_library(
        &self,
        ctx: &Context<'_>,
        library_id: String,
    ) -> GqlResult<LibraryScanProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let session = app
            .trigger_library_scan_by_id(&actor, &library_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_scan_session(session))
    }

    async fn scan_title_library(
        &self,
        ctx: &Context<'_>,
        input: TitleIdInput,
    ) -> GqlResult<LibraryScanSummaryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let summary = app
            .scan_title_library(&actor, &input.title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_scan_summary(summary))
    }

    async fn cancel_library_scan(
        &self,
        ctx: &Context<'_>,
        input: CancelLibraryScanInput,
    ) -> GqlResult<CancelLibraryScanPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .cancel_library_scan(&actor, &input.session_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_cancel_library_scan_result(result))
    }

    async fn resolve_pending_import(
        &self,
        ctx: &Context<'_>,
        input: ResolvePendingImportInput,
    ) -> GqlResult<ResolvePendingImportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .resolve_pending_import(&actor, &input.pending_import_id, &input.tvdb_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_resolve_pending_import_result(result))
    }

    async fn bind_pending_import(
        &self,
        ctx: &Context<'_>,
        input: BindPendingImportInput,
    ) -> GqlResult<ResolvePendingImportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .bind_title_bound_pending_import(
                &actor,
                &input.pending_import_id,
                input.collection_id.as_deref(),
                &input.episode_ids,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_resolve_pending_import_result(result))
    }

    async fn ignore_pending_import(
        &self,
        ctx: &Context<'_>,
        input: IgnorePendingImportInput,
    ) -> GqlResult<IgnorePendingImportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .ignore_pending_import(&actor, &input.pending_import_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_ignore_pending_import_result(result))
    }

    async fn apply_media_rename(
        &self,
        ctx: &Context<'_>,
        input: MediaRenameApplyInput,
    ) -> GqlResult<MediaRenameApplyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let MediaRenameApplyInput {
            facet,
            title_id,
            fingerprint,
            idempotency_key,
        } = input;
        let facet = facet.into_domain();
        let facet_name = facet.as_str();
        let idempotency_key = claim_rename_idempotency_key("apply_media_rename", idempotency_key)?;

        let result = app
            .apply_rename_for_title(&actor, &title_id, facet, &fingerprint)
            .await
            .map_err(to_gql_error)?;
        let _ = app
            .record_rename_apply_audit(
                &actor,
                "rename_apply_title",
                facet_name,
                Some(&title_id),
                idempotency_key.as_deref(),
                &result,
            )
            .await;

        Ok(from_media_rename_apply(result))
    }

    async fn delete_media_file(
        &self,
        ctx: &Context<'_>,
        input: DeleteMediaFileInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_media_file(
            &actor,
            &input.file_id,
            input.delete_from_disk.unwrap_or(true),
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

    async fn apply_media_rename_bulk(
        &self,
        ctx: &Context<'_>,
        input: MediaRenameBulkApplyInput,
    ) -> GqlResult<MediaRenameApplyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let MediaRenameBulkApplyInput {
            facet,
            fingerprint,
            idempotency_key,
        } = input;
        let facet = facet.into_domain();
        let facet_name = facet.as_str();
        let idempotency_key =
            claim_rename_idempotency_key("apply_media_rename_bulk", idempotency_key)?;

        let result = app
            .apply_rename_for_facet(&actor, facet, &fingerprint)
            .await
            .map_err(to_gql_error)?;
        let _ = app
            .record_rename_apply_audit(
                &actor,
                "rename_apply_facet",
                facet_name,
                None,
                idempotency_key.as_deref(),
                &result,
            )
            .await;

        Ok(from_media_rename_apply(result))
    }

    async fn rehydrate_all_metadata(&self, ctx: &Context<'_>, language: String) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let cleared = app
            .rehydrate_all_metadata(&actor, &language)
            .await
            .map_err(to_gql_error)?;

        tracing::info!(
            language = %language,
            titles_cleared = cleared,
            "metadata rehydration accepted"
        );

        Ok(true)
    }
}
