use chrono::{DateTime, Duration, NaiveDate, Utc};

use crate::scoring_weights::ScoringPersona;
use crate::types::{IndexerSearchResult, TitleMediaFile};

/// Episodes become searchable six hours before air time.
pub(crate) const EPISODE_PRE_AIR_WINDOW_HOURS: i64 = 6;

/// Configurable thresholds for the acquisition upgrade policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquisitionThresholds {
    pub upgrade_cooldown_hours: i64,
    pub same_tier_min_delta: i32,
    pub cross_tier_min_delta: i32,
    pub forced_upgrade_delta_bypass: i32,
}

impl Default for AcquisitionThresholds {
    fn default() -> Self {
        Self::for_persona(&ScoringPersona::Balanced)
    }
}

impl AcquisitionThresholds {
    /// Build thresholds tuned to the given scoring persona.
    pub fn for_persona(persona: &ScoringPersona) -> Self {
        match persona {
            ScoringPersona::Audiophile => Self {
                upgrade_cooldown_hours: 12,
                same_tier_min_delta: 50,
                cross_tier_min_delta: 20,
                forced_upgrade_delta_bypass: 200,
            },
            ScoringPersona::Balanced | ScoringPersona::Compatible => Self {
                upgrade_cooldown_hours: 24,
                same_tier_min_delta: 200,
                cross_tier_min_delta: 30,
                forced_upgrade_delta_bypass: 400,
            },
            ScoringPersona::Efficient => Self {
                upgrade_cooldown_hours: 24,
                same_tier_min_delta: 150,
                cross_tier_min_delta: 40,
                forced_upgrade_delta_bypass: 500,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeDecision {
    AcceptInitial,
    AcceptUpgrade,
    RejectInsufficientDelta,
    RejectCooldown,
    RejectNotAllowed,
}

impl UpgradeDecision {
    #[cfg(test)]
    pub fn code(&self) -> &'static str {
        match self {
            Self::AcceptInitial => "accept_initial",
            Self::AcceptUpgrade => "accept_upgrade",
            Self::RejectInsufficientDelta => "reject_insufficient_delta",
            Self::RejectCooldown => "reject_cooldown",
            Self::RejectNotAllowed => "reject_not_allowed",
        }
    }

    pub fn is_accept(&self) -> bool {
        matches!(self, Self::AcceptInitial | Self::AcceptUpgrade)
    }
}

pub fn evaluate_upgrade(
    candidate_score: i32,
    current_score: Option<i32>,
    allow_upgrades: bool,
    last_import_at: Option<&str>,
    now: &DateTime<Utc>,
    thresholds: &AcquisitionThresholds,
    min_score_to_grab: Option<i32>,
) -> UpgradeDecision {
    let Some(current) = current_score else {
        return UpgradeDecision::AcceptInitial;
    };

    if !allow_upgrades {
        return UpgradeDecision::RejectNotAllowed;
    }

    let delta = candidate_score - current;
    if delta <= 0 {
        return UpgradeDecision::RejectInsufficientDelta;
    }

    // Check cooldown
    if let Some(import_time_str) = last_import_at
        && let Ok(import_time) = DateTime::parse_from_rfc3339(import_time_str)
    {
        let cooldown_end = import_time + Duration::hours(thresholds.upgrade_cooldown_hours);
        if *now < cooldown_end.with_timezone(&Utc) {
            if delta >= thresholds.forced_upgrade_delta_bypass {
                return UpgradeDecision::AcceptUpgrade;
            }
            return UpgradeDecision::RejectCooldown;
        }
    }

    // Cross-tier upgrades (delta >= 1000 due to tier_weight) use the lower threshold.
    let is_cross_tier = delta >= 1000;
    if !is_cross_tier && min_score_to_grab.is_some_and(|minimum| current >= minimum) {
        return UpgradeDecision::RejectInsufficientDelta;
    }

    let min_delta = if is_cross_tier {
        thresholds.cross_tier_min_delta
    } else {
        thresholds.same_tier_min_delta
    };

    if delta >= min_delta {
        return UpgradeDecision::AcceptUpgrade;
    }

    UpgradeDecision::RejectInsufficientDelta
}

pub(crate) fn parse_schedule_baseline_date(baseline_date: Option<&str>) -> Option<DateTime<Utc>> {
    baseline_date.and_then(|d| {
        // Try RFC 3339 first, then fall back to "YYYY-MM-DD" (midnight UTC).
        DateTime::parse_from_rfc3339(d)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
            .or_else(|| {
                NaiveDate::parse_from_str(d, "%Y-%m-%d")
                    .ok()
                    .and_then(|nd| nd.and_hms_opt(0, 0, 0))
                    .map(|ndt| ndt.and_utc())
            })
    })
}

pub(crate) fn episode_search_window_is_open(
    baseline_date: Option<&str>,
    now: &DateTime<Utc>,
) -> bool {
    parse_schedule_baseline_date(baseline_date)
        .is_some_and(|baseline| *now >= baseline - Duration::hours(EPISODE_PRE_AIR_WINDOW_HOURS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> AcquisitionThresholds {
        AcquisitionThresholds::default()
    }

    #[test]
    fn test_accept_initial_when_no_current() {
        let now = Utc::now();
        let decision = evaluate_upgrade(1000, None, true, None, &now, &t(), None);
        assert_eq!(decision, UpgradeDecision::AcceptInitial);
    }

    #[test]
    fn test_reject_lower_score() {
        let now = Utc::now();
        let decision = evaluate_upgrade(500, Some(1000), true, None, &now, &t(), None);
        assert_eq!(decision, UpgradeDecision::RejectInsufficientDelta);
    }

    #[test]
    fn test_accept_sufficient_delta() {
        let now = Utc::now();
        let decision = evaluate_upgrade(1200, Some(1000), true, None, &now, &t(), None);
        assert_eq!(decision, UpgradeDecision::AcceptUpgrade);
    }

    #[test]
    fn test_reject_insufficient_delta() {
        let now = Utc::now();
        let decision = evaluate_upgrade(1050, Some(1000), true, None, &now, &t(), None);
        assert_eq!(decision, UpgradeDecision::RejectInsufficientDelta);
    }

    #[test]
    fn test_reject_cooldown() {
        let now = Utc::now();
        let recent_import = (now - Duration::hours(1)).to_rfc3339();
        let decision = evaluate_upgrade(
            1200,
            Some(1000),
            true,
            Some(&recent_import),
            &now,
            &t(),
            None,
        );
        assert_eq!(decision, UpgradeDecision::RejectCooldown);
    }

    #[test]
    fn test_accept_forced_bypass_during_cooldown() {
        let now = Utc::now();
        let recent_import = (now - Duration::hours(1)).to_rfc3339();
        let decision = evaluate_upgrade(
            1500,
            Some(1000),
            true,
            Some(&recent_import),
            &now,
            &t(),
            None,
        );
        assert_eq!(decision, UpgradeDecision::AcceptUpgrade);
    }

    #[test]
    fn test_reject_upgrades_not_allowed() {
        let now = Utc::now();
        let decision = evaluate_upgrade(2000, Some(1000), false, None, &now, &t(), None);
        assert_eq!(decision, UpgradeDecision::RejectNotAllowed);
    }

    #[test]
    fn test_cross_tier_upgrade_uses_lower_delta() {
        let now = Utc::now();
        // Cross-tier: delta >= 1000, so cross_tier_min_delta (30) applies
        let decision = evaluate_upgrade(2050, Some(1000), true, None, &now, &t(), None);
        assert_eq!(decision, UpgradeDecision::AcceptUpgrade);
    }

    #[test]
    fn test_audiophile_thresholds_are_aggressive() {
        let t = AcquisitionThresholds::for_persona(&ScoringPersona::Audiophile);
        assert_eq!(t.same_tier_min_delta, 50);
        assert_eq!(t.cross_tier_min_delta, 20);
        assert_eq!(t.upgrade_cooldown_hours, 12);
        assert_eq!(t.forced_upgrade_delta_bypass, 200);
    }

    #[test]
    fn test_balanced_thresholds_are_conservative() {
        let t = AcquisitionThresholds::for_persona(&ScoringPersona::Balanced);
        assert_eq!(t.same_tier_min_delta, 200);
        assert_eq!(t.cross_tier_min_delta, 30);
        assert_eq!(t.upgrade_cooldown_hours, 24);
    }

    #[test]
    fn test_efficient_thresholds_moderate() {
        let t = AcquisitionThresholds::for_persona(&ScoringPersona::Efficient);
        assert_eq!(t.same_tier_min_delta, 150);
        assert_eq!(t.cross_tier_min_delta, 40);
        assert_eq!(t.forced_upgrade_delta_bypass, 500);
    }

    #[test]
    fn test_compatible_matches_balanced() {
        let balanced = AcquisitionThresholds::for_persona(&ScoringPersona::Balanced);
        let compatible = AcquisitionThresholds::for_persona(&ScoringPersona::Compatible);
        assert_eq!(balanced, compatible);
    }

    #[test]
    fn test_audiophile_accepts_small_same_tier_delta() {
        let now = Utc::now();
        let thresholds = AcquisitionThresholds::for_persona(&ScoringPersona::Audiophile);
        // 60pt delta: audiophile accepts (threshold 50), balanced would reject (threshold 200)
        let decision = evaluate_upgrade(1060, Some(1000), true, None, &now, &thresholds, None);
        assert_eq!(decision, UpgradeDecision::AcceptUpgrade);
    }

    #[test]
    fn test_balanced_rejects_small_same_tier_delta() {
        let now = Utc::now();
        let thresholds = AcquisitionThresholds::for_persona(&ScoringPersona::Balanced);
        // 60pt delta: balanced rejects (threshold 200)
        let decision = evaluate_upgrade(1060, Some(1000), true, None, &now, &thresholds, None);
        assert_eq!(decision, UpgradeDecision::RejectInsufficientDelta);
    }

    #[test]
    fn test_same_tier_upgrade_rejected_when_current_already_meets_min_score_to_grab() {
        let now = Utc::now();
        let decision = evaluate_upgrade(1200, Some(1100), true, None, &now, &t(), Some(1000));
        assert_eq!(decision, UpgradeDecision::RejectInsufficientDelta);
    }

    #[test]
    fn test_cross_tier_upgrade_still_allowed_above_min_score_to_grab() {
        let now = Utc::now();
        let decision = evaluate_upgrade(2200, Some(1100), true, None, &now, &t(), Some(1000));
        assert_eq!(decision, UpgradeDecision::AcceptUpgrade);
    }
}

/// Check if a repack candidate's release group matches the existing file's
/// release group.  Returns `true` if the candidate should be **skipped**.
///
/// A REPACK is a re-release by the *same* group to fix their own encode.
/// Grabbing a different group's repack is a false upgrade — the fix is for
/// issues you don't have.
pub fn should_skip_repack_group_mismatch(
    candidate: &IndexerSearchResult,
    existing_files: &[TitleMediaFile],
    episode_id: Option<&str>,
) -> bool {
    let meta = match candidate.parsed_release_metadata.as_ref() {
        Some(m) if m.is_repack => m,
        _ => return false, // not a repack → no check
    };

    let candidate_group = match meta.release_group.as_deref() {
        Some(g) if !g.is_empty() => g,
        _ => return true, // repack with unknown group → skip
    };

    // Find existing file for this episode (series) or any file (movie)
    let existing_file = existing_files.iter().find(|f| match episode_id {
        Some(eid) => f.episode_id.as_deref() == Some(eid),
        None => true,
    });

    let Some(file) = existing_file else {
        return false; // no existing file → initial grab, allow
    };

    let existing_group = match file.release_group.as_deref() {
        Some(g) if !g.is_empty() => g,
        _ => return true, // existing file has unknown group → skip repack
    };

    !candidate_group.eq_ignore_ascii_case(existing_group)
}

#[cfg(test)]
#[path = "acquisition_policy_tests.rs"]
mod acquisition_policy_tests;
