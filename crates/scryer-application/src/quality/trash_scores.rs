use crate::quality_profile::BLOCK_SCORE;

/// Upstream scores at or below this magnitude are tie-breakers, not ranking
/// axes, and pass through unchanged.
pub const TRASH_PASSTHROUGH_KNEE: i64 = 100;

/// Upper bound of the proportional band, so upstream informs ranking instead of
/// deciding it outright against Scryer's native weights.
pub const TRASH_CEILING: i64 = 1000;

/// Map an upstream TRaSH score into Scryer's scoring band.
///
/// Vetoes stay semantic: anything at or below `BLOCK_SCORE` blocks and is never
/// scaled. Below the knee the map is the identity, so upstream's fine-grained
/// ordering survives exactly. Above it the score is expressed as a fraction of
/// its own score set's cutoff — `set_veto_magnitude`, the smallest veto that set
/// uses — so the same intent produces the same output across sets whose cutoffs
/// differ by 3.5x. The clamp is negative-side only, so strong positive
/// requirements are compressed rather than blocked.
pub fn normalize_trash_score(upstream: i64, set_veto_magnitude: i64) -> i32 {
    if upstream <= i64::from(BLOCK_SCORE) {
        return BLOCK_SCORE;
    }

    let magnitude = upstream.abs();
    let span = set_veto_magnitude - TRASH_PASSTHROUGH_KNEE;
    let scaled = if magnitude <= TRASH_PASSTHROUGH_KNEE {
        magnitude
    } else if span <= 0 {
        // A set whose standard veto sits at or below the knee has no
        // proportional band left to divide, so everything above it is extreme.
        TRASH_CEILING
    } else {
        let over = (magnitude - TRASH_PASSTHROUGH_KNEE).min(span);
        TRASH_PASSTHROUGH_KNEE + over * (TRASH_CEILING - TRASH_PASSTHROUGH_KNEE) / span
    };

    (upstream.signum() * scaled) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_VETO: i64 = 10_000;
    const GERMAN_VETO: i64 = 35_000;

    #[test]
    fn tie_break_band_is_the_identity() {
        for upstream in [1, 3, 75, 100] {
            assert_eq!(
                normalize_trash_score(upstream, DEFAULT_VETO),
                upstream as i32
            );
            assert_eq!(
                normalize_trash_score(upstream, GERMAN_VETO),
                upstream as i32
            );
            assert_eq!(
                normalize_trash_score(-upstream, DEFAULT_VETO),
                -(upstream as i32)
            );
        }
    }

    #[test]
    fn proportional_band_matches_the_published_mapping() {
        assert_eq!(normalize_trash_score(1600, DEFAULT_VETO), 236);
        assert_eq!(normalize_trash_score(1800, DEFAULT_VETO), 254);
        assert_eq!(normalize_trash_score(3100, DEFAULT_VETO), 372);
        assert_eq!(normalize_trash_score(11_000, DEFAULT_VETO), 1000);

        assert_eq!(normalize_trash_score(1600, GERMAN_VETO), 138);
        assert_eq!(normalize_trash_score(1800, GERMAN_VETO), 143);
        assert_eq!(normalize_trash_score(11_000, GERMAN_VETO), 381);
    }

    #[test]
    fn equivalent_proportions_land_on_equivalent_output() {
        // 11000 of german's 35000 cutoff and 3100 of default's 10000 cutoff are
        // the same share of their own set, so they must score alike.
        let german = normalize_trash_score(11_000, GERMAN_VETO);
        let default = normalize_trash_score(3100, DEFAULT_VETO);
        assert!((german - default).abs() <= 10, "{german} vs {default}");
    }

    #[test]
    fn vetoes_block_and_strong_positives_do_not() {
        assert_eq!(normalize_trash_score(-10_000, DEFAULT_VETO), BLOCK_SCORE);
        assert_eq!(normalize_trash_score(-35_000, DEFAULT_VETO), BLOCK_SCORE);
        assert_eq!(normalize_trash_score(-35_000, GERMAN_VETO), BLOCK_SCORE);
        assert_eq!(normalize_trash_score(-50_000, GERMAN_VETO), BLOCK_SCORE);

        assert_eq!(normalize_trash_score(10_000, DEFAULT_VETO), 1000);
        assert!(normalize_trash_score(10_000, GERMAN_VETO) > 0);
    }

    #[test]
    fn degenerate_veto_magnitudes_clamp_to_the_ceiling() {
        assert_eq!(normalize_trash_score(500, TRASH_PASSTHROUGH_KNEE), 1000);
        assert_eq!(normalize_trash_score(-500, 0), -1000);
        assert_eq!(normalize_trash_score(75, 0), 75);
    }

    #[test]
    fn zero_and_near_veto_scores_stay_ordered() {
        assert_eq!(normalize_trash_score(0, DEFAULT_VETO), 0);
        assert!(
            normalize_trash_score(9999, DEFAULT_VETO) < normalize_trash_score(10_000, DEFAULT_VETO)
        );
        assert_eq!(normalize_trash_score(-9999, DEFAULT_VETO), -999);
    }
}
