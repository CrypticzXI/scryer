use chrono::NaiveDate;
use scryer_domain::{Collection, Episode, MediaFacet, Title};

pub use scryer_release_parser::{
    ContextAlias, ContextEpisode, ContextFacetHint, ContextTitle, ParseDisposition,
    ParsedEpisodeMetadata, ParsedEpisodeReleaseType, ParsedReleaseMetadata, ParsedSpecialKind,
    ReleaseParseAnalysis, ReleaseParseContext, TargetedReleaseParseAnalysis, VideoCodec,
    analyze_release_against_targets, analyze_release_for_target, best_parse_for_target,
};

pub fn parse_release_metadata(raw: &str) -> ParsedReleaseMetadata {
    let context = synthesize_release_parse_context(raw);
    project_analysis(raw, &analyze_release_for_target(raw, &context))
}

pub fn parse_release_metadata_for_target(
    raw: &str,
    context: &ReleaseParseContext,
) -> ParsedReleaseMetadata {
    project_analysis(raw, &analyze_release_for_target(raw, context))
}

pub fn build_release_parse_context(
    title: &Title,
    episode: Option<&Episode>,
    _collection: Option<&Collection>,
    facet_hint: Option<&str>,
) -> ReleaseParseContext {
    build_release_parse_context_from_episodes(title, episode, facet_hint)
}

pub fn build_release_parse_context_for_title(
    title: &Title,
    episodes: &[Episode],
    facet_hint: Option<&str>,
) -> ReleaseParseContext {
    build_release_parse_context_from_episodes(title, episodes.iter(), facet_hint)
}

fn build_release_parse_context_from_episodes<'a>(
    title: &Title,
    episodes: impl IntoIterator<Item = &'a Episode>,
    facet_hint: Option<&str>,
) -> ReleaseParseContext {
    let aliases = title
        .aliases
        .iter()
        .map(|alias| ContextAlias {
            name: alias.clone(),
        })
        .chain(title.tagged_aliases.iter().map(|alias| ContextAlias {
            name: alias.name.clone(),
        }))
        .collect::<Vec<_>>();

    let imdb_ids = title
        .external_ids
        .iter()
        .filter(|external_id| external_id.source.eq_ignore_ascii_case("imdb"))
        .map(|external_id| external_id.value.clone())
        .collect::<Vec<_>>();

    let episodes = episodes
        .into_iter()
        .map(domain_episode_to_parse_context)
        .collect::<Vec<_>>();

    ReleaseParseContext {
        facet_hint: parse_context_facet_hint(facet_hint, &title.facet),
        title: ContextTitle {
            name: title.name.clone(),
        },
        aliases,
        known_years: title.year.into_iter().collect(),
        imdb_ids,
        episodes,
    }
}

fn domain_episode_to_parse_context(episode: &Episode) -> ContextEpisode {
    ContextEpisode {
        season: episode
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok()),
        episode: episode
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok()),
        absolute_number: episode
            .absolute_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok()),
        air_date: episode
            .air_date
            .as_deref()
            .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()),
        title: episode.title.clone(),
        title_aliases: episode.title.clone().into_iter().collect(),
    }
}

pub fn build_candidate_bank_contexts<'a>(
    titles: impl IntoIterator<Item = &'a Title>,
    episode: Option<&Episode>,
    collection: Option<&Collection>,
    facet_hint: Option<&str>,
    limit: usize,
) -> Vec<ReleaseParseContext> {
    titles
        .into_iter()
        .take(limit)
        .map(|title| build_release_parse_context(title, episode, collection, facet_hint))
        .collect()
}

fn parse_context_facet_hint(facet_hint: Option<&str>, facet: &MediaFacet) -> ContextFacetHint {
    match facet_hint.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.eq_ignore_ascii_case("movie") => ContextFacetHint::Movie,
        Some(value) if value.eq_ignore_ascii_case("anime") => ContextFacetHint::Anime,
        Some(value) if value.eq_ignore_ascii_case("series") || value.eq_ignore_ascii_case("tv") => {
            ContextFacetHint::Series
        }
        _ => match facet {
            MediaFacet::Movie => ContextFacetHint::Movie,
            MediaFacet::Series => ContextFacetHint::Series,
            MediaFacet::Anime => ContextFacetHint::Anime,
        },
    }
}

fn project_analysis(raw: &str, analysis: &ReleaseParseAnalysis) -> ParsedReleaseMetadata {
    let mut projected = analysis
        .best_candidate()
        .map(|candidate| candidate.projected.clone())
        .unwrap_or_else(|| ParsedReleaseMetadata::empty(raw, analysis.parser_version));

    projected.ambiguity_margin = analysis.ambiguity_margin;
    projected.is_ambiguous = analysis.is_ambiguous;
    projected.disposition = analysis.disposition;
    projected.scoring_model_version = analysis.scoring_model_version;

    merge_analysis_hints(&mut projected, analysis);
    projected
}

fn merge_analysis_hints(parsed: &mut ParsedReleaseMetadata, analysis: &ReleaseParseAnalysis) {
    for hint in &analysis.parse_hints {
        push_unique_hint(&mut parsed.parse_hints, hint.clone());
    }

    push_unique_hint(
        &mut parsed.parse_hints,
        format!(
            "parse_status:{}",
            match parsed.disposition {
                ParseDisposition::Parsed => "parsed",
                ParseDisposition::Ambiguous => "ambiguous",
                ParseDisposition::Unparseable => "unparseable",
            }
        ),
    );
    push_unique_hint(
        &mut parsed.parse_hints,
        format!("parse_ambiguity_margin:{}", analysis.ambiguity_margin),
    );
    push_unique_hint(
        &mut parsed.parse_hints,
        format!("scoring_model_version:{}", analysis.scoring_model_version),
    );
    if parsed.is_ambiguous {
        push_unique_hint(&mut parsed.parse_hints, "v2:ambiguous".to_string());
    }
    if matches!(parsed.disposition, ParseDisposition::Unparseable) {
        push_unique_hint(&mut parsed.parse_hints, "v2:unparseable".to_string());
    }
}

fn push_unique_hint(hints: &mut Vec<String>, value: String) {
    if !hints.iter().any(|existing| existing == &value) {
        hints.push(value);
    }
}

fn synthesize_release_parse_context(raw: &str) -> ReleaseParseContext {
    let tokens = raw
        .split(|ch: char| {
            matches!(
                ch,
                '.' | '_' | ' ' | '-' | '/' | '[' | ']' | '(' | ')' | '{' | '}'
            )
        })
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    let title_start = usize::from(raw.trim_start().starts_with('[') && tokens.len() > 1);
    let title_boundary = tokens
        .iter()
        .enumerate()
        .skip(title_start)
        .find(|(_, token)| looks_like_release_metadata_token(token, &tokens))
        .map(|(index, _)| index)
        .unwrap_or(tokens.len());
    let title_tokens = tokens
        .iter()
        .skip(title_start)
        .take(title_boundary.saturating_sub(title_start))
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    let title = if title_tokens.is_empty() {
        tokens
            .get(title_start)
            .or_else(|| tokens.first())
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        title_tokens.join(" ")
    };

    ReleaseParseContext {
        facet_hint: guess_context_facet(raw, &tokens),
        title: ContextTitle { name: title },
        aliases: Vec::new(),
        known_years: tokens
            .iter()
            .filter_map(|token| parse_context_year(token))
            .collect(),
        imdb_ids: tokens
            .iter()
            .filter_map(|token| normalize_context_imdb_id(token))
            .collect(),
        episodes: Vec::new(),
    }
}

fn guess_context_facet(raw: &str, tokens: &[String]) -> ContextFacetHint {
    if tokens
        .iter()
        .any(|token| looks_like_standard_episode_token(token))
        || tokens.iter().any(|token| looks_like_daily_token(token))
        || looks_like_daily_token_sequence(tokens)
        || looks_like_season_pack_release(tokens)
    {
        return ContextFacetHint::Series;
    }
    if raw.contains('[')
        && tokens
            .iter()
            .any(|token| looks_like_absolute_episode_token(token))
        && !tokens
            .iter()
            .any(|token| parse_context_year(token).is_some())
    {
        return ContextFacetHint::Anime;
    }
    if tokens
        .iter()
        .any(|token| parse_context_year(token).is_some())
    {
        return ContextFacetHint::Movie;
    }
    ContextFacetHint::Unknown
}

fn parse_context_year(token: &str) -> Option<i32> {
    let normalized = token
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (normalized.len() == 4)
        .then(|| normalized.parse::<i32>().ok())
        .flatten()
        .filter(|year| (1900..=2099).contains(year))
}

fn normalize_context_imdb_id(token: &str) -> Option<String> {
    let normalized = token
        .trim()
        .trim_matches(|ch| matches!(ch, '{' | '}' | '[' | ']'));
    let imdb = normalized.strip_prefix("tt").unwrap_or(normalized);
    (!imdb.is_empty() && imdb.chars().all(|ch| ch.is_ascii_digit())).then(|| format!("tt{imdb}"))
}

fn looks_like_release_metadata_token(token: &str, tokens: &[String]) -> bool {
    let normalized = token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();

    parse_context_year(&normalized).is_some()
        || looks_like_standard_episode_token(&normalized)
        || looks_like_daily_token(&normalized)
        || looks_like_absolute_episode_token(&normalized)
        || looks_like_season_pack_marker(&normalized, tokens)
        || matches!(
            normalized.as_str(),
            "WEB"
                | "WEBDL"
                | "WEBRIP"
                | "BLURAY"
                | "BDRIP"
                | "BRRIP"
                | "HDTV"
                | "CAM"
                | "HQCAM"
                | "TS"
                | "TC"
                | "TELESYNC"
                | "TELECINE"
                | "DVDSCR"
                | "WORKPRINT"
                | "DVDRIP"
                | "NF"
                | "AMZN"
                | "DSNP"
                | "CR"
                | "HULU"
                | "MAX"
                | "HEVC"
                | "AVC"
                | "X265"
                | "X264"
                | "H265"
                | "H264"
                | "AAC"
                | "DDP"
                | "DTS"
                | "DTSX"
                | "DTSHD"
                | "DTSMA"
                | "TRUEHD"
                | "FLAC"
                | "DUAL"
                | "DUALAUDIO"
                | "AUDIO"
                | "DUB"
                | "DUBS"
                | "DUBBED"
                | "SUB"
                | "SUBS"
                | "MULTI"
                | "MULTIAUDIO"
                | "MULTISUB"
                | "MULTISUBS"
                | "EN"
                | "ENG"
                | "ENGLISH"
                | "JP"
                | "JPN"
                | "JAPANESE"
                | "HDR"
                | "HDR10"
                | "HDR10PLUS"
                | "HDR10P"
                | "HLG"
                | "DV"
                | "DOVI"
                | "ATMOS"
                | "PROPER"
                | "REPACK"
                | "REMUX"
                | "UNCENSORED"
        )
        || normalized.contains("2160")
        || normalized.contains("1080")
        || normalized.contains("720")
        || normalized.contains("480")
}

fn looks_like_season_pack_release(tokens: &[String]) -> bool {
    let normalized = tokens
        .iter()
        .map(|token| {
            token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_uppercase()
        })
        .collect::<Vec<_>>();
    normalized
        .iter()
        .any(|token| looks_like_bare_season_token(token))
        && normalized
            .iter()
            .any(|token| matches!(token.as_str(), "COMPLETE" | "SEASON" | "PACK" | "BATCH"))
}

fn looks_like_season_pack_marker(token: &str, tokens: &[String]) -> bool {
    if !looks_like_bare_season_token(token) {
        return false;
    }
    looks_like_season_pack_release(tokens)
        || tokens.iter().any(|candidate| {
            candidate
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .eq_ignore_ascii_case("COMPLETE")
        })
}

fn looks_like_bare_season_token(token: &str) -> bool {
    let Some(rest) = token.strip_prefix('S') else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 3
        && rest.chars().all(|ch| ch.is_ascii_digit())
        && !token.contains('E')
}

fn looks_like_standard_episode_token(token: &str) -> bool {
    let normalized = token.to_ascii_uppercase();
    normalized.contains('E')
        && normalized.starts_with('S')
        && normalized.chars().any(|ch| ch.is_ascii_digit())
        || normalized.split_once('X').is_some_and(|(left, right)| {
            !left.is_empty()
                && !right.is_empty()
                && left.chars().all(|ch| ch.is_ascii_digit())
                && right.chars().all(|ch| ch.is_ascii_digit())
        })
}

fn looks_like_daily_token(token: &str) -> bool {
    let normalized = token.trim();
    matches!(normalized.len(), 8 | 10)
        && normalized
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-'))
        && normalized.chars().filter(|ch| ch.is_ascii_digit()).count() >= 8
}

fn looks_like_daily_token_sequence(tokens: &[String]) -> bool {
    tokens.windows(3).any(|window| {
        let [year, month, day] = window else {
            return false;
        };
        let Some(year) = parse_context_year(year) else {
            return false;
        };
        let Some(month) = month.parse::<u32>().ok() else {
            return false;
        };
        let Some(day) = day.parse::<u32>().ok() else {
            return false;
        };
        (1900..=2099).contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day)
    })
}

fn looks_like_absolute_episode_token(token: &str) -> bool {
    let normalized = token.to_ascii_uppercase();
    if let Some((number, version)) = normalized.split_once('V')
        && !number.is_empty()
        && !version.is_empty()
        && number.chars().all(|ch| ch.is_ascii_digit())
        && version.chars().all(|ch| ch.is_ascii_digit())
    {
        return true;
    }
    normalized
        .split_once('-')
        .map(|(left, right)| {
            !left.is_empty()
                && !right.is_empty()
                && left.chars().all(|ch| ch.is_ascii_digit())
                && right.chars().all(|ch| ch.is_ascii_digit())
        })
        .unwrap_or_else(|| {
            (2..=4).contains(&normalized.len()) && normalized.chars().all(|ch| ch.is_ascii_digit())
        })
}
