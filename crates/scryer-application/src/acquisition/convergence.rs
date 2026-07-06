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

/// Master switch: when true (default) background acquisition prefers RSS and
/// converges targets once per indexer; when false long-tail scopes go straight
/// to RSS-only.
#[allow(dead_code)] // wired by the convergence cursor (RFC 119 Phase 1d)
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
    #[allow(dead_code)] // wired by the convergence cursor (RFC 119 Phase 1d)
    pub(crate) async fn convergence_settings(&self) -> AppResult<ConvergenceSettings> {
        let rss_first_enabled = self
            .read_setting_bool_value(ACQUISITION_RSS_FIRST_ENABLED_KEY, None)
            .await?
            .unwrap_or(true);
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

#[cfg(test)]
mod tests {
    use super::compute_search_fingerprint;

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
}
