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

/// Master switch for RFC 119 convergence. When enabled, background acquisition
/// records per-indexer coverage after each search and — once the read-gate lands
/// (Phase 1d) — prefers RSS for scopes that have converged. Defaults **off**
/// during rollout: the write-hook is inert until this is set. Flipping the
/// default on is gated on the read-gate plus per-indexer query-outcome plumbing,
/// so a transiently failed/backed-off indexer is never recorded as covered.
pub(crate) const ACQUISITION_RSS_FIRST_ENABLED_KEY: &str = "acquisition.rss_first_enabled";

/// How many uncovered `(scope, indexer)` pairs the cold (long-tail) lane may
/// enqueue per cycle. The pacer that stops a freshly added indexer from
/// stampeding; the scheduler (plan 112) further caps effective spend by quota.
#[allow(dead_code)] // wired by the convergence cursor (RFC 119 Phase 1d)
pub(crate) const ACQUISITION_LONG_TAIL_BACKFILL_BATCH_PER_CYCLE_KEY: &str =
    "acquisition.long_tail_backfill_batch_per_cycle";

/// Optional slow re-converge backstop: coverage older than this many days is
/// treated as stale and re-converged (insurance against RSS feed gaps).
/// `0` = off (the default).
#[allow(dead_code)] // wired by the convergence cursor (RFC 119 Phase 1d)
pub(crate) const ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY: &str =
    "acquisition.long_tail_reconverge_days";

#[allow(dead_code)] // wired by the convergence cursor (RFC 119 Phase 1d)
pub(crate) const DEFAULT_LONG_TAIL_BACKFILL_BATCH_PER_CYCLE: i64 = 25;

#[derive(Debug, Clone)]
#[allow(dead_code)] // wired by the convergence cursor (RFC 119 Phase 1d)
pub(crate) struct ConvergenceSettings {
    pub rss_first_enabled: bool,
    pub long_tail_backfill_batch_per_cycle: i64,
    /// `None` when the backstop is off.
    pub long_tail_reconverge: Option<chrono::Duration>,
}

impl AppUseCase {
    pub(crate) async fn convergence_settings(&self) -> AppResult<ConvergenceSettings> {
        let rss_first_enabled = self
            .read_setting_bool_value(ACQUISITION_RSS_FIRST_ENABLED_KEY, None)
            .await?
            .unwrap_or(false);
        let long_tail_backfill_batch_per_cycle = self
            .read_setting_i64_value(ACQUISITION_LONG_TAIL_BACKFILL_BATCH_PER_CYCLE_KEY, None)
            .await?
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_LONG_TAIL_BACKFILL_BATCH_PER_CYCLE);
        let reconverge_days = self
            .read_setting_i64_value(ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY, None)
            .await?
            .unwrap_or(0);
        let long_tail_reconverge = (reconverge_days > 0)
            .then(|| chrono::Duration::days(reconverge_days))
            .filter(|d| *d > chrono::Duration::zero());

        Ok(ConvergenceSettings {
            rss_first_enabled,
            long_tail_backfill_batch_per_cycle,
            long_tail_reconverge,
        })
    }
}

/// Canonical fingerprint for a scope's effective search criteria. Stable across
/// audio-language ordering. `profile_version` must change whenever the profile's
/// acceptance criteria (cutoff, allowed qualities, scoring) change, so an edit
/// re-opens convergence for still-unsatisfied scopes.
#[allow(dead_code)] // wired by the convergence cursor / coverage hook (Phase 1c/1d)
pub(crate) fn compute_search_fingerprint(
    profile_id: &str,
    profile_version: &str,
    required_audio_languages: &[String],
) -> String {
    let mut langs: Vec<String> = required_audio_languages
        .iter()
        .map(|lang| lang.trim().to_ascii_lowercase())
        .filter(|lang| !lang.is_empty())
        .collect();
    langs.sort();
    langs.dedup();
    let canonical = format!(
        "v1;profile={};version={};audio={}",
        profile_id.trim(),
        profile_version.trim(),
        langs.join(",")
    );
    crate::sha256_hex(canonical)
}

impl AppUseCase {
    /// Indexers among `routed_indexer_ids` that still need a convergence search
    /// for `scope_key`/`facet` under `fingerprint` — routed minus current-
    /// fingerprint coverage. The optional slow re-converge backstop treats
    /// coverage older than the configured window as uncovered.
    #[allow(dead_code)] // wired by the convergence cursor (RFC 119 Phase 1d)
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

    /// A scope has converged (→ RSS-only, no active search) once every routed
    /// indexer has current-fingerprint coverage.
    #[allow(dead_code)] // wired by the convergence cursor (RFC 119 Phase 1d)
    pub(crate) async fn scope_is_converged(
        &self,
        scope_key: &str,
        facet: &str,
        fingerprint: &str,
        routed_indexer_ids: &[String],
    ) -> AppResult<bool> {
        Ok(self
            .uncovered_indexers_for_scope(scope_key, facet, fingerprint, routed_indexer_ids)
            .await?
            .is_empty())
    }
}

/// A scope's convergence coordinates: its stable coverage key, media facet,
/// current search-criteria fingerprint, and the indexer ids routed to it. The
/// coverage write-hook (after a background search) and the convergence read-gate
/// (the RSS-only decision, Phase 1d) both derive this from the same resolution,
/// so writer and reader agree on the fingerprint by construction.
#[derive(Debug, Clone)]
pub(crate) struct ScopeConvergence {
    pub scope_key: String,
    pub facet: String,
    pub fingerprint: String,
    pub routed_indexer_ids: Vec<String>,
}

/// Stable coverage key for a submission scope, or `None` for scopes that are not
/// a single convergence unit (episode sets, orphans) and so are never recorded
/// as converged.
pub(crate) fn convergence_scope_key(scope: &SubmissionScope, title_id: &str) -> Option<String> {
    match scope {
        SubmissionScope::Episode { episode_id } => Some(format!("episode:{episode_id}")),
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => Some(format!("series_movie:{series_movie_link_id}")),
        SubmissionScope::Collection { collection_id } => Some(format!("collection:{collection_id}")),
        SubmissionScope::Title => {
            let title_id = title_id.trim();
            (!title_id.is_empty()).then(|| format!("title:{title_id}"))
        }
        SubmissionScope::EpisodeSet { .. } | SubmissionScope::Orphan => None,
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

    /// Record convergence coverage for a background acquisition search that just
    /// queried this scope's routed indexers. Gated behind
    /// `acquisition.rss_first_enabled` (default off during rollout); a no-op when
    /// disabled, when the scope is not a convergence unit, or when nothing is
    /// routed. Best-effort: a failed write is logged, never propagated, so it can
    /// never break the acquisition path.
    pub(crate) async fn record_background_search_coverage(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
    ) {
        let enabled = self
            .convergence_settings()
            .await
            .map(|settings| settings.rss_first_enabled)
            .unwrap_or(false);
        if !enabled {
            return;
        }
        let Some(convergence) = self.resolve_scope_convergence(title, subject).await else {
            return;
        };
        for indexer_id in &convergence.routed_indexer_ids {
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
        let a = compute_search_fingerprint("p1", "v1", &["en".into(), "ja".into()]);
        let b = compute_search_fingerprint("p1", "v1", &["JA".into(), " en ".into()]);
        assert_eq!(a, b, "audio-language order/case/whitespace must not matter");
    }

    #[test]
    fn fingerprint_changes_on_profile_version() {
        let a = compute_search_fingerprint("p1", "v1", &["en".into()]);
        let b = compute_search_fingerprint("p1", "v2", &["en".into()]);
        assert_ne!(
            a, b,
            "a profile edit (version bump) must change the fingerprint"
        );
    }

    #[test]
    fn fingerprint_changes_on_profile_id_and_audio() {
        let base = compute_search_fingerprint("p1", "v1", &["en".into()]);
        assert_ne!(base, compute_search_fingerprint("p2", "v1", &["en".into()]));
        assert_ne!(
            base,
            compute_search_fingerprint("p1", "v1", &["en".into(), "ja".into()])
        );
        assert_ne!(base, compute_search_fingerprint("p1", "v1", &[]));
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
        // Scopes that are not a single convergence unit are never recorded.
        assert_eq!(convergence_scope_key(&SubmissionScope::Title, "   "), None);
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::EpisodeSet {
                    episode_ids: vec!["e1".into()]
                },
                "t1"
            ),
            None
        );
        assert_eq!(convergence_scope_key(&SubmissionScope::Orphan, "t1"), None);
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
