use async_trait::async_trait;
use scryer_application::{AppError, AppResult, FileImporter};
use scryer_domain::{
    ImportContentProof, ImportFileIdentity, ImportFileResult, ImportMode, ImportSourceCleanupGuard,
    ImportSourceIdentity, ImportSourceIdentityKind, ImportStrategy,
};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, symlink};

const IMPORT_CONTENT_PROOF_SAMPLE_BYTES: usize = 1024 * 1024;

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

fn cleanup_guard(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
) -> AppResult<ImportSourceCleanupGuard> {
    let source_proof = import_content_proof(source).map_err(|error| {
        AppError::Repository(format!(
            "failed to build import source cleanup proof for source {}: {}",
            source.display(),
            error
        ))
    })?;
    let dest_proof = import_content_proof(dest).map_err(|error| {
        AppError::Repository(format!(
            "failed to build import source cleanup proof for destination {}: {}",
            dest.display(),
            error
        ))
    })?;
    if source_proof.size_bytes != size || dest_proof.size_bytes != size {
        return Err(AppError::Repository(format!(
            "failed to build import source cleanup proof because source/destination sizes changed: source={} dest={} expected={}",
            source_proof.size_bytes, dest_proof.size_bytes, size
        )));
    }
    if source_proof != dest_proof {
        return Err(AppError::Repository(format!(
            "failed to build import source cleanup proof because source and destination content differ: source={} dest={}",
            source.display(),
            dest.display()
        )));
    }

    Ok(ImportSourceCleanupGuard {
        source_path: source.to_path_buf(),
        dest_path: dest.to_path_buf(),
        size_bytes: size,
        source_identity: source_identity_from_fingerprint(source_fingerprint),
        source_proof,
        dest_proof,
    })
}

fn cleanup_guard_after_placement(
    source_cleanup_required: bool,
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
) -> AppResult<Option<ImportSourceCleanupGuard>> {
    if !source_cleanup_required {
        return Ok(None);
    }

    match cleanup_guard(source, dest, source_fingerprint, size) {
        Ok(guard) => Ok(Some(guard)),
        Err(error) => {
            let _ = std::fs::remove_file(dest);
            Err(error)
        }
    }
}

fn import_content_proof(path: &Path) -> AppResult<ImportContentProof> {
    let mut file = std::fs::File::open(path).map_err(|e| {
        AppError::Repository(format!(
            "failed to open import content proof path: {}: {}",
            path.display(),
            e
        ))
    })?;
    let size_bytes = file
        .metadata()
        .map_err(|e| {
            AppError::Repository(format!(
                "failed to stat import content proof path: {}: {}",
                path.display(),
                e
            ))
        })?
        .len();

    let first_len = size_bytes.min(IMPORT_CONTENT_PROOF_SAMPLE_BYTES as u64) as usize;
    let mut sample = Vec::with_capacity(first_len.saturating_mul(2));
    read_import_content_sample(&mut file, path, 0, first_len, &mut sample)?;

    let remaining_after_first = size_bytes.saturating_sub(first_len as u64);
    let last_len = remaining_after_first.min(IMPORT_CONTENT_PROOF_SAMPLE_BYTES as u64) as usize;
    if last_len > 0 {
        let last_offset = size_bytes - last_len as u64;
        read_import_content_sample(&mut file, path, last_offset, last_len, &mut sample)?;
    }

    Ok(ImportContentProof {
        size_bytes,
        sample_bytes: sample.len() as u64,
        sample_blake3: blake3::hash(&sample).to_hex().to_string(),
    })
}

fn read_import_content_sample(
    file: &mut std::fs::File,
    path: &Path,
    offset: u64,
    len: usize,
    sample: &mut Vec<u8>,
) -> AppResult<()> {
    file.seek(SeekFrom::Start(offset)).map_err(|e| {
        AppError::Repository(format!(
            "failed to seek import content proof path: {}: {}",
            path.display(),
            e
        ))
    })?;
    let start = sample.len();
    sample.resize(start + len, 0);
    file.read_exact(&mut sample[start..]).map_err(|e| {
        AppError::Repository(format!(
            "failed to read import content proof path: {}: {}",
            path.display(),
            e
        ))
    })
}

fn source_identity_from_fingerprint(
    source_fingerprint: &ImportSourceFingerprint,
) -> ImportSourceIdentity {
    ImportSourceIdentity {
        file: import_file_identity_from_fingerprint(&source_fingerprint.file),
        kind: match &source_fingerprint.kind {
            ImportSourceKind::Regular => ImportSourceIdentityKind::Regular,
            ImportSourceKind::Symlink {
                source_link_target,
                resolved_target,
            } => ImportSourceIdentityKind::Symlink {
                source_link_target: source_link_target.clone(),
                resolved_target: resolved_target.clone(),
            },
        },
    }
}

fn import_file_identity_from_fingerprint(file: &FileFingerprint) -> ImportFileIdentity {
    ImportFileIdentity {
        len: file.len,
        modified: file.modified,
        #[cfg(unix)]
        dev: file.dev,
        #[cfg(unix)]
        ino: file.ino,
    }
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

#[derive(Clone, Copy, Debug, Default)]
struct ImportFileOptions {
    #[cfg(test)]
    force_cross_device_move: bool,
    #[cfg(test)]
    force_copy_verification_failure: bool,
    #[cfg(test)]
    force_delete_failure: bool,
}

#[cfg(test)]
fn force_cross_device_move(options: &ImportFileOptions) -> bool {
    options.force_cross_device_move
}

#[cfg(not(test))]
fn force_cross_device_move(_: &ImportFileOptions) -> bool {
    false
}

#[cfg(test)]
fn force_copy_verification_failure(options: &ImportFileOptions) -> bool {
    options.force_copy_verification_failure
}

#[cfg(not(test))]
fn force_copy_verification_failure(_: &ImportFileOptions) -> bool {
    false
}

#[cfg(test)]
fn force_delete_failure(options: &ImportFileOptions) -> bool {
    options.force_delete_failure
}

#[cfg(not(test))]
fn force_delete_failure(_: &ImportFileOptions) -> bool {
    false
}

fn prepare_import_destination(
    source: &Path,
    dest: &Path,
) -> AppResult<(ImportSourceFingerprint, u64)> {
    let source_fingerprint = fingerprint_import_source(source)?;
    let size = source_fingerprint.file.len;
    if size == 0 {
        return Err(AppError::Repository(format!(
            "import source is zero bytes: {}",
            source.display()
        )));
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::Repository(format!(
                "failed to create destination directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    Ok((source_fingerprint, size))
}

fn import_symlink_source(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
) -> AppResult<()> {
    #[cfg(not(unix))]
    {
        let _ = dest;
        let _ = source_fingerprint;
        let _ = size;
        Err(AppError::Repository(format!(
            "import path is a symlink, but symlink imports are not supported on this platform: {}",
            source.display()
        )))
    }

    #[cfg(unix)]
    {
        let ImportSourceKind::Symlink {
            resolved_target, ..
        } = &source_fingerprint.kind
        else {
            unreachable!("import_symlink_source called for non-symlink source");
        };
        let symlink_target = build_destination_symlink_target(dest, resolved_target);
        symlink(&symlink_target, dest).map_err(|e| {
            AppError::Repository(format!(
                "failed to create symlink import {} -> {}: {}",
                dest.display(),
                symlink_target.display(),
                e
            ))
        })?;
        let dest_meta = std::fs::symlink_metadata(dest).map_err(|e| {
            AppError::Repository(format!(
                "failed to inspect imported symlink {}: {}",
                dest.display(),
                e
            ))
        })?;
        if !dest_meta.file_type().is_symlink() {
            let _ = std::fs::remove_file(dest);
            return Err(AppError::Repository(format!(
                "import destination is not a symlink: {}",
                dest.display()
            )));
        }
        ensure_same_source(source, source_fingerprint)?;
        let dest_target_meta = std::fs::metadata(dest).map_err(|e| {
            let _ = std::fs::remove_file(dest);
            AppError::Repository(format!(
                "imported symlink target is unavailable: {}: {}",
                dest.display(),
                e
            ))
        })?;
        if dest_target_meta.len() != size {
            let _ = std::fs::remove_file(dest);
            return Err(AppError::Repository(format!(
                "symlink import size mismatch: source={} dest={}",
                size,
                dest_target_meta.len()
            )));
        }

        Ok(())
    }
}

fn copy_regular_source_to_destination(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
    options: ImportFileOptions,
) -> AppResult<()> {
    let temp_dest = dest.with_extension("tmp_import");

    let copy_result = (|| -> Result<(), std::io::Error> {
        ensure_same_source(source, source_fingerprint).map_err(io_other)?;
        let mut source_file = std::fs::File::open(source)?;
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

        ensure_same_source(source, source_fingerprint).map_err(io_other)?;

        std::fs::rename(&temp_dest, dest)?;

        Ok(())
    })();

    if let Err(e) = copy_result {
        let _ = std::fs::remove_file(&temp_dest);
        return Err(AppError::Repository(format!(
            "import copy failed: {} -> {}: {}",
            source.display(),
            dest.display(),
            e
        )));
    }

    if force_copy_verification_failure(&options) {
        let _ = std::fs::remove_file(dest);
        return Err(AppError::Repository(format!(
            "copy verification failed for test: {}",
            dest.display()
        )));
    }

    ensure_same_source(source, source_fingerprint)?;
    let dest_fingerprint = fingerprint_import_source(dest)?.file;

    if dest_fingerprint.len != size {
        let _ = std::fs::remove_file(dest);
        return Err(AppError::Repository(format!(
            "copy size mismatch: source={} dest={}",
            size, dest_fingerprint.len
        )));
    }

    Ok(())
}

fn remove_import_source_after_verified_import_blocking(
    guard: ImportSourceCleanupGuard,
    final_dest_path: PathBuf,
    options: ImportFileOptions,
) -> AppResult<()> {
    if guard.source_path == final_dest_path {
        return Err(AppError::Repository(format!(
            "refusing to remove import source because it is the library file: {}",
            guard.source_path.display()
        )));
    }

    let dest_proof = import_content_proof(&final_dest_path).map_err(|_| {
        AppError::Repository(format!(
            "import source cleanup failed because destination is missing or inaccessible: {}",
            final_dest_path.display()
        ))
    })?;
    if dest_proof != guard.dest_proof {
        return Err(AppError::Repository(format!(
            "import source cleanup failed because destination proof changed: {} expected_size={} actual_size={} expected_blake3={} actual_blake3={}",
            final_dest_path.display(),
            guard.dest_proof.size_bytes,
            dest_proof.size_bytes,
            guard.dest_proof.sample_blake3,
            dest_proof.sample_blake3
        )));
    }

    let source_fingerprint = fingerprint_import_source(&guard.source_path).map_err(|_| {
        AppError::Repository(format!(
            "import source cleanup failed because source is missing or inaccessible: {}",
            guard.source_path.display()
        ))
    })?;
    if source_identity_from_fingerprint(&source_fingerprint) != guard.source_identity {
        return Err(AppError::Repository(format!(
            "import source cleanup failed because source changed: {}",
            guard.source_path.display()
        )));
    }
    let source_proof = import_content_proof(&guard.source_path).map_err(|_| {
        AppError::Repository(format!(
            "import source cleanup failed because source is missing or inaccessible: {}",
            guard.source_path.display()
        ))
    })?;
    if source_proof != guard.source_proof {
        return Err(AppError::Repository(format!(
            "import source cleanup failed because source proof changed: {} expected_size={} actual_size={} expected_blake3={} actual_blake3={}",
            guard.source_path.display(),
            guard.source_proof.size_bytes,
            source_proof.size_bytes,
            guard.source_proof.sample_blake3,
            source_proof.sample_blake3
        )));
    }

    let remove_result = if force_delete_failure(&options) {
        Err(io_other("forced source delete failure for test"))
    } else {
        std::fs::remove_file(&guard.source_path)
    };

    if let Err(error) = remove_result {
        return Err(AppError::Repository(format!(
            "import source cleanup failed after destination verification; failed to remove source {}: {}",
            guard.source_path.display(),
            error
        )));
    }

    Ok(())
}

fn import_hardlink_or_copy_blocking(
    source: PathBuf,
    dest: PathBuf,
    options: ImportFileOptions,
    source_cleanup_required: bool,
) -> AppResult<ImportFileResult> {
    let (source_fingerprint, size) = prepare_import_destination(&source, &dest)?;

    if let ImportSourceKind::Symlink { .. } = &source_fingerprint.kind {
        import_symlink_source(&source, &dest, &source_fingerprint, size)?;
        let source_cleanup = cleanup_guard_after_placement(
            source_cleanup_required,
            &source,
            &dest,
            &source_fingerprint,
            size,
        )?;
        return Ok(ImportFileResult {
            strategy: ImportStrategy::Symlink,
            source_path: source,
            dest_path: dest,
            size_bytes: size,
            source_cleanup,
        });
    }

    if !force_cross_device_move(&options) {
        match std::fs::hard_link(&source, &dest) {
            Ok(()) => {
                if let Err(error) = ensure_same_source(&source, &source_fingerprint)
                    .and_then(|_| ensure_same_file_identity(&dest, &source_fingerprint.file))
                {
                    let _ = std::fs::remove_file(&dest);
                    return Err(error);
                }
                match std::fs::metadata(&dest) {
                    Ok(dest_meta) if dest_meta.len() == size => {
                        let source_cleanup = cleanup_guard_after_placement(
                            source_cleanup_required,
                            &source,
                            &dest,
                            &source_fingerprint,
                            size,
                        )?;
                        return Ok(ImportFileResult {
                            strategy: ImportStrategy::HardLink,
                            source_path: source,
                            dest_path: dest,
                            size_bytes: size,
                            source_cleanup,
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
    }

    copy_regular_source_to_destination(&source, &dest, &source_fingerprint, size, options)?;

    let source_cleanup = cleanup_guard_after_placement(
        source_cleanup_required,
        &source,
        &dest,
        &source_fingerprint,
        size,
    )?;
    Ok(ImportFileResult {
        strategy: ImportStrategy::Copy,
        source_path: source,
        dest_path: dest,
        size_bytes: size,
        source_cleanup,
    })
}

fn import_file_blocking(
    source: PathBuf,
    dest: PathBuf,
    mode: ImportMode,
    options: ImportFileOptions,
) -> AppResult<ImportFileResult> {
    match mode {
        ImportMode::HardlinkOrCopy => {
            import_hardlink_or_copy_blocking(source, dest, ImportFileOptions::default(), false)
        }
        ImportMode::Move => import_hardlink_or_copy_blocking(source, dest, options, true),
    }
}

#[async_trait]
impl FileImporter for FsFileImporter {
    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
    ) -> AppResult<ImportFileResult> {
        let source = source.to_path_buf();
        let dest = dest.to_path_buf();

        tokio::task::spawn_blocking(move || {
            import_file_blocking(source, dest, mode, ImportFileOptions::default())
        })
        .await
        .map_err(|e| AppError::Repository(format!("import task panicked: {}", e)))?
    }

    async fn remove_import_source_after_verified_import(
        &self,
        guard: ImportSourceCleanupGuard,
        final_dest_path: &Path,
    ) -> AppResult<()> {
        let final_dest_path = final_dest_path.to_path_buf();

        tokio::task::spawn_blocking(move || {
            remove_import_source_after_verified_import_blocking(
                guard,
                final_dest_path,
                ImportFileOptions::default(),
            )
        })
        .await
        .map_err(|e| AppError::Repository(format!("import cleanup task panicked: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hardlink_or_copy_preserves_regular_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let result = FsFileImporter::new()
            .import_file(&source, &dest, ImportMode::HardlinkOrCopy)
            .await
            .expect("import file");

        assert_eq!(result.size_bytes, 16);
        assert!(matches!(
            result.strategy,
            ImportStrategy::HardLink | ImportStrategy::Copy
        ));
        assert!(result.source_cleanup.is_none());
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );
    }

    #[tokio::test]
    async fn move_mode_places_regular_source_and_returns_cleanup_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let result = FsFileImporter::new()
            .import_file(&source, &dest, ImportMode::Move)
            .await
            .expect("place file");

        assert!(matches!(
            result.strategy,
            ImportStrategy::HardLink | ImportStrategy::Copy
        ));
        assert_eq!(result.size_bytes, 16);
        assert!(result.source_cleanup.is_some());
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );
    }

    #[test]
    fn move_mode_cross_device_fallback_copies_then_cleanup_deletes_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect("move placement fallback");

        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );

        remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect("source cleanup");

        assert!(!source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest after cleanup"),
            b"fake video bytes"
        );
    }

    #[test]
    fn move_mode_copy_failure_leaves_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        std::fs::create_dir(dest.with_extension("tmp_import")).expect("create temp conflict");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect_err("copy should fail");

        assert!(error.to_string().contains("import copy failed"));
        assert!(source.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn move_mode_verification_failure_leaves_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_copy_verification_failure: true,
                ..Default::default()
            },
        )
        .expect_err("verification should fail");

        assert!(error.to_string().contains("copy verification failed"));
        assert!(source.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn move_mode_cleanup_delete_failure_reports_failure_without_removing_dest() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect("place file");

        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions {
                force_delete_failure: true,
                ..Default::default()
            },
        )
        .expect_err("delete should fail");

        assert!(error.to_string().contains("failed to remove source"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_changed_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect("place file");

        std::fs::write(&source, b"different video bytes").expect("change source");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("changed source should fail cleanup");

        assert!(error.to_string().contains("source changed"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_missing_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect("place file");

        std::fs::remove_file(&source).expect("remove source");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("missing source should fail cleanup");

        assert!(error.to_string().contains("source is missing"));
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_missing_destination() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect("place file");

        std::fs::remove_file(&dest).expect("remove dest");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("missing dest should fail cleanup");

        assert!(error.to_string().contains("destination is missing"));
        assert!(source.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_changed_destination_with_same_size() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect("place file");

        std::fs::write(&dest, b"same size change").expect("change dest");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("changed dest should fail cleanup");

        assert!(error.to_string().contains("destination proof changed"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_changed_destination_tail_sample() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        let mut bytes = vec![b'a'; IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1024];
        bytes[IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1023] = b'z';
        std::fs::write(&source, &bytes).expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect("place file");

        let mut changed_bytes = bytes;
        changed_bytes[IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1023] = b'y';
        std::fs::write(&dest, &changed_bytes).expect("change dest tail");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("changed dest tail should fail cleanup");

        assert!(error.to_string().contains("destination proof changed"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_source_as_final_destination() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
        )
        .expect("place file");

        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            source.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("same source and final dest should fail cleanup");

        assert!(error.to_string().contains("library file"));
        assert!(source.exists());
        assert!(dest.exists());
    }

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
            .import_file(&source_link, &dest_path, ImportMode::HardlinkOrCopy)
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
    async fn move_mode_cleanup_removes_source_symlink_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_target = dir.path().join("source-target.mkv");
        std::fs::write(&source_target, b"fake video bytes").expect("write target");
        let source_link = dir.path().join("source-link.mkv");
        symlink(PathBuf::from("source-target.mkv"), &source_link).expect("create source symlink");

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let dest_path = dest_dir.path().join("Imported.Movie.mkv");
        let result = FsFileImporter::new()
            .import_file(&source_link, &dest_path, ImportMode::Move)
            .await
            .expect("import symlink");

        assert_eq!(result.strategy, ImportStrategy::Symlink);
        assert!(source_link.exists());
        assert!(source_target.exists());
        assert!(dest_path.exists());

        FsFileImporter::new()
            .remove_import_source_after_verified_import(
                result.source_cleanup.expect("cleanup guard"),
                &dest_path,
            )
            .await
            .expect("cleanup symlink source");

        assert!(!source_link.exists());
        assert!(source_target.exists());
        assert!(dest_path.exists());
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
            .import_file(&source_link, &dest_path, ImportMode::HardlinkOrCopy)
            .await
            .expect_err("broken symlink should fail");

        assert!(
            error
                .to_string()
                .contains("import symlink target not found")
        );
    }
}
