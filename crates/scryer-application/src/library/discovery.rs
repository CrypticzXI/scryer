use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use super::*;
use scryer_domain::VIDEO_EXTENSIONS;

const LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY: usize = 16;
const LIBRARY_PROBE_SIGNATURE_DIRECTORY_SCHEME: &str = "immediate_children_v1";
const LIBRARY_PROBE_SIGNATURE_FILE_SCHEME: &str = "file_snapshot_v1";

#[derive(Clone, Debug)]
pub(crate) struct MovieTopLevelEntry {
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
}

type LibraryPathBatch = Vec<PathBuf>;
pub(crate) type LibraryPathBatchReceiver = tokio::sync::mpsc::Receiver<AppResult<LibraryPathBatch>>;

pub(crate) fn extract_library_queries(
    path: &str,
    library_root: &str,
) -> (Vec<String>, Option<u32>) {
    let root = library_root.trim_end_matches('/');

    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(stem);
    let parsed_queries = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    let mut queries = Vec::new();
    let mut seen_normalized = HashSet::new();
    let mut folder_year = None;
    let mut folder_query = None;

    for query in parsed_queries {
        push_unique_query(&mut queries, &mut seen_normalized, query);
    }

    if let Some(parent) = Path::new(path).parent() {
        let parent_str = parent.to_string_lossy();
        if parent_str.trim_end_matches('/') != root
            && let Some(folder_name) = parent.file_name().and_then(|n| n.to_str())
        {
            let clean = normalize_folder_name(folder_name);
            let (title, year) = strip_year_suffix(&clean);
            if !title.trim().is_empty() {
                folder_query = Some(title);
                folder_year = year;
            }
        }
    }

    if let Some(folder_query) = folder_query {
        push_unique_query(&mut queries, &mut seen_normalized, folder_query);
    }

    (queries, parsed.year.or(folder_year))
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

pub(crate) fn elapsed_ms_u64(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(crate) async fn list_child_directories(root: &Path) -> AppResult<Vec<PathBuf>> {
    crate::filesystem_walk::FilesystemWalker::new().list_child_directories(root)
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
        if file_type.is_dir() {
            results.push(MovieTopLevelEntry { path, is_dir: true });
            continue;
        }

        if file_type.is_file() && is_allowed_video_path(&path) {
            results.push(MovieTopLevelEntry {
                path,
                is_dir: false,
            });
        }
    }

    results.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(results)
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
        return Some(same_stem.to_string_lossy().to_string());
    }

    let parent = path.parent()?;
    let movie_nfo = parent.join("movie.nfo");
    if movie_nfo.is_file() {
        return Some(movie_nfo.to_string_lossy().to_string());
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
    {
        return Some(PathBuf::from(folder_path));
    }

    let mut ordered_paths = collections
        .iter()
        .filter_map(|collection| collection.ordered_path.as_deref())
        .map(PathBuf::from)
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
    let path_string = path.to_string_lossy().to_string();
    let now = Utc::now();
    let (scheme, value) = compute_library_probe_signature(path).await?;
    let stored_probe = app
        .services
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

            let marker = child_metadata
                .as_ref()
                .map(metadata_probe_marker)
                .unwrap_or_else(|| "unknown".to_string());
            let name = child_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
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
