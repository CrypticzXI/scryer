use async_trait::async_trait;
use scryer_application::{AppError, AppResult, FileImporter};
use scryer_domain::{ImportFileResult, ImportStrategy};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, symlink};

pub struct FsFileImporter;

impl Default for FsFileImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl FsFileImporter {
    pub fn new() -> Self {
        Self
    }
}

fn is_cross_device_error(err: &std::io::Error) -> bool {
    // EXDEV = errno 18 on both Linux and macOS
    // Windows: ERROR_NOT_SAME_DEVICE = 17
    matches!(err.raw_os_error(), Some(18) | Some(17))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImportSourceKind {
    Regular,
    Symlink {
        source_link_target: PathBuf,
        resolved_target: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportSourceFingerprint {
    file: FileFingerprint,
    kind: ImportSourceKind,
}

fn fingerprint_import_source(path: &Path) -> AppResult<ImportSourceFingerprint> {
    let link_meta = std::fs::symlink_metadata(path).map_err(|e| {
        AppError::Repository(format!(
            "import path not found or inaccessible: {}: {}",
            path.display(),
            e
        ))
    })?;
    let file_type = link_meta.file_type();
    if file_type.is_symlink() {
        let source_link_target = std::fs::read_link(path).map_err(|e| {
            AppError::Repository(format!(
                "failed to read import symlink target: {}: {}",
                path.display(),
                e
            ))
        })?;
        let resolved_target = resolve_symlink_target(path, &source_link_target);
        let target_meta = std::fs::metadata(&resolved_target).map_err(|e| {
            AppError::Repository(format!(
                "import symlink target not found or inaccessible: {} -> {}: {}",
                path.display(),
                resolved_target.display(),
                e
            ))
        })?;
        return Ok(ImportSourceFingerprint {
            file: fingerprint_from_metadata(&target_meta)?,
            kind: ImportSourceKind::Symlink {
                source_link_target,
                resolved_target,
            },
        });
    }
    if !file_type.is_file() {
        return Err(AppError::Repository(format!(
            "import path is not a regular file: {}",
            path.display()
        )));
    }
    Ok(ImportSourceFingerprint {
        file: fingerprint_from_metadata(&link_meta)?,
        kind: ImportSourceKind::Regular,
    })
}

fn fingerprint_from_metadata(metadata: &std::fs::Metadata) -> AppResult<FileFingerprint> {
    if !metadata.is_file() {
        return Err(AppError::Repository(
            "import path is not a regular file".into(),
        ));
    }
    Ok(FileFingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        dev: metadata.dev(),
        #[cfg(unix)]
        ino: metadata.ino(),
    })
}

fn ensure_same_source(path: &Path, expected: &ImportSourceFingerprint) -> AppResult<()> {
    let actual = fingerprint_import_source(path)?;
    if &actual != expected {
        return Err(AppError::Repository(format!(
            "import source changed during import: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_same_file_identity(path: &Path, expected: &FileFingerprint) -> AppResult<()> {
    let actual = fingerprint_import_source(path)?.file;
    if actual.dev != expected.dev || actual.ino != expected.ino {
        return Err(AppError::Repository(format!(
            "import destination is not linked to the expected source: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_same_file_identity(path: &Path, expected: &FileFingerprint) -> AppResult<()> {
    let actual = fingerprint_import_source(path)?.file;
    if actual != *expected {
        return Err(AppError::Repository(format!(
            "import destination does not match the expected source: {}",
            path.display()
        )));
    }
    Ok(())
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}

fn resolve_symlink_target(source: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    }
}

#[cfg(unix)]
fn build_destination_symlink_target(dest: &Path, resolved_target: &Path) -> PathBuf {
    let dest_parent = dest.parent().unwrap_or_else(|| Path::new("/"));
    relative_path_between(dest_parent, resolved_target)
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or_else(|| resolved_target.to_path_buf())
}

#[cfg(unix)]
fn relative_path_between(from_dir: &Path, to_path: &Path) -> Option<PathBuf> {
    if !from_dir.is_absolute() || !to_path.is_absolute() {
        return None;
    }

    let from_components = from_dir.components().collect::<Vec<_>>();
    let to_components = to_path.components().collect::<Vec<_>>();
    if !matches!(from_components.first(), Some(Component::RootDir))
        || !matches!(to_components.first(), Some(Component::RootDir))
    {
        return None;
    }

    let mut shared_prefix_len = 0;
    while shared_prefix_len < from_components.len()
        && shared_prefix_len < to_components.len()
        && from_components[shared_prefix_len] == to_components[shared_prefix_len]
    {
        shared_prefix_len += 1;
    }

    let mut relative = PathBuf::new();
    for component in &from_components[shared_prefix_len..] {
        if !matches!(component, Component::CurDir) {
            relative.push("..");
        }
    }
    for component in &to_components[shared_prefix_len..] {
        relative.push(component.as_os_str());
    }

    Some(relative)
}

#[async_trait]
impl FileImporter for FsFileImporter {
    async fn import_file(&self, source: &Path, dest: &Path) -> AppResult<ImportFileResult> {
        let source = source.to_path_buf();
        let dest = dest.to_path_buf();

        tokio::task::spawn_blocking(move || {
            let source_fingerprint = fingerprint_import_source(&source)?;
            let size = source_fingerprint.file.len;
            if size == 0 {
                return Err(AppError::Repository(format!(
                    "import source is zero bytes: {}",
                    source.display()
                )));
            }

            // Create destination parent directories
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    AppError::Repository(format!(
                        "failed to create destination directory {}: {}",
                        parent.display(),
                        e
                    ))
                })?;
            }

            if let ImportSourceKind::Symlink { resolved_target, .. } = &source_fingerprint.kind {
                #[cfg(not(unix))]
                {
                    return Err(AppError::Repository(format!(
                        "import path is a symlink, but symlink imports are not supported on this platform: {}",
                        source.display()
                    )));
                }

                #[cfg(unix)]
                {
                    let symlink_target = build_destination_symlink_target(&dest, resolved_target);
                    symlink(&symlink_target, &dest).map_err(|e| {
                        AppError::Repository(format!(
                            "failed to create symlink import {} -> {}: {}",
                            dest.display(),
                            symlink_target.display(),
                            e
                        ))
                    })?;
                    let dest_meta = std::fs::symlink_metadata(&dest).map_err(|e| {
                        AppError::Repository(format!(
                            "failed to inspect imported symlink {}: {}",
                            dest.display(),
                            e
                        ))
                    })?;
                    if !dest_meta.file_type().is_symlink() {
                        let _ = std::fs::remove_file(&dest);
                        return Err(AppError::Repository(format!(
                            "import destination is not a symlink: {}",
                            dest.display()
                        )));
                    }
                    ensure_same_source(&source, &source_fingerprint)?;
                    let dest_target_meta = std::fs::metadata(&dest).map_err(|e| {
                        let _ = std::fs::remove_file(&dest);
                        AppError::Repository(format!(
                            "imported symlink target is unavailable: {}: {}",
                            dest.display(),
                            e
                        ))
                    })?;
                    if dest_target_meta.len() != size {
                        let _ = std::fs::remove_file(&dest);
                        return Err(AppError::Repository(format!(
                            "symlink import size mismatch: source={} dest={}",
                            size,
                            dest_target_meta.len()
                        )));
                    }

                    return Ok(ImportFileResult {
                        strategy: ImportStrategy::Symlink,
                        source_path: source,
                        dest_path: dest,
                        size_bytes: size,
                    });
                }
            }

            // Attempt hard link first
            match std::fs::hard_link(&source, &dest) {
                Ok(()) => {
                    if let Err(error) = ensure_same_source(&source, &source_fingerprint)
                        .and_then(|_| ensure_same_file_identity(&dest, &source_fingerprint.file))
                    {
                        let _ = std::fs::remove_file(&dest);
                        return Err(error);
                    }
                    // Verify destination exists and size matches
                    match std::fs::metadata(&dest) {
                        Ok(dest_meta) if dest_meta.len() == size => {
                            return Ok(ImportFileResult {
                                strategy: ImportStrategy::HardLink,
                                source_path: source,
                                dest_path: dest,
                                size_bytes: size,
                            });
                        }
                        Ok(dest_meta) => {
                            let _ = std::fs::remove_file(&dest);
                            tracing::warn!(
                                "hard link size mismatch: source={} dest={}, falling back to copy",
                                size,
                                dest_meta.len()
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "hard link created but dest stat failed: {}, falling back to copy",
                                e
                            );
                        }
                    }
                }
                Err(e) if is_cross_device_error(&e) => {
                    tracing::info!(
                        "hard link failed (cross-device), falling back to copy: {} -> {}",
                        source.display(),
                        dest.display()
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "hard link failed: {}, falling back to copy: {} -> {}",
                        e,
                        source.display(),
                        dest.display()
                    );
                }
            }

            // Copy fallback: copy to temp file, fsync, rename atomically
            let temp_dest = dest.with_extension("tmp_import");

            let copy_result = (|| -> Result<(), std::io::Error> {
                ensure_same_source(&source, &source_fingerprint).map_err(io_other)?;
                let mut source_file = std::fs::File::open(&source)?;
                let source_open_fingerprint =
                    fingerprint_from_metadata(&source_file.metadata()?).map_err(io_other)?;
                if source_open_fingerprint != source_fingerprint.file {
                    return Err(io_other("import source changed before copy"));
                }

                let mut temp_file = std::fs::File::create(&temp_dest)?;
                std::io::copy(&mut source_file, &mut temp_file)?;
                temp_file.flush()?;
                temp_file.sync_all()?;
                drop(temp_file);

                ensure_same_source(&source, &source_fingerprint).map_err(io_other)?;

                // Atomic rename (same filesystem)
                std::fs::rename(&temp_dest, &dest)?;

                Ok(())
            })();

            match copy_result {
                Ok(()) => {
                    ensure_same_source(&source, &source_fingerprint)?;
                    // Verify destination size matches
                    let dest_fingerprint = fingerprint_import_source(&dest)?.file;

                    if dest_fingerprint.len != size {
                        let _ = std::fs::remove_file(&dest);
                        return Err(AppError::Repository(format!(
                            "copy size mismatch: source={} dest={}",
                            size, dest_fingerprint.len
                        )));
                    }

                    Ok(ImportFileResult {
                        strategy: ImportStrategy::Copy,
                        source_path: source,
                        dest_path: dest,
                        size_bytes: size,
                    })
                }
                Err(e) => {
                    // Clean up partial temp file
                    let _ = std::fs::remove_file(&temp_dest);
                    Err(AppError::Repository(format!(
                        "import copy failed: {} -> {}: {}",
                        source.display(),
                        dest.display(),
                        e
                    )))
                }
            }
        })
        .await
        .map_err(|e| AppError::Repository(format!("import task panicked: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn import_file_preserves_symlink_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_target = dir.path().join("source-target.mkv");
        std::fs::write(&source_target, b"fake video bytes").expect("write target");
        let source_link = dir.path().join("source-link.mkv");
        let relative_target = PathBuf::from("source-target.mkv");
        symlink(&relative_target, &source_link).expect("create source symlink");

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let dest_path = dest_dir.path().join("Imported.Movie.mkv");
        let result = FsFileImporter::new()
            .import_file(&source_link, &dest_path)
            .await
            .expect("import symlink");

        assert_eq!(result.strategy, ImportStrategy::Symlink);
        assert_eq!(result.size_bytes, 16);
        assert!(
            std::fs::symlink_metadata(&dest_path)
                .expect("dest metadata")
                .file_type()
                .is_symlink()
        );
        assert!(
            !std::fs::read_link(&dest_path)
                .expect("read dest symlink")
                .is_absolute()
        );
        assert_eq!(
            std::fs::canonicalize(&dest_path).expect("canonicalize dest symlink"),
            std::fs::canonicalize(&source_target).expect("canonicalize source target")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn import_file_rejects_broken_symlink_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_link = dir.path().join("broken-link.mkv");
        symlink(PathBuf::from("missing-target.mkv"), &source_link).expect("create broken symlink");

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let dest_path = dest_dir.path().join("Imported.Movie.mkv");
        let error = FsFileImporter::new()
            .import_file(&source_link, &dest_path)
            .await
            .expect_err("broken symlink should fail");

        assert!(
            error
                .to_string()
                .contains("import symlink target not found")
        );
    }
}
