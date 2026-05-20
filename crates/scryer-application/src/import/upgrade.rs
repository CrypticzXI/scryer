//! Quality-upgrade workflow for media files.
//!
//! When a new import scores higher than an existing file for the same title,
//! the old file is recycled and the new one takes its place. If the new import
//! fails, the old file is restored from the recycle bin to avoid data loss.

use crate::domain_events::{
    created_media_update, deleted_media_update, modified_media_update, new_title_domain_event,
    title_context_snapshot,
};
use crate::recycle_bin::{self, RecycleBinConfig, RecycleManifest};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::types::TitleMediaFile;
use crate::{AppError, AppResult, AppUseCase, InsertMediaFileInput};
use scryer_domain::{DomainEventPayload, MediaFileUpgradedEventData, Title, User};

/// Result of a successful upgrade operation.
#[derive(Debug)]
pub struct UpgradeOutcome {
    pub old_score: i32,
    pub new_score: i32,
    pub new_file_id: String,
}

pub enum UpgradeResult {
    Upgraded(UpgradeOutcome),
    Rejected(crate::post_download_gate::ImportedFileRejection),
}

/// Execute an atomic file upgrade: recycle old → import new → update DB.
///
/// If the new file import fails, the old file is restored from the recycle bin
/// so that we never lose both copies.
#[expect(
    clippy::too_many_arguments,
    reason = "upgrade execution coordinates file movement, scoring, and persistence state in one transaction"
)]
pub(crate) async fn execute_upgrade(
    app: &AppUseCase,
    _actor: &User,
    title: &Title,
    existing_file: &TitleMediaFile,
    source_path: &std::path::Path,
    dest_path: &std::path::Path,
    prepared: &crate::post_download_gate::PreparedImportCandidate,
    stored_quality_label: Option<&str>,
    final_score: i32,
    old_score: i32,
    target_episode_ids: &[String],
    recycle_config: &RecycleBinConfig,
) -> AppResult<UpgradeResult> {
    let old_path = stored_path_to_path_buf(&existing_file.file_path);
    let dest_path_string = path_to_stored_string(dest_path);
    let source_path_string = path_to_stored_string(source_path);

    let scoring_log = format!(
        "upgrade {} → {} (delta {}){}",
        old_score,
        final_score,
        final_score - old_score,
        if prepared.rescore_changes.is_empty() {
            String::new()
        } else {
            format!("; rescore: {}", prepared.rescore_changes.join(", "))
        }
    );

    // 3. Recycle the old file
    let manifest = RecycleManifest {
        recycled_at: chrono::Utc::now().to_rfc3339(),
        original_path: existing_file.file_path.clone(),
        size_bytes: existing_file.size_bytes as u64,
        title_id: Some(title.id.clone()),
        reason: "upgrade_replaced".to_string(),
    };
    let recycle_result = recycle_bin::recycle_file(recycle_config, &old_path, manifest).await?;

    // 4. Import the new file
    let import_result = app
        .services
        .workflow
        .file_importer
        .import_file(source_path, dest_path)
        .await;

    let file_result = match import_result {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(
                error = %err,
                old_path = %old_path.display(),
                new_source = %source_path.display(),
                "upgrade import failed, restoring old file"
            );
            restore_old_file(&recycle_result, &old_path).await;
            return Err(AppError::Repository(format!(
                "upgrade import failed: {err}"
            )));
        }
    };

    // 5. Delete old media_files record
    let old_file_id = existing_file.id.clone();
    let old_episode_id = existing_file.episode_id.clone();
    if let Err(err) = app
        .services
        .library
        .media_files
        .delete_media_file(&old_file_id)
        .await
    {
        remove_imported_replacement(dest_path).await;
        restore_old_file(&recycle_result, &old_path).await;
        return Err(AppError::Repository(format!(
            "failed to delete old media file record during upgrade: {err}"
        )));
    }

    // 6. Insert new record with rich schema
    let media_file_input = InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: dest_path_string.clone(),
        size_bytes: file_result.size_bytes as i64,
        quality_label: stored_quality_label
            .map(str::to_string)
            .or_else(|| prepared.parsed.quality.clone()),
        scene_name: Some(prepared.parsed.raw_title.clone()),
        release_group: prepared.parsed.release_group.clone(),
        source_type: prepared.parsed.source.clone(),
        resolution: stored_quality_label
            .map(str::to_string)
            .or_else(|| prepared.parsed.quality.clone()),
        video_codec_parsed: prepared.parsed.video_codec.clone(),
        audio_codec_parsed: prepared.parsed.audio.clone(),
        audio_channels_parsed: prepared.parsed.audio_channels.clone(),
        original_file_path: Some(source_path_string),
        acquisition_score: Some(final_score),
        scoring_log: Some(scoring_log.clone()),
        ..Default::default()
    };
    let new_file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await?;
    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        &new_file_id,
        prepared.accepted.as_ref(),
    )
    .await;

    // 7. Re-link episode mappings.
    if target_episode_ids.is_empty() {
        if let Some(ref episode_id) = old_episode_id {
            let _ = app
                .services
                .library
                .media_files
                .link_file_to_episode(&new_file_id, episode_id)
                .await;
        }
    } else {
        for episode_id in target_episode_ids {
            let _ = app
                .services
                .library
                .media_files
                .link_file_to_episode(&new_file_id, episode_id)
                .await;
        }
    }

    {
        let media_updates = if existing_file.file_path == dest_path_string {
            vec![modified_media_update(dest_path_string.clone())]
        } else {
            vec![
                deleted_media_update(existing_file.file_path.clone()),
                created_media_update(dest_path_string.clone()),
            ]
        };
        app.append_domain_event(new_title_domain_event(
            None,
            title,
            DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
                title: title_context_snapshot(title),
                media_updates,
                previous_file_id: Some(existing_file.id.clone()),
                current_file_id: Some(new_file_id.clone()),
                old_score: Some(old_score),
                new_score: Some(final_score),
            }),
        ))
        .await?;
    }

    Ok(UpgradeResult::Upgraded(UpgradeOutcome {
        old_score,
        new_score: final_score,
        new_file_id,
    }))
}

async fn remove_imported_replacement(dest_path: &std::path::Path) {
    if let Err(remove_err) = tokio::fs::remove_file(dest_path).await
        && remove_err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::error!(
            error = %remove_err,
            path = %dest_path.display(),
            "failed to remove imported replacement after upgrade database failure"
        );
    }
}

async fn restore_old_file(
    recycle_result: &Option<recycle_bin::RecycleResult>,
    old_path: &std::path::Path,
) {
    if let Some(ref recycle_result) = *recycle_result
        && let Err(restore_err) =
            recycle_bin::restore_from_recycle(&recycle_result.recycled_path, old_path).await
    {
        tracing::error!(
            error = %restore_err,
            recycled = %recycle_result.recycled_path.display(),
            "CRITICAL: failed to restore recycled file after upgrade failure"
        );
    }
}
