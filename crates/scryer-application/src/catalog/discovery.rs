use super::*;
use crate::acquisition_release_search::ResolvedReleaseSearchSubject;
use crate::ports::IndexerSearchLearningContext;
use crate::settings::keys::default_indexer_routing_categories_for_scope;
use scryer_domain::{MediaFacet, TaggedAlias};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

fn release_search_tagged_aliases(title: &Title) -> Vec<TaggedAlias> {
    let mut aliases = title.tagged_aliases.clone();
    let mut seen: HashSet<String> = aliases
        .iter()
        .map(|alias| alias.name.trim().to_ascii_lowercase())
        .filter(|alias| !alias.is_empty())
        .collect();
    for alias in &title.aliases {
        let key = alias.trim().to_ascii_lowercase();
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        aliases.push(TaggedAlias {
            name: alias.clone(),
            language: "und".to_string(),
        });
    }
    aliases
}

fn merge_newznab_category_codes(
    base: impl IntoIterator<Item = String>,
    extras: &[String],
) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    for category in base.into_iter().chain(extras.iter().cloned()) {
        let category = category.trim().to_string();
        if !category.is_empty() && seen.insert(category.clone()) {
            merged.push(category);
        }
    }
    merged
}

fn merge_series_movie_categories_into_routing(
    plan: &mut IndexerRoutingPlan,
    owner_facet: &MediaFacet,
    extra_categories: &[String],
) {
    if extra_categories.is_empty() {
        return;
    }
    for entry in plan.entries.values_mut().filter(|entry| entry.enabled) {
        let base_categories = if entry.categories.is_empty() {
            default_indexer_routing_categories_for_scope(owner_facet.as_str())
        } else {
            std::mem::take(&mut entry.categories)
        };
        entry.categories = merge_newznab_category_codes(base_categories, extra_categories);
    }
}

fn source_kind_matches_preference(result: &IndexerSearchResult, preferred: &str) -> bool {
    match result.source_kind {
        Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => preferred == "nzb",
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
            preferred == "torrent"
        }
        None => false,
    }
}

#[cfg(test)]
pub(crate) fn extract_http_status_from_message(message: &str) -> Option<u16> {
    let marker = "status ";
    let lowered = message.to_ascii_lowercase();
    let marker_position = lowered.find(marker)?;
    let mut digits = String::new();

    for character in lowered[marker_position + marker.len()..].chars() {
        if character.is_ascii_digit() {
            digits.push(character);
        } else if !digits.is_empty() {
            break;
        }
    }

    digits.parse::<u16>().ok()
}

#[cfg(test)]
pub(crate) fn is_4xx_or_5xx_status(status: u16) -> bool {
    (400..=599).contains(&status)
}

fn resolve_requested_episode(
    episodes: &[Episode],
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> Option<&Episode> {
    if let (Some(season), Some(episode_number)) = (season, episode)
        && let Some(found) = episodes.iter().find(|candidate| {
            candidate
                .season_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                == Some(season)
                && candidate
                    .episode_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                    == Some(episode_number)
        })
    {
        return Some(found);
    }

    absolute_episode.and_then(|wanted_absolute| {
        episodes.iter().find(|candidate| {
            candidate
                .absolute_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                == Some(wanted_absolute)
        })
    })
}

#[cfg(test)]
fn extract_indexer_http_status(error: &AppError) -> Option<u16> {
    match error {
        AppError::Repository(message) => extract_http_status_from_message(message),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn is_indexer_http_error(error: &AppError) -> bool {
    extract_indexer_http_status(error).is_some_and(is_4xx_or_5xx_status)
}

pub(crate) fn release_search_key(result: &IndexerSearchResult) -> String {
    if let Some(download_url) = result
        .download_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return download_url.to_string();
    }

    if let Some(link) = result
        .link
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return link.to_string();
    }

    result.title.clone()
}

fn looks_like_structured_query_token(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        return false;
    }

    let upper = trimmed.to_ascii_uppercase();
    if upper == "OVA" || upper == "SPECIAL" {
        return true;
    }

    if let Some(rest) = upper.strip_prefix('S') {
        if rest.chars().all(|ch| ch.is_ascii_digit()) {
            return true;
        }
        if let Some((season_part, episode_part)) = rest.split_once('E') {
            return !season_part.is_empty()
                && !episode_part.is_empty()
                && season_part.chars().all(|ch| ch.is_ascii_digit())
                && episode_part.chars().all(|ch| ch.is_ascii_digit());
        }
    }

    false
}

fn normalize_structured_dispatch_query(query: &str, absolute_episode: Option<u32>) -> String {
    let mut tokens: Vec<&str> = query.split_whitespace().collect();
    while let Some(last) = tokens.last().copied() {
        let trimmed = last.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
        if trimmed.is_empty() {
            tokens.pop();
            continue;
        }

        let removable_numeric = absolute_episode.is_some_and(|value| {
            trimmed.chars().all(|ch| ch.is_ascii_digit())
                && trimmed.parse::<u32>().ok() == Some(value)
        });
        let removable_structured = removable_numeric || looks_like_structured_query_token(trimmed);
        if removable_structured {
            tokens.pop();
            continue;
        }
        break;
    }

    tokens.join(" ").trim().to_string()
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum StructuredDispatchQueryShape {
    AbsoluteEpisode,
    SeasonEpisode,
    Season,
    Other,
}

fn structured_dispatch_query_shape(
    query: &str,
    absolute_episode: Option<u32>,
) -> StructuredDispatchQueryShape {
    let Some(last) = query.split_whitespace().last() else {
        return StructuredDispatchQueryShape::Other;
    };
    let trimmed = last.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        return StructuredDispatchQueryShape::Other;
    }

    if absolute_episode.is_some_and(|value| {
        trimmed.chars().all(|ch| ch.is_ascii_digit()) && trimmed.parse::<u32>().ok() == Some(value)
    }) {
        return StructuredDispatchQueryShape::AbsoluteEpisode;
    }

    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with('S') && upper.contains('E') {
        return StructuredDispatchQueryShape::SeasonEpisode;
    }
    if upper.starts_with('S') || upper == "OVA" || upper == "SPECIAL" {
        return StructuredDispatchQueryShape::Season;
    }

    StructuredDispatchQueryShape::Other
}

fn dedupe_structured_dispatch_queries(
    queries: Vec<String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> Vec<String> {
    if season.is_none() && episode.is_none() && absolute_episode.is_none() {
        return queries;
    }

    let mut deduped = Vec::with_capacity(queries.len());
    let mut seen = std::collections::HashSet::new();

    for query in queries {
        let normalized = normalize_structured_dispatch_query(&query, absolute_episode);
        let key_source = if normalized.is_empty() {
            query.trim()
        } else {
            normalized.as_str()
        };
        if seen.insert(key_source.to_ascii_lowercase()) {
            deduped.push(query);
        }
    }

    deduped
}

fn dedupe_text_safe_structured_dispatch_queries(
    queries: Vec<String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> Vec<String> {
    if season.is_none() && episode.is_none() && absolute_episode.is_none() {
        return queries;
    }

    let mut deduped = Vec::with_capacity(queries.len());
    let mut seen = std::collections::HashSet::new();

    for query in queries {
        let normalized = normalize_structured_dispatch_query(&query, absolute_episode);
        let key_source = if normalized.is_empty() {
            query.trim()
        } else {
            normalized.as_str()
        };
        let key = (
            key_source.to_ascii_lowercase(),
            structured_dispatch_query_shape(&query, absolute_episode),
        );
        if seen.insert(key) {
            deduped.push(query);
        }
    }

    deduped
}

fn should_collapse_structured_nab_queries(
    configs: &[IndexerConfig],
    routing: Option<&IndexerRoutingPlan>,
    mode: SearchMode,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if configs.is_empty() {
        return false;
    }

    let mut saw_nab_transport = false;

    for config in configs {
        if !config.is_enabled {
            continue;
        }
        if config.disabled_until.is_some_and(|until| until > now) {
            continue;
        }

        let mode_ok = match mode {
            SearchMode::Interactive => config.enable_interactive_search,
            SearchMode::Auto => auto_mode_enabled_for_structured_collapse(config),
        };
        if !mode_ok {
            continue;
        }

        let routing_entry = routing.and_then(|plan| plan.entries.get(&config.id));
        if routing_entry.is_some_and(|entry| !entry.enabled) {
            continue;
        }

        match config.nab_transport_kind() {
            Some(_) => saw_nab_transport = true,
            None => return false,
        }
    }

    saw_nab_transport
}

#[derive(Debug, Default, Deserialize)]
struct ManagedIndexerAutoModeMetadata {
    enable_automatic_search: Option<bool>,
}

fn auto_mode_enabled_for_structured_collapse(config: &IndexerConfig) -> bool {
    if !config.enable_auto_search {
        return false;
    }

    let Some(raw) = config.managed_metadata_json.as_deref() else {
        return true;
    };
    let Ok(metadata) = serde_json::from_str::<ManagedIndexerAutoModeMetadata>(raw) else {
        return true;
    };

    metadata.enable_automatic_search.unwrap_or(true)
}

/// Presentation order for scored release results: **allowed, then tier, then
/// revision, then score** — the head of the search rank, shared with it
/// (`RankHead`) so the two orderings cannot drift apart.
///
/// Used by the interactive job's incremental merge, and only there: it re-sorts
/// a partial snapshot as batches arrive, and the GraphQL payload truncates to
/// the requested limit, so a comparator that disagreed with the rank would cut
/// the wrong releases. It compared allowed → score only, and with the tier no
/// longer inside the score that listed a 720p release above a 2160p one (D11).
/// The one-shot path sorts with the full [`SearchRank`]
/// (`compare_ranked_results`), so the two surfaces agree on the head of the key
/// and differ below it — a deferred follow-on, not a claim this doc should make.
///
/// [`SearchRank`]: crate::acquisition::scoring::SearchRank
///
/// The listing steps (indexer priority, seeders, age, coverage) are deliberately
/// absent: a merge sees results from several indexers arriving at different
/// times, and nothing here may depend on when a batch landed.
pub(crate) fn compare_release_search_results(
    left: &IndexerSearchResult,
    right: &IndexerSearchResult,
) -> std::cmp::Ordering {
    crate::acquisition::scoring::RankHead::compare(left, right)
}

pub(crate) fn dedupe_cross_indexer_release_results(
    results: Vec<IndexerSearchResult>,
    indexer_priority_by_name: &HashMap<String, i64>,
    preferred_source_kind: &str,
) -> Vec<IndexerSearchResult> {
    let mut best_by_key: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut remove_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (idx, result) in results.iter().enumerate() {
        let key = result
            .parsed_release_metadata
            .as_ref()
            .map(crate::release_dedup::build_release_dedup_key)
            .unwrap_or_default();
        if key.is_empty() {
            continue;
        }

        if let Some(&existing_idx) = best_by_key.get(&key) {
            let existing = &results[existing_idx];

            let existing_prio = indexer_priority_by_name
                .get(&existing.source)
                .copied()
                .unwrap_or(i64::MAX);
            let new_prio = indexer_priority_by_name
                .get(&result.source)
                .copied()
                .unwrap_or(i64::MAX);

            let new_wins = if new_prio != existing_prio {
                new_prio < existing_prio
            } else {
                let existing_preferred =
                    source_kind_matches_preference(existing, preferred_source_kind);
                let new_preferred = source_kind_matches_preference(result, preferred_source_kind);
                new_preferred && !existing_preferred
            };

            if new_wins {
                remove_indices.insert(existing_idx);
                best_by_key.insert(key, idx);
            } else {
                remove_indices.insert(idx);
            }
        } else {
            best_by_key.insert(key, idx);
        }
    }

    if remove_indices.is_empty() {
        return results;
    }

    let before = results.len();
    let mut idx = 0usize;
    let mut deduped = results;
    deduped.retain(|_| {
        let keep = !remove_indices.contains(&idx);
        idx += 1;
        keep
    });
    debug!(before, after = deduped.len(), "cross-indexer release dedup");
    deduped
}

impl AppUseCase {
    pub(crate) async fn download_source_capabilities(&self) -> (bool, bool, String) {
        let clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await
            .unwrap_or_default();
        let enabled: Vec<_> = clients.iter().filter(|c| c.is_enabled).collect();
        let plugin_provider = self
            .services
            .integrations
            .download_client_plugin_provider
            .available();
        let client_accepts = |c: &&scryer_domain::DownloadClientConfig,
                              kind: DownloadSourceKind| {
            let inputs = crate::accepted_inputs_for_client(&c.client_type, plugin_provider);
            inputs.contains(&kind)
        };
        let has_usenet = enabled
            .iter()
            .any(|c| client_accepts(c, DownloadSourceKind::NzbFile));
        let has_torrent = enabled.iter().any(|c| {
            client_accepts(c, DownloadSourceKind::TorrentFile)
                || client_accepts(c, DownloadSourceKind::MagnetUri)
        });
        let preferred = enabled
            .iter()
            .min_by_key(|c| c.client_priority)
            .map(|c| {
                if client_accepts(c, DownloadSourceKind::NzbFile) {
                    "nzb"
                } else {
                    "torrent"
                }
            })
            .unwrap_or("nzb")
            .to_string();

        (has_usenet, has_torrent, preferred)
    }

    pub(crate) async fn build_indexer_priority_by_name(
        &self,
        indexer_routing: Option<&IndexerRoutingPlan>,
    ) -> HashMap<String, i64> {
        let Some(plan) = indexer_routing else {
            return HashMap::new();
        };

        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_default();
        let id_to_name: std::collections::HashMap<&str, &str> = configs
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect();
        plan.entries
            .iter()
            .filter_map(|(id, entry)| {
                id_to_name
                    .get(id.as_str())
                    .map(|name| (name.to_string(), entry.priority))
            })
            .collect()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "release scoring needs the full search context to produce deterministic ranking decisions"
    )]
    pub(crate) async fn score_release_results(
        &self,
        mut raw_results: Vec<IndexerSearchResult>,
        quality_profile: &QualityProfile,
        title_id: &str,
        // The library and settings scope used to be resolved here; the canonical
        // scoring context resolves them from the title, the same way import
        // does, so passing them separately could only make the two disagree.
        indexer_routing: Option<&IndexerRoutingPlan>,
        // The search `category` and the title's tags used to be read here for
        // audio-language inference; that now lives in
        // `canonical_context::announced_metadata_for_title`, keyed on the
        // title's own facet and tags so every lane infers identically.
        runtime_minutes: Option<i32>,
        parse_context: &ReleaseParseContext,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        // Search-time exclusion reads the per-title blocklist only: an entry
        // for one title never hides the same release from another title, and
        // removing the entry re-allows the release immediately. The
        // `release_download_attempts` log is history/audit and never gates.
        let TitleReleaseBlocklistSignatures {
            source_hints: blocklisted_source_hints,
            source_titles: blocklisted_source_titles,
        } = self.load_title_release_blocklist_signatures(title_id).await;

        let (has_usenet_client, has_torrent_client, preferred_source_kind) =
            self.download_source_capabilities().await;

        raw_results.retain(|result| match result.source_kind {
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => has_usenet_client,
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
                has_torrent_client
            }
            None => true,
        });

        // A failure here is a failure, not an empty search (D12). Swallowing it
        // returned zero results from a "successful" pass, and the caller then
        // recorded the fired indexers as convergence coverage — so the scope was
        // marked searched and skipped until the next cycle, on the strength of a
        // transient store error. A title that is genuinely missing is not a
        // normal search outcome either: nothing can be scored for it.
        let scored_title = self.services.catalog.titles.get_by_id(title_id).await?;
        // The scoring inputs, resolved exactly once and exactly the way import
        // resolves them. Sharing this resolver — not just the term pipeline — is
        // what makes the two sides agree; a persona or language set resolved
        // differently would reintroduce the split this change set removes.
        let Some(scored_title) = scored_title else {
            return Err(AppError::NotFound(format!(
                "title {title_id} for release scoring"
            )));
        };
        let canonical_context = self
            .resolve_canonical_scoring_context(&scored_title, quality_profile)
            .await;
        let resolved_profile = canonical_context.profile().clone();

        let catalog_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(title_id)
            .await
            .unwrap_or_default();
        let catalog_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(title_id)
            .await
            .unwrap_or_default();
        let requested_episode =
            resolve_requested_episode(&catalog_episodes, season, episode, absolute_episode);
        let mut scored = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut rank_by_key: HashMap<String, crate::acquisition::scoring::SearchRank> =
            HashMap::new();
        let indexer_priority_by_name = self.build_indexer_priority_by_name(indexer_routing).await;
        let now = chrono::Utc::now();

        for result in raw_results {
            let key = release_search_key(&result);
            if !seen.insert(key) {
                continue;
            }

            if is_release_blocklisted(
                &result,
                &blocklisted_source_hints,
                &blocklisted_source_titles,
            ) {
                continue;
            }

            let parsed_release_metadata =
                parse_release_metadata_for_target(&result.title, parse_context);
            let scored_release_metadata =
                crate::quality::canonical_context::announced_metadata_for_title(
                    &scored_title,
                    &parsed_release_metadata,
                    &resolved_profile,
                    result.indexer_languages.as_deref(),
                );

            let release_coverage = crate::acquisition_coverage::resolve_release_coverage(
                &scored_release_metadata,
                &catalog_episodes,
                &catalog_collections,
                requested_episode,
            );
            if let Some(wanted_episode) = requested_episode
                && !release_coverage.covers_episode(wanted_episode)
            {
                continue;
            }
            let candidate_runtime_minutes = crate::acquisition_coverage::coverage_runtime_minutes(
                &release_coverage,
                &scored_release_metadata,
                &catalog_episodes,
                runtime_minutes,
            );

            if requested_episode.is_none()
                && let Some(ref ep_meta) = scored_release_metadata.episode
                && let Some(wanted_season) = season
                && let Some(parsed_season) = ep_meta.season
                && parsed_season != wanted_season
            {
                continue;
            }
            if requested_episode.is_none()
                && let Some(ref ep_meta) = scored_release_metadata.episode
                && let Some(wanted_episode) = episode
            {
                if !ep_meta.episode_numbers.is_empty()
                    && !ep_meta.episode_numbers.contains(&wanted_episode)
                {
                    continue;
                }
                if ep_meta.episode_numbers.is_empty()
                    && ep_meta.absolute_episode_numbers.is_empty()
                    && let (Some(parsed_abs), Some(wanted_abs)) =
                        (ep_meta.absolute_episode, absolute_episode)
                    && parsed_abs != wanted_abs
                {
                    continue;
                }
            }

            // One canonical score, from the same function and the same resolved
            // context the import path uses. Everything that used to be added
            // here and nowhere else — the freshness bonus, the single-episode
            // pack penalty, the listing-metadata rule inputs — is gone from the
            // number; what survives of it orders the results, in
            // `acquisition::scoring`, and never crosses a comparison.
            let scored_release = crate::canonical_scoring::score_release(
                &crate::canonical_scoring::ReleaseEvidence::announced(
                    scored_release_metadata.clone(),
                    result.size_bytes,
                ),
                &canonical_context.view(candidate_runtime_minutes, false),
            );
            let decision = scored_release.announced_decision;

            // Rank is built here, where the listing and the coverage are still
            // in hand, and dropped when the search ends. It is keyed by release
            // rather than carried on the result so it cannot leak into anything
            // that gets stored or compared later.
            rank_by_key.insert(
                release_search_key(&result),
                crate::acquisition::scoring::SearchRank {
                    head: crate::acquisition::scoring::RankHead {
                        blocked: !decision.allowed,
                        tier_index: decision.tier_index.unwrap_or(usize::MAX),
                        negated_revision: -(i32::from(scored_release_metadata.is_proper_upload)
                            + i32::from(scored_release_metadata.is_repack)),
                        negated_score: decision.preference_score.saturating_neg(),
                    },
                    indexer_priority: indexer_priority_by_name
                        .get(&result.source)
                        .copied()
                        .unwrap_or(i64::MAX),
                    negated_seeders: crate::acquisition::scoring::listing_negated_seeders(&result),
                    age_hours: crate::acquisition::scoring::listing_age_hours(
                        result.published_at.as_deref(),
                        now,
                    ),
                    coverage_distance: release_coverage.coverage_distance(requested_episode),
                    episode_number: scored_release_metadata
                        .episode
                        .as_ref()
                        .and_then(|episode| episode.episode_numbers.iter().min().copied())
                        .unwrap_or(0),
                },
            );

            scored.push(IndexerSearchResult {
                parsed_release_metadata: Some(scored_release_metadata),
                quality_profile_decision: Some(decision),
                // Carry the coverage the scoring pass already resolved (D21);
                // the auto evaluator has no catalog of its own and cannot
                // recompute it.
                coverage_scope: match release_coverage {
                    // `Title` and `Unknown` both map to `SubmissionScope::Title`,
                    // which would read as "this release covers the whole title"
                    // — an assertion neither of them makes. Absent is honest.
                    crate::acquisition_coverage::ReleaseCoverage::Title
                    | crate::acquisition_coverage::ReleaseCoverage::Unknown => None,
                    resolved => Some(resolved.submission_scope()),
                },
                ..result
            });
        }

        let mut scored = dedupe_cross_indexer_release_results(
            scored,
            &indexer_priority_by_name,
            preferred_source_kind.as_str(),
        );

        scored.sort_by(|left, right| {
            crate::acquisition::scoring::compare_ranked_results(
                left,
                right,
                &rank_by_key,
                release_search_key,
            )
        });

        Ok(scored)
    }

    /// Internal search+score pipeline shared by both user-facing search and background acquisition.
    /// Returns the scored releases plus the set of indexer ids that actually
    /// **fired** a query and returned a response (empty included), aggregated across
    /// all queries. The fired
    /// set — never the routed set — is what background acquisition records as
    /// Convergence coverage.
    pub(crate) async fn search_and_score_releases(
        &self,
        request: ReleaseSearchRequest<'_>,
    ) -> AppResult<(Vec<IndexerSearchResult>, Vec<String>)> {
        let ReleaseSearchRequest {
            queries,
            imdb_id,
            tmdb_id,
            tvdb_id,
            anidb_id,
            mal_id,
            category,
            owner_facet,
            search_facet,
            id_search_facet,
            newznab_categories,
            title_id,
            title_tags,
            library_id,
            caller_label,
            mode,
            runtime_minutes,
            parse_context,
            season,
            episode,
            absolute_episode,
            tagged_aliases,
            search_subject_kind,
            cancel_token,
            restrict_to_indexer_ids,
            background_value,
        } = request;
        if cancel_token.is_cancelled() {
            return Err(AppError::canceled("indexer search canceled"));
        }
        let quality_profile_lookup = QualityProfileLookup {
            title_tags,
            library_id,
            imdb_id: imdb_id.as_deref(),
            tvdb_id: tvdb_id.as_deref(),
            category_hint: Some(owner_facet.as_str()),
        };
        let quality_profile = self.resolve_quality_profile(quality_profile_lookup).await?;

        let scope_id = self.quality_profile_scope_id(quality_profile_lookup);
        let mut indexer_routing = self
            .resolve_indexer_routing(library_id, scope_id.as_deref())
            .await;
        // Restrict the search to the requested indexer subset (the convergence
        // cursor's uncovered indexers). With no routing plan
        // configured, synthesize one over the enabled indexers so the
        // restriction still applies.
        if let Some(allowed) = restrict_to_indexer_ids.as_ref() {
            let mut plan = match indexer_routing.take() {
                Some(plan) => plan,
                None => crate::contracts::IndexerRoutingPlan {
                    entries: self
                        .services
                        .integrations
                        .indexer_configs
                        .list(None)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|config| config.is_enabled)
                        .map(|config| {
                            (
                                config.id,
                                crate::contracts::IndexerRoutingEntry {
                                    enabled: true,
                                    categories: Vec::new(),
                                    priority: 0,
                                },
                            )
                        })
                        .collect(),
                },
            };
            for (indexer_id, entry) in plan.entries.iter_mut() {
                if !allowed.contains(indexer_id) {
                    entry.enabled = false;
                }
            }
            indexer_routing = Some(plan);
        }
        let newznab_categories = if newznab_categories.is_empty() {
            None
        } else {
            if let Some(plan) = indexer_routing.as_mut() {
                merge_series_movie_categories_into_routing(plan, &owner_facet, &newznab_categories);
            }
            Some(merge_newznab_category_codes(
                default_indexer_routing_categories_for_scope(owner_facet.as_str()),
                &newznab_categories,
            ))
        };

        // If routing exists and every indexer is disabled, skip the search entirely.
        if let Some(ref plan) = indexer_routing {
            let any_enabled = plan.entries.values().any(|e| e.enabled);
            if !any_enabled {
                info!(
                    caller = caller_label,
                    scope_id = scope_id.as_deref().unwrap_or("none"),
                    "all indexers disabled for scope, skipping search"
                );
                return Ok((Vec::new(), Vec::new()));
            }
        }

        let configured_indexers = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_else(|error| {
                warn!(error = %error, "failed to load indexer configs for transport-aware query collapse");
                vec![]
            });
        let collapse_structured_queries = search_subject_kind == ReleaseSearchSubjectKind::Episode
            && should_collapse_structured_nab_queries(
                &configured_indexers,
                indexer_routing.as_ref(),
                mode,
                chrono::Utc::now(),
            );

        // Auto mode normally conserves API calls by using the first query, but
        // episode acquisition keeps season/title fallbacks so packs and ranges
        // can be considered for a single requested episode. Broad structured
        // collapse is only safe when provider dispatch uses season/episode
        // parameters; text dispatch still needs distinct SxxEyy/Sxx/title forms.
        let effective_queries = match mode {
            SearchMode::Auto if search_subject_kind == ReleaseSearchSubjectKind::Episode => queries,
            SearchMode::Auto => queries.into_iter().take(1).collect(),
            SearchMode::Interactive => queries,
        };
        let effective_queries = if collapse_structured_queries {
            dedupe_structured_dispatch_queries(effective_queries, season, episode, absolute_episode)
        } else if mode == SearchMode::Auto
            && search_subject_kind == ReleaseSearchSubjectKind::Episode
        {
            dedupe_text_safe_structured_dispatch_queries(
                effective_queries,
                season,
                episode,
                absolute_episode,
            )
        } else {
            effective_queries
        };

        let mut set = JoinSet::new();
        let mut ids = HashMap::new();
        if let Some(imdb_id) = imdb_id.clone() {
            ids.insert("imdb_id".to_string(), imdb_id);
        }
        if let Some(tmdb_id) = tmdb_id.clone() {
            ids.insert("tmdb_id".to_string(), tmdb_id);
        }
        if let Some(tvdb_id) = tvdb_id.clone() {
            ids.insert("tvdb_id".to_string(), tvdb_id);
        }
        if let Some(anidb_id) = anidb_id.clone() {
            ids.insert("anidb_id".to_string(), anidb_id);
        }
        if let Some(mal_id) = mal_id.clone() {
            ids.insert("mal_id".to_string(), mal_id);
        }
        let learning_context = if mode == SearchMode::Auto && !title_id.trim().is_empty() {
            Some(IndexerSearchLearningContext {
                title_id: title_id.to_string(),
                facet: search_facet.as_str().to_string(),
                subject_kind: search_subject_kind,
                // The convergence value hint rides the Auto background context so
                // the scheduler can lane-rank this scope.
                background_value,
            })
        } else {
            None
        };

        for query in effective_queries {
            let indexer_client = self.services.integrations.indexer_client.clone();
            let ids = ids.clone();
            let category = category.clone();
            let facet = Some(search_facet.as_str().to_string());
            let id_search_facet = id_search_facet
                .as_ref()
                .map(|facet| facet.as_str().to_string());
            let indexer_routing = indexer_routing.clone();
            let newznab_categories = newznab_categories.clone();
            let tagged_aliases = tagged_aliases.to_vec();
            let learning_context = learning_context.clone();
            let query = query.clone();
            let query_cancel_token = cancel_token.child_token();

            set.spawn(async move {
                indexer_client
                    .search(
                        query,
                        ids,
                        category.clone(),
                        facet,
                        id_search_facet,
                        newznab_categories,
                        indexer_routing,
                        mode,
                        season,
                        episode,
                        absolute_episode,
                        tagged_aliases,
                        learning_context,
                        query_cancel_token,
                    )
                    .await
            });
        }

        let mut query_failures = 0usize;
        let mut successful_searches = 0usize;
        let mut first_failure: Option<String> = None;
        let mut raw_results: Vec<IndexerSearchResult> = Vec::new();
        // Indexers that fired a query and returned a response (empty
        // included) across any query. Aggregated here so the coverage write-hook
        // records exactly the fired subset, never the routed set.
        let mut fired_indexers: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        loop {
            let result = tokio::select! {
                _ = cancel_token.cancelled() => {
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    return Err(AppError::canceled("indexer search canceled"));
                }
                result = set.join_next() => result,
            };

            let Some(result) = result else {
                break;
            };

            match result {
                Ok(Ok(mut response)) => {
                    successful_searches += 1;
                    for outcome in &response.indexer_outcomes {
                        if outcome.outcome.fired() {
                            fired_indexers.insert(outcome.indexer_id.clone());
                        }
                    }
                    for result in &mut response.results {
                        let provenance =
                            result.provenance.get_or_insert(ReleaseCandidateProvenance {
                                search_subject_kind,
                                strategy_kind: ReleaseStrategyKind::Fallback,
                                title_validated_upstream: false,
                            });
                        provenance.search_subject_kind = search_subject_kind;
                    }
                    raw_results.append(&mut response.results);
                }
                Ok(Err(error)) => {
                    if error.is_canceled() {
                        set.abort_all();
                        while set.join_next().await.is_some() {}
                        return Err(error);
                    }
                    query_failures += 1;
                    first_failure = first_failure.or_else(|| Some(error.to_string()));
                    warn!(
                        caller = caller_label,
                        error = %error,
                        "indexer search query failed"
                    );
                }
                Err(error) => {
                    query_failures += 1;
                    first_failure = first_failure.or_else(|| Some(error.to_string()));
                    warn!(
                        caller = caller_label,
                        error = %error,
                        "indexer search task panicked"
                    );
                }
            }
        }

        if raw_results.is_empty() && successful_searches == 0 && query_failures > 0 {
            let details =
                first_failure.unwrap_or_else(|| "all indexer search queries failed".to_string());
            return Err(AppError::Repository(details));
        }

        let scored = self
            .score_release_results(
                raw_results,
                &quality_profile,
                title_id,
                indexer_routing.as_ref(),
                runtime_minutes,
                parse_context,
                season,
                episode,
                absolute_episode,
            )
            .await?;
        Ok((scored, fired_indexers.into_iter().collect()))
    }

    pub(crate) async fn search_and_evaluate_subject(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        self.search_and_evaluate_subject_restricted(
            title,
            subject,
            caller_label,
            mode,
            cancel_token,
            None,
            None,
        )
        .await
    }

    /// Search and evaluate `subject`, optionally restricted to a subset of
    /// indexers. The convergence cursor passes the scope's uncovered subset
    /// — a covered indexer's catalog holds no new information
    /// for this scope, so re-querying it is pure spend.
    #[expect(
        clippy::too_many_arguments,
        reason = "background search threads the convergence subset and value hint alongside the subject"
    )]
    pub(crate) async fn search_and_evaluate_subject_restricted(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
        background_value: Option<f64>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let tagged_aliases = release_search_tagged_aliases(title);
        let (results, fired_indexer_ids) = self
            .search_and_score_releases(ReleaseSearchRequest {
                queries: subject.queries.clone(),
                imdb_id: subject.imdb_id.clone(),
                tmdb_id: subject.tmdb_id.clone(),
                tvdb_id: subject.tvdb_id.clone(),
                anidb_id: subject.anidb_id.clone(),
                mal_id: subject.mal_id.clone(),
                category: Some(subject.category.clone()),
                owner_facet: subject.owner_facet.clone(),
                search_facet: subject.search_facet.clone(),
                id_search_facet: subject.id_search_facet.clone(),
                newznab_categories: subject.newznab_categories.clone(),
                title_id: subject.title_id.as_str(),
                title_tags: &subject.title_tags,
                library_id: Some(title.library_id.as_str()),
                caller_label,
                mode,
                runtime_minutes: subject.runtime_minutes,
                season: subject.season,
                episode: subject.episode,
                absolute_episode: subject.absolute_episode,
                tagged_aliases: &tagged_aliases,
                search_subject_kind: subject.subject_kind,
                parse_context: &subject.title_evidence.parse_context,
                cancel_token,
                restrict_to_indexer_ids,
                background_value,
            })
            .await?;

        let evaluated = self
            .evaluate_search_results_for_subject(title, subject, results)
            .await;
        // A search is a search: every scoped search — background,
        // interactive, season-pack — records per-indexer convergence coverage
        // for the indexers that actually fired. Best-effort.
        self.record_search_coverage(title, subject, &fired_indexer_ids)
            .await;
        Ok(evaluated)
    }

    /// Interactive search for a title (movie or standalone). Resolves all
    /// external IDs and search category from the title record so the frontend
    /// only needs to pass the title ID.
    pub(crate) async fn attach_candidate_tokens(
        &self,
        actor: &User,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        results: &mut [IndexerSearchResult],
        preserve_subject_scope: bool,
    ) {
        let signing_key = match self.release_candidate_signing_key_for_actor(actor).await {
            Ok(signing_key) => signing_key,
            Err(err) => {
                warn!(
                    actor = actor.id.as_str(),
                    title_id = title.id.as_str(),
                    scope = ?subject.submission_scope,
                    error = %err,
                    "failed to resolve candidate-token signing key for title-aware search"
                );
                for result in results.iter_mut() {
                    result.candidate_token = None;
                }
                return;
            }
        };

        let catalog_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        let catalog_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let requested_episode = resolve_requested_episode(
            &catalog_episodes,
            subject.season,
            subject.episode,
            subject.absolute_episode,
        );

        for result in results.iter_mut() {
            let scope = if preserve_subject_scope {
                subject.submission_scope.clone()
            } else {
                result
                    .parsed_release_metadata
                    .as_ref()
                    .map(|parsed| {
                        crate::acquisition_coverage::resolve_release_coverage(
                            parsed,
                            &catalog_episodes,
                            &catalog_collections,
                            requested_episode,
                        )
                        .submission_scope_or(&subject.submission_scope)
                    })
                    .unwrap_or_else(|| subject.submission_scope.clone())
            };
            result.queue_scope = Some(scope.clone());
            let canonical_source = result.canonical_download_source();
            let selection = QueuedReleaseSelection {
                indexer_id: result.indexer_id.clone(),
                source_hint: canonical_source.as_ref().map(|(source, _)| source.clone()),
                source_kind: canonical_source
                    .as_ref()
                    .map(|(_, kind)| *kind)
                    .or(result.source_kind),
                source_title: Some(result.title.clone()),
                source_password: result.password_hint.clone(),
                size_bytes: result.size_bytes,
                seeders: crate::acquisition::seed_goals::seeders_from_extra(&result.extra),
            };
            result.candidate_token = if selection.source_hint.is_some() {
                match self.issue_release_candidate_token_with_signing_key(
                    actor,
                    &title.id,
                    &scope,
                    &selection,
                    &signing_key,
                ) {
                    Ok(token) => Some(token),
                    Err(err) => {
                        warn!(
                            actor = actor.id.as_str(),
                            title_id = title.id.as_str(),
                            scope = ?scope,
                            release_title = result.title.as_str(),
                            error = %err,
                            "failed to attach candidate token to title-aware search result"
                        );
                        None
                    }
                }
            } else {
                None
            };
        }
    }

    pub async fn search_indexers_for_title(
        &self,
        actor: &User,
        title_id: String,
        cancel_token: CancellationToken,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let subject = self
            .resolve_release_search_subject_for_title(&title)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for title"
        );

        let mut results = self
            .search_and_evaluate_subject(
                &title,
                &subject,
                &actor.id,
                SearchMode::Interactive,
                cancel_token,
            )
            .await?;
        self.attach_candidate_tokens(actor, &title, &subject, &mut results, false)
            .await;

        self.emit_discovery_search_completed_event(
            actor,
            subject.category.clone(),
            subject.queries.first().cloned(),
            results.len() as i64,
        )
        .await;

        Ok(results)
    }

    pub async fn search_indexers_for_series_movie(
        &self,
        actor: &User,
        title_id: String,
        series_movie_link_id: String,
        cancel_token: CancellationToken,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let link = self
            .services
            .catalog
            .shows
            .get_series_movie_link_by_id(&series_movie_link_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("series movie {series_movie_link_id}")))?;
        if link.series_title_id != title.id {
            return Err(AppError::Validation(
                "series movie does not belong to title".into(),
            ));
        }

        let (search_title, subject) = self
            .resolve_release_search_subject_for_series_movie(&title, &link)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            series_movie_link_id = series_movie_link_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for series movie"
        );

        let mut results = self
            .search_and_evaluate_subject(
                &search_title,
                &subject,
                &actor.id,
                SearchMode::Interactive,
                cancel_token,
            )
            .await?;
        self.attach_candidate_tokens(actor, &search_title, &subject, &mut results, true)
            .await;

        self.emit_discovery_search_completed_event(
            actor,
            subject.category.clone(),
            subject.queries.first().cloned(),
            results.len() as i64,
        )
        .await;

        Ok(results)
    }

    /// Interactive search for a specific episode. Resolves all external IDs,
    /// search category, and absolute episode number from the title/episode
    /// records so the frontend only needs to pass title ID + season + episode.
    pub async fn search_indexers_for_episode(
        &self,
        actor: &User,
        title_id: String,
        season: String,
        episode: String,
        cancel_token: CancellationToken,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let subject = self
            .resolve_release_search_subject_for_episode(&title, &season, &episode)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for episode"
        );

        let mut results = self
            .search_and_evaluate_subject(
                &title,
                &subject,
                &actor.id,
                SearchMode::Interactive,
                cancel_token,
            )
            .await?;
        self.attach_candidate_tokens(actor, &title, &subject, &mut results, false)
            .await;

        self.emit_discovery_search_completed_event(
            actor,
            subject.category.clone(),
            subject.queries.first().cloned(),
            results.len() as i64,
        )
        .await;

        Ok(results)
    }
}

/// Upper bound on blocklist entries read per title for search-time exclusion.
const TITLE_RELEASE_BLOCKLIST_READ_LIMIT: usize = 1_000;

/// A title's blocklisted release signatures, normalized exactly the way
/// [`is_release_blocklisted`] normalizes candidates (hints trimmed, titles
/// trimmed + lowercased). The per-title `blocklist` table is the single
/// search-time exclusion source; `release_download_attempts` is history only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleReleaseBlocklistSignatures {
    pub(crate) source_hints: HashSet<String>,
    pub(crate) source_titles: HashSet<String>,
}

impl AppUseCase {
    /// Loads and normalizes the title's blocklist entries. Entries are stored
    /// with mixed casing (grab-time writers keep the indexer casing, failure
    /// writers lowercase), so both sides normalize here. A repository error
    /// warns and yields an empty set: a storage hiccup degrades to "nothing
    /// excluded" rather than failing the search.
    pub(crate) async fn load_title_release_blocklist_signatures(
        &self,
        title_id: &str,
    ) -> TitleReleaseBlocklistSignatures {
        let entries = match self
            .services
            .workflow
            .blocklist_repo
            .list_for_title(title_id, TITLE_RELEASE_BLOCKLIST_READ_LIMIT)
            .await
        {
            Ok(entries) => entries,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id,
                    "failed to load title release blocklist; excluding nothing this search"
                );
                Vec::new()
            }
        };

        let mut signatures = TitleReleaseBlocklistSignatures::default();
        for entry in &entries {
            if let Some(hint) = normalize_release_attempt_hint(entry.source_hint.as_deref()) {
                signatures.source_hints.insert(hint);
            }
            if let Some(title) = normalize_release_attempt_title(entry.source_title.as_deref()) {
                signatures.source_titles.insert(title);
            }
        }
        signatures
    }
}

/// Whether a release title matches a normalized per-title blocklist title set
/// (see [`TitleReleaseBlocklistSignatures::source_titles`]).
pub(crate) fn is_release_title_blocklisted(
    release_title: &str,
    blocklisted_source_titles: &HashSet<String>,
) -> bool {
    normalize_release_attempt_title(Some(release_title))
        .is_some_and(|title| blocklisted_source_titles.contains(&title))
}

pub(crate) fn is_release_blocklisted(
    result: &IndexerSearchResult,
    failed_source_hints: &std::collections::HashSet<String>,
    failed_source_titles: &std::collections::HashSet<String>,
) -> bool {
    if result.source_aliases().iter().any(|alias| {
        normalize_release_attempt_hint(Some(alias.as_str()))
            .is_some_and(|alias| failed_source_hints.contains(&alias))
    }) {
        return true;
    }

    if let Some(title) = normalize_release_attempt_title(Some(result.title.as_str()))
        && failed_source_titles.contains(&title)
    {
        return true;
    }

    false
}

#[derive(Clone, Copy)]
pub(crate) struct QualityProfileLookup<'a> {
    pub(crate) title_tags: &'a [String],
    pub(crate) library_id: Option<&'a str>,
    pub(crate) imdb_id: Option<&'a str>,
    pub(crate) tvdb_id: Option<&'a str>,
    pub(crate) category_hint: Option<&'a str>,
}

pub(crate) struct ReleaseSearchRequest<'a> {
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
    pub(crate) mal_id: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) owner_facet: MediaFacet,
    pub(crate) search_facet: MediaFacet,
    pub(crate) id_search_facet: Option<MediaFacet>,
    pub(crate) newznab_categories: Vec<String>,
    pub(crate) title_id: &'a str,
    pub(crate) title_tags: &'a [String],
    pub(crate) library_id: Option<&'a str>,
    pub(crate) caller_label: &'a str,
    pub(crate) mode: SearchMode,
    pub(crate) runtime_minutes: Option<i32>,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) absolute_episode: Option<u32>,
    pub(crate) tagged_aliases: &'a [TaggedAlias],
    pub(crate) search_subject_kind: ReleaseSearchSubjectKind,
    pub(crate) parse_context: &'a ReleaseParseContext,
    pub(crate) cancel_token: CancellationToken,
    /// When set, only these indexer ids are queried (the convergence cursor's
    /// uncovered subset). `None` = every routed indexer.
    pub(crate) restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
    /// Background convergence value hint: the target's recency
    /// lane maps to a scheduler candidate value (hot → high, cold → low) so
    /// the quota-pressure gate can drain cold work first. Only the Auto
    /// background path carries it; interactive/RSS leave it `None` (neutral).
    pub(crate) background_value: Option<f64>,
}

/// The configured scope that supplied an effective quality profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QualityProfileResolutionSource {
    Title,
    Library,
    Category,
    Global,
    Builtin,
}

impl QualityProfileResolutionSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Library => "library",
            Self::Category => "category",
            Self::Global => "global",
            Self::Builtin => "builtin",
        }
    }
}

/// Effective profile data and the precedence scope that selected it.
#[derive(Clone, Debug)]
pub(crate) struct QualityProfileResolution {
    pub(crate) profile: QualityProfile,
    pub(crate) profile_id: String,
    pub(crate) source: QualityProfileResolutionSource,
}

impl AppUseCase {
    pub(crate) async fn resolve_quality_profile(
        &self,
        lookup: QualityProfileLookup<'_>,
    ) -> AppResult<QualityProfile> {
        let resolution = self.resolve_quality_profile_resolution(lookup).await?;
        debug!(
            quality_profile_id = resolution.profile_id,
            quality_profile_source = resolution.source.as_str(),
            "resolved quality profile"
        );
        Ok(resolution.profile)
    }

    /// Resolves the effective profile together with the precedence scope that
    /// supplied it. Callers that only need scoring can use
    /// [`Self::resolve_quality_profile`]; diagnostics should retain this
    /// provenance instead of attempting to recompute it later.
    pub(crate) async fn resolve_quality_profile_resolution(
        &self,
        lookup: QualityProfileLookup<'_>,
    ) -> AppResult<QualityProfileResolution> {
        let catalog = self.load_quality_profiles().await?;
        let category_scope_id = self.quality_profile_scope_id(lookup);

        let title_profile_id = lookup
            .title_tags
            .iter()
            .find(|t| t.starts_with("scryer:quality-profile:"))
            .map(|t| {
                t.trim_start_matches("scryer:quality-profile:")
                    .trim()
                    .to_string()
            })
            .filter(|value| !value.is_empty() && value != QUALITY_PROFILE_INHERIT_VALUE);

        let category_profile_id = self
            .read_setting_string_value_explicit(
                QUALITY_PROFILE_ID_KEY,
                category_scope_id.as_deref(),
            )
            .await?;
        let library_profile_id = match lookup.library_id {
            Some(library_id) => {
                self.read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(library_id))
                    .await?
            }
            None => None,
        };
        let global_profile_id = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
            .await?;

        let (active_profile_id, source) = if let Some(profile_id) = title_profile_id {
            (Some(profile_id), QualityProfileResolutionSource::Title)
        } else if let Some(profile_id) = library_profile_id {
            (Some(profile_id), QualityProfileResolutionSource::Library)
        } else if let Some(profile_id) = category_profile_id {
            (Some(profile_id), QualityProfileResolutionSource::Category)
        } else if let Some(profile_id) = global_profile_id {
            (Some(profile_id), QualityProfileResolutionSource::Global)
        } else {
            (None, QualityProfileResolutionSource::Builtin)
        };

        if let Some(profile_id) = active_profile_id {
            let profile = crate::settings::runtime::quality_profile_by_id(&catalog, &profile_id)?
                .cloned()
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "configured quality profile '{profile_id}' from {} is missing from the catalog",
                        source.as_str()
                    ))
                })?;
            return Ok(QualityProfileResolution {
                profile_id: profile.id.clone(),
                profile,
                source,
            });
        }

        if !catalog.is_empty() {
            return Err(AppError::Validation(
                "no quality profile is configured; choose a global, category, library, or title profile"
                    .to_string(),
            ));
        }

        let profile = builtin_default_quality_profile();
        Ok(QualityProfileResolution {
            profile_id: profile.id.clone(),
            profile,
            source: QualityProfileResolutionSource::Builtin,
        })
    }

    async fn load_quality_profiles(&self) -> AppResult<Vec<QualityProfile>> {
        self.services
            .config
            .quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await
    }

    pub(crate) async fn read_setting_string_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, key_name, scope_id)
            .await
    }

    pub(crate) async fn read_setting_string_value_explicit(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.read_setting_string_value_for_scope_explicit(SETTINGS_SCOPE_SYSTEM, key_name, scope_id)
            .await
    }

    pub(crate) async fn read_setting_string_value_for_scope(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        let scope_id = scope_id.map(std::string::ToString::to_string);
        let Some(raw_value) = self
            .services
            .config
            .settings
            .get_setting_json(scope, key_name, scope_id)
            .await?
        else {
            return Ok(None);
        };

        let trimmed = raw_value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        if trimmed == INHERIT_QUALITY_PROFILE_VALUE {
            return Ok(None);
        }

        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(Some(trimmed.to_string()));
        };
        match parsed {
            Value::Null => Ok(None),
            Value::String(value) => {
                let normalized = value.trim();
                if normalized.is_empty() || normalized == INHERIT_QUALITY_PROFILE_VALUE {
                    Ok(None)
                } else {
                    Ok(Some(normalized.to_string()))
                }
            }
            _ => Ok(Some(trimmed.to_string())),
        }
    }

    pub(crate) async fn read_setting_string_value_for_scope_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        let scope_id = scope_id.map(std::string::ToString::to_string);
        let Some(raw_value) = self
            .services
            .config
            .settings
            .get_setting_json_explicit(scope, key_name, scope_id)
            .await?
        else {
            return Ok(None);
        };

        let trimmed = raw_value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        if trimmed == INHERIT_QUALITY_PROFILE_VALUE {
            return Ok(None);
        }

        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(Some(trimmed.to_string()));
        };
        match parsed {
            Value::Null => Ok(None),
            Value::String(value) => {
                let normalized = value.trim();
                if normalized.is_empty() || normalized == INHERIT_QUALITY_PROFILE_VALUE {
                    Ok(None)
                } else {
                    Ok(Some(normalized.to_string()))
                }
            }
            _ => Ok(Some(trimmed.to_string())),
        }
    }

    pub(crate) fn quality_profile_scope_id(
        &self,
        lookup: QualityProfileLookup<'_>,
    ) -> Option<String> {
        if let Some(value) = lookup.category_hint {
            let normalized = value.to_ascii_lowercase();
            match normalized.as_str() {
                "movie" => return Some("movie".to_string()),
                "series" => return Some("series".to_string()),
                "anime" => return Some("anime".to_string()),
                "5070" => return Some("series".to_string()),
                _ => {}
            }
        }

        if lookup.imdb_id.is_some() {
            return Some("movie".to_string());
        }
        if lookup.tvdb_id.is_some() {
            return Some("series".to_string());
        }

        None
    }

    /// Resolve Newznab category codes from the user's indexer routing settings
    /// for the given scope_id (movie/series/anime).
    ///
    /// Returns `None` if no routing is configured (caller falls back to
    /// hardcoded defaults). Returns `Some(vec![])` if all indexers are
    /// disabled for this scope (caller should skip search).
    pub(crate) async fn resolve_indexer_routing(
        &self,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> Option<IndexerRoutingPlan> {
        if let Some(library_id) = library_id {
            match self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(library_id))
                .await
            {
                Ok(Some(value)) => {
                    if let Some(plan) = self.parse_indexer_routing_plan(library_id, &value) {
                        return Some(plan);
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(
                        error = %err,
                        library_id = library_id,
                        "failed to read library indexer routing setting, falling back to facet defaults"
                    );
                }
            }
        }

        let scope_id = scope_id?;

        let raw_json = match self
            .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
        {
            Ok(Some(value)) => value,
            Ok(None) => return None,
            Err(err) => {
                warn!(
                    error = %err,
                    scope_id = scope_id,
                    "failed to read indexer routing setting, falling back to defaults"
                );
                return None;
            }
        };

        self.parse_indexer_routing_plan(scope_id, &raw_json)
    }

    pub(crate) fn parse_indexer_routing_plan(
        &self,
        scope_id: &str,
        raw_json: &str,
    ) -> Option<IndexerRoutingPlan> {
        let parsed: Value = match serde_json::from_str(raw_json) {
            Ok(value) => value,
            Err(_) => return None,
        };

        let obj = parsed.as_object()?;
        if obj.is_empty() {
            return None;
        }

        let mut entries = std::collections::HashMap::new();

        // The canonical write paths in settings.rs and the startup
        // `normalize_routing_settings` migration always emit `enabled` and
        // `priority`. The `unwrap_or` fallbacks here are transitional
        // legacy-compat for installs that haven't yet been normalized.
        for (indexer_id, config) in obj {
            let enabled = match config.get("enabled").and_then(|v| v.as_bool()) {
                Some(value) => value,
                None => {
                    debug!(
                        scope_id,
                        indexer_id,
                        "indexer routing entry missing `enabled`; using legacy default `true`"
                    );
                    true
                }
            };

            let mut categories: Vec<String> = Vec::new();
            if let Some(cats) = config.get("categories").and_then(|v| v.as_array()) {
                for cat in cats {
                    if let Some(cat_str) = cat.as_str() {
                        let trimmed = cat_str.trim();
                        if !trimmed.is_empty() {
                            categories.push(trimmed.to_string());
                        }
                    }
                }
            }

            let priority = match config.get("priority").and_then(|v| v.as_i64()) {
                Some(value) => value,
                None => {
                    debug!(
                        scope_id,
                        indexer_id,
                        "indexer routing entry missing `priority`; using legacy default `i64::MAX`"
                    );
                    i64::MAX
                }
            };

            entries.insert(
                indexer_id.clone(),
                IndexerRoutingEntry {
                    enabled,
                    categories,
                    priority,
                },
            );
        }

        debug!(
            scope_id = scope_id,
            indexer_count = entries.len(),
            "resolved per-indexer routing plan"
        );
        Some(IndexerRoutingPlan { entries })
    }
}

#[cfg(test)]
mod structured_dispatch_query_tests {
    use super::*;

    #[test]
    fn text_safe_dedupe_preserves_distinct_episode_season_absolute_and_title_queries() {
        let queries = vec![
            "Silver Horizon 033".to_string(),
            "Silver Horizon S02E05".to_string(),
            "Silver Horizon S02".to_string(),
            "Silver Horizon".to_string(),
        ];

        let deduped = dedupe_text_safe_structured_dispatch_queries(
            queries.clone(),
            Some(2),
            Some(5),
            Some(33),
        );

        assert_eq!(deduped, queries);
    }

    #[test]
    fn broad_structured_dedupe_still_collapses_equivalent_parameterized_queries() {
        let queries = vec![
            "Silver Horizon 033".to_string(),
            "Silver Horizon S02E05".to_string(),
            "Silver Horizon S02".to_string(),
            "Silver Horizon".to_string(),
        ];

        let deduped = dedupe_structured_dispatch_queries(queries, Some(2), Some(5), Some(33));

        assert_eq!(deduped, vec!["Silver Horizon 033".to_string()]);
    }
}

#[cfg(test)]
#[path = "app_usecase_discovery_tests.rs"]
mod app_usecase_discovery_tests;
