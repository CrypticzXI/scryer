use super::acquisition::{
    collection_download_submission_scope_for_wanted_item,
    direct_download_submission_scope_for_wanted_item,
};
use super::*;
use crate::acquisition_policy::evaluate_upgrade;
use crate::acquisition_search_queries::{
    anidb_id_from_external_ids, build_movie_search_queries, build_search_queries,
    imdb_id_from_title, mal_id_from_external_ids, movie_text_search_query,
    tmdb_id_from_external_ids, tvdb_id_from_external_ids,
};
use crate::delay_profile::DelayProfile;
use crate::quality::release_parser::ParseDisposition;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

/// Library-local identity ambiguity for a canonical title (Pillar A tier 0):
/// the subject's canonical lookup keys that at least one *other* library title
/// also claims. Empty means the title is unambiguous and a bare release name
/// stays sufficient evidence. Derived from Scryer's own catalog only — no
/// catalog/SMG knowledge is involved.
#[derive(Clone, Debug, Default)]
pub(crate) struct TitleIdentityAmbiguity {
    pub(crate) shared_lookup_keys: Vec<String>,
}

impl TitleIdentityAmbiguity {
    pub(crate) fn from_shared_keys(shared_lookup_keys: Vec<String>) -> Self {
        Self { shared_lookup_keys }
    }

    /// True when an auto candidate must present a positive disambiguator (A2).
    pub(crate) fn requires_disambiguator(&self) -> bool {
        !self.shared_lookup_keys.is_empty()
    }

    /// True when `key` is an alias only this title claims within the library
    /// collision set — the A2(3) "unique alias hit" disambiguator.
    pub(crate) fn key_is_unique_to_title(&self, key: &str) -> bool {
        !self
            .shared_lookup_keys
            .iter()
            .any(|shared| shared.as_str() == key)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CanonicalTitleEvidence {
    pub(crate) lookup_keys: Vec<String>,
    pub(crate) canonical_key: String,
    pub(crate) year: Option<i32>,
    pub(crate) parse_context: crate::ReleaseParseContext,
    /// Library-local collision data. Defaults to "not ambiguous" so every
    /// existing construction site keeps its behavior; the resolution paths
    /// attach real data through [`CanonicalTitleEvidence::with_ambiguity`].
    pub(crate) ambiguity: TitleIdentityAmbiguity,
}

impl CanonicalTitleEvidence {
    pub(crate) fn with_ambiguity(mut self, ambiguity: TitleIdentityAmbiguity) -> Self {
        self.ambiguity = ambiguity;
        self
    }
}

/// How a parsed release name matched a canonical title, retained so the Pillar A
/// disambiguator check can tell a shared bare key from a unique alias.
#[derive(Clone, Debug)]
pub(crate) struct TitleEvidenceMatch {
    /// The canonical lookup key that actually matched.
    pub(crate) matched_key: String,
    /// The release carries the title's year (A2(1)).
    pub(crate) year_corroborated: bool,
    /// A one-word alias is too weak to establish identity without an external
    /// id (or the title year, represented by `year_corroborated`).
    pub(crate) requires_external_id: bool,
}

/// A candidate's title match proven from the raw release title.
#[derive(Clone, Debug)]
pub(crate) struct CandidateTitleMatch {
    pub(crate) evidence_match: Option<TitleEvidenceMatch>,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedReleaseSearchSubject {
    pub(crate) title_id: String,
    pub(crate) title_tags: Vec<String>,
    pub(crate) title_evidence: CanonicalTitleEvidence,
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
    pub(crate) mal_id: Option<String>,
    pub(crate) category: String,
    pub(crate) owner_facet: MediaFacet,
    pub(crate) search_facet: MediaFacet,
    pub(crate) id_search_facet: Option<MediaFacet>,
    pub(crate) newznab_categories: Vec<String>,
    pub(crate) runtime_minutes: Option<i32>,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) absolute_episode: Option<u32>,
    pub(crate) subject_kind: ReleaseSearchSubjectKind,
    pub(crate) current_score: Option<i32>,
    pub(crate) last_search_at: Option<String>,
    pub(crate) grabbed_release: Option<String>,
    pub(crate) submission_scope: SubmissionScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseAutoDecisionCode {
    Eligible,
    ParseAmbiguous,
    ParseUnparseable,
    TitleMismatch,
    EpisodeMismatch,
    CategoryMismatch,
    AmbiguousIdentity,
    QualityBlocked,
    NegativeScore,
    UpgradeRejected,
    CutoffReached,
    AlreadyActive,
    DbBlocklisted,
    PendingDelay,
    DownloadClientUnavailable,
    RepackGroupMismatch,
}

impl ReleaseAutoDecisionCode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "eligible" => Some(Self::Eligible),
            "parse_ambiguous" => Some(Self::ParseAmbiguous),
            "parse_unparseable" => Some(Self::ParseUnparseable),
            "title_mismatch" => Some(Self::TitleMismatch),
            "episode_mismatch" => Some(Self::EpisodeMismatch),
            // Deliberately the same string the D1 pre-submission gate records on
            // its Failed attempts, so both category vetoes read alike.
            "category_mismatch" => Some(Self::CategoryMismatch),
            "ambiguous_identity" => Some(Self::AmbiguousIdentity),
            "quality_blocked" => Some(Self::QualityBlocked),
            "negative_score" => Some(Self::NegativeScore),
            "upgrade_rejected" => Some(Self::UpgradeRejected),
            "cutoff_reached" => Some(Self::CutoffReached),
            "already_active" => Some(Self::AlreadyActive),
            "db_blocklisted" => Some(Self::DbBlocklisted),
            "pending_delay" => Some(Self::PendingDelay),
            "download_client_unavailable" => Some(Self::DownloadClientUnavailable),
            "repack_group_mismatch" => Some(Self::RepackGroupMismatch),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::ParseAmbiguous => "parse_ambiguous",
            Self::ParseUnparseable => "parse_unparseable",
            Self::TitleMismatch => "title_mismatch",
            Self::EpisodeMismatch => "episode_mismatch",
            Self::CategoryMismatch => "category_mismatch",
            Self::AmbiguousIdentity => "ambiguous_identity",
            Self::QualityBlocked => "quality_blocked",
            Self::NegativeScore => "negative_score",
            Self::UpgradeRejected => "upgrade_rejected",
            Self::CutoffReached => "cutoff_reached",
            Self::AlreadyActive => "already_active",
            Self::DbBlocklisted => "db_blocklisted",
            Self::PendingDelay => "pending_delay",
            Self::DownloadClientUnavailable => "download_client_unavailable",
            Self::RepackGroupMismatch => "repack_group_mismatch",
        }
    }

    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Eligible => "auto search would grab this release",
            Self::ParseAmbiguous => "release parse is ambiguous and blocks auto-grab",
            Self::ParseUnparseable => "release could not be parsed and blocks auto-grab",
            Self::TitleMismatch => "release title does not match the target title",
            Self::EpisodeMismatch => "release numbering does not match the target episode",
            Self::CategoryMismatch => "indexer category contradicts the target title",
            Self::AmbiguousIdentity => {
                "canonical title is ambiguous and no disambiguator was present"
            }
            Self::QualityBlocked => "quality profile blocked this release",
            Self::NegativeScore => "release score is negative after scoring penalties",
            Self::UpgradeRejected => "upgrade policy rejected this release",
            Self::CutoffReached => "existing file already meets the configured cutoff",
            Self::AlreadyActive => "release is already active or covered in the queue",
            Self::DbBlocklisted => "release is blocklisted from prior failures",
            Self::PendingDelay => "release is eligible but held by a delay profile",
            Self::DownloadClientUnavailable => "matching download clients are unavailable",
            Self::RepackGroupMismatch => "repack group does not match the existing file",
        }
    }

    pub(crate) fn is_eligible(self) -> bool {
        matches!(self, Self::Eligible)
    }
}

#[derive(Clone)]
pub(crate) struct AutoCandidateEvaluationContext<'a> {
    pub(crate) title: &'a Title,
    pub(crate) subject: &'a ResolvedReleaseSearchSubject,
    pub(crate) current_score: Option<i32>,
    pub(crate) last_search_at: Option<&'a str>,
    pub(crate) profile: &'a QualityProfile,
    pub(crate) thresholds: &'a AcquisitionThresholds,
    pub(crate) cutoff_reached: bool,
    pub(crate) now: &'a DateTime<Utc>,
    pub(crate) dl_snapshot: Option<&'a crate::acquisition_workflow::DownloadClientSnapshot>,
    pub(crate) db_blocklist: &'a HashSet<String>,
    pub(crate) existing_files: &'a [TitleMediaFile],
    pub(crate) delay_profiles: &'a [DelayProfile],
    pub(crate) failed_source_kinds: Option<&'a [DownloadSourceKind]>,
}

pub fn release_strategy_kind_for_label(label: &str, is_rss_request: bool) -> ReleaseStrategyKind {
    if is_rss_request {
        return ReleaseStrategyKind::RssFeed;
    }

    if label.starts_with("ids") {
        return ReleaseStrategyKind::IdBacked;
    }

    match label {
        "freetext" | "freetext_alias" => ReleaseStrategyKind::Freetext,
        _ => ReleaseStrategyKind::Fallback,
    }
}

pub(crate) fn canonical_title_lookup_keys(title: &Title) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();

    for candidate in std::iter::once(title.name.as_str())
        .chain(title.aliases.iter().map(String::as_str))
        .chain(title.tagged_aliases.iter().map(|alias| alias.name.as_str()))
    {
        let normalized = crate::title_matching::canonical_lookup_key(candidate);
        if !normalized.is_empty() && seen.insert(normalized.clone()) {
            keys.push(normalized);
        }
    }

    keys
}

pub(crate) fn canonical_title_evidence(title: &Title) -> CanonicalTitleEvidence {
    canonical_title_evidence_for_episode(title, None)
}

fn canonical_title_evidence_for_episode(
    title: &Title,
    episode: Option<&Episode>,
) -> CanonicalTitleEvidence {
    let lookup_keys = canonical_title_lookup_keys(title);
    let canonical_key = crate::title_matching::canonical_lookup_key(&title.name);
    let mut parse_context =
        crate::build_release_parse_context(title, episode, None, Some(title.facet.as_str()));
    if title.year.is_some() {
        let stripped_key = crate::import_title_resolution::strip_trailing_year_key(&canonical_key);
        if stripped_key != canonical_key
            && !stripped_key.is_empty()
            && !parse_context.aliases.iter().any(|alias| {
                crate::title_matching::canonical_lookup_key(&alias.name) == stripped_key
            })
        {
            parse_context
                .aliases
                .push(crate::release_parser::ContextAlias {
                    name: stripped_key.to_string(),
                });
        }
    }

    CanonicalTitleEvidence {
        lookup_keys,
        canonical_key,
        year: title.year,
        parse_context,
        ambiguity: TitleIdentityAmbiguity::default(),
    }
}

pub(crate) fn series_movie_search_title(
    title: &Title,
    link: &scryer_domain::SeriesMovieLink,
) -> Title {
    let movie = &link.movie;
    let mut search_title = title.clone();
    search_title.name = movie.title.clone();
    search_title.facet = MediaFacet::Movie;
    search_title.year = movie.year;
    search_title.imdb_id = movie.imdb_id.clone();
    search_title.runtime_minutes = movie.runtime_minutes;
    search_title.external_ids.retain(|external_id| {
        !matches!(
            external_id.source.trim().to_ascii_lowercase().as_str(),
            "imdb" | "tvdb" | "tmdb" | "anidb" | "mal"
        )
    });
    if let Some(imdb_id) = movie
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "imdb".to_string(),
            value: imdb_id,
        });
    }
    if let Some(tvdb_id) = movie
        .tvdb_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.clone(),
        });
    }
    if let Some(tmdb_id) = movie
        .tmdb_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "tmdb".to_string(),
            value: tmdb_id.clone(),
        });
    }
    if let Some(anidb_id) = movie
        .anidb_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "anidb".to_string(),
            value: anidb_id.clone(),
        });
    }
    if let Some(mal_id) = movie
        .mal_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "mal".to_string(),
            value: mal_id.clone(),
        });
    }
    search_title.aliases = series_movie_search_aliases(&search_title);
    search_title.tagged_aliases = search_title
        .aliases
        .iter()
        .map(|alias| scryer_domain::TaggedAlias {
            name: alias.clone(),
            language: "und".to_string(),
        })
        .collect();
    search_title
}

fn series_movie_search_aliases(search_title: &Title) -> Vec<String> {
    let mut aliases = Vec::new();
    let mut seen = HashSet::new();
    let primary_key = crate::title_matching::canonical_lookup_key(&search_title.name);
    push_series_movie_alias_evidence(&mut aliases, &mut seen, search_title.name.clone());

    for candidate in crate::title_matching::search_variants(&search_title.name) {
        push_series_movie_alias(&mut aliases, &mut seen, &primary_key, candidate);
    }
    let reduced = crate::title_matching::reduced_comparison_key(
        &search_title.name,
        crate::title_matching::TitleMatchProfile::Movie,
    );
    push_series_movie_alias(&mut aliases, &mut seen, &primary_key, reduced);

    let parsed = crate::parse_release_metadata_for_target(
        &search_title.name,
        &crate::build_release_parse_context(search_title, None, None, Some("movie")),
    );
    let mut variants = parsed.normalized_title_variants.clone();
    if !variants
        .iter()
        .any(|title| title.eq_ignore_ascii_case(&parsed.normalized_title))
    {
        variants.push(parsed.normalized_title);
    }
    for variant in variants {
        push_series_movie_alias(&mut aliases, &mut seen, &primary_key, variant);
    }

    aliases
}

fn push_series_movie_alias(
    aliases: &mut Vec<String>,
    seen: &mut HashSet<String>,
    primary_key: &str,
    alias: String,
) {
    let alias = alias.split_whitespace().collect::<Vec<_>>().join(" ");
    if alias.is_empty() {
        return;
    }
    let key = crate::title_matching::canonical_lookup_key(&alias);
    if !key.is_empty() && key != primary_key && seen.insert(key) {
        aliases.push(alias);
    }
}

fn push_series_movie_alias_evidence(
    aliases: &mut Vec<String>,
    seen: &mut HashSet<String>,
    alias: String,
) {
    let alias = alias.split_whitespace().collect::<Vec<_>>().join(" ");
    if alias.is_empty() {
        return;
    }
    let key = crate::title_matching::canonical_lookup_key(&alias);
    if !key.is_empty() && seen.insert(key) {
        aliases.push(alias);
    }
}

fn media_facet_from_str(value: &str) -> Option<MediaFacet> {
    match value.trim().to_ascii_lowercase().as_str() {
        "movie" => Some(MediaFacet::Movie),
        "series" => Some(MediaFacet::Series),
        "anime" => Some(MediaFacet::Anime),
        _ => None,
    }
}

fn owner_facet_for_wanted_item(title: &Title, item: &AcquisitionScopeState) -> MediaFacet {
    item.title_facet
        .as_deref()
        .and_then(media_facet_from_str)
        .unwrap_or_else(|| title.facet.clone())
}

fn series_movie_newznab_categories(owner_facet: &MediaFacet) -> Vec<String> {
    let mut categories = vec!["2000".to_string()];
    if matches!(owner_facet, MediaFacet::Anime) {
        categories.push("5070".to_string());
    }
    categories
}

pub(crate) fn parsed_release_matches_title_evidence(
    parsed: &ParsedReleaseMetadata,
    evidence: &CanonicalTitleEvidence,
) -> bool {
    match_parsed_release_to_title_evidence(parsed, evidence).is_some()
}

fn push_anchor_key(keys: &mut Vec<String>, key: &str) {
    let key = key.trim();
    if !key.is_empty() && !keys.iter().any(|existing| existing == key) {
        keys.push(key.to_string());
    }
}

/// Pass 1 of the identity proof: canonical keys the *context-free* parse
/// extracts from a release name, before any target bias is applied.
///
/// A target-biased parse can project the target title out of a longer raw
/// span (`Electric Bloom` projecting the `BLOOM` alias), so bias may only
/// refine an identity that unbiased extraction already supports. Besides the
/// neutral title and its variants, two principled near-miss forms anchor too:
/// a leading known release-group run stripped (`Erai-raws.Title...`), and the
/// halves of an `AKA` dual-titled name.
pub(crate) fn context_free_identity_anchor_keys(raw_title: &str) -> Vec<String> {
    let neutral = crate::parse_release_metadata(raw_title);
    let mut extracted = neutral.normalized_title_variants.clone();
    extracted.push(neutral.normalized_title.clone());

    // Year tokens in the raw name re-attach to the extraction: a boundary
    // heuristic reads `Blade.Runner.2049.2160p` as title `Blade Runner`, but
    // the subject's key is `blade runner 2049`.
    let mut year_tokens = Vec::<String>::new();
    for digits in raw_title.split(|ch: char| !ch.is_ascii_digit()) {
        if digits.len() == 4
            && digits
                .parse::<i32>()
                .is_ok_and(|year| (1900..=2099).contains(&year))
            && !year_tokens.iter().any(|existing| existing == digits)
        {
            year_tokens.push(digits.to_string());
        }
    }

    let mut keys = Vec::<String>::new();
    for title in extracted {
        let key = crate::title_matching::canonical_lookup_key(&title);
        if key.is_empty() {
            continue;
        }
        push_anchor_key(&mut keys, &key);
        for year in &year_tokens {
            if !key.ends_with(year.as_str()) {
                push_anchor_key(&mut keys, &format!("{key} {year}"));
            }
        }

        // `Title AKA Other Title` names both subjects; each half anchors.
        if key.contains(" aka ") {
            for half in key.split(" aka ") {
                push_anchor_key(&mut keys, half);
            }
        }

        // An unbracketed leading group tag reads as title text to a neutral
        // parse. Only a run the release-group database recognizes may be
        // elided — an unknown prefix stays, so containment junk cannot anchor.
        let tokens = key.split_whitespace().collect::<Vec<_>>();
        for dropped in 1..=tokens.len().saturating_sub(1).min(3) {
            let prefix = &tokens[..dropped];
            if [prefix.join("-"), prefix.join(" ")]
                .iter()
                .any(|candidate| crate::release_group_db::is_known_release_group(candidate))
            {
                push_anchor_key(&mut keys, &tokens[dropped..].join(" "));
            }
        }
    }

    keys
}

/// The one key-comparison rule shared by the anchor gate and the contextual
/// confirm loop: a normalized string names the title when it equals a lookup
/// key outright, or equals a key with the title's own year elided.
fn evidence_key_for_normalized(
    evidence: &CanonicalTitleEvidence,
    normalized: &str,
) -> Option<String> {
    evidence
        .lookup_keys
        .iter()
        .find(|key| {
            key.as_str() == normalized
                || evidence
                    .year
                    .is_some_and(|year| key.strip_suffix(&format!(" {year}")) == Some(normalized))
        })
        .cloned()
}

/// Stacked-alias anchor: fansub names often glue two alias forms of the same
/// subject together (`Sora.no.Vale.Silver.Horizon.Beyond.the.Vale.-.01`), so a
/// neutral parse extracts one long title no single key equals. The extraction
/// still anchors when it decomposes *completely* into two or three distinct
/// lookup keys of this title — full coverage, so containment junk (extra words
/// that are no key of the subject) can never satisfy it.
fn extraction_decomposes_into_evidence_keys(
    evidence: &CanonicalTitleEvidence,
    extracted_key: &str,
) -> bool {
    const MAX_STACKED_SEGMENTS: usize = 3;

    fn covers(
        evidence: &CanonicalTitleEvidence,
        tokens: &[&str],
        start: usize,
        used: &mut Vec<String>,
    ) -> bool {
        if start == tokens.len() {
            return used.len() >= 2;
        }
        if used.len() >= MAX_STACKED_SEGMENTS {
            return false;
        }
        for end in start + 1..=tokens.len() {
            let segment = tokens[start..end].join(" ");
            let Some(matched_key) = evidence_key_for_normalized(evidence, &segment) else {
                continue;
            };
            if used.contains(&matched_key) {
                continue;
            }
            used.push(matched_key);
            if covers(evidence, tokens, end, used) {
                return true;
            }
            used.pop();
        }
        false
    }

    let tokens = extracted_key.split_whitespace().collect::<Vec<_>>();
    !tokens.is_empty() && covers(evidence, &tokens, 0, &mut Vec::new())
}

/// Matching counterpart of [`parsed_release_matches_title_evidence`] that keeps
/// *which* lookup key matched and whether the release year corroborated it.
/// Pillar A needs both: a shared bare key is not identity evidence for an
/// ambiguous subject, while a unique alias or a year agreement is.
pub(crate) fn match_parsed_release_to_title_evidence(
    parsed: &ParsedReleaseMetadata,
    evidence: &CanonicalTitleEvidence,
) -> Option<TitleEvidenceMatch> {
    if let (Some(parsed_year), Some(expected_year)) = (parsed.year, evidence.year)
        && parsed_year != expected_year
    {
        return None;
    }

    let year_corroborated =
        parsed.year.is_some() && evidence.year.is_some() && parsed.year == evidence.year;
    contextual_release_matches_title_evidence(parsed, evidence, year_corroborated)
}

fn contextual_release_matches_title_evidence(
    parsed: &ParsedReleaseMetadata,
    evidence: &CanonicalTitleEvidence,
    year_corroborated: bool,
) -> Option<TitleEvidenceMatch> {
    // Pass 1 — the unbiased extraction must name this title before the
    // target-biased parse is allowed to prove anything. Bias can refine an
    // anchored identity (projection, numbering, year); it can never
    // manufacture one.
    let anchored = context_free_identity_anchor_keys(&parsed.raw_title)
        .iter()
        .any(|anchor_key| {
            evidence_key_for_normalized(evidence, anchor_key).is_some()
                || extraction_decomposes_into_evidence_keys(evidence, anchor_key)
        });
    if !anchored {
        return None;
    }

    // Pass 2 — the target-biased parse confirms the anchor and supplies the
    // projection the acquisition pipeline actually consumes.
    let contextual = crate::analyze_release_for_target(&parsed.raw_title, &evidence.parse_context);
    if contextual.is_unparseable() {
        return None;
    }
    let best_candidate = contextual.best_candidate()?;

    let year_corroborated = year_corroborated
        || (best_candidate.projected.year.is_some()
            && evidence.year.is_some()
            && best_candidate.projected.year == evidence.year);

    // The biased parse's pre-projection canonical/alias spans confirm the
    // anchor; each must sit inside a recognized title zone. Full zone
    // accounting is the anchor's job now — `Electric Bloom` already failed
    // pass 1 for `BLOOM`, because its unbiased extraction names no such key.
    best_candidate
        .context_title_matches
        .iter()
        .filter(|context_match| {
            !matches!(
                context_match.kind,
                crate::release_parser::ContextTitleMatchKind::EpisodeTitle
            )
        })
        .filter(|context_match| {
            best_candidate.zones.title_zones.iter().any(|zone| {
                context_match.token_range.start_token >= zone.start_token
                    && context_match.token_range.end_token <= zone.end_token
            })
        })
        .filter_map(|context_match| {
            let normalized = crate::title_matching::canonical_lookup_key(&context_match.normalized);
            let matched_key = evidence_key_for_normalized(evidence, &normalized)?;
            let canonical_shape =
                crate::import_title_resolution::strip_trailing_year_key(&evidence.canonical_key);
            let is_single_word_alias = context_match.kind
                == crate::release_parser::ContextTitleMatchKind::TitleAlias
                && normalized.split_whitespace().count() == 1
                && normalized != evidence.canonical_key
                && normalized != canonical_shape;
            Some(TitleEvidenceMatch {
                matched_key,
                year_corroborated,
                requires_external_id: is_single_word_alias && !year_corroborated,
            })
        })
        .max_by_key(|evidence_match| {
            (
                evidence
                    .ambiguity
                    .key_is_unique_to_title(&evidence_match.matched_key),
                evidence_match.matched_key.len(),
            )
        })
}

#[cfg(test)]
pub(crate) fn candidate_matches_title_subject(
    candidate: &IndexerSearchResult,
    evidence: &CanonicalTitleEvidence,
) -> bool {
    candidate_title_match(candidate, evidence).is_some()
}

/// Matching counterpart of [`candidate_matches_title_subject`] that retains the
/// Pillar A disambiguator inputs (matched key and year agreement).
pub(crate) fn candidate_title_match(
    candidate: &IndexerSearchResult,
    evidence: &CanonicalTitleEvidence,
) -> Option<CandidateTitleMatch> {
    let parsed_owned;
    let parsed = if let Some(parsed) = candidate.parsed_release_metadata.as_ref() {
        parsed
    } else {
        parsed_owned =
            crate::parse_release_metadata_for_target(&candidate.title, &evidence.parse_context);
        &parsed_owned
    };

    match_parsed_release_to_title_evidence(parsed, evidence).map(|evidence_match| {
        CandidateTitleMatch {
            evidence_match: Some(evidence_match),
        }
    })
}

/// Pillar A2: for an identity-ambiguous subject an auto candidate must present
/// one positive disambiguator. `external_id_agreement` is the A2(2) input,
/// computed by [`candidate_external_id_agreement`] from the captured response
/// attrs. Per §9 decision 3 an indexer-asserted id suffices alone; a
/// contradicting parsed year has already vetoed the match upstream in
/// [`match_parsed_release_to_title_evidence`], so the year veto still outranks
/// it. Only `Some(true)` satisfies the gate — a disagreement or an absent
/// assertion is simply not a disambiguator, never a veto of its own.
pub(crate) fn candidate_presents_identity_disambiguator(
    evidence: &CanonicalTitleEvidence,
    title_match: &CandidateTitleMatch,
    external_id_agreement: Option<bool>,
) -> bool {
    if let Some(evidence_match) = title_match.evidence_match.as_ref() {
        // A2(1) — the release carries the title's year.
        if evidence_match.year_corroborated {
            return true;
        }
        // A2(3) — the matched key is an alias unique to this title within the
        // library collision set, not the shared bare key.
        if evidence
            .ambiguity
            .key_is_unique_to_title(&evidence_match.matched_key)
        {
            return true;
        }
    }

    // A2(2) — external id agreement. `title_validated_upstream` remains
    // diagnostic provenance and cannot break an identity tie.
    external_id_agreement.unwrap_or(false)
}

/// A2(2): compare the indexer's response ids against ids Scryer already holds.
///
/// `Some(true)` as soon as one id kind both sides carry agrees, `Some(false)`
/// when at least one kind was comparable and none agreed, and `None` when there
/// was nothing to compare — the indexer asserted no ids, or asserted only kinds
/// this subject has no value for.
pub(crate) fn external_id_agreement(
    response: &IndexerResponseAttributes,
    tvdb_id: Option<&str>,
    tmdb_id: Option<&str>,
    imdb_id: Option<&str>,
) -> Option<bool> {
    let agreements = [
        numeric_external_id_agreement(response.tvdb_id.as_deref(), tvdb_id),
        numeric_external_id_agreement(response.tmdb_id.as_deref(), tmdb_id),
        imdb_external_id_agreement(response.imdb_id.as_deref(), imdb_id),
    ];

    if agreements.contains(&Some(true)) {
        return Some(true);
    }
    agreements
        .iter()
        .any(|agreement| agreement.is_some())
        .then_some(false)
}

fn numeric_external_id_agreement(response: Option<&str>, subject: Option<&str>) -> Option<bool> {
    let response = response.map(str::trim).filter(|value| !value.is_empty())?;
    let subject = subject.map(str::trim).filter(|value| !value.is_empty())?;
    Some(response == subject)
}

fn imdb_external_id_agreement(response: Option<&str>, subject: Option<&str>) -> Option<bool> {
    let response = crate::normalize::normalize_imdb_id(response?)?;
    let subject = crate::normalize::normalize_imdb_id(subject?)?;
    Some(response == subject)
}

fn candidate_external_id_agreement(
    candidate: &IndexerSearchResult,
    subject: &ResolvedReleaseSearchSubject,
) -> Option<bool> {
    external_id_agreement(
        &candidate.response_attributes,
        subject.tvdb_id.as_deref(),
        subject.tmdb_id.as_deref(),
        subject.imdb_id.as_deref(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateParseState {
    Parsed,
    Ambiguous,
    Unparseable,
}

fn candidate_parse_state(candidate: &IndexerSearchResult) -> CandidateParseState {
    let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
        return CandidateParseState::Unparseable;
    };

    if matches!(parsed.disposition, ParseDisposition::Unparseable)
        || parsed
            .parse_hints
            .iter()
            .any(|hint| hint == "v2:unparseable" || hint == "parse_status:unparseable")
    {
        return CandidateParseState::Unparseable;
    }
    if parsed.is_ambiguous
        || matches!(parsed.disposition, ParseDisposition::Ambiguous)
        || parsed
            .parse_hints
            .iter()
            .any(|hint| hint == "v2:ambiguous" || hint == "parse_status:ambiguous")
    {
        return CandidateParseState::Ambiguous;
    }
    CandidateParseState::Parsed
}

fn normalized_release_identity(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn media_file_matches_release_identity(file: &TitleMediaFile, release_title: &str) -> bool {
    if release_title.is_empty() {
        return false;
    }

    [
        file.grabbed_release_title.as_deref(),
        file.scene_name.as_deref(),
        file.original_file_path.as_deref(),
        Some(file.file_path.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(normalized_release_identity)
    .any(|candidate| candidate == release_title || candidate.contains(release_title))
}

fn candidate_matches_existing_media_file(
    candidate: &IndexerSearchResult,
    existing_files: &[TitleMediaFile],
    episode_id: Option<&str>,
) -> bool {
    let release_title = normalized_release_identity(&candidate.title);
    if release_title.is_empty() {
        return false;
    }

    existing_files.iter().any(|file| {
        episode_id.is_none_or(|episode_id| file.episode_id.as_deref() == Some(episode_id))
            && media_file_matches_release_identity(file, &release_title)
    })
}

fn grabbed_release_for_search_subject(item: &AcquisitionScopeState) -> Option<String> {
    if item.status == AcquisitionScopeStatus::Completed && item.current_score.is_some() {
        None
    } else {
        item.grabbed_release.clone()
    }
}

pub(crate) fn annotate_auto_decision(
    candidate: &mut IndexerSearchResult,
    code: ReleaseAutoDecisionCode,
) {
    candidate.auto_eligible = Some(code.is_eligible());
    candidate.auto_decision_code = Some(code.as_str().to_string());
    candidate.auto_decision_summary = Some(code.summary().to_string());
}

pub(crate) fn serialize_decision_explanation(candidate: &IndexerSearchResult) -> Option<String> {
    let quality = candidate.quality_profile_decision.as_ref();
    let scoring_log = quality
        .map(|decision| {
            decision
                .scoring_log
                .iter()
                .map(|entry| serde_json::json!({"code": entry.code, "delta": entry.delta}))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let parsed = candidate.parsed_release_metadata.as_ref().map(|parsed| {
        serde_json::json!({
            "raw_title": parsed.raw_title.as_str(),
            "normalized_title": parsed.normalized_title.as_str(),
            "normalized_title_variants": &parsed.normalized_title_variants,
            "year": parsed.year,
            "quality": parsed.quality.as_deref(),
            "source": parsed.source.as_ref().map(|source| format!("{source:?}")),
            "release_group": parsed.release_group.as_deref(),
            "disposition": format!("{:?}", parsed.disposition),
            "parse_family": format!("{:?}", parsed.parse_family),
            "parse_confidence": parsed.parse_confidence,
            "is_ambiguous": parsed.is_ambiguous,
            "parse_hints": &parsed.parse_hints,
        })
    });
    let payload = serde_json::json!({
        "candidate": {
            "source": candidate.source.as_str(),
            "source_kind": candidate.source_kind.map(|kind| kind.as_str()),
            "guid": candidate.guid.as_deref(),
            "download_url_present": candidate.download_url.as_deref().is_some_and(|value| !value.trim().is_empty()),
            "link_present": candidate.link.as_deref().is_some_and(|value| !value.trim().is_empty()),
        },
        "auto_decision": {
            "eligible": candidate.auto_eligible,
            "code": candidate.auto_decision_code.as_deref(),
            "summary": candidate.auto_decision_summary.as_deref(),
        },
        "quality_profile_decision": {
            "allowed": quality.map(|decision| decision.allowed),
            "block_codes": quality.map(|decision| &decision.block_codes),
            "release_score": quality.map(|decision| decision.release_score),
            "preference_score": quality.map(|decision| decision.preference_score),
            "scoring_log": scoring_log,
        },
        "parsed": parsed,
    });

    serde_json::to_string(&payload).ok()
}

/// Sonarr-style episode anchoring for numbering-scoped subjects: an episode or
/// season search must not auto-grab a release whose parse carries no episode
/// identity at all (movie-shaped bare-title junk survives the title guard for
/// generic names like "Friends"), or whose numbering contradicts the target.
/// Absent parse data stays permissive — the ambiguous/unparseable handling in
/// `evaluate_auto_candidate` owns that case. Daily (air-date) and season-pack
/// parses carry an episode identity and are only checked for fields both
/// sides actually have, mirroring the strategy-level guard in the search
/// client.
fn candidate_numbering_contradicts_subject(
    candidate: &IndexerSearchResult,
    subject: &ResolvedReleaseSearchSubject,
) -> bool {
    if subject.season.is_none() && subject.episode.is_none() {
        return false;
    }
    let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
        return false;
    };
    let Some(episode) = parsed.episode.as_ref() else {
        return true;
    };
    if let (Some(expected_season), Some(found_season)) = (subject.season, episode.season)
        && expected_season != found_season
    {
        return true;
    }
    if let Some(expected_episode) = subject.episode
        && !episode.episode_numbers.is_empty()
        && !episode.episode_numbers.contains(&expected_episode)
    {
        return true;
    }
    false
}

pub(crate) fn evaluate_auto_candidate(
    candidate: &IndexerSearchResult,
    context: &AutoCandidateEvaluationContext<'_>,
) -> ReleaseAutoDecisionCode {
    let parse_state = candidate_parse_state(candidate);
    let title_match = candidate_title_match(candidate, &context.subject.title_evidence);
    let matches_title = title_match.is_some();
    match parse_state {
        CandidateParseState::Ambiguous if !matches_title => {
            return ReleaseAutoDecisionCode::ParseAmbiguous;
        }
        CandidateParseState::Unparseable if !matches_title => {
            return ReleaseAutoDecisionCode::ParseUnparseable;
        }
        CandidateParseState::Ambiguous
        | CandidateParseState::Unparseable
        | CandidateParseState::Parsed => {}
    }

    let Some(title_match) = title_match else {
        return ReleaseAutoDecisionCode::TitleMismatch;
    };

    if candidate_numbering_contradicts_subject(candidate, context.subject) {
        return ReleaseAutoDecisionCode::EpisodeMismatch;
    }

    // Pillar D2: the indexer filed this release under a category that
    // contradicts the subject. Checked before the ambiguity gate because it is
    // the sharper reason — an explicit contradiction rather than absent
    // evidence — and it is the only category protection torrent/magnet grabs and
    // out-of-band plugin NZB fetches get, since D1 only sees NZBs Scryer itself
    // downloads before submission.
    // Compare against the facet the subject was SEARCHED as, not the owning
    // title's facet: a series-movie subject is movie-faceted while its owner
    // is a series, and a correctly categorized Movies release must not read
    // as a contradiction.
    if crate::indexer_category::indexer_categories_contradict_facet(
        &candidate.response_attributes.categories,
        &context.subject.search_facet,
    ) {
        return ReleaseAutoDecisionCode::CategoryMismatch;
    }

    // Burned releases report as blocklisted BEFORE the ambiguity gate runs: a
    // release that already failed must never be re-parked for review.
    if context
        .db_blocklist
        .contains(&candidate.title.to_ascii_lowercase())
    {
        return ReleaseAutoDecisionCode::DbBlocklisted;
    }

    let external_id_agreement = candidate_external_id_agreement(candidate, context.subject);
    if title_match
        .evidence_match
        .as_ref()
        .is_some_and(|evidence_match| evidence_match.requires_external_id)
        && external_id_agreement != Some(true)
    {
        return ReleaseAutoDecisionCode::AmbiguousIdentity;
    }

    // Pillar A3: a bare release name is not identity evidence when the subject's
    // canonical title collides with another library title.
    if context
        .subject
        .title_evidence
        .ambiguity
        .requires_disambiguator()
        && !candidate_presents_identity_disambiguator(
            &context.subject.title_evidence,
            &title_match,
            external_id_agreement,
        )
    {
        return ReleaseAutoDecisionCode::AmbiguousIdentity;
    }

    let is_allowed = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.allowed)
        .unwrap_or(false);
    if !is_allowed {
        return ReleaseAutoDecisionCode::QualityBlocked;
    }

    if context.cutoff_reached {
        return ReleaseAutoDecisionCode::CutoffReached;
    }

    let candidate_score = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.preference_score)
        .unwrap_or(0);
    if candidate_score < 0 {
        return ReleaseAutoDecisionCode::NegativeScore;
    }

    if let Some(dl_snapshot) = context.dl_snapshot
        && dl_snapshot.is_active(&candidate.title)
    {
        return ReleaseAutoDecisionCode::AlreadyActive;
    }

    if candidate_matches_existing_media_file(
        candidate,
        context.existing_files,
        context.subject.submission_scope.episode_id(),
    ) {
        return ReleaseAutoDecisionCode::AlreadyActive;
    }

    if let Some(failed_source_kinds) = context.failed_source_kinds
        && let Some(source_kind) = candidate.source_kind
        && failed_source_kinds.contains(&source_kind)
    {
        return ReleaseAutoDecisionCode::DownloadClientUnavailable;
    }

    let decision = evaluate_upgrade(
        candidate_score,
        context.current_score,
        context.profile.criteria.allow_upgrades,
        context.last_search_at,
        context.now,
        context.thresholds,
        context.profile.criteria.min_score_to_grab,
    );
    if !decision.is_accept() {
        return ReleaseAutoDecisionCode::UpgradeRejected;
    }

    if crate::acquisition_policy::should_skip_repack_group_mismatch(
        candidate,
        context.existing_files,
        context.subject.submission_scope.episode_id(),
    ) {
        return ReleaseAutoDecisionCode::RepackGroupMismatch;
    }

    if let Some(delay_decision) = crate::delay_profile::resolve_delay_decision(
        context.delay_profiles,
        &context.title.tags,
        &context.title.facet,
        candidate.source_kind,
        candidate
            .published_at
            .as_deref()
            .and_then(crate::quality_profile::parse_published_at),
        candidate_score,
        context.now,
    ) && delay_decision.should_hold()
    {
        return ReleaseAutoDecisionCode::PendingDelay;
    }

    ReleaseAutoDecisionCode::Eligible
}

fn preferred_scoped_external_id(ids: &[ScopedExternalId], source: &str) -> Option<String> {
    ids.iter()
        .find(|id| {
            id.source.eq_ignore_ascii_case(source)
                && id
                    .source_scope
                    .as_deref()
                    .is_some_and(|scope| scope.eq_ignore_ascii_case("R"))
                && !id.external_id.trim().is_empty()
        })
        .or_else(|| {
            ids.iter().find(|id| {
                id.source.eq_ignore_ascii_case(source) && !id.external_id.trim().is_empty()
            })
        })
        .map(|id| id.external_id.trim().to_string())
}

impl AppUseCase {
    /// Library-local identity ambiguity for a search subject (Pillar A tier 0).
    /// Reads the cached monitored-title matcher, whose normalized-title index is
    /// already built from `canonical_title_lookup_keys`, so a convergence cycle
    /// pays for one index build instead of a query per subject. Falls back to
    /// "not ambiguous" when the index cannot be loaded — Pillar B still catches
    /// the bad import.
    pub(crate) async fn title_identity_ambiguity(&self, title: &Title) -> TitleIdentityAmbiguity {
        match self.monitored_title_matcher().await {
            Ok(matcher) => TitleIdentityAmbiguity::from_shared_keys(
                matcher.shared_lookup_keys(&title.id, &canonical_title_lookup_keys(title)),
            ),
            Err(error) => {
                tracing::debug!(
                    title_id = title.id.as_str(),
                    error = %error,
                    "identity ambiguity: monitored title index unavailable, treating title as unambiguous"
                );
                TitleIdentityAmbiguity::default()
            }
        }
    }

    fn release_search_category_for_facet(&self, facet: &MediaFacet) -> String {
        self.facet_registry
            .get(facet)
            .map(|handler| handler.search_category().to_string())
            .unwrap_or_else(|| match facet {
                MediaFacet::Movie => "movie".to_string(),
                MediaFacet::Series => "series".to_string(),
                MediaFacet::Anime => "anime".to_string(),
            })
    }

    pub(crate) async fn local_scoped_anidb_id_for_episode(
        &self,
        episode: Option<&Episode>,
    ) -> Option<String> {
        let episode = episode?;
        // Prefer season/collection-scoped AniDB mappings, then let callers fall
        // back to the title-level AniDB ID.
        let collection_id = episode.collection_id.as_deref()?;
        self.local_scoped_anidb_id_for_collection(collection_id)
            .await
    }

    async fn local_scoped_anidb_id_for_collection(&self, collection_id: &str) -> Option<String> {
        let collection_ids = self
            .services
            .catalog
            .shows
            .list_collection_external_ids(collection_id)
            .await
            .unwrap_or_default();
        preferred_scoped_external_id(&collection_ids, "anidb")
    }

    pub(crate) async fn release_search_title_for_wanted_item(
        &self,
        title: &Title,
        item: &AcquisitionScopeState,
        episode: Option<&Episode>,
    ) -> Title {
        let search_title = if item.media_type == "series_movie" {
            if let Some(ref link_id) = item.series_movie_link_id
                && let Ok(Some(link)) = self
                    .services
                    .catalog
                    .shows
                    .get_series_movie_link_by_id(link_id)
                    .await
            {
                series_movie_search_title(title, &link)
            } else {
                title.clone()
            }
        } else {
            title.clone()
        };

        if item.media_type == "episode"
            && let Some(anidb_id) = self.local_scoped_anidb_id_for_episode(episode).await
        {
            let mut search_title = search_title;
            search_title.external_ids.retain(|id| {
                !matches!(
                    id.source.trim().to_ascii_lowercase().as_str(),
                    "anidb" | "anidb_id"
                )
            });
            search_title.external_ids.push(scryer_domain::ExternalId {
                source: "anidb".into(),
                value: anidb_id,
            });
            return search_title;
        }

        search_title
    }

    pub(crate) async fn evaluate_search_results_for_subject(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        mut results: Vec<IndexerSearchResult>,
    ) -> Vec<IndexerSearchResult> {
        let db_blocklist: HashSet<String> = self
            .services
            .workflow
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 200)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| entry.source_title)
            .map(|value| value.to_ascii_lowercase())
            .collect();

        let dl_snapshot = crate::acquisition_workflow::DownloadClientSnapshot::fetch(self).await;
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
        let delay_profiles = self.load_delay_profiles().await;
        let now = Utc::now();
        let analyzed_cutoff_quality =
            crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
                &existing_files,
                subject.submission_scope.episode_id(),
                subject.submission_scope.series_movie_link_id(),
            );
        let upgrade_context = self
            .resolve_upgrade_context_for_title_with_category_and_quality(
                title,
                subject.grabbed_release.as_deref(),
                Some(subject.category.as_str()),
                analyzed_cutoff_quality,
            )
            .await;

        let evaluation_context = AutoCandidateEvaluationContext {
            title,
            subject,
            current_score: subject.current_score,
            last_search_at: subject.last_search_at.as_deref(),
            profile: &upgrade_context.profile,
            thresholds: &upgrade_context.thresholds,
            cutoff_reached: upgrade_context.cutoff_reached,
            now: &now,
            dl_snapshot: Some(&dl_snapshot),
            db_blocklist: &db_blocklist,
            existing_files: &existing_files,
            delay_profiles: &delay_profiles,
            failed_source_kinds: None,
        };

        for candidate in &mut results {
            let code = evaluate_auto_candidate(candidate, &evaluation_context);
            annotate_auto_decision(candidate, code);
        }

        results
    }

    pub(crate) async fn resolve_release_search_subject_for_title(
        &self,
        title: &Title,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let category = self.release_search_category_for_facet(&title.facet);
        let query = if title.facet == MediaFacet::Movie {
            movie_text_search_query(&title.name, title.year)
        } else {
            title.name.trim().to_string()
        };
        if query.is_empty() && imdb_id.is_none() && tvdb_id.is_none() && anidb_id.is_none() {
            return Err(AppError::Validation(
                "title has no name or external IDs".into(),
            ));
        }

        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_for_title(&title.id, None)
            .await
            .ok()
            .flatten();

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title)
                .with_ambiguity(self.title_identity_ambiguity(title).await),
            queries: vec![query],
            imdb_id,
            tmdb_id: tmdb_id_from_external_ids(&title.external_ids),
            tvdb_id,
            anidb_id,
            mal_id: mal_id_from_external_ids(&title.external_ids),
            category: category.clone(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            current_score: wanted.as_ref().and_then(|item| item.current_score),
            last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
            grabbed_release: wanted.as_ref().and_then(grabbed_release_for_search_subject),
            submission_scope: SubmissionScope::Title,
        })
    }

    pub(crate) async fn resolve_release_search_subject_for_episode(
        &self,
        title: &Title,
        season: &str,
        episode: &str,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let season = season.trim();
        let episode = episode.trim();
        if season.is_empty() || episode.is_empty() {
            return Err(AppError::Validation(
                "season and episode are required".into(),
            ));
        }

        let season_digits: String = season
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();
        let episode_digits: String = episode
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();
        if season_digits.is_empty() || episode_digits.is_empty() {
            return Err(AppError::Validation(
                "season and episode must include numeric values".into(),
            ));
        }

        let season_num = season_digits
            .parse::<u32>()
            .map_err(|_| AppError::Validation("invalid season value".into()))?;
        let episode_num = episode_digits
            .parse::<u32>()
            .map_err(|_| AppError::Validation("invalid episode value".into()))?;

        let episode_record = self
            .services
            .catalog
            .shows
            .find_episode_by_title_and_numbers(&title.id, &season_digits, &episode_digits)
            .await?;

        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_for_title(
                &title.id,
                episode_record.as_ref().map(|episode| episode.id.as_str()),
            )
            .await
            .ok()
            .flatten();

        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let title_anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = self
            .local_scoped_anidb_id_for_episode(episode_record.as_ref())
            .await
            .or(title_anidb_id);

        let absolute_episode = episode_record
            .as_ref()
            .and_then(|episode| episode.absolute_number.as_deref())
            .and_then(|value| value.trim().parse::<u32>().ok());

        let category = self.release_search_category_for_facet(&title.facet);

        let mut queries = vec![format!(
            "{} S{:0>2}E{:0>2}",
            title.name.trim(),
            season_num,
            episode_num
        )];
        queries.push(format!("{} S{:0>2}", title.name.trim(), season_num));
        if title.facet == MediaFacet::Anime {
            if let Some(absolute) = absolute_episode {
                queries.insert(0, format!("{} {:0>3}", title.name.trim(), absolute));
            }
            queries.push(title.name.trim().to_string());
        }
        let mut seen = HashSet::new();
        queries.retain(|query| !query.trim().is_empty() && seen.insert(query.to_ascii_lowercase()));

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence_for_episode(title, episode_record.as_ref())
                .with_ambiguity(self.title_identity_ambiguity(title).await),
            queries,
            imdb_id,
            tmdb_id: tmdb_id_from_external_ids(&title.external_ids),
            tvdb_id,
            anidb_id,
            mal_id: mal_id_from_external_ids(&title.external_ids),
            category: category.clone(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: episode_record
                .as_ref()
                .and_then(|episode| episode.duration_seconds)
                .map(|seconds| (seconds / 60) as i32)
                .or(title.runtime_minutes),
            season: Some(season_num),
            episode: Some(episode_num),
            absolute_episode,
            subject_kind: ReleaseSearchSubjectKind::Episode,
            current_score: wanted.as_ref().and_then(|item| item.current_score),
            last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
            grabbed_release: wanted.as_ref().and_then(grabbed_release_for_search_subject),
            submission_scope: episode_record
                .as_ref()
                .map(|episode| SubmissionScope::Episode {
                    episode_id: episode.id.clone(),
                })
                .unwrap_or(SubmissionScope::Title),
        })
    }

    pub(crate) async fn resolve_release_search_subject_for_season_pack(
        &self,
        title: &Title,
        item: &AcquisitionScopeState,
        episode: Option<&Episode>,
        season_num: u32,
        runtime_minutes: Option<i32>,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let collection_anidb_id = match episode.and_then(|episode| episode.collection_id.as_deref())
        {
            Some(collection_id) => {
                self.local_scoped_anidb_id_for_collection(collection_id)
                    .await
            }
            None => None,
        };
        let anidb_id = collection_anidb_id.or(anidb_id);
        let category = self.release_search_category_for_facet(&title.facet);
        let mut queries = vec![format!("{} S{:0>2}", title.name.trim(), season_num)];
        queries.retain(|query| !query.trim().is_empty());
        if queries.is_empty() && (imdb_id.is_some() || tvdb_id.is_some() || anidb_id.is_some()) {
            queries.push(String::new());
        }
        if queries.is_empty() {
            return Err(AppError::Validation(
                "season pack search subject has no searchable title or external IDs".into(),
            ));
        }

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title)
                .with_ambiguity(self.title_identity_ambiguity(title).await),
            queries,
            imdb_id,
            tmdb_id: tmdb_id_from_external_ids(&title.external_ids),
            tvdb_id,
            anidb_id,
            mal_id: mal_id_from_external_ids(&title.external_ids),
            category: category.clone(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes,
            season: Some(season_num),
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Season,
            current_score: item.current_score,
            last_search_at: item.last_search_at.clone(),
            grabbed_release: grabbed_release_for_search_subject(item),
            submission_scope: collection_download_submission_scope_for_wanted_item(item, episode),
        })
    }

    pub(crate) async fn resolve_release_search_subject_for_series_movie(
        &self,
        title: &Title,
        link: &scryer_domain::SeriesMovieLink,
    ) -> AppResult<(Title, ResolvedReleaseSearchSubject)> {
        let search_title = series_movie_search_title(title, link);
        if search_title.name.trim().is_empty() {
            return Err(AppError::Validation(
                "series movie search subject has no searchable title".into(),
            ));
        }

        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                media_types: vec!["series_movie".into()],
                title_id: Some(title.id.clone()),
                limit: 500,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?
            .into_iter()
            .find(|item| item.series_movie_link_id.as_deref() == Some(link.id.as_str()));

        let imdb_id = search_title
            .imdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_imdb_id);
        let tvdb_id = tvdb_id_from_external_ids(&search_title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = anidb_id_from_external_ids(&search_title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let query_result = build_movie_search_queries(
            &search_title,
            "series_movie",
            self.release_search_category_for_facet(&search_title.facet),
        );
        let mut queries = query_result.queries;
        if queries.is_empty() && imdb_id.is_some() {
            queries.push(String::new());
        }
        let category = self.release_search_category_for_facet(&search_title.facet);

        Ok((
            search_title.clone(),
            ResolvedReleaseSearchSubject {
                title_id: title.id.clone(),
                title_tags: title.tags.clone(),
                title_evidence: canonical_title_evidence(&search_title)
                    .with_ambiguity(self.title_identity_ambiguity(&search_title).await),
                queries,
                imdb_id,
                tmdb_id: tmdb_id_from_external_ids(&search_title.external_ids),
                tvdb_id,
                anidb_id,
                mal_id: mal_id_from_external_ids(&search_title.external_ids),
                category,
                owner_facet: title.facet.clone(),
                search_facet: search_title.facet.clone(),
                id_search_facet: Some(MediaFacet::Movie),
                newznab_categories: series_movie_newznab_categories(&title.facet),
                runtime_minutes: search_title.runtime_minutes,
                season: None,
                episode: None,
                absolute_episode: None,
                subject_kind: ReleaseSearchSubjectKind::Title,
                current_score: wanted.as_ref().and_then(|item| item.current_score),
                last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
                grabbed_release: wanted.as_ref().and_then(grabbed_release_for_search_subject),
                submission_scope: SubmissionScope::SeriesMovie {
                    series_movie_link_id: link.id.clone(),
                },
            },
        ))
    }

    pub(crate) async fn resolve_release_search_subject_for_wanted_item(
        &self,
        owner_title: &Title,
        search_title: &Title,
        item: &AcquisitionScopeState,
        episode: Option<&Episode>,
    ) -> ResolvedReleaseSearchSubject {
        let query_result = build_search_queries(search_title, item, episode, &self.facet_registry);
        let owner_facet = if item.media_type == "series_movie" {
            owner_title.facet.clone()
        } else {
            owner_facet_for_wanted_item(owner_title, item)
        };
        let absolute_episode = episode
            .and_then(|episode| episode.absolute_number.as_deref())
            .and_then(|value| value.parse::<u32>().ok());

        ResolvedReleaseSearchSubject {
            title_id: owner_title.id.clone(),
            title_tags: owner_title.tags.clone(),
            title_evidence: canonical_title_evidence_for_episode(search_title, episode)
                .with_ambiguity(self.title_identity_ambiguity(search_title).await),
            queries: query_result.queries,
            imdb_id: query_result.imdb_id,
            tmdb_id: query_result.tmdb_id,
            tvdb_id: query_result.tvdb_id,
            anidb_id: query_result.anidb_id,
            mal_id: query_result.mal_id,
            category: query_result.category.clone(),
            owner_facet: owner_facet.clone(),
            search_facet: search_title.facet.clone(),
            id_search_facet: (item.media_type == "series_movie").then_some(MediaFacet::Movie),
            newznab_categories: if item.media_type == "series_movie" {
                series_movie_newznab_categories(&owner_facet)
            } else {
                Vec::new()
            },
            runtime_minutes: episode
                .and_then(|episode| episode.duration_seconds)
                .map(|seconds| (seconds / 60) as i32)
                .or(search_title.runtime_minutes),
            season: query_result.season,
            episode: query_result.episode,
            absolute_episode,
            subject_kind: match item.media_type.as_str() {
                "episode" => ReleaseSearchSubjectKind::Episode,
                _ => ReleaseSearchSubjectKind::Title,
            },
            current_score: item.current_score,
            last_search_at: item.last_search_at.clone(),
            grabbed_release: grabbed_release_for_search_subject(item),
            submission_scope: direct_download_submission_scope_for_wanted_item(item, episode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{MediaFacet, TaggedAlias, Title};

    fn make_title() -> Title {
        Title {
            id: "title-1".to_string(),
            name: "Nightfall!!".to_string(),
            facet: MediaFacet::Anime,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Anime),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2022),
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
            tagged_aliases: vec![TaggedAlias {
                name: "Nightfall Heavy Metal Dark Fantasy".to_string(),
                language: "eng".to_string(),
            }],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn make_candidate(
        release_title: &str,
        provenance: Option<ReleaseCandidateProvenance>,
    ) -> IndexerSearchResult {
        IndexerSearchResult {
            indexer_id: None,
            source: "nzbgeek".to_string(),
            title: release_title.to_string(),
            link: None,
            download_url: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            size_bytes: None,
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: Some(crate::parse_release_metadata(release_title)),
            quality_profile_decision: None,
            extra: Default::default(),
            response_attributes: Default::default(),
            guid: None,
            info_url: None,
            provenance,
            candidate_token: None,
            queue_scope: None,
            auto_eligible: None,
            auto_decision_code: None,
            auto_decision_summary: None,
        }
    }

    fn make_media_file(release_title: &str, episode_id: Option<&str>) -> TitleMediaFile {
        TitleMediaFile {
            id: "media-file-1".to_string(),
            title_id: "title-1".to_string(),
            episode_id: episode_id.map(str::to_string),
            series_movie_link_ids: Vec::new(),
            role: crate::MediaFileRole::Primary,
            file_path: format!("/data/series/{release_title}.mkv"),
            size_bytes: 1,
            source_signature_scheme: None,
            source_signature_value: None,
            quality_label: Some("720p".to_string()),
            scan_status: "scanned".to_string(),
            created_at: Utc::now().to_rfc3339(),
            video_codec: None,
            video_width: Some(1280),
            video_height: Some(720),
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: None,
            audio_profile: None,
            audio_channels: None,
            audio_bitrate_kbps: None,
            audio_languages: Vec::new(),
            audio_streams: Vec::new(),
            subtitle_languages: Vec::new(),
            subtitle_codecs: Vec::new(),
            subtitle_streams: Vec::new(),
            has_multiaudio: false,
            duration_seconds: None,
            num_chapters: None,
            container_format: None,
            scene_name: Some(release_title.to_string()),
            release_group: None,
            source_type: None,
            resolution: Some("720p".to_string()),
            video_codec_parsed: None,
            audio_codec_parsed: None,
            audio_channels_parsed: None,
            acquisition_score: Some(-15),
            scoring_log: None,
            indexer_source: None,
            grabbed_release_title: None,
            grabbed_at: None,
            edition: None,
            original_file_path: Some(format!(
                "/nzbget-downloads/completed/{release_title}/{release_title}.mkv"
            )),
            release_hash: None,
        }
    }

    fn make_wanted_item(
        status: AcquisitionScopeStatus,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AcquisitionScopeState {
        AcquisitionScopeState {
            id: "wanted-1".to_string(),
            title_id: "title-1".to_string(),
            title_name: None,
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status,
            grabbed_release: grabbed_release.map(str::to_string),
            current_score,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn allowed_quality_decision(score: i32) -> QualityProfileDecision {
        QualityProfileDecision {
            release_score: score,
            scoring_log: Vec::new(),
            allowed: true,
            block_codes: Vec::new(),
            preference_score: score,
        }
    }

    #[test]
    fn canonical_title_lookup_keys_include_tagged_aliases() {
        let title = make_title();
        let keys = canonical_title_lookup_keys(&title);

        assert!(keys.iter().any(|key| key == "nightfall"));
        assert!(
            keys.iter()
                .any(|key| key == "nightfall heavy metal dark fantasy")
        );
    }

    #[test]
    fn upstream_validation_cannot_bypass_raw_title_proof() {
        let mut title = make_title();
        title.name = "Resident Evil".to_string();
        title.facet = MediaFacet::Movie;
        title.year = Some(2026);
        let candidate = make_candidate(
            "Resident.Evil.2002.1080p.WEB-DL",
            Some(ReleaseCandidateProvenance {
                search_subject_kind: ReleaseSearchSubjectKind::Episode,
                strategy_kind: ReleaseStrategyKind::IdBacked,
                title_validated_upstream: true,
            }),
        );

        assert!(!candidate_matches_title_subject(
            &candidate,
            &canonical_title_evidence(&title)
        ));
    }

    #[test]
    fn text_title_matching_rejects_only_mismatched_parsed_years() {
        let mut title = make_title();
        title.name = "Resident Evil".to_string();
        title.facet = MediaFacet::Movie;
        title.year = Some(2026);
        let evidence = canonical_title_evidence(&title);

        let mismatched = crate::parse_release_metadata("Resident.Evil.2002.1080p.WEB-DL");
        assert_eq!(mismatched.year, Some(2002));
        assert!(!parsed_release_matches_title_evidence(
            &mismatched,
            &evidence
        ));

        let mut matching = mismatched.clone();
        matching.year = Some(2026);
        assert!(parsed_release_matches_title_evidence(&matching, &evidence));

        let mut missing_year = mismatched;
        missing_year.year = None;
        assert!(parsed_release_matches_title_evidence(
            &missing_year,
            &evidence
        ));
    }

    #[test]
    fn automatic_text_candidate_with_mismatched_year_is_not_eligible() {
        let mut title = make_title();
        title.name = "Resident Evil".to_string();
        title.facet = MediaFacet::Movie;
        title.year = Some(2026);
        let candidate = make_candidate("Resident.Evil.2002.1080p.WEB-DL", None);
        let subject = ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(&title),
            queries: vec!["Resident Evil 2026".to_string()],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category: title.facet.as_str().to_string(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            current_score: None,
            last_search_at: None,
            grabbed_release: None,
            submission_scope: SubmissionScope::Title,
        };
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = HashSet::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            current_score: None,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            cutoff_reached: false,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_source_kinds: None,
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::TitleMismatch
        );
    }

    fn numbering_scoped_subject(
        title: &Title,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> ResolvedReleaseSearchSubject {
        ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title),
            queries: vec![title.name.clone()],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category: title.facet.as_str().to_string(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season,
            episode,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Episode,
            current_score: None,
            last_search_at: None,
            grabbed_release: None,
            submission_scope: SubmissionScope::Title,
        }
    }

    #[test]
    fn episode_subject_rejects_candidates_without_episode_identity() {
        // Bare-title junk for a generic name ("Friends") carries neither a
        // contradicting year nor episode numbering — the movie-shaped parse is
        // the only signal, and an episode-scoped subject must refuse it.
        let mut title = make_title();
        title.name = "Friends".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = make_candidate("Friends.1080p.BluRay.x264-GRP", None);
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = HashSet::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            current_score: None,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            cutoff_reached: false,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_source_kinds: None,
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::EpisodeMismatch
        );
    }

    #[test]
    fn episode_subject_rejects_contradicting_season_numbering() {
        let mut title = make_title();
        title.name = "Friends".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = make_candidate("Friends.S05E01.1080p.WEB-DL", None);
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = HashSet::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            current_score: None,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            cutoff_reached: false,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_source_kinds: None,
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::EpisodeMismatch
        );
    }

    #[test]
    fn episode_subject_accepts_matching_numbering() {
        // A real multi-episode release (Friends.S09E23E24) agrees on season
        // and contains the wanted episode; it must clear the numbering gate
        // and fall through to the quality decision (absent here → blocked).
        let mut title = make_title();
        title.name = "Friends".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = make_candidate("Friends.S09E23E24.1080p.BluRay.x264-TENEIGHTY", None);
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = HashSet::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            current_score: None,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            cutoff_reached: false,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_source_kinds: None,
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn friends_year_bearing_junk_is_vetoed_but_bare_releases_match() {
        // Pins the intent of the unconditional year veto: wrong-property junk
        // that carries its own year (the Korean "Friends" class) must never
        // match the 1994 series, while the real scene releases — which are
        // bare-titled — must keep matching.
        let mut title = make_title();
        title.name = "Friends".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let evidence = canonical_title_evidence(&title);

        let mut junk = crate::parse_release_metadata("Friends.S01E01.1080p.WEB-DL");
        junk.year = Some(2002);
        assert!(!parsed_release_matches_title_evidence(&junk, &evidence));

        let legit =
            crate::parse_release_metadata("Friends.S09E23E24.1080p.NF.WEB-DL.DDP5.1.x264-PRAGMA");
        assert_eq!(legit.year, None);
        assert!(parsed_release_matches_title_evidence(&legit, &evidence));
    }

    // ── Pillar A: identity ambiguity + required disambiguators ───────────────

    /// The incident pair: a live-action `One Piece` (2023, series) and the
    /// anime `One Piece` (1999) in the same library, both claiming the bare
    /// canonical key `one piece`. `aliases` is applied to the live-action title
    /// so a unique-alias hit can be exercised.
    fn one_piece_library(aliases: Vec<String>) -> (Title, Vec<Title>) {
        let mut live_action = make_title();
        live_action.id = "title-one-piece-live".to_string();
        live_action.name = "One Piece".to_string();
        live_action.facet = MediaFacet::Series;
        live_action.year = Some(2023);
        live_action.aliases = aliases;
        live_action.tagged_aliases = Vec::new();

        let mut anime = make_title();
        anime.id = "title-one-piece-anime".to_string();
        anime.name = "One Piece".to_string();
        anime.facet = MediaFacet::Anime;
        anime.year = Some(1999);
        anime.aliases = Vec::new();
        anime.tagged_aliases = Vec::new();

        let library = vec![live_action.clone(), anime];
        (live_action, library)
    }

    /// Tier 0 ambiguity exactly as the acquisition paths derive it: from the
    /// monitored-title index over the library, with no schema or SMG input.
    fn library_local_ambiguity(subject: &Title, library: &[Title]) -> TitleIdentityAmbiguity {
        let matcher = crate::import_title_resolution::MonitoredTitleMatcher::new(library.to_vec());
        TitleIdentityAmbiguity::from_shared_keys(
            matcher.shared_lookup_keys(&subject.id, &canonical_title_lookup_keys(subject)),
        )
    }

    fn ambiguous_episode_subject(
        title: &Title,
        library: &[Title],
        season: Option<u32>,
        episode: Option<u32>,
    ) -> ResolvedReleaseSearchSubject {
        let mut subject = numbering_scoped_subject(title, season, episode);
        subject.title_evidence = subject
            .title_evidence
            .with_ambiguity(library_local_ambiguity(title, library));
        subject
    }

    fn decision_for(
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        candidate: &IndexerSearchResult,
    ) -> ReleaseAutoDecisionCode {
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = HashSet::new();
        let context = AutoCandidateEvaluationContext {
            title,
            subject,
            current_score: None,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            cutoff_reached: false,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_source_kinds: None,
        };
        evaluate_auto_candidate(candidate, &context)
    }

    #[test]
    fn library_local_collision_flags_shared_bare_key() {
        let (live_action, library) = one_piece_library(vec!["One Piece Live Action".to_string()]);
        let ambiguity = library_local_ambiguity(&live_action, &library);

        assert!(ambiguity.requires_disambiguator());
        assert_eq!(ambiguity.shared_lookup_keys, vec!["one piece".to_string()]);
        assert!(!ambiguity.key_is_unique_to_title("one piece"));
        assert!(ambiguity.key_is_unique_to_title("one piece live action"));
    }

    #[test]
    fn ambiguous_title_rejects_bare_candidate_without_disambiguator() {
        // The driving incident: a bare `One.Piece.S02E01` names both library
        // titles equally well, so it is not identity evidence for either.
        let (live_action, library) = one_piece_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate("One.Piece.S02E01.1080p.WEB-DL.x264-GRP", None);

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity
        );
    }

    #[test]
    fn ambiguous_title_accepts_year_disambiguator() {
        // A2(1): the release carries the live-action title's year, so it names
        // one of the two colliding titles and clears the identity gate.
        let (live_action, library) = one_piece_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate("One.Piece.2023.S02E01.1080p.WEB-DL.x264-GRP", None);

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ambiguous_title_accepts_unique_alias_disambiguator() {
        // A2(3): the matched key is an alias only the live-action title claims.
        let (live_action, library) = one_piece_library(vec!["One Piece Live Action".to_string()]);
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate("One.Piece.Live.Action.S02E01.1080p.WEB-DL.x264-GRP", None);

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ambiguous_title_rejects_upstream_provenance_without_release_id() {
        let (live_action, library) = one_piece_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate(
            "One.Piece.S02E01.1080p.WEB-DL.x264-GRP",
            Some(ReleaseCandidateProvenance {
                search_subject_kind: ReleaseSearchSubjectKind::Episode,
                strategy_kind: ReleaseStrategyKind::IdBacked,
                title_validated_upstream: true,
            }),
        );

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity
        );
    }

    #[test]
    fn year_suffixed_title_pair_still_collides_and_bare_release_is_ambiguous() {
        // Adversarial-review regression: `One Piece` vs `One Piece (2023)` is
        // the commonest real collision shape; byte-equality collision
        // detection missed it, and the with_year matching bridge then
        // laundered the synthesized `one piece 2023` key into a "unique
        // alias" disambiguator for a bare release.
        let (mut live_action, mut library) = one_piece_library(Vec::new());
        live_action.name = "One Piece (2023)".to_string();
        library[0] = live_action.clone();

        let ambiguity = library_local_ambiguity(&live_action, &library);
        assert!(
            ambiguity.requires_disambiguator(),
            "year-suffixed pair must collide: {ambiguity:?}"
        );

        let mut subject = numbering_scoped_subject(&live_action, Some(2), Some(1));
        subject.title_evidence = subject.title_evidence.with_ambiguity(ambiguity);
        let candidate = make_candidate("One.Piece.S02E01.1080p.WEB-DL.x264-GRP", None);
        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity,
            "a bare release must not clear the gate via a synthesized year key"
        );
    }

    #[test]
    fn blocklisted_release_reports_blocklisted_not_ambiguous() {
        // A burned release must never be re-parked for review: DbBlocklisted
        // outranks AmbiguousIdentity in the decision order.
        let (live_action, library) = one_piece_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = make_candidate("One.Piece.S02E01.1080p.WEB-DL.x264-GRP", None);

        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = HashSet::from(["one.piece.s02e01.1080p.web-dl.x264-grp".to_string()]);
        let context = AutoCandidateEvaluationContext {
            title: &live_action,
            subject: &subject,
            current_score: None,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            cutoff_reached: false,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_source_kinds: None,
        };
        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::DbBlocklisted
        );
    }

    #[test]
    fn unambiguous_title_demands_no_disambiguator() {
        // Friends is alone on its canonical key, so a bare scene release keeps
        // clearing the identity gate untouched.
        let mut title = make_title();
        title.id = "title-friends".to_string();
        title.name = "Friends".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        title.aliases = Vec::new();
        title.tagged_aliases = Vec::new();
        let library = vec![title.clone()];
        let subject = ambiguous_episode_subject(&title, &library, Some(9), Some(23));
        assert!(!subject.title_evidence.ambiguity.requires_disambiguator());

        let candidate = make_candidate("Friends.S09E23E24.1080p.BluRay.x264-TENEIGHTY", None);
        assert_eq!(
            decision_for(&title, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    // ── A2(2) + D2: indexer response attributes ─────────────────────────────

    fn series_episode_candidate(
        release_title: &str,
        response_attributes: IndexerResponseAttributes,
    ) -> IndexerSearchResult {
        let mut candidate = make_candidate(release_title, None);
        candidate.response_attributes = response_attributes;
        candidate
    }

    fn response_categories(categories: &[&str]) -> IndexerResponseAttributes {
        IndexerResponseAttributes {
            categories: categories.iter().map(|value| value.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn anime_only_response_category_vetoes_a_series_subject() {
        // D2 on the response lane: the indexer filed this under anime only, and
        // the wanted item is a plain series episode.
        let mut title = make_title();
        title.name = "Friends".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = series_episode_candidate(
            "Friends.S09E23E24.1080p.BluRay.x264-TENEIGHTY",
            response_categories(&["5070"]),
        );

        assert_eq!(
            decision_for(&title, &subject, &candidate),
            ReleaseAutoDecisionCode::CategoryMismatch
        );
    }

    #[test]
    fn dual_categorized_response_clears_the_category_gate() {
        // The set rule: `5000` is a plain-TV assertion the series subject
        // satisfies, so the additional `5070` is not a contradiction.
        let mut title = make_title();
        title.name = "Friends".to_string();
        title.facet = MediaFacet::Series;
        title.year = Some(1994);
        let subject = numbering_scoped_subject(&title, Some(9), Some(23));
        let candidate = series_episode_candidate(
            "Friends.S09E23E24.1080p.BluRay.x264-TENEIGHTY",
            response_categories(&["5000", "5070"]),
        );

        assert_eq!(
            decision_for(&title, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ambiguous_title_accepts_response_id_disambiguator() {
        // A2(2): the indexer asserts the live-action title's own TVDB id, which
        // per §9 decision 3 suffices on its own for a bare release name.
        let (live_action, library) = one_piece_library(Vec::new());
        let mut subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        subject.tvdb_id = Some("393199".to_string());
        let candidate = series_episode_candidate(
            "One.Piece.S02E01.1080p.WEB-DL.x264-GRP",
            IndexerResponseAttributes {
                tvdb_id: Some("393199".to_string()),
                ..Default::default()
            },
        );

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::QualityBlocked
        );
    }

    #[test]
    fn ambiguous_title_without_response_ids_stays_ambiguous() {
        // Same subject, same release name — only the indexer's id assertion is
        // missing, and absence is not a disambiguator.
        let (live_action, library) = one_piece_library(Vec::new());
        let mut subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        subject.tvdb_id = Some("393199".to_string());
        let candidate = make_candidate("One.Piece.S02E01.1080p.WEB-DL.x264-GRP", None);

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::AmbiguousIdentity
        );
    }

    #[test]
    fn category_contradiction_outranks_identity_ambiguity() {
        // The incident release is both anime-categorized and identity-ambiguous.
        // The category is the sharper, more actionable reason, so it reports
        // first.
        let (live_action, library) = one_piece_library(Vec::new());
        let subject = ambiguous_episode_subject(&live_action, &library, Some(2), Some(1));
        let candidate = series_episode_candidate(
            "One.Piece.S02E01.1080p.WEB-DL.x264-GRP",
            response_categories(&["5070"]),
        );

        assert_eq!(
            decision_for(&live_action, &subject, &candidate),
            ReleaseAutoDecisionCode::CategoryMismatch
        );
    }

    #[test]
    fn external_id_agreement_reports_only_comparable_kinds() {
        let response = IndexerResponseAttributes {
            tvdb_id: Some(" 393199 ".to_string()),
            imdb_id: Some("14688458".to_string()),
            ..Default::default()
        };

        assert_eq!(
            external_id_agreement(&response, Some("393199"), None, None),
            Some(true),
            "a trimmed numeric id agrees with the subject's own"
        );
        assert_eq!(
            external_id_agreement(&response, None, None, Some("tt14688458")),
            Some(true),
            "imdb ids agree once both sides are normalized"
        );
        assert_eq!(
            external_id_agreement(&response, Some("81189"), None, None),
            Some(false),
            "a comparable id that disagrees is a disagreement"
        );
        assert_eq!(
            external_id_agreement(&response, None, Some("140342"), None),
            None,
            "the indexer asserted no tmdb id, so there is nothing to compare"
        );
        assert_eq!(
            external_id_agreement(
                &IndexerResponseAttributes::default(),
                Some("393199"),
                None,
                None
            ),
            None,
            "a response with no ids is not evidence either way"
        );
    }

    #[test]
    fn candidate_matches_title_subject_uses_contextual_alias_parse_when_needed() {
        let mut title = make_title();
        title.name = "Silver Horizon Beyond the Vale".to_string();
        title.year = Some(2023);
        title.aliases = vec!["Sora no Vale".to_string()];
        title.tagged_aliases = vec![TaggedAlias {
            name: "Silver Horizon Beyond the Vale".to_string(),
            language: "eng".to_string(),
        }];

        let candidate = make_candidate(
            "[SubsPlease] Sora.no.Vale.Silver.Horizon.Beyond.the.Vale.-.01.[1080p].[HEVC]",
            None,
        );

        assert!(candidate_matches_title_subject(
            &candidate,
            &canonical_title_evidence(&title)
        ));
    }

    #[test]
    fn candidate_parse_state_marks_ambiguous_parse() {
        let mut candidate = make_candidate("Nightfall.S01E01.1080p.WEB-DL", None);
        let parsed = candidate
            .parsed_release_metadata
            .as_mut()
            .expect("candidate has parsed metadata");
        parsed.is_ambiguous = true;
        parsed.disposition = ParseDisposition::Ambiguous;
        parsed.parse_hints.push("v2:ambiguous".to_string());

        assert_eq!(
            candidate_parse_state(&candidate),
            CandidateParseState::Ambiguous
        );
    }

    #[test]
    fn candidate_matches_existing_media_file_for_same_episode_release() {
        let candidate = make_candidate("Nightfall.S01E01.1080p.WEB-DL", None);
        let existing = vec![make_media_file(
            "Nightfall.S01E01.1080p.WEB-DL",
            Some("episode-1"),
        )];

        assert!(candidate_matches_existing_media_file(
            &candidate,
            &existing,
            Some("episode-1")
        ));
        assert!(!candidate_matches_existing_media_file(
            &candidate,
            &existing,
            Some("episode-2")
        ));
    }

    #[test]
    fn analyzed_cutoff_quality_matches_the_current_scope() {
        let mut title_file = make_media_file("Nightfall.2022.1080p.WEB-DL", None);
        title_file.quality_label = Some("1080p".to_string());
        title_file.acquisition_score = Some(900);
        let episode_file = make_media_file("Nightfall.S01E01.1080p.WEB-DL", Some("episode-1"));
        let existing = vec![title_file, episode_file];

        assert_eq!(
            crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
                &existing,
                Some("episode-1"),
                None,
            ),
            Some("720p")
        );
        assert_eq!(
            crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
                &existing, None, None,
            ),
            Some("1080p")
        );
    }

    #[test]
    fn completed_current_score_suppresses_stale_grabbed_release_cutoff() {
        let completed = make_wanted_item(
            AcquisitionScopeStatus::Completed,
            Some(1200),
            Some(r#"{"title":"Nightfall.2022.1080p.WEB-DL"}"#),
        );
        assert_eq!(grabbed_release_for_search_subject(&completed), None);

        let grabbed = make_wanted_item(
            AcquisitionScopeStatus::Grabbed,
            Some(1200),
            Some(r#"{"title":"Nightfall.2022.1080p.WEB-DL"}"#),
        );
        assert_eq!(
            grabbed_release_for_search_subject(&grabbed),
            Some(r#"{"title":"Nightfall.2022.1080p.WEB-DL"}"#.to_string())
        );

        let title = make_title();
        let mut candidate = make_candidate("Nightfall.2022.1080p.WEB-DL", None);
        candidate.quality_profile_decision = Some(allowed_quality_decision(2400));
        let subject = ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(&title),
            queries: vec!["Nightfall".to_string()],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category: title.facet.as_str().to_string(),
            owner_facet: title.facet.clone(),
            search_facet: title.facet.clone(),
            id_search_facet: None,
            newznab_categories: Vec::new(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            current_score: completed.current_score,
            last_search_at: None,
            grabbed_release: grabbed_release_for_search_subject(&completed),
            submission_scope: SubmissionScope::Title,
        };
        let profile = QualityProfile::default();
        let thresholds = AcquisitionThresholds::default();
        let now = Utc::now();
        let db_blocklist = HashSet::new();
        let context = AutoCandidateEvaluationContext {
            title: &title,
            subject: &subject,
            current_score: subject.current_score,
            last_search_at: None,
            profile: &profile,
            thresholds: &thresholds,
            cutoff_reached: false,
            now: &now,
            dl_snapshot: None,
            db_blocklist: &db_blocklist,
            existing_files: &[],
            delay_profiles: &[],
            failed_source_kinds: None,
        };

        assert_eq!(
            evaluate_auto_candidate(&candidate, &context),
            ReleaseAutoDecisionCode::Eligible
        );
    }
}
