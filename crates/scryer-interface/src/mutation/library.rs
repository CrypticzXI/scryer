use async_graphql::{Context, Error, Object, Result as GqlResult};
use scryer_application::DeleteExecutionConfirmation;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_cancel_library_scan_result, from_library_scan_session, from_library_scan_summary,
    from_media_rename_apply, from_resolve_pending_import_result,
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
#[derive(Default)]
pub(crate) struct LibraryMutations;

#[Object]
impl LibraryMutations {
    async fn scan_library(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
    ) -> GqlResult<LibraryScanProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let facet = facet.into_domain();
        let session = app
            .trigger_library_scan(&actor, facet)
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
