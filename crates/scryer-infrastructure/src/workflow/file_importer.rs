use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, FileImporter, ImportFileTransferProgress,
    ImportFileTransferProgressSender, fs_integrity::import_content_proof,
};
use scryer_domain::{
    ImportContentProof, ImportFileIdentity, ImportFileResult, ImportMode, ImportSourceCleanupGuard,
    ImportSourceIdentity, ImportSourceIdentityKind, ImportSourceSnapshot, ImportStrategy,
    ImportTransferPhase,
};
use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, symlink};
#[cfg(windows)]
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
};

const TRANSIENT_BAD_FILE_DESCRIPTOR_ERRNO: i32 = 9;
const IMPORT_COPY_MAX_ATTEMPTS: usize = 3;
const IMPORT_COPY_BUFFER_BYTES: usize = 8 * 1024 * 1024;

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
struct DirectoryFingerprint {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
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

fn stable_import_source_snapshot(
    path: &Path,
    initial_fingerprint: Option<&ImportSourceFingerprint>,
) -> AppResult<ImportSourceSnapshot> {
    let initial = match initial_fingerprint {
        Some(fingerprint) => fingerprint.clone(),
        None => fingerprint_import_source(path)?,
    };
    let proof = import_content_proof(path)?;
    ensure_same_source(path, &initial)?;
    Ok(ImportSourceSnapshot {
        identity: source_identity_from_fingerprint(&initial),
        proof,
    })
}

fn snapshot_import_source_blocking(path: PathBuf) -> AppResult<ImportSourceSnapshot> {
    stable_import_source_snapshot(&path, None)
}

fn ensure_expected_source_snapshot(
    path: &Path,
    current_fingerprint: &ImportSourceFingerprint,
    expected: Option<&ImportSourceSnapshot>,
) -> AppResult<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = stable_import_source_snapshot(path, Some(current_fingerprint))?;
    if &actual != expected {
        return Err(AppError::Repository(format!(
            "import source changed after validation: {}",
            path.display()
        )));
    }
    Ok(())
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

fn directory_fingerprint_from_path(path: &Path) -> AppResult<DirectoryFingerprint> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        AppError::Repository(format!(
            "failed to stat destination directory {}: {}",
            path.display(),
            e
        ))
    })?;
    if !metadata.is_dir() {
        return Err(AppError::Repository(
            "import destination parent is not a directory".into(),
        ));
    }
    #[cfg(unix)]
    {
        Ok(DirectoryFingerprint {
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map_err(|e| {
                AppError::Repository(format!(
                    "failed to open destination directory {} for identity check: {}",
                    path.display(),
                    e
                ))
            })?;
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        let result =
            unsafe { GetFileInformationByHandle(directory.as_raw_handle(), info.as_mut_ptr()) };
        if result == 0 {
            return Err(AppError::Repository(format!(
                "failed to read destination directory identity {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            )));
        }
        let info = unsafe { info.assume_init() };
        let file_index = ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64;
        Ok(DirectoryFingerprint {
            volume_serial_number: info.dwVolumeSerialNumber,
            file_index,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(DirectoryFingerprint {})
    }
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
    #[cfg(test)]
    force_transient_copy_failures: u8,
    #[cfg(test)]
    force_non_transient_copy_failure: bool,
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

#[cfg(test)]
fn force_copy_attempt_error(
    temp_file: &mut std::fs::File,
    options: &ImportFileOptions,
    attempt: usize,
) -> std::io::Result<()> {
    if options.force_non_transient_copy_failure {
        temp_file.write_all(b"partial non-transient copy failure")?;
        temp_file.flush()?;
        return Err(io_other("forced non-transient copy failure"));
    }
    if attempt <= usize::from(options.force_transient_copy_failures) {
        temp_file.write_all(format!("partial transient copy failure {attempt}").as_bytes())?;
        temp_file.flush()?;
        return Err(std::io::Error::from_raw_os_error(
            TRANSIENT_BAD_FILE_DESCRIPTOR_ERRNO,
        ));
    }
    Ok(())
}

#[cfg(not(test))]
fn force_copy_attempt_error(
    _temp_file: &mut std::fs::File,
    _options: &ImportFileOptions,
    _attempt: usize,
) -> std::io::Result<()> {
    Ok(())
}

struct ImportDestinationGuard {
    requested_path: PathBuf,
    parent_path: PathBuf,
    approved_parent_canonical: PathBuf,
    approved_parent_fingerprint: DirectoryFingerprint,
}

fn destination_parent_for_guard(dest: &Path) -> &Path {
    dest.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn prepare_import_destination(
    source: &Path,
    dest: &Path,
) -> AppResult<(ImportSourceFingerprint, u64, ImportDestinationGuard)> {
    let source_fingerprint = fingerprint_import_source(source)?;
    let size = source_fingerprint.file.len;
    if size == 0 {
        return Err(AppError::Repository(format!(
            "import source is zero bytes: {}",
            source.display()
        )));
    }

    let parent = destination_parent_for_guard(dest);
    std::fs::create_dir_all(parent).map_err(|e| {
        AppError::Repository(format!(
            "failed to create destination directory {}: {}",
            parent.display(),
            e
        ))
    })?;
    let approved_parent_canonical = std::fs::canonicalize(parent).map_err(|e| {
        AppError::Repository(format!(
            "failed to inspect destination directory {}: {}",
            parent.display(),
            e
        ))
    })?;
    let approved_parent_fingerprint = directory_fingerprint_from_path(parent)?;

    Ok((
        source_fingerprint,
        size,
        ImportDestinationGuard {
            requested_path: dest.to_path_buf(),
            parent_path: parent.to_path_buf(),
            approved_parent_canonical,
            approved_parent_fingerprint,
        },
    ))
}

fn validate_import_destination_parent(
    guard: &ImportDestinationGuard,
    returned_dest: &Path,
) -> AppResult<()> {
    if returned_dest != guard.requested_path {
        return Err(AppError::Repository(format!(
            "import destination changed during placement: expected {} got {}",
            guard.requested_path.display(),
            returned_dest.display()
        )));
    }

    let current_parent_canonical = std::fs::canonicalize(&guard.parent_path).map_err(|e| {
        AppError::Repository(format!(
            "failed to re-check destination directory {}: {}",
            guard.parent_path.display(),
            e
        ))
    })?;
    if current_parent_canonical != guard.approved_parent_canonical {
        return Err(AppError::Repository(format!(
            "import destination parent changed during placement: {} resolved to {} before import and {} after import",
            guard.parent_path.display(),
            guard.approved_parent_canonical.display(),
            current_parent_canonical.display()
        )));
    }
    let current_parent_fingerprint = directory_fingerprint_from_path(&guard.parent_path)?;
    if current_parent_fingerprint != guard.approved_parent_fingerprint {
        return Err(AppError::Repository(format!(
            "import destination parent changed during placement: {}",
            guard.parent_path.display()
        )));
    }

    Ok(())
}

fn validate_import_destination_file(guard: &ImportDestinationGuard) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(&guard.requested_path).map_err(|e| {
        AppError::Repository(format!(
            "failed to inspect imported destination {}: {}",
            guard.requested_path.display(),
            e
        ))
    })?;
    let file_type = metadata.file_type();
    if !file_type.is_file() && !file_type.is_symlink() {
        return Err(AppError::Repository(format!(
            "import destination is not a file or symlink: {}",
            guard.requested_path.display()
        )));
    }

    Ok(())
}

#[cfg(test)]
fn validate_import_destination_guard(
    guard: &ImportDestinationGuard,
    returned_dest: &Path,
) -> AppResult<()> {
    validate_import_destination_parent(guard, returned_dest)?;
    validate_import_destination_file(guard)
}

fn validate_import_destination_guard_after_placement(
    guard: &ImportDestinationGuard,
    returned_dest: &Path,
) -> AppResult<()> {
    validate_import_destination_parent(guard, returned_dest)?;
    match validate_import_destination_file(guard) {
        Ok(()) => Ok(()),
        Err(validation_error) => {
            let validation_message = validation_error.to_string();
            match std::fs::remove_file(returned_dest) {
                Ok(()) => Err(validation_error),
                Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => {
                    Err(validation_error)
                }
                Err(cleanup_error) => Err(AppError::Repository(format!(
                    "{validation_message}; additionally failed to remove placed import destination {} after validation failure: {cleanup_error}",
                    returned_dest.display()
                ))),
            }
        }
    }
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

#[derive(Debug)]
struct ImportCopyAttemptError {
    stage: &'static str,
    error: std::io::Error,
}

impl ImportCopyAttemptError {
    fn new(stage: &'static str, error: std::io::Error) -> Self {
        Self { stage, error }
    }

    fn is_transient_file_handle_error(&self) -> bool {
        self.error.raw_os_error() == Some(TRANSIENT_BAD_FILE_DESCRIPTOR_ERRNO)
    }
}

impl fmt::Display for ImportCopyAttemptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.stage, self.error)
    }
}

fn remove_import_temp_file(temp_dest: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(temp_dest) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn sleep_before_import_copy_retry(_delay: Duration) {}

#[cfg(not(test))]
fn sleep_before_import_copy_retry(delay: Duration) {
    std::thread::sleep(delay);
}

fn import_copy_retry_delay(attempt: usize) -> Duration {
    match attempt {
        1 => Duration::from_secs(1),
        _ => Duration::from_secs(3),
    }
}

fn report_import_transfer_progress(
    progress: Option<&ImportFileTransferProgressSender>,
    phase: ImportTransferPhase,
    bytes: u64,
    total_bytes: u64,
) {
    if let Some(progress) = progress {
        let _ = progress.send(ImportFileTransferProgress {
            phase,
            bytes,
            total_bytes,
        });
    }
}

struct ImportCopyAttempt<'a> {
    source: &'a Path,
    dest: &'a Path,
    temp_dest: &'a Path,
    source_fingerprint: &'a ImportSourceFingerprint,
    size: u64,
    progress: Option<&'a ImportFileTransferProgressSender>,
}

fn copy_regular_source_to_destination_once(
    attempt_context: ImportCopyAttempt<'_>,
    options: ImportFileOptions,
    attempt: usize,
) -> Result<(), ImportCopyAttemptError> {
    let ImportCopyAttempt {
        source,
        dest,
        temp_dest,
        source_fingerprint,
        size,
        progress,
    } = attempt_context;

    ensure_same_source(source, source_fingerprint)
        .map_err(io_other)
        .map_err(|error| ImportCopyAttemptError::new("source preflight", error))?;
    let mut source_file = std::fs::File::open(source)
        .map_err(|error| ImportCopyAttemptError::new("source open", error))?;
    let source_open_fingerprint = fingerprint_from_metadata(
        &source_file
            .metadata()
            .map_err(|error| ImportCopyAttemptError::new("source metadata", error))?,
    )
    .map_err(io_other)
    .map_err(|error| ImportCopyAttemptError::new("source validation", error))?;
    if source_open_fingerprint != source_fingerprint.file {
        return Err(ImportCopyAttemptError::new(
            "source validation",
            io_other("import source changed before copy"),
        ));
    }

    let mut temp_file = std::fs::File::create(temp_dest)
        .map_err(|error| ImportCopyAttemptError::new("temp create", error))?;
    force_copy_attempt_error(&mut temp_file, &options, attempt)
        .map_err(|error| ImportCopyAttemptError::new("copy", error))?;
    report_import_transfer_progress(progress, ImportTransferPhase::Copying, 0, size);
    let mut copied = 0u64;
    let mut buffer = vec![0u8; IMPORT_COPY_BUFFER_BYTES];
    loop {
        let read = source_file
            .read(&mut buffer)
            .map_err(|error| ImportCopyAttemptError::new("copy", error))?;
        if read == 0 {
            break;
        }
        temp_file
            .write_all(&buffer[..read])
            .map_err(|error| ImportCopyAttemptError::new("copy", error))?;
        copied = copied.saturating_add(read as u64);
        report_import_transfer_progress(progress, ImportTransferPhase::Copying, copied, size);
    }
    report_import_transfer_progress(progress, ImportTransferPhase::Finalizing, copied, size);
    temp_file
        .flush()
        .map_err(|error| ImportCopyAttemptError::new("flush", error))?;
    temp_file
        .sync_all()
        .map_err(|error| ImportCopyAttemptError::new("sync", error))?;
    drop(temp_file);

    ensure_same_source(source, source_fingerprint)
        .map_err(io_other)
        .map_err(|error| ImportCopyAttemptError::new("source verification", error))?;

    std::fs::rename(temp_dest, dest)
        .map_err(|error| ImportCopyAttemptError::new("final rename", error))?;

    Ok(())
}

fn copy_regular_source_to_destination(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
    options: ImportFileOptions,
    progress: Option<&ImportFileTransferProgressSender>,
) -> AppResult<()> {
    let temp_dest = dest.with_extension("tmp_import");
    let mut attempt = 1usize;

    loop {
        if let Err(error) = remove_import_temp_file(&temp_dest) {
            return Err(AppError::Repository(format!(
                "import copy failed before attempt {}: {} -> {}: cleanup of temporary destination {} failed: {}",
                attempt,
                source.display(),
                dest.display(),
                temp_dest.display(),
                error
            )));
        }

        match copy_regular_source_to_destination_once(
            ImportCopyAttempt {
                source,
                dest,
                temp_dest: &temp_dest,
                source_fingerprint,
                size,
                progress,
            },
            options,
            attempt,
        ) {
            Ok(()) => break,
            Err(error) => {
                let should_retry =
                    error.is_transient_file_handle_error() && attempt < IMPORT_COPY_MAX_ATTEMPTS;
                let _ = remove_import_temp_file(&temp_dest);
                if should_retry {
                    sleep_before_import_copy_retry(import_copy_retry_delay(attempt));
                    attempt += 1;
                    continue;
                }

                return Err(AppError::Repository(format!(
                    "import copy failed after {} attempt(s): {} -> {}: {}",
                    attempt,
                    source.display(),
                    dest.display(),
                    error
                )));
            }
        }
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
    expected_source: Option<ImportSourceSnapshot>,
    progress: Option<ImportFileTransferProgressSender>,
) -> AppResult<ImportFileResult> {
    let (source_fingerprint, size, destination_guard) = prepare_import_destination(&source, &dest)?;
    ensure_expected_source_snapshot(&source, &source_fingerprint, expected_source.as_ref())?;

    if let ImportSourceKind::Symlink { .. } = &source_fingerprint.kind {
        import_symlink_source(&source, &dest, &source_fingerprint, size)?;
        validate_import_destination_guard_after_placement(&destination_guard, &dest)?;
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
                        validate_import_destination_guard_after_placement(
                            &destination_guard,
                            &dest,
                        )?;
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

    copy_regular_source_to_destination(
        &source,
        &dest,
        &source_fingerprint,
        size,
        options,
        progress.as_ref(),
    )?;
    validate_import_destination_guard_after_placement(&destination_guard, &dest)?;

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
    expected_source: Option<ImportSourceSnapshot>,
    progress: Option<ImportFileTransferProgressSender>,
) -> AppResult<ImportFileResult> {
    match mode {
        ImportMode::HardlinkOrCopy => import_hardlink_or_copy_blocking(
            source,
            dest,
            ImportFileOptions::default(),
            false,
            expected_source,
            progress,
        ),
        ImportMode::Move => {
            import_hardlink_or_copy_blocking(source, dest, options, true, expected_source, progress)
        }
    }
}

#[async_trait]
impl FileImporter for FsFileImporter {
    async fn snapshot_import_source(&self, source: &Path) -> AppResult<ImportSourceSnapshot> {
        let source = source.to_path_buf();

        tokio::task::spawn_blocking(move || snapshot_import_source_blocking(source))
            .await
            .map_err(|e| AppError::Repository(format!("import snapshot task panicked: {}", e)))?
    }

    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
        expected_source: Option<&ImportSourceSnapshot>,
    ) -> AppResult<ImportFileResult> {
        self.import_file_with_progress(source, dest, mode, expected_source, None)
            .await
    }

    async fn import_file_with_progress(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
        expected_source: Option<&ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
    ) -> AppResult<ImportFileResult> {
        let source = source.to_path_buf();
        let dest = dest.to_path_buf();
        let expected_source = expected_source.cloned();

        tokio::task::spawn_blocking(move || {
            import_file_blocking(
                source,
                dest,
                mode,
                ImportFileOptions::default(),
                expected_source,
                progress,
            )
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
            .import_file(&source, &dest, ImportMode::HardlinkOrCopy, None)
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
    async fn snapshot_import_source_is_stable_for_regular_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");

        let importer = FsFileImporter::new();
        let snapshot = importer
            .snapshot_import_source(&source)
            .await
            .expect("snapshot source");
        let second_snapshot = importer
            .snapshot_import_source(&source)
            .await
            .expect("snapshot source again");

        assert_eq!(snapshot, second_snapshot);
        assert!(matches!(
            snapshot.identity.kind,
            ImportSourceIdentityKind::Regular
        ));
        assert_eq!(snapshot.proof.size_bytes, 16);
    }

    #[tokio::test]
    async fn import_file_rejects_replaced_regular_source_after_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let importer = FsFileImporter::new();
        let snapshot = importer
            .snapshot_import_source(&source)
            .await
            .expect("snapshot source");
        std::fs::write(&source, b"changed video bytes").expect("replace source");

        let error = importer
            .import_file(&source, &dest, ImportMode::HardlinkOrCopy, Some(&snapshot))
            .await
            .expect_err("changed source should fail import");

        assert!(
            error
                .to_string()
                .contains("import source changed after validation")
        );
        assert!(!dest.exists());
    }

    #[test]
    fn destination_guard_rejects_parent_replacement_after_approval() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let parent = dir.path().join("library");
        let dest = parent.join("Imported.Movie.mkv");
        let (_source_fingerprint, _size, guard) =
            prepare_import_destination(&source, &dest).expect("prepare destination");

        let old_parent = dir.path().join("library-old");
        std::fs::rename(&parent, &old_parent).expect("replace approved parent");
        std::fs::create_dir_all(&parent).expect("create replacement parent");
        std::fs::write(&dest, b"fake video bytes").expect("write destination");

        let error = validate_import_destination_guard(&guard, &dest)
            .expect_err("changed parent should be rejected");
        assert!(
            error
                .to_string()
                .contains("import destination parent changed during placement")
        );
    }

    #[test]
    fn destination_guard_allows_child_creation_in_approved_parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let parent = dir.path().join("library");
        let dest = parent.join("Imported.Movie.mkv");
        let (_source_fingerprint, _size, guard) =
            prepare_import_destination(&source, &dest).expect("prepare destination");

        std::fs::write(&dest, b"fake video bytes").expect("write destination");

        validate_import_destination_guard(&guard, &dest)
            .expect("normal child creation should not change parent identity");
    }

    #[test]
    fn destination_guard_after_placement_preserves_files_when_parent_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let parent = dir.path().join("library");
        let dest = parent.join("Imported.Movie.mkv");
        let (_source_fingerprint, _size, guard) =
            prepare_import_destination(&source, &dest).expect("prepare destination");

        std::fs::write(&dest, b"placed video bytes").expect("write placed destination");
        let old_parent = dir.path().join("library-old");
        std::fs::rename(&parent, &old_parent).expect("replace approved parent");
        std::fs::create_dir_all(&parent).expect("create replacement parent");
        std::fs::write(&dest, b"replacement parent bytes").expect("write replacement occupant");

        let error = validate_import_destination_guard_after_placement(&guard, &dest)
            .expect_err("changed parent should be rejected");
        assert!(
            error
                .to_string()
                .contains("import destination parent changed during placement")
        );
        assert_eq!(
            std::fs::read(old_parent.join("Imported.Movie.mkv")).expect("read placed destination"),
            b"placed video bytes"
        );
        assert_eq!(
            std::fs::read(&dest).expect("read replacement occupant"),
            b"replacement parent bytes"
        );
    }

    #[test]
    fn destination_guard_after_placement_reports_cleanup_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let parent = dir.path().join("library");
        let dest = parent.join("Imported.Movie.mkv");
        let (_source_fingerprint, _size, guard) =
            prepare_import_destination(&source, &dest).expect("prepare destination");

        std::fs::create_dir(&dest).expect("create non-file destination");

        let error = validate_import_destination_guard_after_placement(&guard, &dest)
            .expect_err("directory destination should be rejected");
        let message = error.to_string();
        assert!(message.contains("import destination is not a file or symlink"));
        assert!(message.contains("additionally failed to remove placed import destination"));
        assert!(dest.exists());
    }

    #[tokio::test]
    async fn move_mode_places_regular_source_and_returns_cleanup_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let result = FsFileImporter::new()
            .import_file(&source, &dest, ImportMode::Move, None)
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
            None,
            None,
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
    fn cross_device_copy_reports_transfer_progress() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        let total_bytes = std::fs::metadata(&source).expect("source metadata").len();
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            Some(progress_tx),
        )
        .expect("move placement fallback");

        assert_eq!(result.strategy, ImportStrategy::Copy);

        let mut updates = Vec::new();
        while let Ok(update) = progress_rx.try_recv() {
            updates.push(update);
        }

        assert!(updates.iter().any(|update| {
            update.phase == ImportTransferPhase::Copying
                && update.bytes == 0
                && update.total_bytes == total_bytes
        }));
        assert!(updates.iter().any(|update| {
            update.phase == ImportTransferPhase::Copying
                && update.bytes == total_bytes
                && update.total_bytes == total_bytes
        }));
        assert!(updates.iter().any(|update| {
            update.phase == ImportTransferPhase::Finalizing
                && update.bytes == total_bytes
                && update.total_bytes == total_bytes
        }));
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
            None,
            None,
        )
        .expect_err("copy should fail");

        assert!(error.to_string().contains("import copy failed"));
        assert!(source.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn move_mode_copy_retries_transient_bad_file_descriptor() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        let temp_dest = dest.with_extension("tmp_import");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_transient_copy_failures: 1,
                ..Default::default()
            },
            None,
            None,
        )
        .expect("copy should retry and succeed");

        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );
        assert!(!temp_dest.exists());
    }

    #[test]
    fn move_mode_copy_exhausts_transient_bad_file_descriptor_retries() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        let temp_dest = dest.with_extension("tmp_import");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_transient_copy_failures: 3,
                ..Default::default()
            },
            None,
            None,
        )
        .expect_err("copy should fail after retry budget");

        let message = error.to_string();
        assert!(message.contains("import copy failed after 3 attempt(s)"));
        assert!(message.contains("copy: Bad file descriptor"));
        assert!(source.exists());
        assert!(!dest.exists());
        assert!(!temp_dest.exists());
    }

    #[test]
    fn move_mode_copy_does_not_retry_non_transient_copy_error() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        let temp_dest = dest.with_extension("tmp_import");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_non_transient_copy_failure: true,
                ..Default::default()
            },
            None,
            None,
        )
        .expect_err("copy should fail without retry");

        let message = error.to_string();
        assert!(message.contains("import copy failed after 1 attempt(s)"));
        assert!(message.contains("copy: forced non-transient copy failure"));
        assert!(source.exists());
        assert!(!dest.exists());
        assert!(!temp_dest.exists());
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
            None,
            None,
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
            None,
            None,
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
            None,
            None,
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
            None,
            None,
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
            None,
            None,
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
            None,
            None,
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
        let mut bytes =
            vec![b'a'; scryer_application::fs_integrity::IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1024];
        bytes[scryer_application::fs_integrity::IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1023] = b'z';
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
            None,
            None,
        )
        .expect("place file");

        let mut changed_bytes = bytes;
        changed_bytes[scryer_application::fs_integrity::IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1023] =
            b'y';
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
            None,
            None,
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
            .import_file(&source_link, &dest_path, ImportMode::HardlinkOrCopy, None)
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
    async fn import_file_rejects_retargeted_symlink_source_after_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_target = dir.path().join("source-target.mkv");
        let replacement_target = dir.path().join("replacement-target.mkv");
        std::fs::write(&source_target, b"fake video bytes").expect("write target");
        std::fs::write(&replacement_target, b"other video bytes").expect("write replacement");
        let source_link = dir.path().join("source-link.mkv");
        symlink(PathBuf::from("source-target.mkv"), &source_link).expect("create source symlink");

        let importer = FsFileImporter::new();
        let snapshot = importer
            .snapshot_import_source(&source_link)
            .await
            .expect("snapshot symlink source");
        std::fs::remove_file(&source_link).expect("remove old source symlink");
        symlink(PathBuf::from("replacement-target.mkv"), &source_link)
            .expect("retarget source symlink");

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let dest_path = dest_dir.path().join("Imported.Movie.mkv");
        let error = importer
            .import_file(
                &source_link,
                &dest_path,
                ImportMode::HardlinkOrCopy,
                Some(&snapshot),
            )
            .await
            .expect_err("changed symlink source should fail import");

        assert!(
            error
                .to_string()
                .contains("import source changed after validation")
        );
        assert!(!dest_path.exists());
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
            .import_file(&source_link, &dest_path, ImportMode::Move, None)
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
            .import_file(&source_link, &dest_path, ImportMode::HardlinkOrCopy, None)
            .await
            .expect_err("broken symlink should fail");

        assert!(
            error
                .to_string()
                .contains("import symlink target not found")
        );
    }
}
