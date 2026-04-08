use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;
use scryer_domain::MediaFacet;

use crate::library_scan_metadata::{
    METADATA_TYPE_MOVIE, METADATA_TYPE_SERIES, MetadataSearchResults,
    PreparedMovieLibraryScanCandidate, PreparedSeriesLibraryScanCandidate,
    build_library_scan_unmatched_search_attempts, library_scan_unmatched_reason_code,
};
use crate::{
    AppResult, AppUseCase, LibraryScanUnmatchedItem, LibraryScanUnmatchedSearchAttempt, sha256_hex,
};

#[derive(Clone, Debug)]
struct MovieUnmatchedScanRecord {
    path: String,
    display_name: String,
    query: String,
    year_hint: Option<u32>,
    reason: &'static str,
    search_attempts: Vec<LibraryScanUnmatchedSearchAttempt>,
}

fn normalize_library_scan_root(library_path: &str) -> String {
    Path::new(library_path).to_string_lossy().trim().to_string()
}

pub(crate) fn normalize_library_scan_item_path(path: &str) -> String {
    path.trim().to_string()
}

fn build_library_scan_unmatched_item_id(facet: &MediaFacet, item_path: &str) -> String {
    let fingerprint = sha256_hex(&format!("{}:{item_path}", facet.as_str()));
    format!("library_scan_unmatched:{}", &fingerprint[..24])
}

fn build_library_scan_unmatched_item(
    facet: &MediaFacet,
    session_id: &str,
    library_path: &str,
    item_path: String,
    display_name: String,
    query: String,
    year_hint: Option<u32>,
    reason_code: &str,
    error_message: Option<String>,
    search_attempts: Vec<LibraryScanUnmatchedSearchAttempt>,
) -> LibraryScanUnmatchedItem {
    let item_path = normalize_library_scan_item_path(&item_path);
    let timestamp = Utc::now().to_rfc3339();

    LibraryScanUnmatchedItem {
        id: build_library_scan_unmatched_item_id(facet, &item_path),
        facet: facet.clone(),
        scan_session_id: session_id.to_string(),
        scan_root: normalize_library_scan_root(library_path),
        item_path,
        display_name,
        query,
        year_hint: year_hint.map(|value| value as i32),
        reason_code: reason_code.to_string(),
        error_message,
        search_attempts,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    }
}

fn series_unmatched_display_name(candidate: &PreparedSeriesLibraryScanCandidate) -> String {
    candidate
        .folder_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            candidate
                .folder_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| candidate.folder_path.to_string_lossy().to_string())
}

fn build_movie_unmatched_scan_record(
    candidate: &PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
) -> MovieUnmatchedScanRecord {
    let search_attempts = build_library_scan_unmatched_search_attempts(
        METADATA_TYPE_MOVIE,
        &candidate.search_candidates,
        candidate.year_hint,
        batch_search_results,
    );
    let reason = library_scan_unmatched_reason_code(&search_attempts);

    MovieUnmatchedScanRecord {
        path: candidate.file.path.clone(),
        display_name: candidate.file.display_name.clone(),
        query: candidate.query.clone(),
        year_hint: candidate.year_hint,
        reason,
        search_attempts,
    }
}

pub(crate) fn build_movie_unmatched_scan_item(
    facet: &MediaFacet,
    session_id: &str,
    library_path: &str,
    candidate: &PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
) -> LibraryScanUnmatchedItem {
    let record = build_movie_unmatched_scan_record(candidate, batch_search_results);
    build_library_scan_unmatched_item(
        facet,
        session_id,
        library_path,
        record.path,
        record.display_name,
        record.query,
        record.year_hint,
        record.reason,
        None,
        record.search_attempts,
    )
}

pub(crate) fn build_series_unmatched_scan_item(
    facet: &MediaFacet,
    session_id: &str,
    library_path: &str,
    candidate: &PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    reason_override: Option<&str>,
    error_message: Option<String>,
) -> LibraryScanUnmatchedItem {
    let search_attempts = build_library_scan_unmatched_search_attempts(
        METADATA_TYPE_SERIES,
        &candidate.search_candidates,
        candidate.year_hint,
        batch_search_results,
    );
    let reason_code =
        reason_override.unwrap_or_else(|| library_scan_unmatched_reason_code(&search_attempts));

    build_library_scan_unmatched_item(
        facet,
        session_id,
        library_path,
        candidate.folder_path.to_string_lossy().to_string(),
        series_unmatched_display_name(candidate),
        candidate.query.clone(),
        candidate.year_hint,
        reason_code,
        error_message,
        search_attempts,
    )
}

pub(crate) fn format_library_scan_unmatched_search_attempts(
    attempts: &[LibraryScanUnmatchedSearchAttempt],
) -> String {
    attempts
        .iter()
        .map(|attempt| {
            let top_results = if attempt.top_results.is_empty() {
                "[]".to_string()
            } else {
                format!("[{}]", attempt.top_results.join(" | "))
            };
            format!("{}:{}:{}", attempt.query, attempt.result_count, top_results)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(crate) async fn persist_library_scan_unmatched_item(
    app: &AppUseCase,
    item: &LibraryScanUnmatchedItem,
) -> AppResult<()> {
    app.services
        .library_scan_unmatched_items
        .upsert_library_scan_unmatched_item(item)
        .await?;
    Ok(())
}

pub(crate) async fn clear_library_scan_unmatched_item(
    app: &AppUseCase,
    facet: &MediaFacet,
    item_path: &str,
) -> AppResult<()> {
    let item_path = normalize_library_scan_item_path(item_path);
    if item_path.is_empty() {
        return Ok(());
    }

    app.services
        .library_scan_unmatched_items
        .delete_library_scan_unmatched_item(facet.clone(), &item_path)
        .await
}

pub(crate) async fn reconcile_library_scan_unmatched_items(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_path: &str,
    seen_paths: &HashSet<String>,
) -> AppResult<()> {
    let scan_root = normalize_library_scan_root(library_path);
    let count = app
        .services
        .library_scan_unmatched_items
        .count_library_scan_unmatched_items(Some(facet.clone()), Some(&scan_root))
        .await?;
    if count <= 0 {
        return Ok(());
    }

    let existing = app
        .services
        .library_scan_unmatched_items
        .list_library_scan_unmatched_items(Some(facet.clone()), Some(&scan_root), count, 0)
        .await?;

    for item in existing {
        if !seen_paths.contains(&item.item_path) {
            app.services
                .library_scan_unmatched_items
                .delete_library_scan_unmatched_item(facet.clone(), &item.item_path)
                .await?;
        }
    }

    Ok(())
}
