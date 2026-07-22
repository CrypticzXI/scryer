use std::collections::HashMap;
use std::sync::LazyLock;

use crate::scoring_weights::ScoringWeights;

/// Reputation tier for a release group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupTier {
    /// Top-tier groups (e.g. TRaSH Tier 01 WEB, Tier 01 Remux)
    Gold,
    /// Great groups (e.g. TRaSH Tier 02)
    Silver,
    /// Good groups (e.g. TRaSH Tier 03)
    Bronze,
    /// Known-bad groups (LQ, bad dual audio)
    Banned,
}

/// What source context a group is known for.
/// A group might be Gold for WEB but unknown for BluRay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceContext {
    Web,
    BluRay,
    UhdBluRay,
    Remux,
    Anime,
    /// Applies regardless of source (e.g. banned groups).
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleFacet {
    Movie,
    Series,
    Anime,
}

#[derive(Debug, Clone, Copy)]
pub struct GroupEntry {
    #[allow(dead_code)]
    pub name: &'static str,
    pub tier: GroupTier,
    pub facet: RuleFacet,
    pub source_context: SourceContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMatchKind {
    Exact,
    Prefix,
}

#[derive(Debug, Clone, Copy)]
pub struct GroupRule {
    pub matcher: &'static str,
    pub match_kind: GroupMatchKind,
    pub entry: GroupEntry,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct TrashGuideRuleMetadata {
    pub matcher: &'static str,
    pub match_kind: GroupMatchKind,
    pub tier: GroupTier,
    pub facet: RuleFacet,
    pub source_context: SourceContext,
    pub app: &'static str,
    pub stem: &'static str,
    pub trash_id: &'static str,
    pub cf_name: &'static str,
    pub spec_name: &'static str,
    pub source_path: &'static str,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct MetadataRuleRecord {
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

include!("trash_guides_release_groups.generated.rs");

struct GroupRuleIndex {
    exact: HashMap<String, Vec<usize>>,
    prefixes: Vec<usize>,
}

static GROUP_RULE_INDEX: LazyLock<GroupRuleIndex> = LazyLock::new(|| {
    let mut exact = HashMap::<String, Vec<usize>>::new();
    let mut prefixes = Vec::new();
    for (index, rule) in GROUP_RULES.iter().enumerate() {
        match rule.match_kind {
            GroupMatchKind::Exact => exact
                .entry(rule.matcher.to_ascii_uppercase())
                .or_default()
                .push(index),
            GroupMatchKind::Prefix => prefixes.push(index),
        }
    }
    GroupRuleIndex { exact, prefixes }
});

/// Look up a release group's tier, considering source context.
///
/// Strategy:
/// 1. Try exact match on (name, source_context) derived from the release
/// 2. Fall back to (name, Any) for groups that are tier-rated regardless of source
/// 3. No match → None (caller applies `group_unknown_penalty`)
pub fn lookup_group(
    name: &str,
    source: Option<&str>,
    quality: Option<&str>,
    is_remux: bool,
    category_hint: Option<&str>,
) -> Option<&'static GroupEntry> {
    let name_upper = name.to_ascii_uppercase();
    let facets = candidate_facets(category_hint);
    let ctx = source_to_context(source, quality, is_remux, category_hint);

    for facet in facets {
        // Try source-specific match first
        if let Some(rule) = indexed_group_rule(&name_upper, *facet, ctx) {
            return Some(&rule.entry);
        }

        // Fall back to Any context (banned groups, etc.).
        if let Some(rule) = indexed_group_rule(&name_upper, *facet, SourceContext::Any) {
            return Some(&rule.entry);
        }
    }

    None
}

fn indexed_group_rule(
    candidate: &str,
    facet: RuleFacet,
    context: SourceContext,
) -> Option<&'static GroupRule> {
    let exact_index = GROUP_RULE_INDEX.exact.get(candidate).and_then(|indices| {
        indices.iter().copied().find(|index| {
            let entry = &GROUP_RULES[*index].entry;
            entry.facet == facet && entry.source_context == context
        })
    });
    let prefix_index = GROUP_RULE_INDEX.prefixes.iter().copied().find(|index| {
        let rule = &GROUP_RULES[*index];
        rule.entry.facet == facet
            && rule.entry.source_context == context
            && group_rule_matches(rule, candidate)
    });

    match (exact_index, prefix_index) {
        (Some(exact), Some(prefix)) => Some(&GROUP_RULES[exact.min(prefix)]),
        (Some(index), None) | (None, Some(index)) => Some(&GROUP_RULES[index]),
        (None, None) => None,
    }
}

fn candidate_facets(category_hint: Option<&str>) -> &'static [RuleFacet] {
    match category_hint
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("anime") => &[RuleFacet::Anime],
        Some("series") => &[RuleFacet::Series],
        Some("movie") => &[RuleFacet::Movie],
        _ => &[RuleFacet::Movie, RuleFacet::Series],
    }
}

/// Map a parsed source string + remux flag to our SourceContext.
fn source_to_context(
    source: Option<&str>,
    quality: Option<&str>,
    is_remux: bool,
    category_hint: Option<&str>,
) -> SourceContext {
    if matches!(
        category_hint
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("anime")
    ) {
        return SourceContext::Anime;
    }
    if is_remux {
        return SourceContext::Remux;
    }
    match source.map(|value| value.trim().to_ascii_uppercase()) {
        Some(value) if matches!(value.as_str(), "WEB-DL" | "WEBRIP") => SourceContext::Web,
        Some(value) if matches!(value.as_str(), "BLURAY" | "BRDISK") => {
            if quality.is_some_and(|value| value.trim().eq_ignore_ascii_case("2160P")) {
                SourceContext::UhdBluRay
            } else {
                SourceContext::BluRay
            }
        }
        Some(value) if value == "RAWHD" => SourceContext::Any,
        _ => SourceContext::Any,
    }
}

fn group_rule_matches(rule: &GroupRule, candidate: &str) -> bool {
    match rule.match_kind {
        GroupMatchKind::Exact => rule.matcher.eq_ignore_ascii_case(candidate),
        GroupMatchKind::Prefix => candidate
            .get(..rule.matcher.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(rule.matcher)),
    }
}

/// Apply release group scoring to a decision.
///
/// Uses the group database to look up the release group's tier for its source
/// context, then applies the corresponding weight from the persona.
#[cfg(test)]
pub fn apply_release_group_scoring(
    weights: &ScoringWeights,
    group: Option<&str>,
    source: Option<&str>,
    is_remux: bool,
) -> (&'static str, i32) {
    apply_release_group_scoring_with_context(weights, group, source, None, is_remux, None)
}

pub fn apply_release_group_scoring_with_context(
    weights: &ScoringWeights,
    group: Option<&str>,
    source: Option<&str>,
    quality: Option<&str>,
    is_remux: bool,
    category_hint: Option<&str>,
) -> (&'static str, i32) {
    let Some(name) = group else {
        return ("group_unknown", weights.group_unknown_penalty);
    };

    if name.is_empty() {
        return ("group_unknown", weights.group_unknown_penalty);
    }

    match lookup_group(name, source, quality, is_remux, category_hint) {
        Some(entry) => match entry.tier {
            GroupTier::Gold => ("group_gold", weights.group_gold),
            GroupTier::Silver => ("group_silver", weights.group_silver),
            GroupTier::Bronze => ("group_bronze", weights.group_bronze),
            GroupTier::Banned => ("group_banned", weights.group_banned),
        },
        None => ("group_unknown", weights.group_unknown_penalty),
    }
}

#[cfg(test)]
#[path = "release_group_db_tests.rs"]
mod release_group_db_tests;
