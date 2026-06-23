use std::path::{Path, PathBuf};

use async_trait::async_trait;
use scryer_application::stored_paths::stored_path_to_path_buf;
use scryer_application::{
    AppError, AppResult, LibraryRenamer, RenameApplyItemResult, RenameApplyStatus, RenamePlan,
    RenameWriteAction,
};
#[cfg(windows)]
use scryer_domain::Id;
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub struct FileSystemLibraryRenamer;

impl Default for FileSystemLibraryRenamer {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemLibraryRenamer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LibraryRenamer for FileSystemLibraryRenamer {
    async fn validate_targets(&self, plan: &RenamePlan) -> AppResult<()> {
        for item in &plan.items {
            if matches!(item.write_action, RenameWriteAction::Replace) {
                return Err(AppError::Validation(
                    "rename replace action is not supported".into(),
                ));
            }

            if !matches!(item.write_action, RenameWriteAction::Move) {
                continue;
            }

            let source = stored_path_to_path_buf(&item.current_path);
            let source_meta = fs::metadata(source)
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;
            if !source_meta.is_file() {
                return Err(AppError::Validation(format!(
                    "rename source is not a file: {}",
                    item.current_path
                )));
            }

            let Some(target_path) = item.proposed_path.as_deref() else {
                return Err(AppError::Validation(
                    "rename target path is required for move/replace actions".into(),
                ));
            };

            let target = stored_path_to_path_buf(target_path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|err| AppError::Repository(err.to_string()))?;
            }

            if !rename_paths_equivalent(&item.current_path, target_path)
                && fs::metadata(target).await.is_ok()
            {
                return Err(AppError::Validation(format!(
                    "rename target already exists: {target_path}"
                )));
            }
        }

        Ok(())
    }

    async fn apply_plan(&self, plan: &RenamePlan) -> AppResult<Vec<RenameApplyItemResult>> {
        let mut out = Vec::with_capacity(plan.items.len());

        for item in &plan.items {
            let mut result = RenameApplyItemResult {
                collection_id: item.collection_id.clone(),
                media_file_id: item.media_file_id.clone(),
                series_movie_link_ids: item.series_movie_link_ids.clone(),
                current_path: item.current_path.clone(),
                proposed_path: item.proposed_path.clone(),
                final_path: None,
                write_action: item.write_action.clone(),
                status: RenameApplyStatus::Skipped,
                reason_code: item.reason_code.clone(),
                error_message: None,
            };

            match item.write_action {
                RenameWriteAction::Noop => {
                    result.status = RenameApplyStatus::Skipped;
                    result.final_path = item.proposed_path.clone();
                }
                RenameWriteAction::Skip => {
                    result.status = RenameApplyStatus::Skipped;
                }
                RenameWriteAction::Error => {
                    result.status = RenameApplyStatus::Failed;
                }
                RenameWriteAction::Replace => {
                    result.status = RenameApplyStatus::Failed;
                    result.reason_code = "replace_not_supported".into();
                    result.error_message = Some("rename replace action is not supported".into());
                }
                RenameWriteAction::Move => {
                    let Some(target) = item.proposed_path.as_deref() else {
                        result.status = RenameApplyStatus::Failed;
                        result.reason_code = "missing_target".into();
                        result.error_message =
                            Some("rename target path missing for move action".into());
                        out.push(result);
                        continue;
                    };

                    match move_file(&item.current_path, target, false).await {
                        Ok(()) => {
                            result.status = RenameApplyStatus::Applied;
                            result.final_path = Some(target.to_string());
                        }
                        Err(err) => {
                            result.status = RenameApplyStatus::Failed;
                            result.reason_code = "rename_io_failed".into();
                            result.error_message = Some(err.to_string());
                        }
                    }
                }
            }

            out.push(result);
        }

        Ok(out)
    }

    async fn rollback(
        &self,
        applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        for item in applied_items.iter().rev() {
            if !matches!(item.write_action, RenameWriteAction::Move) {
                continue;
            }

            let Some(final_path) = item.final_path.as_deref() else {
                continue;
            };

            if final_path == item.current_path {
                continue;
            }

            move_file(final_path, &item.current_path, false)
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;
        }

        Ok(applied_items.to_vec())
    }
}

async fn move_file(source: &str, target: &str, replace: bool) -> std::io::Result<()> {
    let source_path = stored_path_to_path_buf(source);
    let target_path = stored_path_to_path_buf(target);

    if replace
        && !rename_paths_equivalent(source, target)
        && fs::metadata(&target_path).await.is_ok()
    {
        fs::remove_file(&target_path).await?;
    }

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    #[cfg(windows)]
    if requires_case_only_intermediate_move(source, target) {
        return move_case_only_file(&source_path, &target_path).await;
    }

    match fs::rename(&source_path, &target_path).await {
        Ok(()) => Ok(()),
        Err(err) if is_cross_device_error(&err) => {
            fs::copy(&source_path, &target_path).await?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .open(&target_path)
                .await?;
            file.flush().await?;
            file.sync_all().await?;
            // Prove the destination is a faithful copy using the sampled size +
            // first/last MiB BLAKE3 verifier before deleting the source.
            if let Err(verify_error) =
                scryer_application::fs_integrity::verify_same_file_async(&source_path, &target_path)
                    .await
            {
                let _ = fs::remove_file(&target_path).await;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    verify_error.to_string(),
                ));
            }
            fs::remove_file(&source_path).await?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn is_cross_device_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(18) | Some(17))
}

fn rename_paths_equivalent(source: &str, target: &str) -> bool {
    rename_path_key(source) == rename_path_key(target)
}

#[cfg(windows)]
fn requires_case_only_intermediate_move(source: &str, target: &str) -> bool {
    source != target && rename_paths_equivalent(source, target)
}

#[cfg(windows)]
fn rename_path_key(stored_path: &str) -> String {
    lexically_normalize(&stored_path_to_path_buf(stored_path))
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
}

#[cfg(not(windows))]
fn rename_path_key(stored_path: &str) -> String {
    lexically_normalize(&stored_path_to_path_buf(stored_path))
        .to_string_lossy()
        .into_owned()
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

#[cfg(windows)]
async fn move_case_only_file(source: &Path, target: &Path) -> std::io::Result<()> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let mut last_claim_error = None;
    for _ in 0..10 {
        let id = Id::new().0;
        let short_id = &id[..8];
        let intermediate = parent.join(format!(".scryer-rename-{short_id}.tmp"));
        if fs::metadata(&intermediate).await.is_ok() {
            continue;
        }

        match fs::rename(source, &intermediate).await {
            Ok(()) => {
                return match fs::rename(&intermediate, target).await {
                    Ok(()) => Ok(()),
                    Err(rename_error) => {
                        let rollback_result = fs::rename(&intermediate, source).await;
                        if let Err(rollback_error) = rollback_result {
                            return Err(std::io::Error::new(
                                rename_error.kind(),
                                format!(
                                    "failed case-only rename {} -> {} after moving through {}; rollback to {} also failed: {}; original error: {}",
                                    source.display(),
                                    target.display(),
                                    intermediate.display(),
                                    source.display(),
                                    rollback_error,
                                    rename_error
                                ),
                            ));
                        }
                        Err(rename_error)
                    }
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_claim_error = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_claim_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "failed to claim intermediate path for case-only rename {} -> {}",
                source.display(),
                target.display()
            ),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_device_error_matches_unix_and_windows_codes() {
        assert!(is_cross_device_error(&std::io::Error::from_raw_os_error(
            18
        )));
        assert!(is_cross_device_error(&std::io::Error::from_raw_os_error(
            17
        )));
        assert!(!is_cross_device_error(&std::io::Error::from_raw_os_error(
            5
        )));
    }

    #[cfg(not(windows))]
    #[test]
    fn rename_path_key_preserves_case_on_non_windows() {
        assert_ne!(
            rename_path_key("/media/Movie.mkv"),
            rename_path_key("/media/movie.mkv")
        );
    }

    #[cfg(windows)]
    #[test]
    fn rename_path_key_folds_case_and_separators_on_windows() {
        assert_eq!(
            rename_path_key(r"C:\Media\Movie.mkv"),
            rename_path_key("C:/media/movie.mkv")
        );
    }
}
