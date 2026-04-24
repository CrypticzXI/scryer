use chrono::NaiveDate;
use serde::Serialize;
use smallvec::SmallVec;

use crate::lex::{ReleaseCst, TextSpan, Token};

/// Token range within the lossless token stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TokenRange {
    pub start_token: usize,
    pub end_token: usize,
}

/// Family selected by the beam parser for a candidate release interpretation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseFamily {
    Movie,
    StandardEpisode,
    DailyEpisode,
    AnimeAbsolute,
    SeasonPack,
    EpisodeRangePack,
    Special,
    #[default]
    Unknown,
}

/// Special episode kinds recognized by the parser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParsedSpecialKindV2 {
    #[default]
    Special,
    Ova,
    Oad,
    Ncop,
    Nced,
    Extra,
}

impl ParsedSpecialKindV2 {
    pub const OVA: Self = Self::Ova;
    pub const OAD: Self = Self::Oad;
    pub const OVD: Self = Self::Oad;
    pub const NCOP: Self = Self::Ncop;
    pub const NCED: Self = Self::Nced;
}

/// Episodic release type recognized by the parser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParsedEpisodeReleaseTypeV2 {
    SingleEpisode,
    MultiEpisode,
    SeasonPack,
    RangePack,
    Daily,
    #[default]
    Unknown,
}

/// Structured episodic metadata projected from the winning parse candidate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ParsedEpisodeMetadataV2 {
    pub season: Option<u32>,
    pub episode_numbers: Vec<u32>,
    pub absolute_episode: Option<u32>,
    pub absolute_episode_numbers: Vec<u32>,
    pub special_absolute_episode_numbers: Vec<u32>,
    pub air_date: Option<NaiveDate>,
    pub daily_part: Option<u32>,
    pub full_season: bool,
    pub is_partial_season: bool,
    pub is_multi_season: bool,
    pub season_part: Option<u32>,
    pub is_season_extra: bool,
    pub is_split_episode: bool,
    pub is_mini_series: bool,
    pub special_kind: Option<ParsedSpecialKindV2>,
    pub release_type: ParsedEpisodeReleaseTypeV2,
    pub raw: Option<String>,
}

impl ParsedEpisodeMetadataV2 {
    #[must_use]
    pub fn first_episode(&self) -> Option<u32> {
        self.episode_numbers
            .first()
            .copied()
            .or_else(|| self.absolute_episode_numbers.first().copied())
            .or_else(|| self.special_absolute_episode_numbers.first().copied())
    }
}

/// Parsed external id projected from raw metadata tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ParsedExternalId {
    pub source: String,
    pub value: String,
}

/// Overall disposition of a parse attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseDisposition {
    #[default]
    Parsed,
    Ambiguous,
    Unparseable,
}

/// Structured release parse returned by the v2 parser.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParsedReleaseMetadataV2 {
    pub raw_title: String,
    pub normalized_title: String,
    pub normalized_title_variants: Vec<String>,
    pub release_group: Option<String>,
    pub languages_audio: Vec<String>,
    pub languages_subtitles: Vec<String>,
    pub external_ids: Vec<ParsedExternalId>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub year: Option<i32>,
    pub quality: Option<String>,
    pub source: Option<String>,
    pub video_codec: Option<String>,
    pub video_encoding: Option<String>,
    pub audio: Option<String>,
    pub audio_codecs: Vec<String>,
    pub audio_channels: Option<String>,
    pub is_dual_audio: bool,
    pub is_atmos: bool,
    pub is_dolby_vision: bool,
    pub detected_hdr: bool,
    pub has_hdr_fallback: bool,
    pub is_hdr10plus: bool,
    pub is_hlg: bool,
    pub is_10bit: bool,
    pub fps: Option<f32>,
    pub is_proper_upload: bool,
    pub is_repack: bool,
    pub is_remux: bool,
    pub is_bd_disk: bool,
    pub is_ai_enhanced: bool,
    pub is_hardcoded_subs: bool,
    pub is_uncensored: bool,
    pub is_dubs_only: bool,
    pub streaming_service: Option<String>,
    pub edition: Option<String>,
    pub anime_version: Option<u32>,
    pub episode: Option<ParsedEpisodeMetadataV2>,
    pub parser_version: &'static str,
    pub scoring_model_version: u16,
    pub parse_confidence: f32,
    pub ambiguity_margin: i32,
    pub is_ambiguous: bool,
    pub disposition: ParseDisposition,
    pub parse_family: ParseFamily,
    pub missing_fields: Vec<String>,
    pub parse_hints: Vec<String>,
}

impl ParsedReleaseMetadataV2 {
    /// Build an empty parse projection for irrecoverable parse failures.
    #[must_use]
    pub fn empty(raw: &str, parser_version: &'static str) -> Self {
        Self {
            raw_title: raw.to_string(),
            normalized_title: String::new(),
            normalized_title_variants: Vec::new(),
            release_group: None,
            languages_audio: Vec::new(),
            languages_subtitles: Vec::new(),
            external_ids: Vec::new(),
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            year: None,
            quality: None,
            source: None,
            video_codec: None,
            video_encoding: None,
            audio: None,
            audio_codecs: Vec::new(),
            audio_channels: None,
            is_dual_audio: false,
            is_atmos: false,
            is_dolby_vision: false,
            detected_hdr: false,
            has_hdr_fallback: false,
            is_hdr10plus: false,
            is_hlg: false,
            is_10bit: false,
            fps: None,
            is_proper_upload: false,
            is_repack: false,
            is_remux: false,
            is_bd_disk: false,
            is_ai_enhanced: false,
            is_hardcoded_subs: false,
            is_uncensored: false,
            is_dubs_only: false,
            streaming_service: None,
            edition: None,
            anime_version: None,
            episode: None,
            parser_version,
            scoring_model_version: 0,
            parse_confidence: 0.0,
            ambiguity_margin: 0,
            is_ambiguous: true,
            disposition: ParseDisposition::Unparseable,
            parse_family: ParseFamily::Unknown,
            missing_fields: Vec::new(),
            parse_hints: vec!["no_candidate".to_string()],
        }
    }
}

impl Default for ParsedReleaseMetadataV2 {
    fn default() -> Self {
        Self::empty("", "unknown")
    }
}

/// Role assigned to a token by the bounded annotator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenRole {
    Year,
    Quality,
    Source,
    StreamingService,
    VideoCodec,
    AudioCodec,
    AudioChannels,
    Language,
    Edition,
    ReleaseFlag,
    EpisodeMarker,
    SeasonMarker,
    AbsoluteEpisodeMarker,
    DateMarker,
    PackMarker,
    SpecialMarker,
    VersionMarker,
    ExternalId,
    ReleaseGroupCandidate,
    ChecksumOrHash,
    Noise,
    #[default]
    TitleWord,
}

/// Bounded role annotation for a token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TokenAnnotations {
    pub primary_role: TokenRole,
    pub alternate_roles: SmallVec<[TokenRole; 2]>,
    pub may_be_title_word: bool,
    pub role_confidence: u8,
    pub role_pruned: bool,
}

/// Parse reason emitted by the scorer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ParseReason {
    pub code: String,
    pub delta: i32,
    pub detail: Option<String>,
}

/// Title segment emitted by a parse candidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleSegmentKind {
    #[default]
    ObservedPrimary,
    ObservedAlternate,
    ContextMatchedAlias,
    Connector,
}

/// Title-bearing span selected by the parser.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TitleSegment {
    pub kind: TitleSegmentKind,
    pub token_start: usize,
    pub token_end: usize,
    pub raw: String,
    pub normalized: String,
}

/// Metadata AST collected before projection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MetadataAst {
    pub year: Option<i32>,
    pub quality: Option<String>,
    pub source: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<String>,
    pub streaming_service: Option<String>,
    pub edition: Option<String>,
    pub external_ids: Vec<ParsedExternalId>,
    pub token_indices: Vec<usize>,
    pub year_span: Option<TokenRange>,
    pub quality_span: Option<TokenRange>,
    pub source_span: Option<TokenRange>,
    pub video_codec_span: Option<TokenRange>,
    pub audio_codec_span: Option<TokenRange>,
    pub audio_channels_span: Option<TokenRange>,
    pub streaming_service_span: Option<TokenRange>,
    pub edition_span: Option<TokenRange>,
    pub external_id_spans: Vec<TokenRange>,
}

/// Semantic identity extracted from a release candidate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReleaseIdentity {
    MovieIdentity,
    StandardEpisodeIdentity {
        season: Option<u32>,
        episode_numbers: Vec<u32>,
    },
    DailyIdentity {
        air_date: NaiveDate,
        part: Option<u32>,
    },
    AbsoluteIdentity {
        absolute_episode_numbers: Vec<u32>,
        version: Option<u32>,
        season_hint: Option<u32>,
    },
    SeasonPackIdentity {
        seasons: Vec<u32>,
        is_partial: bool,
        season_part: Option<u32>,
    },
    RangePackIdentity {
        season: Option<u32>,
        range_start: u32,
        range_end: u32,
    },
    SpecialIdentity {
        special_kind: ParsedSpecialKindV2,
        season_hint: Option<u32>,
        episode_hint: Option<u32>,
    },
    #[default]
    Unknown,
}

/// Candidate parse emitted by the beam search.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReleaseParseCandidate {
    pub family: ParseFamily,
    pub title_segments: Vec<TitleSegment>,
    pub identity: ReleaseIdentity,
    pub metadata: MetadataAst,
    pub zones: CandidateZones,
    pub release_group: Option<String>,
    pub unconsumed_tokens: Vec<TextSpan>,
    pub reasons: Vec<ParseReason>,
    pub raw_evidence: Vec<String>,
    pub context_evidence: Vec<String>,
    pub raw_score: i32,
    pub enrichment: Option<MetadataEnrichment>,
    pub projected: ParsedReleaseMetadataV2,
}

/// Explicit token zones handed off from the beam to metadata enrichment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CandidateZones {
    pub title_zones: Vec<TokenRange>,
    pub metadata_zone: Option<TokenRange>,
    pub trailing_zone: Option<TokenRange>,
    pub source_span: Option<TokenRange>,
    pub service_span: Option<TokenRange>,
    pub video_span: Option<TokenRange>,
    pub audio_span: Option<TokenRange>,
    pub language_span: Option<TokenRange>,
    pub edition_span: Option<TokenRange>,
    pub release_group_span: Option<TokenRange>,
}

/// Local metadata classification emitted by the deterministic enrichment pass.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MetadataEnrichment {
    pub languages_audio: Vec<String>,
    pub languages_subtitles: Vec<String>,
    pub external_ids: Vec<ParsedExternalId>,
    pub tmdb_id: Option<String>,
    pub video_codec: Option<String>,
    pub video_encoding: Option<String>,
    pub audio: Option<String>,
    pub audio_codecs: Vec<String>,
    pub audio_channels: Option<String>,
    pub is_dual_audio: bool,
    pub is_atmos: bool,
    pub is_dolby_vision: bool,
    pub detected_hdr: bool,
    pub has_hdr_fallback: bool,
    pub is_hdr10plus: bool,
    pub is_hlg: bool,
    pub is_10bit: bool,
    pub fps: Option<f32>,
    pub is_proper_upload: bool,
    pub is_repack: bool,
    pub is_bd_disk: bool,
    pub is_ai_enhanced: bool,
    pub is_hardcoded_subs: bool,
    pub is_uncensored: bool,
    pub is_dubs_only: bool,
    pub edition: Option<String>,
    pub anime_version: Option<u32>,
    pub normalized_source: Option<String>,
    pub parse_hints: Vec<String>,
}

/// Target-aware analysis result for one release string.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReleaseParseAnalysis {
    pub raw_input: String,
    pub sanitized_input: String,
    pub parse_hints: Vec<String>,
    pub tokens: Vec<Token>,
    pub annotations: Vec<TokenAnnotations>,
    pub cst: ReleaseCst,
    pub candidates: Vec<ReleaseParseCandidate>,
    pub best_candidate_index: Option<usize>,
    pub parser_version: &'static str,
    pub scoring_model_version: u16,
    pub ambiguity_margin: i32,
    pub is_ambiguous: bool,
    pub disposition: ParseDisposition,
}

impl ReleaseParseAnalysis {
    /// Return the highest-scoring parse candidate, if one exists.
    #[must_use]
    pub fn best_candidate(&self) -> Option<&ReleaseParseCandidate> {
        self.best_candidate_index
            .and_then(|index| self.candidates.get(index))
    }

    /// Return whether the parser found no viable candidate.
    #[must_use]
    pub fn is_unparseable(&self) -> bool {
        matches!(self.disposition, ParseDisposition::Unparseable)
    }
}

/// One target-specific analysis result with a stable target index.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TargetScoredAnalysis {
    pub target_index: usize,
    pub analysis: ReleaseParseAnalysis,
    pub best_score: i32,
}

/// Multi-target analysis result for one raw release string.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TargetedReleaseParseAnalysis {
    pub targets: Vec<TargetScoredAnalysis>,
    pub best_target_index: Option<usize>,
}

impl TargetedReleaseParseAnalysis {
    /// Return the highest-scoring target analysis, if one exists.
    #[must_use]
    pub fn best_target(&self) -> Option<&TargetScoredAnalysis> {
        let best_index = self.best_target_index?;
        self.targets
            .iter()
            .find(|target| target.target_index == best_index)
    }

    /// Return the score margin between the best and second-best target contexts.
    #[must_use]
    pub fn ambiguity_margin(&self) -> i32 {
        let Some(best_target) = self.best_target() else {
            return 0;
        };
        let second_best = self
            .targets
            .iter()
            .filter(|target| target.target_index != best_target.target_index)
            .filter(|target| !target.analysis.is_unparseable())
            .map(|target| target.best_score)
            .max();
        second_best.map_or(i32::MAX, |score| {
            best_target.best_score.saturating_sub(score)
        })
    }

    /// Return whether the best target choice is ambiguous.
    #[must_use]
    pub fn is_ambiguous(&self) -> bool {
        let Some(best_target) = self.best_target() else {
            return true;
        };
        if best_target.analysis.is_unparseable() || best_target.analysis.is_ambiguous {
            return true;
        }
        let parsed_target_count = self
            .targets
            .iter()
            .filter(|target| !target.analysis.is_unparseable())
            .count();
        parsed_target_count > 1 && self.ambiguity_margin() < 10
    }
}
