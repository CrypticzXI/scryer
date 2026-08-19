use std::path::{Path, PathBuf};

use async_trait::async_trait;
use scryer_application::stored_paths::stored_path_to_path_buf;
use scryer_application::{
    AppError, AppResult, LibraryRenamer, RenameApplyItemResult, RenameApplyStatus, RenamePlan,
    RenameWriteAction,
};
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
    /// Reports whether the plan is shaped correctly. Per-file conditions are
    /// deliberately not checked here: they are re-checked immediately before
    /// each move, so one unusable file fails on its own instead of cancelling
    /// every other rename in the plan.
    async fn validate_targets(&self, plan: &RenamePlan) -> AppResult<()> {
        for item in &plan.items {
            if matches!(item.write_action, RenameWriteAction::Move) && item.proposed_path.is_none()
            {
                return Err(AppError::Validation(
                    "rename target path is required for move actions".into(),
                ));
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

                    // Checked here rather than up front so the window between
                    // the check and the write stays as small as the filesystem
                    // allows, and so a file that cannot move does not cancel
                    // the rest of the plan.
                    if let Err((reason_code, message)) =
                        prepare_move_target(&item.current_path, target).await
                    {
                        result.status = RenameApplyStatus::Failed;
                        result.reason_code = reason_code;
                        result.error_message = Some(message);
                        out.push(result);
                        continue;
                    }

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

/// Verifies one file can move and prepares its destination directory,
/// returning a per-item reason code and message when it cannot.
async fn prepare_move_target(
    current_path: &str,
    target_path: &str,
) -> Result<(), (String, String)> {
    let source = stored_path_to_path_buf(current_path);
    let source_meta = fs::symlink_metadata(&source).await.map_err(|err| {
        (
            "source_missing".to_string(),
            format!("rename source is unreadable: {current_path}: {err}"),
        )
    })?;
    if source_meta.is_dir() {
        return Err((
            "source_not_file".to_string(),
            format!("rename source is not a file: {current_path}"),
        ));
    }

    let target = stored_path_to_path_buf(target_path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            (
                "target_parent_unwritable".to_string(),
                format!("could not create {}: {err}", parent.display()),
            )
        })?;
    }

    // A destination that is the same file as the source is a case-only rename,
    // not a collision.
    let same_file = paths_are_same_file(&source, &target).await.unwrap_or(false);
    if !same_file
        && !rename_paths_equivalent(current_path, target_path)
        && fs::symlink_metadata(&target).await.is_ok()
    {
        return Err((
            "target_exists".to_string(),
            format!("rename target already exists: {target_path}"),
        ));
    }

    Ok(())
}

async fn move_file(source: &str, target: &str, replace: bool) -> std::io::Result<()> {
    let source_path = stored_path_to_path_buf(source);
    let target_path = stored_path_to_path_buf(target);

    // Case-only renames: on a case-insensitive volume (APFS, SMB, Windows) the
    // target name resolves to the source file itself, so "the target exists" is
    // not a collision and a direct rename can be a silent no-op. Route those
    // through a temporary name, the way Sonarr does, on every platform.
    if paths_are_same_file(&source_path, &target_path).await? && source_path != target_path {
        return move_case_only_file(&source_path, &target_path).await;
    }

    if replace
        && !rename_paths_equivalent(source, target)
        && fs::metadata(&target_path).await.is_ok()
    {
        fs::remove_file(&target_path).await?;
    }

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    match rename_without_clobbering(&source_path, &target_path).await {
        Ok(()) => Ok(()),
        Err(err) if is_cross_device_error(&err) => {
            transfer_across_devices(&source_path, &target_path).await
        }
        Err(err) => Err(err),
    }
}

/// Moves a file across devices, preserving a symlink rather than materializing
/// the data it points at, and proving the copy before the source is unlinked.
async fn transfer_across_devices(source_path: &Path, target_path: &Path) -> std::io::Result<()> {
    // A symlinked media file is a pointer, not the payload: copying it would
    // silently duplicate the whole file and drop the link.
    if fs::symlink_metadata(source_path).await?.is_symlink() {
        let link_target = fs::read_link(source_path).await?;
        create_symlink(&link_target, target_path).await?;
        fs::remove_file(source_path).await?;
        return Ok(());
    }

    fs::copy(source_path, target_path).await?;
    let file = fs::OpenOptions::new().write(true).open(target_path).await?;
    file.sync_all().await?;
    // Prove the destination is a faithful copy using the sampled size +
    // first/last MiB BLAKE3 verifier before deleting the source.
    if let Err(verify_error) =
        scryer_application::fs_integrity::verify_same_file_async(source_path, target_path).await
    {
        let _ = fs::remove_file(target_path).await;
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            verify_error.to_string(),
        ));
    }
    fs::remove_file(source_path).await?;
    Ok(())
}

#[cfg(unix)]
async fn create_symlink(link_target: &Path, at: &Path) -> std::io::Result<()> {
    fs::symlink(link_target, at).await
}

#[cfg(windows)]
async fn create_symlink(link_target: &Path, at: &Path) -> std::io::Result<()> {
    fs::symlink_file(link_target, at).await
}

/// Renames `source` onto `target`, failing rather than replacing anything that
/// is already there.
///
/// `rename(2)` replaces the destination silently, so a checked rename would
/// only narrow the window between the check and the write, not close it. The
/// kernel offers an exclusive rename on Linux and macOS; elsewhere, and on
/// filesystems that reject it (SMB and NFS commonly do), the destination name
/// is claimed with an exclusive create first, which fails if another writer
/// already holds it.
async fn rename_without_clobbering(source: &Path, target: &Path) -> std::io::Result<()> {
    if let Some(result) = exclusive_rename(source, target).await {
        return result;
    }

    let placeholder = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .await;
    match placeholder {
        Ok(_) => {}
        // The destination appeared between planning and now.
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Err(err),
        // The volume rejected the reservation for some other reason; a rename
        // may still work, and the pre-flight check remains the guard there.
        Err(_) => return fs::rename(source, target).await,
    }

    match fs::rename(source, target).await {
        Ok(()) => Ok(()),
        Err(err) => {
            // Do not leave the empty reservation behind.
            let _ = fs::remove_file(target).await;
            Err(err)
        }
    }
}

/// `Some` when the platform could answer with an exclusive rename, `None` when
/// the caller should fall back.
#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn exclusive_rename(source: &Path, target: &Path) -> Option<std::io::Result<()>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source_c = CString::new(source.as_os_str().as_bytes()).ok()?;
    let target_c = CString::new(target.as_os_str().as_bytes()).ok()?;
    let result = tokio::task::spawn_blocking(move || {
        // SAFETY: both paths are NUL-terminated and live for the call.
        let code = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::renameat2(
                    libc::AT_FDCWD,
                    source_c.as_ptr(),
                    libc::AT_FDCWD,
                    target_c.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            }
            #[cfg(target_os = "macos")]
            {
                libc::renamex_np(source_c.as_ptr(), target_c.as_ptr(), libc::RENAME_EXCL)
            }
        };
        if code == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    })
    .await
    .ok()?;

    match result {
        Err(ref err)
            if matches!(
                err.raw_os_error(),
                Some(libc::ENOSYS) | Some(libc::ENOTSUP) | Some(libc::EINVAL)
            ) =>
        {
            // Old kernel or a filesystem without exclusive rename.
            None
        }
        other => Some(other),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn exclusive_rename(_source: &Path, _target: &Path) -> Option<std::io::Result<()>> {
    None
}

/// True when both paths name the same file on disk, which is how a case-only
/// rename looks on a case-insensitive volume.
async fn paths_are_same_file(source: &Path, target: &Path) -> std::io::Result<bool> {
    let Ok(source_meta) = fs::symlink_metadata(source).await else {
        return Ok(false);
    };
    let Ok(target_meta) = fs::symlink_metadata(target).await else {
        return Ok(false);
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(source_meta.dev() == target_meta.dev() && source_meta.ino() == target_meta.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (source_meta, target_meta);
        Ok(rename_paths_equivalent(
            &source.to_string_lossy(),
            &target.to_string_lossy(),
        ))
    }
}

/// Cross-device rename. The bare numbers differ per platform: 18 is `EXDEV` on
/// Unix, while 17 is `ERROR_NOT_SAME_DEVICE` on Windows. Matching both
/// everywhere treated Unix `EEXIST` (17) as cross-device.
fn is_cross_device_error(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(error.raw_os_error(), Some(libc::EXDEV))
    }
    #[cfg(windows)]
    {
        // ERROR_NOT_SAME_DEVICE
        matches!(error.raw_os_error(), Some(17))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        false
    }
}

fn rename_paths_equivalent(source: &str, target: &str) -> bool {
    rename_path_key(source) == rename_path_key(target)
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

    /// 17 is `EEXIST` on Unix and `ERROR_NOT_SAME_DEVICE` on Windows, so it
    /// only means cross-device on Windows. Treating it as cross-device on Unix
    /// sent an existing-destination failure down the copy-and-delete path.
    #[test]
    fn cross_device_error_is_platform_specific() {
        #[cfg(unix)]
        {
            assert!(is_cross_device_error(&std::io::Error::from_raw_os_error(
                libc::EXDEV
            )));
            assert!(!is_cross_device_error(&std::io::Error::from_raw_os_error(
                libc::EEXIST
            )));
        }
        #[cfg(windows)]
        {
            assert!(is_cross_device_error(&std::io::Error::from_raw_os_error(
                17
            )));
        }
        assert!(!is_cross_device_error(&std::io::Error::from_raw_os_error(
            5
        )));
    }

    /// `rename(2)` replaces the destination silently, so this is the guarantee
    /// that stops a rename from destroying a file that appeared after planning.
    #[tokio::test]
    async fn move_file_refuses_to_replace_an_existing_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let target = dir.path().join("target.mkv");
        std::fs::write(&source, b"source-payload").expect("write source");
        std::fs::write(&target, b"target-payload").expect("write target");

        let error = move_file(&source.to_string_lossy(), &target.to_string_lossy(), false)
            .await
            .expect_err("an occupied destination must not be replaced");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);

        assert_eq!(
            std::fs::read(&target).expect("target still present"),
            b"target-payload",
            "the destination must keep its bytes"
        );
        assert_eq!(
            std::fs::read(&source).expect("source still present"),
            b"source-payload",
            "the source must stay put when the move is refused"
        );
    }

    /// On a case-insensitive volume the destination name resolves to the source
    /// file, which is a rename to perform rather than a collision to reject.
    #[tokio::test]
    async fn move_file_performs_a_case_only_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("one piece.mkv");
        let target = dir.path().join("ONE PIECE.mkv");
        std::fs::write(&source, b"payload").expect("write source");

        move_file(&source.to_string_lossy(), &target.to_string_lossy(), false)
            .await
            .expect("case-only rename should succeed");

        assert_eq!(
            std::fs::read(&target).expect("renamed file present"),
            b"payload"
        );
        let names = std::fs::read_dir(dir.path())
            .expect("list dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["ONE PIECE.mkv".to_string()]);
    }

    /// One unusable file used to abort the whole plan, so a single squatting
    /// destination cancelled every other rename in a bulk operation.
    #[tokio::test]
    async fn apply_plan_fails_only_the_item_it_cannot_move() {
        let dir = tempfile::tempdir().expect("tempdir");
        let blocked_source = dir.path().join("blocked.mkv");
        let blocked_target = dir.path().join("blocked-target.mkv");
        let movable_source = dir.path().join("movable.mkv");
        let movable_target = dir.path().join("movable-target.mkv");
        std::fs::write(&blocked_source, b"blocked").expect("write");
        std::fs::write(&blocked_target, b"occupied").expect("write");
        std::fs::write(&movable_source, b"movable").expect("write");

        let item = |current: &std::path::Path, proposed: &std::path::Path| {
            scryer_application::RenamePlanItem {
                collection_id: None,
                media_file_id: None,
                series_movie_link_ids: Vec::new(),
                current_path: current.to_string_lossy().into_owned(),
                proposed_path: Some(proposed.to_string_lossy().into_owned()),
                normalized_filename: None,
                collision: false,
                reason_code: "rename_move".to_string(),
                write_action: RenameWriteAction::Move,
                source_size_bytes: None,
                source_mtime_unix_ms: None,
            }
        };
        let plan = RenamePlan {
            facet: scryer_domain::MediaFacet::Movie,
            title_id: None,
            template: String::new(),
            collision_policy: scryer_application::RenameCollisionPolicy::Skip,
            missing_metadata_policy: scryer_application::RenameMissingMetadataPolicy::FallbackTitle,
            fingerprint: String::new(),
            total: 2,
            renamable: 2,
            noop: 0,
            conflicts: 0,
            errors: 0,
            items: vec![
                item(&blocked_source, &blocked_target),
                item(&movable_source, &movable_target),
            ],
        };

        let results = FileSystemLibraryRenamer::new()
            .apply_plan(&plan)
            .await
            .expect("apply should report per-item outcomes");

        assert_eq!(results[0].status, RenameApplyStatus::Failed);
        assert_eq!(results[0].reason_code, "target_exists");
        assert_eq!(results[1].status, RenameApplyStatus::Applied);
        assert_eq!(
            std::fs::read(&blocked_target).expect("occupied target intact"),
            b"occupied"
        );
        assert_eq!(
            std::fs::read(&movable_target).expect("the movable file still moved"),
            b"movable"
        );
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
