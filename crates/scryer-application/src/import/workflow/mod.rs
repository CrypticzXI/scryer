use crate::helpers::parse_usable_release_title;
#[cfg(test)]
use crate::import_title_resolution::normalize_imdb_id;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{
    AcquisitionScopeCompleteTransition, AcquisitionScopeStatesQuery, AppError, AppResult,
    AppUseCase, ClientJobLocator, DownloadSubmission, DownloadSubmissionIdentity, ImportArtifact,
    ParsedReleaseMetadata, SubmissionScope,
    activity::NotificationMediaUpdate,
    app_usecase_post_processing::{PostProcessingContext, spawn_post_processing},
    apply_remote_path_mappings_to_completed_download,
    domain_events::{
        created_media_update, deleted_media_update, new_title_domain_event, title_context_snapshot,
    },
    effective_title_folder_path,
    helpers::{has_usable_release_title_signal, normalize_release_title_signal},
    import_parameters::{extract_parameter, submission_has_scryer_origin},
    nfo::{render_episode_nfo, render_movie_nfo, render_plexmatch, render_tvshow_nfo},
    parse_download_client_remote_path_mappings, parse_release_metadata,
    polling_worker::PollingWorker,
    render_rename_template, sanitize_filesystem_component,
};
use chrono::{DateTime, Utc};
use scryer_domain::{
    Collection, CollectionType, CompletedDownload, DomainEventPayload, DownloadQueueItem, Id,
    ImportCompletedEventData, ImportDecision, ImportErrorCode, ImportRecord, ImportResult,
    ImportSkipReason, ImportStatus, ImportType, MediaFacet, Title, TrackedDownloadState, User,
    is_video_file,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const IMPORT_TRANSFER_PROGRESS_MIN_INTERVAL: Duration = Duration::from_secs(1);
const IMPORT_TRANSFER_PROGRESS_MIN_BYTES: u64 = 64 * 1024 * 1024;
const IMPORT_TRANSFER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const IMPORT_STALE_RECOVERY_SECONDS: i64 = 45 * 60;

fn should_persist_import_transfer_progress(
    progress: &crate::ImportFileTransferProgress,
    last_phase: Option<scryer_domain::ImportTransferPhase>,
    last_bytes: u64,
    last_emit: Option<Instant>,
) -> bool {
    if last_phase != Some(progress.phase) {
        return true;
    }
    if progress.bytes == 0 || progress.bytes >= progress.total_bytes {
        return true;
    }
    if progress.bytes.saturating_sub(last_bytes) >= IMPORT_TRANSFER_PROGRESS_MIN_BYTES {
        return true;
    }
    last_emit.is_none_or(|instant| instant.elapsed() >= IMPORT_TRANSFER_PROGRESS_MIN_INTERVAL)
}

fn should_persist_import_transfer_heartbeat(last_emit: Option<Instant>) -> bool {
    last_emit.is_none_or(|instant| instant.elapsed() >= IMPORT_TRANSFER_HEARTBEAT_INTERVAL)
}

#[expect(
    clippy::too_many_arguments,
    reason = "import progress wiring carries source, destination, library, and source validation context"
)]
pub(crate) async fn import_file_with_record_progress(
    app: &AppUseCase,
    import_id: &str,
    library_id: &str,
    facet: &scryer_domain::MediaFacet,
    source: &Path,
    dest: &Path,
    mode: scryer_domain::ImportMode,
    expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    completed: Option<&scryer_domain::CompletedDownload>,
) -> AppResult<CoordinatedImportFileResult> {
    let permissions = app
        .resolve_import_file_permissions(Some(library_id), facet)
        .await?;
    let active_stream = app
        .runtime
        .imports
        .active_streams
        .register(import_id, library_id, facet.clone(), source, dest)
        .await;
    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::ImportFileTransferProgress>();
    let progress_app = app.clone();
    let progress_import_id = import_id.to_string();
    let progress_stream = active_stream.clone();
    let progress_task = tokio::spawn(async move {
        let mut last_phase = None;
        let mut last_bytes = 0u64;
        let mut last_emit = None;
        let mut last_progress: Option<crate::ImportFileTransferProgress> = None;
        let mut heartbeat = tokio::time::interval(IMPORT_TRANSFER_HEARTBEAT_INTERVAL);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                maybe_progress = progress_rx.recv() => {
                    let Some(progress) = maybe_progress else {
                        break;
                    };
                    progress_stream
                        .update_transfer(progress.phase, progress.bytes, progress.total_bytes)
                        .await;
                    last_progress = Some(progress.clone());
                    if !should_persist_import_transfer_progress(
                        &progress, last_phase, last_bytes, last_emit,
                    ) {
                        continue;
                    }

                    match progress_app
                        .update_import_transfer_progress_and_notify(
                            &progress_import_id,
                            progress.phase,
                            progress.bytes,
                            progress.total_bytes,
                        )
                        .await
                    {
                        Ok(()) => {
                            last_phase = Some(progress.phase);
                            last_bytes = progress.bytes;
                            last_emit = Some(Instant::now());
                        }
                        Err(error) => {
                            tracing::warn!(
                                import_id = %progress_import_id,
                                error = %error,
                                "failed to persist import transfer progress"
                            );
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    if !should_persist_import_transfer_heartbeat(last_emit) {
                        continue;
                    }
                    let Some(progress) = last_progress.clone() else {
                        continue;
                    };

                    match progress_app
                        .update_import_transfer_progress_and_notify(
                            &progress_import_id,
                            progress.phase,
                            progress.bytes,
                            progress.total_bytes,
                        )
                        .await
                    {
                        Ok(()) => {
                            last_phase = Some(progress.phase);
                            last_bytes = progress.bytes;
                            last_emit = Some(Instant::now());
                        }
                        Err(error) => {
                            tracing::warn!(
                                import_id = %progress_import_id,
                                error = %error,
                                "failed to persist import transfer heartbeat"
                            );
                        }
                    }
                }
            }
        }
    });

    let execution_context = crate::ImportFileExecutionContext::new(
        completed.map_or("", |item| item.client_id.as_str()),
        completed.map_or("", |item| item.client_type.as_str()),
    )
    .with_active_import_stream(active_stream.clone());
    let result = app
        .services
        .workflow
        .file_importer
        .import_file_with_execution_context(
            source,
            dest,
            mode,
            expected_source,
            Some(progress_tx),
            &permissions,
            &execution_context,
        )
        .await;

    if matches!(&result, Err(AppError::Canceled(_))) {
        let temporary_destination = dest.with_extension("tmp_import");
        if let Err(error) = std::fs::remove_file(&temporary_destination)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                import_id,
                path = %temporary_destination.display(),
                error = %error,
                "failed to remove cancelled import temporary destination"
            );
        }
    }

    if let Err(error) = progress_task.await {
        tracing::warn!(import_id, error = %error, "import transfer progress task failed");
    }

    active_stream.finish().await;

    let result = result?;
    let finalization_permit = app
        .runtime
        .imports
        .execution_coordinator
        .acquire_finalization()
        .await;
    Ok(CoordinatedImportFileResult {
        result,
        _finalization_permit: finalization_permit,
    })
}

impl AppUseCase {
    pub async fn list_active_import_streams(
        &self,
        actor: &User,
    ) -> AppResult<Vec<crate::ActiveImportStream>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        Ok(self
            .runtime
            .imports
            .active_streams
            .snapshot()
            .await
            .into_iter()
            .filter(|stream| allowed_library_ids.contains(&stream.library_id))
            .collect())
    }

    pub async fn subscribe_active_import_streams(
        &self,
        actor: &User,
    ) -> AppResult<tokio::sync::watch::Receiver<crate::ActiveImportStreamSync>> {
        if self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .is_empty()
        {
            return Err(AppError::Unauthorized(
                "You do not have access to any libraries".to_string(),
            ));
        }
        Ok(self.runtime.imports.active_streams.subscribe())
    }

    pub async fn cancel_active_import_stream(
        &self,
        actor: &User,
        stream_id: &str,
    ) -> AppResult<()> {
        let stream = self
            .runtime
            .imports
            .active_streams
            .get(stream_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("active import stream {stream_id}")))?;
        self.require_library_permission(
            actor,
            &stream.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        self.runtime
            .imports
            .active_streams
            .request_cancel(stream_id)
            .await
            .ok_or_else(|| {
                AppError::Validation("The import is no longer cancellable".to_string())
            })?;
        Ok(())
    }
}

pub(crate) struct CoordinatedImportFileResult {
    result: scryer_domain::ImportFileResult,
    _finalization_permit: tokio::sync::OwnedSemaphorePermit,
}

impl std::ops::Deref for CoordinatedImportFileResult {
    type Target = scryer_domain::ImportFileResult;

    fn deref(&self) -> &Self::Target {
        &self.result
    }
}

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("poller.rs");
include!("completed.rs");
include!("movie.rs");
include!("series_movie.rs");
include!("series_plan.rs");
include!("series.rs");
include!("paths.rs");
include!("metadata.rs");
include!("wanted.rs");
include!("manual.rs");
include!("results.rs");
include!("burned_source.rs");
include!("tests.rs");
