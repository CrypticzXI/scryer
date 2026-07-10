use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::model::{GuideFact, ParsedReleaseMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenPatternKind {
    Sequence,
    RequiredTokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenPattern {
    pub kind: TokenPatternKind,
    pub tokens: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ParserSignalKind {
    AiEnhanced,
    Proper,
    Repack,
    DubsOnly,
    HardcodedSubs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleFacet {
    Movie,
    Series,
    Anime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ServiceAliasRule {
    pub token: &'static str,
    pub service: &'static str,
    pub facet: RuleFacet,
    pub app: &'static str,
    pub stem: &'static str,
    pub trash_id: &'static str,
    pub cf_name: &'static str,
    pub spec_name: &'static str,
    pub source_path: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenSignalRule {
    pub kind: ParserSignalKind,
    pub pattern: TokenPattern,
    pub facet: RuleFacet,
    pub app: &'static str,
    pub stem: &'static str,
    pub trash_id: &'static str,
    pub cf_name: &'static str,
    pub spec_name: &'static str,
    pub source_path: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleCategoryScope {
    Any,
    Anime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockedTitleRule {
    pub code: &'static str,
    pub facet: RuleFacet,
    pub category: TitleCategoryScope,
    pub pattern: TokenPattern,
    pub app: &'static str,
    pub stem: &'static str,
    pub trash_id: &'static str,
    pub cf_name: &'static str,
    pub spec_name: &'static str,
    pub source_path: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FactRule {
    pub code: &'static str,
    pub facet: RuleFacet,
    pub category: TitleCategoryScope,
    pub pattern: TokenPattern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocaleGroupMatchKind {
    Exact,
    #[allow(dead_code)]
    Prefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocaleSourceContext {
    Web,
    BluRay,
    UhdBluRay,
    Remux,
    Anime,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocaleGroupFactRule {
    pub code: &'static str,
    pub matcher: &'static str,
    pub match_kind: LocaleGroupMatchKind,
    pub facet: RuleFacet,
    pub source_context: LocaleSourceContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct MetadataRuleRecord {
    pub app: &'static str,
    pub facet: RuleFacet,
    pub stem: &'static str,
    pub trash_id: &'static str,
    pub cf_name: &'static str,
    pub spec_name: &'static str,
    pub implementation: &'static str,
    pub value: &'static str,
    pub trash_scores_json: &'static str,
    pub required_json: &'static str,
    pub negate_json: &'static str,
    pub complete_json: &'static str,
    pub reason: &'static str,
    pub source_path: &'static str,
}

include!("trash_guides_parser_knowledge.generated.rs");

#[derive(Debug, Default)]
struct TokenAnchorIndex {
    rules_by_anchor: BTreeMap<&'static str, Vec<usize>>,
}

impl TokenAnchorIndex {
    fn from_patterns(patterns: impl Iterator<Item = (usize, &'static TokenPattern)>) -> Self {
        let mut rules_by_anchor = BTreeMap::<&'static str, Vec<usize>>::new();
        for (index, pattern) in patterns {
            let Some(anchor) = pattern_anchor(pattern) else {
                continue;
            };
            rules_by_anchor.entry(anchor).or_default().push(index);
        }
        Self { rules_by_anchor }
    }

    fn candidate_indices(&self, normalized_tokens: &[String]) -> Vec<usize> {
        let mut indices = BTreeSet::new();
        for token in normalized_tokens {
            if let Some(matches) = self.rules_by_anchor.get(token.as_str()) {
                indices.extend(matches.iter().copied());
            }
        }
        indices.into_iter().collect()
    }
}

fn pattern_anchor(pattern: &TokenPattern) -> Option<&'static str> {
    match pattern.kind {
        TokenPatternKind::Sequence => pattern.tokens.first().copied(),
        TokenPatternKind::RequiredTokens => pattern.tokens.iter().copied().min(),
    }
}

fn token_signal_index() -> &'static TokenAnchorIndex {
    static INDEX: OnceLock<TokenAnchorIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        TokenAnchorIndex::from_patterns(
            TOKEN_SIGNAL_RULES
                .iter()
                .enumerate()
                .map(|(index, rule)| (index, &rule.pattern)),
        )
    })
}

fn blocked_title_index() -> &'static TokenAnchorIndex {
    static INDEX: OnceLock<TokenAnchorIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        TokenAnchorIndex::from_patterns(
            BLOCKED_TITLE_RULES
                .iter()
                .enumerate()
                .map(|(index, rule)| (index, &rule.pattern)),
        )
    })
}

fn fact_index() -> &'static TokenAnchorIndex {
    static INDEX: OnceLock<TokenAnchorIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        TokenAnchorIndex::from_patterns(
            FACT_RULES
                .iter()
                .enumerate()
                .map(|(index, rule)| (index, &rule.pattern)),
        )
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenSignalMatch {
    pub ai_enhanced: bool,
    pub proper: bool,
    pub repack: bool,
    pub dubs_only: bool,
    pub hardcoded_subs: bool,
}

pub(crate) fn normalize_streaming_service_alias(token: &str) -> Option<&'static str> {
    SERVICE_ALIAS_RULES
        .iter()
        .find(|rule| rule.token.eq_ignore_ascii_case(token))
        .map(|rule| rule.service)
}

pub(crate) fn detect_token_signals(normalized_tokens: &[String]) -> TokenSignalMatch {
    let mut matched = TokenSignalMatch::default();
    for index in token_signal_index().candidate_indices(normalized_tokens) {
        let rule = &TOKEN_SIGNAL_RULES[index];
        if !pattern_matches(&rule.pattern, normalized_tokens) {
            continue;
        }
        match rule.kind {
            ParserSignalKind::AiEnhanced => matched.ai_enhanced = true,
            ParserSignalKind::Proper => matched.proper = true,
            ParserSignalKind::Repack => {
                matched.proper = true;
                matched.repack = true;
            }
            ParserSignalKind::DubsOnly => matched.dubs_only = true,
            ParserSignalKind::HardcodedSubs => matched.hardcoded_subs = true,
        }
    }
    matched
}

pub(crate) fn derive_facts(raw_title: &str, category_hint: Option<&str>) -> Vec<GuideFact> {
    let tokens = normalize_raw_title_tokens(raw_title);
    derive_facts_from_tokens(&tokens, category_hint)
}

pub(crate) fn derive_locale_group_facts(
    projected: &ParsedReleaseMetadata,
    category_hint: Option<&str>,
) -> Vec<GuideFact> {
    let Some(release_group) = projected.release_group.as_deref() else {
        return Vec::new();
    };
    let facet = normalize_facet(category_hint);
    let mut codes = BTreeSet::new();
    for rule in LOCALE_GROUP_FACT_RULES {
        if rule.facet == facet
            && locale_group_matches(rule, release_group)
            && locale_source_context_matches(rule.source_context, projected)
        {
            codes.insert(rule.code);
        }
    }
    codes
        .into_iter()
        .map(|code| GuideFact {
            code: code.to_string(),
        })
        .collect()
}

pub(crate) fn derive_structural_facts(
    projected: &ParsedReleaseMetadata,
    category_hint: Option<&str>,
) -> Vec<GuideFact> {
    if projected.release_group.is_some() {
        return Vec::new();
    }
    let facet = normalize_facet(category_hint);
    NO_RELEASE_GROUP_FACT_FACETS
        .contains(&facet)
        .then(|| GuideFact {
            code: "trash.no_release_group".to_string(),
        })
        .into_iter()
        .collect()
}

fn locale_group_matches(rule: &LocaleGroupFactRule, release_group: &str) -> bool {
    match rule.match_kind {
        LocaleGroupMatchKind::Exact => rule.matcher.eq_ignore_ascii_case(release_group),
        LocaleGroupMatchKind::Prefix => release_group
            .get(..rule.matcher.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(rule.matcher)),
    }
}

fn locale_source_context_matches(
    source_context: LocaleSourceContext,
    projected: &ParsedReleaseMetadata,
) -> bool {
    match source_context {
        LocaleSourceContext::Any | LocaleSourceContext::Anime => true,
        LocaleSourceContext::Web => matches!(
            projected.source,
            Some(crate::model::ReleaseSource::WebDl | crate::model::ReleaseSource::WebRip)
        ),
        LocaleSourceContext::Remux => projected.is_remux,
        LocaleSourceContext::BluRay => {
            matches!(projected.source, Some(crate::model::ReleaseSource::BluRay))
                && !projected.is_remux
                && !is_uhd_quality(projected.quality.as_deref())
        }
        LocaleSourceContext::UhdBluRay => {
            matches!(projected.source, Some(crate::model::ReleaseSource::BluRay))
                && is_uhd_quality(projected.quality.as_deref())
        }
    }
}

fn is_uhd_quality(quality: Option<&str>) -> bool {
    quality.is_some_and(|quality| quality.contains("2160"))
}

fn derive_facts_from_tokens(
    normalized_tokens: &[String],
    category_hint: Option<&str>,
) -> Vec<GuideFact> {
    let scope = normalize_scope(category_hint);
    let facet = normalize_facet(category_hint);
    let mut codes = BTreeSet::new();

    for index in token_signal_index().candidate_indices(normalized_tokens) {
        let rule = &TOKEN_SIGNAL_RULES[index];
        if !pattern_matches(&rule.pattern, normalized_tokens) {
            continue;
        }
        match rule.kind {
            ParserSignalKind::AiEnhanced => {
                codes.insert("trash.ai_enhanced");
            }
            ParserSignalKind::Proper => {
                codes.insert("trash.proper");
            }
            ParserSignalKind::Repack => {
                codes.insert("trash.proper");
                codes.insert("trash.repack");
            }
            ParserSignalKind::DubsOnly => {
                codes.insert("trash.dubs_only");
            }
            ParserSignalKind::HardcodedSubs => {
                codes.insert("trash.hardcoded_subs");
            }
        }
    }

    for index in blocked_title_index().candidate_indices(normalized_tokens) {
        let rule = &BLOCKED_TITLE_RULES[index];
        if rule_applies(rule.facet, rule.category, facet, scope)
            && pattern_matches(&rule.pattern, normalized_tokens)
        {
            codes.insert(blocked_fact_code(rule.code));
        }
    }

    for index in fact_index().candidate_indices(normalized_tokens) {
        let rule = &FACT_RULES[index];
        if rule_applies(rule.facet, rule.category, facet, scope)
            && pattern_matches(&rule.pattern, normalized_tokens)
        {
            codes.insert(rule.code);
        }
    }

    codes
        .into_iter()
        .map(|code| GuideFact {
            code: code.to_string(),
        })
        .collect()
}

pub(crate) fn project_safe_facts(projected: &mut ParsedReleaseMetadata) {
    for fact in &projected.guide_facts {
        match fact.code.as_str() {
            "trash.ai_enhanced" => projected.is_ai_enhanced = true,
            "trash.proper" => projected.is_proper_upload = true,
            "trash.repack" => {
                projected.is_proper_upload = true;
                projected.is_repack = true;
            }
            "trash.hardcoded_subs" => projected.is_hardcoded_subs = true,
            _ => {}
        }
    }
}

pub fn detect_blocked_title(raw_title: &str, category_hint: Option<&str>) -> Option<&'static str> {
    let tokens = normalize_raw_title_tokens(raw_title);
    detect_blocked_title_tokens(&tokens, category_hint)
}

pub(crate) fn detect_blocked_title_tokens(
    normalized_tokens: &[String],
    category_hint: Option<&str>,
) -> Option<&'static str> {
    let scope = normalize_scope(category_hint);
    let facet = normalize_facet(category_hint);
    blocked_title_index()
        .candidate_indices(normalized_tokens)
        .into_iter()
        .map(|index| &BLOCKED_TITLE_RULES[index])
        .find(|rule| {
            rule_applies(rule.facet, rule.category, facet, scope)
                && pattern_matches(&rule.pattern, normalized_tokens)
        })
        .map(|rule| rule.code)
}

fn rule_applies(
    rule_facet: RuleFacet,
    rule_category: TitleCategoryScope,
    facet: RuleFacet,
    scope: TitleCategoryScope,
) -> bool {
    rule_facet == facet
        && (matches!(rule_category, TitleCategoryScope::Any)
            || matches!(scope, TitleCategoryScope::Anime)
                && matches!(rule_category, TitleCategoryScope::Anime))
}

fn blocked_fact_code(code: &str) -> &'static str {
    match code {
        "trash_guides_anime_raws" => "trash.blocked.anime_raws",
        "trash_guides_lq_release_title" => "trash.blocked.lq_release_title",
        "trash_guides_fansub" => "trash.blocked.fansub",
        "trash_guides_fastsub" => "trash.blocked.fastsub",
        _ => "trash.blocked.legacy",
    }
}

fn normalize_facet(category_hint: Option<&str>) -> RuleFacet {
    match category_hint
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("anime") => RuleFacet::Anime,
        Some("series") => RuleFacet::Series,
        _ => RuleFacet::Movie,
    }
}

fn normalize_scope(category_hint: Option<&str>) -> TitleCategoryScope {
    match category_hint
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("anime") => TitleCategoryScope::Anime,
        _ => TitleCategoryScope::Any,
    }
}

fn normalize_raw_title_tokens(raw_title: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in raw_title.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_uppercase());
        } else if !current.is_empty() {
            tokens.push(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn pattern_matches(pattern: &TokenPattern, normalized_tokens: &[String]) -> bool {
    match pattern.kind {
        TokenPatternKind::Sequence => {
            normalized_tokens
                .windows(pattern.tokens.len())
                .any(|window| {
                    window
                        .iter()
                        .map(String::as_str)
                        .eq(pattern.tokens.iter().copied())
                })
        }
        TokenPatternKind::RequiredTokens => pattern
            .tokens
            .iter()
            .all(|token| normalized_tokens.iter().any(|candidate| candidate == token)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_generated_streaming_alias_and_token_signals() {
        assert_eq!(normalize_streaming_service_alias("MAX"), Some("HBO Max"));

        let tokens = vec![
            "THE".to_string(),
            "UPSCALER".to_string(),
            "REPACK".to_string(),
        ];
        let matched = detect_token_signals(&tokens);
        assert!(matched.ai_enhanced);
        assert!(matched.proper);
        assert!(matched.repack);
    }

    #[test]
    fn detects_blocked_titles_with_category_scopes() {
        assert_eq!(
            detect_blocked_title("Series.Name.2160p.BiTOR.WEB-DL", Some("series")),
            Some("trash_guides_lq_release_title")
        );
        assert_eq!(
            detect_blocked_title("Series.Name.2160p.BiTOR.WEB-DL", None),
            None
        );
        assert_eq!(
            detect_blocked_title("[Asuka-Raws] Anime Episode 01", Some("anime")),
            Some("trash_guides_anime_raws")
        );
        assert_eq!(
            detect_blocked_title("[Asuka-Raws] Anime Episode 01", None),
            None
        );

        let facts = derive_facts("Series.Name.2160p.BiTOR.WEB-DL", Some("series"));
        assert!(
            facts
                .iter()
                .any(|fact| fact.code == "trash.blocked.lq_release_title")
        );
    }

    #[test]
    fn anchor_index_matches_reference_rule_scans() {
        for tokens in [
            vec!["THE", "UPSCALER", "REPACK"],
            vec!["2160P", "BITOR", "WEB", "DL"],
            vec!["ASUKA", "RAWS", "1080P"],
            vec!["UNRELATED", "TITLE"],
        ] {
            let tokens = tokens.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(
                detect_token_signals(&tokens),
                detect_token_signals_reference(&tokens)
            );
            assert_eq!(
                detect_blocked_title_tokens(&tokens, Some("anime")),
                detect_blocked_title_tokens_reference(&tokens, Some("anime"))
            );
        }
    }

    fn detect_token_signals_reference(normalized_tokens: &[String]) -> TokenSignalMatch {
        let mut matched = TokenSignalMatch::default();
        for rule in TOKEN_SIGNAL_RULES {
            if !pattern_matches(&rule.pattern, normalized_tokens) {
                continue;
            }
            match rule.kind {
                ParserSignalKind::AiEnhanced => matched.ai_enhanced = true,
                ParserSignalKind::Proper => matched.proper = true,
                ParserSignalKind::Repack => {
                    matched.proper = true;
                    matched.repack = true;
                }
                ParserSignalKind::DubsOnly => matched.dubs_only = true,
                ParserSignalKind::HardcodedSubs => matched.hardcoded_subs = true,
            }
        }
        matched
    }

    fn detect_blocked_title_tokens_reference(
        normalized_tokens: &[String],
        category_hint: Option<&str>,
    ) -> Option<&'static str> {
        let scope = normalize_scope(category_hint);
        let facet = normalize_facet(category_hint);
        BLOCKED_TITLE_RULES
            .iter()
            .find(|rule| {
                rule_applies(rule.facet, rule.category, facet, scope)
                    && pattern_matches(&rule.pattern, normalized_tokens)
            })
            .map(|rule| rule.code)
    }
}
