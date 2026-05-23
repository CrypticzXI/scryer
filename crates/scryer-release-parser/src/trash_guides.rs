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
    pub reason: &'static str,
    pub source_path: &'static str,
}

include!("trash_guides_parser_knowledge.generated.rs");

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
    BLOCKED_TITLE_RULES
        .iter()
        .find(|rule| {
            rule.facet == facet
                && (matches!(rule.category, TitleCategoryScope::Any)
                    || matches!(scope, TitleCategoryScope::Anime)
                        && matches!(rule.category, TitleCategoryScope::Anime))
                && pattern_matches(&rule.pattern, normalized_tokens)
        })
        .map(|rule| rule.code)
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
            "REPACK2".to_string(),
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
    }
}
