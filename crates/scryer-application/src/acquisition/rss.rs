use super::*;
use crate::acquisition_decision_helpers::{
    blocklist_entry_data, is_download_submit_unavailable_error,
};
use crate::acquisition_release_search::{
    AutoCandidateEvaluationContext, CandidateTitleMatch, ReleaseAutoDecisionCode,
    annotate_auto_decision, candidate_presents_identity_disambiguator, canonical_title_evidence,
    context_free_identity_anchor_keys, evaluate_auto_candidate, external_id_agreement,
    match_parsed_release_to_title_evidence, parsed_release_matches_title_evidence,
    serialize_decision_explanation, series_movie_search_title,
};
use crate::acquisition_search_queries::{
    imdb_id_from_title, tmdb_id_from_external_ids, tvdb_id_from_external_ids,
};
use crate::delay_profile::DelayProfile;
use crate::domain_events::{new_title_domain_event, title_context_snapshot};
use crate::settings::keys::default_indexer_routing_categories_for_scope;
use chrono::{DateTime, Utc};
use scryer_domain::{DomainEventPayload, ReleaseGrabbedEventData};
use std::collections::{HashMap, HashSet};
use tracing::{info, warn};

const RSS_SYNC_MAX_GUIDS: usize = 2000;

fn rss_categories_for_routing_entry(scope_id: &str, entry: &IndexerRoutingEntry) -> Vec<String> {
    if entry.categories.is_empty() {
        default_indexer_routing_categories_for_scope(scope_id)
    } else {
        entry.categories.clone()
    }
}

/// Normalize a title string for fuzzy matching: lowercase, strip non-alphanumeric,
/// collapse whitespace.
pub(crate) fn normalize_for_matching(title: &str) -> String {
    crate::title_matching::canonical_lookup_key(title)
}

#[derive(Clone)]
struct TitleMatchInfo {
    title_id: String,
    year: Option<i32>,
    /// External ids Scryer already holds for this title, so an indexer-asserted
    /// response id can disambiguate a shared canonical key (A2(2)).
    tvdb_id: Option<String>,
    tmdb_id: Option<String>,
    imdb_id: Option<String>,
}

#[derive(Clone)]
struct TitleContextCandidate {
    info: TitleMatchInfo,
    evidence: crate::acquisition_release_search::CanonicalTitleEvidence,
}

struct TitleContextBank {
    candidates: Vec<TitleContextCandidate>,
    key_index: HashMap<String, Vec<usize>>,
    tvdb_index: HashMap<String, Vec<usize>>,
    tmdb_index: HashMap<String, Vec<usize>>,
    imdb_index: HashMap<String, Vec<usize>>,
}

impl std::ops::Deref for TitleContextBank {
    type Target = [TitleContextCandidate];

    fn deref(&self) -> &Self::Target {
        &self.candidates
    }
}

fn build_title_context_bank(titles: &[Title]) -> TitleContextBank {
    let mut candidates = titles
        .iter()
        .filter(|title| title.monitored)
        .map(|title| TitleContextCandidate {
            info: TitleMatchInfo {
                title_id: title.id.clone(),
                year: title.year,
                tvdb_id: tvdb_id_from_external_ids(&title.external_ids),
                tmdb_id: tmdb_id_from_external_ids(&title.external_ids),
                imdb_id: imdb_id_from_title(title),
            },
            evidence: canonical_title_evidence(title),
        })
        .collect::<Vec<_>>();

    // Pillar A tier 0 on the RSS path: collisions are grouped over ALL input
    // titles (an unmonitored collider is still a collider) on the
    // year-stripped key shape, so `Tide Chart` and `Tide Chart (2023)` collide.
    // The bank itself stays monitored-only — no extra queries either way.
    let mut titles_per_stripped_key: HashMap<&str, HashSet<&str>> = HashMap::new();
    let all_title_keys = titles
        .iter()
        .map(crate::acquisition_release_search::canonical_title_lookup_keys)
        .collect::<Vec<_>>();
    for (title, keys) in titles.iter().zip(&all_title_keys) {
        for key in keys {
            titles_per_stripped_key
                .entry(crate::import_title_resolution::strip_trailing_year_key(key))
                .or_default()
                .insert(title.id.as_str());
        }
    }
    let shared_stripped_keys = titles_per_stripped_key
        .into_iter()
        .filter(|(_, title_ids)| title_ids.len() >= 2)
        .map(|(key, _)| key.to_string())
        .collect::<HashSet<_>>();

    if !shared_stripped_keys.is_empty() {
        for candidate in &mut candidates {
            candidate.evidence = candidate.evidence.clone().with_ambiguity(
                crate::acquisition_release_search::TitleIdentityAmbiguity::from_shared_keys(
                    candidate
                        .evidence
                        .lookup_keys
                        .iter()
                        .filter(|key| {
                            shared_stripped_keys.contains(
                                crate::import_title_resolution::strip_trailing_year_key(key),
                            )
                        })
                        .cloned()
                        .collect(),
                ),
            );
        }
    }

    let mut key_index = HashMap::<String, Vec<usize>>::new();
    let mut tvdb_index = HashMap::<String, Vec<usize>>::new();
    let mut tmdb_index = HashMap::<String, Vec<usize>>::new();
    let mut imdb_index = HashMap::<String, Vec<usize>>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        for key in &candidate.evidence.lookup_keys {
            for indexed_key in [
                key.as_str(),
                crate::import_title_resolution::strip_trailing_year_key(key),
            ] {
                if indexed_key.is_empty() {
                    continue;
                }
                let indexes = key_index.entry(indexed_key.to_string()).or_default();
                if !indexes.contains(&index) {
                    indexes.push(index);
                }
            }
        }
        for (value, index_map) in [
            (candidate.info.tvdb_id.as_ref(), &mut tvdb_index),
            (candidate.info.tmdb_id.as_ref(), &mut tmdb_index),
            (candidate.info.imdb_id.as_ref(), &mut imdb_index),
        ] {
            if let Some(value) = value {
                index_map
                    .entry(value.to_ascii_lowercase())
                    .or_default()
                    .push(index);
            }
        }
    }

    TitleContextBank {
        candidates,
        key_index,
        tvdb_index,
        tmdb_index,
        imdb_index,
    }
}

/// Extract the series/movie title portion from a release name by taking
/// everything before the first recognized quality/episode marker.
#[cfg(test)]
fn extract_title_from_release(parsed: &ParsedReleaseMetadata) -> String {
    extract_titles_from_release(parsed)
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[cfg(test)]
fn extract_titles_from_release(parsed: &ParsedReleaseMetadata) -> Vec<String> {
    let mut titles = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    if titles.is_empty() {
        titles.push(parsed.normalized_title.clone());
    }

    titles
        .into_iter()
        .map(|title| normalize_for_matching(&title))
        .filter(|title| !title.is_empty())
        .fold(Vec::<String>::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == &value) {
                acc.push(value);
            }
            acc
        })
}

/// Match an RSS release against monitored titles using real title contexts.
/// Candidates come from exact lookups of the release's context-free anchor
/// keys (plus the indexer's own id assertions) — there is no lexical
/// containment scan to admit junk — and every candidate then faces the full
/// contextual proof in [`match_parsed_release_to_title_evidence`].
/// `response_attributes` carries the indexer's own id assertions so a collision
/// on a shared canonical key can still be resolved (A2(2)).
fn match_release_to_title_context<'a>(
    release_title: &str,
    response_attributes: &IndexerResponseAttributes,
    context_bank: &'a TitleContextBank,
) -> Option<&'a TitleMatchInfo> {
    let anchor_keys = context_free_identity_anchor_keys(release_title);
    let mut candidate_indexes = Vec::<usize>::new();
    // A stacked-alias name extracts as one glued title no single key equals, so
    // candidacy also probes every token prefix of each anchor key. Discovery
    // only — each candidate still faces the full anchored proof below.
    for key in &anchor_keys {
        let tokens = key.split_whitespace().collect::<Vec<_>>();
        for end in 1..=tokens.len() {
            if let Some(indexes) = context_bank.key_index.get(&tokens[..end].join(" ")) {
                for index in indexes {
                    if !candidate_indexes.contains(index) {
                        candidate_indexes.push(*index);
                    }
                }
            }
        }
    }
    for (asserted_id, index_map) in [
        (
            response_attributes.tvdb_id.as_ref(),
            &context_bank.tvdb_index,
        ),
        (
            response_attributes.tmdb_id.as_ref(),
            &context_bank.tmdb_index,
        ),
        (
            response_attributes.imdb_id.as_ref(),
            &context_bank.imdb_index,
        ),
    ] {
        if let Some(asserted_id) = asserted_id
            && let Some(indexes) = index_map.get(&asserted_id.to_ascii_lowercase())
        {
            for index in indexes {
                if !candidate_indexes.contains(index) {
                    candidate_indexes.push(*index);
                }
            }
        }
    }
    if candidate_indexes.is_empty() {
        return None;
    }

    let mut best: Option<(&TitleMatchInfo, i32, bool)> = None;
    let mut titles_per_matched_key: HashMap<String, HashSet<&str>> = HashMap::new();
    for index in candidate_indexes {
        let Some(candidate) = context_bank.candidates.get(index) else {
            continue;
        };
        // The target-biased parse supplies year/projection semantics (it knows
        // when a year token is part of the title, as in `Signal Runner 2049`);
        // identity still anchors on the context-free extraction inside
        // `match_parsed_release_to_title_evidence`.
        let parsed =
            parse_release_metadata_for_target(release_title, &candidate.evidence.parse_context);
        if let (Some(parsed_year), Some(title_year)) = (parsed.year, candidate.info.year)
            && parsed_year != title_year
        {
            continue;
        }
        let Some(evidence_match) =
            match_parsed_release_to_title_evidence(&parsed, &candidate.evidence)
        else {
            continue;
        };

        // The matched key's specificity ranks colliding candidates: a longer
        // key names the release more precisely than a shared bare key. Score
        // the year-stripped shape — a year-suffixed twin (`HarborTales (2017)`)
        // is not more specific than its bare twin when the release itself
        // carries no year, and an undisambiguated twin tie must keep the
        // deterministic title-id winner so ambiguity parking downstream has a
        // stable subject.
        let key_score = i32::try_from(
            crate::import_title_resolution::strip_trailing_year_key(&evidence_match.matched_key)
                .split_whitespace()
                .count(),
        )
        .unwrap_or(i32::MAX / 10)
            * 10;
        titles_per_matched_key
            .entry(evidence_match.matched_key.clone())
            .or_default()
            .insert(candidate.info.title_id.as_str());
        let external_id_agreement = external_id_agreement(
            response_attributes,
            candidate.info.tvdb_id.as_deref(),
            candidate.info.tmdb_id.as_deref(),
            candidate.info.imdb_id.as_deref(),
        );
        if evidence_match.requires_external_id && external_id_agreement != Some(true) {
            continue;
        }
        let disambiguated = candidate_presents_identity_disambiguator(
            &candidate.evidence,
            &CandidateTitleMatch {
                evidence_match: Some(evidence_match),
            },
            external_id_agreement,
        );

        let year_bonus = i32::from(parsed.year.is_some() && parsed.year == candidate.info.year) * 8;
        let parser_bonus = (parsed.parse_confidence * 10.0).round() as i32;
        let score = key_score + year_bonus + parser_bonus;

        // A disambiguated candidate outranks an undisambiguated one outright:
        // an indexer-asserted id or a unique alias names the show, while the
        // score is only a lexical guess that both colliding titles earn equally.
        match best {
            Some((best_info, best_score, best_disambiguated))
                if (disambiguated, score) < (best_disambiguated, best_score)
                    || ((disambiguated, score) == (best_disambiguated, best_score)
                        && candidate.info.title_id >= best_info.title_id) => {}
            _ => best = Some((&candidate.info, score, disambiguated)),
        }
    }

    // Sonarr collision conservatism (Pillar A3): when the same bare key resolves
    // to two different library titles and nothing disambiguates them, the
    // release names neither show. Assigning by score or title-id tiebreak is how
    // the wrong show gets the grab, so skip the release entirely.
    if !best.is_some_and(|(_, _, disambiguated)| disambiguated)
        && let Some((collision_key, colliding_titles)) = titles_per_matched_key
            .iter()
            .find(|(_, title_ids)| title_ids.len() >= 2)
    {
        tracing::debug!(
            release = release_title,
            matched_key = collision_key.as_str(),
            title_count = colliding_titles.len(),
            "RSS sync: skipping release — shared canonical key matches multiple titles with no disambiguator"
        );
        return None;
    }

    best.map(|(info, _, _)| info)
}

#[cfg(test)]
pub(crate) fn parsed_release_matches_title(parsed: &ParsedReleaseMetadata, title: &Title) -> bool {
    parsed_release_matches_title_evidence(parsed, &canonical_title_evidence(title))
}

impl AppUseCase {
    /// Run a single RSS sync cycle: fetch latest releases from all enabled indexers,
    /// match against monitored titles, score, and grab approved releases.
    pub async fn run_rss_sync(&self, actor: &User) -> AppResult<RssSyncReport> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.run_scheduled_rss_sync().await
    }

    pub(crate) async fn run_scheduled_rss_sync(&self) -> AppResult<RssSyncReport> {
        // Process expired pending releases BEFORE evaluating fresh RSS.
        // Ensures delayed releases are grabbed before new RSS results
        // compete for the same wanted item.  Mirrors Sonarr's pattern of
        // including pending releases in the RSS decision cycle.
        match self.process_expired_pending_releases().await {
            Ok(grabbed) if grabbed > 0 => {
                info!(
                    grabbed,
                    "rss sync: promoted expired pending releases before RSS fetch"
                );
            }
            Err(e) => {
                warn!(error = %e, "rss sync: pending release processing failed, continuing with RSS");
            }
            _ => {}
        }

        let now = Utc::now();
        let sync_start = std::time::Instant::now();
        info!("starting RSS sync cycle");

        // Load all monitored titles for matching
        let titles = self
            .services
            .catalog
            .titles
            .list_for_matching(None, None)
            .await?;
        let title_context_bank = build_title_context_bank(&titles);

        if title_context_bank.is_empty() {
            info!("RSS sync: no monitored titles, skipping");
            metrics::counter!("scryer_rss_sync_total").increment(1);
            metrics::histogram!("scryer_rss_sync_duration_seconds")
                .record(sync_start.elapsed().as_secs_f64());
            return Ok(RssSyncReport::default());
        }

        if !super::acquisition_workflow::has_enabled_download_clients(self).await {
            warn!("RSS sync: no enabled download clients configured, skipping indexer search");
            metrics::counter!("scryer_rss_sync_total").increment(1);
            metrics::histogram!("scryer_rss_sync_duration_seconds")
                .record(sync_start.elapsed().as_secs_f64());
            return Ok(RssSyncReport::default());
        }

        // Collect Newznab categories from indexer routing config across all facets.
        // These tell Newznab plugins which categories to fetch in RSS mode.
        let rss_categories = {
            let mut cats: std::collections::HashSet<String> = std::collections::HashSet::new();
            for scope in &["movie", "series", "anime"] {
                if let Some(plan) = self.resolve_indexer_routing(None, Some(scope)).await {
                    for entry in plan.entries.values() {
                        if entry.enabled {
                            for cat in rss_categories_for_routing_entry(scope, entry) {
                                cats.insert(cat);
                            }
                        }
                    }
                }
            }
            if cats.is_empty() {
                None
            } else {
                let sorted: Vec<String> = {
                    let mut v: Vec<_> = cats.into_iter().collect();
                    v.sort();
                    v
                };
                info!(categories = ?sorted, "RSS sync: resolved categories from routing config");
                Some(sorted)
            }
        };

        // Fetch RSS feed (empty query = latest releases) from all indexers
        let rss_results = self
            .services
            .integrations
            .indexer_client
            .search(
                String::new(), // empty query = RSS feed
                HashMap::new(),
                None, // no category filter
                None, // no facet hint
                None, // no ID-search facet override
                rss_categories,
                None, // no routing filter
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
                None,
                tokio_util::sync::CancellationToken::new(),
            )
            .await;

        let response = match rss_results {
            Ok(r) => r,
            Err(err) => {
                warn!(error = %err, "RSS sync: failed to fetch RSS feed from indexers");
                metrics::counter!("scryer_rss_sync_total").increment(1);
                metrics::histogram!("scryer_rss_sync_duration_seconds")
                    .record(sync_start.elapsed().as_secs_f64());
                return Ok(RssSyncReport::default());
            }
        };

        if response.results.is_empty() {
            info!("RSS sync: no results from indexers");
            metrics::counter!("scryer_rss_sync_total").increment(1);
            metrics::histogram!("scryer_rss_sync_duration_seconds")
                .record(sync_start.elapsed().as_secs_f64());
            return Ok(RssSyncReport::default());
        }

        info!(
            result_count = response.results.len(),
            "RSS sync: fetched releases from indexers"
        );

        // Dedup against previously seen GUIDs (in-memory, resets on restart)
        let mut seen_guids = self.runtime.acquisition.rss_seen_guids.write().await;
        let initial_seen_count = seen_guids.len();

        let mut new_results: Vec<IndexerSearchResult> = Vec::new();
        for result in response.results {
            let guid = result
                .guid
                .as_deref()
                .or(result.download_url.as_deref())
                .or(result.link.as_deref())
                .unwrap_or(&result.title);

            if seen_guids.insert(guid.to_string()) {
                new_results.push(result);
            }
        }

        // Cap the seen set to prevent unbounded growth
        if seen_guids.len() > RSS_SYNC_MAX_GUIDS {
            let excess = seen_guids.len() - RSS_SYNC_MAX_GUIDS;
            let to_remove: Vec<String> = seen_guids.iter().take(excess).cloned().collect();
            for key in to_remove {
                seen_guids.remove(&key);
            }
        }

        // Release the write lock before doing any I/O
        drop(seen_guids);

        info!(
            new_count = new_results.len(),
            previously_seen = initial_seen_count,
            "RSS sync: filtered to new releases"
        );

        if new_results.is_empty() {
            metrics::counter!("scryer_rss_sync_total").increment(1);
            metrics::histogram!("scryer_rss_sync_duration_seconds")
                .record(sync_start.elapsed().as_secs_f64());
            return Ok(RssSyncReport::default());
        }

        // Parse each release and match against monitored titles
        let mut matched_by_title: HashMap<String, Vec<IndexerSearchResult>> = HashMap::new();
        let mut matched_count = 0usize;
        let total_new = new_results.len();

        for result in new_results {
            if let Some(title_info) = match_release_to_title_context(
                &result.title,
                &result.response_attributes,
                &title_context_bank,
            ) {
                matched_count += 1;
                matched_by_title
                    .entry(title_info.title_id.clone())
                    .or_default()
                    .push(result);
            }
        }

        info!(
            matched = matched_count,
            titles_matched = matched_by_title.len(),
            "RSS sync: matched releases to monitored titles"
        );

        // Snapshot download client state
        let dl_snapshot = super::acquisition_workflow::DownloadClientSnapshot::fetch(self).await;
        let delay_profiles = self.load_delay_profiles().await;
        let mut grabbed_urls: HashSet<String> = HashSet::new();
        let mut report = RssSyncReport {
            releases_fetched: total_new,
            releases_matched: matched_count,
            ..Default::default()
        };

        // For each matched title, score and potentially grab. By design,
        // target-ness is derived from library state at match time — a monitored
        // scope that is missing or below cutoff — never gated on a pre-existing
        // wanted row. The activity ledger row, when present, still supplies
        // upgrade state and is created on the first anchored write.
        for (title_id, releases) in &matched_by_title {
            let title = match self.services.catalog.titles.get_by_id(title_id).await {
                Ok(Some(t)) => t,
                _ => continue,
            };
            if !title.monitored {
                continue;
            }

            // For series, we need to match individual episodes
            let has_episodes = self
                .facet_registry
                .get(&title.facet)
                .map(|h| h.has_episodes())
                .unwrap_or(false);

            if has_episodes {
                // For series: route each release to its covered episode(s) or
                // pack, gated on per-scope monitoring + missing/below-cutoff.
                self.process_rss_series_releases(
                    &title,
                    releases,
                    &dl_snapshot,
                    &delay_profiles,
                    &mut grabbed_urls,
                    &mut report,
                    &now,
                )
                .await;
                self.process_rss_series_movie_releases(
                    &title,
                    releases,
                    &dl_snapshot,
                    &delay_profiles,
                    &mut grabbed_urls,
                    &mut report,
                    &now,
                )
                .await;
            } else {
                // For movies: the monitored title is a target while it has no
                // primary file or sits below cutoff. Availability gates active
                // grabs the same way the derived movie target set does (§D1).
                if !super::targets::movie_is_available_for_acquisition(
                    title.first_aired.as_deref(),
                    title.digital_release_date.as_deref(),
                    title.min_availability.as_deref().unwrap_or("announced"),
                    &now,
                ) {
                    continue;
                }
                self.process_rss_title_releases(
                    &title,
                    releases,
                    &dl_snapshot,
                    &delay_profiles,
                    &mut grabbed_urls,
                    &mut report,
                    &now,
                )
                .await;
            }
        }

        info!(
            fetched = report.releases_fetched,
            matched = report.releases_matched,
            grabbed = report.releases_grabbed,
            held = report.releases_held,
            "RSS sync cycle completed"
        );

        metrics::counter!("scryer_rss_sync_total").increment(1);
        metrics::histogram!("scryer_rss_sync_duration_seconds")
            .record(sync_start.elapsed().as_secs_f64());
        metrics::counter!("scryer_rss_releases_fetched_total")
            .increment(report.releases_fetched as u64);
        metrics::counter!("scryer_rss_releases_matched_total")
            .increment(report.releases_matched as u64);
        metrics::counter!("scryer_rss_releases_grabbed_total")
            .increment(report.releases_grabbed as u64);

        Ok(report)
    }

    /// Process RSS releases matched to a movie title. Target-ness (§D5) is the
    /// monitored title being missing or below cutoff; the state row, if any,
    /// only supplies upgrade/pause state.
    #[expect(
        clippy::too_many_arguments,
        reason = "RSS movie processing threads grab state, timing, and scoring context together"
    )]
    async fn process_rss_title_releases(
        &self,
        title: &Title,
        releases: &[IndexerSearchResult],
        dl_snapshot: &super::acquisition_workflow::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        // The activity ledger row is optional: when absent, an unpersisted view
        // stands in and is materialized on the first anchored write.
        let wanted = match self
            .find_wanted_state_for_scope(&title.id, None, None, None)
            .await
        {
            Ok(Some(existing)) => existing,
            Ok(None) => self.new_wanted_state_view(title, "movie", None, None, None, None),
            Err(err) => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %err,
                    "RSS sync: failed to load movie acquisition state"
                );
                return;
            }
        };
        // A user pause is honored even against a monitored missing scope (§D5).
        if wanted.status == AcquisitionScopeStatus::Paused {
            return;
        }
        let category = self
            .facet_registry
            .get(&title.facet)
            .map(|h| h.search_category().to_string())
            .unwrap_or_else(|| "movie".to_string());

        let tvdb_id = title
            .external_ids
            .iter()
            .find(|id| id.source == "tvdb")
            .map(|id| id.value.clone());
        let parse_context = build_release_parse_context(title, None, None, Some(category.as_str()));

        // Score all releases against quality profile
        let scored = match self
            .score_rss_releases(
                releases,
                &title.id,
                &title.library_id,
                title.imdb_id.clone(),
                tvdb_id.clone(),
                Some(category.clone()),
                &title.tags,
                title.runtime_minutes,
                &parse_context,
                None,
                None,
                None,
            )
            .await
        {
            Ok(s) => s,
            Err(err) => {
                warn!(
                    title = title.name.as_str(),
                    error = %err,
                    "RSS sync: failed to score releases"
                );
                return;
            }
        };

        // Try to grab the best candidate using the same logic as acquisition
        self.try_grab_rss_release(
            title,
            &wanted,
            &scored,
            &category,
            dl_snapshot,
            delay_profiles,
            grabbed_urls,
            report,
            now,
        )
        .await;
    }

    /// Process RSS releases matched to a series title. Single-episode postings
    /// converge per episode; multi-episode/season packs converge once at pack
    /// granularity, never fanned out to per-episode rows.
    #[expect(
        clippy::too_many_arguments,
        reason = "RSS series processing carries per-episode routing state through one workflow step"
    )]
    async fn process_rss_series_releases(
        &self,
        title: &Title,
        releases: &[IndexerSearchResult],
        dl_snapshot: &super::acquisition_workflow::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        let category = self
            .facet_registry
            .get(&title.facet)
            .map(|h| h.search_category().to_string())
            .unwrap_or_else(|| "series".to_string());

        let tvdb_id = title
            .external_ids
            .iter()
            .find(|id| id.source == "tvdb")
            .map(|id| id.value.clone());
        let title_parse_context =
            build_release_parse_context(title, None, None, Some(category.as_str()));

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
        let episodes_by_id = catalog_episodes
            .iter()
            .map(|episode| (episode.id.clone(), episode.clone()))
            .collect::<HashMap<_, _>>();
        let monitored_collection_ids = catalog_collections
            .iter()
            .filter(|collection| collection.monitored)
            .map(|collection| collection.id.clone())
            .collect::<HashSet<_>>();

        // Route single-episode postings per episode; keep pack items (absolute
        // ranges and season packs) whole so each pack is evaluated once.
        let mut by_episode: HashMap<String, Vec<IndexerSearchResult>> = HashMap::new();
        let mut pack_items: Vec<(
            crate::acquisition_coverage::ReleaseCoverage,
            IndexerSearchResult,
        )> = Vec::new();
        let mut seen_pack_keys: HashSet<String> = HashSet::new();
        for release in releases {
            let parsed = parse_release_metadata_for_target(&release.title, &title_parse_context);
            let coverage = crate::acquisition_coverage::resolve_release_coverage(
                &parsed,
                &catalog_episodes,
                &catalog_collections,
                None,
            );
            match &coverage {
                crate::acquisition_coverage::ReleaseCoverage::SingleEpisode(episode_id) => {
                    by_episode
                        .entry(episode_id.clone())
                        .or_default()
                        .push(release.clone());
                }
                crate::acquisition_coverage::ReleaseCoverage::EpisodeSet(episode_ids) => {
                    let pack_key = format!("set:{}", episode_ids.join(","));
                    if seen_pack_keys.insert(pack_key) {
                        pack_items.push((coverage, release.clone()));
                    }
                }
                crate::acquisition_coverage::ReleaseCoverage::Collection(collection_id) => {
                    let pack_key = format!("collection:{collection_id}");
                    if seen_pack_keys.insert(pack_key) {
                        pack_items.push((coverage, release.clone()));
                    }
                }
                crate::acquisition_coverage::ReleaseCoverage::Title
                | crate::acquisition_coverage::ReleaseCoverage::Unknown => {}
            }
        }

        for (episode_id, episode_releases) in &by_episode {
            let Some(episode_record) = episodes_by_id.get(episode_id).cloned() else {
                continue;
            };
            // Target-ness (§D5): the episode and its owning collection must be
            // monitored. Missing vs below-cutoff vs satisfied is decided by the
            // cutoff/upgrade gate in `try_grab_rss_release`. The state row is
            // optional and, when present, only supplies upgrade/pause state.
            if !self.rss_episode_scope_is_target(&episode_record, &monitored_collection_ids) {
                continue;
            }
            let wanted = match self
                .find_wanted_state_for_scope(&title.id, Some(episode_id), None, None)
                .await
            {
                Ok(Some(existing)) if existing.status == AcquisitionScopeStatus::Paused => continue,
                Ok(Some(existing)) => existing,
                Ok(None) => self.new_wanted_state_view(
                    title,
                    "episode",
                    Some(episode_id.clone()),
                    episode_record.collection_id.clone(),
                    None,
                    episode_record.season_number.clone(),
                ),
                Err(_) => continue,
            };

            let episode_parse_context = build_release_parse_context(
                title,
                Some(&episode_record),
                None,
                Some(category.as_str()),
            );
            let absolute_episode = episode_record
                .absolute_number
                .as_deref()
                .and_then(|value| value.trim().parse::<u32>().ok());

            // Score these releases
            let owned_releases: Vec<IndexerSearchResult> = episode_releases.to_vec();
            let scored = match self
                .score_rss_releases(
                    &owned_releases,
                    &title.id,
                    &title.library_id,
                    title.imdb_id.clone(),
                    tvdb_id.clone(),
                    Some(category.clone()),
                    &title.tags,
                    title.runtime_minutes,
                    &episode_parse_context,
                    episode_record
                        .season_number
                        .as_deref()
                        .and_then(|value| value.parse::<u32>().ok()),
                    episode_record
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse::<u32>().ok()),
                    absolute_episode,
                )
                .await
            {
                Ok(s) => s,
                Err(_) => continue,
            };

            self.try_grab_rss_release(
                title,
                &wanted,
                &scored,
                &category,
                dl_snapshot,
                delay_profiles,
                grabbed_urls,
                report,
                now,
            )
            .await;
        }

        for (coverage, release) in pack_items {
            self.process_rss_pack_release(
                title,
                &coverage,
                &release,
                &category,
                tvdb_id.as_deref(),
                &catalog_episodes,
                &monitored_collection_ids,
                dl_snapshot,
                delay_profiles,
                grabbed_urls,
                report,
                now,
            )
            .await;
        }
    }

    /// Whether a monitored episode scope is a live RSS target: the episode and
    /// Whether an episode scope is a monitorable RSS target (§D5): the episode
    /// and its owning collection are monitored. Missing vs below-cutoff vs
    /// satisfied is decided downstream — the cutoff early-return inside
    /// `try_grab_rss_release` is authoritative for "satisfied → skip", so a
    /// below-cutoff episode with a file still flows through for upgrade.
    fn rss_episode_scope_is_target(
        &self,
        episode: &Episode,
        monitored_collection_ids: &HashSet<String>,
    ) -> bool {
        if !episode.monitored {
            return false;
        }
        episode
            .collection_id
            .as_deref()
            .is_none_or(|collection_id| monitored_collection_ids.contains(collection_id))
    }

    /// Evaluate one multi-episode/season pack once at pack granularity (§D5 #3).
    /// A pack is a target when ≥1 monitored member episode is missing or below
    /// cutoff and it is not dominated (every member already has a file scoring at
    /// least the pack). The grab anchors to one monitored member's state row and
    /// submits with the pack submission scope and `season_pack: true`.
    #[expect(
        clippy::too_many_arguments,
        reason = "RSS pack evaluation threads catalog, coverage, and grab state through one step"
    )]
    async fn process_rss_pack_release(
        &self,
        title: &Title,
        coverage: &crate::acquisition_coverage::ReleaseCoverage,
        release: &IndexerSearchResult,
        category: &str,
        tvdb_id: Option<&str>,
        catalog_episodes: &[Episode],
        monitored_collection_ids: &HashSet<String>,
        dl_snapshot: &super::acquisition_workflow::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        // Monitored member episodes covered by this pack. A pack with no
        // monitored member is not a target for this title.
        let monitored_members = catalog_episodes
            .iter()
            .filter(|episode| coverage.covers_episode(episode))
            .filter(|episode| episode.monitored)
            .filter(|episode| {
                episode
                    .collection_id
                    .as_deref()
                    .is_none_or(|collection_id| monitored_collection_ids.contains(collection_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        let Some(anchor) = monitored_members.first().cloned() else {
            return;
        };

        // Anchor the grab to the first monitored member's state row (optional;
        // created on the first anchored write). A paused anchor blocks the pack.
        let wanted = match self
            .find_wanted_state_for_scope(&title.id, Some(&anchor.id), None, None)
            .await
        {
            Ok(Some(existing)) if existing.status == AcquisitionScopeStatus::Paused => return,
            Ok(Some(existing)) => existing,
            Ok(None) => self.new_wanted_state_view(
                title,
                "episode",
                Some(anchor.id.clone()),
                anchor.collection_id.clone(),
                None,
                anchor.season_number.clone(),
            ),
            Err(_) => return,
        };

        // Parse the pack at title level (no episode anchor): anchoring to a
        // single member would re-classify a season pack as that one episode and
        // collapse the pack scope to a single-episode submission (§D5 #3).
        let pack_parse_context = build_release_parse_context(title, None, None, Some(category));
        let season = anchor
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok());
        let scored = match self
            .score_rss_releases(
                std::slice::from_ref(release),
                &title.id,
                &title.library_id,
                title.imdb_id.clone(),
                tvdb_id.map(str::to_string),
                Some(category.to_string()),
                &title.tags,
                title.runtime_minutes,
                &pack_parse_context,
                season,
                None,
                None,
            )
            .await
        {
            Ok(scored) => scored,
            Err(_) => return,
        };

        // Pack upgrade guard (mirrors the task-runner pack-dominated check): the
        // pack is dominated — pure waste — when every monitored member already
        // has a primary file scoring at least the pack's score. Only skip when we
        // can see the pack's score; an unscored pack falls through to the grab
        // path, which applies the per-scope cutoff/upgrade gates.
        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let episode_file_scores: HashMap<String, i32> = existing_files
            .iter()
            .filter(|file| file.role.is_primary())
            .filter_map(|file| {
                file.episode_id
                    .as_ref()
                    .zip(file.acquisition_score)
                    .map(|(episode_id, score)| (episode_id.clone(), score))
            })
            .collect();
        if let Some(pack_score) = scored.first().and_then(|candidate| {
            candidate
                .quality_profile_decision
                .as_ref()
                .map(|decision| decision.preference_score)
        }) {
            let benefits = monitored_members.iter().any(|episode| {
                episode_file_scores
                    .get(&episode.id)
                    .map(|existing| pack_score > *existing)
                    .unwrap_or(true) // no file → member benefits
            });
            if !benefits {
                info!(
                    title = title.name.as_str(),
                    release = release.title.as_str(),
                    "RSS sync: pack skipped, every monitored member already has an equal or better file"
                );
                return;
            }
        }

        // The submission scope + season_pack flag are derived from the winning
        // release's coverage inside the grab path, so no per-episode fan-out and
        // no duplicate submission of the same pack.
        self.try_grab_rss_release(
            title,
            &wanted,
            &scored,
            category,
            dl_snapshot,
            delay_profiles,
            grabbed_urls,
            report,
            now,
        )
        .await;
    }

    /// Process RSS releases matched to series-owned movies. Target-ness (§D5) is
    /// a monitored link that is missing or below cutoff; the state row, if any,
    /// only supplies upgrade/pause state.
    #[expect(
        clippy::too_many_arguments,
        reason = "RSS series-movie processing carries wanted, scoring, and queue state together"
    )]
    async fn process_rss_series_movie_releases(
        &self,
        title: &Title,
        releases: &[IndexerSearchResult],
        dl_snapshot: &super::acquisition_workflow::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        let links = match self
            .services
            .catalog
            .shows
            .list_series_movie_links_for_title(&title.id)
            .await
        {
            Ok(links) => links,
            Err(err) => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %err,
                    "RSS sync: failed to load series-movie links"
                );
                return;
            }
        };

        for link in links {
            // Target-ness (§D5): a monitored link. Missing vs below-cutoff vs
            // satisfied is decided by the cutoff/upgrade gate downstream.
            if !link.monitored {
                continue;
            }
            let wanted = match self
                .find_wanted_state_for_scope(&title.id, None, None, Some(&link.id))
                .await
            {
                Ok(Some(existing)) if existing.status == AcquisitionScopeStatus::Paused => continue,
                Ok(Some(existing)) => existing,
                Ok(None) => self.new_wanted_state_view(
                    title,
                    "series_movie",
                    None,
                    None,
                    Some(link.id.clone()),
                    Some("0".to_string()),
                ),
                Err(_) => continue,
            };

            let search_title = series_movie_search_title(title, &link);
            let subject = self
                .resolve_release_search_subject_for_wanted_item(title, &search_title, &wanted, None)
                .await;
            let matched_releases = releases
                .iter()
                .filter(|release| {
                    let parsed = parse_release_metadata_for_target(
                        &release.title,
                        &subject.title_evidence.parse_context,
                    );
                    if let (Some(parsed_year), Some(title_year)) =
                        (parsed.year, subject.title_evidence.year)
                        && parsed_year != title_year
                    {
                        return false;
                    }
                    parsed_release_matches_title_evidence(&parsed, &subject.title_evidence)
                })
                .cloned()
                .collect::<Vec<_>>();

            if matched_releases.is_empty() {
                continue;
            }

            let quality_profile_lookup = crate::app_usecase_discovery::QualityProfileLookup {
                title_tags: &subject.title_tags,
                library_id: Some(title.library_id.as_str()),
                imdb_id: subject.imdb_id.as_deref(),
                tvdb_id: subject.tvdb_id.as_deref(),
                category_hint: Some(subject.owner_facet.as_str()),
            };
            let quality_profile = match self.resolve_quality_profile(quality_profile_lookup).await {
                Ok(profile) => profile,
                Err(err) => {
                    warn!(
                        title_id = title.id.as_str(),
                        series_movie_link_id = link.id.as_str(),
                        error = %err,
                        "RSS sync: failed to resolve series-movie quality profile"
                    );
                    continue;
                }
            };
            let scope_id = self.quality_profile_scope_id(quality_profile_lookup);
            let indexer_routing = self
                .resolve_indexer_routing(Some(title.library_id.as_str()), scope_id.as_deref())
                .await;
            let scored = self
                .score_release_results(
                    matched_releases,
                    &quality_profile,
                    &subject.title_id,
                    Some(title.library_id.as_str()),
                    scope_id.as_deref(),
                    indexer_routing.as_ref(),
                    Some(subject.category.as_str()),
                    &subject.title_tags,
                    subject.runtime_minutes,
                    &subject.title_evidence.parse_context,
                    subject.season,
                    subject.episode,
                    subject.absolute_episode,
                )
                .await;

            self.try_grab_rss_release(
                title,
                &wanted,
                &scored,
                &subject.category,
                dl_snapshot,
                delay_profiles,
                grabbed_urls,
                report,
                now,
            )
            .await;
        }
    }

    /// Score a batch of RSS releases against the quality profile.
    #[expect(
        clippy::too_many_arguments,
        reason = "RSS scoring needs the full release and title context to match interactive search behavior"
    )]
    async fn score_rss_releases(
        &self,
        releases: &[IndexerSearchResult],
        title_id: &str,
        library_id: &str,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
        title_tags: &[String],
        runtime_minutes: Option<i32>,
        parse_context: &ReleaseParseContext,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let quality_profile_lookup = crate::app_usecase_discovery::QualityProfileLookup {
            title_tags,
            library_id: Some(library_id),
            imdb_id: imdb_id.as_deref(),
            tvdb_id: tvdb_id.as_deref(),
            category_hint: category.as_deref(),
        };
        let quality_profile = self.resolve_quality_profile(quality_profile_lookup).await?;
        let scope_id = self.quality_profile_scope_id(quality_profile_lookup);
        let indexer_routing = self
            .resolve_indexer_routing(Some(library_id), scope_id.as_deref())
            .await;

        Ok(self
            .score_release_results(
                releases.to_vec(),
                &quality_profile,
                title_id,
                Some(library_id),
                scope_id.as_deref(),
                indexer_routing.as_ref(),
                category.as_deref(),
                title_tags,
                runtime_minutes,
                parse_context,
                season,
                episode,
                absolute_episode,
            )
            .await)
    }

    /// Try to grab the best candidate from scored RSS releases.
    /// Reuses the same logic as process_single_wanted_item for consistency.
    ///
    /// `wanted` may be an unpersisted state view: the scope's
    /// ledger row is materialized via `ensure_acquisition_scope_state` before the first
    /// anchored write (release decision, pending release, grab), and every FK
    /// write uses the persisted id returned by it.
    #[expect(
        clippy::too_many_arguments,
        reason = "RSS grab attempts coordinate release state, client state, and reporting in one place"
    )]
    async fn try_grab_rss_release(
        &self,
        title: &Title,
        wanted: &AcquisitionScopeState,
        scored: &[IndexerSearchResult],
        _category: &str,
        dl_snapshot: &super::acquisition_workflow::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        let mut wanted = wanted.clone();
        // `DbBlocklisted` reads the per-title blocklist (the single, removable
        // exclusion source), never the failed-attempt history.
        let db_blocklist = self
            .load_title_release_blocklist_signatures(&title.id)
            .await
            .source_titles;
        let episode = match wanted.episode_id.as_deref() {
            Some(episode_id) => self
                .services
                .catalog
                .shows
                .get_episode_by_id(episode_id)
                .await
                .ok()
                .flatten(),
            None => None,
        };
        let search_title = self
            .release_search_title_for_wanted_item(title, &wanted, episode.as_ref())
            .await;
        let subject = self
            .resolve_release_search_subject_for_wanted_item(
                title,
                &search_title,
                &wanted,
                episode.as_ref(),
            )
            .await;
        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|file| file.role.is_primary())
            .collect::<Vec<_>>();
        let analyzed_cutoff_quality =
            crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
                &existing_files,
                subject.submission_scope.episode_id(),
                subject.submission_scope.series_movie_link_id(),
            );
        let upgrade_context = match self
            .resolve_upgrade_context_for_title_with_category_and_quality(
                title,
                wanted.grabbed_release.as_deref(),
                Some(subject.owner_facet.as_str()),
                analyzed_cutoff_quality,
            )
            .await
        {
            Ok(context) => context,
            Err(error) => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %error,
                    "RSS grab: failed to resolve quality profile; skipping scope"
                );
                return;
            }
        };
        if upgrade_context.cutoff_reached {
            return;
        }

        // The scope is a genuine target from here on, so its ledger row exists
        // before the first anchored write (§D5). An existing row's id is reused;
        // an unpersisted view is materialized.
        match self
            .services
            .workflow
            .acquisition_scope_states
            .ensure_acquisition_scope_state(&wanted)
            .await
        {
            Ok(id) => wanted.id = id,
            Err(err) => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %err,
                    "RSS sync: failed to materialize acquisition state row"
                );
                return;
            }
        }

        // Hoisted above the loop on purpose: the evaluation context is rebuilt
        // per candidate below, and resolving thresholds inside it would repeat
        // the same repository reads for every release in the feed.
        let minimum_seeders = self.minimum_seeders_for_candidates(scored).await;
        let mut selected: Option<&IndexerSearchResult> = None;

        for candidate in scored {
            let is_allowed = candidate
                .quality_profile_decision
                .as_ref()
                .map(|d| d.allowed)
                .unwrap_or(false);
            if !is_allowed {
                continue;
            }

            if dl_snapshot.is_active(&candidate.title) {
                continue;
            }
            if dl_snapshot.failed_item(None, &candidate.title).is_some() {
                continue;
            }

            let evaluation_context = AutoCandidateEvaluationContext {
                title,
                subject: &subject,
                current_score: wanted.current_score,
                last_search_at: wanted.last_search_at.as_deref(),
                profile: &upgrade_context.profile,
                thresholds: &upgrade_context.thresholds,
                cutoff_reached: upgrade_context.cutoff_reached,
                now,
                dl_snapshot: Some(dl_snapshot),
                db_blocklist: &db_blocklist,
                existing_files: &existing_files,
                delay_profiles,
                failed_routes: None,
                minimum_seeders: &minimum_seeders,
            };
            let decision_code = evaluate_auto_candidate(candidate, &evaluation_context);
            let candidate_score = candidate
                .quality_profile_decision
                .as_ref()
                .map(|d| d.preference_score)
                .unwrap_or(0);
            let mut decision_candidate = candidate.clone();
            annotate_auto_decision(&mut decision_candidate, decision_code);

            let decision_record = ReleaseDecision {
                id: Id::new().0,
                wanted_item_id: wanted.id.clone(),
                title_id: title.id.clone(),
                release_title: decision_candidate.title.clone(),
                release_url: decision_candidate
                    .canonical_download_source()
                    .map(|(source, _)| source),
                release_size_bytes: decision_candidate.size_bytes,
                decision_code: decision_code.as_str().to_string(),
                candidate_score,
                current_score: wanted.current_score,
                score_delta: wanted.current_score.map(|c| candidate_score - c),
                explanation_json: serialize_decision_explanation(&decision_candidate),
                created_at: now.to_rfc3339(),
            };

            let _ = self
                .services
                .workflow
                .acquisition_scope_states
                .insert_release_decision(&decision_record)
                .await;

            if matches!(decision_code, ReleaseAutoDecisionCode::PendingDelay) {
                let delay_minutes = crate::delay_profile::resolve_delay_decision(
                    delay_profiles,
                    &title.tags,
                    &title.facet,
                    candidate.source_kind,
                    candidate
                        .published_at
                        .as_deref()
                        .and_then(crate::quality_profile::parse_published_at),
                    candidate_score,
                    now,
                )
                .map(|delay| delay.effective_delay_minutes)
                .unwrap_or_default();
                let canonical_source = candidate.canonical_download_source();
                self.insert_pending_release(
                    &wanted,
                    title,
                    &candidate.title,
                    canonical_source.as_ref().map(|(source, _)| source.as_str()),
                    canonical_source
                        .as_ref()
                        .map(|(_, kind)| *kind)
                        .or(candidate.source_kind),
                    candidate.size_bytes,
                    candidate_score,
                    serialize_decision_explanation(&decision_candidate),
                    Some(candidate.source.as_str()),
                    candidate.indexer_id.as_deref(),
                    candidate.guid.as_deref(),
                    delay_minutes,
                    candidate.password_hint.as_deref(),
                    candidate.published_at.as_deref(),
                    candidate.extra.get("info_hash").and_then(|v| v.as_str()),
                    crate::ReleaseSeedMinimums::from_release_extra(&candidate.extra),
                    crate::acquisition::seed_goals::seeders_from_extra(&candidate.extra),
                )
                .await;
                report.releases_held += 1;
                return;
            }

            if decision_code.is_eligible() {
                selected = Some(candidate);
                break;
            }
        }

        let Some(best) = selected else {
            return;
        };

        let candidate_score = best
            .quality_profile_decision
            .as_ref()
            .map(|d| d.preference_score)
            .unwrap_or(0);

        let canonical_source = best.canonical_download_source();
        let source_hint = canonical_source.as_ref().map(|(source, _)| source.clone());
        let canonical_source_kind = canonical_source
            .as_ref()
            .map(|(_, kind)| *kind)
            .or(best.source_kind);
        if let Some(url) = source_hint.as_deref()
            && !grabbed_urls.insert(url.to_string())
        {
            return;
        }

        let source_title = Some(best.title.clone());
        let source_hint_for_attempt = normalize_release_attempt_hint(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_title(source_title.as_deref());
        let source_password = normalize_release_password(best.password_hint.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint.as_deref(),
            source_title.as_deref(),
            canonical_source_kind,
        );

        let _ = self
            .services
            .workflow
            .release_attempts
            .record_release_attempt(
                Some(title.id.clone()),
                source_hint_for_attempt.clone(),
                source_title_for_attempt.clone(),
                ReleaseDownloadAttemptOutcome::Pending,
                None,
                source_password.clone(),
            )
            .await;

        let download_cat = self.derive_download_category(&title.facet).await;
        let is_recent = self.is_recent_for_queue_priority(
            best.published_at
                .as_deref()
                .or(title.first_aired.as_deref())
                .or(title.digital_release_date.as_deref()),
        );

        info!(
            title = title.name.as_str(),
            release = best.title.as_str(),
            score = candidate_score,
            "RSS sync: auto-grabbing release"
        );

        let info_hash_hint = best
            .extra
            .get("info_hash")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let seed_minimums = crate::ReleaseSeedMinimums::from_release_extra(&best.extra);
        let download_id = crate::download_identity::new_download_id();
        let submission_identity = DownloadSubmissionIdentity {
            download_id: Some(download_id.clone()),
        };

        // Resolve the submission scope from the winning release's coverage before
        // submitting: a multi-episode/season pack grabs once with the pack scope
        // and `season_pack: true` (§D5 #3), never per member episode.
        let submission_scope = if let Some(parsed) = best.parsed_release_metadata.as_ref() {
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
            crate::acquisition_coverage::resolve_release_coverage(
                parsed,
                &catalog_episodes,
                &catalog_collections,
                episode.as_ref(),
            )
            .submission_scope_or(&subject.submission_scope)
        } else {
            super::acquisition::download_submission_scope_for_release_title(
                &wanted,
                episode.as_ref(),
                &best.title,
            )
        };
        let is_season_pack = matches!(
            submission_scope,
            SubmissionScope::EpisodeSet { .. } | SubmissionScope::Collection { .. }
        );
        let mut grabbed_episode_ids = match &submission_scope {
            SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
            SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
            SubmissionScope::Collection { collection_id } => self
                .services
                .catalog
                .shows
                .list_episodes_for_collection(collection_id)
                .await
                .map(|episodes| episodes.into_iter().map(|episode| episode.id).collect())
                .unwrap_or_default(),
            SubmissionScope::Title
            | SubmissionScope::SeriesMovie { .. }
            | SubmissionScope::Orphan => Vec::new(),
        };
        grabbed_episode_ids.sort();
        grabbed_episode_ids.dedup();
        // Blocklist attribution for a definitive failure below; captured before
        // the submission scope moves into the persisted download submission.
        let blocklist_data = blocklist_entry_data(
            &grabbed_episode_ids,
            submission_scope.collection_id(),
            submission_scope.series_movie_link_id(),
        );

        let grab_result = self
            .services
            .integrations
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                search_facet: Some(subject.search_facet.clone()),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                download_id: Some(download_id),
                source_hint: source_hint.clone(),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: canonical_source_kind,
                source_title: source_title.clone(),
                source_password: source_password.clone(),
                category: Some(download_cat),
                queue_priority: None,
                download_directory: None,
                release_title: Some(best.title.clone()),
                indexer_name: Some(best.source.clone()),
                indexer_id: best.indexer_id.clone(),
                info_hash_hint: info_hash_hint.clone(),
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                tracker_min_seed_ratio: seed_minimums.min_seed_ratio,
                tracker_min_seed_time_minutes: seed_minimums.min_seed_time_minutes,
                season_pack_seed_ratio: seed_minimums.season_pack_seed_ratio,
                season_pack_seed_time_minutes: seed_minimums.season_pack_seed_time_minutes,
                is_recent,
                season_pack: is_season_pack.then_some(true),
            })
            .await;

        match grab_result {
            Ok(grab) => {
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => best.source.clone(), "facet" => facet_label).increment(1);
                }
                self.record_indexer_grab(best.indexer_id.as_deref(), Some(best.source.as_str()));

                let facet_str =
                    serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
                let accepted_identity =
                    crate::download_identity::accepted_download_submission_identity(
                        crate::download_identity::AcceptedDownloadIdentityInput {
                            initial_download_id: submission_identity.download_id.as_deref(),
                            source_kind: best.source_kind,
                            source_hint: source_hint.as_deref(),
                            info_hash_hint: info_hash_hint.as_deref(),
                            client_type: Some(grab.client_type.as_str()),
                            client_item_id: Some(grab.job_id.as_str()),
                            accepted_info_hash: grab.info_hash.as_deref(),
                        },
                    );
                let log_download_id = accepted_identity.download_id.clone();
                if let Err(error) = self
                    .services
                    .workflow
                    .download_submissions
                    .record_submission_with_identity(
                        DownloadSubmission {
                            title_id: title.id.clone(),
                            purpose: crate::DownloadSubmissionPurpose::Standard,
                            facet: facet_str.trim_matches('"').to_string(),
                            download_client_id: grab.client_id.clone(),
                            download_client_type: grab.client_type.clone(),
                            download_client_item_id: grab.job_id.clone(),
                            source_hint: None,
                            source_provider_id: best.indexer_id.clone(),
                            source_provider_name: Some(best.source.clone()),
                            source_kind: None,
                            source_title: source_title.clone(),
                            request_signature: request_signature.clone(),
                            scope: submission_scope,
                        },
                        accepted_identity,
                    )
                    .await
                {
                    tracing::warn!(
                        error = %error,
                        client_id = ?grab.client_id,
                        client_type = %grab.client_type,
                        download_client_item_id = %grab.job_id,
                        download_id = ?log_download_id,
                        "download_identity_persistence_failed"
                    );
                    let _ = self
                        .services
                        .workflow
                        .release_attempts
                        .record_release_attempt(
                            Some(title.id.clone()),
                            source_hint_for_attempt.clone(),
                            source_title_for_attempt.clone(),
                            ReleaseDownloadAttemptOutcome::Failed,
                            Some(error.to_string()),
                            source_password,
                        )
                        .await;
                    // The client accepted a job Scryer can no longer track, so
                    // burn the release for this title (visible and removable in
                    // the blocklist) rather than re-grabbing it into a duplicate.
                    if let Err(blocklist_error) = self
                        .services
                        .workflow
                        .blocklist_repo
                        .add(&NewBlocklistEntry {
                            title_id: title.id.clone(),
                            source_title: source_title_for_attempt,
                            source_hint: source_hint_for_attempt,
                            quality: None,
                            download_id: None,
                            reason: Some(format!(
                                "grab accepted but download tracking could not be persisted: {error}"
                            )),
                            data: blocklist_data,
                        })
                        .await
                    {
                        warn!(
                            error = %blocklist_error,
                            title_id = title.id.as_str(),
                            release = best.title.as_str(),
                            "failed to persist blocklist entry for untracked RSS grab"
                        );
                    }
                    return;
                }

                let _ = self
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt,
                        source_title_for_attempt,
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        source_password,
                    )
                    .await;

                let grabbed_json = serde_json::json!({
                    "title": best.title,
                    "score": candidate_score,
                    "grabbed_at": now.to_rfc3339(),
                    "source": "rss_sync",
                })
                .to_string();

                let _ = self
                    .services
                    .workflow
                    .acquisition_scope_states
                    .transition_acquisition_scope_to_grabbed(&AcquisitionScopeGrabTransition {
                        id: wanted.id.clone(),
                        last_search_at: Some(now.to_rfc3339()),
                        current_score: Some(candidate_score),
                        grabbed_release: grabbed_json,
                    })
                    .await;

                let _ = self
                    .append_domain_event(new_title_domain_event(
                        None,
                        title,
                        DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                            title: title_context_snapshot(title),
                            source_title: Some(best.title.clone()),
                            source_hint: Some(best.source.clone()),
                            source_provider: Some(best.source.clone()),
                            download_id: None,
                            episode_ids: grabbed_episode_ids,
                        }),
                    ))
                    .await;

                report.releases_grabbed += 1;
            }
            Err(err) => {
                warn!(
                    title = title.name.as_str(),
                    release = best.title.as_str(),
                    error = %err,
                    "RSS sync: download submission failed"
                );

                // Transient (client unavailable) and ambiguous (request may have
                // been accepted, response lost) submits are deferred: Pending
                // attempt, never blocklisted. Only a definitive failure burns
                // the release for this title.
                let defer = is_download_submit_unavailable_error(&err)
                    || err.is_download_submit_ambiguous();
                let _ = self
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        if defer {
                            ReleaseDownloadAttemptOutcome::Pending
                        } else {
                            ReleaseDownloadAttemptOutcome::Failed
                        },
                        Some(err.to_string()),
                        source_password,
                    )
                    .await;
                if defer {
                    return;
                }
                if let Err(error) = self
                    .services
                    .workflow
                    .blocklist_repo
                    .add(&NewBlocklistEntry {
                        title_id: title.id.clone(),
                        source_title: source_title_for_attempt,
                        source_hint: source_hint_for_attempt,
                        quality: None,
                        download_id: None,
                        reason: Some(format!("grab failed: {err}")),
                        data: blocklist_data,
                    })
                    .await
                {
                    warn!(
                        error = %error,
                        title_id = title.id.as_str(),
                        release = best.title.as_str(),
                        "failed to persist blocklist entry for failed RSS grab"
                    );
                }
            }
        }
    }
}

#[derive(Default, Debug)]
pub struct RssSyncReport {
    pub releases_fetched: usize,
    pub releases_matched: usize,
    pub releases_grabbed: usize,
    pub releases_held: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{MediaFacet, Title};

    fn make_title(id: &str, name: &str, year: Option<i32>) -> Title {
        Title {
            id: id.to_string(),
            name: name.to_string(),
            facet: MediaFacet::Movie,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: chrono::Utc::now(),
            year,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
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
            folder_path: None,
        }
    }

    fn make_title_with_aliases(
        id: &str,
        name: &str,
        year: Option<i32>,
        aliases: Vec<&str>,
    ) -> Title {
        let mut t = make_title(id, name, year);
        t.aliases = aliases.into_iter().map(|s| s.to_string()).collect();
        t
    }

    fn make_unmonitored(id: &str, name: &str) -> Title {
        let mut t = make_title(id, name, None);
        t.monitored = false;
        t
    }

    #[test]
    fn rss_categories_expand_empty_routing_entries_to_scope_defaults() {
        let entry = IndexerRoutingEntry {
            enabled: true,
            categories: vec![],
            priority: 0,
        };

        assert_eq!(
            rss_categories_for_routing_entry("movie", &entry),
            vec!["2000"]
        );
        assert_eq!(
            rss_categories_for_routing_entry("series", &entry),
            vec!["5000"]
        );
        assert_eq!(
            rss_categories_for_routing_entry("anime", &entry),
            vec!["5070"]
        );
    }

    #[test]
    fn rss_categories_preserve_explicit_routing_categories() {
        let entry = IndexerRoutingEntry {
            enabled: true,
            categories: vec!["5040".to_string()],
            priority: 0,
        };

        assert_eq!(
            rss_categories_for_routing_entry("series", &entry),
            vec!["5040"]
        );
    }

    // ── normalize_for_matching ──────────────────────────────────────

    #[test]
    fn normalize_basic_title() {
        assert_eq!(
            normalize_for_matching("The Silver Harbor"),
            "the silver harbor"
        );
    }

    #[test]
    fn normalize_dots_and_dashes() {
        assert_eq!(
            normalize_for_matching("The.Silver.Harbor-2008"),
            "the silver harbor 2008"
        );
    }

    #[test]
    fn normalize_underscores() {
        assert_eq!(
            normalize_for_matching("the_silver_harbor"),
            "the silver harbor"
        );
    }

    #[test]
    fn normalize_strips_special_chars() {
        assert_eq!(
            normalize_for_matching("Sky-Rider: Beyond the Silent City"),
            "sky rider beyond the silent city"
        );
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(
            normalize_for_matching("  The   Silver   Harbor  "),
            "the silver harbor"
        );
    }

    #[test]
    fn normalize_empty() {
        assert_eq!(normalize_for_matching(""), "");
    }

    #[test]
    fn normalize_unicode_alphanumeric() {
        // é is alphanumeric in Unicode, so it's preserved
        assert_eq!(normalize_for_matching("café"), "café");
    }

    // ── build_title_context_bank ────────────────────────────────────

    #[test]
    fn context_bank_indexes_by_primary_name() {
        let titles = vec![make_title("t1", "Neon Cipher", Some(2010))];
        let bank = build_title_context_bank(&titles);
        assert_eq!(bank.len(), 1);
        assert_eq!(bank[0].info.title_id, "t1");
        assert!(
            bank[0]
                .evidence
                .lookup_keys
                .iter()
                .any(|key| key == "neon cipher")
        );
    }

    #[test]
    fn context_bank_skips_unmonitored() {
        let titles = vec![make_unmonitored("t1", "Neon Cipher")];
        let bank = build_title_context_bank(&titles);
        assert!(bank.is_empty());
    }

    #[test]
    fn context_bank_indexes_aliases() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "Lantern Tide",
            Some(2001),
            vec!["Lantern Tide: Hidden Current"],
        )];
        let bank = build_title_context_bank(&titles);
        assert_eq!(bank.len(), 1);
        assert!(
            bank[0]
                .evidence
                .lookup_keys
                .iter()
                .any(|key| key == "lantern tide")
        );
        assert!(
            bank[0]
                .evidence
                .lookup_keys
                .iter()
                .any(|key| key == "lantern tide hidden current")
        );
    }

    #[test]
    fn context_bank_keeps_multiple_titles_same_normalized_name() {
        let titles = vec![
            make_title("t1", "Glass Harbor", Some(1984)),
            make_title("t2", "Glass Harbor", Some(2021)),
        ];
        let bank = build_title_context_bank(&titles);
        assert_eq!(bank.len(), 2);
    }

    // ── match_release_to_title_context ──────────────────────────────

    /// Title matching without any indexer id assertion — the shape almost every
    /// matcher test wants. Tests that exercise A2(2) call the real function.
    fn match_release<'a>(
        release_title: &str,
        context_bank: &'a TitleContextBank,
    ) -> Option<&'a TitleMatchInfo> {
        match_release_to_title_context(
            release_title,
            &IndexerResponseAttributes::default(),
            context_bank,
        )
    }

    #[test]
    fn match_exact_title() {
        let titles = vec![make_title("t1", "Neon Cipher", Some(2010))];
        let bank = build_title_context_bank(&titles);
        let result = match_release("Neon.Cipher.2010.1080p.BluRay.x264", &bank);
        assert!(result.is_some(), "exact match should succeed");
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_prefers_year_match() {
        let titles = vec![
            make_title("t1", "Glass Harbor", Some(1984)),
            make_title("t2", "Glass Harbor", Some(2021)),
        ];
        let bank = build_title_context_bank(&titles);
        let result = match_release("Glass.Harbor.2021.1080p.BluRay.x264", &bank);
        assert!(result.is_some(), "result was None");
        assert_eq!(result.unwrap().title_id, "t2");
    }

    #[test]
    fn match_with_year_stripped_from_release() {
        // Release has "Title 2010", lookup only has "Title" (with year in metadata)
        let t = make_title("t1", "Neon Cipher", Some(2010));
        // Name doesn't include the year
        let titles = vec![t];
        let bank = build_title_context_bank(&titles);
        let result = match_release("Neon.Cipher.2010.1080p.BluRay", &bank);
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_release_title_without_year_finds_title_with_year() {
        // Lookup has "title 2024", release only has "title"
        let titles = vec![make_title("t1", "Glass Harbor 2024", Some(2024))];
        let bank = build_title_context_bank(&titles);
        let result = match_release("Glass Harbor", &bank);
        // Should match via the reverse year-addition path
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_no_match_returns_none() {
        let titles = vec![make_title("t1", "Neon Cipher", Some(2010))];
        let bank = build_title_context_bank(&titles);
        let result = match_release("Totally.Unknown.Movie.2024.1080p", &bank);
        assert!(result.is_none());
    }

    #[test]
    fn match_empty_release_title_returns_none() {
        let titles = vec![make_title("t1", "Neon Cipher", Some(2010))];
        let bank = build_title_context_bank(&titles);
        let result = match_release("", &bank);
        assert!(result.is_none());
    }

    #[test]
    fn match_via_alias() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "Lantern Tide",
            Some(2001),
            vec!["Hoshi to Kaze no Shirabe"],
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release("Hoshi.to.Kaze.no.Shirabe", &bank);
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_via_release_aka_title_variant() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "My Lighthouse",
            Some(2020),
            vec!["Mon Phare"],
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release(
            "Mon.Phare.A.K.A.My.Lighthouse.2020.1080p.BluRay.x264-GRP",
            &bank,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_via_release_slash_title_variant() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "My Lighthouse",
            Some(2020),
            vec!["Mon Phare"],
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release(
            "Mon Phare / My Lighthouse 2020 1080p BluRay x264-GRP",
            &bank,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    // ── single-token containment junk (prod regression: title "Pals") ──

    fn make_series_title(id: &str, name: &str, year: Option<i32>) -> Title {
        let mut t = make_title(id, name, year);
        t.facet = MediaFacet::Series;
        t.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
        t
    }

    #[test]
    fn single_token_title_rejects_mid_name_containment_junk() {
        let titles = vec![make_series_title("pals", "Pals", Some(1994))];
        let bank = build_title_context_bank(&titles);
        for junk in [
            "Puppy.Harbor.Days.S02E23E24.Heroes.and.Pals.The.Cookie.Crumbles.1080p.PMTP.WEB-DL.AAC2.0.x264-AndreMor",
            "Wardens.of.the.Shadow.Reef.S01E14.Family.and.Pals.1080p.CR.WEB-DL.MULTi.AAC2.0.H264.Msubs-ToonsHub",
            "Suburban.Dad.S09E05.Why.Cant.We.Be.Pals.1080p.DSNP.WEB-DL.DDP5.1.H.264-AndreMor",
            "Kalou.S01E10.Kalous.Pals.720p.PLUTO.WEB-DL.AAC2.0.x264-AndreMor",
            "ToonsHub.My.Pals.Little.Cousin.Has.A.Grudge.S01E09.1080p.CR.WEB-DL.AAC2.0.H264",
        ] {
            assert!(
                match_release(junk, &bank).is_none(),
                "containment junk must not match single-token title: {junk}"
            );
        }
    }

    #[test]
    fn single_token_title_still_matches_head_anchored_release() {
        let titles = vec![make_series_title("pals", "Pals", Some(1994))];
        let bank = build_title_context_bank(&titles);
        let result = match_release(
            "Pals.S01E10.The.One.With.The.Parrot.1080p.BluRay.x264-GRP",
            &bank,
        );
        assert!(result.is_some(), "head-anchored release must still match");
        assert_eq!(result.unwrap().title_id, "pals");
    }

    #[test]
    fn single_token_title_matches_with_year_corroboration() {
        let titles = vec![make_series_title("pals", "Pals", Some(1994))];
        let bank = build_title_context_bank(&titles);
        let result = match_release("Pals.1994.S02E14.1080p.WEB-DL.x264-GRP", &bank);
        assert!(result.is_some(), "year-corroborated release must match");
    }

    // ── Pillar A3: RSS collision conservatism (Sonarr parity) ────────────────

    /// The incident pair on the RSS path: the live-action `Tide Chart` (2023)
    /// and the anime (1999) share the bare canonical key in one library.
    fn tide_chart_rss_bank() -> TitleContextBank {
        let mut anime = make_series_title("tide-chart-anime", "Tide Chart", Some(1999));
        anime.facet = MediaFacet::Anime;
        build_title_context_bank(&[
            make_series_title("tide-chart-live", "Tide Chart", Some(2023)),
            anime,
        ])
    }

    #[test]
    fn shared_bare_key_collision_skips_release_without_disambiguator() {
        // Two library titles answer to the same bare key and nothing separates
        // them, so assigning by score/title-id tiebreak would be a coin flip.
        let bank = tide_chart_rss_bank();
        assert!(
            match_release("Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP", &bank).is_none(),
            "colliding bare key with no disambiguator must skip the release"
        );
    }

    #[test]
    fn shared_bare_key_collision_assigns_when_a_response_id_disambiguates() {
        // A2(2) on the RSS lane: the release name is the same coin flip, but the
        // indexer asserted the live-action title's own TVDB id.
        let mut live_action = make_series_title("tide-chart-live", "Tide Chart", Some(2023));
        live_action.external_ids = vec![scryer_domain::ExternalId {
            source: "tvdb".to_string(),
            value: "393199".to_string(),
        }];
        let mut anime = make_series_title("tide-chart-anime", "Tide Chart", Some(1999));
        anime.facet = MediaFacet::Anime;
        let bank = build_title_context_bank(&[live_action, anime]);

        let result = match_release_to_title_context(
            "Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP",
            &IndexerResponseAttributes {
                tvdb_id: Some("393199".to_string()),
                ..Default::default()
            },
            &bank,
        );

        assert_eq!(
            result.map(|info| info.title_id.as_str()),
            Some("tide-chart-live"),
            "an indexer-asserted id resolves the collision instead of skipping"
        );
    }

    #[test]
    fn shared_bare_key_collision_assigns_when_year_disambiguates() {
        let bank = tide_chart_rss_bank();
        let result = match_release("Tide.Chart.2023.S02E01.1080p.WEB-DL.x264-GRP", &bank);
        assert_eq!(
            result.map(|info| info.title_id.as_str()),
            Some("tide-chart-live"),
            "a year-stamped release names exactly one of the colliding titles"
        );
    }

    #[test]
    fn validator_rejects_target_biased_containment_projection() {
        // The target-biased parse projects "PALS" out of this name even
        // though the release is a different show whose episode title merely
        // contains the word; the validator must not accept that projection.
        let title = make_series_title("pals", "Pals", Some(1994));
        let evidence = canonical_title_evidence(&title);
        let junk = "Puppy.Harbor.Days.S02E23E24.Heroes.and.Pals.The.Cookie.Crumbles.1080p.PMTP.WEB-DL.AAC2.0.x264-AndreMor";
        let parsed = crate::parse_release_metadata_for_target(junk, &evidence.parse_context);
        assert!(!parsed_release_matches_title_evidence(&parsed, &evidence));
    }

    #[test]
    fn electric_bloom_cannot_prove_the_bloom_alias() {
        let titles = vec![make_title_with_aliases(
            "quiet-meadow",
            "The Quiet Meadow Blooms with Splendor",
            Some(2025),
            vec!["BLOOM"],
        )];
        let bank = build_title_context_bank(&titles);
        let release = "Electric.Bloom.S01E09.How.it.all.came.out.of.the.wash.MULTI.1080p.DSNP.WEB-DL.DDP5.1.H.264";

        assert!(
            match_release(release, &bank).is_none(),
            "a partial alias must not project the target over an unexplained title token"
        );
    }

    #[test]
    fn one_word_alias_requires_year_or_external_id() {
        let titles = vec![make_title_with_aliases(
            "quiet-meadow",
            "The Quiet Meadow Blooms with Splendor",
            Some(2025),
            vec!["BLOOM"],
        )];
        let bank = build_title_context_bank(&titles);

        assert!(match_release("Bloom.S01E01.1080p.WEB-DL", &bank).is_none());
        assert_eq!(
            match_release("Bloom.2025.S01E01.1080p.WEB-DL", &bank)
                .map(|info| info.title_id.as_str()),
            Some("quiet-meadow")
        );
    }

    #[test]
    fn validator_accepts_head_anchored_release_without_year() {
        let title = make_series_title("pals", "Pals", Some(1994));
        let evidence = canonical_title_evidence(&title);
        let legit = "Pals.S05E03.The.Hundredth.One.1080p.BluRay.x264-GRP";
        let parsed = crate::parse_release_metadata_for_target(legit, &evidence.parse_context);
        assert!(parsed_release_matches_title_evidence(&parsed, &evidence));
    }

    #[test]
    fn multi_token_title_matches_with_unbracketed_group_prefix() {
        // One leading release-group token before the title, no year in the
        // release name — must still head-anchor within the tolerance window.
        let titles = vec![make_series_title("cookxfamily", "Cook x Family", None)];
        let bank = build_title_context_bank(&titles);
        let parsed = crate::parse_release_metadata_for_target(
            "ToonsHub.Cook.x.Family.S03E07.1080p.AMZN.WEB-DL.DDP2.0.H264",
            &bank[0].evidence.parse_context,
        );
        assert!(parsed_release_matches_title_evidence(
            &parsed,
            &bank[0].evidence
        ));
        let result = match_release(
            "ToonsHub.Cook.x.Family.S03E07.1080p.AMZN.WEB-DL.DDP2.0.H264",
            &bank,
        );
        assert!(result.is_some(), "group-prefixed release must still match");
        assert_eq!(result.unwrap().title_id, "cookxfamily");
    }

    #[test]
    fn unknown_unbracketed_prefix_does_not_count_as_a_release_group() {
        let titles = vec![make_series_title("cookxfamily", "Cook x Family", None)];
        let bank = build_title_context_bank(&titles);

        assert!(
            match_release(
                "RandomTag.Cook.x.Family.S03E07.1080p.AMZN.WEB-DL.DDP2.0.H264",
                &bank,
            )
            .is_none()
        );
    }

    #[test]
    fn title_matches_with_bracketed_group_prefix() {
        let titles = vec![make_series_title(
            "kagerou",
            "Kagerou Kanmuri no Koubou",
            None,
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release(
            "[SubsPlease] Kagerou Kanmuri no Koubou - 12 (720p) [53B226F0]",
            &bank,
        );
        assert!(
            result.is_some(),
            "bracket-group release must strip to a head-anchored title"
        );
        assert_eq!(result.unwrap().title_id, "kagerou");
    }

    #[test]
    fn hyphenated_bracket_group_prefix_matches() {
        let titles = vec![make_series_title(
            "hoshiba",
            "Hoshiba Kaisei Nameless Wanderer",
            None,
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release(
            "[Erai-raws] Hoshiba Kaisei Nameless Wanderer S03E05 [1080p CR WEB-DL AVC AAC][MultiSub]",
            &bank,
        );
        assert!(
            result.is_some(),
            "multi-token bracket group must strip out of the title span"
        );
        assert_eq!(result.unwrap().title_id, "hoshiba");
    }

    #[test]
    fn two_token_bracket_group_prefix_matches() {
        let titles = vec![make_series_title(
            "silver-vale",
            "Silver Horizon Distant Vale",
            None,
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release(
            "[Anime Time] Silver Horizon Distant Vale - 05 [1080p][HEVC 10bit x265][AAC][Multi Sub]",
            &bank,
        );
        assert!(
            result.is_some(),
            "space-separated bracket group must strip out of the title span"
        );
        assert_eq!(result.unwrap().title_id, "silver-vale");
    }

    #[test]
    fn known_release_group_dotted_prefix_matches() {
        let titles = vec![make_series_title(
            "hoshiba",
            "Hoshiba Kaisei Nameless Wanderer",
            None,
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release(
            "Erai-raws.Hoshiba.Kaisei.Nameless.Wanderer.S03E05.1080p.CR.WEB-DL",
            &bank,
        );
        assert!(
            result.is_some(),
            "an unbracketed known release-group run must anchor past the prefix"
        );
        assert_eq!(result.unwrap().title_id, "hoshiba");
    }

    #[test]
    fn unknown_multi_token_prefix_still_rejects() {
        let titles = vec![make_series_title(
            "hoshiba",
            "Hoshiba Kaisei Nameless Wanderer",
            None,
        )];
        let bank = build_title_context_bank(&titles);
        assert!(
            match_release(
                "Totally.Unknown.Grp.Hoshiba.Kaisei.Nameless.Wanderer.S03E05.1080p.WEB-DL",
                &bank,
            )
            .is_none(),
            "an unknown multi-token prefix is containment junk, not a group tag"
        );
    }

    #[test]
    fn bare_release_between_year_twins_keeps_deterministic_winner() {
        // A bare release naming two year-distinguished twins must resolve the
        // same way it always has — smallest title id — so the ambiguity
        // parking downstream has a stable subject. The year-suffixed lookup
        // key must not make one twin "more specific" than the other when the
        // release itself carries no year.
        let classic = make_series_title("harbortales-1987", "HarborTales", Some(1987));
        let reboot = make_series_title("harbortales-2017", "HarborTales (2017)", Some(2017));
        for titles in [
            vec![classic.clone(), reboot.clone()],
            vec![reboot.clone(), classic.clone()],
        ] {
            let bank = build_title_context_bank(&titles);
            let result = match_release("HarborTales.S01E01.1080p.WEB-DL.AAC2.0.H.264", &bank);
            assert_eq!(
                result.map(|info| info.title_id.as_str()),
                Some("harbortales-1987"),
                "bare twin release must keep the deterministic title-id tiebreak"
            );
        }

        // The year-stamped control still resolves by year, not by tiebreak.
        let bank = build_title_context_bank(&[classic, reboot]);
        let result = match_release("HarborTales.2017.S01E03.1080p.WEB-DL", &bank);
        assert_eq!(
            result.map(|info| info.title_id.as_str()),
            Some("harbortales-2017"),
        );
    }

    #[test]
    fn bracket_styled_title_matches() {
        let titles = vec![make_series_title("nagi-no-ko", "Nagi no Ko", None)];
        let bank = build_title_context_bank(&titles);
        let result = match_release("[Nagi no Ko].S02E01.1080p.WEB-DL.AAC2.0.H.264", &bank);
        assert!(
            result.is_some(),
            "a bracket-styled title with no title text after the brackets must match"
        );
        assert_eq!(result.unwrap().title_id, "nagi-no-ko");
    }

    #[test]
    fn bracket_group_before_unknown_title_still_rejects() {
        let titles = vec![make_series_title("judas", "Judas", Some(2021))];
        let bank = build_title_context_bank(&titles);
        assert!(
            match_release("[Judas].Some.Other.Show.S01E01.1080p.WEB-DL", &bank).is_none(),
            "a bracket group followed by another show's title text is a tag, not the subject"
        );
    }

    #[test]
    fn stacked_alias_release_matches_via_bank() {
        let mut title = make_series_title("vale", "Silver Horizon Beyond the Vale", Some(2023));
        title.aliases = vec!["Sora no Vale".to_string()];
        let bank = build_title_context_bank(&[title]);
        let result = match_release(
            "[SubsPlease] Sora.no.Vale.Silver.Horizon.Beyond.the.Vale.-.01.[1080p].[HEVC]",
            &bank,
        );
        assert!(
            result.is_some(),
            "a name stacking two alias forms of the same subject must anchor"
        );
        assert_eq!(result.unwrap().title_id, "vale");
    }

    #[test]
    fn trailing_year_title_matches_with_and_without_release_year() {
        let titles = vec![make_series_title(
            "sr2049",
            "Signal Runner 2049",
            Some(2017),
        )];
        let bank = build_title_context_bank(&titles);
        for release in [
            "Signal.Runner.2049.2017.2160p.WEB-DL.DDP5.1.HDR.HEVC",
            "Signal.Runner.2049.1080p.BluRay.x264",
        ] {
            let result = match_release(release, &bank);
            assert!(
                result.is_some(),
                "year-suffixed title must anchor even when the boundary heuristic splits it: {release}"
            );
            assert_eq!(result.unwrap().title_id, "sr2049");
        }
    }

    #[test]
    fn parsed_release_matches_series_title_when_library_title_includes_year() {
        let title = make_title("t1", "Harbor Pals (2018)", Some(2018));
        let parsed = crate::parse_release_metadata("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");

        assert!(parsed_release_matches_title(&parsed, &title));
    }

    #[test]
    fn parsed_release_does_not_match_unrelated_series_title() {
        let title = make_title("t1", "Harbor Pals (2018)", Some(2018));
        let parsed =
            crate::parse_release_metadata("Azure.Warden.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");

        assert!(!parsed_release_matches_title(&parsed, &title));
    }

    // ── extract_title_from_release ──────────────────────────────────

    #[test]
    fn extract_title_normalizes() {
        let parsed = crate::parse_release_metadata("The.Grey.Harbor.2008.1080p.BluRay");
        let title = extract_title_from_release(&parsed);
        assert_eq!(title, "the grey harbor");
    }

    #[test]
    fn extract_title_variants_returns_canonical_then_alternates() {
        let parsed =
            crate::parse_release_metadata("Portmere.A.K.A.Hard.Nine.1996.1080p.WEB-DL.H.264");
        let titles = extract_titles_from_release(&parsed);
        assert_eq!(
            titles,
            vec![
                "portmere aka hard nine".to_string(),
                "portmere".to_string(),
                "hard nine".to_string()
            ]
        );
    }
}
