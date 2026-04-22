use std::path::Path;

use super::provider::{SubtitleMatch, SubtitleProvider, SubtitleQuery, compute_opensubtitles_hash};
use crate::AppResult;

/// Orchestrates subtitle searching: tries hash-based lookup first,
/// falls back to metadata-based search if no good matches found.
pub struct SubtitleSearchOrchestrator {
    min_score: i32,
}

impl SubtitleSearchOrchestrator {
    pub fn new(min_score: i32) -> Self {
        Self { min_score }
    }

    /// Search for subtitles for a media file.
    ///
    /// Strategy:
    /// 1. Compute file hash and search with it (highest confidence matches).
    /// 2. If no results above min_score, fall back to metadata search (IMDB ID, title+year).
    /// 3. Return all results sorted by score descending.
    pub async fn search(
        &self,
        provider: &dyn SubtitleProvider,
        file_path: &Path,
        query: &SubtitleQuery,
    ) -> AppResult<Vec<SubtitleMatch>> {
        // Try hash-based search first
        let file_hash = compute_opensubtitles_hash(file_path).ok();

        if file_hash.is_some() {
            let mut hash_query = query.clone();
            hash_query.file_hash = file_hash.clone();

            match provider.search(&hash_query).await {
                Ok(results) if results.iter().any(|r| r.score >= self.min_score) => {
                    return Ok(results);
                }
                Ok(results) => {
                    tracing::debug!(
                        provider = provider.name(),
                        hash = ?file_hash,
                        results = results.len(),
                        "hash search returned results below min_score, trying metadata fallback"
                    );
                    // Keep hash results, we'll merge with metadata results
                    if !results.is_empty() {
                        // If we have hash results, use them even if below threshold
                        // (the metadata search might not find anything better)
                        let metadata_results = self
                            .search_by_metadata(provider, query)
                            .await
                            .unwrap_or_default();

                        return Ok(merge_results(results, metadata_results));
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "hash-based subtitle search failed, trying metadata");
                }
            }
        }

        // Metadata-based fallback
        self.search_by_metadata(provider, query).await
    }

    async fn search_by_metadata(
        &self,
        provider: &dyn SubtitleProvider,
        query: &SubtitleQuery,
    ) -> AppResult<Vec<SubtitleMatch>> {
        let mut metadata_query = query.clone();
        metadata_query.file_hash = None;

        provider.search(&metadata_query).await
    }
}

/// Merge two result sets, deduplicating by provider_file_id, keeping higher scores.
fn merge_results(primary: Vec<SubtitleMatch>, secondary: Vec<SubtitleMatch>) -> Vec<SubtitleMatch> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();

    for r in primary {
        if seen.insert(r.provider_file_id.clone()) {
            merged.push(r);
        }
    }
    for r in secondary {
        if seen.insert(r.provider_file_id.clone()) {
            merged.push(r);
        }
    }

    merged.sort_by(|a, b| b.score.cmp(&a.score));
    merged
}
