use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use super::*;
use crate::helpers::{
    has_usable_release_title_signal, normalize_release_title_signal, parse_usable_release_title,
};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use scryer_domain::VIDEO_EXTENSIONS;
use unicode_normalization::UnicodeNormalization;

const LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY: usize = 16;
const LIBRARY_PROBE_SIGNATURE_DIRECTORY_SCHEME: &str = "immediate_children_v1";
const LIBRARY_PROBE_SIGNATURE_FILE_SCHEME: &str = "file_snapshot_v1";
pub(crate) const LIBRARY_SCAN_MAX_RECURSIVE_DEPTH: usize = 3;

const LIBRARY_IGNORED_DIR_NAMES: &[&str] = &["@eadir", ".@__thumb", "plex versions"];
const LIBRARY_IGNORED_MOVIE_SUBDIR_NAMES: &[&str] = &[
    "extras",
    "extrafanart",
    "behind the scenes",
    "deleted scenes",
    "featurette",
    "featurettes",
    "interview",
    "interviews",
    "other",
    "scene",
    "scenes",
    "sample",
    "samples",
    "short",
    "shorts",
    "trailer",
    "trailers",
];

#[derive(Clone, Debug)]
pub(crate) struct MovieTopLevelEntry {
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
}

type LibraryPathBatch = Vec<PathBuf>;
pub(crate) type LibraryPathBatchReceiver = tokio::sync::mpsc::Receiver<AppResult<LibraryPathBatch>>;
pub(crate) type MovieTopLevelEntryBatchReceiver =
    tokio::sync::mpsc::Receiver<AppResult<Vec<MovieTopLevelEntry>>>;

pub(crate) fn extract_library_queries(
    path: &str,
    library_root: &str,
) -> (Vec<String>, Option<u32>) {
    let path = stored_path_to_path_buf(path);
    let root = stored_path_to_path_buf(library_root);

    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parsed = normalize_release_title_signal(parse_release_metadata(stem.as_str()));
    let parsed_has_usable_title_signal = has_usable_release_title_signal(&parsed);
    let parsed_queries = if parsed_has_usable_title_signal {
        if parsed.normalized_title_variants.is_empty() {
            vec![parsed.normalized_title.clone()]
        } else {
            parsed.normalized_title_variants.clone()
        }
    } else {
        Vec::new()
    };

    let mut queries = Vec::new();
    let mut seen_normalized = HashSet::new();
    let mut folder_year = None;
    let mut folder_queries = Vec::new();
    let mut raw_folder_query = None;

    if let Some(parent) = path.parent() {
        if parent != root.as_path()
            && let Some(folder_name) = parent
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.trim().is_empty())
        {
            let clean = normalize_folder_name(&folder_name);
            let (clean_title, clean_year) = strip_year_suffix(&clean);
            if let Some(parsed_folder) = parse_usable_release_title(&folder_name) {
                let looks_human_named = !folder_name.contains('.') && !folder_name.contains('_');
                let has_release_decoration = parsed_folder
                    .release_group
                    .as_ref()
                    .is_some_and(|group| !group.trim().is_empty())
                    || parsed_folder.quality.is_some()
                    || parsed_folder.source.is_some()
                    || parsed_folder.video_codec.is_some()
                    || parsed_folder.video_encoding.is_some()
                    || parsed_folder.audio.is_some()
                    || !parsed_folder.audio_codecs.is_empty()
                    || parsed_folder.audio_channels.is_some()
                    || parsed_folder.streaming_service.is_some()
                    || parsed_folder.edition.is_some()
                    || parsed_folder.is_proper_upload
                    || parsed_folder.is_repack
                    || parsed_folder.is_remux
                    || parsed_folder.is_bd_disk
                    || parsed_folder.is_dual_audio
                    || parsed_folder.episode.is_some();
                let raw_folder_title = parsed_folder
                    .year
                    .and_then(|year| u32::try_from(year).ok())
                    .map(|year| strip_trailing_plain_year_token(&clean_title, year))
                    .unwrap_or_else(|| clean_title.clone());
                if !clean_title.trim().is_empty()
                    && !has_release_decoration
                    && (clean_year.is_some() || parsed_folder.year.is_some() || looks_human_named)
                {
                    raw_folder_query = Some(raw_folder_title);
                }
                let parsed_folder_queries = if parsed_folder.normalized_title_variants.is_empty() {
                    vec![parsed_folder.normalized_title.clone()]
                } else {
                    parsed_folder.normalized_title_variants.clone()
                };
                folder_queries.extend(parsed_folder_queries);
                folder_year = parsed_folder.year.and_then(|year| u32::try_from(year).ok());
                if folder_year.is_none() {
                    folder_year = clean_year;
                }
            } else if !clean_title.trim().is_empty() {
                folder_queries.push(clean_title);
                folder_year = clean_year;
            }
        }
    }

    if !parsed_has_usable_title_signal {
        for folder_query in folder_queries.iter().cloned() {
            push_unique_query(&mut queries, &mut seen_normalized, folder_query);
        }
    }

    for query in parsed_queries {
        if let Some(reduced) = part_reduced_query(query.as_str()) {
            push_unique_query(&mut queries, &mut seen_normalized, reduced);
        } else {
            push_unique_query(&mut queries, &mut seen_normalized, query);
        }
    }

    if parsed_has_usable_title_signal {
        for folder_query in folder_queries {
            push_unique_query(&mut queries, &mut seen_normalized, folder_query);
        }
    }

    if let Some(raw_folder_query) = raw_folder_query {
        push_unique_literal_query(&mut queries, raw_folder_query);
    }

    (
        queries,
        parsed_has_usable_title_signal
            .then_some(parsed.year)
            .flatten()
            .and_then(|year| u32::try_from(year).ok())
            .or(folder_year),
    )
}

pub(crate) fn normalize_folder_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_space = false;
    for ch in name.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

pub(crate) fn strip_year_suffix(folder: &str) -> (String, Option<u32>) {
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let Some(close_pos) = folder.rfind(close)
            && let Some(open_pos) = folder[..close_pos].rfind(open)
            && let Ok(year) = folder[open_pos + 1..close_pos].trim().parse::<u32>()
            && (1888..=2100).contains(&year)
        {
            let title = folder[..open_pos].trim_end().to_string();
            if !title.is_empty() {
                return (title, Some(year));
            }
        }
    }

    (folder.to_string(), None)
}

fn strip_trailing_plain_year_token(folder: &str, year: u32) -> String {
    let suffix = year.to_string();
    if let Some(prefix) = folder.strip_suffix(&suffix) {
        let trimmed = prefix.trim_end();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    folder.to_string()
}

fn push_unique_query(
    queries: &mut Vec<String>,
    seen_normalized: &mut HashSet<String>,
    query: String,
) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return;
    }

    let normalized = crate::app_usecase_rss::normalize_for_matching(trimmed);
    if normalized.is_empty() || !seen_normalized.insert(normalized) {
        return;
    }

    queries.push(trimmed.to_string());
}

fn push_unique_literal_query(queries: &mut Vec<String>, query: String) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return;
    }

    let normalized = trimmed
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized.trim().is_empty() {
        return;
    }

    if queries.iter().any(|existing| {
        existing
            .trim()
            .nfkc()
            .flat_map(char::to_lowercase)
            .collect::<String>()
            == normalized
    }) {
        return;
    }

    queries.push(trimmed.to_string());
}

fn part_reduced_query(query: &str) -> Option<String> {
    let tokens = query.split_whitespace().collect::<Vec<_>>();
    if !tokens
        .iter()
        .any(|token| token.eq_ignore_ascii_case("part"))
    {
        return None;
    }
    let reduced = tokens
        .into_iter()
        .filter(|token| !token.eq_ignore_ascii_case("part"))
        .collect::<Vec<_>>();
    (reduced.len() >= 2).then(|| reduced.join(" "))
}

pub(crate) fn elapsed_ms_u64(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(crate) async fn list_child_directories(root: &Path) -> AppResult<Vec<PathBuf>> {
    Ok(crate::filesystem_walk::FilesystemWalker::new()
        .list_child_directories(root)?
        .into_iter()
        .filter(|path| !should_skip_library_top_level_entry(path, true))
        .collect())
}

pub(crate) async fn list_series_loose_root_files(root: &Path) -> AppResult<Vec<LibraryFile>> {
    let mut entries = tokio::fs::read_dir(root).await.map_err(|error| {
        AppError::Repository(format!("failed to read {}: {error}", root.display()))
    })?;
    let mut results = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        AppError::Repository(format!("failed to read {}: {error}", root.display()))
    })? {
        let path = entry.path();
        let file_type = entry.file_type().await.map_err(|error| {
            AppError::Repository(format!("failed to inspect {}: {error}", path.display()))
        })?;
        if !file_type.is_file()
            || should_skip_library_top_level_entry(&path, false)
            || !is_allowed_video_path(&path)
        {
            continue;
        }

        results.push(LibraryFile {
            path: path_to_stored_string(&path),
            display_name: path
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        });
    }

    results.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(results)
}

pub(crate) async fn stream_child_directories_batched(
    root: &Path,
    batch_size: usize,
) -> AppResult<LibraryPathBatchReceiver> {
    if batch_size == 0 {
        return Err(AppError::Validation(
            "batch size must be greater than 0".into(),
        ));
    }

    let root = root.to_path_buf();
    let (sender, receiver) = tokio::sync::mpsc::channel(LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY);

    tokio::spawn(async move {
        let sender_for_worker = sender.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut receiver_closed = false;
            let mut batch = Vec::with_capacity(batch_size.min(256));

            crate::filesystem_walk::FilesystemWalker::new().visit_child_directories(
                &root,
                |path| {
                    if receiver_closed {
                        return Ok(());
                    }

                    if should_skip_library_top_level_entry(&path, true) {
                        return Ok(());
                    }

                    batch.push(path);
                    if batch.len() >= batch_size {
                        let next_batch = std::mem::take(&mut batch);
                        if sender_for_worker.blocking_send(Ok(next_batch)).is_err() {
                            receiver_closed = true;
                        }
                    }

                    Ok(())
                },
            )?;

            if !receiver_closed && !batch.is_empty() {
                let _ = sender_for_worker.blocking_send(Ok(batch));
            }

            Ok::<(), AppError>(())
        })
        .await
        .map_err(|error| AppError::Repository(error.to_string()))
        .and_then(|result| result);

        if let Err(error) = result {
            let _ = sender.send(Err(error)).await;
        }
    });

    Ok(receiver)
}

pub(crate) async fn list_movie_top_level_entries(
    root: &Path,
) -> AppResult<Vec<MovieTopLevelEntry>> {
    let mut entries = tokio::fs::read_dir(root).await.map_err(|error| {
        AppError::Repository(format!("failed to read {}: {error}", root.display()))
    })?;
    let mut results = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        AppError::Repository(format!("failed to read {}: {error}", root.display()))
    })? {
        let path = entry.path();
        let file_type = entry.file_type().await.map_err(|error| {
            AppError::Repository(format!("failed to inspect {}: {error}", path.display()))
        })?;
        if file_type.is_dir() && !should_skip_library_top_level_entry(&path, true) {
            results.push(MovieTopLevelEntry { path, is_dir: true });
            continue;
        }

        if file_type.is_file()
            && !should_skip_library_top_level_entry(&path, false)
            && is_allowed_video_path(&path)
        {
            results.push(MovieTopLevelEntry {
                path,
                is_dir: false,
            });
        }
    }

    results.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(results)
}

pub(crate) async fn stream_movie_top_level_entries_batched(
    root: &Path,
    batch_size: usize,
) -> AppResult<MovieTopLevelEntryBatchReceiver> {
    if batch_size == 0 {
        return Err(AppError::Validation(
            "batch size must be greater than 0".into(),
        ));
    }

    let root = root.to_path_buf();
    let (sender, receiver) = tokio::sync::mpsc::channel(LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY);

    tokio::spawn(async move {
        let result = async {
            let mut entries = tokio::fs::read_dir(&root).await.map_err(|error| {
                AppError::Repository(format!("failed to read {}: {error}", root.display()))
            })?;
            let mut batch = Vec::with_capacity(batch_size.min(256));

            while let Some(entry) = entries.next_entry().await.map_err(|error| {
                AppError::Repository(format!("failed to read {}: {error}", root.display()))
            })? {
                let path = entry.path();
                let file_type = entry.file_type().await.map_err(|error| {
                    AppError::Repository(format!("failed to inspect {}: {error}", path.display()))
                })?;
                if file_type.is_dir() && !should_skip_library_top_level_entry(&path, true) {
                    batch.push(MovieTopLevelEntry { path, is_dir: true });
                } else if file_type.is_file()
                    && !should_skip_library_top_level_entry(&path, false)
                    && is_allowed_video_path(&path)
                {
                    batch.push(MovieTopLevelEntry {
                        path,
                        is_dir: false,
                    });
                }

                if batch.len() >= batch_size {
                    let next_batch = std::mem::take(&mut batch);
                    if sender.send(Ok(next_batch)).await.is_err() {
                        return Ok(());
                    }
                }
            }

            if !batch.is_empty() {
                let _ = sender.send(Ok(batch)).await;
            }

            Ok::<(), AppError>(())
        }
        .await;

        if let Err(error) = result {
            let _ = sender.send(Err(error)).await;
        }
    });

    Ok(receiver)
}

pub(crate) fn is_ignored_library_dir_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized.starts_with('.')
        || normalized.ends_with(".trickplay")
        || LIBRARY_IGNORED_DIR_NAMES.contains(&normalized.as_str())
}

pub(crate) fn is_ignored_library_file_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized == ".ds_store"
        || normalized == "thumbs.db"
        || normalized.starts_with("._")
        || normalized.starts_with(".unmanic")
}

pub(crate) fn is_ignored_movie_subdir_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    LIBRARY_IGNORED_MOVIE_SUBDIR_NAMES.contains(&normalized.as_str())
}

pub(crate) fn should_skip_library_top_level_entry(path: &Path, is_dir: bool) -> bool {
    let Some(name) = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
    else {
        return false;
    };

    if is_dir {
        is_ignored_library_dir_name(name.as_str())
    } else {
        is_ignored_library_file_name(name.as_str())
    }
}

pub(crate) fn should_skip_library_subpath(root: &Path, path: &Path, is_dir: bool) -> bool {
    let Some(relative) = path.strip_prefix(root).ok() else {
        return false;
    };

    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else {
            continue;
        };
        if ((components.peek().is_some() || is_dir) && is_ignored_library_dir_name(name))
            || is_ignored_library_file_name(name)
        {
            return true;
        }
    }

    false
}

pub(crate) fn should_skip_movie_library_subpath(root: &Path, path: &Path, is_dir: bool) -> bool {
    if should_skip_library_subpath(root, path, is_dir) {
        return true;
    }

    let Some(relative) = path.strip_prefix(root).ok() else {
        return false;
    };

    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else {
            continue;
        };
        if (components.peek().is_some() || is_dir) && is_ignored_movie_subdir_name(name) {
            return true;
        }
    }

    false
}

fn is_allowed_video_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.as_str()))
}

pub(crate) fn matching_movie_nfo_path(path: &Path) -> Option<String> {
    let same_stem = path.with_extension("nfo");
    if same_stem.is_file() {
        return Some(path_to_stored_string(&same_stem));
    }

    let parent = path.parent()?;
    let movie_nfo = parent.join("movie.nfo");
    if movie_nfo.is_file() {
        return Some(path_to_stored_string(&movie_nfo));
    }

    None
}

pub(crate) async fn matching_movie_nfo_path_async(path: &Path) -> Option<String> {
    let same_stem = path.with_extension("nfo");
    if tokio::fs::metadata(&same_stem)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Some(path_to_stored_string(&same_stem));
    }

    let parent = path.parent()?;
    let movie_nfo = parent.join("movie.nfo");
    if tokio::fs::metadata(&movie_nfo)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Some(path_to_stored_string(&movie_nfo));
    }

    None
}

pub(crate) fn derive_movie_probe_path(
    root: &Path,
    title: &Title,
    collections: &[Collection],
) -> Option<PathBuf> {
    if let Some(folder_path) = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(stored_path_to_path_buf)
        .filter(|path| path.starts_with(root))
    {
        return Some(folder_path);
    }

    let mut ordered_paths = collections
        .iter()
        .filter_map(|collection| collection.ordered_path.as_deref())
        .map(stored_path_to_path_buf)
        .filter(|path| path.starts_with(root))
        .collect::<Vec<_>>();
    ordered_paths.sort();
    ordered_paths.dedup();

    let first = ordered_paths.into_iter().next()?;
    if let Some(parent) = first.parent()
        && parent != root
    {
        return Some(parent.to_path_buf());
    }

    Some(first)
}

async fn compute_library_probe_signature(path: &Path) -> AppResult<(String, String)> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || compute_library_probe_signature_blocking(path))
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
}

#[derive(Clone, Debug)]
struct PendingLibraryProbe {
    path: String,
    scheme: String,
    value: String,
    now: chrono::DateTime<Utc>,
    stored_probe: Option<LibraryProbeSignature>,
}

pub(crate) enum BackgroundRefreshProbeOutcome<T> {
    Unchanged,
    Changed(T),
}

async fn begin_background_refresh_probe(
    app: &AppUseCase,
    title_id: &str,
    path: &Path,
) -> AppResult<Option<PendingLibraryProbe>> {
    let path_string = path_to_stored_string(path);
    let now = Utc::now();
    let (scheme, value) = compute_library_probe_signature(path).await?;
    let stored_probe = app
        .services
        .library
        .library_probe_signatures
        .get_probe_signature(title_id)
        .await?;
    let unchanged = stored_probe.as_ref().is_some_and(|probe| {
        probe.path == path_string
            && probe.probe_signature_scheme.as_deref() == Some(scheme.as_str())
            && probe.probe_signature_value.as_deref() == Some(value.as_str())
    });

    if unchanged {
        app.services
            .library
            .library_probe_signatures
            .upsert_probe_signature(&LibraryProbeSignature {
                title_id: title_id.to_string(),
                path: path_string,
                probe_signature_scheme: Some(scheme),
                probe_signature_value: Some(value),
                last_probed_at: Some(now),
                last_changed_at: stored_probe.and_then(|probe| probe.last_changed_at),
            })
            .await?;
        return Ok(None);
    }

    Ok(Some(PendingLibraryProbe {
        path: path_string,
        scheme,
        value,
        now,
        stored_probe,
    }))
}

async fn persist_background_refresh_probe_result(
    app: &AppUseCase,
    title_id: &str,
    probe: PendingLibraryProbe,
    has_delta: bool,
) -> AppResult<()> {
    app.services
        .library
        .library_probe_signatures
        .upsert_probe_signature(&LibraryProbeSignature {
            title_id: title_id.to_string(),
            path: probe.path,
            probe_signature_scheme: Some(probe.scheme),
            probe_signature_value: Some(probe.value),
            last_probed_at: Some(probe.now),
            last_changed_at: has_delta
                .then_some(probe.now)
                .or_else(|| probe.stored_probe.and_then(|stored| stored.last_changed_at)),
        })
        .await
}

pub(crate) async fn run_background_refresh_probe_with_delta<T, Fut>(
    app: &AppUseCase,
    title_id: &str,
    path: &Path,
    scan_and_diff: Fut,
) -> AppResult<BackgroundRefreshProbeOutcome<T>>
where
    Fut: std::future::Future<Output = AppResult<(T, HashSet<String>, HashSet<String>)>>,
{
    let Some(probe) = begin_background_refresh_probe(app, title_id, path).await? else {
        return Ok(BackgroundRefreshProbeOutcome::Unchanged);
    };

    let (payload, discovered_paths, existing_paths) = scan_and_diff.await?;
    let has_delta = discovered_paths != existing_paths;
    persist_background_refresh_probe_result(app, title_id, probe, has_delta).await?;

    if has_delta {
        Ok(BackgroundRefreshProbeOutcome::Changed(payload))
    } else {
        Ok(BackgroundRefreshProbeOutcome::Unchanged)
    }
}

fn compute_library_probe_signature_blocking(path: PathBuf) -> AppResult<(String, String)> {
    let metadata = std::fs::metadata(&path).map_err(|error| {
        AppError::Repository(format!("failed to inspect {}: {error}", path.display()))
    })?;

    if metadata.is_dir() {
        let mut markers = Vec::new();
        let entries = std::fs::read_dir(&path).map_err(|error| {
            AppError::Repository(format!("failed to read {}: {error}", path.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                AppError::Repository(format!(
                    "failed to read entry in {}: {error}",
                    path.display()
                ))
            })?;
            let child_path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect filesystem entry {}: {error}",
                    child_path.display()
                ))
            })?;

            let (kind, child_metadata) = if file_type.is_dir() {
                ("dir", std::fs::metadata(&child_path).ok())
            } else if file_type.is_file() {
                ("file", std::fs::metadata(&child_path).ok())
            } else if file_type.is_symlink() {
                match std::fs::metadata(&child_path) {
                    Ok(metadata) if metadata.is_dir() => ("dir", Some(metadata)),
                    Ok(metadata) if metadata.is_file() => ("file", Some(metadata)),
                    _ => continue,
                }
            } else {
                continue;
            };

            if should_skip_library_top_level_entry(&child_path, kind == "dir") {
                continue;
            }

            let marker = child_metadata
                .as_ref()
                .map(metadata_probe_marker)
                .unwrap_or_else(|| "unknown".to_string());
            let name = child_path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            markers.push(format!("{name}|{kind}|{marker}"));
        }
        markers.sort();
        let payload = markers.join("\n");
        Ok((
            LIBRARY_PROBE_SIGNATURE_DIRECTORY_SCHEME.to_string(),
            sha256_hex(payload),
        ))
    } else {
        let payload = metadata_probe_marker(&metadata);
        Ok((
            LIBRARY_PROBE_SIGNATURE_FILE_SCHEME.to_string(),
            sha256_hex(payload),
        ))
    }
}

fn metadata_probe_marker(metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| format!("{}:{}", value.as_secs(), value.subsec_nanos()))
        .unwrap_or_else(|| "unknown".to_string());
    format!("{modified}|{}", metadata.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn list_child_directories_skips_library_junk_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::create_dir_all(dir.path().join("Show A"))
            .await
            .expect("show a");
        tokio::fs::create_dir_all(dir.path().join("@eaDir"))
            .await
            .expect("@eaDir");
        tokio::fs::create_dir_all(dir.path().join(".stfolder"))
            .await
            .expect(".stfolder");
        tokio::fs::create_dir_all(dir.path().join("Show A.trickplay"))
            .await
            .expect(".trickplay");

        let child_dirs = list_child_directories(dir.path())
            .await
            .expect("child dirs");

        assert_eq!(child_dirs, vec![dir.path().join("Show A")]);
    }

    #[tokio::test]
    async fn list_movie_top_level_entries_skips_junk_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::create_dir_all(dir.path().join("Movie A"))
            .await
            .expect("movie dir");
        tokio::fs::create_dir_all(dir.path().join("@eaDir"))
            .await
            .expect("@eaDir");
        tokio::fs::write(dir.path().join("Movie.B.2024.mkv"), b"video")
            .await
            .expect("movie file");
        tokio::fs::write(dir.path().join(".DS_Store"), b"junk")
            .await
            .expect(".DS_Store");

        let entries = list_movie_top_level_entries(dir.path())
            .await
            .expect("movie entries");

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string())
                .collect::<Vec<_>>(),
            vec!["Movie A".to_string(), "Movie.B.2024.mkv".to_string()]
        );
    }

    #[test]
    fn should_skip_movie_library_subpath_allows_sample_leaf_files() {
        let root = Path::new("/library");
        let path = Path::new("/library/Movie Title/Sample.2024.BluRay.mkv");

        assert!(!should_skip_movie_library_subpath(root, path, false));
    }

    #[test]
    fn should_skip_library_subpath_for_trickplay_directories() {
        let root = Path::new("/library");
        let path = Path::new("/library/Show Name/Show.Name.S01E01.trickplay");

        assert!(should_skip_library_subpath(root, path, true));
    }

    #[cfg(unix)]
    #[test]
    fn should_not_skip_non_utf8_top_level_entries() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"/library/\xFFMovie.mkv"));
        assert!(!should_skip_library_top_level_entry(path, false));
    }

    #[test]
    fn derive_movie_probe_path_ignores_stale_folder_path_from_other_root() {
        let root = Path::new("/Volumes/Media/Movies");
        let title = Title {
            id: "title-1".into(),
            name: "Existing Movie".into(),
            facet: MediaFacet::Movie,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: Some("/Volumes/Archive/Movies/Existing Movie".into()),
        };
        let collections = vec![Collection {
            id: "collection-1".into(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".into(),
            label: None,
            ordered_path: Some(
                "/Volumes/Media/Movies/Existing Movie/Existing.Movie.2024.2160p.WEB-DL.mkv".into(),
            ),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        }];

        assert_eq!(
            derive_movie_probe_path(root, &title, &collections),
            Some(PathBuf::from("/Volumes/Media/Movies/Existing Movie"))
        );
    }
}
