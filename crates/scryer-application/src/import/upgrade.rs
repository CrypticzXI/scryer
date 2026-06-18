//! Quality-upgrade workflow for media files.
//!
//! When a new import scores higher than an existing file for the same title,
//! the replacement is imported and validated before the old file is recycled
//! or deleted.

use crate::domain_events::{
    DomainEventActor, created_media_update, deleted_media_update, modified_media_update,
    new_title_domain_event, title_context_snapshot,
};
use crate::recycle_bin::{self, RecycleBinConfig, RecycleManifest};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::types::TitleMediaFile;
use crate::{AppError, AppResult, AppUseCase, InsertMediaFileInput};
use scryer_domain::{
    DomainEventPayload, ImportMode, ImportSourceCleanupGuard, MediaFileDeletedEventData,
    MediaFileDeletedReason, MediaFileUpgradedEventData, Title, User,
};
use std::path::{Path, PathBuf};

/// Result of a successful upgrade operation.
#[derive(Debug)]
pub struct UpgradeOutcome {
    pub old_score: i32,
    pub new_score: i32,
    pub new_file_id: String,
    pub recycle_entry_committed: bool,
}

pub enum UpgradeResult {
    Upgraded(UpgradeOutcome),
    Rejected(crate::post_download_gate::ImportedFileRejection),
}

/// Execute a guarded file upgrade: import and validate replacement, then retire old.
///
/// The old file is not recycled or deleted until the replacement file is on disk,
/// represented in storage, linked, and validated.
#[expect(
    clippy::too_many_arguments,
    reason = "upgrade execution coordinates file movement, scoring, and persistence state in one transaction"
)]
pub(crate) async fn execute_upgrade(
    app: &AppUseCase,
    actor: &User,
    title: &Title,
    existing_file: &TitleMediaFile,
    source_path: &std::path::Path,
    dest_path: &std::path::Path,
    prepared: &crate::post_download_gate::PreparedImportCandidate,
    stored_quality_label: Option<&str>,
    final_score: i32,
    old_score: i32,
    target_episode_ids: &[String],
    media_root: Option<&str>,
    recycle_config: &RecycleBinConfig,
    import_mode: ImportMode,
) -> AppResult<UpgradeResult> {
    ensure_old_file_disposition_ready(recycle_config)?;
    let audit_actor = DomainEventActor::from(actor);

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

    let same_final_path = old_path == dest_path;
    let import_path = if same_final_path {
        sibling_guard_path(dest_path, "replacement")
    } else {
        dest_path.to_path_buf()
    };

    let replacement = prepare_replacement_before_old_removal(
        app,
        title,
        existing_file,
        source_path,
        &import_path,
        dest_path_string.clone(),
        same_final_path,
        prepared,
        stored_quality_label,
        final_score,
        target_episode_ids,
        media_root,
        &scoring_log,
        &source_path_string,
        import_mode,
    )
    .await?;

    let recycle_entry_committed = finalize_prepared_upgrade(
        app,
        title,
        existing_file,
        &replacement,
        recycle_config,
        &old_path,
        media_root,
    )
    .await?;

    append_upgrade_event(
        app,
        audit_actor.clone(),
        title,
        existing_file,
        UpgradeEventDetails {
            new_file_id: &replacement.new_file_id,
            dest_path_string: &replacement.final_path_string,
            old_score,
            final_score,
        },
    )
    .await?;

    if recycle_entry_committed {
        append_upgrade_recycle_event(app, audit_actor.clone(), title, existing_file).await;
    }

    if import_mode == ImportMode::Move {
        remove_upgrade_import_source_after_verified_commit(app, &replacement).await?;
    }

    Ok(UpgradeResult::Upgraded(UpgradeOutcome {
        old_score,
        new_score: final_score,
        new_file_id: replacement.new_file_id,
        recycle_entry_committed,
    }))
}

struct PreparedUpgradeReplacement {
    new_file_id: String,
    import_path: PathBuf,
    final_path_string: String,
    same_final_path: bool,
    source_cleanup: Option<ImportSourceCleanupGuard>,
}

fn ensure_old_file_disposition_ready(recycle_config: &RecycleBinConfig) -> AppResult<()> {
    if recycle_config.enabled && !recycle_config.cleanup_enabled {
        return Err(AppError::Validation(format!(
            "refusing to upgrade because the recycle bin path is unsafe: {}",
            recycle_config
                .validation_error
                .as_deref()
                .unwrap_or("invalid recycle bin configuration")
        )));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "preparing a replacement needs import, metadata, scoring, and episode-link context"
)]
async fn prepare_replacement_before_old_removal(
    app: &AppUseCase,
    title: &Title,
    existing_file: &TitleMediaFile,
    source_path: &Path,
    import_path: &Path,
    final_path_string: String,
    same_final_path: bool,
    prepared: &crate::post_download_gate::PreparedImportCandidate,
    stored_quality_label: Option<&str>,
    final_score: i32,
    target_episode_ids: &[String],
    media_root: Option<&str>,
    scoring_log: &str,
    source_path_string: &str,
    import_mode: ImportMode,
) -> AppResult<PreparedUpgradeReplacement> {
    let import_path_string = path_to_stored_string(import_path);
    let file_result = app
        .services
        .workflow
        .file_importer
        .import_file(
            source_path,
            import_path,
            import_mode,
            Some(&prepared.source_snapshot),
        )
        .await
        .map_err(|err| {
            AppError::Repository(format!(
                "upgrade import failed before old file removal: {err}"
            ))
        })?;

    let media_file_input = InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: import_path_string.clone(),
        size_bytes: file_result.size_bytes as i64,
        quality_label: stored_quality_label
            .map(str::to_string)
            .or_else(|| prepared.parsed.quality.clone()),
        scene_name: Some(prepared.parsed.raw_title.clone()),
        release_group: prepared.parsed.release_group.clone(),
        source_type: crate::release_parser::parsed_release_source_type(&prepared.parsed),
        resolution: stored_quality_label
            .map(str::to_string)
            .or_else(|| prepared.parsed.quality.clone()),
        video_codec_parsed: prepared.parsed.video_codec,
        audio_codec_parsed: prepared.parsed.audio.as_ref().map(ToString::to_string),
        audio_channels_parsed: prepared.parsed.audio_channels.clone(),
        original_file_path: Some(source_path_string.to_string()),
        acquisition_score: Some(final_score),
        scoring_log: Some(scoring_log.to_string()),
        ..Default::default()
    };
    let new_file_id = match app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await
    {
        Ok(id) => id,
        Err(err) => {
            remove_imported_replacement(import_path).await;
            return Err(AppError::Repository(format!(
                "failed to insert replacement media file before old file removal: {err}"
            )));
        }
    };
    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        &new_file_id,
        prepared.accepted.as_ref(),
    )
    .await;

    if !write_replacement_episode_links(app, &new_file_id, existing_file, target_episode_ids).await
    {
        rollback_new_replacement(app, &new_file_id, import_path).await;
        return Err(AppError::Repository(
            "failed to link replacement media file before old file removal".to_string(),
        ));
    }

    if let Err(reason) = validate_replacement_media_file(
        app,
        &new_file_id,
        &import_path_string,
        &title.id,
        media_root,
    )
    .await
    {
        rollback_new_replacement(app, &new_file_id, import_path).await;
        return Err(AppError::Repository(format!(
            "replacement validation failed before old file removal: {reason}"
        )));
    }

    Ok(PreparedUpgradeReplacement {
        new_file_id,
        import_path: import_path.to_path_buf(),
        final_path_string,
        same_final_path,
        source_cleanup: file_result.source_cleanup,
    })
}

async fn finalize_prepared_upgrade(
    app: &AppUseCase,
    title: &Title,
    existing_file: &TitleMediaFile,
    replacement: &PreparedUpgradeReplacement,
    recycle_config: &RecycleBinConfig,
    old_path: &Path,
    media_root: Option<&str>,
) -> AppResult<bool> {
    if replacement.same_final_path {
        finalize_same_path_upgrade(
            app,
            title,
            existing_file,
            replacement,
            recycle_config,
            old_path,
            media_root,
        )
        .await
    } else {
        finalize_distinct_path_upgrade(
            app,
            title,
            existing_file,
            replacement,
            recycle_config,
            old_path,
            media_root,
        )
        .await
    }
}

async fn finalize_distinct_path_upgrade(
    app: &AppUseCase,
    title: &Title,
    existing_file: &TitleMediaFile,
    replacement: &PreparedUpgradeReplacement,
    recycle_config: &RecycleBinConfig,
    old_path: &Path,
    media_root: Option<&str>,
) -> AppResult<bool> {
    if let Err(error) = app
        .services
        .library
        .media_files
        .replace_media_file_for_upgrade(
            &existing_file.id,
            &replacement.new_file_id,
            &replacement.final_path_string,
        )
        .await
    {
        rollback_new_replacement(app, &replacement.new_file_id, &replacement.import_path).await;
        return Err(AppError::Repository(format!(
            "failed to replace media file record after replacement validation: {error}"
        )));
    }
    validate_replacement_media_file(
        app,
        &replacement.new_file_id,
        &replacement.final_path_string,
        &title.id,
        media_root,
    )
    .await
    .map_err(|reason| {
        AppError::Repository(format!(
            "replacement validation failed after old row removal; old file left in place: {reason}"
        ))
    })?;
    validate_original_inactive_for_delete(
        app,
        &existing_file.id,
        &existing_file.file_path,
        &replacement.new_file_id,
    )
    .await
    .map_err(|reason| {
        AppError::Repository(format!(
            "old file deletion blocked after replacement validation: {reason}"
        ))
    })?;

    dispose_old_file_after_verified_upgrade(
        recycle_config,
        existing_file,
        old_path,
        &existing_file.file_path,
        title,
        media_root,
        &replacement.new_file_id,
        Path::new(&replacement.final_path_string),
    )
    .await
}

async fn finalize_same_path_upgrade(
    app: &AppUseCase,
    title: &Title,
    existing_file: &TitleMediaFile,
    replacement: &PreparedUpgradeReplacement,
    recycle_config: &RecycleBinConfig,
    old_path: &Path,
    media_root: Option<&str>,
) -> AppResult<bool> {
    let backup_path = sibling_guard_path(old_path, "old");

    if let Err(error) =
        swap_staged_replacement_into_place(old_path, &replacement.import_path, &backup_path).await
    {
        rollback_new_replacement(app, &replacement.new_file_id, &replacement.import_path).await;
        return Err(error);
    }

    if let Err(error) = app
        .services
        .library
        .media_files
        .replace_media_file_for_upgrade(
            &existing_file.id,
            &replacement.new_file_id,
            &replacement.final_path_string,
        )
        .await
    {
        restore_same_path_backup(old_path, &backup_path).await;
        rollback_new_replacement(app, &replacement.new_file_id, &replacement.import_path).await;
        return Err(AppError::Repository(format!(
            "failed to replace same-path media file record after guarded swap: {error}"
        )));
    }

    if let Err(reason) = validate_replacement_media_file(
        app,
        &replacement.new_file_id,
        &replacement.final_path_string,
        &title.id,
        media_root,
    )
    .await
    {
        return Err(AppError::Repository(format!(
            "replacement validation failed after same-path swap; old file kept at {}: {reason}",
            backup_path.display()
        )));
    }
    validate_original_inactive_for_delete(
        app,
        &existing_file.id,
        &existing_file.file_path,
        &replacement.new_file_id,
    )
    .await
    .map_err(|reason| {
        AppError::Repository(format!(
            "same-path old file deletion blocked after replacement validation; old file kept at {}: {reason}",
            backup_path.display()
        ))
    })?;

    dispose_old_file_after_verified_upgrade(
        recycle_config,
        existing_file,
        &backup_path,
        &existing_file.file_path,
        title,
        media_root,
        &replacement.new_file_id,
        Path::new(&replacement.final_path_string),
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "old-file disposition needs original, replacement, and recycle context"
)]
async fn dispose_old_file_after_verified_upgrade(
    recycle_config: &RecycleBinConfig,
    existing_file: &TitleMediaFile,
    old_file_source_path: &Path,
    manifest_original_path: &str,
    title: &Title,
    media_root: Option<&str>,
    replacement_file_id: &str,
    replacement_path: &Path,
) -> AppResult<bool> {
    if !recycle_config.enabled {
        remove_old_file_after_verified_upgrade(old_file_source_path).await?;
        return Ok(false);
    }

    let manifest = RecycleManifest::pending_upgrade(
        manifest_original_path.to_string(),
        existing_file.id.clone(),
        existing_file.size_bytes as u64,
        title.id.clone(),
        media_root.map(str::to_string),
    );
    let recycle_result =
        recycle_bin::recycle_file(recycle_config, old_file_source_path, manifest).await?;

    if recycle_result.is_none() {
        return Ok(false);
    }

    if let Err(error) =
        recycle_bin::commit_recycle_entry(&recycle_result, replacement_file_id, replacement_path)
            .await
    {
        tracing::warn!(
            error = %error,
            file_id = %replacement_file_id,
            "replacement imported but recycle entry could not be committed; it will not auto-purge"
        );
        return Ok(false);
    }

    Ok(true)
}

async fn write_replacement_episode_links(
    app: &AppUseCase,
    new_file_id: &str,
    existing_file: &TitleMediaFile,
    target_episode_ids: &[String],
) -> bool {
    let mut links_written = true;
    if target_episode_ids.is_empty() {
        if let Some(ref episode_id) = existing_file.episode_id
            && let Err(error) = app
                .services
                .library
                .media_files
                .link_file_to_episode(new_file_id, episode_id)
                .await
        {
            links_written = false;
            tracing::warn!(
                error = %error,
                file_id = %new_file_id,
                episode_id,
                "failed to link replacement file to episode"
            );
        }
    } else {
        for episode_id in target_episode_ids {
            if let Err(error) = app
                .services
                .library
                .media_files
                .link_file_to_episode(new_file_id, episode_id)
                .await
            {
                links_written = false;
                tracing::warn!(
                    error = %error,
                    file_id = %new_file_id,
                    episode_id,
                    "failed to link replacement file to episode"
                );
            }
        }
    }
    links_written
}

async fn validate_replacement_media_file(
    app: &AppUseCase,
    replacement_file_id: &str,
    replacement_path: &str,
    title_id: &str,
    media_root: Option<&str>,
) -> Result<(), String> {
    let replacement = app
        .services
        .library
        .media_files
        .get_media_file_by_id(replacement_file_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "replacement media file row is missing".to_string())?;

    if replacement.file_path != replacement_path {
        return Err(format!(
            "replacement media file path mismatch: expected={} db={}",
            replacement_path, replacement.file_path
        ));
    }
    if !stored_path_to_path_buf(&replacement.file_path).exists() {
        return Err(format!(
            "replacement media file does not exist on disk: {}",
            replacement.file_path
        ));
    }
    if replacement.title_id != title_id {
        return Err(format!(
            "replacement title mismatch: expected={} db={}",
            title_id, replacement.title_id
        ));
    }
    if let Some(media_root) = media_root.map(str::trim).filter(|root| !root.is_empty())
        && !stored_path_to_path_buf(&replacement.file_path).starts_with(media_root)
    {
        return Err(format!(
            "replacement path is outside media root: replacement={} root={}",
            replacement.file_path, media_root
        ));
    }

    Ok(())
}

async fn validate_original_inactive_for_delete(
    app: &AppUseCase,
    original_file_id: &str,
    original_path: &str,
    replacement_file_id: &str,
) -> Result<(), String> {
    if app
        .services
        .library
        .media_files
        .get_media_file_by_id(original_file_id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("original media file row is still active".to_string());
    }

    if let Some(active_at_original_path) = app
        .services
        .library
        .media_files
        .get_media_file_by_path(original_path)
        .await
        .map_err(|error| error.to_string())?
        && active_at_original_path.id != replacement_file_id
    {
        return Err(format!(
            "original path is active for a different media file: {}",
            active_at_original_path.id
        ));
    }

    Ok(())
}

async fn rollback_new_replacement(app: &AppUseCase, new_file_id: &str, path: &Path) {
    let _ = app
        .services
        .library
        .media_files
        .delete_media_file(new_file_id)
        .await;
    remove_imported_replacement(path).await;
}

fn sibling_guard_path(path: &Path, label: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("media"));
    parent.join(format!(
        ".scryer-upgrade-{}-{}-{}",
        label,
        scryer_domain::Id::new().0,
        file_name.to_string_lossy()
    ))
}

async fn swap_staged_replacement_into_place(
    final_path: &Path,
    staged_replacement_path: &Path,
    backup_path: &Path,
) -> AppResult<()> {
    tokio::fs::rename(final_path, backup_path)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to move old file aside before same-path upgrade: {} -> {}: {}",
                final_path.display(),
                backup_path.display(),
                error
            ))
        })?;

    if let Err(error) = tokio::fs::rename(staged_replacement_path, final_path).await {
        restore_same_path_backup(final_path, backup_path).await;
        return Err(AppError::Repository(format!(
            "failed to move verified replacement into final path: {} -> {}: {}",
            staged_replacement_path.display(),
            final_path.display(),
            error
        )));
    }

    Ok(())
}

async fn restore_same_path_backup(final_path: &Path, backup_path: &Path) {
    let _ = tokio::fs::remove_file(final_path).await;
    if let Err(error) = tokio::fs::rename(backup_path, final_path).await {
        tracing::error!(
            error = %error,
            backup = %backup_path.display(),
            final_path = %final_path.display(),
            "failed to restore old file after guarded same-path upgrade failure"
        );
    }
}

async fn remove_old_file_after_verified_upgrade(path: &Path) -> AppResult<()> {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(AppError::Repository(format!(
            "failed to remove old file after replacement validation {}: {}",
            path.display(),
            error
        )));
    }
    Ok(())
}

async fn remove_upgrade_import_source_after_verified_commit(
    app: &AppUseCase,
    replacement: &PreparedUpgradeReplacement,
) -> AppResult<()> {
    let guard = replacement.source_cleanup.clone().ok_or_else(|| {
        AppError::Repository(format!(
            "move upgrade did not return a source cleanup guard for {}",
            replacement.import_path.display()
        ))
    })?;
    let final_path = stored_path_to_path_buf(&replacement.final_path_string);
    app.services
        .workflow
        .file_importer
        .remove_import_source_after_verified_import(guard, &final_path)
        .await
}

struct UpgradeEventDetails<'a> {
    new_file_id: &'a str,
    dest_path_string: &'a str,
    old_score: i32,
    final_score: i32,
}

async fn append_upgrade_event(
    app: &AppUseCase,
    actor: DomainEventActor,
    title: &Title,
    existing_file: &TitleMediaFile,
    details: UpgradeEventDetails<'_>,
) -> AppResult<()> {
    let media_updates = if existing_file.file_path == details.dest_path_string {
        vec![modified_media_update(details.dest_path_string.to_string())]
    } else {
        vec![
            deleted_media_update(existing_file.file_path.clone()),
            created_media_update(details.dest_path_string.to_string()),
        ]
    };
    app.append_domain_event(new_title_domain_event(
        actor,
        title,
        DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
            title: title_context_snapshot(title),
            media_updates,
            previous_file_id: Some(existing_file.id.clone()),
            current_file_id: Some(details.new_file_id.to_string()),
            old_score: Some(details.old_score),
            new_score: Some(details.final_score),
        }),
    ))
    .await
    .map(|_| ())
}

async fn append_upgrade_recycle_event(
    app: &AppUseCase,
    actor: DomainEventActor,
    title: &Title,
    existing_file: &TitleMediaFile,
) {
    let _ = app
        .append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                title: title_context_snapshot(title),
                media_updates: vec![deleted_media_update(existing_file.file_path.clone())],
                file_id: Some(existing_file.id.clone()),
                reason: MediaFileDeletedReason::UpgradeCleanup,
                episode_ids: Vec::new(),
            }),
        ))
        .await
        .inspect_err(|error| {
            tracing::warn!(
                error = %error,
                file_id = %existing_file.id,
                "old media file recycled during upgrade but audit event could not be recorded"
            );
        });
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
