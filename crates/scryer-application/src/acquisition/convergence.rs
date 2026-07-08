//! RFC 119 convergence model: search-criteria fingerprints and the settings
//! that pace the background convergence cursor.
//!
//! A scope's *fingerprint* captures what a "correct" search is — the effective
//! quality profile (identity + a version that bumps on edits) and the required
//! audio languages (subtitles are a separate subsystem and never a factor). It
//! is stored on `scope_indexer_coverage` rows; when the live fingerprint differs
//! from a row's, that coverage is stale (the scope re-converges). The
//! fingerprint only governs *coverage staleness* — target membership
//! (missing / below-cutoff / missing-audio) is a separate, prior gate, so a
//! fingerprint change never resurrects a requirements-met scope.

use super::*;
use crate::acquisition_release_search::ResolvedReleaseSearchSubject;
use crate::app_usecase_discovery::QualityProfileLookup;
use crate::quality_profile::QualityProfileCriteria;
use scryer_domain::Title;

/// Per-tick evaluation cost ceiling for the convergence cursor (§D3): how many
/// scopes the cursor may *evaluate* per cycle (coverage lookup, routing resolve,
/// fingerprint compute). Sized above the scheduler's realistic per-tick
/// admission capacity so plan-112 backpressure — never this count — is what
/// paces actual requests.
pub(crate) const ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY: &str =
    "acquisition.long_tail_backfill_max_scopes_per_cycle";

/// Optional slow re-converge backstop: coverage older than this many days is
/// treated as stale and re-converged (insurance against a lossy RSS feed).
/// `0` = off — the default and the intended steady state (§D6): we trust RSS,
/// and this knob exists only as break-glass for a feed that proves incomplete.
pub(crate) const ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY: &str =
    "acquisition.long_tail_reconverge_days";

pub(crate) const DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE: i64 = 500;

#[derive(Debug, Clone)]
pub(crate) struct ConvergenceSettings {
    pub long_tail_backfill_max_scopes_per_cycle: i64,
    /// `None` when the backstop is off.
    pub long_tail_reconverge: Option<chrono::Duration>,
}

impl AppUseCase {
    pub(crate) async fn convergence_settings(&self) -> AppResult<ConvergenceSettings> {
        let long_tail_backfill_max_scopes_per_cycle = self
            .read_setting_i64_value(
                ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
                None,
            )
            .await?
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE);
        let reconverge_days = self
            .read_setting_i64_value(ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY, None)
            .await?
            .unwrap_or(0);
        let long_tail_reconverge = (reconverge_days > 0)
            .then(|| chrono::Duration::days(reconverge_days))
            .filter(|d| *d > chrono::Duration::zero());

        Ok(ConvergenceSettings {
            long_tail_backfill_max_scopes_per_cycle,
            long_tail_reconverge,
        })
    }
}

/// System-settings key persisting the cold-lane rotation position across
/// cycles and restarts (§D3: the cursor is keyed on the last-considered
/// scope_key, not a numeric offset, so it survives target-set changes).
pub(crate) const ACQUISITION_CONVERGENCE_RESUME_AFTER_KEY: &str =
    "acquisition.convergence_resume_after";

/// Marker set once the run-once cutover seed has completed (RFC 119 §12.3).
pub(crate) const ACQUISITION_CONVERGENCE_SEEDED_AT_KEY: &str = "acquisition.convergence_seeded_at";

/// Scopes the legacy scheduler searched within this window start converged at
/// cutover instead of being re-swept on first boot.
const CUTOVER_SEED_RECENT_SEARCH_DAYS: i64 = 14;

impl AppUseCase {
    /// Run-once cutover reconciliation (RFC 119 §12.3): scopes with a recent
    /// legacy search start *converged* — coverage recorded for every routed
    /// indexer under the current fingerprint — so the first convergence sweep
    /// only covers what the old scheduler had genuinely not searched.
    /// Best-effort and idempotent: imperfect seeding only causes a safe
    /// re-converge, so any failure is logged and skipped.
    pub(crate) async fn seed_convergence_from_legacy_history(&self) {
        let already_seeded = self
            .services
            .config
            .settings
            .get_setting_json_explicit(
                SETTINGS_SCOPE_SYSTEM,
                ACQUISITION_CONVERGENCE_SEEDED_AT_KEY,
                None,
            )
            .await
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<String>(&value).ok())
            .is_some_and(|value| !value.trim().is_empty());
        if already_seeded {
            return;
        }

        let cutoff = chrono::Utc::now() - chrono::Duration::days(CUTOVER_SEED_RECENT_SEARCH_DAYS);
        let items = match self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(crate::contracts::AcquisitionScopeStatesQuery {
                limit: i64::MAX,
                ..crate::contracts::AcquisitionScopeStatesQuery::default()
            })
            .await
        {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(error = %error, "convergence seed: failed to list legacy state rows");
                return;
            }
        };

        let mut seeded_scopes = 0usize;
        for item in items {
            let recently_searched = item
                .last_search_at
                .as_deref()
                .and_then(crate::quality_profile::parse_published_at)
                .is_some_and(|searched_at| searched_at >= cutoff);
            if !recently_searched || item.status != AcquisitionScopeStatus::Wanted {
                continue;
            }
            let Ok(Some(title)) = self.services.catalog.titles.get_by_id(&item.title_id).await
            else {
                continue;
            };
            let episode = match item.episode_id.as_deref() {
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
                .resolve_release_search_subject_for_wanted_item(
                    &title,
                    &title,
                    &item,
                    episode.as_ref(),
                )
                .await;
            let Some(convergence) = self.resolve_scope_convergence(&title, &subject).await else {
                continue;
            };
            self.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
                .await;
            seeded_scopes += 1;
        }

        let now = chrono::Utc::now().to_rfc3339();
        if let Ok(value_json) = serde_json::to_string(&now)
            && let Err(error) = self
                .services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    ACQUISITION_CONVERGENCE_SEEDED_AT_KEY,
                    None,
                    value_json,
                    "system",
                    None,
                )
                .await
        {
            tracing::warn!(error = %error, "convergence seed: failed to persist completion marker");
        }
        tracing::info!(
            seeded_scopes,
            "convergence cutover seed complete: recently-searched scopes start converged"
        );
    }
}

/// Account quota at or below this remaining fraction counts as exhausted for
/// the cursor's pre-skip (mirrors the scheduler's own quota gate — the cursor
/// only avoids spending evaluation budget on requests the scheduler would
/// refuse anyway).
const QUOTA_EXHAUSTED_REMAINING_FRACTION: f64 = 0.01;

/// Per-cycle view of which scheduler hosts/accounts can take background work
/// right now, derived from the plan-112 snapshot. A stale quota observation
/// counts as available so a wedged probe can never starve the lane. This is a
/// *pre-skip* only — admission stays entirely the scheduler's inside the
/// search; the cursor just declines to spend evaluation budget on scopes whose
/// every routed indexer is currently unreachable.
pub(crate) struct SchedulerAvailability {
    cooled_hosts: std::collections::HashSet<String>,
    exhausted_accounts: std::collections::HashSet<String>,
}

impl SchedulerAvailability {
    /// An indexer can be searched when its host is not cooling down and its
    /// account quota (keyed by indexer config id) is not exhausted.
    pub fn indexer_available(&self, host_key: Option<&str>, indexer_id: &str) -> bool {
        if let Some(host) = host_key
            && self.cooled_hosts.contains(host)
        {
            return false;
        }
        !self
            .exhausted_accounts
            .contains(&indexer_id.trim().to_ascii_lowercase())
    }
}

/// The scheduler host key for an indexer base URL — the URL's host, matching
/// the keys the plan-112 snapshot reports.
pub(crate) fn indexer_scheduler_host_key(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    url::Url::parse(trimmed)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()))
        .or_else(|| Some(trimmed.to_ascii_lowercase()))
}

impl AppUseCase {
    pub(crate) async fn scheduler_availability(&self) -> SchedulerAvailability {
        let now = chrono::Utc::now();
        let mut cooled_hosts = std::collections::HashSet::new();
        let mut exhausted_accounts = std::collections::HashSet::new();
        match self
            .upstream_scheduler_snapshot(
                crate::upstream_scheduler::SchedulerSnapshotFilter::default(),
            )
            .await
        {
            Ok(snapshot) => {
                for entry in snapshot.entries {
                    if entry.cooldown_until.is_some_and(|until| until > now) {
                        cooled_hosts.insert(entry.host_key.as_str().to_string());
                    }
                    if !entry.quota_stale
                        && entry
                            .api_remaining_fraction
                            .is_some_and(|fraction| fraction <= QUOTA_EXHAUSTED_REMAINING_FRACTION)
                        && let Some(account) = entry.account_quota_key.as_ref()
                    {
                        exhausted_accounts.insert(account.as_str().to_string());
                    }
                }
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "scheduler snapshot unavailable; cursor pre-skip disabled this cycle"
                );
            }
        }
        SchedulerAvailability {
            cooled_hosts,
            exhausted_accounts,
        }
    }

    /// Indexer config id → scheduler host key, for the cursor's pre-skip.
    pub(crate) async fn indexer_scheduler_host_keys(
        &self,
    ) -> std::collections::HashMap<String, String> {
        self.services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|config| {
                indexer_scheduler_host_key(&config.base_url).map(|host| (config.id, host))
            })
            .collect()
    }

    /// The persisted cold-lane rotation position (§D3), if any.
    pub(crate) async fn convergence_cursor_resume_position(&self) -> Option<String> {
        let value_json = self
            .services
            .config
            .settings
            .get_setting_json_explicit(
                SETTINGS_SCOPE_SYSTEM,
                ACQUISITION_CONVERGENCE_RESUME_AFTER_KEY,
                None,
            )
            .await
            .ok()
            .flatten()?;
        serde_json::from_str::<String>(&value_json)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    /// Persist the cold-lane rotation position for the next cycle.
    pub(crate) async fn store_convergence_cursor_resume_position(&self, position: Option<&str>) {
        let value = position.unwrap_or_default();
        let Ok(value_json) = serde_json::to_string(value) else {
            return;
        };
        if let Err(error) = self
            .services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                ACQUISITION_CONVERGENCE_RESUME_AFTER_KEY,
                None,
                value_json,
                "system",
                None,
            )
            .await
        {
            tracing::warn!(error = %error, "failed to persist convergence cursor position");
        }
    }
}

/// Canonical fingerprint for a scope's effective search criteria. Stable across
/// audio-language ordering. `profile_version` must change whenever the profile's
/// acceptance criteria (cutoff, allowed qualities, scoring) change, and
/// `match_identity` (the scope's SMG match — its resolved external ids) must change
/// on a rematch; either re-opens convergence for still-unsatisfied scopes
/// (RFC 119 §D2). The profile inputs are the *effective* profile (resolved with
/// library/tag/category scoping), so overrides fold in.
pub(crate) fn compute_search_fingerprint(
    profile_id: &str,
    profile_version: &str,
    required_audio_languages: &[String],
    match_identity: &str,
) -> String {
    let mut langs: Vec<String> = required_audio_languages
        .iter()
        .map(|lang| lang.trim().to_ascii_lowercase())
        .filter(|lang| !lang.is_empty())
        .collect();
    langs.sort();
    langs.dedup();
    let canonical = format!(
        "v2;profile={};version={};audio={};match={}",
        profile_id.trim(),
        profile_version.trim(),
        langs.join(","),
        match_identity.trim(),
    );
    crate::sha256_hex(canonical)
}

/// Canonical identity of a scope's SMG match — its resolved external ids. A rematch
/// re-maps the title to a different canonical subject, changing these ids, so folding
/// them into the fingerprint re-opens convergence (RFC 119 §D2). Plain metadata edits
/// that leave the match unchanged do not.
fn scope_match_identity(subject: &ResolvedReleaseSearchSubject) -> String {
    fn part(label: &str, value: &Option<String>) -> String {
        format!(
            "{label}={}",
            value.as_deref().map(str::trim).unwrap_or_default()
        )
    }
    [
        part("imdb", &subject.imdb_id),
        part("tmdb", &subject.tmdb_id),
        part("tvdb", &subject.tvdb_id),
        part("anidb", &subject.anidb_id),
        part("mal", &subject.mal_id),
    ]
    .join(";")
}

impl AppUseCase {
    /// Indexers among `routed_indexer_ids` that still need a convergence search
    /// for `scope_key`/`facet` under `fingerprint` — routed minus current-
    /// fingerprint coverage. The optional slow re-converge backstop treats
    /// coverage older than the configured window as uncovered.
    pub(crate) async fn uncovered_indexers_for_scope(
        &self,
        scope_key: &str,
        facet: &str,
        fingerprint: &str,
        routed_indexer_ids: &[String],
    ) -> AppResult<Vec<String>> {
        if routed_indexer_ids.is_empty() {
            return Ok(Vec::new());
        }
        let stale_before = self
            .convergence_settings()
            .await?
            .long_tail_reconverge
            .map(|window| chrono::Utc::now() - window);
        let covered: std::collections::HashSet<String> = self
            .services
            .integrations
            .scope_indexer_coverage
            .covered_indexers(scope_key, facet, fingerprint, stale_before)
            .await?
            .into_iter()
            .collect();
        Ok(routed_indexer_ids
            .iter()
            .filter(|id| !covered.contains(id.as_str()))
            .cloned()
            .collect())
    }
}

/// A scope's convergence coordinates: its stable coverage key, media facet,
/// current search-criteria fingerprint, and the indexer ids routed to it. The
/// coverage write-hook (after a search) and the convergence read-gate (the
/// RSS-only decision) both derive this from the same resolution, so writer and
/// reader agree on the fingerprint by construction.
#[derive(Debug, Clone)]
pub(crate) struct ScopeConvergence {
    pub scope_key: String,
    pub facet: String,
    pub fingerprint: String,
    pub routed_indexer_ids: Vec<String>,
}

/// Stable coverage key for a submission scope, or `None` for a true `Orphan` (no
/// derivable target identity), which is never a convergence unit. Episode sets /
/// season packs converge as first-class units keyed on their canonical member set
/// (RFC 119 §D2 #1); a member-set change yields a new key (re-converges).
pub(crate) fn convergence_scope_key(scope: &SubmissionScope, title_id: &str) -> Option<String> {
    match scope {
        SubmissionScope::Episode { episode_id } => Some(format!("episode:{episode_id}")),
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => Some(format!("series_movie:{series_movie_link_id}")),
        SubmissionScope::Collection { collection_id } => {
            Some(format!("collection:{collection_id}"))
        }
        SubmissionScope::Title => {
            let title_id = title_id.trim();
            (!title_id.is_empty()).then(|| format!("title:{title_id}"))
        }
        SubmissionScope::EpisodeSet { episode_ids } => {
            let mut ids: Vec<&str> = episode_ids
                .iter()
                .map(|id| id.trim())
                .filter(|id| !id.is_empty())
                .collect();
            ids.sort_unstable();
            ids.dedup();
            (!ids.is_empty()).then(|| format!("episode_set:{}", crate::sha256_hex(ids.join(","))))
        }
        SubmissionScope::Orphan => None,
    }
}

/// Deterministic version string for a quality profile's acceptance criteria. Any
/// edit that changes acceptance (cutoff, tiers, codecs, required audio, scoring)
/// changes this, so the fingerprint changes and still-unsatisfied scopes re-open
/// for convergence. Canonical (recursively sorted-key) JSON keeps the hash stable
/// regardless of map iteration order.
pub(crate) fn profile_criteria_version(criteria: &QualityProfileCriteria) -> String {
    let value = serde_json::to_value(criteria).unwrap_or(serde_json::Value::Null);
    crate::sha256_hex(canonical_json_string(&value))
}

fn canonical_json_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = String::from("{");
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                out.push_str(&canonical_json_string(&map[key]));
            }
            out.push('}');
            out
        }
        serde_json::Value::Array(items) => {
            let mut out = String::from("[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&canonical_json_string(item));
            }
            out.push(']');
            out
        }
        other => other.to_string(),
    }
}

impl AppUseCase {
    /// Convergence coordinates for an active background search of `subject` under
    /// `title`, or `None` when the scope is not a single convergence unit or no
    /// indexers are routed to it.
    pub(crate) async fn resolve_scope_convergence(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
    ) -> Option<ScopeConvergence> {
        let scope_key = convergence_scope_key(&subject.submission_scope, &subject.title_id)?;
        let facet = subject.owner_facet.as_str().to_string();

        let context = self
            .resolve_upgrade_context_for_title_with_category_and_quality(
                title,
                subject.grabbed_release.as_deref(),
                Some(subject.category.as_str()),
                None,
            )
            .await;
        let fingerprint = compute_search_fingerprint(
            &context.profile.id,
            &profile_criteria_version(&context.profile.criteria),
            &context.profile.criteria.required_audio_languages,
            &scope_match_identity(subject),
        );

        let routed_indexer_ids = self.routed_indexer_ids_for_search(title, subject).await;
        if routed_indexer_ids.is_empty() {
            return None;
        }

        Some(ScopeConvergence {
            scope_key,
            facet,
            fingerprint,
            routed_indexer_ids,
        })
    }

    /// The indexer ids an active search of `subject` targets — the enabled routing
    /// entries for the scope, or every configured indexer when no routing is set.
    /// Mirrors the indexer selection in `search_and_score_releases` so coverage is
    /// recorded for exactly the indexers a search would query.
    async fn routed_indexer_ids_for_search(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
    ) -> Vec<String> {
        let lookup = QualityProfileLookup {
            title_tags: &subject.title_tags,
            library_id: Some(title.library_id.as_str()),
            imdb_id: subject.imdb_id.as_deref(),
            tvdb_id: subject.tvdb_id.as_deref(),
            category_hint: Some(subject.owner_facet.as_str()),
        };
        let scope_id = self.quality_profile_scope_id(lookup);
        match self
            .resolve_indexer_routing(Some(title.library_id.as_str()), scope_id.as_deref())
            .await
        {
            Some(plan) => plan
                .entries
                .into_iter()
                .filter(|(_, entry)| entry.enabled)
                .map(|(indexer_id, _)| indexer_id)
                .collect(),
            None => self
                .services
                .integrations
                .indexer_configs
                .list(None)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|config| config.is_enabled)
                .map(|config| config.id)
                .collect(),
        }
    }

    /// Record convergence coverage for a search of `subject`. A search is a
    /// search (§D5): background, interactive, and season-pack searches all
    /// record coverage, since any of them proves what the indexer's catalog
    /// holds for this scope. Records **only the indexers that actually fired a
    /// query and returned a response** (`fired_indexer_ids`, from the augmented
    /// search return) intersected with the scope's routed set — never a routed
    /// indexer the scheduler deferred/skipped or whose query errored (§D2). An
    /// empty response still counts: "no results" is coverage. A no-op when the
    /// scope is not a convergence unit or when nothing fired. Best-effort: a
    /// failed write is logged, never propagated, so it can never break the
    /// acquisition path.
    pub(crate) async fn record_search_coverage(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        fired_indexer_ids: &[String],
    ) {
        let Some(convergence) = self.resolve_scope_convergence(title, subject).await else {
            return;
        };
        // Only routed indexers that actually fired are recorded as covered; a
        // deferred/skipped/errored routed indexer stays uncovered so the cursor
        // retries it (RFC 119 §D2).
        let fired: std::collections::HashSet<&str> =
            fired_indexer_ids.iter().map(String::as_str).collect();
        for indexer_id in &convergence.routed_indexer_ids {
            if !fired.contains(indexer_id.as_str()) {
                continue;
            }
            if let Err(error) = self
                .services
                .integrations
                .scope_indexer_coverage
                .record_coverage(
                    &convergence.scope_key,
                    &convergence.facet,
                    indexer_id,
                    &convergence.fingerprint,
                )
                .await
            {
                tracing::warn!(
                    scope_key = convergence.scope_key.as_str(),
                    facet = convergence.facet.as_str(),
                    indexer_id = indexer_id.as_str(),
                    error = %error,
                    "failed to record convergence coverage"
                );
            }
        }
    }

    /// Re-open a scope's convergence after an event that invalidates its
    /// acquired state — a failed grab, a rejected import, or an operator
    /// replacing the download: reset the state row to `wanted` (clearing the
    /// in-flight grab, keeping the upgrade baseline), prune the scope's
    /// coverage so the cursor re-searches every routed indexer, and wake the
    /// acquisition loop. Best-effort — recovery paths must never fail on
    /// bookkeeping.
    pub(crate) async fn reopen_wanted_scope_for_acquisition(&self, item: &AcquisitionScopeState) {
        if let Err(error) = self
            .services
            .workflow
            .acquisition_scope_states
            .transition_acquisition_scope_to_reopened(&item.id)
            .await
        {
            tracing::warn!(
                wanted_item_id = item.id.as_str(),
                error = %error,
                "failed to reset wanted state row while re-opening scope"
            );
        }
        let scope = crate::contracts::SubmissionScope::from_persisted(
            &item.title_id,
            item.episode_id.clone(),
            item.collection_id.clone(),
            item.series_movie_link_id.clone(),
            None,
        );
        if let Some(scope_key) = convergence_scope_key(&scope, &item.title_id)
            && let Err(error) = self
                .services
                .integrations
                .scope_indexer_coverage
                .prune_scope(&scope_key)
                .await
        {
            tracing::warn!(
                wanted_item_id = item.id.as_str(),
                scope_key = scope_key.as_str(),
                error = %error,
                "failed to prune scope coverage while re-opening scope"
            );
        }
        self.runtime.acquisition.acquisition_wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_json_string, compute_search_fingerprint, convergence_scope_key,
        profile_criteria_version,
    };
    use crate::contracts::SubmissionScope;
    use crate::quality_profile::QualityProfileCriteria;

    #[test]
    fn fingerprint_is_stable_and_order_independent_for_audio() {
        let a = compute_search_fingerprint("p1", "v1", &["en".into(), "ja".into()], "m1");
        let b = compute_search_fingerprint("p1", "v1", &["JA".into(), " en ".into()], "m1");
        assert_eq!(a, b, "audio-language order/case/whitespace must not matter");
    }

    #[test]
    fn fingerprint_changes_on_profile_version() {
        let a = compute_search_fingerprint("p1", "v1", &["en".into()], "m1");
        let b = compute_search_fingerprint("p1", "v2", &["en".into()], "m1");
        assert_ne!(
            a, b,
            "a profile edit (version bump) must change the fingerprint"
        );
    }

    #[test]
    fn fingerprint_changes_on_profile_id_and_audio() {
        let base = compute_search_fingerprint("p1", "v1", &["en".into()], "m1");
        assert_ne!(
            base,
            compute_search_fingerprint("p2", "v1", &["en".into()], "m1")
        );
        assert_ne!(
            base,
            compute_search_fingerprint("p1", "v1", &["en".into(), "ja".into()], "m1")
        );
        assert_ne!(base, compute_search_fingerprint("p1", "v1", &[], "m1"));
    }

    #[test]
    fn fingerprint_changes_on_rematch() {
        // A rematch changes the scope's external-id identity → new fingerprint →
        // convergence re-opens (RFC 119 §D2 #2).
        let a = compute_search_fingerprint("p1", "v1", &["en".into()], "imdb=tt1");
        let b = compute_search_fingerprint("p1", "v1", &["en".into()], "imdb=tt2");
        assert_ne!(
            a, b,
            "a rematch (changed SMG match id) must change the fingerprint"
        );
    }

    fn test_criteria() -> QualityProfileCriteria {
        crate::quality_profile::default_quality_profile_for_search().criteria
    }

    #[test]
    fn convergence_scope_key_maps_each_scope_kind() {
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::Episode {
                    episode_id: "e1".into()
                },
                "t1"
            ),
            Some("episode:e1".to_string())
        );
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::SeriesMovie {
                    series_movie_link_id: "l1".into()
                },
                "t1"
            ),
            Some("series_movie:l1".to_string())
        );
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::Collection {
                    collection_id: "c1".into()
                },
                "t1"
            ),
            Some("collection:c1".to_string())
        );
        assert_eq!(
            convergence_scope_key(&SubmissionScope::Title, "t1"),
            Some("title:t1".to_string())
        );
        // A true orphan (and an empty title) is never a convergence unit.
        assert_eq!(convergence_scope_key(&SubmissionScope::Title, "   "), None);
        assert_eq!(convergence_scope_key(&SubmissionScope::Orphan, "t1"), None);

        // Episode sets / season packs DO converge, keyed on their canonical member
        // set — order/whitespace/duplicate independent, empty-set excluded.
        let pack_ab = convergence_scope_key(
            &SubmissionScope::EpisodeSet {
                episode_ids: vec!["e1".into(), "e2".into()],
            },
            "t1",
        );
        assert!(
            pack_ab
                .as_deref()
                .is_some_and(|key| key.starts_with("episode_set:"))
        );
        assert_eq!(
            pack_ab,
            convergence_scope_key(
                &SubmissionScope::EpisodeSet {
                    episode_ids: vec![" e2 ".into(), "e1".into(), "e1".into()],
                },
                "t1",
            ),
            "canonical member set is order/whitespace/duplicate independent"
        );
        assert_ne!(
            pack_ab,
            convergence_scope_key(
                &SubmissionScope::EpisodeSet {
                    episode_ids: vec!["e1".into(), "e3".into()],
                },
                "t1",
            ),
            "a different member set is a different pack scope"
        );
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::EpisodeSet {
                    episode_ids: vec![]
                },
                "t1"
            ),
            None
        );
    }

    #[test]
    fn canonical_json_string_is_key_order_independent() {
        let a = serde_json::json!({ "b": 1, "a": [ { "y": 1, "x": 2 } ] });
        let b = serde_json::json!({ "a": [ { "x": 2, "y": 1 } ], "b": 1 });
        assert_eq!(canonical_json_string(&a), canonical_json_string(&b));
        assert!(canonical_json_string(&a).starts_with("{\"a\":"));
    }

    #[test]
    fn profile_criteria_version_is_stable_and_edit_sensitive() {
        let base = test_criteria();
        assert_eq!(
            profile_criteria_version(&base),
            profile_criteria_version(&base.clone()),
            "the same criteria must hash to the same version"
        );

        let mut edited = base.clone();
        edited.allow_upgrades = !base.allow_upgrades;
        assert_ne!(
            profile_criteria_version(&base),
            profile_criteria_version(&edited),
            "an acceptance-criteria edit must change the version"
        );

        let mut audio_edited = base.clone();
        audio_edited.required_audio_languages.push("ja".to_string());
        assert_ne!(
            profile_criteria_version(&base),
            profile_criteria_version(&audio_edited),
            "a required-audio change must change the version"
        );
    }
}
