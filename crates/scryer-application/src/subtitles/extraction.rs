use std::borrow::Cow;
use std::cmp::Reverse;
use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::path::{Component, Path};

use flate2::read::GzDecoder;

use super::provider::SubtitleFile;
use crate::{AppError, AppResult};

const MAX_RECURSION_DEPTH: usize = 3;
const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 128 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 512;

const SUPPORTED_SUBTITLE_FORMATS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx"];

#[derive(Debug, Clone, Default)]
pub struct SubtitleExtractionContext {
    pub language: Option<String>,
    pub episode: Option<i32>,
    pub absolute_episode: Option<i32>,
}

struct ArchiveCandidate {
    filename: String,
    file: SubtitleFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    Zip,
    Tar,
    SevenZip,
    Rar,
    Gzip,
    Zstd,
    Xz,
}

pub fn is_supported_subtitle_format(format: &str) -> bool {
    SUPPORTED_SUBTITLE_FORMATS.contains(&normalize_extension(format).as_str())
}

pub async fn normalize_downloaded_subtitle(
    file: SubtitleFile,
    context: SubtitleExtractionContext,
) -> AppResult<SubtitleFile> {
    tokio::task::spawn_blocking(move || normalize_sync(file, &context, 0))
        .await
        .map_err(|error| {
            AppError::Repository(format!("subtitle extraction task failed: {error}"))
        })?
}

fn normalize_sync(
    file: SubtitleFile,
    context: &SubtitleExtractionContext,
    depth: usize,
) -> AppResult<SubtitleFile> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(AppError::Validation(
            "subtitle artifact nesting is too deep".to_string(),
        ));
    }
    if file.content.len() > MAX_ARTIFACT_BYTES {
        return Err(AppError::Validation(format!(
            "subtitle artifact is too large: {} bytes",
            file.content.len()
        )));
    }

    match detect_artifact_kind(&file) {
        Some(ArtifactKind::Gzip) => {
            let filename = file.filename;
            let format = file.format;
            let content = read_limited(GzDecoder::new(Cursor::new(file.content)))?;
            normalize_sync(
                inner_file_from_parts(filename, format, content, ".gz"),
                context,
                depth + 1,
            )
        }
        Some(ArtifactKind::Zstd) => {
            let filename = file.filename;
            let format = file.format;
            let decoder =
                zstd::stream::read::Decoder::new(Cursor::new(file.content)).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to initialize Zstandard subtitle decoder: {error}"
                    ))
                })?;
            let content = read_limited(decoder)?;
            normalize_sync(
                inner_file_from_parts(filename, format, content, ".zst"),
                context,
                depth + 1,
            )
        }
        Some(ArtifactKind::Xz) => {
            let filename = file.filename;
            let format = file.format;
            let mut writer = LimitedWriter::new(MAX_EXPANDED_BYTES);
            lzma_rs::xz_decompress(&mut Cursor::new(file.content), &mut writer).map_err(
                |error| AppError::Repository(format!("failed to decompress XZ subtitle: {error}")),
            )?;
            let content = writer.into_inner();
            normalize_sync(
                inner_file_from_parts(filename, format, content, ".xz"),
                context,
                depth + 1,
            )
        }
        Some(ArtifactKind::Zip) => {
            select_archive_candidate(extract_zip_candidates(file)?, context, depth)
        }
        Some(ArtifactKind::Tar) => {
            select_archive_candidate(extract_tar_candidates(file.content)?, context, depth)
        }
        Some(ArtifactKind::SevenZip) => {
            select_archive_candidate(extract_sevenz_candidates(file.content)?, context, depth)
        }
        Some(ArtifactKind::Rar) => {
            select_archive_candidate(extract_rar_candidates(file.content)?, context, depth)
        }
        None => finalize_subtitle(file),
    }
}

fn finalize_subtitle(mut file: SubtitleFile) -> AppResult<SubtitleFile> {
    let Some(format) = final_subtitle_format(&file) else {
        return Err(AppError::Validation(format!(
            "unsupported subtitle artifact format: {}",
            file.filename
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or(file.format.as_str())
        )));
    };

    if !file.content.iter().any(|byte| !byte.is_ascii_whitespace()) {
        return Err(AppError::Repository(
            "subtitle download returned empty content".to_string(),
        ));
    }

    file.format = format;
    file.content_type = subtitle_content_type(file.format.as_str()).map(str::to_string);
    Ok(file)
}

fn select_archive_candidate(
    candidates: Vec<ArchiveCandidate>,
    context: &SubtitleExtractionContext,
    depth: usize,
) -> AppResult<SubtitleFile> {
    let mut normalized = Vec::new();
    for candidate in candidates {
        if let Ok(file) = normalize_sync(candidate.file, context, depth + 1) {
            normalized.push((candidate.filename, file));
        }
    }

    if normalized.is_empty() {
        return Err(AppError::Validation(
            "subtitle archive did not contain a supported subtitle file".to_string(),
        ));
    }

    normalized.sort_by_key(|(filename, file)| Reverse(candidate_rank(filename, file, context)));

    if normalized.len() > 1 {
        let first = candidate_rank(&normalized[0].0, &normalized[0].1, context);
        let second = candidate_rank(&normalized[1].0, &normalized[1].1, context);
        if first == second {
            return Err(AppError::Validation(format!(
                "subtitle archive has multiple equally ranked subtitle files: {}, {}",
                normalized[0].0, normalized[1].0
            )));
        }
    }

    Ok(normalized.remove(0).1)
}

fn candidate_rank(
    filename: &str,
    file: &SubtitleFile,
    context: &SubtitleExtractionContext,
) -> (i32, i32, i32, usize) {
    let mut episode_score = 0;
    if let Some(episode) = context.episode
        && filename_matches_number(filename, episode)
    {
        episode_score += 2;
    }
    if let Some(absolute_episode) = context.absolute_episode
        && filename_matches_number(filename, absolute_episode)
    {
        episode_score += 1;
    }

    let language_score = context
        .language
        .as_deref()
        .is_some_and(|language| filename_matches_language(filename, language))
        as i32;
    let format_score = subtitle_format_rank(file.format.as_str());
    let size_score = file.content.len().min(1024 * 1024);

    (episode_score, language_score, format_score, size_score)
}

fn extract_zip_candidates(file: SubtitleFile) -> AppResult<Vec<ArchiveCandidate>> {
    let reader = Cursor::new(file.content);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|error| AppError::Repository(format!("invalid ZIP subtitle archive: {error}")))?;
    if archive.len() > MAX_ARCHIVE_FILES {
        return Err(AppError::Validation(
            "subtitle archive contains too many files".to_string(),
        ));
    }
    let mut candidates = Vec::new();
    let mut expanded_bytes = 0usize;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            AppError::Repository(format!("failed to read ZIP subtitle entry: {error}"))
        })?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !is_safe_relative_path(&name) || !is_extractable_subtitle_artifact(&name) {
            continue;
        }
        let content = read_limited(&mut entry)?;
        expanded_bytes = checked_expanded_size(expanded_bytes, content.len())?;
        candidates.push(candidate(name, content));
    }

    Ok(candidates)
}

fn extract_tar_candidates(content: Vec<u8>) -> AppResult<Vec<ArchiveCandidate>> {
    let mut archive = tar::Archive::new(Cursor::new(content));
    let mut candidates = Vec::new();
    let mut expanded_bytes = 0usize;
    let mut entry_count = 0usize;

    for entry in archive
        .entries()
        .map_err(|error| AppError::Repository(format!("invalid tar subtitle archive: {error}")))?
    {
        entry_count += 1;
        if entry_count > MAX_ARCHIVE_FILES {
            return Err(AppError::Validation(
                "subtitle archive contains too many files".to_string(),
            ));
        }
        let mut entry = entry.map_err(|error| {
            AppError::Repository(format!("failed to read tar subtitle entry: {error}"))
        })?;
        let path = entry.path().map_err(|error| {
            AppError::Repository(format!("failed to read tar subtitle entry path: {error}"))
        })?;
        let name = path.to_string_lossy().to_string();
        if !is_safe_relative_path(&name) || !is_extractable_subtitle_artifact(&name) {
            continue;
        }
        let content = read_limited(&mut entry)?;
        expanded_bytes = checked_expanded_size(expanded_bytes, content.len())?;
        candidates.push(candidate(name, content));
    }

    Ok(candidates)
}

fn extract_sevenz_candidates(content: Vec<u8>) -> AppResult<Vec<ArchiveCandidate>> {
    let mut reader =
        sevenz_rust2::ArchiveReader::new(Cursor::new(content), sevenz_rust2::Password::empty())
            .map_err(|error| {
                AppError::Repository(format!("failed to parse 7z subtitle archive: {error}"))
            })?;

    if reader.archive().files.len() > MAX_ARCHIVE_FILES {
        return Err(AppError::Validation(
            "subtitle archive contains too many files".to_string(),
        ));
    }

    let mut declared_expanded_bytes = 0usize;
    for entry in &reader.archive().files {
        if entry.is_directory() {
            continue;
        }
        let size = usize::try_from(entry.size()).map_err(|_| {
            AppError::Validation("7z subtitle entry is too large for this platform".to_string())
        })?;
        declared_expanded_bytes = checked_expanded_size(declared_expanded_bytes, size)?;
    }

    let mut candidates = Vec::new();
    let mut actual_expanded_bytes = 0usize;
    reader
        .for_each_entries(|entry, entry_reader| {
            if entry.is_directory() {
                return Ok(true);
            }
            let name = entry.name().to_string();
            if !is_safe_relative_path(&name) || !is_extractable_subtitle_artifact(&name) {
                let drained = drain_limited(entry_reader).map_err(sevenz_error)?;
                actual_expanded_bytes =
                    checked_expanded_size(actual_expanded_bytes, drained).map_err(sevenz_error)?;
                return Ok(true);
            }

            let content = read_limited(entry_reader).map_err(sevenz_error)?;
            actual_expanded_bytes = checked_expanded_size(actual_expanded_bytes, content.len())
                .map_err(sevenz_error)?;
            candidates.push(candidate(name, content));
            Ok(true)
        })
        .map_err(|error| AppError::Repository(format!("7z subtitle extraction failed: {error}")))?;

    Ok(candidates)
}

fn extract_rar_candidates(content: Vec<u8>) -> AppResult<Vec<ArchiveCandidate>> {
    let temp = tempfile::tempdir()
        .map_err(|error| AppError::Repository(format!("failed to create temp dir: {error}")))?;
    let archive_path = temp.path().join("subtitle.rar");
    std::fs::write(&archive_path, content)
        .map_err(|error| AppError::Repository(format!("failed to stage RAR archive: {error}")))?;
    let output_dir = temp.path().join("out");
    std::fs::create_dir(&output_dir).map_err(|error| {
        AppError::Repository(format!("failed to create RAR extraction dir: {error}"))
    })?;

    let file = File::open(&archive_path)
        .map_err(|error| AppError::Repository(format!("failed to open RAR archive: {error}")))?;
    let mut archive = weaver_unrar::RarArchive::open(file)
        .map_err(|error| AppError::Repository(format!("failed to parse RAR archive: {error}")))?;
    let metadata = archive.metadata();
    if metadata.members.len() > MAX_ARCHIVE_FILES {
        return Err(AppError::Validation(
            "subtitle archive contains too many files".to_string(),
        ));
    }
    let options = weaver_unrar::ExtractOptions::default();
    let mut expanded_bytes = 0usize;

    for (idx, member) in metadata.members.iter().enumerate() {
        if member.is_directory {
            continue;
        }
        let safe_name = weaver_unrar::sanitize_path(&member.name);
        if !is_safe_relative_path(&safe_name) || !is_extractable_subtitle_artifact(&safe_name) {
            continue;
        }
        let unpacked_size = member.unpacked_size.ok_or_else(|| {
            AppError::Validation("RAR subtitle entry is missing unpacked size".to_string())
        })?;
        let unpacked_size = usize::try_from(unpacked_size).map_err(|_| {
            AppError::Validation("RAR subtitle entry is too large for this platform".to_string())
        })?;
        expanded_bytes = checked_expanded_size(expanded_bytes, unpacked_size)?;
        let dest = output_dir.join(&safe_name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                AppError::Repository(format!("failed to create RAR output dir: {error}"))
            })?;
        }
        archive
            .extract_member_to_file(idx, &options, None, &dest)
            .map_err(|error| {
                AppError::Repository(format!("failed to extract RAR subtitle entry: {error}"))
            })?;
    }

    collect_extracted_candidates(&output_dir)
}

fn collect_extracted_candidates(root: &Path) -> AppResult<Vec<ArchiveCandidate>> {
    let mut candidates = Vec::new();
    let mut expanded_bytes = 0usize;
    let mut entry_count = 0usize;
    collect_extracted_candidates_inner(
        root,
        root,
        &mut candidates,
        &mut expanded_bytes,
        &mut entry_count,
    )?;
    Ok(candidates)
}

fn collect_extracted_candidates_inner(
    root: &Path,
    dir: &Path,
    candidates: &mut Vec<ArchiveCandidate>,
    expanded_bytes: &mut usize,
    entry_count: &mut usize,
) -> AppResult<()> {
    for entry in std::fs::read_dir(dir)
        .map_err(|error| AppError::Repository(format!("failed to read extraction dir: {error}")))?
    {
        *entry_count += 1;
        if *entry_count > MAX_ARCHIVE_FILES {
            return Err(AppError::Validation(
                "subtitle archive contains too many files".to_string(),
            ));
        }
        let entry = entry.map_err(|error| {
            AppError::Repository(format!("failed to read extracted subtitle entry: {error}"))
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_extracted_candidates_inner(
                root,
                &path,
                candidates,
                expanded_bytes,
                entry_count,
            )?;
            continue;
        }
        let relative = path.strip_prefix(root).map_err(|error| {
            AppError::Repository(format!(
                "failed to resolve extracted subtitle path: {error}"
            ))
        })?;
        let name = relative.to_string_lossy().to_string();
        if !is_safe_relative_path(&name) || !is_extractable_subtitle_artifact(&name) {
            continue;
        }
        let file = File::open(&path).map_err(|error| {
            AppError::Repository(format!("failed to read subtitle entry: {error}"))
        })?;
        let content = read_limited(file)?;
        *expanded_bytes = checked_expanded_size(*expanded_bytes, content.len())?;
        candidates.push(candidate(name, content));
    }
    Ok(())
}

fn candidate(filename: String, content: Vec<u8>) -> ArchiveCandidate {
    let format = extension_for_filename(&filename)
        .or_else(|| detect_subtitle_format_from_content(&content))
        .unwrap_or_else(|| "bin".to_string());
    ArchiveCandidate {
        filename: filename.clone(),
        file: SubtitleFile {
            content,
            format,
            filename: Some(filename),
            content_type: None,
        },
    }
}

fn inner_file_from_parts(
    parent_filename: Option<String>,
    parent_format: String,
    content: Vec<u8>,
    suffix: &str,
) -> SubtitleFile {
    let filename = compressed_tar_alias(parent_filename.as_deref(), suffix).or_else(|| {
        parent_filename
            .as_deref()
            .and_then(|name| strip_extension_suffix(name, suffix))
    });
    let format = filename
        .as_deref()
        .and_then(extension_for_filename)
        .filter(|format| format != "gz" && format != "zst" && format != "xz")
        .unwrap_or(parent_format);
    SubtitleFile {
        content,
        format,
        filename,
        content_type: None,
    }
}

fn detect_artifact_kind(file: &SubtitleFile) -> Option<ArtifactKind> {
    if let Some(filename) = file.filename.as_deref()
        && let Some(kind) = artifact_kind_from_filename(filename)
    {
        return Some(kind);
    }
    if let Some(kind) = artifact_kind_from_content_type(file.content_type.as_deref()) {
        return Some(kind);
    }
    artifact_kind_from_magic(&file.content)
}

fn artifact_kind_from_filename(filename: &str) -> Option<ArtifactKind> {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        Some(ArtifactKind::Zip)
    } else if lower.ends_with(".tar") {
        Some(ArtifactKind::Tar)
    } else if lower.ends_with(".7z") {
        Some(ArtifactKind::SevenZip)
    } else if lower.ends_with(".rar") {
        Some(ArtifactKind::Rar)
    } else if lower.ends_with(".gz") || lower.ends_with(".tgz") {
        Some(ArtifactKind::Gzip)
    } else if lower.ends_with(".zst") || lower.ends_with(".tzst") {
        Some(ArtifactKind::Zstd)
    } else if lower.ends_with(".xz") || lower.ends_with(".txz") {
        Some(ArtifactKind::Xz)
    } else {
        None
    }
}

fn artifact_kind_from_content_type(content_type: Option<&str>) -> Option<ArtifactKind> {
    let lower = content_type?.to_ascii_lowercase();
    if lower.contains("zip") {
        Some(ArtifactKind::Zip)
    } else if lower.contains("x-tar") || lower.contains("tar") {
        Some(ArtifactKind::Tar)
    } else if lower.contains("7z") {
        Some(ArtifactKind::SevenZip)
    } else if lower.contains("rar") {
        Some(ArtifactKind::Rar)
    } else if lower.contains("gzip") || lower.contains("x-gzip") {
        Some(ArtifactKind::Gzip)
    } else if lower.contains("zstd") || lower.contains("zst") {
        Some(ArtifactKind::Zstd)
    } else if lower.contains("xz") || lower.contains("lzma") {
        Some(ArtifactKind::Xz)
    } else {
        None
    }
}

fn artifact_kind_from_magic(content: &[u8]) -> Option<ArtifactKind> {
    if content.starts_with(b"PK\x03\x04")
        || content.starts_with(b"PK\x05\x06")
        || content.starts_with(b"PK\x07\x08")
    {
        Some(ArtifactKind::Zip)
    } else if content.starts_with(b"7z\xBC\xAF\x27\x1C") {
        Some(ArtifactKind::SevenZip)
    } else if content.starts_with(b"Rar!\x1A\x07\x00")
        || content.starts_with(b"Rar!\x1A\x07\x01\x00")
    {
        Some(ArtifactKind::Rar)
    } else if content.starts_with(b"\x1F\x8B") {
        Some(ArtifactKind::Gzip)
    } else if content.starts_with(b"\x28\xB5\x2F\xFD") {
        Some(ArtifactKind::Zstd)
    } else if content.starts_with(b"\xFD\x37\x7A\x58\x5A\x00") {
        Some(ArtifactKind::Xz)
    } else if content.len() > 262 && &content[257..262] == b"ustar" {
        Some(ArtifactKind::Tar)
    } else {
        None
    }
}

fn final_subtitle_format(file: &SubtitleFile) -> Option<String> {
    file.filename
        .as_deref()
        .and_then(extension_for_filename)
        .filter(|format| is_supported_subtitle_format(format))
        .or_else(|| {
            is_supported_subtitle_format(&file.format).then(|| normalize_extension(&file.format))
        })
        .or_else(|| detect_subtitle_format_from_content(&file.content))
}

fn detect_subtitle_format_from_content(content: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(content)
        .ok()?
        .trim_start_matches('\u{feff}')
        .trim_start();
    if text.starts_with("WEBVTT") {
        return Some("vtt".to_string());
    }
    if text.starts_with("[Script Info]") || text.contains("\nDialogue:") {
        return Some("ass".to_string());
    }
    if text.contains("-->") {
        return Some("srt".to_string());
    }
    None
}

fn subtitle_format_rank(format: &str) -> i32 {
    match normalize_extension(format).as_str() {
        "ass" => 60,
        "ssa" => 55,
        "srt" => 50,
        "vtt" => 40,
        "sub" => 10,
        "idx" => 5,
        _ => 0,
    }
}

fn subtitle_content_type(format: &str) -> Option<&'static str> {
    match normalize_extension(format).as_str() {
        "srt" => Some("application/x-subrip"),
        "ass" | "ssa" => Some("text/x-ssa"),
        "vtt" => Some("text/vtt"),
        "sub" | "idx" => Some("application/octet-stream"),
        _ => None,
    }
}

fn is_extractable_subtitle_artifact(filename: &str) -> bool {
    extension_for_filename(filename)
        .as_deref()
        .is_some_and(is_supported_subtitle_format)
        || artifact_kind_from_filename(filename).is_some()
}

fn extension_for_filename(filename: &str) -> Option<String> {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(normalize_extension)
}

fn normalize_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

fn strip_extension_suffix(filename: &str, suffix: &str) -> Option<String> {
    filename
        .to_ascii_lowercase()
        .ends_with(suffix)
        .then(|| filename[..filename.len() - suffix.len()].to_string())
}

fn filename_matches_number(filename: &str, number: i32) -> bool {
    let lower = filename.to_ascii_lowercase();
    let plain = number.to_string();
    let padded = format!("{number:02}");
    lower.contains(&format!("e{padded}"))
        || lower.contains(&format!("ep{padded}"))
        || lower.contains(&format!("episode {number}"))
        || split_filename_tokens(&lower)
            .iter()
            .any(|token| token == &plain || token == &padded)
}

fn filename_matches_language(filename: &str, language: &str) -> bool {
    let normalized = language.to_ascii_lowercase();
    let fallback = [normalized.as_str()];
    let aliases: &[&str] = match normalized.as_str() {
        "eng" | "en" => &["eng", "en", "english"],
        "jpn" | "ja" => &["jpn", "ja", "japanese"],
        "spa" | "es" => &["spa", "es", "spanish"],
        "fre" | "fr" => &["fre", "fra", "fr", "french"],
        "ger" | "de" => &["ger", "deu", "de", "german"],
        "ita" | "it" => &["ita", "it", "italian"],
        "por" | "pt" => &["por", "pt", "portuguese"],
        _ => &fallback,
    };
    let tokens = split_filename_tokens(&filename.to_ascii_lowercase());
    aliases
        .iter()
        .any(|alias| tokens.iter().any(|token| token == alias))
}

fn compressed_tar_alias(filename: Option<&str>, suffix: &str) -> Option<String> {
    let filename = filename?;
    match suffix {
        ".gz" => strip_extension_suffix(filename, ".tgz").map(|name| format!("{name}.tar")),
        ".zst" => strip_extension_suffix(filename, ".tzst").map(|name| format!("{name}.tar")),
        ".xz" => strip_extension_suffix(filename, ".txz").map(|name| format!("{name}.tar")),
        _ => None,
    }
}

fn split_filename_tokens(filename: &str) -> Vec<String> {
    filename
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn is_safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn read_limited<R: Read>(reader: R) -> AppResult<Vec<u8>> {
    let mut limited = reader.take((MAX_EXPANDED_BYTES + 1) as u64);
    let mut out = Vec::new();
    limited.read_to_end(&mut out).map_err(|error| {
        AppError::Repository(format!("failed to read subtitle artifact: {error}"))
    })?;
    ensure_expanded_size(out.len())?;
    Ok(out)
}

fn drain_limited<R: Read>(reader: R) -> AppResult<usize> {
    let mut limited = reader.take((MAX_EXPANDED_BYTES + 1) as u64);
    let bytes = io::copy(&mut limited, &mut io::sink()).map_err(|error| {
        AppError::Repository(format!("failed to drain subtitle archive entry: {error}"))
    })?;
    let bytes = bytes as usize;
    ensure_expanded_size(bytes)?;
    Ok(bytes)
}

fn sevenz_error(error: impl ToString) -> sevenz_rust2::Error {
    sevenz_rust2::Error::Other(Cow::Owned(error.to_string()))
}

struct LimitedWriter {
    content: Vec<u8>,
    limit: usize,
}

impl LimitedWriter {
    fn new(limit: usize) -> Self {
        Self {
            content: Vec::new(),
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.content
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.content.len().saturating_add(buf.len()) > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "subtitle artifact expands beyond the configured byte limit",
            ));
        }
        self.content.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn ensure_expanded_size(size: usize) -> AppResult<()> {
    if size > MAX_EXPANDED_BYTES {
        return Err(AppError::Validation(format!(
            "subtitle artifact expands beyond the {} byte limit",
            MAX_EXPANDED_BYTES
        )));
    }
    Ok(())
}

fn checked_expanded_size(current: usize, next: usize) -> AppResult<usize> {
    let total = current.saturating_add(next);
    ensure_expanded_size(total)?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const ASS_CONTENT: &[u8] = b"[Script Info]\nTitle: Test\n\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hello\n";
    const SRT_CONTENT: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n";

    fn subtitle_file(filename: &str, content: Vec<u8>) -> SubtitleFile {
        SubtitleFile {
            content,
            format: extension_for_filename(filename).unwrap_or_else(|| "bin".to_string()),
            filename: Some(filename.to_string()),
            content_type: None,
        }
    }

    fn context() -> SubtitleExtractionContext {
        SubtitleExtractionContext {
            language: Some("eng".to_string()),
            episode: Some(17),
            absolute_episode: Some(17),
        }
    }

    fn tar_with_entry(name: &str, content: &[u8]) -> Vec<u8> {
        let mut tar = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar);
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_cksum();
            builder.append_data(&mut header, name, content).unwrap();
            builder.finish().unwrap();
        }
        tar
    }

    #[tokio::test]
    async fn raw_subtitle_passes_through() {
        let normalized = normalize_downloaded_subtitle(
            subtitle_file("release.ass", ASS_CONTENT.to_vec()),
            context(),
        )
        .await
        .unwrap();

        assert_eq!(normalized.format, "ass");
        assert_eq!(normalized.content, ASS_CONTENT);
    }

    #[tokio::test]
    async fn gzip_subtitle_decompresses() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(SRT_CONTENT).unwrap();
        let gz = encoder.finish().unwrap();

        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.srt.gz", gz), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "srt");
        assert_eq!(normalized.content, SRT_CONTENT);
    }

    #[tokio::test]
    async fn zstd_subtitle_decompresses() {
        let zst = zstd::encode_all(Cursor::new(SRT_CONTENT), 0).unwrap();
        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.srt.zst", zst), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "srt");
        assert_eq!(normalized.content, SRT_CONTENT);
    }

    #[tokio::test]
    async fn xz_subtitle_decompresses() {
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut Cursor::new(ASS_CONTENT), &mut xz).unwrap();
        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.ass.xz", xz), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "ass");
        assert_eq!(normalized.content, ASS_CONTENT);
    }

    #[tokio::test]
    async fn zip_selects_episode_language_subtitle() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("Show.S01E16.eng.srt", options).unwrap();
        writer.write_all(SRT_CONTENT).unwrap();
        writer.start_file("Show.S01E17.eng.ass", options).unwrap();
        writer.write_all(ASS_CONTENT).unwrap();
        let zip = writer.finish().unwrap().into_inner();

        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.zip", zip), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "ass");
        assert_eq!(normalized.content, ASS_CONTENT);
    }

    #[tokio::test]
    async fn tar_archive_extracts_subtitle() {
        let tar = tar_with_entry("Show.S01E17.eng.srt", SRT_CONTENT);

        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.tar", tar), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "srt");
        assert_eq!(normalized.content, SRT_CONTENT);
    }

    #[tokio::test]
    async fn compressed_tar_archive_extracts_subtitle() {
        let tar = tar_with_entry("Show.S01E17.eng.ass", ASS_CONTENT);
        let zst = zstd::encode_all(Cursor::new(tar), 0).unwrap();

        let normalized =
            normalize_downloaded_subtitle(subtitle_file("release.tar.zst", zst), context())
                .await
                .unwrap();

        assert_eq!(normalized.format, "ass");
        assert_eq!(normalized.content, ASS_CONTENT);
    }

    #[test]
    fn review_regression_tgz_aliases_to_tar_when_unwrapping_gzip() {
        let file = inner_file_from_parts(
            Some("release.tgz".to_string()),
            "gz".to_string(),
            Vec::new(),
            ".gz",
        );

        assert_eq!(file.filename.as_deref(), Some("release.tar"));
        assert_eq!(file.format, "tar");
    }

    #[tokio::test]
    async fn ambiguous_archive_fails() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("one.eng.srt", options).unwrap();
        writer.write_all(SRT_CONTENT).unwrap();
        writer.start_file("two.eng.srt", options).unwrap();
        writer.write_all(SRT_CONTENT).unwrap();
        let zip = writer.finish().unwrap().into_inner();

        let err = normalize_downloaded_subtitle(subtitle_file("release.zip", zip), context())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("multiple equally ranked"));
    }
}
