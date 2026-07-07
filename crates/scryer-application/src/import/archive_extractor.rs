//! Archive extraction for the import pipeline.
//!
//! Detects RAR, 7z, and zip archives in download directories and extracts
//! them before the video file scanner runs. Uses `weaver-unrar` for RAR
//! archives and `sevenz-rust2` for 7z/zip.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Component, Path, PathBuf};

use crate::{AppError, AppResult};
use tracing::info;

const EXTRACTED_DIR_NAME: &str = "_scryer_extracted";

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
/// extract them to a subdirectory and return the extraction path.
/// Returns `None` if no extraction was needed (video files exist directly).
pub async fn extract_archives_if_needed(
    dir: &Path,
    password: Option<&str>,
) -> AppResult<Option<PathBuf>> {
    let dir = dir.to_path_buf();
    let password = password.map(|s| s.to_string());
    tokio::task::spawn_blocking(move || extract_archives_sync(&dir, password.as_deref()))
        .await
        .map_err(|e| AppError::Repository(format!("archive extraction task failed: {e}")))?
}

/// Check if an extraction error indicates a password-protected archive.
pub fn is_password_required_error(error: &AppError) -> bool {
    let msg = error.to_string().to_ascii_lowercase();
    msg.contains("password") || msg.contains("encrypted") || msg.contains("wrong password")
}

fn extract_archives_sync(dir: &Path, password: Option<&str>) -> AppResult<Option<PathBuf>> {
    // If video files already exist, no extraction needed.
    if has_video_files(dir) {
        return Ok(None);
    }

    // Look for archives to extract.
    let archive = find_primary_archive(dir);
    let Some((archive_path, archive_type)) = archive else {
        return Ok(None);
    };

    let output_dir = dir.join(EXTRACTED_DIR_NAME);
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| AppError::Repository(format!("failed to create extraction directory: {e}")))?;

    info!(
        archive = %archive_path.display(),
        archive_type = archive_type.as_str(),
        output = %output_dir.display(),
        "extracting archive before import"
    );

    match archive_type {
        ArchiveType::Rar => extract_rar(&archive_path, dir, &output_dir, password)?,
        ArchiveType::SevenZip | ArchiveType::Zip => {
            extract_sevenz(&archive_path, &output_dir, password)?
        }
    }

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

fn extract_rar(
    rar_path: &Path,
    source_dir: &Path,
    output_dir: &Path,
    password: Option<&str>,
) -> AppResult<()> {
    let file = File::open(rar_path)
        .map_err(|e| AppError::Repository(format!("failed to open RAR archive: {e}")))?;

    let mut archive = weaver_unrar::RarArchive::open(file)
        .map_err(|e| AppError::Repository(format!("failed to parse RAR archive: {e}")))?;

    // Add continuation volumes (.r00, .r01, ... or .part2.rar, .part3.rar, etc.)
    // Collect all RAR volume files sorted by name
    let mut vol_files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(source_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && p != rar_path && scryer_domain::is_rar_volume(&p) {
                vol_files.push(p);
            }
        }
    }
    vol_files.sort();

    for (vol_idx, vol_path) in vol_files.iter().enumerate() {
        let vol_file = File::open(vol_path).map_err(|e| {
            AppError::Repository(format!(
                "failed to open RAR volume {}: {e}",
                vol_path.display()
            ))
        })?;
        archive
            .add_volume(vol_idx + 1, Box::new(vol_file))
            .map_err(|e| {
                AppError::Repository(format!(
                    "failed to add RAR volume {}: {e}",
                    vol_path.display()
                ))
            })?;
    }

    let metadata = archive.metadata();
    let options = weaver_unrar::ExtractOptions {
        password: password.map(|s| s.to_string()),
        ..Default::default()
    };

    for (idx, member) in metadata.members.iter().enumerate() {
        if member.is_directory {
            continue;
        }
        let dest = safe_archive_output_path(output_dir, &member.name)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::Repository(format!("failed to create directory for extraction: {e}"))
            })?;
            ensure_path_under_output(parent, output_dir)?;
        }

        info!(
            member = member.name.as_str(),
            size = member.unpacked_size,
            "extracting RAR member"
        );

        archive
            .extract_member_to_file(idx, &options, None, &dest)
            .map_err(|e| {
                AppError::Repository(format!(
                    "failed to extract RAR member '{}': {e}",
                    member.name
                ))
            })?;
    }

    Ok(())
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
    let canonical = path.canonicalize().map_err(|e| {
        AppError::Repository(format!(
            "failed to canonicalize extraction path {}: {e}",
            path.display()
        ))
    })?;
    if !canonical.starts_with(&output_root) {
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
    if dir.ends_with(EXTRACTED_DIR_NAME) {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
        let result = extract_archives_sync(dir.path(), None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn extract_rar4_store_archive() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archives/rar4_store.rar");
        if !fixture.exists() {
            return; // Skip if fixture not available
        }

        let dir = tempfile::tempdir().unwrap();
        fs::copy(&fixture, dir.path().join("archive.rar")).unwrap();

        let result = extract_archives_sync(dir.path(), None).unwrap();
        // The RAR4 store fixture contains a small text file, not a video.
        // Extraction should succeed but return None (no video files found).
        // The key test is that extraction doesn't panic or error.
        assert!(
            result.is_none(),
            "expected None since fixture contains text, not video files"
        );
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
    async fn cleanup_refuses_non_extracted_dir() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("important_data");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("file.txt"), b"data").unwrap();

        cleanup_extracted_dir(&other).await;
        // Should NOT be deleted — name doesn't match EXTRACTED_DIR_NAME
        assert!(other.exists());
    }
}
