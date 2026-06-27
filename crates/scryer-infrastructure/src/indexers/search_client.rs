use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, IndexerClient, IndexerConfigRepository, IndexerPluginProvider,
    IndexerRoutingPlan, IndexerSearchResponse, IndexerSearchResult, IndexerStatsTracker,
    IndexerSystemBackoff, ReleaseCandidateProvenance, ReleaseSearchSubjectKind, SearchMode,
};
use scryer_domain::{
    IndexerCapsSearchNode, IndexerCapsSnapshot, IndexerConfig, IndexerProviderCapabilities,
    IndexerSearchInputCapability, NabTransportKind,
};
use serde::Deserialize;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// A single search strategy dispatched as an independent parallel task.
/// Each strategy carries the raw query/ID params to pass through to the plugin.
#[derive(Clone, Debug)]
struct SearchStrategy {
    request_query: String,
    request_facet: String,
    ids: HashMap<String, String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
    generic_query_only: bool,
    label: String,
}

#[derive(Debug)]
struct StrategyExecutionOutcome {
    label: String,
    title_guard_mode: TitleGuardMode,
    response: AppResult<IndexerSearchResponse>,
    elapsed: std::time::Duration,
    retry_after: Option<std::time::Duration>,
}

#[derive(Clone)]
struct StrategyTierContext {
    client: Arc<dyn IndexerClient>,
    search_limit: Arc<Semaphore>,
    rate_limiter: IndexerRateLimiter,
    indexer_id: String,
    rate_limit_seconds: Option<i64>,
    category: Option<String>,
    per_indexer_categories: Option<Vec<String>>,
    mode: SearchMode,
    tagged_aliases: Vec<scryer_domain::TaggedAlias>,
    cancel_token: CancellationToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TitleGuardMode {
    SkipTitleMatch,
    ExactTitleMatch,
}

#[derive(Debug, Default, Deserialize)]
struct ManagedIndexerMetadata {
    enable_rss: Option<bool>,
    enable_automatic_search: Option<bool>,
    #[serde(default)]
    caps_snapshot: Option<IndexerCapsSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdDispatchMode {
    LegacyAggregate,
    Aggregate,
    QueryOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextDispatchMode {
    None,
    FacetScoped,
    GenericOnly,
}

impl TextDispatchMode {
    fn can_dispatch(self) -> bool {
        !matches!(self, Self::None)
    }

    fn is_generic_only(self) -> bool {
        matches!(self, Self::GenericOnly)
    }
}

#[derive(Clone, Debug)]
struct ResolvedSearchCapabilities {
    caps: IndexerProviderCapabilities,
    id_dispatch_mode: IdDispatchMode,
    text_dispatch_mode: TextDispatchMode,
    query_only_reason: Option<&'static str>,
    transport_kind: Option<NabTransportKind>,
    caps_source: &'static str,
}

struct FilterStrategyContext<'a> {
    query: &'a str,
    season: Option<u32>,
    episode: Option<u32>,
    tagged_aliases: &'a [scryer_domain::TaggedAlias],
    title_guard_mode: TitleGuardMode,
    strategy_label: &'a str,
    is_rss_request: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SearchLane {
    Interactive,
    BackgroundAuto,
}

impl SearchLane {
    fn from_mode(mode: SearchMode) -> Self {
        match mode {
            SearchMode::Interactive => Self::Interactive,
            SearchMode::Auto => Self::BackgroundAuto,
        }
    }
}

#[derive(Default)]
struct StrategyBatchHealth {
    any_success: bool,
    any_error: bool,
    retry_after: Option<std::time::Duration>,
}

impl StrategyBatchHealth {
    fn mark_success(&mut self) {
        self.any_success = true;
    }

    fn mark_error(&mut self, retry_after: Option<std::time::Duration>) {
        self.any_error = true;
        if let Some(retry_after) = retry_after
            && self.retry_after.is_none_or(|current| retry_after > current)
        {
            self.retry_after = Some(retry_after);
        }
    }

    async fn apply(
        self,
        backoff_tracker: &IndexerBackoffTracker,
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
        had_persisted_system_backoff: bool,
    ) {
        if self.any_success {
            let had_in_memory_backoff = backoff_tracker.record_success(indexer_id).await;
            if had_in_memory_backoff || had_persisted_system_backoff {
                MultiIndexerSearchClient::clear_indexer_system_backoff(
                    indexer_configs,
                    indexer_id,
                    indexer_name,
                )
                .await;
            }
        } else if self.any_error {
            let backoff = backoff_tracker
                .record_failure(indexer_id, self.retry_after)
                .await;
            MultiIndexerSearchClient::record_indexer_system_backoff(
                indexer_configs,
                indexer_id,
                indexer_name,
                backoff.clone(),
            )
            .await;
            warn!(
                indexer = indexer_name,
                disabled_until = %backoff.disabled_until,
                escalation_level = backoff.escalation_level,
                "indexer backoff escalated"
            );
        }
    }
}

const INDEXER_SEARCH_TIMEOUT_SECS: u64 = 12;
const BACKGROUND_INDEXER_SEARCH_CONCURRENCY_LIMIT: usize = 12;
const INTERACTIVE_INDEXER_SEARCH_CONCURRENCY_LIMIT: usize = 24;

fn log_indexer_skip(
    mode: SearchMode,
    indexer_name: &str,
    reason: &str,
    disabled_until: Option<chrono::DateTime<chrono::Utc>>,
) {
    if matches!(mode, SearchMode::Interactive) {
        if let Some(disabled_until) = disabled_until {
            info!(
                indexer = indexer_name,
                reason,
                disabled_until = %disabled_until,
                "skipping indexer before dispatch"
            );
        } else {
            info!(
                indexer = indexer_name,
                reason, "skipping indexer before dispatch"
            );
        }
    } else if let Some(disabled_until) = disabled_until {
        debug!(
            indexer = indexer_name,
            reason,
            disabled_until = %disabled_until,
            "skipping indexer before dispatch"
        );
    } else {
        debug!(
            indexer = indexer_name,
            reason, "skipping indexer before dispatch"
        );
    }
}

fn should_run_fallback_tier(
    mode: SearchMode,
    collected_results: &[IndexerSearchResult],
    primary_attempted: bool,
    primary_had_error: bool,
    fallback_strategies: &[SearchStrategy],
) -> bool {
    if mode == SearchMode::Auto && primary_had_error {
        return false;
    }

    collected_results.is_empty() && primary_attempted && !fallback_strategies.is_empty()
}

fn retry_after_from_error(error: &AppError) -> Option<std::time::Duration> {
    parse_retry_after_seconds(&error.to_string())
}

fn parse_retry_after_seconds(message: &str) -> Option<std::time::Duration> {
    let marker = "retry_after_seconds=";
    let (_, rest) = message.split_once(marker)?;
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    Some(std::time::Duration::from_secs(digits.parse::<u64>().ok()?))
}

/// Records transport metrics per outbound indexer request.
///
/// A single high-level search can emit multiple request attempts when we fan
/// out across ID/episode strategies or run a freetext fallback tier. Keeping
/// these counters at request granularity makes the tier labels actionable in
/// dashboards and keeps latency tied to the specific outbound call.
fn record_strategy_metrics(
    indexer_name: &str,
    mode_label: &str,
    status: &str,
    elapsed: std::time::Duration,
    result_count: Option<usize>,
) {
    metrics::counter!(
        "scryer_indexer_queries_total",
        "indexer" => indexer_name.to_string(),
        "status" => status.to_string(),
        "mode" => mode_label.to_string()
    )
    .increment(1);
    metrics::histogram!(
        "scryer_indexer_query_duration_seconds",
        "indexer" => indexer_name.to_string(),
        "mode" => mode_label.to_string()
    )
    .record(elapsed.as_secs_f64());

    if let Some(result_count) = result_count {
        metrics::counter!(
            "scryer_indexer_query_results_total",
            "indexer" => indexer_name.to_string(),
            "mode" => mode_label.to_string()
        )
        .increment(result_count as u64);
    }
}

fn record_auto_strategy_selection(
    indexer_name: &str,
    caps_source: &'static str,
    primary_strategies: &[SearchStrategy],
    fallback_strategies: &[SearchStrategy],
) {
    let strategy_count = primary_strategies.len() + fallback_strategies.len();
    let primary_labels = primary_strategies
        .iter()
        .map(|strategy| strategy.label.as_str())
        .collect::<Vec<_>>();
    let fallback_labels = fallback_strategies
        .iter()
        .map(|strategy| strategy.label.as_str())
        .collect::<Vec<_>>();

    metrics::histogram!(
        "scryer_indexer_auto_strategy_count",
        "indexer" => indexer_name.to_string(),
        "caps_source" => caps_source.to_string()
    )
    .record(strategy_count as f64);

    debug!(
        indexer = indexer_name,
        mode = "auto",
        caps_source,
        auto_strategy_count = strategy_count,
        primary_strategy_count = primary_strategies.len(),
        fallback_strategy_count = fallback_strategies.len(),
        primary_strategies = ?primary_labels,
        fallback_strategies = ?fallback_labels,
        "selected automatic indexer search strategies"
    );
}

fn preferred_anime_alias_query(
    query: &str,
    tagged_aliases: &[scryer_domain::TaggedAlias],
) -> Option<String> {
    let canonical = strip_query_context(query);
    if canonical.is_empty() {
        return None;
    }

    let alias_candidates: Vec<(String, String, bool, bool)> = tagged_aliases
        .iter()
        .map(|alias| {
            let trimmed = alias.name.trim().to_string();
            let language_matches = alias.language.eq_ignore_ascii_case("jpn");
            let romanized = is_romanized_alias(&alias.name);
            (trimmed, alias.language.clone(), language_matches, romanized)
        })
        .collect();

    alias_candidates
        .iter()
        .find(|(name, _, language_matches, romanized)| {
            !name.is_empty()
                && *language_matches
                && *romanized
                && !canonical.eq_ignore_ascii_case(name)
        })
        .map(|(name, _, _, _)| name.clone())
}

fn is_freetext_strategy_label(label: &str) -> bool {
    matches!(label, "freetext" | "freetext_alias")
}

fn is_title_query_strategy_label(label: &str) -> bool {
    is_freetext_strategy_label(label) || label == "fallback"
}

fn should_defer_freetext_to_fallback(_facet: &str, strategies: &[SearchStrategy]) -> bool {
    strategies
        .iter()
        .any(|strategy| !is_freetext_strategy_label(&strategy.label))
        && strategies
            .iter()
            .any(|strategy| is_freetext_strategy_label(&strategy.label))
}

fn split_strategy_tiers(
    mode: SearchMode,
    facet: &str,
    strategies: Vec<SearchStrategy>,
) -> (Vec<SearchStrategy>, Vec<SearchStrategy>) {
    if mode == SearchMode::Auto {
        return split_auto_strategy_tiers(strategies);
    }

    if !should_defer_freetext_to_fallback(facet, &strategies) {
        return (strategies, Vec::new());
    }

    let mut primary = Vec::new();
    let mut fallback = Vec::new();

    for strategy in strategies {
        if is_freetext_strategy_label(&strategy.label) {
            fallback.push(strategy);
        } else {
            primary.push(strategy);
        }
    }

    if primary.is_empty() || fallback.is_empty() {
        let mut merged = primary;
        merged.extend(fallback);
        return (merged, Vec::new());
    }

    (primary, fallback)
}

fn split_auto_strategy_tiers(
    strategies: Vec<SearchStrategy>,
) -> (Vec<SearchStrategy>, Vec<SearchStrategy>) {
    if strategies.len() <= 1 {
        return (strategies, Vec::new());
    }

    let mut primary_candidates = Vec::new();
    let mut fallback_candidates = Vec::new();

    for strategy in strategies {
        if is_title_query_strategy_label(&strategy.label) {
            fallback_candidates.push(strategy);
        } else {
            primary_candidates.push(strategy);
        }
    }

    if primary_candidates.is_empty() {
        return (
            take_best_auto_strategy(&mut fallback_candidates)
                .into_iter()
                .collect(),
            Vec::new(),
        );
    }

    let primary = take_best_auto_strategy(&mut primary_candidates)
        .into_iter()
        .collect();
    let fallback = take_best_auto_strategy(&mut fallback_candidates)
        .into_iter()
        .collect();

    (primary, fallback)
}

fn take_best_auto_strategy(strategies: &mut Vec<SearchStrategy>) -> Option<SearchStrategy> {
    let index = strategies
        .iter()
        .enumerate()
        .min_by_key(|(_, strategy)| auto_strategy_rank(strategy))
        .map(|(index, _)| index)?;
    Some(strategies.swap_remove(index))
}

fn auto_strategy_rank(strategy: &SearchStrategy) -> (u8, u8) {
    match strategy.label.as_str() {
        "ids_abs" => (0, 0),
        "ids_sxex" => (0, 1),
        "ids" => (0, 2),
        "rss" => (0, 3),
        "freetext" => (1, 0),
        "freetext_alias" => (1, 1),
        "fallback" => (1, 2),
        _ if !strategy.ids.is_empty() => (0, 4),
        _ => (1, 3),
    }
}

fn strip_query_context(query: &str) -> &str {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return query.trim();
    }

    let mut start = tokens.len();
    for index in (0..tokens.len()).rev() {
        if looks_like_context_token(tokens[index]) {
            start = index;
        } else if start != tokens.len() {
            break;
        }
    }

    if start == tokens.len() {
        query.trim()
    } else {
        query[..query.rfind(tokens[start]).unwrap_or(query.len())].trim()
    }
}

fn looks_like_context_token(token: &str) -> bool {
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

    trimmed.chars().all(|ch| ch.is_ascii_digit())
}

fn is_romanized_alias(alias: &str) -> bool {
    let trimmed = alias.trim();
    !trimmed.is_empty()
        && trimmed.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    ' ' | '-' | '_' | ':' | ';' | ',' | '.' | '\'' | '&' | '!' | '?'
                )
        })
}

/// Per-indexer rate limiter tracking the last request time.
#[derive(Clone)]
struct IndexerRateLimiter {
    last_request: Arc<Mutex<HashMap<(String, SearchLane), tokio::time::Instant>>>,
}

impl IndexerRateLimiter {
    fn new() -> Self {
        Self {
            last_request: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Wait until the rate limit period has elapsed for this indexer.
    /// When `rate_limit_seconds` is set (from config/plugin), that value wins.
    /// Otherwise the default depends on the search mode:
    ///   - Interactive: 1s (fast for end-user experience)
    ///   - Auto: 1s (keeps background acquisition moving without blocking the
    ///     interactive lane behind a long default wait)
    async fn acquire(&self, indexer_id: &str, rate_limit_seconds: Option<i64>, mode: SearchMode) {
        let default_secs = match mode {
            SearchMode::Interactive => 1,
            SearchMode::Auto => 1,
        };
        let interval_secs = rate_limit_seconds.unwrap_or(default_secs).max(0) as u64;
        if interval_secs == 0 {
            return;
        }

        let interval = std::time::Duration::from_secs(interval_secs);
        let now = tokio::time::Instant::now();
        let lane_key = (indexer_id.to_string(), SearchLane::from_mode(mode));

        let mut map = self.last_request.lock().await;
        if let Some(last) = map.get(&lane_key) {
            let elapsed = now.duration_since(*last);
            if elapsed < interval {
                let wait = interval - elapsed;
                drop(map); // Release lock while sleeping
                tokio::time::sleep(wait).await;
                let mut map = self.last_request.lock().await;
                map.insert(lane_key, tokio::time::Instant::now());
                return;
            }
        }
        map.insert(lane_key, now);
    }
}

/// Short escalating system backoff periods. Provider `Retry-After` handling can
/// choose longer when explicitly supplied, but generic storm containment caps at
/// one hour to avoid stranding every indexer after one transient burst.
const BACKOFF_PERIODS_SECS: &[u64] = &[
    5 * 60,  // 5 minutes
    10 * 60, // 10 minutes
    15 * 60, // 15 minutes
    30 * 60, // 30 minutes
    60 * 60, // 1 hour
];

#[derive(Clone, Debug)]
struct IndexerBackoffState {
    escalation_level: usize,
    disabled_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// In-memory indexer backoff tracker. Persistent system backoffs seed this
/// state on startup/search so escalation survives process restarts.
#[derive(Clone)]
struct IndexerBackoffTracker {
    state: Arc<Mutex<HashMap<String, IndexerBackoffState>>>,
}

impl IndexerBackoffTracker {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn seed_persisted(&self, indexer_id: &str, backoff: &IndexerSystemBackoff) {
        let mut map = self.state.lock().await;
        let state = map
            .entry(indexer_id.to_string())
            .or_insert(IndexerBackoffState {
                escalation_level: 0,
                disabled_until: None,
            });
        state.escalation_level = state.escalation_level.max(backoff.escalation_level);
        if backoff.disabled_until > chrono::Utc::now()
            && state
                .disabled_until
                .is_none_or(|current| backoff.disabled_until > current)
        {
            state.disabled_until = Some(backoff.disabled_until);
        }
    }

    /// Record a failure and escalate the backoff level. Returns the persisted row.
    async fn record_failure(
        &self,
        indexer_id: &str,
        retry_after: Option<std::time::Duration>,
    ) -> IndexerSystemBackoff {
        let mut map = self.state.lock().await;
        let state = map
            .entry(indexer_id.to_string())
            .or_insert(IndexerBackoffState {
                escalation_level: 0,
                disabled_until: None,
            });

        if let Some(until) = state.disabled_until
            && until > chrono::Utc::now()
        {
            return IndexerSystemBackoff {
                disabled_until: until,
                escalation_level: state.escalation_level,
            };
        }

        let period_index = state.escalation_level.min(BACKOFF_PERIODS_SECS.len() - 1);
        let backoff_secs = retry_after
            .map(|duration| duration.as_secs())
            .unwrap_or(BACKOFF_PERIODS_SECS[period_index]);
        let backoff_secs = backoff_secs.min(i64::MAX as u64) as i64;
        let until = chrono::Utc::now() + chrono::Duration::seconds(backoff_secs);

        state.escalation_level = (state.escalation_level + 1).min(BACKOFF_PERIODS_SECS.len());
        state.disabled_until = Some(until);

        IndexerSystemBackoff {
            disabled_until: until,
            escalation_level: state.escalation_level,
        }
    }

    /// Record a success and de-escalate by one level. Returns true when local
    /// backoff state existed and may need persistent cleanup.
    async fn record_success(&self, indexer_id: &str) -> bool {
        let mut map = self.state.lock().await;
        if let Some(state) = map.get_mut(indexer_id) {
            state.escalation_level = state.escalation_level.saturating_sub(1);
            if state.escalation_level == 0 {
                state.disabled_until = None;
            }
            true
        } else {
            false
        }
    }

    /// Check if this indexer is currently in backoff.
    async fn is_disabled(&self, indexer_id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        let map = self.state.lock().await;
        map.get(indexer_id)
            .and_then(|s| s.disabled_until)
            .filter(|until| *until > chrono::Utc::now())
    }
}

/// Short-lived cache for RSS feed results. Multiple concurrent callers
/// awaiting the same indexer's feed will share a single HTTP fetch.
type RssFeedCache =
    Arc<Mutex<HashMap<String, Arc<tokio::sync::OnceCell<Vec<IndexerSearchResult>>>>>>;

#[derive(Clone)]
pub struct MultiIndexerSearchClient {
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    stats_tracker: Arc<dyn IndexerStatsTracker>,
    plugin_provider: Arc<dyn IndexerPluginProvider>,
    rate_limiter: IndexerRateLimiter,
    backoff_tracker: IndexerBackoffTracker,
    rss_feed_cache: RssFeedCache,
    background_search_limit: Arc<Semaphore>,
    interactive_search_limit: Arc<Semaphore>,
}

impl MultiIndexerSearchClient {
    pub fn new(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        stats_tracker: Arc<dyn IndexerStatsTracker>,
        plugin_provider: Arc<dyn IndexerPluginProvider>,
    ) -> Self {
        Self {
            indexer_configs,
            stats_tracker,
            plugin_provider,
            rate_limiter: IndexerRateLimiter::new(),
            backoff_tracker: IndexerBackoffTracker::new(),
            rss_feed_cache: Arc::new(Mutex::new(HashMap::new())),
            background_search_limit: Arc::new(Semaphore::new(
                BACKGROUND_INDEXER_SEARCH_CONCURRENCY_LIMIT,
            )),
            interactive_search_limit: Arc::new(Semaphore::new(
                INTERACTIVE_INDEXER_SEARCH_CONCURRENCY_LIMIT,
            )),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "test and direct callers use the same search envelope as the IndexerClient trait"
    )]
    pub async fn search(
        &self,
        query: String,
        ids: HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<scryer_domain::TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        <Self as IndexerClient>::search(
            self,
            query,
            ids,
            category,
            facet,
            id_search_facet,
            newznab_categories,
            indexer_routing,
            mode,
            season,
            episode,
            absolute_episode,
            tagged_aliases,
            CancellationToken::new(),
        )
        .await
    }

    fn search_limit_for_mode(&self, mode: SearchMode) -> Arc<Semaphore> {
        if matches!(mode, SearchMode::Interactive) {
            self.interactive_search_limit.clone()
        } else {
            self.background_search_limit.clone()
        }
    }

    fn client_from_config(
        config: &IndexerConfig,
        plugin_provider: &Arc<dyn IndexerPluginProvider>,
    ) -> AppResult<Arc<dyn IndexerClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();

        if let Some(client) = plugin_provider.client_for_provider(config) {
            return Ok(client);
        }

        Err(AppError::Validation(format!(
            "unsupported indexer provider: '{provider}'"
        )))
    }

    async fn record_indexer_last_error(
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
    ) {
        if let Err(error) = indexer_configs.touch_last_error(indexer_id).await {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to update indexer last_error_at"
            );
        }
    }

    async fn clear_indexer_last_error(
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
    ) {
        if let Err(error) = indexer_configs.clear_last_error(indexer_id).await {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to clear indexer last_error_at"
            );
        }
    }

    async fn record_indexer_system_backoff(
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
        backoff: IndexerSystemBackoff,
    ) {
        if let Err(error) = indexer_configs
            .set_system_backoff(indexer_id, backoff)
            .await
        {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to persist indexer system backoff"
            );
        }
    }

    async fn clear_indexer_system_backoff(
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
    ) {
        if let Err(error) = indexer_configs.clear_system_backoff(indexer_id).await {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to clear indexer system backoff"
            );
        }
    }

    fn is_rss_sync_request(
        query: &str,
        ids_present: bool,
        filters_present: bool,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> bool {
        matches!(mode, SearchMode::Auto)
            && query.trim().is_empty()
            && !ids_present
            && !filters_present
            && season.is_none()
            && episode.is_none()
    }

    fn auto_mode_enabled(config: &IndexerConfig, is_rss_request: bool) -> bool {
        if !config.enable_auto_search {
            return false;
        }

        let Some(raw) = config.managed_metadata_json.as_deref() else {
            return true;
        };
        let Ok(metadata) = serde_json::from_str::<ManagedIndexerMetadata>(raw) else {
            return true;
        };

        if is_rss_request {
            metadata.enable_rss.unwrap_or(true)
        } else {
            metadata.enable_automatic_search.unwrap_or(true)
        }
    }

    fn resolve_search_capabilities(
        config: &IndexerConfig,
        static_caps: &IndexerProviderCapabilities,
        query_facet: &str,
        id_facet: &str,
    ) -> ResolvedSearchCapabilities {
        let transport_kind = config.nab_transport_kind();
        if transport_kind.is_none() {
            return ResolvedSearchCapabilities {
                caps: static_caps.clone(),
                id_dispatch_mode: IdDispatchMode::LegacyAggregate,
                text_dispatch_mode: text_dispatch_mode_for_static(static_caps, query_facet),
                query_only_reason: None,
                transport_kind: None,
                caps_source: "static",
            };
        }

        let snapshot = stored_caps_snapshot(config);
        match transport_kind {
            Some(NabTransportKind::DirectNab) if snapshot.is_none() => {
                return ResolvedSearchCapabilities {
                    caps: static_caps.clone(),
                    id_dispatch_mode: IdDispatchMode::LegacyAggregate,
                    text_dispatch_mode: text_dispatch_mode_for_static(static_caps, query_facet),
                    query_only_reason: None,
                    transport_kind,
                    caps_source: "legacy_static",
                };
            }
            Some(NabTransportKind::ProwlarrNabProxy) if snapshot.is_none() => {
                return ResolvedSearchCapabilities {
                    caps: IndexerProviderCapabilities {
                        supported_ids: HashMap::new(),
                        search_inputs: static_caps.search_inputs.clone(),
                        supported_external_ids: Vec::new(),
                        query_param: static_caps.query_param.clone(),
                        ..static_caps.clone()
                    },
                    id_dispatch_mode: IdDispatchMode::QueryOnly,
                    text_dispatch_mode: if static_caps.query_param.is_some() {
                        TextDispatchMode::GenericOnly
                    } else {
                        TextDispatchMode::None
                    },
                    query_only_reason: Some("caps snapshot unavailable"),
                    transport_kind,
                    caps_source: "query_only_fallback",
                };
            }
            _ => {}
        }

        let Some(snapshot) = snapshot.as_ref() else {
            return ResolvedSearchCapabilities {
                caps: static_caps.clone(),
                id_dispatch_mode: IdDispatchMode::LegacyAggregate,
                text_dispatch_mode: text_dispatch_mode_for_static(static_caps, query_facet),
                query_only_reason: None,
                transport_kind,
                caps_source: "static",
            };
        };

        let mut caps = static_caps.clone();
        caps.supported_ids = supported_ids_from_caps_snapshot(snapshot);
        let text_dispatch_mode = caps_snapshot_text_dispatch_mode(snapshot, query_facet);
        caps.query_param = text_dispatch_mode.can_dispatch().then_some("q".to_string());
        caps.supported_query_facets = if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        {
            vec![query_facet.to_string()]
        } else {
            Vec::new()
        };
        caps.search_inputs = caps_search_inputs(snapshot, query_facet);
        caps.supported_external_ids = supported_external_ids_from_caps_snapshot(snapshot);
        caps.season_param = node_supports_param(snapshot.tv_search.as_ref(), "season")
            .then_some("season".to_string());
        caps.episode_param =
            node_supports_param(snapshot.tv_search.as_ref(), "ep").then_some("ep".to_string());

        let id_dispatch_mode = if caps.has_facet(id_facet) {
            IdDispatchMode::Aggregate
        } else {
            IdDispatchMode::QueryOnly
        };
        let query_only_reason = (id_dispatch_mode == IdDispatchMode::QueryOnly)
            .then_some("no actionable IDs in caps snapshot");

        ResolvedSearchCapabilities {
            caps,
            id_dispatch_mode,
            text_dispatch_mode,
            query_only_reason,
            transport_kind,
            caps_source: "snapshot",
        }
    }

    fn is_prowlarr_nab_proxy(config: &IndexerConfig) -> bool {
        config.is_prowlarr_nab_proxy()
    }

    fn default_newznab_categories_for_facet(facet: &str) -> Option<Vec<String>> {
        let categories = match facet {
            "movie" => &["2000"][..],
            "series" => &["5000"][..],
            "anime" => &["5070"][..],
            _ => &[][..],
        };
        (!categories.is_empty()).then(|| {
            categories
                .iter()
                .map(|value| (*value).to_string())
                .collect()
        })
    }

    fn split_rss_category_requests(categories: Option<Vec<String>>) -> Vec<Option<Vec<String>>> {
        let normalized: Vec<String> = categories
            .unwrap_or_default()
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        if normalized.is_empty() {
            vec![None]
        } else if normalized.len() == 1 {
            vec![Some(normalized)]
        } else {
            normalized
                .into_iter()
                .map(|value| Some(vec![value]))
                .collect()
        }
    }

    fn rss_feed_cache_key(indexer_id: &str, categories: Option<&[String]>) -> String {
        match categories {
            Some(categories) if !categories.is_empty() => {
                format!("{indexer_id}:{}", categories.join(","))
            }
            _ => indexer_id.to_string(),
        }
    }

    async fn execute_strategy_tier(
        context: StrategyTierContext,
        strategies: Vec<SearchStrategy>,
    ) -> Vec<StrategyExecutionOutcome> {
        let mut set = tokio::task::JoinSet::<StrategyExecutionOutcome>::new();

        for strategy in strategies {
            let context = context.clone();
            let strategy_label = strategy.label.clone();
            let title_guard_mode =
                if !strategy.ids.is_empty() || strategy.request_query.trim().is_empty() {
                    TitleGuardMode::SkipTitleMatch
                } else {
                    TitleGuardMode::ExactTitleMatch
                };

            set.spawn(async move {
                let StrategyTierContext {
                    client,
                    search_limit,
                    rate_limiter,
                    indexer_id,
                    rate_limit_seconds,
                    category,
                    per_indexer_categories,
                    mode,
                    tagged_aliases,
                    cancel_token,
                } = context;
                let permit = tokio::select! {
                    _ = cancel_token.cancelled() => {
                        return StrategyExecutionOutcome {
                            label: strategy_label,
                            title_guard_mode,
                            response: Err(AppError::canceled("indexer strategy canceled")),
                            elapsed: std::time::Duration::ZERO,
                            retry_after: None,
                        };
                    }
                    permit = search_limit.acquire_owned() => permit,
                };
                let response = match permit {
                    Ok(_permit) => {
                        tokio::select! {
                            _ = cancel_token.cancelled() => {
                                return StrategyExecutionOutcome {
                                    label: strategy_label,
                                    title_guard_mode,
                                    response: Err(AppError::canceled("indexer strategy canceled")),
                                    elapsed: std::time::Duration::ZERO,
                                    retry_after: None,
                                };
                            }
                            _ = rate_limiter.acquire(&indexer_id, rate_limit_seconds, mode) => {}
                        }
                        let start = std::time::Instant::now();
                        let request_cancel_token = cancel_token.child_token();
                        let response = tokio::select! {
                            _ = cancel_token.cancelled() => Err(AppError::canceled("indexer strategy canceled")),
                            response = tokio::time::timeout(
                                std::time::Duration::from_secs(INDEXER_SEARCH_TIMEOUT_SECS),
                                client.search(
                                    strategy.request_query,
                                    strategy.ids,
                                    if strategy.generic_query_only {
                                        None
                                    } else {
                                        category
                                    },
                                    if strategy.generic_query_only {
                                        None
                                    } else {
                                        Some(strategy.request_facet)
                                    },
                                    None,
                                    if strategy.generic_query_only {
                                        None
                                    } else {
                                        per_indexer_categories
                                    },
                                    None,
                                    mode,
                                    if strategy.generic_query_only {
                                        None
                                    } else {
                                        strategy.season
                                    },
                                    if strategy.generic_query_only {
                                        None
                                    } else {
                                        strategy.episode
                                    },
                                    if strategy.generic_query_only {
                                        None
                                    } else {
                                        strategy.absolute_episode
                                    },
                                    tagged_aliases,
                                    request_cancel_token,
                                ),
                            ) => response.unwrap_or_else(|_| {
                                Err(AppError::Repository("indexer search timed out".into()))
                            }),
                        };
                        let retry_after = response.as_ref().err().and_then(retry_after_from_error);

                        return StrategyExecutionOutcome {
                            label: strategy_label,
                            title_guard_mode,
                            response,
                            elapsed: start.elapsed(),
                            retry_after,
                        };
                    }
                    Err(error) => Err(AppError::Repository(format!(
                        "indexer search limiter closed: {error}"
                    ))),
                };

                StrategyExecutionOutcome {
                    label: strategy_label,
                    title_guard_mode,
                    response,
                    elapsed: std::time::Duration::ZERO,
                    retry_after: None,
                }
            });
        }

        let mut outcomes = Vec::new();
        loop {
            let join_result = tokio::select! {
                _ = context.cancel_token.cancelled() => {
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    outcomes.push(StrategyExecutionOutcome {
                        label: "cancel".into(),
                        title_guard_mode: TitleGuardMode::SkipTitleMatch,
                        response: Err(AppError::canceled("indexer strategy tier canceled")),
                        elapsed: std::time::Duration::ZERO,
                        retry_after: None,
                    });
                    break;
                }
                join_result = set.join_next() => join_result,
            };

            let Some(join_result) = join_result else {
                break;
            };

            match join_result {
                Ok(outcome) => outcomes.push(outcome),
                Err(error) => outcomes.push(StrategyExecutionOutcome {
                    label: "join".into(),
                    title_guard_mode: TitleGuardMode::SkipTitleMatch,
                    response: Err(AppError::Repository(format!(
                        "indexer search task panicked: {error}"
                    ))),
                    elapsed: std::time::Duration::ZERO,
                    retry_after: None,
                }),
            }
        }

        outcomes
    }
}

#[async_trait]
impl IndexerClient for MultiIndexerSearchClient {
    async fn search(
        &self,
        query: String,
        ids: HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<scryer_domain::TaggedAlias>,
        cancel_token: CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        if cancel_token.is_cancelled() {
            return Err(AppError::canceled("indexer search canceled"));
        }
        let is_rss_request = Self::is_rss_sync_request(
            &query,
            !ids.is_empty(),
            category
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty()),
            mode,
            season,
            episode,
        );

        let configs = self.indexer_configs.list(None).await.unwrap_or_else(|err| {
            warn!(error = %err, "failed to load indexer configs");
            vec![]
        });

        let now = chrono::Utc::now();
        let system_backoffs = self
            .indexer_configs
            .list_system_backoffs()
            .await
            .unwrap_or_else(|err| {
                warn!(error = %err, "failed to load persisted indexer system backoffs");
                HashMap::new()
            });

        // Filter by is_enabled, search mode flag, disabled_until (config), and backoff state
        let mut enabled: Vec<(&IndexerConfig, bool)> = Vec::new();
        for c in &configs {
            if !c.is_enabled {
                log_indexer_skip(mode, c.name.as_str(), "disabled", None);
                continue;
            }
            // Check persistent disabled_until from config
            if let Some(until) = c.disabled_until
                && until > now
            {
                log_indexer_skip(
                    mode,
                    c.name.as_str(),
                    "temporarily disabled (config)",
                    Some(until),
                );
                continue;
            }
            let persisted_system_backoff = system_backoffs.get(&c.id).cloned();
            if let Some(backoff) = persisted_system_backoff.as_ref() {
                self.backoff_tracker.seed_persisted(&c.id, backoff).await;
            }
            let had_persisted_system_backoff = persisted_system_backoff.is_some();
            if let Some(backoff) = persisted_system_backoff.as_ref()
                && backoff.disabled_until > now
            {
                log_indexer_skip(
                    mode,
                    c.name.as_str(),
                    "temporarily disabled (system backoff)",
                    Some(backoff.disabled_until),
                );
                continue;
            }
            // Check in-memory backoff escalation
            if let Some(until) = self.backoff_tracker.is_disabled(&c.id).await {
                log_indexer_skip(
                    mode,
                    c.name.as_str(),
                    "temporarily disabled (backoff)",
                    Some(until),
                );
                continue;
            }
            let mode_ok = match mode {
                SearchMode::Interactive => c.enable_interactive_search,
                SearchMode::Auto => Self::auto_mode_enabled(c, is_rss_request),
            };
            if mode_ok {
                enabled.push((c, had_persisted_system_backoff));
            } else {
                log_indexer_skip(mode, c.name.as_str(), "disabled for search mode", None);
            }
        }

        if enabled.is_empty() {
            info!(mode = ?mode, "no enabled indexer configs found");
            return Ok(IndexerSearchResponse {
                results: vec![],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            });
        }

        debug!(
            mode = ?mode,
            count = enabled.len(),
            indexers = ?enabled.iter().map(|(c, _)| c.name.as_str()).collect::<Vec<_>>(),
            "dispatching search to indexers"
        );

        let facet = match facet
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some("movie") | Some("series") | Some("anime") => facet.unwrap(),
            Some(other) => {
                return Err(AppError::Validation(format!(
                    "unsupported search facet: {other}"
                )));
            }
            None if is_rss_request => "series".to_string(),
            None => {
                return Err(AppError::Validation("search facet is required".to_string()));
            }
        };
        let id_search_facet = match id_search_facet
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value @ ("movie" | "series" | "anime")) => value.to_string(),
            Some(other) => {
                return Err(AppError::Validation(format!(
                    "unsupported ID search facet: {other}"
                )));
            }
            None => facet.clone(),
        };

        tracing::debug!(
            %facet,
            %id_search_facet,
            ?category,
            ?ids,
            ?season,
            ?episode,
            ?absolute_episode,
            %query,
            "search context"
        );
        let available_ids = ids;

        // Spawn parallel searches across enabled indexers, applying per-indexer routing.
        // Each indexer may still execute multiple strategies internally, but for
        // TV/series searches we run ID searches first and only fall back to a
        // broad freetext query if that indexer returned no releases.
        let mut set =
            tokio::task::JoinSet::<(String, String, AppResult<IndexerSearchResponse>)>::new();
        let search_limit = self.search_limit_for_mode(mode);
        for (config, had_persisted_system_backoff) in enabled {
            // Apply per-indexer facet scoping: if routing is configured and this
            // indexer is disabled for the current scope, skip it entirely.
            let routing_entry = indexer_routing
                .as_ref()
                .and_then(|plan| plan.entries.get(&config.id));

            if let Some(entry) = routing_entry
                && !entry.enabled
            {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: disabled for scope via routing config"
                );
                continue;
            }

            // Use per-indexer categories from routing if available. Prowlarr
            // proxy children may fall back to per-facet defaults when no
            // routed categories exist yet; direct *nab indexers stay broad.
            let per_indexer_categories = routing_entry
                .map(|entry| {
                    if entry.categories.is_empty() {
                        if Self::is_prowlarr_nab_proxy(config) {
                            newznab_categories
                                .clone()
                                .or_else(|| Self::default_newznab_categories_for_facet(&facet))
                        } else {
                            newznab_categories.clone()
                        }
                    } else {
                        Some(entry.categories.clone())
                    }
                })
                .unwrap_or_else(|| {
                    if Self::is_prowlarr_nab_proxy(config) {
                        newznab_categories
                            .clone()
                            .or_else(|| Self::default_newznab_categories_for_facet(&facet))
                    } else {
                        newznab_categories.clone()
                    }
                });
            let rss_category_requests = if is_rss_request {
                Self::split_rss_category_requests(per_indexer_categories.clone())
            } else {
                vec![per_indexer_categories.clone()]
            };

            // Skip indexers at or near their API quota for auto searches.
            if mode == SearchMode::Auto && self.stats_tracker.is_at_quota(&config.id) {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: at API quota limit"
                );
                continue;
            }

            let static_caps = self
                .plugin_provider
                .capabilities_for_provider(&config.provider_type);
            let resolved_caps =
                Self::resolve_search_capabilities(config, &static_caps, &facet, &id_search_facet);
            let caps = resolved_caps.caps.clone();
            debug!(
                indexer = config.name.as_str(),
                transport = resolved_caps
                    .transport_kind
                    .map(|kind| kind.as_str())
                    .unwrap_or("other"),
                caps_source = resolved_caps.caps_source,
                "resolved effective indexer search capabilities"
            );

            // RSS-only check: skip non-RSS indexers for RSS sync requests
            if is_rss_request && !caps.rss {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support RSS sync"
                );
                continue;
            }

            let eligible_ids =
                filter_ids_for_types(&available_ids, caps.id_types_for_facet(&id_search_facet));
            let can_dispatch_id = !eligible_ids.is_empty()
                && caps.has_facet(&id_search_facet)
                && !matches!(resolved_caps.id_dispatch_mode, IdDispatchMode::QueryOnly);
            let can_dispatch_text =
                !query.trim().is_empty() && resolved_caps.text_dispatch_mode.can_dispatch();
            if !is_rss_request && !can_dispatch_id && !can_dispatch_text {
                info!(
                    indexer = config.name.as_str(),
                    facet, "skipping indexer: no supported IDs for facet and no freetext"
                );
                continue;
            }

            if matches!(resolved_caps.id_dispatch_mode, IdDispatchMode::QueryOnly)
                && let Some(reason) = resolved_caps.query_only_reason
            {
                info!(
                    indexer = config.name.as_str(),
                    reason, "ProwlarrNabProxy running in query-only fallback mode"
                );
            }

            if matches!(
                resolved_caps.id_dispatch_mode,
                IdDispatchMode::Aggregate | IdDispatchMode::QueryOnly
            ) {
                let extra_ids = available_ids
                    .keys()
                    .filter(|id_type| !eligible_ids.contains_key(*id_type))
                    .cloned()
                    .collect::<Vec<_>>();
                if !available_ids.is_empty() {
                    debug!(
                        indexer = config.name.as_str(),
                        facet,
                        id_search_facet,
                        eligible_ids = ?eligible_ids.keys().collect::<Vec<_>>(),
                        carried_ids = ?available_ids.keys().collect::<Vec<_>>(),
                        extra_ids = ?extra_ids,
                        "ID strategy capability resolved; carrying full ID envelope when strategy runs"
                    );
                }
            }

            let client = match Self::client_from_config(config, &self.plugin_provider) {
                Ok(c) => c,
                Err(err) => {
                    warn!(
                        indexer = config.name.as_str(),
                        error = %err,
                        "skipping indexer: unsupported provider"
                    );
                    continue;
                }
            };

            // RSS-only indexers: fetch the feed once, cache it, return cached
            // results for all concurrent callers. The feed content is the same
            // regardless of query — the caller matches results downstream.
            let is_rss_only = !caps.supports_any_search() && caps.rss;
            if is_rss_only {
                for rss_category_request in rss_category_requests {
                    let cell = {
                        let mut cache = self.rss_feed_cache.lock().await;
                        cache
                            .entry(Self::rss_feed_cache_key(
                                &config.id,
                                rss_category_request.as_deref(),
                            ))
                            .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new()))
                            .clone()
                    };
                    let client = client.clone();
                    let query = query.clone();
                    let category = category.clone();
                    let tagged_aliases = tagged_aliases.clone();
                    let indexer_id = config.id.clone();
                    let indexer_name = config.name.clone();
                    let rate_limiter = self.rate_limiter.clone();
                    let rate_limit_seconds = config.rate_limit_seconds;
                    let stats_tracker = self.stats_tracker.clone();
                    let backoff_tracker = self.backoff_tracker.clone();
                    let indexer_configs = self.indexer_configs.clone();
                    let facet = facet.clone();
                    let search_limit = search_limit.clone();
                    let task_cancel_token = cancel_token.child_token();

                    set.spawn(async move {
                        let results = tokio::select! {
                            _ = task_cancel_token.cancelled() => {
                                return (
                                    indexer_id,
                                    indexer_name,
                                    Err(AppError::canceled("RSS indexer search canceled")),
                                );
                            }
                            results = cell.get_or_init(|| async {
                                let permit = tokio::select! {
                                    _ = task_cancel_token.cancelled() => return vec![],
                                    permit = search_limit.acquire_owned() => permit,
                                };
                                let search_result = match permit {
                                    Ok(_permit) => {
                                        tokio::select! {
                                            _ = task_cancel_token.cancelled() => return vec![],
                                            _ = rate_limiter.acquire(&indexer_id, rate_limit_seconds, mode) => {}
                                        }
                                        let start = std::time::Instant::now();
                                        let request_cancel_token = task_cancel_token.child_token();
                                        let response = tokio::select! {
                                            _ = task_cancel_token.cancelled() => {
                                                return vec![];
                                            }
                                            response = tokio::time::timeout(
                                                std::time::Duration::from_secs(
                                                    INDEXER_SEARCH_TIMEOUT_SECS,
                                                ),
                                                client.search(
                                                    query,
                                                    HashMap::new(),
                                                    category,
                                                    Some(facet),
                                                    None,
                                                    rss_category_request.clone(),
                                                    None,
                                                    mode,
                                                    season,
                                                    episode,
                                                    absolute_episode,
                                                    tagged_aliases,
                                                    request_cancel_token,
                                                ),
                                            ) => response,
                                        };
                                        (response, start.elapsed())
                                    }
                                    Err(error) => {
                                        warn!(indexer = indexer_name.as_str(), error = %error, "RSS feed search limiter closed");
                                        return vec![];
                                    }
                                };
                                let (search_response, elapsed) = search_result;

                                match search_response {
                                    Ok(Ok(response)) => {
                                        info!(indexer = indexer_name.as_str(), count = response.results.len(), "RSS feed cached");
                                        stats_tracker.record_query(&indexer_id, &indexer_name, true);
                                        let had_in_memory_backoff = backoff_tracker.record_success(&indexer_id).await;
                                        if had_in_memory_backoff || had_persisted_system_backoff {
                                            Self::clear_indexer_system_backoff(
                                                &indexer_configs,
                                                &indexer_id,
                                                &indexer_name,
                                            )
                                            .await;
                                        }
                                        Self::clear_indexer_last_error(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                        )
                                        .await;
                                        metrics::counter!("scryer_indexer_queries_total", "indexer" => indexer_name.clone(), "status" => "success", "mode" => "rss_cached").increment(1);
                                        metrics::histogram!("scryer_indexer_query_duration_seconds", "indexer" => indexer_name.clone(), "mode" => "rss_cached").record(elapsed.as_secs_f64());
                                        response.results
                                    }
                                    Ok(Err(err)) => {
                                        if err.is_canceled() {
                                            return vec![];
                                        }
                                        warn!(indexer = indexer_name.as_str(), error = %err, "RSS feed fetch failed");
                                        stats_tracker.record_query(&indexer_id, &indexer_name, false);
                                        let backoff = backoff_tracker
                                            .record_failure(
                                                &indexer_id,
                                                retry_after_from_error(&err),
                                            )
                                            .await;
                                        Self::record_indexer_system_backoff(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                            backoff,
                                        )
                                        .await;
                                        Self::record_indexer_last_error(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                        )
                                        .await;
                                        vec![]
                                    }
                                    Err(_) => {
                                        warn!(indexer = indexer_name.as_str(), "RSS feed fetch timed out");
                                        stats_tracker.record_query(&indexer_id, &indexer_name, false);
                                        let backoff =
                                            backoff_tracker.record_failure(&indexer_id, None).await;
                                        Self::record_indexer_system_backoff(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                            backoff,
                                        )
                                        .await;
                                        Self::record_indexer_last_error(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                        )
                                        .await;
                                        vec![]
                                    }
                                }
                            }) => results,
                        };
                        if task_cancel_token.is_cancelled() {
                            return (
                                indexer_id,
                                indexer_name,
                                Err(AppError::canceled("RSS indexer search canceled")),
                            );
                        }

                        let response = IndexerSearchResponse {
                            results: results.clone(),
                            api_current: None,
                            api_max: None,
                            grab_current: None,
                            grab_max: None,
                        };
                        (indexer_id, indexer_name, Ok(response))
                    });
                }
                continue;
            }

            let mut strategies: Vec<SearchStrategy> = build_strategies(&StrategyParams {
                query: &query,
                query_facet: &facet,
                id_facet: &id_search_facet,
                ids: &available_ids,
                season,
                episode,
                absolute_episode,
                caps: &caps,
                id_dispatch_mode: resolved_caps.id_dispatch_mode,
                text_dispatch_mode: resolved_caps.text_dispatch_mode,
                is_alias_query: false,
            });

            if facet == "anime"
                && let Some(alias_query) = preferred_anime_alias_query(&query, &tagged_aliases)
            {
                let alias_strategies = build_strategies(&StrategyParams {
                    query: &alias_query,
                    query_facet: &facet,
                    id_facet: &id_search_facet,
                    ids: &available_ids,
                    season,
                    episode,
                    absolute_episode,
                    caps: &caps,
                    id_dispatch_mode: resolved_caps.id_dispatch_mode,
                    text_dispatch_mode: resolved_caps.text_dispatch_mode,
                    is_alias_query: true,
                });

                strategies.extend(alias_strategies);
            }
            if is_rss_request && strategies.is_empty() {
                strategies.push(SearchStrategy {
                    request_query: String::new(),
                    request_facet: facet.clone(),
                    ids: HashMap::new(),
                    season: None,
                    episode: None,
                    absolute_episode: None,
                    generic_query_only: false,
                    label: "rss".into(),
                });
            }
            let (primary_strategies, fallback_strategies) =
                split_strategy_tiers(mode, &facet, strategies);
            if mode == SearchMode::Auto {
                record_auto_strategy_selection(
                    config.name.as_str(),
                    resolved_caps.caps_source,
                    &primary_strategies,
                    &fallback_strategies,
                );
            }

            for rss_category_request in rss_category_requests {
                let indexer_id = config.id.clone();
                let indexer_name = config.name.clone();
                let facet = facet.clone();
                let search_query = query.clone();
                let category_for_indexer = category.clone();
                let tagged_aliases_for_indexer = tagged_aliases.clone();
                let stats_tracker = self.stats_tracker.clone();
                let backoff_tracker = self.backoff_tracker.clone();
                let indexer_configs = self.indexer_configs.clone();
                let client = client.clone();
                let primary_strategies = primary_strategies.clone();
                let fallback_strategies = fallback_strategies.clone();
                let search_limit = search_limit.clone();
                let rate_limiter = self.rate_limiter.clone();
                let rate_limit_seconds = config.rate_limit_seconds;
                let task_cancel_token = cancel_token.child_token();

                set.spawn(async move {
                    if task_cancel_token.is_cancelled() {
                        return (
                            indexer_id,
                            indexer_name,
                            Err(AppError::canceled("indexer search canceled")),
                        );
                    }
                    let mut collected_results = Vec::new();
                    let mut primary_attempted = false;
                    let mut primary_had_error = false;
                    let mut batch_health = StrategyBatchHealth::default();

                    let primary_outcomes = Self::execute_strategy_tier(
                        StrategyTierContext {
                            client: client.clone(),
                            search_limit: search_limit.clone(),
                            rate_limiter: rate_limiter.clone(),
                            indexer_id: indexer_id.clone(),
                            rate_limit_seconds,
                            category: category_for_indexer.clone(),
                            per_indexer_categories: rss_category_request.clone(),
                            mode,
                            tagged_aliases: tagged_aliases_for_indexer.clone(),
                            cancel_token: task_cancel_token.child_token(),
                        },
                        primary_strategies,
                    )
                    .await;

                    for outcome in primary_outcomes {
                        primary_attempted = true;
                        match outcome.response {
                            Ok(mut response) => {
                                batch_health.mark_success();
                                debug!(
                                    indexer = indexer_name.as_str(),
                                    strategy = outcome.label.as_str(),
                                    count = response.results.len(),
                                    "indexer returned results"
                                );
                                stats_tracker.record_query(&indexer_id, &indexer_name, true);
                                stats_tracker.record_api_limits(
                                    &indexer_id,
                                    response.api_current,
                                    response.api_max,
                                    response.grab_current,
                                    response.grab_max,
                                );

                                record_strategy_metrics(
                                    &indexer_name,
                                    &outcome.label,
                                    "success",
                                    outcome.elapsed,
                                    Some(response.results.len()),
                                );

                                filter_strategy_results(
                                    &mut response.results,
                                    &FilterStrategyContext {
                                        query: &search_query,
                                        season,
                                        episode,
                                        tagged_aliases: &tagged_aliases_for_indexer,
                                        title_guard_mode: outcome.title_guard_mode,
                                        strategy_label: &outcome.label,
                                        is_rss_request,
                                    },
                                );
                                collected_results.append(&mut response.results);
                            }
                            Err(err) => {
                                if err.is_canceled() {
                                    return (
                                        indexer_id,
                                        indexer_name,
                                        Err(AppError::canceled("indexer search canceled")),
                                    );
                                }
                                primary_had_error = true;
                                batch_health.mark_error(outcome.retry_after);
                                debug!(
                                    indexer = indexer_name.as_str(),
                                    strategy = outcome.label.as_str(),
                                    error = %err,
                                    "indexer search failed"
                                );
                                stats_tracker.record_query(&indexer_id, &indexer_name, false);
                                Self::record_indexer_last_error(
                                    &indexer_configs,
                                    &indexer_id,
                                    &indexer_name,
                                )
                                .await;

                                record_strategy_metrics(
                                    &indexer_name,
                                    &outcome.label,
                                    "error",
                                    outcome.elapsed,
                                    None,
                                );
                            }
                        }
                    }

                    if should_run_fallback_tier(
                        mode,
                        &collected_results,
                        primary_attempted,
                        primary_had_error,
                        &fallback_strategies,
                    ) {
                        info!(
                            indexer = indexer_name.as_str(),
                            facet = facet.as_str(),
                            query = search_query.as_str(),
                            reason = "zero_usable_results",
                            "indexer search falling back to title tier"
                        );

                        let fallback_outcomes = Self::execute_strategy_tier(
                            StrategyTierContext {
                                client,
                                search_limit,
                                rate_limiter,
                                indexer_id: indexer_id.clone(),
                                rate_limit_seconds,
                                category: category_for_indexer,
                                per_indexer_categories: rss_category_request,
                                mode,
                                tagged_aliases: tagged_aliases_for_indexer.clone(),
                                cancel_token: task_cancel_token.child_token(),
                            },
                            fallback_strategies,
                        )
                        .await;

                        for outcome in fallback_outcomes {
                            match outcome.response {
                                Ok(mut response) => {
                                    batch_health.mark_success();
                                    debug!(
                                        indexer = indexer_name.as_str(),
                                        strategy = outcome.label.as_str(),
                                        count = response.results.len(),
                                        "indexer returned fallback results"
                                    );
                                    stats_tracker.record_query(&indexer_id, &indexer_name, true);
                                    stats_tracker.record_api_limits(
                                        &indexer_id,
                                        response.api_current,
                                        response.api_max,
                                        response.grab_current,
                                        response.grab_max,
                                    );

                                    record_strategy_metrics(
                                        &indexer_name,
                                        &outcome.label,
                                        "success",
                                        outcome.elapsed,
                                        Some(response.results.len()),
                                    );

                                    filter_strategy_results(
                                        &mut response.results,
                                        &FilterStrategyContext {
                                            query: &search_query,
                                            season,
                                            episode,
                                            tagged_aliases: &tagged_aliases_for_indexer,
                                            title_guard_mode: outcome.title_guard_mode,
                                            strategy_label: &outcome.label,
                                            is_rss_request,
                                        },
                                    );
                                    collected_results.append(&mut response.results);
                                }
                                Err(err) => {
                                    if err.is_canceled() {
                                        return (
                                            indexer_id,
                                            indexer_name,
                                            Err(AppError::canceled("indexer search canceled")),
                                        );
                                    }
                                    batch_health.mark_error(outcome.retry_after);
                                    debug!(
                                        indexer = indexer_name.as_str(),
                                        strategy = outcome.label.as_str(),
                                        error = %err,
                                        "indexer fallback search failed"
                                    );
                                    stats_tracker.record_query(&indexer_id, &indexer_name, false);
                                    Self::record_indexer_last_error(
                                        &indexer_configs,
                                        &indexer_id,
                                        &indexer_name,
                                    )
                                    .await;

                                    record_strategy_metrics(
                                        &indexer_name,
                                        &outcome.label,
                                        "error",
                                        outcome.elapsed,
                                        None,
                                    );
                                }
                            }
                        }
                    }

                    let batch_had_success = batch_health.any_success;
                    let batch_had_error = batch_health.any_error;
                    batch_health
                        .apply(
                            &backoff_tracker,
                            &indexer_configs,
                            &indexer_id,
                            &indexer_name,
                            had_persisted_system_backoff,
                        )
                        .await;
                    if batch_had_success {
                        Self::clear_indexer_last_error(
                            &indexer_configs,
                            &indexer_id,
                            &indexer_name,
                        )
                        .await;
                    }

                    if mode == SearchMode::Interactive
                        && collected_results.is_empty()
                        && !batch_had_success
                        && batch_had_error
                    {
                        return (
                            indexer_id,
                            indexer_name,
                            Err(AppError::Repository(
                                "all attempted indexer strategies failed".to_string(),
                            )),
                        );
                    }

                    (
                        indexer_id,
                        indexer_name,
                        Ok(IndexerSearchResponse {
                            results: collected_results,
                            api_current: None,
                            api_max: None,
                            grab_current: None,
                            grab_max: None,
                        }),
                    )
                });
            }
        }

        let mut all_results: Vec<IndexerSearchResult> = Vec::new();
        let mut successful_searches = 0usize;
        let mut failed_searches = 0usize;
        let mut first_failure: Option<String> = None;
        loop {
            let join_result = tokio::select! {
                _ = cancel_token.cancelled() => {
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    self.rss_feed_cache.lock().await.clear();
                    return Err(AppError::canceled("indexer search canceled"));
                }
                join_result = set.join_next() => join_result,
            };

            let Some(join_result) = join_result else {
                break;
            };

            match join_result {
                Ok((_id, name, Ok(mut response))) => {
                    successful_searches += 1;
                    debug!(
                        indexer = name.as_str(),
                        count = response.results.len(),
                        "indexer returned aggregated results"
                    );
                    all_results.append(&mut response.results);
                }
                Ok((id, name, Err(err))) => {
                    if err.is_canceled() {
                        set.abort_all();
                        while set.join_next().await.is_some() {}
                        self.rss_feed_cache.lock().await.clear();
                        return Err(err);
                    }
                    failed_searches += 1;
                    first_failure = first_failure.or_else(|| Some(err.to_string()));
                    warn!(indexer = name.as_str(), error = %err, "indexer search failed");
                    let _ = id;
                }
                Err(err) => {
                    failed_searches += 1;
                    first_failure = first_failure.or_else(|| Some(err.to_string()));
                    warn!(error = %err, "indexer search task panicked");
                }
            }
        }

        // Clear the RSS feed cache after all tasks complete so the next
        // search session gets fresh feeds.
        self.rss_feed_cache.lock().await.clear();

        // Dedup by download_url (exact duplicates from parallel strategies).
        // Cross-indexer release-identity dedup happens in the discovery layer
        // where download client preferences are available.
        {
            let before = all_results.len();
            let mut seen_urls: HashSet<String> = HashSet::new();
            all_results.retain(|r| {
                if let Some(ref url) = r.download_url {
                    seen_urls.insert(url.to_ascii_lowercase())
                } else {
                    true
                }
            });
            let deduped = before - all_results.len();
            if deduped > 0 {
                debug!(
                    before,
                    after = all_results.len(),
                    deduped,
                    "deduplicated search results by URL"
                );
            }
        }

        if all_results.is_empty()
            && successful_searches == 0
            && failed_searches > 0
            && mode == SearchMode::Interactive
        {
            return Err(AppError::Repository(first_failure.unwrap_or_else(|| {
                "all indexer search attempts failed".to_string()
            })));
        }

        for result in &mut all_results {
            if result.parsed_release_metadata.is_none() {
                result.parsed_release_metadata =
                    Some(scryer_application::parse_release_metadata(&result.title));
            }
        }

        Ok(IndexerSearchResponse {
            results: all_results,
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

/// Build parallel search strategies for interactive mode.
///
/// Uses the plugin's facet-scoped `supported_ids` to determine which ID-based
/// strategies to generate. Each strategy targets one ID type so the host can
/// dispatch them all in parallel.
struct StrategyParams<'a> {
    query: &'a str,
    query_facet: &'a str,
    id_facet: &'a str,
    ids: &'a HashMap<String, String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
    caps: &'a scryer_domain::IndexerProviderCapabilities,
    id_dispatch_mode: IdDispatchMode,
    text_dispatch_mode: TextDispatchMode,
    is_alias_query: bool,
}

/// The query facet controls text-search endpoint shape. The ID facet controls
/// which provider IDs are valid for ID-backed strategies.
fn build_strategies(p: &StrategyParams<'_>) -> Vec<SearchStrategy> {
    let query = p.query;
    let query_facet = p.query_facet;
    let id_facet = p.id_facet;
    let ids = p.ids;
    let season = p.season;
    let episode = p.episode;
    let absolute_episode = p.absolute_episode;
    let caps = p.caps;
    let id_dispatch_mode = p.id_dispatch_mode;
    let text_dispatch_mode = p.text_dispatch_mode;
    let is_alias_query = p.is_alias_query;
    let structured_season = season.filter(|_| caps.season_param.is_some());
    let structured_episode = episode.filter(|_| caps.episode_param.is_some());
    let supports_absolute_episode = caps.episode_param.is_some()
        || caps
            .search_inputs
            .contains(&IndexerSearchInputCapability::AbsoluteEpisode);
    let structured_absolute_episode = absolute_episode.filter(|_| supports_absolute_episode);
    // Alias queries skip indexers that deduplicate aliases internally
    if is_alias_query && caps.deduplicates_aliases {
        return vec![];
    }

    let mut strategies = Vec::with_capacity(4);

    let eligible_ids = filter_ids_for_types(ids, caps.id_types_for_facet(id_facet));
    if !eligible_ids.is_empty() && !is_alias_query {
        let full_ids = ids
            .iter()
            .filter(|(_, value)| !value.trim().is_empty())
            .map(|(id_type, value)| (id_type.clone(), value.clone()))
            .collect::<HashMap<_, _>>();
        let selected_ids = match id_dispatch_mode {
            IdDispatchMode::LegacyAggregate => full_ids,
            IdDispatchMode::Aggregate => eligible_ids.clone(),
            IdDispatchMode::QueryOnly => HashMap::new(),
        };
        if id_facet == "anime" && !selected_ids.is_empty() {
            if let Some(absolute_episode) = structured_absolute_episode {
                strategies.push(SearchStrategy {
                    request_query: String::new(),
                    request_facet: id_facet.to_string(),
                    ids: selected_ids.clone(),
                    season: None,
                    episode: None,
                    absolute_episode: Some(absolute_episode),
                    generic_query_only: false,
                    label: "ids_abs".into(),
                });
            }

            if structured_episode.is_some() {
                strategies.push(SearchStrategy {
                    request_query: String::new(),
                    request_facet: id_facet.to_string(),
                    ids: selected_ids.clone(),
                    season: structured_season,
                    episode: structured_episode,
                    absolute_episode: None,
                    generic_query_only: false,
                    label: "ids_sxex".into(),
                });
            }
        }

        if strategies.is_empty() && !selected_ids.is_empty() {
            strategies.push(SearchStrategy {
                request_query: String::new(),
                request_facet: id_facet.to_string(),
                ids: selected_ids,
                season: structured_season,
                episode: structured_episode,
                absolute_episode: structured_absolute_episode,
                generic_query_only: false,
                label: "ids".into(),
            });
        }
    }

    let generic_query_only = text_dispatch_mode.is_generic_only();
    let text_season = text_strategy_season(caps, text_dispatch_mode, season);
    let text_episode = text_strategy_episode(caps, text_dispatch_mode, episode);
    let text_absolute_episode =
        text_strategy_absolute_episode(caps, text_dispatch_mode, absolute_episode);
    if text_dispatch_mode.can_dispatch() && caps.query_param.is_some() && !query.is_empty() {
        strategies.push(SearchStrategy {
            request_query: query.to_string(),
            request_facet: query_facet.to_string(),
            ids: HashMap::new(),
            season: text_season,
            episode: text_episode,
            absolute_episode: text_absolute_episode,
            generic_query_only,
            label: if is_alias_query {
                "freetext_alias".into()
            } else {
                "freetext".into()
            },
        });
    }

    // If no strategies were generated, fall back to a single combined call
    if strategies.is_empty()
        && !query.is_empty()
        && caps.query_param.is_some()
        && text_dispatch_mode.can_dispatch()
    {
        strategies.push(SearchStrategy {
            request_query: query.to_string(),
            request_facet: query_facet.to_string(),
            ids: HashMap::new(),
            season: text_season,
            episode: text_episode,
            absolute_episode: text_absolute_episode,
            generic_query_only,
            label: "fallback".into(),
        });
    }

    strategies
}

fn stored_caps_snapshot(config: &IndexerConfig) -> Option<IndexerCapsSnapshot> {
    if let Some(raw) = config.caps_snapshot_json.as_deref()
        && let Ok(snapshot) = serde_json::from_str::<IndexerCapsSnapshot>(raw)
    {
        return Some(snapshot);
    }

    config
        .managed_metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<ManagedIndexerMetadata>(raw).ok())
        .and_then(|metadata| metadata.caps_snapshot)
}

fn supported_ids_from_caps_snapshot(
    snapshot: &IndexerCapsSnapshot,
) -> HashMap<String, Vec<String>> {
    let mut supported_ids = HashMap::new();

    let movie_ids = actionable_ids_for_node(snapshot.movie_search.as_ref(), "movie");
    if !movie_ids.is_empty() {
        supported_ids.insert("movie".to_string(), movie_ids);
    }

    let tv_ids = actionable_ids_for_node(snapshot.tv_search.as_ref(), "tv");
    if !tv_ids.is_empty() {
        supported_ids.insert("series".to_string(), tv_ids.clone());
        supported_ids.insert("anime".to_string(), tv_ids);
    }

    supported_ids
}

fn supported_external_ids_from_caps_snapshot(snapshot: &IndexerCapsSnapshot) -> Vec<String> {
    let mut ids = actionable_ids_for_node(snapshot.movie_search.as_ref(), "movie");
    ids.extend(actionable_ids_for_node(snapshot.tv_search.as_ref(), "tv"));
    ids.sort();
    ids.dedup();
    ids
}

fn text_dispatch_mode_for_static(
    caps: &IndexerProviderCapabilities,
    facet: &str,
) -> TextDispatchMode {
    if caps.supports_query_for_facet(facet) {
        TextDispatchMode::FacetScoped
    } else {
        TextDispatchMode::None
    }
}

fn actionable_ids_for_node(node: Option<&IndexerCapsSearchNode>, search_kind: &str) -> Vec<String> {
    let Some(node) = node else {
        return Vec::new();
    };
    if !node.available {
        return Vec::new();
    }

    actionable_ids_for_params(&node.supported_params, search_kind)
}

fn actionable_ids_for_params(params: &[String], search_kind: &str) -> Vec<String> {
    let mut ids = Vec::new();
    if params.iter().any(|param| param == "imdbid") {
        ids.push("imdb_id".to_string());
    }
    if params.iter().any(|param| param == "tvdbid") {
        ids.push("tvdb_id".to_string());
    }
    if params.iter().any(|param| param == "tmdbid") {
        ids.push("tmdb_id".to_string());
    }

    if search_kind == "movie" {
        ids.sort_by_key(|value| match value.as_str() {
            "tmdb_id" => 0,
            "imdb_id" => 1,
            _ => 2,
        });
    } else {
        ids.sort_by_key(|value| match value.as_str() {
            "tvdb_id" => 0,
            "imdb_id" => 1,
            "tmdb_id" => 2,
            _ => 3,
        });
    }

    ids.dedup();
    ids
}

fn node_supports_param(node: Option<&IndexerCapsSearchNode>, param: &str) -> bool {
    node.is_some_and(|node| {
        node.available
            && node
                .supported_params
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(param))
    })
}

fn caps_snapshot_has_facet_query(snapshot: &IndexerCapsSnapshot, facet: &str) -> bool {
    match facet {
        "movie" => node_supports_param(snapshot.movie_search.as_ref(), "q"),
        "series" | "anime" => node_supports_param(snapshot.tv_search.as_ref(), "q"),
        _ => false,
    }
}

fn caps_snapshot_has_generic_query(snapshot: &IndexerCapsSnapshot) -> bool {
    node_supports_param(snapshot.search.as_ref(), "q")
}

fn caps_snapshot_text_dispatch_mode(
    snapshot: &IndexerCapsSnapshot,
    facet: &str,
) -> TextDispatchMode {
    if caps_snapshot_has_facet_query(snapshot, facet) {
        TextDispatchMode::FacetScoped
    } else if caps_snapshot_has_generic_query(snapshot) {
        TextDispatchMode::GenericOnly
    } else {
        TextDispatchMode::None
    }
}

fn caps_search_inputs(
    snapshot: &IndexerCapsSnapshot,
    facet: &str,
) -> Vec<scryer_domain::IndexerSearchInputCapability> {
    let mut inputs = Vec::new();
    if caps_snapshot_text_dispatch_mode(snapshot, facet).can_dispatch() {
        inputs.push(scryer_domain::IndexerSearchInputCapability::TitleQuery);
    }

    let facet_ids = supported_ids_from_caps_snapshot(snapshot);
    if facet_ids.get(facet).is_some_and(|ids| !ids.is_empty()) {
        inputs.push(scryer_domain::IndexerSearchInputCapability::IdQuery);
        inputs.push(scryer_domain::IndexerSearchInputCapability::AggregateIdQuery);
    }

    if node_supports_param(snapshot.tv_search.as_ref(), "season") {
        inputs.push(scryer_domain::IndexerSearchInputCapability::Season);
    }
    if node_supports_param(snapshot.tv_search.as_ref(), "ep") {
        inputs.push(scryer_domain::IndexerSearchInputCapability::Episode);
    }

    inputs
}

fn supports_search_input_or_legacy(
    caps: &IndexerProviderCapabilities,
    input: scryer_domain::IndexerSearchInputCapability,
) -> bool {
    caps.search_inputs.is_empty() || caps.search_inputs.contains(&input)
}

fn text_strategy_season(
    caps: &IndexerProviderCapabilities,
    text_dispatch_mode: TextDispatchMode,
    season: Option<u32>,
) -> Option<u32> {
    if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        && supports_search_input_or_legacy(
            caps,
            scryer_domain::IndexerSearchInputCapability::Season,
        )
    {
        season
    } else {
        None
    }
}

fn text_strategy_episode(
    caps: &IndexerProviderCapabilities,
    text_dispatch_mode: TextDispatchMode,
    episode: Option<u32>,
) -> Option<u32> {
    if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        && supports_search_input_or_legacy(
            caps,
            scryer_domain::IndexerSearchInputCapability::Episode,
        )
    {
        episode
    } else {
        None
    }
}

fn text_strategy_absolute_episode(
    caps: &IndexerProviderCapabilities,
    text_dispatch_mode: TextDispatchMode,
    absolute_episode: Option<u32>,
) -> Option<u32> {
    if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        && caps
            .search_inputs
            .contains(&scryer_domain::IndexerSearchInputCapability::AbsoluteEpisode)
    {
        absolute_episode
    } else {
        None
    }
}

fn filter_ids_for_types(
    ids: &HashMap<String, String>,
    supported_types: &[String],
) -> HashMap<String, String> {
    if supported_types.is_empty() {
        return HashMap::new();
    }

    let supported_types: HashSet<&str> = supported_types.iter().map(String::as_str).collect();
    ids.iter()
        .filter(|(id_type, value)| {
            supported_types.contains(id_type.as_str()) && !value.trim().is_empty()
        })
        .map(|(id_type, value)| (id_type.clone(), value.clone()))
        .collect()
}

/// Normalize a title for substring comparison: lowercase, alpha-only, no spaces.
fn normalize_for_comparison(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Returns the normalized titles that can legitimately identify this parsed
/// release. The title guard uses exact matches against this set to reject
/// nearby-but-wrong releases like "Signal Road" for a "Signal Run" search.
fn parsed_title_candidates(parsed: &scryer_application::ParsedReleaseMetadata) -> Vec<String> {
    let mut titles = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    if titles.is_empty() {
        titles.push(parsed.normalized_title.clone());
    }

    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for title in titles {
        let candidate = normalize_for_comparison(&title);
        if !candidate.is_empty() && seen.insert(candidate.clone()) {
            normalized.push(candidate);
        }
    }

    normalized
}

fn filter_strategy_results(
    results: &mut Vec<IndexerSearchResult>,
    context: &FilterStrategyContext<'_>,
) {
    if results.is_empty() {
        return;
    }

    for result in results.iter_mut() {
        result.provenance = Some(ReleaseCandidateProvenance {
            search_subject_kind: if context.is_rss_request {
                ReleaseSearchSubjectKind::Rss
            } else {
                ReleaseSearchSubjectKind::Freetext
            },
            strategy_kind: scryer_application::release_strategy_kind_for_label(
                context.strategy_label,
                context.is_rss_request,
            ),
            title_validated_upstream: context.title_guard_mode == TitleGuardMode::SkipTitleMatch,
        });
        if result.parsed_release_metadata.is_none() {
            result.parsed_release_metadata =
                Some(scryer_application::parse_release_metadata(&result.title));
        }
    }

    if context.query.is_empty() && context.season.is_none() && context.episode.is_none() {
        return;
    }

    let mut expected_titles = if context.query.is_empty() {
        Vec::new()
    } else {
        parsed_title_candidates(&scryer_application::parse_release_metadata(context.query))
    };
    expected_titles.extend(
        context
            .tagged_aliases
            .iter()
            .map(|alias| normalize_for_comparison(&alias.name))
            .filter(|alias| !alias.is_empty()),
    );
    let mut seen_titles = HashSet::new();
    expected_titles.retain(|title| seen_titles.insert(title.clone()));

    let before = results.len();
    results.retain(|result| {
        let Some(ref parsed) = result.parsed_release_metadata else {
            return true;
        };

        if context.title_guard_mode == TitleGuardMode::ExactTitleMatch
            && !expected_titles.is_empty()
        {
            let release_titles = parsed_title_candidates(parsed);
            let title_ok = release_titles.iter().any(|release_title| {
                expected_titles
                    .iter()
                    .any(|expected| expected == release_title)
            });
            if !title_ok {
                tracing::debug!(
                    strategy = context.strategy_label,
                    query = %context.query,
                    expected = ?expected_titles,
                    got = ?release_titles,
                    "title guard: title mismatch"
                );
                return false;
            }
        }

        if let Some(expected_s) = context.season
            && let Some(ref res_ep) = parsed.episode
            && let Some(rs) = res_ep.season
            && rs != expected_s
        {
            tracing::debug!(
                strategy = context.strategy_label,
                query = %context.query,
                expected_season = expected_s,
                got_season = rs,
                "title guard: season mismatch"
            );
            return false;
        }

        if let Some(expected_e) = context.episode
            && let Some(ref res_ep) = parsed.episode
            && !res_ep.episode_numbers.is_empty()
            && !res_ep.episode_numbers.contains(&expected_e)
        {
            tracing::debug!(
                strategy = context.strategy_label,
                query = %context.query,
                expected_episode = expected_e,
                got_episodes = ?res_ep.episode_numbers,
                "title guard: episode mismatch"
            );
            return false;
        }

        true
    });

    let filtered = before - results.len();
    if filtered > 0 {
        debug!(
            strategy = context.strategy_label,
            before,
            after = results.len(),
            filtered,
            "title guard: removed irrelevant results"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc as StdArc, Mutex as StdMutex};

    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_application::{IndexerQueryStats, IndexerSearchResponse};
    use scryer_domain::IndexerProviderCapabilities;

    use super::*;

    struct MockIndexerConfigRepository {
        configs: Vec<IndexerConfig>,
    }

    #[async_trait]
    impl IndexerConfigRepository for MockIndexerConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(
            &self,
            _update: scryer_application::IndexerConfigUpdate,
        ) -> AppResult<IndexerConfig> {
            Err(AppError::Validation("not implemented in test".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct RecordingTouchIndexerConfigRepository {
        configs: Vec<IndexerConfig>,
        touched_ids: StdArc<StdMutex<Vec<String>>>,
        cleared_ids: StdArc<StdMutex<Vec<String>>>,
    }

    #[async_trait]
    impl IndexerConfigRepository for RecordingTouchIndexerConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, id: &str) -> AppResult<()> {
            self.touched_ids
                .lock()
                .expect("touched ids mutex")
                .push(id.to_string());
            Ok(())
        }

        async fn clear_last_error(&self, id: &str) -> AppResult<()> {
            self.cleared_ids
                .lock()
                .expect("cleared ids mutex")
                .push(id.to_string());
            Ok(())
        }

        async fn update(
            &self,
            _update: scryer_application::IndexerConfigUpdate,
        ) -> AppResult<IndexerConfig> {
            Err(AppError::Validation("not implemented in test".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct MockIndexerStatsTracker;

    impl IndexerStatsTracker for MockIndexerStatsTracker {
        fn record_query(&self, _indexer_id: &str, _indexer_name: &str, _success: bool) {}

        fn record_api_limits(
            &self,
            _indexer_id: &str,
            _api_current: Option<u32>,
            _api_max: Option<u32>,
            _grab_current: Option<u32>,
            _grab_max: Option<u32>,
        ) {
        }

        fn all_stats(&self) -> Vec<IndexerQueryStats> {
            vec![]
        }
    }

    #[derive(Default)]
    struct RecordingIndexerStatsTracker {
        queries: StdArc<StdMutex<Vec<bool>>>,
    }

    impl IndexerStatsTracker for RecordingIndexerStatsTracker {
        fn record_query(&self, _indexer_id: &str, _indexer_name: &str, success: bool) {
            self.queries.lock().expect("stats log mutex").push(success);
        }

        fn record_api_limits(
            &self,
            _indexer_id: &str,
            _api_current: Option<u32>,
            _api_max: Option<u32>,
            _grab_current: Option<u32>,
            _grab_max: Option<u32>,
        ) {
        }

        fn all_stats(&self) -> Vec<IndexerQueryStats> {
            vec![]
        }
    }

    struct MockIndexerClient {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl IndexerClient for MockIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _cancel_token: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(IndexerSearchResponse {
                results: vec![],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    struct MockIndexerPluginProvider {
        rss: bool,
        calls: Arc<AtomicUsize>,
    }

    impl IndexerPluginProvider for MockIndexerPluginProvider {
        fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            Some(Arc::new(MockIndexerClient {
                calls: self.calls.clone(),
            }))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["mock".into()]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }

        fn capabilities_for_provider(&self, _provider_type: &str) -> IndexerProviderCapabilities {
            IndexerProviderCapabilities {
                rss: self.rss,
                supported_ids: HashMap::from([
                    ("movie".into(), vec!["imdb_id".into()]),
                    ("series".into(), vec!["tvdb_id".into()]),
                ]),
                deduplicates_aliases: false,
                season_param: Some("season".into()),
                episode_param: Some("ep".into()),
                query_param: Some("q".into()),
                search: true,
                imdb_search: true,
                tvdb_search: true,
                anidb_search: false,
                ..Default::default()
            }
        }
    }

    fn mock_indexer_config() -> IndexerConfig {
        IndexerConfig {
            id: "idx-1".into(),
            name: "Mock Indexer".into(),
            provider_type: "mock".into(),
            base_url: "https://example.test".into(),
            api_key_encrypted: None,
            rate_limit_seconds: Some(0),
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn managed_auto_mode_metadata(enable_rss: bool, enable_automatic_search: bool) -> String {
        serde_json::json!({
            "enable_rss": enable_rss,
            "enable_automatic_search": enable_automatic_search,
        })
        .to_string()
    }

    fn managed_metadata_with_caps(snapshot: Option<IndexerCapsSnapshot>) -> String {
        serde_json::json!({
            "enable_rss": true,
            "enable_automatic_search": true,
            "caps_snapshot": snapshot,
        })
        .to_string()
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RecordedCall {
        query: String,
        ids: HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        categories: Vec<String>,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    }

    type ResponseFn = dyn Fn(&RecordedCall) -> AppResult<IndexerSearchResponse> + Send + Sync;

    struct ScriptedIndexerClient {
        calls: StdArc<StdMutex<Vec<RecordedCall>>>,
        responder: StdArc<ResponseFn>,
    }

    #[async_trait]
    impl IndexerClient for ScriptedIndexerClient {
        async fn search(
            &self,
            query: String,
            ids: HashMap<String, String>,
            category: Option<String>,
            facet: Option<String>,
            _id_search_facet: Option<String>,
            newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            season: Option<u32>,
            episode: Option<u32>,
            absolute_episode: Option<u32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _cancel_token: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            let call = RecordedCall {
                query,
                ids,
                category,
                facet,
                categories: newznab_categories.unwrap_or_default(),
                season,
                episode,
                absolute_episode,
            };
            self.calls
                .lock()
                .expect("call log mutex")
                .push(call.clone());
            (self.responder)(&call)
        }
    }

    struct ScriptedIndexerPluginProvider {
        client: Arc<dyn IndexerClient>,
        caps: IndexerProviderCapabilities,
    }

    impl IndexerPluginProvider for ScriptedIndexerPluginProvider {
        fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            Some(self.client.clone())
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["mock".into()]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }

        fn capabilities_for_provider(&self, _provider_type: &str) -> IndexerProviderCapabilities {
            self.caps.clone()
        }
    }

    fn scripted_search_client(
        caps: IndexerProviderCapabilities,
        responder: impl Fn(&RecordedCall) -> AppResult<IndexerSearchResponse> + Send + Sync + 'static,
    ) -> (
        MultiIndexerSearchClient,
        StdArc<StdMutex<Vec<RecordedCall>>>,
    ) {
        scripted_search_client_with_stats(caps, Arc::new(MockIndexerStatsTracker), responder)
    }

    fn scripted_search_client_with_stats(
        caps: IndexerProviderCapabilities,
        stats_tracker: Arc<dyn IndexerStatsTracker>,
        responder: impl Fn(&RecordedCall) -> AppResult<IndexerSearchResponse> + Send + Sync + 'static,
    ) -> (
        MultiIndexerSearchClient,
        StdArc<StdMutex<Vec<RecordedCall>>>,
    ) {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(responder),
        });

        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            stats_tracker,
            Arc::new(ScriptedIndexerPluginProvider { client, caps }),
        );

        (multi, calls)
    }

    #[derive(Default)]
    struct SearchConcurrencyProbe {
        active: AtomicUsize,
        max_active: AtomicUsize,
        started: AtomicUsize,
        released: AtomicBool,
        release: tokio::sync::Notify,
    }

    impl SearchConcurrencyProbe {
        fn mark_started(&self) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.started.fetch_add(1, Ordering::SeqCst);

            let mut max_active = self.max_active.load(Ordering::SeqCst);
            while active > max_active {
                match self.max_active.compare_exchange(
                    max_active,
                    active,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(observed) => max_active = observed,
                }
            }
        }

        async fn wait_until_released(&self) {
            while !self.released.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
        }

        fn mark_finished(&self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
        }

        fn release_all(&self) {
            self.released.store(true, Ordering::SeqCst);
            self.release.notify_waiters();
        }
    }

    struct BlockingIndexerClient {
        probe: StdArc<SearchConcurrencyProbe>,
    }

    #[async_trait]
    impl IndexerClient for BlockingIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            cancel_token: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            self.probe.mark_started();
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    self.probe.mark_finished();
                    return Err(AppError::canceled("blocking indexer search canceled"));
                }
                _ = self.probe.wait_until_released() => {}
            }
            self.probe.mark_finished();
            Ok(IndexerSearchResponse {
                results: vec![],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    async fn wait_for_started(probe: &SearchConcurrencyProbe, expected: usize) {
        for _ in 0..100 {
            if probe.started.load(Ordering::SeqCst) >= expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!(
            "timed out waiting for {expected} searches to start; saw {}",
            probe.started.load(Ordering::SeqCst)
        );
    }

    fn indexed_mock_configs(count: usize) -> Vec<IndexerConfig> {
        (0..count)
            .map(|idx| {
                let mut config = mock_indexer_config();
                config.id = format!("idx-{idx}");
                config.name = format!("Mock Indexer {idx}");
                config
            })
            .collect()
    }

    async fn assert_leaf_search_limit_shared_across_clones(mode: SearchMode, limit: usize) {
        let config_count = limit + 4;
        let probe = StdArc::new(SearchConcurrencyProbe::default());
        let client = Arc::new(BlockingIndexerClient {
            probe: probe.clone(),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: indexed_mock_configs(config_count),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let first = multi.clone();
        let second = multi.clone();
        let first_search = tokio::spawn(async move {
            first
                .search(
                    "Search Limit".to_string(),
                    HashMap::new(),
                    None,
                    Some("movie".to_string()),
                    None,
                    None,
                    None,
                    mode,
                    None,
                    None,
                    None,
                    vec![],
                )
                .await
        });
        let second_search = tokio::spawn(async move {
            second
                .search(
                    "Search Limit".to_string(),
                    HashMap::new(),
                    None,
                    Some("movie".to_string()),
                    None,
                    None,
                    None,
                    mode,
                    None,
                    None,
                    None,
                    vec![],
                )
                .await
        });

        wait_for_started(&probe, limit).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(probe.max_active.load(Ordering::SeqCst), limit);
        assert_eq!(probe.started.load(Ordering::SeqCst), limit);

        probe.release_all();
        tokio::time::timeout(std::time::Duration::from_secs(2), first_search)
            .await
            .expect("first search should finish")
            .expect("first search task should join")
            .expect("first search should succeed");
        tokio::time::timeout(std::time::Duration::from_secs(2), second_search)
            .await
            .expect("second search should finish")
            .expect("second search task should join")
            .expect("second search should succeed");

        assert_eq!(probe.started.load(Ordering::SeqCst), config_count * 2);
        assert!(probe.max_active.load(Ordering::SeqCst) <= limit);
    }

    #[tokio::test]
    async fn interactive_search_cancellation_returns_promptly() {
        let probe = StdArc::new(SearchConcurrencyProbe::default());
        let client = Arc::new(BlockingIndexerClient {
            probe: probe.clone(),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: indexed_mock_configs(3),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );
        let cancel_token = CancellationToken::new();
        let search_cancel_token = cancel_token.clone();
        let search = tokio::spawn(async move {
            <MultiIndexerSearchClient as IndexerClient>::search(
                &multi,
                "Cancel Me".to_string(),
                HashMap::new(),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
                search_cancel_token,
            )
            .await
        });

        wait_for_started(&probe, 1).await;
        cancel_token.cancel();
        let error = tokio::time::timeout(std::time::Duration::from_secs(2), search)
            .await
            .expect("search should return promptly")
            .expect("search task should join")
            .expect_err("search should be canceled");
        assert!(error.is_canceled(), "unexpected error: {error}");
    }

    async fn backoff_state(
        client: &MultiIndexerSearchClient,
        indexer_id: &str,
    ) -> Option<IndexerBackoffState> {
        client
            .backoff_tracker
            .state
            .lock()
            .await
            .get(indexer_id)
            .cloned()
    }

    fn search_result(title: &str) -> IndexerSearchResult {
        IndexerSearchResult {
            source: "mock".into(),
            title: title.into(),
            link: None,
            download_url: Some(format!(
                "https://example.test/download/{}",
                title.replace(' ', "_")
            )),
            source_kind: None,
            size_bytes: None,
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: None,
            quality_profile_decision: None,
            extra: HashMap::new(),
            guid: None,
            info_url: None,
            provenance: None,
            candidate_token: None,
            queue_scope: None,
            auto_eligible: None,
            auto_decision_code: None,
            auto_decision_summary: None,
        }
    }

    fn response_with_titles(titles: &[&str]) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            results: titles.iter().map(|title| search_result(title)).collect(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }

    fn movie_caps() -> IndexerProviderCapabilities {
        IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("movie".into(), vec!["imdb_id".into()])]),
            deduplicates_aliases: false,
            season_param: None,
            episode_param: None,
            query_param: Some("q".into()),
            search: true,
            imdb_search: true,
            tvdb_search: false,
            anidb_search: false,
            ..Default::default()
        }
    }

    fn series_caps() -> IndexerProviderCapabilities {
        IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("series".into(), vec!["tvdb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: false,
            tvdb_search: true,
            anidb_search: false,
            ..Default::default()
        }
    }

    fn anime_caps() -> IndexerProviderCapabilities {
        IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("anime".into(), vec!["anidb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: false,
            tvdb_search: false,
            anidb_search: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn auto_search_leaf_concurrency_is_shared_across_cloned_clients() {
        assert_leaf_search_limit_shared_across_clones(
            SearchMode::Auto,
            BACKGROUND_INDEXER_SEARCH_CONCURRENCY_LIMIT,
        )
        .await;
    }

    #[tokio::test]
    async fn interactive_search_leaf_concurrency_is_shared_across_cloned_clients() {
        assert_leaf_search_limit_shared_across_clones(
            SearchMode::Interactive,
            INTERACTIVE_INDEXER_SEARCH_CONCURRENCY_LIMIT,
        )
        .await;
    }

    fn prowlarr_caps_snapshot(movie_params: &[&str], tv_params: &[&str]) -> IndexerCapsSnapshot {
        prowlarr_caps_snapshot_with_availability(true, movie_params, true, tv_params)
    }

    fn prowlarr_caps_snapshot_with_availability(
        movie_available: bool,
        movie_params: &[&str],
        tv_available: bool,
        tv_params: &[&str],
    ) -> IndexerCapsSnapshot {
        IndexerCapsSnapshot {
            search: Some(IndexerCapsSearchNode {
                available: true,
                supported_params: vec!["q".to_string()],
                search_engine: None,
            }),
            movie_search: Some(IndexerCapsSearchNode {
                available: movie_available,
                supported_params: movie_params.iter().map(|value| value.to_string()).collect(),
                search_engine: None,
            }),
            tv_search: Some(IndexerCapsSearchNode {
                available: tv_available,
                supported_params: tv_params.iter().map(|value| value.to_string()).collect(),
                search_engine: None,
            }),
            ..IndexerCapsSnapshot::default()
        }
    }

    #[tokio::test]
    async fn indexer_failure_records_last_error_for_config_id() {
        let touched_ids = StdArc::new(StdMutex::new(Vec::new()));
        let cleared_ids = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: StdArc::new(StdMutex::new(Vec::new())),
            responder: StdArc::new(|_| Err(AppError::Repository("upstream status 503".into()))),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(RecordingTouchIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
                touched_ids: touched_ids.clone(),
                cleared_ids: cleared_ids.clone(),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: true,
                    search: false,
                    imdb_search: false,
                    tvdb_search: false,
                    anidb_search: false,
                    supported_ids: HashMap::new(),
                    ..Default::default()
                },
            }),
        );

        let response = multi
            .search(
                String::new(),
                HashMap::new(),
                None,
                Some("series".to_string()),
                None,
                None,
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("RSS failure is isolated to the indexer");

        assert!(response.results.is_empty());
        assert_eq!(
            *touched_ids.lock().expect("touched ids mutex"),
            vec!["idx-1".to_string()]
        );
        assert!(cleared_ids.lock().expect("cleared ids mutex").is_empty());
    }

    #[tokio::test]
    async fn indexer_success_clears_last_error_for_config_id() {
        let touched_ids = StdArc::new(StdMutex::new(Vec::new()));
        let cleared_ids = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: StdArc::new(StdMutex::new(Vec::new())),
            responder: StdArc::new(|_| {
                Ok(IndexerSearchResponse {
                    results: vec![search_result("Recovered.Show.S01E01")],
                    api_current: None,
                    api_max: None,
                    grab_current: None,
                    grab_max: None,
                })
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(RecordingTouchIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
                touched_ids: touched_ids.clone(),
                cleared_ids: cleared_ids.clone(),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: true,
                    search: false,
                    imdb_search: false,
                    tvdb_search: false,
                    anidb_search: false,
                    supported_ids: HashMap::new(),
                    ..Default::default()
                },
            }),
        );

        let response = multi
            .search(
                String::new(),
                HashMap::new(),
                None,
                Some("series".to_string()),
                None,
                None,
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("RSS success should succeed");

        assert_eq!(response.results.len(), 1);
        assert!(touched_ids.lock().expect("touched ids mutex").is_empty());
        assert_eq!(
            *cleared_ids.lock().expect("cleared ids mutex"),
            vec!["idx-1".to_string()]
        );
    }

    #[tokio::test]
    async fn indexer_failure_then_fallback_success_records_and_clears_last_error() {
        let touched_ids = StdArc::new(StdMutex::new(Vec::new()));
        let cleared_ids = StdArc::new(StdMutex::new(Vec::new()));
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let attempts = StdArc::new(AtomicUsize::new(0));
        let attempts_for_responder = attempts.clone();
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(move |call| {
                let attempt = attempts_for_responder.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    assert!(call.ids.contains_key("tvdb_id"));
                    return Err(AppError::Validation("id tier failed".into()));
                }

                assert!(call.ids.is_empty());
                response_with_titles(&["Signal.Run.S01E12.720p.WEB-DL"])
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(RecordingTouchIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
                touched_ids: touched_ids.clone(),
                cleared_ids: cleared_ids.clone(),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: series_caps(),
            }),
        );

        let response = multi
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("fallback success should succeed");

        let recorded_calls = calls.lock().expect("calls").clone();
        assert_eq!(recorded_calls.len(), 2);
        assert_eq!(response.results.len(), 1);
        assert_eq!(
            *touched_ids.lock().expect("touched ids mutex"),
            vec!["idx-1".to_string()]
        );
        assert_eq!(
            *cleared_ids.lock().expect("cleared ids mutex"),
            vec!["idx-1".to_string()]
        );
    }

    #[tokio::test]
    async fn rss_sync_search_skips_providers_without_rss_capability() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: false,
                calls: calls.clone(),
            }),
        );

        let response = client
            .search(
                String::new(),
                HashMap::new(),
                None,
                None,
                None,
                None,
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("rss sync search should succeed");

        assert!(response.results.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn rss_sync_search_skips_managed_indexers_when_metadata_disables_rss() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut config = mock_indexer_config();
        config.managed_metadata_json = Some(managed_auto_mode_metadata(false, true));
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: true,
                calls: calls.clone(),
            }),
        );

        let response = client
            .search(
                String::new(),
                HashMap::new(),
                None,
                None,
                None,
                None,
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("rss sync search should succeed");

        assert!(response.results.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn automatic_search_skips_managed_indexers_when_metadata_disables_automatic_search() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut config = mock_indexer_config();
        config.managed_metadata_json = Some(managed_auto_mode_metadata(true, false));
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: true,
                calls: calls.clone(),
            }),
        );

        let response = client
            .search(
                "Example Show".to_string(),
                HashMap::new(),
                None,
                Some("series".to_string()),
                None,
                None,
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("automatic search should succeed");

        assert!(response.results.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn managed_prowlarr_movie_caps_send_only_advertised_ids() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid", "genre"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["12.Lanterns.of.Winter.2013"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "12 Lanterns of Winter".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12004567".to_string()),
                    ("tmdb_id".to_string(), "120045".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([("imdb_id".to_string(), "tt12004567".to_string())])
        );
    }

    #[tokio::test]
    async fn direct_newznab_without_caps_snapshot_uses_legacy_static_ids_to_carry_full_id_envelope()
    {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["12.Lanterns.of.Winter.2013"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "12 Lanterns of Winter".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12004567".to_string()),
                    ("tmdb_id".to_string(), "120045".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([
                ("imdb_id".to_string(), "tt12004567".to_string()),
                ("tmdb_id".to_string(), "120045".to_string()),
            ])
        );
    }

    #[tokio::test]
    async fn direct_newznab_caps_snapshot_can_widen_ids_when_live_caps_allow_it() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.caps_snapshot_json = Some(
            serde_json::to_string(&prowlarr_caps_snapshot(
                &["q", "tmdbid", "imdbid"],
                &["q", "season", "ep", "tvdbid"],
            ))
            .expect("serialize direct caps snapshot"),
        );

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Solar.Divide.Part.Two.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "Solar Divide Part Two".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt22006789".to_string()),
                    ("tmdb_id".to_string(), "220067".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([
                ("imdb_id".to_string(), "tt22006789".to_string()),
                ("tmdb_id".to_string(), "220067".to_string()),
            ])
        );
    }

    #[tokio::test]
    async fn managed_prowlarr_caps_snapshot_can_aggregate_supported_ids() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "tmdbid", "imdbid"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Solar.Divide.Part.Two.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "Solar Divide Part Two".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt22006789".to_string()),
                    ("tmdb_id".to_string(), "220067".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([
                ("imdb_id".to_string(), "tt22006789".to_string()),
                ("tmdb_id".to_string(), "220067".to_string()),
            ])
        );
    }

    #[tokio::test]
    async fn managed_prowlarr_series_caps_drop_unadvertised_ids() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Storm.Signal.S01E02.2026"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: series_caps(),
            }),
        );

        let response = multi
            .search(
                "Storm Signal".to_string(),
                HashMap::from([
                    ("tvdb_id".to_string(), "424242".to_string()),
                    ("imdb_id".to_string(), "tt42424242".to_string()),
                    ("tmdb_id".to_string(), "424242".to_string()),
                ]),
                Some("series".to_string()),
                Some("series".to_string()),
                None,
                Some(vec!["5000".to_string()]),
                None,
                SearchMode::Interactive,
                Some(1),
                Some(2),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([("tvdb_id".to_string(), "424242".to_string())])
        );
        assert_eq!(recorded[0].season, Some(1));
        assert_eq!(recorded[0].episode, Some(2));
    }

    #[tokio::test]
    async fn managed_prowlarr_without_caps_snapshot_falls_back_to_query_only() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(None));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["12.Lanterns.of.Winter.2013"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "12 Lanterns of Winter".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12004567".to_string()),
                    ("tmdb_id".to_string(), "120045".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].query, "12 Lanterns of Winter");
        assert_eq!(recorded[0].facet, None);
        assert!(recorded[0].categories.is_empty());
        assert_eq!(recorded[0].season, None);
        assert_eq!(recorded[0].episode, None);
        assert_eq!(recorded[0].absolute_episode, None);
    }

    #[tokio::test]
    async fn managed_prowlarr_prefers_supplied_newznab_categories_over_facet_defaults() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["q", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Demon.Slayer.Mugen.Train.2020"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Demon Slayer Mugen Train 2020".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt11032374".to_string())]),
                Some("anime".to_string()),
                Some("movie".to_string()),
                Some("movie".to_string()),
                Some(vec!["5070".to_string(), "2000".to_string()]),
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].categories,
            vec!["5070".to_string(), "2000".to_string()]
        );
    }

    #[tokio::test]
    async fn managed_prowlarr_caps_with_unavailable_movie_search_fall_back_to_generic_query() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot_with_availability(
                false,
                &["q", "imdbid", "tmdbid"],
                true,
                &["q", "season", "ep", "tvdbid"],
            ),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["12.Lanterns.of.Winter.2013"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "12 Lanterns of Winter".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12004567".to_string()),
                    ("tmdb_id".to_string(), "120045".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
                Some(vec!["2000".to_string()]),
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].query, "12 Lanterns of Winter");
        assert_eq!(recorded[0].category, None);
        assert_eq!(recorded[0].facet, None);
        assert!(recorded[0].categories.is_empty());
    }

    #[tokio::test]
    async fn id_free_text_capable_movie_provider_receives_freetext() {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Jujutsu.Kaisen.0.2021.1080p"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: false,
                    supported_ids: HashMap::new(),
                    query_param: Some("q".into()),
                    supported_query_facets: vec!["movie".into()],
                    search: true,
                    ..Default::default()
                },
            }),
        );

        let response = multi
            .search(
                "JUJUTSU KAISEN 0".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt14331144".to_string())]),
                Some("movie".to_string()),
                Some("movie".to_string()),
                None,
                Some(vec!["2000".to_string()]),
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("movie freetext search should dispatch");

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].query, "JUJUTSU KAISEN 0");
        assert_eq!(recorded[0].category.as_deref(), Some("movie"));
        assert_eq!(recorded[0].facet.as_deref(), Some("movie"));
        assert_eq!(recorded[0].categories, vec!["2000".to_string()]);
    }

    #[tokio::test]
    async fn legacy_anime_id_provider_does_not_receive_movie_freetext() {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Unexpected.Movie.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: anime_caps(),
            }),
        );

        let response = multi
            .search(
                "JUJUTSU KAISEN 0".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt14331144".to_string())]),
                Some("movie".to_string()),
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("unsupported facet should skip provider");

        assert!(response.results.is_empty());
        assert!(calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn generic_nab_query_only_fallback_strips_structured_context() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot_with_availability(false, &["q"], false, &["q"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Naruto.Shippuuden.09.1080p"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: anime_caps(),
            }),
        );

        let _response = multi
            .search(
                "Naruto Shippuuden 09".to_string(),
                HashMap::from([("anidb_id".to_string(), "1234".to_string())]),
                Some("anime".to_string()),
                Some("anime".to_string()),
                None,
                Some(vec!["5070".to_string()]),
                None,
                SearchMode::Interactive,
                Some(1),
                Some(9),
                Some(9),
                vec![],
            )
            .await
            .expect("generic fallback should search");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].category, None);
        assert_eq!(recorded[0].facet, None);
        assert!(recorded[0].categories.is_empty());
        assert_eq!(recorded[0].season, None);
        assert_eq!(recorded[0].episode, None);
        assert_eq!(recorded[0].absolute_episode, None);
    }

    #[tokio::test]
    async fn live_caps_basic_query_fallback_strips_facet_params_when_tvsearch_lacks_q() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|call| {
                if call.ids.is_empty() {
                    response_with_titles(&["Storm.Signal.S01E02.2026"])
                } else {
                    Ok(IndexerSearchResponse {
                        results: vec![],
                        api_current: None,
                        api_max: None,
                        grab_current: None,
                        grab_max: None,
                    })
                }
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Storm Signal".to_string(),
                HashMap::from([("tvdb_id".to_string(), "424242".to_string())]),
                Some("series".to_string()),
                Some("series".to_string()),
                None,
                Some(vec!["5000".to_string()]),
                None,
                SearchMode::Interactive,
                Some(1),
                Some(2),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 2);

        assert_eq!(recorded[0].ids.get("tvdb_id"), Some(&"424242".to_string()));
        assert_eq!(recorded[0].facet, Some("series".to_string()));
        assert_eq!(recorded[0].categories, vec!["5000".to_string()]);
        assert_eq!(recorded[0].season, Some(1));
        assert_eq!(recorded[0].episode, Some(2));

        assert!(recorded[1].ids.is_empty());
        assert_eq!(recorded[1].query, "Storm Signal");
        assert_eq!(recorded[1].facet, None);
        assert!(recorded[1].categories.is_empty());
        assert_eq!(recorded[1].season, None);
        assert_eq!(recorded[1].episode, None);
    }

    #[tokio::test]
    async fn facet_scoped_text_dispatch_preserves_advertised_anime_context() {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Naruto.Shippuuden.09.1080p"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: false,
                    supported_ids: HashMap::new(),
                    query_param: Some("q".into()),
                    supported_query_facets: vec!["anime".into()],
                    search_inputs: vec![
                        scryer_domain::IndexerSearchInputCapability::TitleQuery,
                        scryer_domain::IndexerSearchInputCapability::Category,
                        scryer_domain::IndexerSearchInputCapability::Season,
                        scryer_domain::IndexerSearchInputCapability::Episode,
                        scryer_domain::IndexerSearchInputCapability::AbsoluteEpisode,
                    ],
                    search: true,
                    ..Default::default()
                },
            }),
        );

        let _response = multi
            .search(
                "Naruto Shippuuden 09".to_string(),
                HashMap::from([("anidb_id".to_string(), "1234".to_string())]),
                Some("anime".to_string()),
                Some("anime".to_string()),
                None,
                Some(vec!["5070".to_string()]),
                None,
                SearchMode::Interactive,
                Some(1),
                Some(9),
                Some(9),
                vec![],
            )
            .await
            .expect("facet-scoped text search should dispatch");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].category.as_deref(), Some("anime"));
        assert_eq!(recorded[0].facet.as_deref(), Some("anime"));
        assert_eq!(recorded[0].categories, vec!["5070".to_string()]);
        assert_eq!(recorded[0].season, Some(1));
        assert_eq!(recorded[0].episode, Some(9));
        assert_eq!(recorded[0].absolute_episode, Some(9));
    }

    #[tokio::test]
    async fn managed_prowlarr_children_fall_back_to_default_categories_when_routing_is_empty() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Category.Fallback.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Category Fallback".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt12345678".to_string())]),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].categories, vec!["2000".to_string()]);
    }

    #[tokio::test]
    async fn direct_newznab_searches_stay_uncategorized_when_routing_is_empty() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Category.Fallback.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Category Fallback".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt12345678".to_string())]),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].categories.is_empty());
    }

    #[tokio::test]
    async fn non_nab_managed_configs_do_not_inherit_prowlarr_proxy_behavior() {
        let mut config = mock_indexer_config();
        config.provider_type = "mock".into();
        config.managed_parent_config_id = Some("parent".into());

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Proxy.Safe.Result.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Proxy Safe Result".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12345678".to_string()),
                    ("tmdb_id".to_string(), "123456".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([
                ("imdb_id".to_string(), "tt12345678".to_string()),
                ("tmdb_id".to_string(), "123456".to_string()),
            ])
        );
        assert_eq!(recorded[0].facet.as_deref(), Some("movie"));
        assert!(recorded[0].categories.is_empty());
    }

    #[tokio::test]
    async fn id_backed_movie_results_skip_freetext_title_guard() {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|call| {
                if call.ids.is_empty() {
                    response_with_titles(&["Should.Not.Fallback.2024"])
                } else {
                    response_with_titles(&["Completely.Different.Title.2024.1080p.BluRay"])
                }
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "Expected Movie 2024".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12345678".to_string()),
                    ("tmdb_id".to_string(), "123456".to_string()),
                    ("tvdb_id".to_string(), "98765".to_string()),
                    ("anidb_id".to_string(), "54321".to_string()),
                    ("mal_id".to_string(), "67890".to_string()),
                ]),
                Some("movie".to_string()),
                Some("movie".to_string()),
                Some("movie".to_string()),
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].title,
            "Completely.Different.Title.2024.1080p.BluRay"
        );
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1, "ID results should suppress fallback");
        assert_eq!(
            recorded[0].ids,
            HashMap::from([
                ("imdb_id".to_string(), "tt12345678".to_string()),
                ("tmdb_id".to_string(), "123456".to_string()),
                ("tvdb_id".to_string(), "98765".to_string()),
                ("anidb_id".to_string(), "54321".to_string()),
                ("mal_id".to_string(), "67890".to_string()),
            ])
        );
    }

    #[tokio::test]
    async fn rss_sync_search_with_newznab_categories_still_uses_rss_mode() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: true,
                calls: calls.clone(),
            }),
        );

        let response = client
            .search(
                String::new(),
                HashMap::new(),
                None,
                None,
                None,
                Some(vec!["2000".into(), "5030".into()]),
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("rss sync search with categories should succeed");

        assert!(response.results.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rss_sync_search_runs_each_newznab_category_in_a_separate_request() {
        let mut caps = movie_caps();
        caps.rss = true;
        let (client, calls) =
            scripted_search_client(caps, |call| match call.categories.as_slice() {
                [category] if category == "2000" => {
                    response_with_titles(&["Movies.Release.2000.1080p.WEB-DL"])
                }
                [category] if category == "5030" => {
                    response_with_titles(&["Series.Release.5030.720p.WEB-DL"])
                }
                other => Err(AppError::Validation(format!(
                    "unexpected rss categories: {:?}",
                    other
                ))),
            });

        let response = client
            .search(
                String::new(),
                HashMap::new(),
                None,
                None,
                None,
                Some(vec!["2000".into(), "5030".into()]),
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("rss sync search should fan out per category");

        let calls = calls.lock().expect("call log mutex");
        let mut categories: Vec<Vec<String>> =
            calls.iter().map(|call| call.categories.clone()).collect();
        categories.sort();

        assert_eq!(
            categories,
            vec![vec!["2000".to_string()], vec!["5030".to_string()]]
        );
        assert_eq!(response.results.len(), 2);
    }

    #[tokio::test]
    async fn series_search_with_tvdb_id_skips_freetext_when_id_tier_returns_results() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&["Signal.Run.S01E12.720p.WEB-DL"])
            } else {
                response_with_titles(&["Signal.Road.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[0].query.is_empty());
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn series_search_with_tvdb_id_falls_back_only_after_empty_id_tier() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Signal.Run.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].ids.is_empty());
        assert_eq!(calls[1].query, "Signal Run S01E12");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn id_empty_then_fallback_still_rejects_false_positive_titles() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&[
                    "Signal.Run.S01E12.720p.WEB-DL",
                    "Signal.Road.2021.S01E12.2160p.WEB-DL",
                ])
            }
        });

        let response = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[1].ids.is_empty());
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn movie_search_with_imdb_id_uses_tiered_fallback() {
        let (client, calls) = scripted_search_client(movie_caps(), |call| {
            if call.ids.contains_key("imdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Lattice.Zero.1999.1080p.BluRay"])
            }
        });

        let response = client
            .search(
                "Lattice Zero".into(),
                HashMap::from([("imdb_id".to_string(), "tt0133093".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].ids.contains_key("imdb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].ids.is_empty());
        assert_eq!(calls[1].query, "Lattice Zero");
        assert_eq!(response.results[0].title, "Lattice.Zero.1999.1080p.BluRay");
    }

    #[test]
    fn series_movie_anime_lane_builds_movie_id_strategy() {
        let caps = movie_caps();
        let ids = HashMap::from([("imdb_id".to_string(), "tt11032374".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Mugen Train 2020",
            query_facet: "anime",
            id_facet: "movie",
            ids: &ids,
            season: None,
            episode: None,
            absolute_episode: None,
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::LegacyAggregate,
            text_dispatch_mode: TextDispatchMode::None,
            is_alias_query: false,
        });

        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].label, "ids");
        assert_eq!(strategies[0].request_facet, "movie");
        assert!(strategies[0].ids.contains_key("imdb_id"));
    }

    #[tokio::test]
    async fn interactive_search_errors_when_every_strategy_fails() {
        let (client, _calls) = scripted_search_client(movie_caps(), |_call| {
            Err(AppError::Repository("forced indexer failure".into()))
        });

        let error = client
            .search(
                "Mugen Train 2020".into(),
                HashMap::from([("imdb_id".to_string(), "tt11032374".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect_err("interactive search should report all-failed attempts");

        assert!(
            error
                .to_string()
                .contains("all attempted indexer strategies failed"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn movie_query_backed_id_search_keeps_synthetic_numeric_title_match() {
        let (client, calls) = scripted_search_client(movie_caps(), |_call| {
            response_with_titles(&["12.Lanterns.of.Winter.2013.1080p.BluRay.x264-GROUP"])
        });

        let response = client
            .search(
                "12 Lanterns of Winter".into(),
                HashMap::from([("imdb_id".to_string(), "tt12004567".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].ids.contains_key("imdb_id"));
        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].title,
            "12.Lanterns.of.Winter.2013.1080p.BluRay.x264-GROUP"
        );
    }

    #[tokio::test]
    async fn anime_search_keeps_id_variants_in_primary_tier_and_falls_back_after_empty_results() {
        let (client, calls) = scripted_search_client(anime_caps(), |call| {
            if call.ids.contains_key("anidb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Blade.Summit.S02E03.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 3);
        assert!(calls[0].ids.contains_key("anidb_id"));
        assert!(calls[1].ids.contains_key("anidb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].query.is_empty());
        assert!(calls[0].absolute_episode == Some(21) || calls[1].absolute_episode == Some(21));
        assert!(calls[0].ids.is_empty() || calls[1].ids.is_empty() || calls[2].ids.is_empty());
        assert!(calls[2].ids.is_empty());
        assert_eq!(calls[2].query, "Blade Summit S02E03");
        assert_eq!(response.results[0].title, "Blade.Summit.S02E03.720p.WEB-DL");
    }

    #[tokio::test]
    async fn id_tier_errors_trigger_title_fallback() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                Err(AppError::Repository("boom".into()))
            } else {
                response_with_titles(&["Signal.Run.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("ID-tier errors should fall back to freetext search");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].ids.is_empty());
        assert_eq!(calls[1].query, "Signal Run S01E12");
        assert_eq!(response.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn mixed_primary_outcomes_trigger_fallback_when_no_primary_results_are_usable() {
        let (client, calls) = scripted_search_client(anime_caps(), |call| {
            if call.ids.contains_key("anidb_id") && call.absolute_episode.is_some() {
                Err(AppError::Repository("abs lookup failed".into()))
            } else if call.ids.contains_key("anidb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Demon.Slayer.S02E03.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("mixed primary outcomes should still aggregate cleanly");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 3);
        assert!(calls[0].ids.contains_key("anidb_id"));
        assert!(calls[1].ids.contains_key("anidb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].query.is_empty());
        assert!(calls[2].ids.is_empty());
        assert_eq!(calls[2].query, "Blade Summit S02E03");
        assert!(response.results.is_empty());
    }

    #[tokio::test]
    async fn mixed_batch_does_not_back_off_when_any_request_succeeds() {
        let stats = Arc::new(RecordingIndexerStatsTracker::default());
        let (client, calls) =
            scripted_search_client_with_stats(anime_caps(), stats.clone(), |call| {
                if call.ids.contains_key("anidb_id") && call.absolute_episode.is_some() {
                    Err(AppError::Repository("abs lookup failed".into()))
                } else if call.ids.contains_key("anidb_id") {
                    response_with_titles(&[])
                } else {
                    response_with_titles(&["Blade.Summit.S02E03.720p.WEB-DL"])
                }
            });

        client.backoff_tracker.state.lock().await.insert(
            "idx-1".into(),
            IndexerBackoffState {
                escalation_level: 1,
                disabled_until: None,
            },
        );

        let response = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("mixed primary outcomes should still aggregate cleanly");

        {
            let calls = calls.lock().expect("call log mutex");
            assert_eq!(calls.len(), 3);
            assert!(calls[2].ids.is_empty());
            assert_eq!(calls[2].query, "Blade Summit S02E03");
            assert_eq!(response.results[0].title, "Blade.Summit.S02E03.720p.WEB-DL");
        }
        assert!(client.backoff_tracker.is_disabled("idx-1").await.is_none());
        let state = backoff_state(&client, "idx-1")
            .await
            .expect("success should preserve a cleared backoff entry");
        assert_eq!(state.escalation_level, 0);
        assert!(state.disabled_until.is_none());

        let stats = stats.queries.lock().expect("stats log mutex");
        assert_eq!(stats.len(), 3);
        assert_eq!(stats.iter().filter(|success| **success).count(), 2);
        assert_eq!(stats.iter().filter(|success| !**success).count(), 1);
    }

    #[tokio::test]
    async fn all_primary_request_failures_fall_back_before_backoff() {
        let stats = Arc::new(RecordingIndexerStatsTracker::default());
        let (client, calls) =
            scripted_search_client_with_stats(anime_caps(), stats.clone(), |call| {
                if call.ids.contains_key("anidb_id") {
                    Err(AppError::Repository("lookup failed".into()))
                } else {
                    response_with_titles(&["Blade.Summit.S02E03.720p.WEB-DL"])
                }
            });

        let response = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("all-failure primary outcomes should fall back to freetext");

        {
            let calls = calls.lock().expect("call log mutex");
            assert_eq!(calls.len(), 3);
            assert!(calls[0].ids.contains_key("anidb_id"));
            assert!(calls[1].ids.contains_key("anidb_id"));
            assert!(calls[2].ids.is_empty());
            assert_eq!(calls[2].query, "Blade Summit S02E03");
        }
        assert_eq!(response.results[0].title, "Blade.Summit.S02E03.720p.WEB-DL");
        assert!(client.backoff_tracker.is_disabled("idx-1").await.is_none());

        assert!(
            backoff_state(&client, "idx-1").await.is_none(),
            "fallback success should not create a new backoff entry"
        );

        let stats = stats.queries.lock().expect("stats log mutex");
        assert_eq!(stats.len(), 3);
        assert_eq!(stats.iter().filter(|success| **success).count(), 1);
        assert_eq!(stats.iter().filter(|success| !**success).count(), 2);
    }

    #[tokio::test]
    async fn record_failure_does_not_extend_active_backoff() {
        let tracker = IndexerBackoffTracker::new();
        let disabled_until = chrono::Utc::now() + chrono::Duration::minutes(45);

        tracker.state.lock().await.insert(
            "idx-1".to_string(),
            IndexerBackoffState {
                escalation_level: 3,
                disabled_until: Some(disabled_until),
            },
        );

        let returned = tracker.record_failure("idx-1", None).await;
        assert_eq!(returned.disabled_until, disabled_until);
        assert_eq!(returned.escalation_level, 3);

        let state = tracker
            .state
            .lock()
            .await
            .get("idx-1")
            .cloned()
            .expect("backoff state should remain present");
        assert_eq!(state.escalation_level, 3);
        assert_eq!(state.disabled_until, Some(disabled_until));
    }

    #[test]
    fn retry_after_parser_extracts_seconds_from_plugin_error_text() {
        let retry_after =
            parse_retry_after_seconds("HTTP 429: rate limited; retry_after_seconds=900")
                .expect("retry after should parse");
        assert_eq!(retry_after, std::time::Duration::from_secs(900));
        assert!(parse_retry_after_seconds("HTTP 429: rate limited").is_none());
    }

    #[tokio::test]
    async fn record_failure_uses_retry_after_override() {
        let tracker = IndexerBackoffTracker::new();
        let before = chrono::Utc::now();
        let backoff = tracker
            .record_failure("idx-1", Some(std::time::Duration::from_secs(900)))
            .await;
        let after = chrono::Utc::now();

        assert_eq!(backoff.escalation_level, 1);
        assert!(backoff.disabled_until >= before + chrono::Duration::seconds(900));
        assert!(backoff.disabled_until <= after + chrono::Duration::seconds(901));
    }

    #[tokio::test]
    async fn persisted_backoff_seeds_next_escalation_after_restart() {
        let tracker = IndexerBackoffTracker::new();
        tracker
            .seed_persisted(
                "idx-1",
                &IndexerSystemBackoff {
                    disabled_until: chrono::Utc::now() - chrono::Duration::minutes(1),
                    escalation_level: 3,
                },
            )
            .await;

        let before = chrono::Utc::now();
        let backoff = tracker.record_failure("idx-1", None).await;
        let after = chrono::Utc::now();

        assert_eq!(backoff.escalation_level, 4);
        assert!(backoff.disabled_until >= before + chrono::Duration::minutes(30));
        assert!(backoff.disabled_until <= after + chrono::Duration::minutes(31));
    }

    #[tokio::test]
    async fn exact_title_guard_rejects_false_positive_series_matches_for_freetext_searches() {
        let (client, _calls) = scripted_search_client(series_caps(), |_call| {
            response_with_titles(&[
                "Signal.Run.S01E12.720p.WEB-DL",
                "Signal.Road.2021.S01E12.2160p.WEB-DL",
                "Friends.Like.These.S01E12.720p.WEB-DL",
                "Smiling.Friends.S01E12.1080p.WEB-DL",
            ])
        });

        let firefly = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::new(),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("firefly search should succeed");
        assert_eq!(firefly.results.len(), 1);
        assert_eq!(firefly.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");

        let friends = client
            .search(
                "Friends S01E12".into(),
                HashMap::new(),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("friends search should succeed");
        assert!(friends.results.is_empty());
    }

    #[tokio::test]
    async fn ids_only_searches_skip_title_guard() {
        let (client, _calls) = scripted_search_client(movie_caps(), |call| {
            if call.ids.contains_key("imdb_id") {
                response_with_titles(&["Lantern.Tide.Hidden.Current.2001.1080p.BluRay"])
            } else {
                response_with_titles(&[])
            }
        });

        let response = client
            .search(
                String::new(),
                HashMap::from([("imdb_id".to_string(), "tt0245429".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("ID-backed search should succeed");

        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].title,
            "Lantern.Tide.Hidden.Current.2001.1080p.BluRay"
        );
    }

    #[tokio::test]
    async fn query_backed_id_searches_skip_title_guard() {
        let (client, _calls) = scripted_search_client(movie_caps(), |call| {
            if call.ids.contains_key("imdb_id") {
                response_with_titles(&[
                    "Lantern.Tide.Hidden.Current.2001.1080p.BluRay",
                    "Lantern.Tide.2001.1080p.BluRay",
                ])
            } else {
                response_with_titles(&[])
            }
        });

        let response = client
            .search(
                "Lantern Tide".into(),
                HashMap::from([("imdb_id".to_string(), "tt0245429".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("query-backed ID search should succeed");

        assert_eq!(response.results.len(), 2);
        assert_eq!(
            response.results[0].title,
            "Lantern.Tide.Hidden.Current.2001.1080p.BluRay"
        );
        assert_eq!(response.results[1].title, "Lantern.Tide.2001.1080p.BluRay");
    }

    #[test]
    fn anime_strategies_try_abs_and_sxex_in_parallel() {
        let caps = IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("anime".into(), vec!["anidb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("s".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: false,
            tvdb_search: false,
            anidb_search: true,
            ..Default::default()
        };

        let ids = HashMap::from([("anidb_id".to_string(), "18886".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Silver Horizon: Beyond Journey's End S02E05",
            query_facet: "anime",
            id_facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(33),
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::LegacyAggregate,
            text_dispatch_mode: TextDispatchMode::FacetScoped,
            is_alias_query: false,
        });

        assert_eq!(strategies.len(), 3);

        assert_eq!(strategies[0].label, "ids_abs");
        assert_eq!(strategies[0].season, None);
        assert_eq!(strategies[0].episode, None);
        assert_eq!(strategies[0].absolute_episode, Some(33));

        assert_eq!(strategies[1].label, "ids_sxex");
        assert_eq!(strategies[1].season, Some(2));
        assert_eq!(strategies[1].episode, Some(5));
        assert_eq!(strategies[1].absolute_episode, None);

        assert_eq!(strategies[2].label, "freetext");
        assert_eq!(strategies[2].season, Some(2));
        assert_eq!(strategies[2].episode, Some(5));
        assert_eq!(strategies[2].absolute_episode, None);
    }

    #[test]
    fn anime_strategies_strip_absolute_episode_when_not_supported() {
        let caps = IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("anime".into(), vec!["anidb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("s".into()),
            episode_param: None,
            query_param: Some("q".into()),
            search_inputs: vec![IndexerSearchInputCapability::TitleQuery],
            search: true,
            imdb_search: false,
            tvdb_search: false,
            anidb_search: true,
            ..Default::default()
        };

        let ids = HashMap::from([("anidb_id".to_string(), "18886".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Silver Horizon: Beyond Journey's End S02E05",
            query_facet: "anime",
            id_facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(33),
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::Aggregate,
            text_dispatch_mode: TextDispatchMode::FacetScoped,
            is_alias_query: false,
        });

        assert_eq!(strategies.len(), 2);
        assert_eq!(strategies[0].label, "ids");
        assert_eq!(strategies[0].absolute_episode, None);
        assert_eq!(strategies[0].episode, None);
        assert_eq!(strategies[1].label, "freetext");
        assert_eq!(strategies[1].absolute_episode, None);
        assert_eq!(strategies[1].episode, None);
    }

    fn strategy_with_label(label: &str) -> SearchStrategy {
        SearchStrategy {
            request_query: "Silver Horizon S02E05".into(),
            request_facet: "anime".into(),
            ids: if label.starts_with("ids") {
                HashMap::from([("anidb_id".to_string(), "18886".to_string())])
            } else {
                HashMap::new()
            },
            season: Some(2),
            episode: Some(5),
            absolute_episode: if label == "ids_abs" { Some(33) } else { None },
            generic_query_only: false,
            label: label.into(),
        }
    }

    #[test]
    fn auto_strategy_tier_prefers_absolute_id_and_reserves_freetext() {
        let (primary, fallback) = split_strategy_tiers(
            SearchMode::Auto,
            "anime",
            vec![
                strategy_with_label("ids_sxex"),
                strategy_with_label("freetext"),
                strategy_with_label("ids_abs"),
            ],
        );

        assert_eq!(primary.len(), 1);
        assert_eq!(primary[0].label, "ids_abs");
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].label, "freetext");
    }

    #[test]
    fn auto_strategy_tier_uses_single_text_strategy_without_ids() {
        let (primary, fallback) = split_strategy_tiers(
            SearchMode::Auto,
            "anime",
            vec![
                strategy_with_label("freetext_alias"),
                strategy_with_label("freetext"),
            ],
        );

        assert_eq!(primary.len(), 1);
        assert_eq!(primary[0].label, "freetext");
        assert!(fallback.is_empty());
    }

    #[test]
    fn interactive_strategy_tier_keeps_parallel_id_strategies() {
        let (primary, fallback) = split_strategy_tiers(
            SearchMode::Interactive,
            "anime",
            vec![
                strategy_with_label("ids_abs"),
                strategy_with_label("ids_sxex"),
                strategy_with_label("freetext"),
            ],
        );

        assert_eq!(
            primary
                .iter()
                .map(|strategy| strategy.label.as_str())
                .collect::<Vec<_>>(),
            vec!["ids_abs", "ids_sxex"]
        );
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].label, "freetext");
    }

    #[test]
    fn auto_fallback_tier_is_not_spent_after_primary_error() {
        let fallback = vec![strategy_with_label("freetext")];

        assert!(!should_run_fallback_tier(
            SearchMode::Auto,
            &[],
            true,
            true,
            &fallback
        ));
        assert!(should_run_fallback_tier(
            SearchMode::Interactive,
            &[],
            true,
            true,
            &fallback
        ));
    }

    #[test]
    fn preferred_anime_alias_query_strips_episode_context() {
        let alias = preferred_anime_alias_query(
            "Silver Horizon: Beyond Journey's End S02E05",
            &[scryer_domain::TaggedAlias {
                name: "Sora no Vale".into(),
                language: "jpn".into(),
            }],
        );

        assert_eq!(alias.as_deref(), Some("Sora no Vale"));
    }

    #[test]
    fn preferred_anime_alias_query_skips_canonical_alias_and_uses_distinct_romanized_alias() {
        let alias = preferred_anime_alias_query(
            "Silver Horizon: Beyond Journey's End S02E05",
            &[
                scryer_domain::TaggedAlias {
                    name: "Silver Horizon: Beyond Journey's End".into(),
                    language: "jpn".into(),
                },
                scryer_domain::TaggedAlias {
                    name: "Sora no Vale".into(),
                    language: "jpn".into(),
                },
            ],
        );

        assert_eq!(alias.as_deref(), Some("Sora no Vale"));
    }

    #[tokio::test]
    async fn indexer_rate_limiter_keeps_interactive_lane_independent_from_auto_lane() {
        let limiter = IndexerRateLimiter::new();

        limiter.acquire("idx", None, SearchMode::Auto).await;

        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            limiter.acquire("idx", None, SearchMode::Interactive),
        )
        .await
        .expect("interactive lane should not wait behind background auto pacing");
    }

    #[tokio::test]
    async fn indexer_rate_limiter_uses_shorter_auto_default_interval() {
        let limiter = IndexerRateLimiter::new();

        limiter.acquire("idx", None, SearchMode::Auto).await;

        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                limiter.acquire("idx", None, SearchMode::Auto),
            )
            .await
            .is_err(),
            "immediate follow-up auto searches should still be paced"
        );

        tokio::time::sleep(std::time::Duration::from_millis(950)).await;

        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            limiter.acquire("idx", None, SearchMode::Auto),
        )
        .await
        .expect("auto lane should become available again after roughly one second");
    }

    #[test]
    fn anime_alias_strategy_is_freetext_only_and_skips_ids() {
        let caps = IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("anime".into(), vec!["tvdb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: false,
            tvdb_search: true,
            anidb_search: false,
            ..Default::default()
        };

        let ids = HashMap::from([("tvdb_id".to_string(), "424536".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Sora no Vale",
            query_facet: "anime",
            id_facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(33),
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::LegacyAggregate,
            text_dispatch_mode: TextDispatchMode::FacetScoped,
            is_alias_query: true,
        });

        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].label, "freetext_alias");
        assert!(strategies[0].ids.is_empty());
        assert_eq!(strategies[0].season, Some(2));
        assert_eq!(strategies[0].episode, Some(5));
        assert_eq!(strategies[0].absolute_episode, None);
    }
}
