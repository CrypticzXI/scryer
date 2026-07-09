//! Archive extraction for the import pipeline.
//!
//! Detects RAR, 7z, and zip archives in download directories. Native 7z
//! extraction remains in core; RAR and zip are delegated to the optional
//! archive extraction plugin.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::{AppError, AppResult, ArchiveExtractorPluginProvider};
use scryer_plugin_sdk::{
    ArchivePluginFormat, ArchivePluginOperation, ArchivePluginProcessRequest,
    ArchivePluginProcessResponse, ArchivePluginRepairFormat, ArchivePluginStatus,
};
use tracing::info;

const EXTRACTED_DIR_NAME: &str = "_scryer_extracted";
const ARCHIVE_STAGING_PREFIX: &str = ".scryer-archive-extract-";
const MAX_PLUGIN_OUTPUT_FILES: usize = 20_000;
const MAX_PLUGIN_OUTPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ArchiveExtractionDestination {
    staging_parent: PathBuf,
    import_id: String,
}

impl ArchiveExtractionDestination {
    pub fn new(staging_parent: impl Into<PathBuf>, import_id: impl Into<String>) -> Self {
        Self {
            staging_parent: staging_parent.into(),
            import_id: import_id.into(),
        }
    }
}

/// Archive type detected in a download directory.
#[derive(Debug, Clone, Copy)]
pub enum ArchiveType {
    Rar,
    SevenZip,
    Zip,
}

impl ArchiveType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rar => "RAR",
            Self::SevenZip => "7z",
            Self::Zip => "zip",
        }
    }
}

/// If the download directory contains no video files but has archive files,
/// extract them to a hidden destination-side staging directory and return the path.
/// Returns `None` if no extraction was needed (video files exist directly).
pub async fn extract_archives_if_needed(
    dir: &Path,
    destination: Option<ArchiveExtractionDestination>,
    password: Option<&str>,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<Option<PathBuf>> {
    let dir = dir.to_path_buf();
    let password = password.map(|s| s.to_string());
    let archive = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || plan_archive_extraction(&dir))
            .await
            .map_err(|e| AppError::Repository(format!("archive detection task failed: {e}")))??
    };

    let Some((archive_path, archive_type)) = archive else {
        return Ok(None);
    };
    let Some(destination) = destination else {
        return Err(AppError::Validation(format!(
            "archive extraction requires a resolved import destination before staging output for {}",
            dir.display()
        )));
    };
    let output_dir = create_archive_staging_dir(&destination)?;

    info!(
        archive = %archive_path.display(),
        archive_type = archive_type.as_str(),
        output = %output_dir.display(),
        "extracting archive before import"
    );

    match archive_type {
        ArchiveType::SevenZip => tokio::task::spawn_blocking(move || {
            extract_native_archive(&archive_path, archive_type, output_dir, password.as_deref())
        })
        .await
        .map_err(|e| AppError::Repository(format!("archive extraction task failed: {e}")))?,
        ArchiveType::Rar | ArchiveType::Zip => {
            let format = archive_plugin_format(archive_type)?;
            let Some(provider) = archive_provider else {
                return Err(AppError::archive_extraction_plugin_required(Some(
                    dir.to_string_lossy().into_owned(),
                )));
            };
            extract_with_archive_plugin(
                &dir,
                archive_path,
                archive_type,
                format,
                password,
                provider,
                output_dir,
            )
            .await
        }
    }
}

pub fn archive_extraction_would_be_needed(dir: &Path) -> AppResult<bool> {
    Ok(plan_archive_extraction(dir)?.is_some())
}

/// Check if an extraction error indicates a password-protected archive.
pub fn is_password_required_error(error: &AppError) -> bool {
    let msg = error.to_string().to_ascii_lowercase();
    msg.contains("password") || msg.contains("encrypted") || msg.contains("wrong password")
}

#[cfg(test)]
fn extract_archives_sync(
    dir: &Path,
    destination: Option<ArchiveExtractionDestination>,
    password: Option<&str>,
) -> AppResult<Option<PathBuf>> {
    let Some((archive_path, archive_type)) = plan_archive_extraction(dir)? else {
        return Ok(None);
    };
    let Some(destination) = destination else {
        return Err(AppError::Validation(format!(
            "archive extraction requires a resolved import destination before staging output for {}",
            dir.display()
        )));
    };
    let output_dir = create_archive_staging_dir(&destination)?;

    info!(
        archive = %archive_path.display(),
        archive_type = archive_type.as_str(),
        output = %output_dir.display(),
        "extracting archive before import"
    );

    match archive_type {
        ArchiveType::SevenZip => {
            extract_native_archive(&archive_path, archive_type, output_dir, password)
        }
        ArchiveType::Rar | ArchiveType::Zip => Err(AppError::archive_extraction_plugin_required(
            Some(archive_path.to_string_lossy().into_owned()),
        )),
    }
}

fn plan_archive_extraction(dir: &Path) -> AppResult<Option<(PathBuf, ArchiveType)>> {
    // If video files already exist, no extraction needed.
    if has_video_files(dir) {
        return Ok(None);
    }

    // Look for archives to extract.
    let archive = find_primary_archive(dir);
    let Some((archive_path, archive_type)) = archive else {
        return Ok(None);
    };

    Ok(Some((archive_path, archive_type)))
}

fn extract_native_archive(
    archive_path: &Path,
    archive_type: ArchiveType,
    output_dir: PathBuf,
    password: Option<&str>,
) -> AppResult<Option<PathBuf>> {
    match archive_type {
        ArchiveType::SevenZip => {
            extract_sevenz(archive_path, &output_dir, password)?;
            verify_extracted_video(archive_type, output_dir)
        }
        ArchiveType::Rar | ArchiveType::Zip => Err(AppError::archive_extraction_plugin_required(
            Some(archive_path.to_string_lossy().into_owned()),
        )),
    }
}

async fn extract_with_archive_plugin(
    dir: &Path,
    archive_path: PathBuf,
    archive_type: ArchiveType,
    format: ArchivePluginFormat,
    password: Option<String>,
    provider: Arc<dyn ArchiveExtractorPluginProvider>,
    output_dir: PathBuf,
) -> AppResult<Option<PathBuf>> {
    let (client, operation) = if let Some(par2_path) = find_primary_par2(dir)
        && let Some(client) =
            provider.client_for_repair_then_extract(format, ArchivePluginRepairFormat::Par2)
    {
        let operation = ArchivePluginOperation::RepairThenExtract {
            source_dir: dir.to_string_lossy().into_owned(),
            output_dir: output_dir.to_string_lossy().into_owned(),
            format,
            par2_path: Some(par2_path.to_string_lossy().into_owned()),
            archive_path: Some(archive_path.to_string_lossy().into_owned()),
            password,
        };
        (client, operation)
    } else {
        let Some(client) = provider.client_for_format(format) else {
            return Err(AppError::archive_extraction_plugin_required(Some(
                dir.to_string_lossy().into_owned(),
            )));
        };
        let operation = ArchivePluginOperation::ExtractArchive {
            archive_path: archive_path.to_string_lossy().into_owned(),
            output_dir: output_dir.to_string_lossy().into_owned(),
            format,
            password,
        };
        (client, operation)
    };
    let request = ArchivePluginProcessRequest { operation };
    let response = client.process(request).await?;
    handle_archive_plugin_response(archive_type, output_dir, response)
}

fn create_archive_staging_dir(destination: &ArchiveExtractionDestination) -> AppResult<PathBuf> {
    std::fs::create_dir_all(&destination.staging_parent).map_err(|error| {
        AppError::Repository(format!(
            "failed to create archive staging parent {}: {error}",
            destination.staging_parent.display()
        ))
    })?;
    let staging_name = format!(
        "{ARCHIVE_STAGING_PREFIX}{}",
        sanitize_archive_staging_id(&destination.import_id)
    );
    let output_dir = destination.staging_parent.join(staging_name);
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir).map_err(|error| {
            AppError::Repository(format!(
                "failed to clear stale archive staging directory {}: {error}",
                output_dir.display()
            ))
        })?;
    }
    std::fs::create_dir(&output_dir).map_err(|error| {
        AppError::Repository(format!(
            "failed to create archive staging directory {}: {error}",
            output_dir.display()
        ))
    })?;
    Ok(output_dir)
}

fn sanitize_archive_staging_id(import_id: &str) -> String {
    let sanitized = import_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.trim_matches('_').is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        sanitized
    }
}

fn archive_plugin_format(archive_type: ArchiveType) -> AppResult<ArchivePluginFormat> {
    match archive_type {
        ArchiveType::Rar => Ok(ArchivePluginFormat::Rar),
        ArchiveType::Zip => Ok(ArchivePluginFormat::Zip),
        ArchiveType::SevenZip => Err(AppError::Validation(
            "7z archives are native and must not be routed to archive plugins".to_string(),
        )),
    }
}

fn handle_archive_plugin_response(
    archive_type: ArchiveType,
    output_dir: PathBuf,
    response: ArchivePluginProcessResponse,
) -> AppResult<Option<PathBuf>> {
    match response.status {
        ArchivePluginStatus::Ok => {
            if let Err(error) = validate_archive_plugin_output(&output_dir, &response) {
                let _ = std::fs::remove_dir_all(&output_dir);
                return Err(error);
            }
            verify_extracted_video(archive_type, output_dir)
        }
        ArchivePluginStatus::UnsupportedFormat => Err(AppError::Validation(format!(
            "archive plugin does not support {} extraction",
            archive_type.as_str()
        ))),
        ArchivePluginStatus::PasswordRequired => Err(AppError::Validation(format!(
            "{} archive requires a password",
            archive_type.as_str()
        ))),
        ArchivePluginStatus::PasswordInvalid => Err(AppError::Validation(format!(
            "{} archive password is invalid",
            archive_type.as_str()
        ))),
        ArchivePluginStatus::RepairRequired => Err(AppError::Validation(format!(
            "{} archive requires PAR2 repair before extraction",
            archive_type.as_str()
        ))),
        ArchivePluginStatus::RepairFailed | ArchivePluginStatus::Failed => {
            let _ = std::fs::remove_dir_all(&output_dir);
            let message = response
                .message
                .or(response.error_code)
                .unwrap_or_else(|| "archive plugin extraction failed".to_string());
            Err(AppError::Repository(message))
        }
    }
}

fn validate_archive_plugin_output(
    output_dir: &Path,
    response: &ArchivePluginProcessResponse,
) -> AppResult<()> {
    let output_root = output_dir.canonicalize().map_err(|error| {
        AppError::Repository(format!(
            "failed to canonicalize archive plugin output directory {}: {error}",
            output_dir.display()
        ))
    })?;

    for file in &response.files {
        let path = safe_archive_output_path(output_dir, &file.relative_path)?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            AppError::Repository(format!(
                "archive plugin manifest output '{}' is missing or unreadable: {error}",
                path.display()
            ))
        })?;
        if !metadata.file_type().is_file() {
            return Err(AppError::Validation(format!(
                "archive plugin manifest output is not a regular file: {}",
                path.display()
            )));
        }
        ensure_path_under_output_with_root(&path, &output_root)?;
        if let Some(expected_size) = file.size
            && expected_size != metadata.len()
        {
            return Err(AppError::Validation(format!(
                "archive plugin manifest size mismatch for {}",
                path.display()
            )));
        }
    }

    let mut file_count = 0usize;
    let mut expanded_bytes = 0u64;
    let mut stack = vec![output_dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect archive plugin output {}: {error}",
                path.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Validation(format!(
                "archive plugin output contains a symlink: {}",
                path.display()
            )));
        }
        ensure_path_under_output_with_root(&path, &output_root)?;
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).map_err(|error| {
                AppError::Repository(format!(
                    "failed to read archive plugin output directory {}: {error}",
                    path.display()
                ))
            })? {
                let entry = entry.map_err(|error| {
                    AppError::Repository(format!(
                        "failed to read archive plugin output entry: {error}"
                    ))
                })?;
                stack.push(entry.path());
            }
            continue;
        }
        if !metadata.is_file() {
            return Err(AppError::Validation(format!(
                "archive plugin output is not a regular file: {}",
                path.display()
            )));
        }

        file_count += 1;
        if file_count > MAX_PLUGIN_OUTPUT_FILES {
            return Err(AppError::Validation(
                "archive plugin output contains too many files".to_string(),
            ));
        }
        expanded_bytes = expanded_bytes.checked_add(metadata.len()).ok_or_else(|| {
            AppError::Validation("archive plugin output is too large".to_string())
        })?;
        if expanded_bytes > MAX_PLUGIN_OUTPUT_BYTES {
            return Err(AppError::Validation(format!(
                "archive plugin output exceeds {} bytes",
                MAX_PLUGIN_OUTPUT_BYTES
            )));
        }
    }

    Ok(())
}

fn verify_extracted_video(
    archive_type: ArchiveType,
    output_dir: PathBuf,
) -> AppResult<Option<PathBuf>> {
    // Verify we got something useful out.
    if has_video_files(&output_dir) {
        info!(
            archive_type = archive_type.as_str(),
            output = %output_dir.display(),
            "archive extraction complete, video files found"
        );
        Ok(Some(output_dir))
    } else {
        info!(
            archive_type = archive_type.as_str(),
            "archive extracted but no video files found in output"
        );
        // Clean up the empty extraction.
        let _ = std::fs::remove_dir_all(&output_dir);
        Ok(None)
    }
}

fn has_video_files(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && scryer_domain::is_video_file(&path) {
            return true;
        }
        if path.is_dir() && has_video_files(&path) {
            return true;
        }
    }
    false
}

/// Find the primary archive file in a directory. Prefers RAR, then 7z, then zip.
fn find_primary_archive(dir: &Path) -> Option<(PathBuf, ArchiveType)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };

    let mut rar: Option<PathBuf> = None;
    let mut sevenz: Option<PathBuf> = None;
    let mut zip: Option<PathBuf> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ext.as_str() {
            "rar" if rar.is_none() => rar = Some(path),
            "7z" if sevenz.is_none() => sevenz = Some(path),
            "zip" if zip.is_none() => zip = Some(path),
            _ => {}
        }
    }

    if let Some(p) = rar {
        Some((p, ArchiveType::Rar))
    } else if let Some(p) = sevenz {
        Some((p, ArchiveType::SevenZip))
    } else {
        zip.map(|p| (p, ArchiveType::Zip))
    }
}

fn find_primary_par2(dir: &Path) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };

    let mut par2: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "par2" {
            par2 = Some(path);
            break;
        }
    }
    par2
}

fn extract_sevenz(archive_path: &Path, output_dir: &Path, password: Option<&str>) -> AppResult<()> {
    let file = File::open(archive_path)
        .map_err(|e| AppError::Repository(format!("failed to open archive: {e}")))?;

    let pw = match password {
        Some(s) => sevenz_rust2::Password::from(s),
        None => sevenz_rust2::Password::empty(),
    };
    sevenz_rust2::decompress_with_extract_fn_and_password(
        file,
        output_dir,
        pw,
        |entry, reader, _dest| {
            let dest = safe_archive_output_path(output_dir, entry.name())
                .map_err(sevenz_extraction_error)?;
            if entry.is_directory() {
                std::fs::create_dir_all(&dest)?;
                ensure_path_under_output(&dest, output_dir).map_err(sevenz_extraction_error)?;
                return Ok(true);
            }

            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
                ensure_path_under_output(parent, output_dir).map_err(sevenz_extraction_error)?;
            }
            let file = File::create(&dest)?;
            if entry.size() > 0 {
                let mut writer = BufWriter::new(file);
                std::io::copy(reader, &mut writer)?;
            }
            Ok(true)
        },
    )
    .map_err(|e| AppError::Repository(format!("archive extraction failed: {e}")))?;

    Ok(())
}

fn safe_archive_output_path(output_dir: &Path, entry_name: &str) -> AppResult<PathBuf> {
    if entry_name.trim().is_empty() || entry_name.contains('\\') {
        return Err(AppError::Validation(format!(
            "unsafe archive entry path: {entry_name}"
        )));
    }

    let mut relative = PathBuf::new();
    for component in Path::new(entry_name).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::Validation(format!(
                    "unsafe archive entry path: {entry_name}"
                )));
            }
        }
    }

    if relative.as_os_str().is_empty() {
        return Err(AppError::Validation(format!(
            "unsafe archive entry path: {entry_name}"
        )));
    }

    Ok(output_dir.join(relative))
}

fn ensure_path_under_output(path: &Path, output_dir: &Path) -> AppResult<()> {
    let output_root = output_dir.canonicalize().map_err(|e| {
        AppError::Repository(format!(
            "failed to canonicalize extraction directory {}: {e}",
            output_dir.display()
        ))
    })?;
    ensure_path_under_output_with_root(path, &output_root)
}

fn ensure_path_under_output_with_root(path: &Path, output_root: &Path) -> AppResult<()> {
    let canonical = path.canonicalize().map_err(|e| {
        AppError::Repository(format!(
            "failed to canonicalize extraction path {}: {e}",
            path.display()
        ))
    })?;
    if !canonical.starts_with(output_root) {
        return Err(AppError::Validation(format!(
            "archive entry escapes extraction directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn sevenz_extraction_error(error: AppError) -> sevenz_rust2::Error {
    sevenz_rust2::Error::Other(std::borrow::Cow::Owned(error.to_string()))
}

/// Clean up the extraction directory after import completes.
pub async fn cleanup_extracted_dir(dir: &Path) {
    if is_archive_staging_dir(dir) {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}

fn is_archive_staging_dir(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == EXTRACTED_DIR_NAME || name.starts_with(ARCHIVE_STAGING_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArchiveExtractorClient, ArchiveExtractorPluginProvider};
    use scryer_plugin_sdk::ArchivePluginExtractedFile;
    use std::fs;
    use std::sync::{Arc, Mutex};

    struct RecordingArchiveClient {
        operation: Arc<Mutex<Option<ArchivePluginOperation>>>,
        write_output_file: bool,
    }

    #[async_trait::async_trait]
    impl ArchiveExtractorClient for RecordingArchiveClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> AppResult<ArchivePluginProcessResponse> {
            let output_dir = match &request.operation {
                ArchivePluginOperation::ExtractArchive { output_dir, .. }
                | ArchivePluginOperation::RepairThenExtract { output_dir, .. } => {
                    PathBuf::from(output_dir)
                }
                ArchivePluginOperation::Inspect { .. }
                | ArchivePluginOperation::VerifyRepairSet { .. } => {
                    return Ok(ArchivePluginProcessResponse {
                        status: ArchivePluginStatus::Failed,
                        files: Vec::new(),
                        repair: None,
                        expanded_bytes: None,
                        copied_bytes: None,
                        staged_bytes: None,
                        error_code: Some("unsupported_operation".to_string()),
                        message: Some("operation does not extract archive files".to_string()),
                    });
                }
            };
            *self.operation.lock().unwrap() = Some(request.operation);
            let files = if self.write_output_file {
                fs::create_dir_all(&output_dir).unwrap();
                let output_file = output_dir.join("movie.mkv");
                fs::write(&output_file, b"fake video").unwrap();
                vec![ArchivePluginExtractedFile {
                    relative_path: "movie.mkv".to_string(),
                    size: Some(10),
                    checksum: None,
                }]
            } else {
                Vec::new()
            };
            Ok(ArchivePluginProcessResponse {
                status: ArchivePluginStatus::Ok,
                files,
                repair: None,
                expanded_bytes: Some(if self.write_output_file { 10 } else { 0 }),
                copied_bytes: None,
                staged_bytes: None,
                error_code: None,
                message: None,
            })
        }
    }

    struct RecordingArchiveProvider {
        client: Arc<dyn ArchiveExtractorClient>,
        repair_client: Option<Arc<dyn ArchiveExtractorClient>>,
    }

    impl ArchiveExtractorPluginProvider for RecordingArchiveProvider {
        fn client_for_format(
            &self,
            format: ArchivePluginFormat,
        ) -> Option<Arc<dyn ArchiveExtractorClient>> {
            matches!(format, ArchivePluginFormat::Zip).then(|| Arc::clone(&self.client))
        }

        fn client_for_repair_then_extract(
            &self,
            format: ArchivePluginFormat,
            repair_format: ArchivePluginRepairFormat,
        ) -> Option<Arc<dyn ArchiveExtractorClient>> {
            (matches!(format, ArchivePluginFormat::Rar | ArchivePluginFormat::Zip)
                && matches!(repair_format, ArchivePluginRepairFormat::Par2))
            .then(|| self.repair_client.as_ref().map(Arc::clone))
            .flatten()
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["recording".to_string()]
        }
    }

    #[test]
    fn has_video_files_detects_mkv() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie.mkv"), b"fake video").unwrap();
        assert!(has_video_files(dir.path()));
    }

    #[test]
    fn has_video_files_ignores_non_video() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), b"text").unwrap();
        fs::write(dir.path().join("archive.rar"), b"rar").unwrap();
        assert!(!has_video_files(dir.path()));
    }

    #[test]
    fn has_video_files_recursive() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("episode.mp4"), b"video").unwrap();
        assert!(has_video_files(dir.path()));
    }

    #[test]
    fn find_primary_archive_prefers_rar() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.rar"), b"rar").unwrap();
        fs::write(dir.path().join("release.7z"), b"7z").unwrap();
        let (path, kind) = find_primary_archive(dir.path()).unwrap();
        assert!(path.extension().unwrap() == "rar");
        assert!(matches!(kind, ArchiveType::Rar));
    }

    #[test]
    fn find_primary_archive_finds_7z() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.7z"), b"7z").unwrap();
        let (_, kind) = find_primary_archive(dir.path()).unwrap();
        assert!(matches!(kind, ArchiveType::SevenZip));
    }

    #[test]
    fn find_primary_archive_finds_zip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.zip"), b"zip").unwrap();
        let (_, kind) = find_primary_archive(dir.path()).unwrap();
        assert!(matches!(kind, ArchiveType::Zip));
    }

    #[test]
    fn find_primary_archive_none_for_video_only() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie.mkv"), b"video").unwrap();
        assert!(find_primary_archive(dir.path()).is_none());
    }

    #[test]
    fn archive_output_path_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(safe_archive_output_path(dir.path(), "../movie.mkv").is_err());
        assert!(safe_archive_output_path(dir.path(), "/tmp/movie.mkv").is_err());
        assert!(safe_archive_output_path(dir.path(), r"nested\movie.mkv").is_err());
    }

    #[test]
    fn archive_output_path_allows_nested_relative_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = safe_archive_output_path(dir.path(), "Season 1/movie.mkv").unwrap();
        assert_eq!(path, dir.path().join("Season 1").join("movie.mkv"));
    }

    #[test]
    fn extract_no_op_when_video_exists() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie.mkv"), b"video").unwrap();
        fs::write(dir.path().join("archive.rar"), b"rar").unwrap();
        let result = extract_archives_sync(dir.path(), None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn rar_archive_requires_archive_plugin() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archives/rar4_store.rar");
        if !fixture.exists() {
            return; // Skip if fixture not available
        }

        let dir = tempfile::tempdir().unwrap();
        fs::copy(&fixture, dir.path().join("archive.rar")).unwrap();
        let destination = tempfile::tempdir().unwrap();

        let err = extract_archives_sync(
            dir.path(),
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "rar-plugin-required",
            )),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            AppError::ArchiveExtractionPluginRequired { .. }
        ));
    }

    #[tokio::test]
    async fn zip_with_par2_sidecar_uses_plain_extract_without_repair_capability() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.zip");
        fs::write(&archive_path, b"zip").unwrap();
        fs::write(dir.path().join("release.par2"), b"par2").unwrap();

        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
            write_output_file: false,
        });
        let destination = tempfile::tempdir().unwrap();
        let output_dir = create_archive_staging_dir(&ArchiveExtractionDestination::new(
            destination.path(),
            "zip-plain-extract",
        ))
        .unwrap();
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                repair_client: None,
            });

        let result = extract_with_archive_plugin(
            dir.path(),
            archive_path,
            ArchiveType::Zip,
            ArchivePluginFormat::Zip,
            None,
            provider,
            output_dir,
        )
        .await
        .unwrap();

        assert!(result.is_none());
        let recorded = operation.lock().unwrap().clone().unwrap();
        assert!(matches!(
            recorded,
            ArchivePluginOperation::ExtractArchive {
                format: ArchivePluginFormat::Zip,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn archive_extraction_requires_destination_for_archived_download() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.rar"), b"rar").unwrap();

        let error = extract_archives_if_needed(dir.path(), None, None, None)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires a resolved import destination")
        );
    }

    #[tokio::test]
    async fn rar_with_par2_stages_under_destination_when_source_is_not_writable() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let archive_path = source.path().join("archive.rar");
        let par2_path = source.path().join("archive.par2");
        fs::write(&archive_path, b"rar").unwrap();
        fs::write(&par2_path, b"par2").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(source.path(), fs::Permissions::from_mode(0o555)).unwrap();
        }

        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
            write_output_file: true,
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client: Arc::clone(&client),
                repair_client: Some(client),
            });

        let extracted = extract_archives_if_needed(
            source.path(),
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "import/with spaces",
            )),
            Some("secret"),
            Some(provider),
        )
        .await
        .unwrap()
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(source.path(), fs::Permissions::from_mode(0o755)).unwrap();
        }

        assert!(extracted.starts_with(destination.path()));
        assert!(!extracted.starts_with(source.path()));
        let staging_name = extracted
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap();
        assert!(staging_name.starts_with(ARCHIVE_STAGING_PREFIX));
        assert!(staging_name.starts_with('.'));
        assert!(extracted.join("movie.mkv").exists());

        let recorded = operation.lock().unwrap().clone().unwrap();
        match recorded {
            ArchivePluginOperation::RepairThenExtract {
                source_dir,
                output_dir,
                format,
                par2_path: recorded_par2,
                archive_path: recorded_archive,
                password,
            } => {
                assert_eq!(source_dir, source.path().to_string_lossy());
                assert_eq!(output_dir, extracted.to_string_lossy());
                assert_eq!(format, ArchivePluginFormat::Rar);
                assert_eq!(recorded_par2.unwrap(), par2_path.to_string_lossy());
                assert_eq!(recorded_archive.unwrap(), archive_path.to_string_lossy());
                assert_eq!(password.as_deref(), Some("secret"));
            }
            other => panic!("expected repair-then-extract operation, got {other:?}"),
        }

        cleanup_extracted_dir(&extracted).await;
        assert!(!extracted.exists());
    }

    #[tokio::test]
    async fn cleanup_only_removes_extracted_dir() {
        let dir = tempfile::tempdir().unwrap();
        let extracted = dir.path().join(EXTRACTED_DIR_NAME);
        fs::create_dir(&extracted).unwrap();
        fs::write(extracted.join("file.txt"), b"data").unwrap();

        cleanup_extracted_dir(&extracted).await;
        assert!(!extracted.exists());
        // Parent still exists
        assert!(dir.path().exists());
    }

    #[tokio::test]
    async fn cleanup_removes_archive_staging_dir() {
        let dir = tempfile::tempdir().unwrap();
        let extracted = dir
            .path()
            .join(format!("{ARCHIVE_STAGING_PREFIX}import-123"));
        fs::create_dir(&extracted).unwrap();
        fs::write(extracted.join("file.txt"), b"data").unwrap();

        cleanup_extracted_dir(&extracted).await;
        assert!(!extracted.exists());
        assert!(dir.path().exists());
    }

    #[tokio::test]
    async fn cleanup_refuses_non_extracted_dir() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("important_data");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("file.txt"), b"data").unwrap();

        cleanup_extracted_dir(&other).await;
        // Should NOT be deleted: name matches neither legacy nor staging dirs.
        assert!(other.exists());
    }
}
