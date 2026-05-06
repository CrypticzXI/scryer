use async_trait::async_trait;
use scryer_application::{AppError, AppResult, FileImporter};
use scryer_domain::{ImportFileResult, ImportStrategy};
use std::io::{self, Write};
use std::path::Path;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

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

fn fingerprint_regular_file(path: &Path) -> AppResult<FileFingerprint> {
    let link_meta = std::fs::symlink_metadata(path).map_err(|e| {
        AppError::Repository(format!(
            "import path not found or inaccessible: {}: {}",
            path.display(),
            e
        ))
    })?;
    let file_type = link_meta.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        return Err(AppError::Repository(format!(
            "import path is not a regular file: {}",
            path.display()
        )));
    }
    fingerprint_from_metadata(&link_meta)
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

fn ensure_same_file(path: &Path, expected: &FileFingerprint) -> AppResult<()> {
    let actual = fingerprint_regular_file(path)?;
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
    let actual = fingerprint_regular_file(path)?;
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
    ensure_same_file(path, expected)
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}

#[async_trait]
impl FileImporter for FsFileImporter {
    async fn import_file(&self, source: &Path, dest: &Path) -> AppResult<ImportFileResult> {
        let source = source.to_path_buf();
        let dest = dest.to_path_buf();

        tokio::task::spawn_blocking(move || {
            // Validate source exists and is a regular file
            let source_fingerprint = fingerprint_regular_file(&source)?;
            let size = source_fingerprint.len;
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

            // Attempt hard link first
            match std::fs::hard_link(&source, &dest) {
                Ok(()) => {
                    if let Err(error) = ensure_same_file(&source, &source_fingerprint)
                        .and_then(|_| ensure_same_file_identity(&dest, &source_fingerprint))
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
                ensure_same_file(&source, &source_fingerprint).map_err(io_other)?;
                let mut source_file = std::fs::File::open(&source)?;
                let source_open_fingerprint =
                    fingerprint_from_metadata(&source_file.metadata()?).map_err(io_other)?;
                if source_open_fingerprint != source_fingerprint {
                    return Err(io_other("import source changed before copy"));
                }

                let mut temp_file = std::fs::File::create(&temp_dest)?;
                std::io::copy(&mut source_file, &mut temp_file)?;
                temp_file.flush()?;
                temp_file.sync_all()?;
                drop(temp_file);

                ensure_same_file(&source, &source_fingerprint).map_err(io_other)?;

                // Atomic rename (same filesystem)
                std::fs::rename(&temp_dest, &dest)?;

                Ok(())
            })();

            match copy_result {
                Ok(()) => {
                    ensure_same_file(&source, &source_fingerprint)?;
                    // Verify destination size matches
                    let dest_fingerprint = fingerprint_regular_file(&dest)?;

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
