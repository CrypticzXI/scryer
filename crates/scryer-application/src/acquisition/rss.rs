use super::*;
use crate::acquisition_decision_helpers::is_download_submit_unavailable_error;
use crate::acquisition_release_search::{
    AutoCandidateEvaluationContext, ReleaseAutoDecisionCode, annotate_auto_decision,
    canonical_title_evidence, evaluate_auto_candidate, parsed_release_matches_title_evidence,
    serialize_decision_explanation,
};
use crate::delay_profile::DelayProfile;
use crate::domain_events::{new_title_domain_event, title_context_snapshot};
use crate::settings::keys::default_indexer_routing_categories_for_scope;
use chrono::{DateTime, Utc};
use scryer_domain::{DomainEventPayload, ReleaseGrabbedEventData};
use std::collections::{HashMap, HashSet};
use tracing::{info, warn};

const RSS_SYNC_MAX_GUIDS: usize = 2000;
const RSS_TITLE_CONTEXT_CANDIDATE_LIMIT: usize = 8;

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
}

#[derive(Clone)]
struct TitleContextCandidate {
    info: TitleMatchInfo,
    evidence: crate::acquisition_release_search::CanonicalTitleEvidence,
}

fn build_title_context_bank(titles: &[Title]) -> Vec<TitleContextCandidate> {
    titles
        .iter()
        .filter(|title| title.monitored)
        .map(|title| TitleContextCandidate {
            info: TitleMatchInfo {
                title_id: title.id.clone(),
                year: title.year,
            },
            evidence: canonical_title_evidence(title),
        })
        .collect()
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

fn release_tokens_for_matching(release_title: &str) -> Vec<String> {
    normalize_for_matching(release_title)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn release_contains_year(release_tokens: &[String], year: i32) -> bool {
    let year = year.to_string();
    release_tokens.iter().any(|token| token == &year)
}

fn token_window_contains(release_tokens: &[String], title_tokens: &[&str]) -> bool {
    !title_tokens.is_empty()
        && release_tokens.windows(title_tokens.len()).any(|window| {
            window
                .iter()
                .map(String::as_str)
                .eq(title_tokens.iter().copied())
        })
}

fn title_key_match_score(
    release_tokens: &[String],
    title_key: &str,
    title_year: Option<i32>,
) -> Option<i32> {
    let title_tokens = title_key.split_whitespace().collect::<Vec<_>>();
    if !token_window_contains(release_tokens, &title_tokens) {
        return None;
    }

    if title_tokens.len() == 1 {
        let token_len = title_tokens[0].chars().count();
        let year_matches =
            title_year.is_some_and(|year| release_contains_year(release_tokens, year));
        if token_len < 3 && !year_matches {
            return None;
        }
    }

    let mut score = i32::try_from(title_tokens.len()).unwrap_or(i32::MAX / 10) * 10;
    if title_year.is_some_and(|year| release_contains_year(release_tokens, year)) {
        score += 6;
    }
    Some(score)
}

fn context_candidate_match_score(
    release_tokens: &[String],
    candidate: &TitleContextCandidate,
) -> Option<i32> {
    let mut best_score: Option<i32> = None;

    for key in &candidate.evidence.lookup_keys {
        if let Some(score) = title_key_match_score(release_tokens, key, candidate.info.year) {
            best_score = Some(best_score.map_or(score, |best| best.max(score)));
        }

        if let Some(year) = candidate.info.year {
            let year_suffix = format!(" {year}");
            if let Some(stripped_key) = key.strip_suffix(&year_suffix)
                && let Some(score) =
                    title_key_match_score(release_tokens, stripped_key, candidate.info.year)
            {
                best_score = Some(best_score.map_or(score, |best| best.max(score)));
            }
        }
    }

    best_score
}

/// Match an RSS release against monitored titles using real title contexts.
/// The cheap lexical pass only builds a small candidate bank; the final match
/// is always a context-aware v2 parse for a concrete catalog title.
fn match_release_to_title_context<'a>(
    release_title: &str,
    context_bank: &'a [TitleContextCandidate],
) -> Option<&'a TitleMatchInfo> {
    let release_tokens = release_tokens_for_matching(release_title);
    if release_tokens.is_empty() {
        return None;
    }

    let mut candidates = context_bank
        .iter()
        .filter_map(|candidate| {
            context_candidate_match_score(&release_tokens, candidate)
                .map(|score| (candidate, score))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(
        |(left_candidate, left_score), (right_candidate, right_score)| {
            right_score.cmp(left_score).then_with(|| {
                left_candidate
                    .info
                    .title_id
                    .cmp(&right_candidate.info.title_id)
            })
        },
    );
    candidates.truncate(RSS_TITLE_CONTEXT_CANDIDATE_LIMIT);

    let mut best: Option<(&TitleMatchInfo, i32)> = None;
    for (candidate, lexical_score) in candidates {
        let parsed =
            parse_release_metadata_for_target(release_title, &candidate.evidence.parse_context);
        if let (Some(parsed_year), Some(title_year)) = (parsed.year, candidate.info.year)
            && parsed_year != title_year
        {
            continue;
        }
        if !parsed_release_matches_title_evidence(&parsed, &candidate.evidence) {
            continue;
        }

        let year_bonus = i32::from(parsed.year.is_some() && parsed.year == candidate.info.year) * 8;
        let parser_bonus = (parsed.parse_confidence * 10.0).round() as i32;
        let score = lexical_score + year_bonus + parser_bonus;

        match best {
            Some((best_info, best_score))
                if score < best_score
                    || (score == best_score && candidate.info.title_id >= best_info.title_id) => {}
            _ => best = Some((&candidate.info, score)),
        }
    }

    best.map(|(info, _)| info)
}

#[cfg_attr(not(test), allow(dead_code))]
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
            if let Some(title_info) =
                match_release_to_title_context(&result.title, &title_context_bank)
            {
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

        // For each matched title, score and potentially grab
        for (title_id, releases) in &matched_by_title {
            let title = match self.services.catalog.titles.get_by_id(title_id).await {
                Ok(Some(t)) => t,
                _ => continue,
            };

            // Check if there's a wanted item for this title
            let wanted = self
                .services
                .workflow
                .wanted_items
                .get_wanted_item_for_title(title_id, None)
                .await
                .ok()
                .flatten();

            // For series, we need to match individual episodes
            let has_episodes = self
                .facet_registry
                .get(&title.facet)
                .map(|h| h.has_episodes())
                .unwrap_or(false);

            if has_episodes {
                // For series: match each release to a specific episode's wanted item
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
            } else {
                // For movies: use the title-level wanted item
                let Some(wanted) = wanted else {
                    continue;
                };
                if wanted.status == WantedStatus::Grabbed && wanted.current_score.is_some() {
                    // Already grabbed — only proceed if upgrade is possible
                }
                self.process_rss_title_releases(
                    &title,
                    &wanted,
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

    /// Process RSS releases matched to a movie title.
    #[expect(
        clippy::too_many_arguments,
        reason = "RSS movie processing threads grab state, timing, and scoring context together"
    )]
    async fn process_rss_title_releases(
        &self,
        title: &Title,
        wanted: &WantedItem,
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
            wanted,
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

    /// Process RSS releases matched to a series title — match episodes individually.
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

        // Route exact episodes, absolute ranges, and season packs to every
        // covered catalog episode. Monitored status is intentionally ignored
        // here; wanted-item lookup below is the only pre-download gate.
        let mut by_episode: HashMap<String, Vec<IndexerSearchResult>> = HashMap::new();
        for release in releases {
            let parsed = parse_release_metadata_for_target(&release.title, &title_parse_context);
            let coverage = crate::acquisition_coverage::resolve_release_coverage(
                &parsed,
                &catalog_episodes,
                &catalog_collections,
                None,
            );
            match coverage {
                crate::acquisition_coverage::ReleaseCoverage::SingleEpisode(episode_id) => {
                    by_episode
                        .entry(episode_id)
                        .or_default()
                        .push(release.clone());
                }
                crate::acquisition_coverage::ReleaseCoverage::EpisodeSet(episode_ids) => {
                    for episode_id in episode_ids {
                        by_episode
                            .entry(episode_id)
                            .or_default()
                            .push(release.clone());
                    }
                }
                crate::acquisition_coverage::ReleaseCoverage::Collection(collection_id) => {
                    for episode in catalog_episodes
                        .iter()
                        .filter(|episode| episode.collection_id.as_deref() == Some(&collection_id))
                    {
                        by_episode
                            .entry(episode.id.clone())
                            .or_default()
                            .push(release.clone());
                    }
                }
                crate::acquisition_coverage::ReleaseCoverage::Title
                | crate::acquisition_coverage::ReleaseCoverage::Unknown => {}
            }
        }

        for (episode_id, episode_releases) in &by_episode {
            let wanted = match self
                .services
                .workflow
                .wanted_items
                .get_wanted_item_for_title(&title.id, Some(episode_id))
                .await
            {
                Ok(Some(w)) => w,
                _ => continue, // Not wanted
            };

            if wanted.status != WantedStatus::Wanted && wanted.status != WantedStatus::Grabbed {
                continue;
            }

            let episode_record = episodes_by_id.get(episode_id).cloned();
            let episode_parse_context = build_release_parse_context(
                title,
                episode_record.as_ref(),
                None,
                Some(category.as_str()),
            );
            let absolute_episode = episode_record
                .as_ref()
                .and_then(|episode| episode.absolute_number.as_deref())
                .and_then(|value| value.trim().parse::<u32>().ok());

            // Score these releases
            let owned_releases: Vec<IndexerSearchResult> = episode_releases.to_vec();
            let scored = match self
                .score_rss_releases(
                    &owned_releases,
                    &title.id,
                    title.imdb_id.clone(),
                    tvdb_id.clone(),
                    Some(category.clone()),
                    &title.tags,
                    title.runtime_minutes,
                    &episode_parse_context,
                    episode_record
                        .as_ref()
                        .and_then(|episode| episode.season_number.as_deref())
                        .and_then(|value| value.parse::<u32>().ok()),
                    episode_record
                        .as_ref()
                        .and_then(|episode| episode.episode_number.as_deref())
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
        let quality_profile = self
            .resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
                title_tags,
                library_id: None,
                imdb_id: imdb_id.as_deref(),
                tvdb_id: tvdb_id.as_deref(),
                category_hint: category.as_deref(),
            })
            .await?;
        let scope_id =
            self.quality_profile_scope_id(crate::app_usecase_discovery::QualityProfileLookup {
                title_tags,
                library_id: None,
                imdb_id: imdb_id.as_deref(),
                tvdb_id: tvdb_id.as_deref(),
                category_hint: category.as_deref(),
            });
        let indexer_routing = self
            .resolve_indexer_routing(None, scope_id.as_deref())
            .await;

        Ok(self
            .score_release_results(
                releases.to_vec(),
                &quality_profile,
                title_id,
                None,
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
    #[expect(
        clippy::too_many_arguments,
        reason = "RSS grab attempts coordinate release state, client state, and reporting in one place"
    )]
    async fn try_grab_rss_release(
        &self,
        title: &Title,
        wanted: &WantedItem,
        scored: &[IndexerSearchResult],
        category: &str,
        dl_snapshot: &super::acquisition_workflow::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        let db_blocklist: HashSet<String> = self
            .services
            .workflow
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 200)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| e.source_title)
            .map(|t| t.to_ascii_lowercase())
            .collect();
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
        let subject = self
            .resolve_release_search_subject_for_wanted_item(title, wanted, episode.as_ref())
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
        let upgrade_context = self
            .resolve_upgrade_context_for_title_with_category(
                title,
                wanted.grabbed_release.as_deref(),
                Some(category),
            )
            .await;
        if upgrade_context.cutoff_reached {
            return;
        }

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
                failed_source_kinds: None,
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
                    .download_url
                    .clone()
                    .or_else(|| decision_candidate.link.clone()),
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
                .wanted_items
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
                self.insert_pending_release(
                    wanted,
                    title,
                    &candidate.title,
                    candidate
                        .download_url
                        .as_deref()
                        .or(candidate.link.as_deref()),
                    candidate.source_kind,
                    candidate.size_bytes,
                    candidate_score,
                    serialize_decision_explanation(&decision_candidate),
                    Some(candidate.source.as_str()),
                    candidate.guid.as_deref(),
                    delay_minutes,
                    candidate.password_hint.as_deref(),
                    candidate.published_at.as_deref(),
                    candidate.extra.get("info_hash").and_then(|v| v.as_str()),
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

        let source_hint = best.download_url.clone().or_else(|| best.link.clone());
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
            best.source_kind,
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
        let download_id = crate::download_identity::new_download_id();
        let submission_identity = DownloadSubmissionIdentity {
            download_id: Some(download_id.clone()),
        };

        let grab_result = self
            .services
            .integrations
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                download_id: Some(download_id),
                source_hint: source_hint.clone(),
                staged_nzb: None,
                source_kind: best.source_kind,
                source_title: source_title.clone(),
                source_password: source_password.clone(),
                category: Some(download_cat),
                queue_priority: None,
                download_directory: None,
                release_title: Some(best.title.clone()),
                indexer_name: Some(best.source.clone()),
                info_hash_hint: info_hash_hint.clone(),
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: None,
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
                        wanted,
                        episode.as_ref(),
                        &best.title,
                    )
                };
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
                            source_hint_for_attempt,
                            source_title_for_attempt,
                            ReleaseDownloadAttemptOutcome::Failed,
                            Some(error.to_string()),
                            source_password,
                        )
                        .await;
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
                    .wanted_items
                    .transition_wanted_to_grabbed(&WantedGrabTransition {
                        id: wanted.id.clone(),
                        last_search_at: Some(now.to_rfc3339()),
                        search_count: wanted.search_count,
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
                            download_id: None,
                            episode_ids: Vec::new(),
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

                let _ = self
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt,
                        source_title_for_attempt,
                        if is_download_submit_unavailable_error(&err) {
                            ReleaseDownloadAttemptOutcome::Pending
                        } else {
                            ReleaseDownloadAttemptOutcome::Failed
                        },
                        Some(err.to_string()),
                        source_password,
                    )
                    .await;
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
            monitored: true,
            tags: vec![],
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

    #[test]
    fn match_exact_title() {
        let titles = vec![make_title("t1", "Neon Cipher", Some(2010))];
        let bank = build_title_context_bank(&titles);
        let result = match_release_to_title_context("Neon.Cipher.2010.1080p.BluRay.x264", &bank);
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
        let result = match_release_to_title_context("Glass.Harbor.2021.1080p.BluRay.x264", &bank);
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
        let result = match_release_to_title_context("Neon.Cipher.2010.1080p.BluRay", &bank);
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_release_title_without_year_finds_title_with_year() {
        // Lookup has "title 2024", release only has "title"
        let titles = vec![make_title("t1", "Glass Harbor 2024", Some(2024))];
        let bank = build_title_context_bank(&titles);
        let result = match_release_to_title_context("Glass Harbor", &bank);
        // Should match via the reverse year-addition path
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_no_match_returns_none() {
        let titles = vec![make_title("t1", "Neon Cipher", Some(2010))];
        let bank = build_title_context_bank(&titles);
        let result = match_release_to_title_context("Totally.Unknown.Movie.2024.1080p", &bank);
        assert!(result.is_none());
    }

    #[test]
    fn match_empty_release_title_returns_none() {
        let titles = vec![make_title("t1", "Neon Cipher", Some(2010))];
        let bank = build_title_context_bank(&titles);
        let result = match_release_to_title_context("", &bank);
        assert!(result.is_none());
    }

    #[test]
    fn match_via_alias() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "Lantern Tide",
            Some(2001),
            vec!["Sen to Chihiro no Kamikakushi"],
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release_to_title_context("Sen.to.Chihiro.no.Kamikakushi", &bank);
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_via_release_aka_title_variant() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "My Cousin",
            Some(2020),
            vec!["Mon Cousin"],
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release_to_title_context(
            "Mon.Cousin.A.K.A.My.Cousin.2020.1080p.BluRay.x264-GRP",
            &bank,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_via_release_slash_title_variant() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "My Cousin",
            Some(2020),
            vec!["Mon Cousin"],
        )];
        let bank = build_title_context_bank(&titles);
        let result = match_release_to_title_context(
            "Mon Cousin / My Cousin 2020 1080p BluRay x264-GRP",
            &bank,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
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
            crate::parse_release_metadata("Blue.Exorcist.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");

        assert!(!parsed_release_matches_title(&parsed, &title));
    }

    // ── extract_title_from_release ──────────────────────────────────

    #[test]
    fn extract_title_normalizes() {
        let parsed = crate::parse_release_metadata("The.Dark.Knight.2008.1080p.BluRay");
        let title = extract_title_from_release(&parsed);
        assert_eq!(title, "the dark knight");
    }

    #[test]
    fn extract_title_variants_returns_canonical_then_alternates() {
        let parsed =
            crate::parse_release_metadata("Sydney.A.K.A.Hard.Eight.1996.1080p.WEB-DL.H.264");
        let titles = extract_titles_from_release(&parsed);
        assert_eq!(
            titles,
            vec![
                "sydney aka hard eight".to_string(),
                "sydney".to_string(),
                "hard eight".to_string()
            ]
        );
    }
}
