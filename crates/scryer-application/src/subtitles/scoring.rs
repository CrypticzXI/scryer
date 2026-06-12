/// Subtitle match scoring, ported from Bazarr's scoring system.
///
/// Each factor contributes points. The `hash` score equals the sum of all
/// non-hash, non-HI factors (so a hash match guarantees maximum quality).
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ScoreWeights {
    pub hash: i32,
    pub title: i32,
    pub year: i32,
    pub season: i32,
    pub episode: i32,
    pub release_group: i32,
    pub source: i32,
    pub audio_codec: i32,
    pub resolution: i32,
    pub video_codec: i32,
    pub hearing_impaired: i32,
}

impl ScoreWeights {
    pub fn max_score(&self) -> i32 {
        self.hash + self.hearing_impaired
    }
}

/// Scoring weights for TV series episodes (from Bazarr).
pub static SERIES_WEIGHTS: SeriesScore = SeriesScore;

pub struct SeriesScore;

impl SeriesScore {
    pub fn weights(&self) -> ScoreWeights {
        ScoreWeights {
            hash: 359,
            title: 180,
            year: 90,
            season: 30,
            episode: 30,
            release_group: 15,
            source: 7,
            audio_codec: 3,
            resolution: 2,
            video_codec: 2,
            hearing_impaired: 1,
        }
    }
}

/// Scoring weights for movies (from Bazarr).
pub static MOVIE_WEIGHTS: MovieScore = MovieScore;

pub struct MovieScore;

impl MovieScore {
    pub fn weights(&self) -> ScoreWeights {
        ScoreWeights {
            hash: 119,
            title: 60,
            year: 30,
            season: 0,
            episode: 0,
            release_group: 15,
            source: 7,
            audio_codec: 3,
            resolution: 2,
            video_codec: 2,
            hearing_impaired: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleScoreKind {
    Movie,
    Episode,
}

impl SubtitleScoreKind {
    pub fn weights(self) -> ScoreWeights {
        match self {
            Self::Movie => MOVIE_WEIGHTS.weights(),
            Self::Episode => SERIES_WEIGHTS.weights(),
        }
    }

    pub fn max_score(self) -> i32 {
        self.weights().max_score()
    }
}

pub fn percent_to_raw_threshold(kind: SubtitleScoreKind, percent: i32) -> i32 {
    let percent = percent.clamp(0, 100);
    (kind.max_score() * percent + 99) / 100
}

pub fn raw_score_to_percent(kind: SubtitleScoreKind, score: i32) -> f64 {
    let max_score = kind.max_score();
    if max_score <= 0 {
        return 0.0;
    }
    (score.max(0) as f64 * 100.0) / max_score as f64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchFlag {
    Hash = 0,
    Title = 1,
    Series = 2,
    Year = 3,
    Season = 4,
    Episode = 5,
    ReleaseGroup = 6,
    Source = 7,
    AudioCodec = 8,
    Resolution = 9,
    VideoCodec = 10,
    HearingImpaired = 11,
    ImdbId = 12,
    ExternalId = 13,
    SeriesImdbId = 14,
    AbsoluteEpisode = 15,
}

impl MatchFlag {
    const fn bit(self) -> u16 {
        1u16 << (self as u8)
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "hash" => Some(Self::Hash),
            "title" => Some(Self::Title),
            "series" => Some(Self::Series),
            "year" => Some(Self::Year),
            "season" => Some(Self::Season),
            "episode" => Some(Self::Episode),
            "release_group" => Some(Self::ReleaseGroup),
            "source" => Some(Self::Source),
            "audio_codec" => Some(Self::AudioCodec),
            "resolution" => Some(Self::Resolution),
            "video_codec" => Some(Self::VideoCodec),
            "hearing_impaired" => Some(Self::HearingImpaired),
            "imdb_id" => Some(Self::ImdbId),
            "external_id" => Some(Self::ExternalId),
            "series_imdb_id" => Some(Self::SeriesImdbId),
            "absolute_episode" => Some(Self::AbsoluteEpisode),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MatchBits(u16);

impl MatchBits {
    pub(crate) fn insert(&mut self, flag: MatchFlag) {
        self.0 |= flag.bit();
    }

    pub(crate) fn contains(self, flag: MatchFlag) -> bool {
        self.0 & flag.bit() != 0
    }

    pub(crate) fn remove(&mut self, flag: MatchFlag) {
        self.0 &= !flag.bit();
    }

    fn from_flag(flag: MatchFlag) -> Self {
        let mut bits = Self::default();
        bits.insert(flag);
        bits
    }

    fn from_bool_map(matches: &HashMap<String, bool>) -> Self {
        let mut bits = Self::default();
        for (key, matched) in matches {
            if *matched && let Some(flag) = MatchFlag::from_key(key) {
                bits.insert(flag);
            }
        }
        bits
    }

    fn from_match_set(matches: &HashSet<String>) -> Self {
        let mut bits = Self::default();
        for key in matches {
            if let Some(flag) = MatchFlag::from_key(key) {
                bits.insert(flag);
            }
        }
        bits
    }
}

fn compute_score_bits(weights: &ScoreWeights, matches: MatchBits) -> i32 {
    let mut score = 0i32;
    if matches.contains(MatchFlag::Hash) {
        score += weights.hash;
    }
    if matches.contains(MatchFlag::Title) || matches.contains(MatchFlag::Series) {
        score += weights.title;
    }
    if matches.contains(MatchFlag::Year) {
        score += weights.year;
    }
    if matches.contains(MatchFlag::Season) {
        score += weights.season;
    }
    if matches.contains(MatchFlag::Episode) {
        score += weights.episode;
    }
    if matches.contains(MatchFlag::ReleaseGroup) {
        score += weights.release_group;
    }
    if matches.contains(MatchFlag::Source) {
        score += weights.source;
    }
    if matches.contains(MatchFlag::AudioCodec) {
        score += weights.audio_codec;
    }
    if matches.contains(MatchFlag::Resolution) {
        score += weights.resolution;
    }
    if matches.contains(MatchFlag::VideoCodec) {
        score += weights.video_codec;
    }
    if matches.contains(MatchFlag::HearingImpaired) {
        score += weights.hearing_impaired;
    }
    score
}

/// Compute a subtitle match score given which factors matched.
pub fn compute_score(weights: &ScoreWeights, matches: &HashMap<String, bool>) -> i32 {
    compute_score_bits(weights, MatchBits::from_bool_map(matches))
}

pub fn compute_verified_score(
    weights: &ScoreWeights,
    kind: SubtitleScoreKind,
    matches: &HashSet<String>,
    is_special: bool,
) -> i32 {
    compute_verified_score_bits(
        weights,
        kind,
        MatchBits::from_match_set(matches),
        is_special,
    )
}

pub(crate) fn compute_verified_score_bits(
    weights: &ScoreWeights,
    kind: SubtitleScoreKind,
    matches: MatchBits,
    is_special: bool,
) -> i32 {
    let mut verified = matches;

    if verified.contains(MatchFlag::Hash) {
        let required: &[MatchFlag] = match kind {
            SubtitleScoreKind::Movie => &[MatchFlag::Source, MatchFlag::VideoCodec],
            SubtitleScoreKind::Episode => &[
                MatchFlag::Series,
                MatchFlag::Season,
                MatchFlag::Episode,
                MatchFlag::Source,
            ],
        };

        if matches!(kind, SubtitleScoreKind::Movie) || !is_special {
            if required.iter().all(|flag| verified.contains(*flag)) {
                verified = MatchBits::from_flag(MatchFlag::Hash);
            } else {
                verified.remove(MatchFlag::Hash);
            }
        }
    }

    let mut expanded = verified;

    match kind {
        SubtitleScoreKind::Movie => {
            if expanded.contains(MatchFlag::ImdbId) || expanded.contains(MatchFlag::ExternalId) {
                expanded.insert(MatchFlag::Title);
                expanded.insert(MatchFlag::Year);
            }
        }
        SubtitleScoreKind::Episode => {
            if expanded.contains(MatchFlag::Title) {
                expanded.insert(MatchFlag::Episode);
            }
            if expanded.contains(MatchFlag::SeriesImdbId) {
                expanded.insert(MatchFlag::Series);
                expanded.insert(MatchFlag::Year);
            }
            if expanded.contains(MatchFlag::ImdbId) {
                expanded.insert(MatchFlag::Series);
                expanded.insert(MatchFlag::Year);
                expanded.insert(MatchFlag::Season);
                expanded.insert(MatchFlag::Episode);
            }
            if expanded.contains(MatchFlag::ExternalId) {
                expanded.insert(MatchFlag::Series);
                expanded.insert(MatchFlag::Year);
            }
            if expanded.contains(MatchFlag::AbsoluteEpisode) {
                expanded.insert(MatchFlag::Episode);
            }
            if is_special
                && expanded.contains(MatchFlag::Title)
                && expanded.contains(MatchFlag::Series)
                && expanded.contains(MatchFlag::Year)
            {
                expanded.insert(MatchFlag::Season);
                expanded.insert(MatchFlag::Episode);
            }
        }
    }

    compute_score_bits(weights, expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_hash_equals_sum_of_non_hash_non_hi() {
        let w = SERIES_WEIGHTS.weights();
        let expected = w.title
            + w.year
            + w.season
            + w.episode
            + w.release_group
            + w.source
            + w.audio_codec
            + w.resolution
            + w.video_codec;
        assert_eq!(w.hash, expected);
    }

    #[test]
    fn movie_hash_equals_sum_of_non_hash_non_hi() {
        let w = MOVIE_WEIGHTS.weights();
        let expected = w.title
            + w.year
            + w.release_group
            + w.source
            + w.audio_codec
            + w.resolution
            + w.video_codec;
        assert_eq!(w.hash, expected);
    }

    #[test]
    fn full_hash_match_gives_max_minus_hi() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("hash".to_string(), true);
        let score = compute_score(&w, &matches);
        assert_eq!(score, w.hash);
        assert_eq!(score, w.max_score() - w.hearing_impaired);
    }

    #[test]
    fn empty_matches_gives_zero() {
        let w = MOVIE_WEIGHTS.weights();
        let matches = HashMap::new();
        assert_eq!(compute_score(&w, &matches), 0);
    }

    #[test]
    fn percent_thresholds_convert_to_bazarr_raw_scores() {
        assert_eq!(
            percent_to_raw_threshold(SubtitleScoreKind::Episode, 90),
            324
        );
        assert_eq!(percent_to_raw_threshold(SubtitleScoreKind::Movie, 70), 84);
    }

    #[test]
    fn percent_thresholds_clamp_to_valid_range() {
        assert_eq!(percent_to_raw_threshold(SubtitleScoreKind::Episode, -10), 0);
        assert_eq!(percent_to_raw_threshold(SubtitleScoreKind::Movie, 150), 120);
    }

    #[test]
    fn raw_scores_convert_to_percent_for_display() {
        let percent = raw_score_to_percent(SubtitleScoreKind::Episode, 242);
        assert!((percent - 67.222).abs() < 0.01);
    }

    // ── Partial match tests ─────────────────────────────────────────

    #[test]
    fn series_partial_match_title_year_season_episode() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), true);
        matches.insert("year".to_string(), true);
        matches.insert("season".to_string(), true);
        matches.insert("episode".to_string(), true);
        let score = compute_score(&w, &matches);
        assert_eq!(score, w.title + w.year + w.season + w.episode);
    }

    #[test]
    fn series_partial_match_title_and_release_group_only() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), true);
        matches.insert("release_group".to_string(), true);
        let score = compute_score(&w, &matches);
        assert_eq!(score, w.title + w.release_group);
    }

    #[test]
    fn movie_partial_match_title_year_source() {
        let w = MOVIE_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), true);
        matches.insert("year".to_string(), true);
        matches.insert("source".to_string(), true);
        let score = compute_score(&w, &matches);
        assert_eq!(score, w.title + w.year + w.source);
    }

    #[test]
    fn movie_partial_match_codecs_and_resolution() {
        let w = MOVIE_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("audio_codec".to_string(), true);
        matches.insert("video_codec".to_string(), true);
        matches.insert("resolution".to_string(), true);
        let score = compute_score(&w, &matches);
        assert_eq!(score, w.audio_codec + w.video_codec + w.resolution);
    }

    // ── Individual factor scoring tests ─────────────────────────────

    #[test]
    fn series_title_factor_adds_exactly_180() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 180);
    }

    #[test]
    fn series_year_factor_adds_exactly_90() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("year".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 90);
    }

    #[test]
    fn series_season_factor_adds_exactly_30() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("season".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 30);
    }

    #[test]
    fn series_episode_factor_adds_exactly_30() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("episode".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 30);
    }

    #[test]
    fn series_release_group_factor_adds_exactly_15() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("release_group".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 15);
    }

    #[test]
    fn series_source_factor_adds_exactly_7() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("source".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 7);
    }

    #[test]
    fn movie_title_factor_adds_exactly_60() {
        let w = MOVIE_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 60);
    }

    #[test]
    fn movie_year_factor_adds_exactly_30() {
        let w = MOVIE_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("year".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 30);
    }

    // ── Title vs series key tests ───────────────────────────────────

    #[test]
    fn series_key_maps_to_title_weight() {
        let w = SERIES_WEIGHTS.weights();
        let mut with_title = HashMap::new();
        with_title.insert("title".to_string(), true);
        let mut with_series = HashMap::new();
        with_series.insert("series".to_string(), true);
        assert_eq!(
            compute_score(&w, &with_title),
            compute_score(&w, &with_series),
        );
    }

    #[test]
    fn series_key_maps_to_title_weight_movie() {
        let w = MOVIE_WEIGHTS.weights();
        let mut with_title = HashMap::new();
        with_title.insert("title".to_string(), true);
        let mut with_series = HashMap::new();
        with_series.insert("series".to_string(), true);
        assert_eq!(
            compute_score(&w, &with_title),
            compute_score(&w, &with_series),
        );
    }

    #[test]
    fn title_and_series_both_true_counts_once() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), true);
        matches.insert("series".to_string(), true);
        // The `||` means it should still only add title weight once
        assert_eq!(compute_score(&w, &matches), w.title);
    }

    // ── All factors matching ────────────────────────────────────────

    #[test]
    fn series_all_non_hash_factors_equal_hash_weight() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), true);
        matches.insert("year".to_string(), true);
        matches.insert("season".to_string(), true);
        matches.insert("episode".to_string(), true);
        matches.insert("release_group".to_string(), true);
        matches.insert("source".to_string(), true);
        matches.insert("audio_codec".to_string(), true);
        matches.insert("resolution".to_string(), true);
        matches.insert("video_codec".to_string(), true);
        let score = compute_score(&w, &matches);
        assert_eq!(score, w.hash);
    }

    #[test]
    fn movie_all_non_hash_factors_equal_hash_weight() {
        let w = MOVIE_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), true);
        matches.insert("year".to_string(), true);
        matches.insert("release_group".to_string(), true);
        matches.insert("source".to_string(), true);
        matches.insert("audio_codec".to_string(), true);
        matches.insert("resolution".to_string(), true);
        matches.insert("video_codec".to_string(), true);
        let score = compute_score(&w, &matches);
        assert_eq!(score, w.hash);
    }

    #[test]
    fn series_all_factors_including_hash_and_hi_equals_max_score() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("hash".to_string(), true);
        matches.insert("title".to_string(), true);
        matches.insert("year".to_string(), true);
        matches.insert("season".to_string(), true);
        matches.insert("episode".to_string(), true);
        matches.insert("release_group".to_string(), true);
        matches.insert("source".to_string(), true);
        matches.insert("audio_codec".to_string(), true);
        matches.insert("resolution".to_string(), true);
        matches.insert("video_codec".to_string(), true);
        matches.insert("hearing_impaired".to_string(), true);
        let score = compute_score(&w, &matches);
        // hash == sum of non-hash non-HI, so all factors = hash + hash + HI
        assert_eq!(score, w.hash + w.hash + w.hearing_impaired);
    }

    // ── Hearing impaired ────────────────────────────────────────────

    #[test]
    fn hearing_impaired_adds_exactly_one_point_series() {
        let w = SERIES_WEIGHTS.weights();
        let mut without_hi = HashMap::new();
        without_hi.insert("title".to_string(), true);
        let mut with_hi = without_hi.clone();
        with_hi.insert("hearing_impaired".to_string(), true);
        assert_eq!(
            compute_score(&w, &with_hi) - compute_score(&w, &without_hi),
            1,
        );
    }

    #[test]
    fn hearing_impaired_adds_exactly_one_point_movie() {
        let w = MOVIE_WEIGHTS.weights();
        let mut without_hi = HashMap::new();
        without_hi.insert("title".to_string(), true);
        let mut with_hi = without_hi.clone();
        with_hi.insert("hearing_impaired".to_string(), true);
        assert_eq!(
            compute_score(&w, &with_hi) - compute_score(&w, &without_hi),
            1,
        );
    }

    #[test]
    fn hearing_impaired_alone_is_one() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("hearing_impaired".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 1);
    }

    // ── Max score ───────────────────────────────────────────────────

    #[test]
    fn series_max_score_is_hash_plus_hi() {
        let w = SERIES_WEIGHTS.weights();
        assert_eq!(w.max_score(), w.hash + w.hearing_impaired);
        assert_eq!(w.max_score(), 360);
    }

    #[test]
    fn movie_max_score_is_hash_plus_hi() {
        let w = MOVIE_WEIGHTS.weights();
        assert_eq!(w.max_score(), w.hash + w.hearing_impaired);
        assert_eq!(w.max_score(), 120);
    }

    // ── False factors don't contribute ──────────────────────────────

    #[test]
    fn false_factors_contribute_nothing() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("title".to_string(), false);
        matches.insert("year".to_string(), false);
        matches.insert("hash".to_string(), false);
        assert_eq!(compute_score(&w, &matches), 0);
    }

    #[test]
    fn movie_season_episode_are_zero() {
        let w = MOVIE_WEIGHTS.weights();
        assert_eq!(w.season, 0);
        assert_eq!(w.episode, 0);
        let mut matches = HashMap::new();
        matches.insert("season".to_string(), true);
        matches.insert("episode".to_string(), true);
        assert_eq!(compute_score(&w, &matches), 0);
    }

    #[test]
    fn verified_movie_hash_requires_source_and_video_codec() {
        let w = MOVIE_WEIGHTS.weights();
        let mut matches = HashSet::new();
        matches.insert("hash".to_string());
        matches.insert("title".to_string());
        assert_eq!(
            compute_verified_score(&w, SubtitleScoreKind::Movie, &matches, false),
            w.title
        );

        matches.insert("source".to_string());
        matches.insert("video_codec".to_string());
        assert_eq!(
            compute_verified_score(&w, SubtitleScoreKind::Movie, &matches, false),
            w.hash
        );
    }

    #[test]
    fn verified_episode_identifier_adds_equivalent_matches() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashSet::new();
        matches.insert("series_imdb_id".to_string());
        matches.insert("season".to_string());
        matches.insert("episode".to_string());
        matches.insert("source".to_string());

        assert_eq!(
            compute_verified_score(&w, SubtitleScoreKind::Episode, &matches, false),
            w.title + w.year + w.season + w.episode + w.source
        );
    }

    #[test]
    fn verified_score_does_not_double_count_title_and_series() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashSet::new();
        matches.insert("title".to_string());
        matches.insert("series".to_string());
        matches.insert("year".to_string());

        assert_eq!(
            compute_verified_score(&w, SubtitleScoreKind::Episode, &matches, false),
            w.title + w.year + w.episode
        );
    }

    #[test]
    fn compute_score_bits_matches_hashmap_wrapper() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashMap::new();
        matches.insert("series".to_string(), true);
        matches.insert("year".to_string(), true);
        matches.insert("season".to_string(), true);
        matches.insert("episode".to_string(), true);
        matches.insert("source".to_string(), true);
        matches.insert("video_codec".to_string(), true);

        let mut bits = MatchBits::default();
        bits.insert(MatchFlag::Series);
        bits.insert(MatchFlag::Year);
        bits.insert(MatchFlag::Season);
        bits.insert(MatchFlag::Episode);
        bits.insert(MatchFlag::Source);
        bits.insert(MatchFlag::VideoCodec);

        assert_eq!(compute_score(&w, &matches), compute_score_bits(&w, bits));
    }

    #[test]
    fn verified_score_bits_matches_hashset_wrapper() {
        let w = SERIES_WEIGHTS.weights();
        let mut matches = HashSet::new();
        matches.insert("series_imdb_id".to_string());
        matches.insert("source".to_string());
        matches.insert("resolution".to_string());
        matches.insert("audio_codec".to_string());

        let mut bits = MatchBits::default();
        bits.insert(MatchFlag::SeriesImdbId);
        bits.insert(MatchFlag::Source);
        bits.insert(MatchFlag::Resolution);
        bits.insert(MatchFlag::AudioCodec);

        assert_eq!(
            compute_verified_score(&w, SubtitleScoreKind::Episode, &matches, false),
            compute_verified_score_bits(&w, SubtitleScoreKind::Episode, bits, false)
        );
    }
}
