use std::path::{Path, PathBuf};

use crate::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootAvailabilityPolicy {
    RequireNonEmpty,
    AllowEmpty,
}

pub(crate) fn most_specific_containing_root(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| crate::recycle_bin::path_is_under_configured_root(path, root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

pub(crate) fn resolve_available_root_for_path(
    path: &Path,
    roots: &[PathBuf],
    policy: RootAvailabilityPolicy,
) -> AppResult<()> {
    let root = most_specific_containing_root(path, roots).ok_or_else(|| {
        AppError::Validation(format!(
            "refusing filesystem operation for {} because it is outside configured media roots",
            path.display()
        ))
    })?;
    ensure_root_available(&root, policy)?;
    Ok(())
}

pub(crate) fn ensure_root_available(root: &Path, policy: RootAvailabilityPolicy) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| {
        AppError::Validation(format!(
            "configured media root {} is unavailable: {}",
            root.display(),
            error
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(format!(
            "configured media root {} is a symlink",
            root.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(AppError::Validation(format!(
            "configured media root {} is not a directory",
            root.display()
        )));
    }

    let mut entries = std::fs::read_dir(root).map_err(|error| {
        AppError::Validation(format!(
            "configured media root {} is unreadable: {}",
            root.display(),
            error
        ))
    })?;
    if matches!(policy, RootAvailabilityPolicy::RequireNonEmpty) {
        match entries.next() {
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                return Err(AppError::Validation(format!(
                    "configured media root {} is unreadable: {}",
                    root.display(),
                    error
                )));
            }
            None => {
                return Err(AppError::Validation(format!(
                    "configured media root {} is empty",
                    root.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn clear_readonly_for_remove(path: &Path) -> AppResult<()> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect {} before delete: {}",
                path.display(),
                error
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        tokio::fs::set_permissions(path, permissions)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to clear read-only attribute on {}: {}",
                    path.display(),
                    error
                ))
            })?;
    }
    Ok(())
}

#[cfg(not(windows))]
async fn clear_readonly_for_remove(_path: &Path) -> AppResult<()> {
    Ok(())
}

pub(crate) async fn remove_file_safely(path: &Path) -> AppResult<()> {
    clear_readonly_for_remove(path).await?;
    tokio::fs::remove_file(path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to remove file {}: {}",
            path.display(),
            error
        ))
    })
}

pub(crate) async fn remove_file_safely_if_exists(path: &Path) -> AppResult<()> {
    clear_readonly_for_remove(path).await?;
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove file {}: {}",
            path.display(),
            error
        ))),
    }
}

pub(crate) async fn remove_dir_safely(path: &Path) -> AppResult<()> {
    clear_readonly_for_remove(path).await?;
    tokio::fs::remove_dir(path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to remove directory {}: {}",
            path.display(),
            error
        ))
    })
}

#[cfg(windows)]
async fn clear_readonly_tree_for_remove(path: &Path) -> AppResult<()> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect {} before recursive delete: {}",
                path.display(),
                error
            )));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return clear_readonly_for_remove(path).await;
    }

    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        clear_readonly_for_remove(&dir).await?;
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(|error| {
            AppError::Repository(format!(
                "failed to read directory {} before recursive delete: {}",
                dir.display(),
                error
            ))
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            AppError::Repository(format!(
                "failed to read directory entry in {} before recursive delete: {}",
                dir.display(),
                error
            ))
        })? {
            let child = entry.path();
            let child_metadata = tokio::fs::symlink_metadata(&child).await.map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect {} before recursive delete: {}",
                    child.display(),
                    error
                ))
            })?;
            if child_metadata.is_dir() && !child_metadata.file_type().is_symlink() {
                stack.push(child);
            } else {
                clear_readonly_for_remove(&child).await?;
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
async fn clear_readonly_tree_for_remove(_path: &Path) -> AppResult<()> {
    Ok(())
}

pub(crate) async fn remove_dir_all_safely(path: &Path) -> AppResult<()> {
    clear_readonly_tree_for_remove(path).await?;
    tokio::fs::remove_dir_all(path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to remove directory tree {}: {}",
            path.display(),
            error
        ))
    })
}

pub(crate) async fn remove_dir_all_safely_if_exists(path: &Path) -> AppResult<()> {
    clear_readonly_tree_for_remove(path).await?;
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove directory tree {}: {}",
            path.display(),
            error
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_availability_requires_content_for_user_disk_delete_policy() {
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let target = tempdir.path().join("missing.mkv");
        let roots = vec![tempdir.path().to_path_buf()];

        let empty_result = resolve_available_root_for_path(
            &target,
            &roots,
            RootAvailabilityPolicy::RequireNonEmpty,
        );
        assert!(
            matches!(empty_result, Err(AppError::Validation(_))),
            "empty roots should fail closed for destructive user disk deletes"
        );

        std::fs::write(tempdir.path().join(".mounted"), b"mounted").expect("write mount marker");
        resolve_available_root_for_path(&target, &roots, RootAvailabilityPolicy::RequireNonEmpty)
            .expect("non-empty roots should prove availability even when target is missing");
    }

    #[test]
    fn root_availability_allows_empty_roots_for_db_only_housekeeping_policy() {
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let target = tempdir.path().join("missing.mkv");
        let roots = vec![tempdir.path().to_path_buf()];

        resolve_available_root_for_path(&target, &roots, RootAvailabilityPolicy::AllowEmpty)
            .expect("DB-only housekeeping may clean stale rows under an empty mounted root");
    }

    #[test]
    fn root_availability_rejects_out_of_root_targets() {
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let outside = tempdir
            .path()
            .parent()
            .expect("temp dir has parent")
            .join("outside.mkv");
        let roots = vec![tempdir.path().join("media")];

        let result =
            resolve_available_root_for_path(&outside, &roots, RootAvailabilityPolicy::AllowEmpty);
        assert!(
            matches!(result, Err(AppError::Validation(_))),
            "targets outside configured roots must fail closed"
        );
    }
}
