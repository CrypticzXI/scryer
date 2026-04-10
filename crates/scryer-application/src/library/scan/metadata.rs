use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::library_discovery::{
    MovieTopLevelEntry, extract_library_queries, matching_movie_nfo_path_async,
    normalize_folder_name, strip_year_suffix,
};
use crate::library_scan_coordinator::LibraryScanCoordinator;
use crate::nfo::{looks_like_movie_nfo, parse_nfo};
use crate::title_matching::TitleMatchProfile;
use crate::{
    AppError, AppResult, LibraryFile, LibraryScanUnmatchedSearchAttempt, LibraryScanner,
    MetadataGateway, MetadataSearchItem, MetadataSearchQuery, parse_release_metadata,
};

pub(crate) const METADATA_TYPE_MOVIE: &str = "movie";
pub(crate) const METADATA_TYPE_SERIES: &str = "series";

const LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE: usize = 20;
const MOVIE_ENTRY_PREP_CONCURRENCY: usize = 8;
const RADARR_MOVIE_NFO_MAX_BYTES: u64 = 10 * 1024 * 1024;

#[cfg(test)]
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct MovieLibraryScanCandidate {
    pub(crate) file: LibraryFile,
    pub(crate) parsed_release: crate::ParsedReleaseMetadata,
    pub(crate) nfo_meta: Option<crate::nfo::NfoMetadata>,
    pub(crate) query: String,
    pub(crate) year_hint: Option<u32>,
    pub(crate) query_variants: Vec<String>,
    pub(crate) selected_metadata: Option<MetadataSearchItem>,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct SeriesLibraryScanCandidate {
    pub(crate) folder_path: PathBuf,
    pub(crate) folder_name: Option<String>,
    pub(crate) nfo_meta: Option<crate::nfo::NfoMetadata>,
    pub(crate) query: String,
    pub(crate) selected_metadata: Option<MetadataSearchItem>,
    pub(crate) metadata_lookup_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BatchMetadataSearchKey {
    type_hint: &'static str,
    query: String,
    year: Option<i32>,
}

impl BatchMetadataSearchKey {
    pub(crate) fn new(type_hint: &'static str, query: &str, year: Option<u32>) -> Option<Self> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return None;
        }

        Some(Self {
            type_hint,
            query: trimmed.to_string(),
            year: year.map(|value| value as i32),
        })
    }
}

type SharedMetadataSearchItems = Arc<Vec<MetadataSearchItem>>;
pub(crate) type MetadataSearchResults = HashMap<BatchMetadataSearchKey, SharedMetadataSearchItems>;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MetadataLookupBatchStats {
    pub(crate) logical_lookups: usize,
    pub(crate) executed_requests: usize,
    pub(crate) coalesced_requests: usize,
}

impl MetadataLookupBatchStats {
    fn absorb(&mut self, other: Self) {
        self.logical_lookups = self.logical_lookups.saturating_add(other.logical_lookups);
        self.executed_requests = self
            .executed_requests
            .saturating_add(other.executed_requests);
        self.coalesced_requests = self
            .coalesced_requests
            .saturating_add(other.coalesced_requests);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedMovieLibraryScanCandidate {
    pub(crate) file: LibraryFile,
    pub(crate) discovered_files: Vec<LibraryFile>,
    pub(crate) parsed_release: crate::ParsedReleaseMetadata,
    pub(crate) nfo_meta: Option<crate::nfo::NfoMetadata>,
    pub(crate) query: String,
    pub(crate) year_hint: Option<u32>,
    pub(crate) query_variants: Vec<String>,
    pub(crate) search_candidates: Vec<String>,
    pub(crate) title_match_candidates: Vec<String>,
    pub(crate) reduced_title_candidates: Vec<String>,
    pub(crate) metadata_lookup_attempted: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum PreparedMovieLibraryScanEntry {
    Candidate(PreparedMovieLibraryScanCandidate),
    Skipped { item_path: String },
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedSeriesLibraryScanCandidate {
    pub(crate) folder_path: PathBuf,
    pub(crate) folder_name: Option<String>,
    pub(crate) nfo_meta: Option<crate::nfo::NfoMetadata>,
    pub(crate) query: String,
    pub(crate) year_hint: Option<u32>,
    pub(crate) search_candidates: Vec<String>,
    pub(crate) title_match_candidates: Vec<String>,
    pub(crate) reduced_title_candidates: Vec<String>,
    pub(crate) metadata_lookup_attempted: bool,
}

pub(crate) async fn read_valid_movie_nfo_metadata(
    nfo_path: Option<&str>,
) -> Option<crate::nfo::NfoMetadata> {
    let path = Path::new(nfo_path?).to_path_buf();
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if !metadata.is_file() || metadata.len() > RADARR_MOVIE_NFO_MAX_BYTES {
        return None;
    }

    let content = tokio::fs::read_to_string(path).await.ok()?;
    if !looks_like_movie_nfo(&content) {
        return None;
    }

    Some(parse_nfo(&content))
}

async fn read_tvshow_nfo_metadata(folder: PathBuf) -> Option<crate::nfo::NfoMetadata> {
    let path = folder.join("tvshow.nfo");
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if !metadata.is_file() {
        return None;
    }
    let content = tokio::fs::read_to_string(path).await.ok()?;
    Some(parse_nfo(&content))
}

fn push_unique_batch_metadata_search(
    searches: &mut Vec<BatchMetadataSearchKey>,
    seen: &mut HashSet<BatchMetadataSearchKey>,
    type_hint: &'static str,
    query: &str,
    year: Option<u32>,
) {
    let Some(key) = BatchMetadataSearchKey::new(type_hint, query, year) else {
        return;
    };

    if seen.insert(key.clone()) {
        searches.push(key);
    }
}

fn summarize_metadata_search_item(item: &MetadataSearchItem) -> String {
    match item.year {
        Some(year) => format!("{} ({year})", item.name),
        None => item.name.clone(),
    }
}

pub(crate) fn build_library_scan_unmatched_search_attempts(
    type_hint: &'static str,
    search_candidates: &[String],
    year_hint: Option<u32>,
    batch_search_results: &MetadataSearchResults,
) -> Vec<LibraryScanUnmatchedSearchAttempt> {
    search_candidates
        .iter()
        .filter_map(|search_candidate| {
            let key = BatchMetadataSearchKey::new(type_hint, search_candidate, year_hint)?;
            let results = batch_search_results
                .get(&key)
                .map_or(&[][..], |items| items.as_slice());

            Some(LibraryScanUnmatchedSearchAttempt {
                query: search_candidate.clone(),
                result_count: results.len(),
                top_results: results
                    .iter()
                    .take(3)
                    .map(summarize_metadata_search_item)
                    .collect(),
            })
        })
        .collect()
}

pub(crate) fn library_scan_unmatched_reason_code(
    search_attempts: &[LibraryScanUnmatchedSearchAttempt],
) -> &'static str {
    if search_attempts
        .iter()
        .all(|attempt| attempt.result_count == 0)
    {
        "no_metadata_search_results"
    } else {
        "no_acceptable_metadata_match"
    }
}

async fn execute_batch_metadata_searches(
    metadata_gateway: Arc<dyn MetadataGateway>,
    search_keys: Vec<BatchMetadataSearchKey>,
) -> AppResult<MetadataSearchResults> {
    if search_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let search_queries = search_keys
        .iter()
        .map(|key| MetadataSearchQuery {
            query: key.query.clone(),
            type_hint: key.type_hint.to_string(),
            year: key.year,
        })
        .collect::<Vec<_>>();
    let batched_results = metadata_gateway.search_tvdb_batch(&search_queries).await?;

    let mut results = HashMap::new();
    for key in search_keys {
        let result_key = MetadataSearchQuery {
            query: key.query.clone(),
            type_hint: key.type_hint.to_string(),
            year: key.year,
        };
        let items = batched_results
            .get(&result_key)
            .cloned()
            .unwrap_or_default();
        results.insert(key, Arc::new(items));
    }

    Ok(results)
}

pub(crate) fn build_movie_metadata_batch_stats(
    candidates: &[PreparedMovieLibraryScanCandidate],
) -> (Vec<BatchMetadataSearchKey>, MetadataLookupBatchStats) {
    let mut stats = MetadataLookupBatchStats::default();
    let mut total_requested_searches = 0usize;
    let mut batch_searches = Vec::new();
    let mut seen_batch_searches = HashSet::new();

    for candidate in candidates {
        if !candidate.metadata_lookup_attempted {
            continue;
        }

        stats.logical_lookups = stats.logical_lookups.saturating_add(1);
        total_requested_searches =
            total_requested_searches.saturating_add(candidate.search_candidates.len());
        for search_candidate in &candidate.search_candidates {
            push_unique_batch_metadata_search(
                &mut batch_searches,
                &mut seen_batch_searches,
                METADATA_TYPE_MOVIE,
                search_candidate,
                candidate.year_hint,
            );
        }
    }

    stats.executed_requests = batch_searches.len();
    stats.coalesced_requests = total_requested_searches.saturating_sub(stats.executed_requests);
    (batch_searches, stats)
}

pub(crate) fn build_series_metadata_batch_stats(
    candidates: &[PreparedSeriesLibraryScanCandidate],
) -> (Vec<BatchMetadataSearchKey>, MetadataLookupBatchStats) {
    let mut stats = MetadataLookupBatchStats::default();
    let mut batch_searches = Vec::new();
    let mut seen_batch_searches = HashSet::new();

    for candidate in candidates {
        if !candidate.metadata_lookup_attempted {
            continue;
        }

        stats.logical_lookups = stats.logical_lookups.saturating_add(1);
        for search_candidate in &candidate.search_candidates {
            push_unique_batch_metadata_search(
                &mut batch_searches,
                &mut seen_batch_searches,
                METADATA_TYPE_SERIES,
                search_candidate,
                candidate.year_hint,
            );
        }
    }

    stats.executed_requests = batch_searches.len();
    stats.coalesced_requests = stats
        .logical_lookups
        .saturating_sub(stats.executed_requests);
    (batch_searches, stats)
}

pub(crate) fn movie_candidate_batch_search_keys(
    candidate: &PreparedMovieLibraryScanCandidate,
) -> AppResult<Vec<BatchMetadataSearchKey>> {
    let mut keys = Vec::with_capacity(candidate.search_candidates.len());

    for search_candidate in &candidate.search_candidates {
        keys.push(
            BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, search_candidate, candidate.year_hint)
                .ok_or_else(|| {
                    AppError::Repository(format!(
                        "movie metadata lookup key unexpectedly missing for query '{}'",
                        search_candidate
                    ))
                })?,
        );
    }

    Ok(keys)
}

pub(crate) fn series_candidate_batch_search_keys(
    candidate: &PreparedSeriesLibraryScanCandidate,
) -> AppResult<Vec<BatchMetadataSearchKey>> {
    if !candidate.metadata_lookup_attempted {
        return Ok(Vec::new());
    }

    candidate
        .search_candidates
        .iter()
        .map(|search_candidate| {
            BatchMetadataSearchKey::new(METADATA_TYPE_SERIES, search_candidate, candidate.year_hint)
                .ok_or_else(|| {
                    AppError::Repository(format!(
                        "series metadata lookup key unexpectedly missing for query '{}'",
                        search_candidate
                    ))
                })
        })
        .collect()
}

pub(crate) fn build_title_match_candidates(
    queries: &[String],
    profile: TitleMatchProfile,
) -> (Vec<String>, Vec<String>) {
    let mut title_match_candidates = Vec::new();
    let mut title_match_seen = HashSet::new();
    let mut reduced_title_candidates = Vec::new();
    let mut reduced_title_seen = HashSet::new();

    for query in queries {
        let title_match_key = crate::title_matching::canonical_lookup_key(query);
        if !title_match_key.is_empty() && title_match_seen.insert(title_match_key.clone()) {
            title_match_candidates.push(title_match_key);
        }

        let reduced_key = crate::title_matching::reduced_comparison_key(query, profile);
        if crate::title_matching::has_usable_reduced_key(&reduced_key)
            && reduced_title_seen.insert(reduced_key.clone())
        {
            reduced_title_candidates.push(reduced_key);
        }
    }

    (title_match_candidates, reduced_title_candidates)
}

fn expand_search_candidates(queries: &[String]) -> Vec<String> {
    let mut search_candidates = Vec::new();
    let mut seen = HashSet::new();

    for query in queries {
        for variant in crate::title_matching::search_variants(query) {
            if variant.trim().is_empty() || !seen.insert(variant.clone()) {
                continue;
            }
            search_candidates.push(variant);
        }
    }

    search_candidates
}

pub(crate) fn split_ready_metadata_candidates<T, F>(
    candidates: Vec<T>,
    search_results: &MetadataSearchResults,
    mut candidate_keys: F,
) -> AppResult<(Vec<T>, Vec<T>)>
where
    F: FnMut(&T) -> AppResult<Vec<BatchMetadataSearchKey>>,
{
    let mut ready = Vec::new();
    let mut pending = Vec::new();

    for candidate in candidates {
        let keys = candidate_keys(&candidate)?;
        if keys.iter().all(|key| search_results.contains_key(key)) {
            ready.push(candidate);
        } else {
            pending.push(candidate);
        }
    }

    Ok((ready, pending))
}

pub(crate) fn next_metadata_search_chunk<T, F>(
    candidates: &[T],
    search_results: &MetadataSearchResults,
    max_keys: usize,
    mut candidate_keys: F,
) -> AppResult<Vec<BatchMetadataSearchKey>>
where
    F: FnMut(&T) -> AppResult<Vec<BatchMetadataSearchKey>>,
{
    let mut chunk = Vec::new();
    let mut seen = HashSet::new();

    for candidate in candidates {
        let mut missing_keys = Vec::new();
        for key in candidate_keys(candidate)? {
            if search_results.contains_key(&key) || !seen.insert(key.clone()) {
                continue;
            }
            missing_keys.push(key);
        }

        if missing_keys.is_empty() {
            continue;
        }

        if !chunk.is_empty() && chunk.len().saturating_add(missing_keys.len()) > max_keys {
            break;
        }

        chunk.extend(missing_keys);
        if chunk.len() >= max_keys {
            break;
        }
    }

    Ok(chunk)
}

fn count_candidates_with_metadata_lookup<T, F>(
    candidates: &[T],
    mut candidate_keys: F,
) -> AppResult<usize>
where
    F: FnMut(&T) -> AppResult<Vec<BatchMetadataSearchKey>>,
{
    let mut count = 0usize;

    for candidate in candidates {
        if !candidate_keys(candidate)?.is_empty() {
            count = count.saturating_add(1);
        }
    }

    Ok(count)
}

pub(crate) async fn resolve_full_scan_metadata_batches<T, BuildStats, CandidateKeys>(
    metadata_gateway: Arc<dyn MetadataGateway>,
    coordinator: &LibraryScanCoordinator,
    unresolved_candidates: Vec<T>,
    metadata_lookup_stats: &mut MetadataLookupBatchStats,
    build_stats: BuildStats,
    candidate_keys: CandidateKeys,
    empty_chunk_message: &'static str,
) -> AppResult<(Vec<Vec<T>>, MetadataSearchResults)>
where
    BuildStats: Fn(&[T]) -> (Vec<BatchMetadataSearchKey>, MetadataLookupBatchStats),
    CandidateKeys: Fn(&T) -> AppResult<Vec<BatchMetadataSearchKey>> + Copy,
{
    let (_searches, batch_lookup_stats) = build_stats(&unresolved_candidates);
    metadata_lookup_stats.absorb(batch_lookup_stats);

    if unresolved_candidates.is_empty() {
        return Ok((Vec::new(), MetadataSearchResults::new()));
    }

    if batch_lookup_stats.logical_lookups > 0 {
        coordinator
            .add_metadata_total(batch_lookup_stats.logical_lookups)
            .await;
        coordinator.mark_metadata_total_known().await;
    }
    coordinator.publish_progress().await;

    let mut pending_candidates = unresolved_candidates;
    let mut ready_batches = Vec::new();
    let mut batch_search_results = MetadataSearchResults::new();

    while !pending_candidates.is_empty() {
        let (ready_candidates, still_pending) = split_ready_metadata_candidates(
            pending_candidates,
            &batch_search_results,
            candidate_keys,
        )?;
        pending_candidates = still_pending;

        if !ready_candidates.is_empty() {
            let ready_lookup_count =
                count_candidates_with_metadata_lookup(&ready_candidates, candidate_keys)?;
            if ready_lookup_count > 0 {
                coordinator
                    .mark_metadata_completed(ready_lookup_count)
                    .await;
                coordinator.publish_progress().await;
            }
            ready_batches.push(ready_candidates);
            continue;
        }

        let search_chunk = next_metadata_search_chunk(
            &pending_candidates,
            &batch_search_results,
            LIBRARY_SCAN_METADATA_SEARCH_BATCH_SIZE,
            candidate_keys,
        )?;
        if search_chunk.is_empty() {
            return Err(AppError::Repository(empty_chunk_message.into()));
        }

        batch_search_results
            .extend(execute_batch_metadata_searches(metadata_gateway.clone(), search_chunk).await?);
    }

    Ok((ready_batches, batch_search_results))
}

#[cfg(test)]
pub(crate) async fn prepare_movie_library_scan_candidates(
    files: &[LibraryFile],
    library_path: &str,
) -> AppResult<Vec<PreparedMovieLibraryScanCandidate>> {
    let mut prepare_set = tokio::task::JoinSet::new();

    for (index, file) in files.iter().cloned().enumerate() {
        let library_path = library_path.to_string();
        prepare_set.spawn(async move {
            Ok::<_, AppError>((
                index,
                prepare_movie_library_scan_candidate(file, library_path).await?,
            ))
        });
    }

    let mut prepared_results = vec![None; prepare_set.len()];
    while let Some(result) = prepare_set.join_next().await {
        let (index, candidate) =
            result.map_err(|error| AppError::Repository(error.to_string()))??;
        prepared_results[index] = Some(candidate);
    }

    Ok(prepared_results.into_iter().flatten().collect())
}

pub(crate) async fn prepare_series_library_scan_candidates(
    folders: &[PathBuf],
) -> AppResult<Vec<PreparedSeriesLibraryScanCandidate>> {
    let mut prepare_set = tokio::task::JoinSet::new();

    for (index, folder) in folders.iter().cloned().enumerate() {
        prepare_set.spawn(async move {
            Ok::<_, AppError>((index, prepare_series_library_scan_candidate(folder).await?))
        });
    }

    let mut prepared_results = vec![None; prepare_set.len()];
    while let Some(result) = prepare_set.join_next().await {
        let (index, candidate) =
            result.map_err(|error| AppError::Repository(error.to_string()))??;
        prepared_results[index] = Some(candidate);
    }

    Ok(prepared_results.into_iter().flatten().collect())
}

pub(crate) fn select_movie_metadata_from_batch_results(
    candidate: &PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
) -> AppResult<Option<MetadataSearchItem>> {
    if !candidate.metadata_lookup_attempted {
        return Ok(None);
    }

    for search_candidate in &candidate.search_candidates {
        let key =
            BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, search_candidate, candidate.year_hint)
                .ok_or_else(|| {
                    AppError::Repository(
                        "movie metadata lookup key unexpectedly missing".to_string(),
                    )
                })?;
        let results_for_query = batch_search_results.get(&key).ok_or_else(|| {
            AppError::Repository(format!(
                "movie metadata lookup result missing for query '{}'",
                search_candidate
            ))
        })?;

        if let Some(best) = select_best_match(
            results_for_query.as_ref(),
            candidate.year_hint,
            &candidate.title_match_candidates,
            &candidate.reduced_title_candidates,
            TitleMatchProfile::Movie,
        ) {
            return Ok(Some(best));
        }
    }

    Ok(None)
}

pub(crate) fn select_series_metadata_from_batch_results(
    candidate: &PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
) -> AppResult<Option<MetadataSearchItem>> {
    if !candidate.metadata_lookup_attempted {
        return Ok(None);
    }

    for search_candidate in &candidate.search_candidates {
        let key = BatchMetadataSearchKey::new(
            METADATA_TYPE_SERIES,
            search_candidate,
            candidate.year_hint,
        )
        .ok_or_else(|| {
            AppError::Repository("series metadata lookup key unexpectedly missing".to_string())
        })?;
        let results_for_query = batch_search_results.get(&key).ok_or_else(|| {
            AppError::Repository(format!(
                "series metadata lookup result missing for query '{}'",
                search_candidate
            ))
        })?;

        if let Some(best) = select_best_match(
            results_for_query.as_ref(),
            candidate.year_hint,
            &candidate.title_match_candidates,
            &candidate.reduced_title_candidates,
            TitleMatchProfile::Series,
        ) {
            return Ok(Some(best));
        }
    }

    Ok(None)
}

#[cfg(test)]
async fn prepare_movie_library_scan_candidate(
    file: LibraryFile,
    library_path: String,
) -> AppResult<PreparedMovieLibraryScanCandidate> {
    build_prepared_movie_library_scan_candidate(file.clone(), vec![file], library_path).await
}

pub(crate) async fn prepare_movie_library_scan_entries(
    library_scanner: Arc<dyn LibraryScanner>,
    entries: &[MovieTopLevelEntry],
    library_path: &str,
) -> AppResult<Vec<PreparedMovieLibraryScanEntry>> {
    let mut prepared_results = vec![None; entries.len()];

    for (chunk_index, entry_chunk) in entries.chunks(MOVIE_ENTRY_PREP_CONCURRENCY).enumerate() {
        let mut prepare_set = tokio::task::JoinSet::new();
        let chunk_start = chunk_index * MOVIE_ENTRY_PREP_CONCURRENCY;

        for (offset, entry) in entry_chunk.iter().cloned().enumerate() {
            let index = chunk_start + offset;
            let library_path = library_path.to_string();
            let library_scanner = library_scanner.clone();
            prepare_set.spawn(async move {
                Ok::<_, AppError>((
                    index,
                    prepare_movie_library_scan_entry(library_scanner, entry, library_path).await?,
                ))
            });
        }

        while let Some(result) = prepare_set.join_next().await {
            let (index, candidate) =
                result.map_err(|error| AppError::Repository(error.to_string()))??;
            prepared_results[index] = Some(candidate);
        }
    }

    Ok(prepared_results.into_iter().flatten().collect())
}

async fn prepare_movie_library_scan_entry(
    library_scanner: Arc<dyn LibraryScanner>,
    entry: MovieTopLevelEntry,
    library_path: String,
) -> AppResult<PreparedMovieLibraryScanEntry> {
    let entry_path = entry.path.to_string_lossy().to_string();
    let mut discovered_files = if entry.is_dir {
        library_scanner
            .scan_library(entry.path.to_string_lossy().as_ref())
            .await?
    } else {
        vec![LibraryFile {
            path: entry_path.clone(),
            display_name: entry
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }]
    };

    if discovered_files.is_empty() {
        return Ok(PreparedMovieLibraryScanEntry::Skipped {
            item_path: entry_path,
        });
    }

    discovered_files.sort_by(|left, right| left.path.cmp(&right.path));
    let file = build_movie_entry_representative_file(&entry, &discovered_files).await?;

    Ok(PreparedMovieLibraryScanEntry::Candidate(
        build_prepared_movie_library_scan_candidate(file, discovered_files, library_path).await?,
    ))
}

async fn build_movie_entry_representative_file(
    entry: &MovieTopLevelEntry,
    discovered_files: &[LibraryFile],
) -> AppResult<LibraryFile> {
    if !entry.is_dir {
        let mut file = discovered_files
            .first()
            .cloned()
            .ok_or_else(|| AppError::Repository("movie entry unexpectedly had no files".into()))?;
        file.nfo_path = matching_movie_nfo_path_async(Path::new(&file.path)).await;
        return Ok(file);
    }

    let primary_candidate = detect_primary_movie_entry_file(&entry.path, discovered_files).await?;
    let mut file = if let Some(primary_path) = primary_candidate.as_ref() {
        discovered_files
            .iter()
            .find(|candidate| &candidate.path == primary_path)
            .cloned()
            .unwrap_or_else(|| discovered_files[0].clone())
    } else {
        discovered_files[0].clone()
    };

    file.nfo_path =
        directory_movie_nfo_path(&entry.path, &file.path, primary_candidate.as_deref()).await;
    Ok(file)
}

async fn same_stem_movie_nfo_path(path: &Path) -> Option<String> {
    let same_stem = path.with_extension("nfo");
    if tokio::fs::metadata(&same_stem)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Some(same_stem.to_string_lossy().to_string());
    }

    None
}

async fn directory_movie_nfo_path(
    entry_path: &Path,
    file_path: &str,
    primary_candidate: Option<&str>,
) -> Option<String> {
    if let Some(nfo_path) = same_stem_movie_nfo_path(Path::new(file_path)).await {
        return Some(nfo_path);
    }

    if primary_candidate == Some(file_path) {
        let movie_nfo = entry_path.join("movie.nfo");
        if tokio::fs::metadata(&movie_nfo)
            .await
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return Some(movie_nfo.to_string_lossy().to_string());
        }
    }

    None
}

async fn detect_primary_movie_entry_file(
    entry_path: &Path,
    discovered_files: &[LibraryFile],
) -> AppResult<Option<String>> {
    let immediate_files = discovered_files
        .iter()
        .filter(|file| Path::new(&file.path).parent() == Some(entry_path))
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();

    if immediate_files.len() == 1 {
        let path = immediate_files[0].clone();
        return Ok((!is_sample_video_candidate(Path::new(&path))).then_some(path));
    }

    if immediate_files.is_empty() {
        return Ok(None);
    }

    let mut non_sample_videos = Vec::new();
    for path in immediate_files {
        if is_sample_video_candidate(Path::new(&path)) {
            continue;
        }
        non_sample_videos.push(path);
    }

    Ok((non_sample_videos.len() == 1).then(|| non_sample_videos[0].clone()))
}

fn is_sample_video_candidate(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    stem.contains("sample")
}

async fn build_prepared_movie_library_scan_candidate(
    file: LibraryFile,
    discovered_files: Vec<LibraryFile>,
    library_path: String,
) -> AppResult<PreparedMovieLibraryScanCandidate> {
    let parsed_release = parse_release_metadata(
        Path::new(&file.path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(file.display_name.as_str()),
    );

    let nfo_meta = read_valid_movie_nfo_metadata(file.nfo_path.as_deref()).await;
    let (query_variants, extracted_year_hint) = extract_library_queries(&file.path, &library_path);
    let fallback_query = query_variants.first().cloned().unwrap_or_default();

    let (query, year_hint) = if let Some(ref meta) = nfo_meta {
        let title = meta.title.clone().unwrap_or_else(|| fallback_query.clone());
        let year = meta.year.map(|value| value as u32).or(extracted_year_hint);
        (title, year)
    } else {
        (fallback_query, extracted_year_hint)
    };

    let mut search_candidates = Vec::new();
    let mut title_match_candidates = Vec::new();
    let mut reduced_title_candidates = Vec::new();
    let metadata_lookup_attempted = nfo_meta
        .as_ref()
        .and_then(|meta| meta.tvdb_id.as_deref())
        .is_none()
        && !query.trim().is_empty();

    if metadata_lookup_attempted {
        let raw_queries = query_variants
            .iter()
            .cloned()
            .chain(std::iter::once(query.clone()))
            .collect::<Vec<_>>();
        search_candidates = expand_search_candidates(&raw_queries);
        (title_match_candidates, reduced_title_candidates) =
            build_title_match_candidates(&raw_queries, TitleMatchProfile::Movie);
    }

    Ok(PreparedMovieLibraryScanCandidate {
        file,
        discovered_files,
        parsed_release,
        nfo_meta,
        query,
        year_hint,
        query_variants,
        search_candidates,
        title_match_candidates,
        reduced_title_candidates,
        metadata_lookup_attempted,
    })
}

async fn prepare_series_library_scan_candidate(
    folder: PathBuf,
) -> AppResult<PreparedSeriesLibraryScanCandidate> {
    let folder_name = folder
        .file_name()
        .and_then(|name| name.to_str())
        .map(std::string::ToString::to_string);

    let Some(folder_name_value) = folder_name.clone() else {
        return Ok(PreparedSeriesLibraryScanCandidate {
            folder_path: folder,
            folder_name: None,
            nfo_meta: None,
            query: String::new(),
            year_hint: None,
            search_candidates: Vec::new(),
            title_match_candidates: Vec::new(),
            reduced_title_candidates: Vec::new(),
            metadata_lookup_attempted: false,
        });
    };

    let nfo_meta = read_tvshow_nfo_metadata(folder.clone()).await;
    let clean_name = normalize_folder_name(&folder_name_value);
    let (fallback_query, extracted_year_hint) = strip_year_suffix(&clean_name);
    let fallback_query = fallback_query.trim().to_string();
    let (query, year_hint) = if let Some(ref meta) = nfo_meta {
        let title = meta.title.clone().unwrap_or_else(|| fallback_query.clone());
        let year = meta.year.map(|value| value as u32).or(extracted_year_hint);
        (title.trim().to_string(), year)
    } else {
        (fallback_query, extracted_year_hint)
    };

    let metadata_lookup_attempted = nfo_meta
        .as_ref()
        .and_then(|meta| meta.tvdb_id.as_deref())
        .is_none()
        && !query.is_empty();
    let (search_candidates, title_match_candidates, reduced_title_candidates) =
        if metadata_lookup_attempted {
            let raw_queries = vec![query.clone()];
            let search_candidates = expand_search_candidates(&raw_queries);
            let (title_match_candidates, reduced_title_candidates) =
                build_title_match_candidates(&raw_queries, TitleMatchProfile::Series);
            (
                search_candidates,
                title_match_candidates,
                reduced_title_candidates,
            )
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };

    Ok(PreparedSeriesLibraryScanCandidate {
        folder_path: folder,
        folder_name,
        nfo_meta,
        query,
        year_hint,
        search_candidates,
        title_match_candidates,
        reduced_title_candidates,
        metadata_lookup_attempted,
    })
}

#[cfg(test)]
pub(crate) async fn preload_movie_library_scan_candidates(
    metadata_gateway: Arc<dyn MetadataGateway>,
    files: &[LibraryFile],
    library_path: &str,
) -> AppResult<(Vec<MovieLibraryScanCandidate>, MetadataLookupBatchStats)> {
    let prepared_candidates = prepare_movie_library_scan_candidates(files, library_path).await?;
    let (batch_searches, stats) = build_movie_metadata_batch_stats(&prepared_candidates);
    let batch_search_results =
        execute_batch_metadata_searches(metadata_gateway, batch_searches).await?;
    let mut results = Vec::with_capacity(prepared_candidates.len());

    for candidate in prepared_candidates {
        let selected_metadata =
            select_movie_metadata_from_batch_results(&candidate, &batch_search_results)?;

        results.push(MovieLibraryScanCandidate {
            file: candidate.file,
            parsed_release: candidate.parsed_release,
            nfo_meta: candidate.nfo_meta,
            query: candidate.query,
            year_hint: candidate.year_hint,
            query_variants: candidate.query_variants,
            selected_metadata,
        });
    }

    Ok((results, stats))
}

#[cfg(test)]
pub(crate) async fn preload_series_library_scan_candidates(
    metadata_gateway: Arc<dyn MetadataGateway>,
    folders: &[PathBuf],
) -> AppResult<(Vec<SeriesLibraryScanCandidate>, MetadataLookupBatchStats)> {
    let prepared_candidates = prepare_series_library_scan_candidates(folders).await?;
    let (batch_searches, stats) = build_series_metadata_batch_stats(&prepared_candidates);
    let batch_search_results =
        execute_batch_metadata_searches(metadata_gateway, batch_searches).await;
    let batch_search_error = batch_search_results.as_ref().err().map(ToString::to_string);
    let batch_search_results = batch_search_results.unwrap_or_default();
    let mut results = Vec::with_capacity(prepared_candidates.len());

    for candidate in prepared_candidates {
        let (selected_metadata, metadata_lookup_error) = if candidate.metadata_lookup_attempted {
            if let Some(error) = batch_search_error.as_ref() {
                (None, Some(error.clone()))
            } else {
                (
                    select_series_metadata_from_batch_results(&candidate, &batch_search_results)?,
                    None,
                )
            }
        } else {
            (None, None)
        };

        results.push(SeriesLibraryScanCandidate {
            folder_path: candidate.folder_path,
            folder_name: candidate.folder_name,
            nfo_meta: candidate.nfo_meta,
            query: candidate.query,
            selected_metadata,
            metadata_lookup_error,
        });
    }

    Ok((results, stats))
}

pub(crate) fn select_best_match(
    results: &[MetadataSearchItem],
    year: Option<u32>,
    title_match_candidates: &[String],
    reduced_title_candidates: &[String],
    profile: TitleMatchProfile,
) -> Option<MetadataSearchItem> {
    if results.is_empty() {
        return None;
    }

    let mut canonical_matches = Vec::new();
    let mut reduced_matches = Vec::new();

    for item in results {
        let canonical_key = crate::title_matching::canonical_lookup_key(&item.name);
        if !canonical_key.is_empty()
            && title_match_candidates
                .iter()
                .any(|candidate| candidate == &canonical_key)
        {
            canonical_matches.push(item);
            continue;
        }

        if year.is_some() && !reduced_title_candidates.is_empty() {
            let reduced_key = crate::title_matching::reduced_comparison_key(&item.name, profile);
            if crate::title_matching::has_usable_reduced_key(&reduced_key)
                && reduced_title_candidates
                    .iter()
                    .any(|candidate| candidate == &reduced_key)
            {
                reduced_matches.push(item);
            }
        }
    }

    if let Some(year) = year.map(|value| value as i32)
        && let Some(match_item) = canonical_matches
            .iter()
            .find(|item| item.year == Some(year))
    {
        return Some((*match_item).clone());
    }

    if let Some(match_item) = canonical_matches.into_iter().next() {
        return Some(match_item.clone());
    }

    let match_year = year.map(|value| value as i32)?;
    reduced_matches
        .into_iter()
        .find(|item| item.year == Some(match_year))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnibridgeSourceMapping, BulkMetadataResult, MovieMetadata, MultiMetadataSearchResult,
        RichMetadataSearchItem, SeriesMetadata,
    };
    use async_trait::async_trait;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    type CountingSearchResults =
        Arc<Mutex<HashMap<(String, String), Result<Vec<MetadataSearchItem>, String>>>>;

    #[derive(Clone, Default)]
    struct CountingMetadataGateway {
        search_results: CountingSearchResults,
        search_calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl CountingMetadataGateway {
        fn search_key(type_hint: &str, query: &str) -> (String, String) {
            (type_hint.to_string(), query.trim().to_ascii_uppercase())
        }

        fn set_search_results(
            &self,
            type_hint: &str,
            query: &str,
            results: Vec<MetadataSearchItem>,
        ) {
            self.search_results
                .lock()
                .unwrap()
                .insert(Self::search_key(type_hint, query), Ok(results));
        }

        fn set_search_error(&self, type_hint: &str, query: &str, message: &str) {
            self.search_results
                .lock()
                .unwrap()
                .insert(Self::search_key(type_hint, query), Err(message.to_string()));
        }

        fn search_call_count(&self, type_hint: &str, query: &str) -> usize {
            let normalized_key = Self::search_key(type_hint, query);
            self.search_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|logged_key| *logged_key == &normalized_key)
                .count()
        }
    }

    #[async_trait]
    impl MetadataGateway for CountingMetadataGateway {
        async fn search_tvdb(
            &self,
            query: &str,
            type_hint: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<MetadataSearchItem>> {
            let key = Self::search_key(type_hint, query);
            self.search_calls.lock().unwrap().push(key.clone());
            match self
                .search_results
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| Ok(Vec::new()))
            {
                Ok(results) => Ok(results),
                Err(message) => Err(AppError::Repository(message)),
            }
        }

        async fn search_tvdb_batch(
            &self,
            queries: &[MetadataSearchQuery],
        ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
            let mut results = HashMap::new();

            for query in queries {
                let key = Self::search_key(&query.type_hint, &query.query);
                self.search_calls.lock().unwrap().push(key.clone());
                let value = match self
                    .search_results
                    .lock()
                    .unwrap()
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| Ok(Vec::new()))
                {
                    Ok(items) => items,
                    Err(message) => return Err(AppError::Repository(message)),
                };
                results.insert(query.clone(), value);
            }

            Ok(results)
        }

        async fn search_tvdb_rich(
            &self,
            _query: &str,
            _type_hint: &str,
            _limit: i32,
            _language: &str,
        ) -> AppResult<Vec<RichMetadataSearchItem>> {
            panic!("unused in test")
        }

        async fn search_tvdb_multi(
            &self,
            _query: &str,
            _limit: i32,
            _language: &str,
        ) -> AppResult<MultiMetadataSearchResult> {
            panic!("unused in test")
        }

        async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
            panic!("unused in test")
        }

        async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
            panic!("unused in test")
        }

        async fn get_metadata_bulk(
            &self,
            _movie_tvdb_ids: &[i64],
            _series_tvdb_ids: &[i64],
            _language: &str,
        ) -> AppResult<BulkMetadataResult> {
            panic!("unused in test")
        }

        async fn anibridge_mappings_for_episode(
            &self,
            _tvdb_id: i64,
            _season: i32,
            _episode: i32,
        ) -> AppResult<Vec<AnibridgeSourceMapping>> {
            panic!("unused in test")
        }
    }

    fn build_library_file(path: &str) -> LibraryFile {
        LibraryFile {
            path: path.to_string(),
            display_name: Path::new(path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }
    }

    fn build_prepared_movie_candidate(
        search_candidates: &[&str],
    ) -> PreparedMovieLibraryScanCandidate {
        PreparedMovieLibraryScanCandidate {
            file: build_library_file("/library/Movie/Movie.mkv"),
            discovered_files: vec![build_library_file("/library/Movie/Movie.mkv")],
            parsed_release: crate::ParsedReleaseMetadata::default(),
            nfo_meta: None,
            query: search_candidates
                .first()
                .copied()
                .unwrap_or_default()
                .to_string(),
            year_hint: None,
            query_variants: search_candidates
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            search_candidates: search_candidates
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            title_match_candidates: vec![],
            reduced_title_candidates: vec![],
            metadata_lookup_attempted: !search_candidates.is_empty(),
        }
    }

    #[test]
    fn extract_library_queries_uses_movie_title_variants_for_root_files() {
        let (queries, year) = extract_library_queries(
            "/library/Mon.Cousin.A.K.A.My.Cousin.2020.1080p.BluRay.mkv",
            "/library",
        );

        assert_eq!(year, Some(2020));
        assert_eq!(
            queries,
            vec![
                "MON COUSIN AKA MY COUSIN".to_string(),
                "MON COUSIN".to_string(),
                "MY COUSIN".to_string()
            ]
        );
    }

    #[test]
    fn extract_library_queries_uses_parent_folder_when_filename_is_placeholder() {
        let (queries, year) =
            extract_library_queries("/library/My Cousin (2020)/movie.mkv", "/library");

        assert_eq!(queries, vec!["MOVIE".to_string(), "My Cousin".to_string()]);
        assert_eq!(year, Some(2020));
    }

    #[test]
    fn extract_library_queries_prefers_release_year_over_stale_folder_year() {
        let (queries, year) = extract_library_queries(
            "/library/Dune (2020)/Dune.2021.2160p.BluRay.REMUX.HEVC.DTS-HD.MA.TrueHD.7.1.Atmos-FGT.mkv",
            "/library",
        );

        assert_eq!(queries, vec!["DUNE MA".to_string(), "Dune".to_string()]);
        assert_eq!(year, Some(2021));
    }

    #[test]
    fn extract_library_queries_prefers_filename_over_parent_folder_for_nested_movie() {
        let (queries, year) = extract_library_queries(
            "/library/Dune (2020)/Dune.Part.Two.2024.2160p.WEB-DL.H265-GRP.mkv",
            "/library",
        );

        assert_eq!(queries, vec!["DUNE TWO".to_string(), "Dune".to_string()]);
        assert_eq!(year, Some(2024));
    }

    #[test]
    fn next_metadata_search_chunk_limits_movie_batch_keys() {
        let candidates = vec![
            build_prepared_movie_candidate(&["Alpha", "Beta"]),
            build_prepared_movie_candidate(&["Gamma"]),
        ];

        let chunk = next_metadata_search_chunk(
            &candidates,
            &HashMap::new(),
            2,
            movie_candidate_batch_search_keys,
        )
        .expect("next metadata search chunk");

        assert_eq!(
            chunk,
            vec![
                BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Alpha", None).expect("alpha key"),
                BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Beta", None).expect("beta key"),
            ]
        );
    }

    #[test]
    fn split_ready_metadata_candidates_waits_for_all_movie_search_results() {
        let ready_candidate = build_prepared_movie_candidate(&["Alpha", "Beta"]);
        let pending_candidate = build_prepared_movie_candidate(&["Gamma"]);
        let mut search_results = HashMap::new();
        search_results.insert(
            BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Alpha", None).expect("alpha key"),
            Arc::new(Vec::new()),
        );
        search_results.insert(
            BatchMetadataSearchKey::new(METADATA_TYPE_MOVIE, "Beta", None).expect("beta key"),
            Arc::new(Vec::new()),
        );

        let (ready, pending) = split_ready_metadata_candidates(
            vec![ready_candidate.clone(), pending_candidate.clone()],
            &search_results,
            movie_candidate_batch_search_keys,
        )
        .expect("split ready metadata candidates");

        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].query, ready_candidate.query);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].query, pending_candidate.query);
    }

    #[tokio::test]
    async fn read_valid_movie_nfo_metadata_accepts_movie_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("movie.nfo");
        std::fs::write(
            &path,
            r#"<movie><title>Test Movie Title</title><tvdbid>123456</tvdbid></movie>"#,
        )
        .expect("write nfo");

        let metadata = read_valid_movie_nfo_metadata(Some(path.to_string_lossy().as_ref()))
            .await
            .expect("movie nfo");
        assert_eq!(metadata.title.as_deref(), Some("Test Movie Title"));
        assert_eq!(metadata.tvdb_id.as_deref(), Some("123456"));
    }

    #[tokio::test]
    async fn read_valid_movie_nfo_metadata_rejects_tvshow_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("movie.nfo");
        std::fs::write(
            &path,
            r#"<tvshow><title>Bluey</title><tvdbid>81189</tvdbid></tvshow>"#,
        )
        .expect("write nfo");

        assert!(
            read_valid_movie_nfo_metadata(Some(path.to_string_lossy().as_ref()))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn preload_movie_library_scan_candidates_coalesces_duplicate_queries() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_results(
            METADATA_TYPE_MOVIE,
            "Dune",
            vec![MetadataSearchItem {
                tvdb_id: "movie-1".into(),
                name: "Dune".into(),
                year: Some(2021),
            }],
        );

        let files = vec![
            build_library_file("/library/Dune (2021)/Dune.2021.2160p.BluRay.mkv"),
            build_library_file("/elsewhere/Dune (2021)/Dune.2021.1080p.WEB-DL.mkv"),
        ];

        let (candidates, stats) =
            preload_movie_library_scan_candidates(Arc::new(gateway.clone()), &files, "/library")
                .await
                .expect("movie preload should succeed");

        assert_eq!(gateway.search_call_count(METADATA_TYPE_MOVIE, "Dune"), 1);
        assert_eq!(stats.logical_lookups, 2);
        assert_eq!(stats.executed_requests, 1);
        assert_eq!(stats.coalesced_requests, 1);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| {
            candidate
                .selected_metadata
                .as_ref()
                .map(|item| item.tvdb_id.as_str())
                == Some("movie-1")
        }));
    }

    #[tokio::test]
    async fn preload_movie_library_scan_candidates_reuses_shared_fallback_queries() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_results(METADATA_TYPE_MOVIE, "MON COUSIN AKA MY COUSIN", vec![]);
        gateway.set_search_results(METADATA_TYPE_MOVIE, "MON COUSIN", vec![]);
        gateway.set_search_results(
            METADATA_TYPE_MOVIE,
            "MY COUSIN",
            vec![MetadataSearchItem {
                tvdb_id: "movie-2".into(),
                name: "My Cousin".into(),
                year: Some(2020),
            }],
        );

        let files = vec![
            build_library_file("/library/Mon.Cousin.A.K.A.My.Cousin.2020.1080p.BluRay.mkv"),
            build_library_file("/library/My.Cousin.2020.720p.WEB-DL.mkv"),
        ];

        let (candidates, stats) =
            preload_movie_library_scan_candidates(Arc::new(gateway.clone()), &files, "/library")
                .await
                .expect("movie preload should succeed");

        assert_eq!(
            gateway.search_call_count(METADATA_TYPE_MOVIE, "MY COUSIN"),
            1
        );
        assert_eq!(stats.logical_lookups, 2);
        assert_eq!(stats.executed_requests, 3);
        assert_eq!(stats.coalesced_requests, 1);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| {
            candidate
                .selected_metadata
                .as_ref()
                .map(|item| item.tvdb_id.as_str())
                == Some("movie-2")
        }));
    }

    #[tokio::test]
    async fn preload_movie_library_scan_candidates_preserves_error_behavior_for_shared_requests() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_error(METADATA_TYPE_MOVIE, "Dune", "rate limited");

        let files = vec![
            build_library_file("/library/Dune (2021)/Dune.2021.2160p.BluRay.mkv"),
            build_library_file("/elsewhere/Dune (2021)/Dune.2021.1080p.WEB-DL.mkv"),
        ];

        let error =
            preload_movie_library_scan_candidates(Arc::new(gateway.clone()), &files, "/library")
                .await
                .expect_err("movie preload should fail on shared request error");

        assert_eq!(gateway.search_call_count(METADATA_TYPE_MOVIE, "Dune"), 1);
        assert!(matches!(error, AppError::Repository(message) if message == "rate limited"));
    }

    #[tokio::test]
    async fn preload_series_library_scan_candidates_coalesces_duplicate_queries() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_results(
            METADATA_TYPE_SERIES,
            "Bluey",
            vec![MetadataSearchItem {
                tvdb_id: "series-1".into(),
                name: "Bluey".into(),
                year: Some(2018),
            }],
        );

        let folders = vec![
            PathBuf::from("/library-a/Bluey (2018)"),
            PathBuf::from("/library-b/Bluey (2018)"),
        ];

        let (candidates, stats) =
            preload_series_library_scan_candidates(Arc::new(gateway.clone()), &folders)
                .await
                .expect("series preload should succeed");

        assert_eq!(gateway.search_call_count(METADATA_TYPE_SERIES, "Bluey"), 1);
        assert_eq!(stats.logical_lookups, 2);
        assert_eq!(stats.executed_requests, 1);
        assert_eq!(stats.coalesced_requests, 1);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| {
            candidate
                .selected_metadata
                .as_ref()
                .map(|item| item.tvdb_id.as_str())
                == Some("series-1")
        }));
    }

    #[tokio::test]
    async fn preload_series_library_scan_candidates_preserves_error_behavior_for_shared_requests() {
        let gateway = CountingMetadataGateway::default();
        gateway.set_search_error(METADATA_TYPE_SERIES, "Bluey", "series rate limited");

        let folders = vec![
            PathBuf::from("/library-a/Bluey (2018)"),
            PathBuf::from("/library-b/Bluey (2018)"),
        ];

        let (candidates, stats) =
            preload_series_library_scan_candidates(Arc::new(gateway.clone()), &folders)
                .await
                .expect("series preload should degrade gracefully");

        assert_eq!(gateway.search_call_count(METADATA_TYPE_SERIES, "Bluey"), 1);
        assert_eq!(stats.logical_lookups, 2);
        assert_eq!(stats.executed_requests, 1);
        assert_eq!(stats.coalesced_requests, 1);
        assert!(candidates.iter().all(|candidate| {
            candidate.metadata_lookup_error.as_deref() == Some("repository: series rate limited")
                && candidate.selected_metadata.is_none()
        }));
    }

    #[test]
    fn sample_video_candidate_requires_sample_name_signal() {
        assert!(is_sample_video_candidate(Path::new(
            "/library/Movie/sample-featurette.mkv"
        )));
        assert!(!is_sample_video_candidate(Path::new(
            "/library/Movie/Short.Film.2024.mkv"
        )));
    }

    #[tokio::test]
    async fn detect_primary_movie_entry_file_keeps_small_non_sample_video() {
        let dir = tempfile::tempdir().expect("tempdir");
        let movie_dir = dir.path().join("Short Film (2024)");
        tokio::fs::create_dir_all(&movie_dir)
            .await
            .expect("movie dir");
        let movie_path = movie_dir.join("Short.Film.2024.mkv");
        tokio::fs::write(&movie_path, b"tiny-but-real")
            .await
            .expect("movie file");

        let discovered_files = vec![LibraryFile {
            path: movie_path.to_string_lossy().to_string(),
            display_name: "Short.Film.2024".to_string(),
            nfo_path: None,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }];

        let primary = detect_primary_movie_entry_file(&movie_dir, &discovered_files)
            .await
            .expect("primary");

        assert_eq!(
            primary.as_deref(),
            Some(movie_path.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn select_best_match_prefers_exact_title_and_matching_year() {
        let results = vec![
            MetadataSearchItem {
                tvdb_id: "wrong".into(),
                name: "Dune Drifter".into(),
                year: Some(2020),
            },
            MetadataSearchItem {
                tvdb_id: "right".into(),
                name: "Dune".into(),
                year: Some(2021),
            },
        ];
        let raw_candidates = vec!["Dune".to_string()];
        let (candidates, reduced) =
            build_title_match_candidates(&raw_candidates, TitleMatchProfile::Movie);

        let selected = select_best_match(
            &results,
            Some(2021),
            &candidates,
            &reduced,
            TitleMatchProfile::Movie,
        )
        .expect("exact title match");

        assert_eq!(selected.tvdb_id, "right");
        assert_eq!(selected.name, "Dune");
    }

    #[test]
    fn select_best_match_rejects_non_exact_title_even_with_year_match() {
        let results = vec![MetadataSearchItem {
            tvdb_id: "wrong".into(),
            name: "Dune Drifter".into(),
            year: Some(2020),
        }];
        let raw_candidates = vec!["Dune".to_string()];
        let (candidates, reduced) =
            build_title_match_candidates(&raw_candidates, TitleMatchProfile::Movie);

        assert!(
            select_best_match(
                &results,
                Some(2020),
                &candidates,
                &reduced,
                TitleMatchProfile::Movie,
            )
            .is_none()
        );
    }

    #[test]
    fn select_best_match_accepts_trailing_article_equivalence() {
        let results = vec![MetadataSearchItem {
            tvdb_id: "right".into(),
            name: "The DUFF".into(),
            year: Some(2015),
        }];
        let raw_candidates = vec!["DUFF, The".to_string()];
        let (candidates, reduced) =
            build_title_match_candidates(&raw_candidates, TitleMatchProfile::Movie);

        let selected = select_best_match(
            &results,
            Some(2015),
            &candidates,
            &reduced,
            TitleMatchProfile::Movie,
        )
        .expect("article-aware canonical match");

        assert_eq!(selected.tvdb_id, "right");
    }

    #[test]
    fn select_best_match_accepts_reduced_movie_boilerplate_with_year() {
        let results = vec![MetadataSearchItem {
            tvdb_id: "right".into(),
            name: "Sasaki and Miyano: Graduation".into(),
            year: Some(2023),
        }];
        let raw_candidates = vec!["Sasaki and Miyano Graduation Arc".to_string()];
        let (candidates, reduced) =
            build_title_match_candidates(&raw_candidates, TitleMatchProfile::Movie);

        let selected = select_best_match(
            &results,
            Some(2023),
            &candidates,
            &reduced,
            TitleMatchProfile::Movie,
        )
        .expect("reduced-tier match");

        assert_eq!(selected.tvdb_id, "right");
    }
}
