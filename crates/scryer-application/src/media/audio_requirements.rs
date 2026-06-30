use crate::{ParsedReleaseMetadata, normalize_detected_audio_language_code};

const DEFAULT_NON_ANIME_AUDIO_LANGUAGE: &str = "eng";
const DEFAULT_ANIME_AUDIO_LANGUAGE: &str = "jpn";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TitleAudioLanguageContext {
    pub original_language: Option<String>,
    pub original_country: Option<String>,
    pub inferred_original_audio_language: String,
    pub is_anime: bool,
}

pub(crate) fn normalize_required_audio_languages(
    languages: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut normalized = Vec::new();
    for language in languages {
        if let Some(code) = normalize_detected_audio_language_code(&language)
            && !normalized.contains(&code)
        {
            normalized.push(code);
        }
    }
    normalized
}

pub(crate) fn normalize_title_country_code(country: &str) -> Option<String> {
    let normalized = country
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    if normalized.is_empty() {
        return None;
    }

    let country = match normalized.as_str() {
        "AR" | "ARG" | "ARGENTINA" => "AR",
        "AU" | "AUS" | "AUSTRALIA" => "AU",
        "BE" | "BEL" | "BELGIUM" => "BE",
        "BR" | "BRA" | "BRAZIL" => "BR",
        "CA" | "CAN" | "CANADA" => "CA",
        "CH" | "CHE" | "SWITZERLAND" => "CH",
        "CN" | "CHN" | "CHINA" => "CN",
        "DE" | "DEU" | "GERMANY" => "DE",
        "DK" | "DNK" | "DENMARK" => "DK",
        "ES" | "ESP" | "SPAIN" => "ES",
        "FI" | "FIN" | "FINLAND" => "FI",
        "FR" | "FRA" | "FRANCE" => "FR",
        "GB" | "GBR" | "UK" | "UNITEDKINGDOM" | "GREATBRITAIN" => "GB",
        "HK" | "HKG" | "HONGKONG" => "HK",
        "IE" | "IRL" | "IRELAND" => "IE",
        "IN" | "IND" | "INDIA" => "IN",
        "IT" | "ITA" | "ITALY" => "IT",
        "JP" | "JPN" | "JAPAN" => "JP",
        "KR" | "KOR" | "SOUTHKOREA" => "KR",
        "MX" | "MEX" | "MEXICO" => "MX",
        "NL" | "NLD" | "NETHERLANDS" => "NL",
        "NO" | "NOR" | "NORWAY" => "NO",
        "NZ" | "NZL" | "NEWZEALAND" => "NZ",
        "PH" | "PHL" | "PHILIPPINES" => "PH",
        "PL" | "POL" | "POLAND" => "PL",
        "PT" | "PRT" | "PORTUGAL" => "PT",
        "RU" | "RUS" | "RUSSIA" => "RU",
        "SE" | "SWE" | "SWEDEN" => "SE",
        "TH" | "THA" | "THAILAND" => "TH",
        "TR" | "TUR" | "TURKEY" => "TR",
        "TW" | "TWN" | "TAIWAN" => "TW",
        "US" | "USA" | "UNITEDSTATES" | "UNITEDSTATESOFAMERICA" => "US",
        _ => return None,
    };

    Some(country.to_string())
}

fn inferred_audio_language_for_country(country: &str) -> Option<&'static str> {
    match country {
        "AR" | "ES" | "MX" => Some("spa"),
        "AU" | "GB" | "IE" | "NZ" | "US" => Some("eng"),
        "BR" | "PT" => Some("por"),
        "CN" | "HK" | "TW" => Some("zho"),
        "DE" => Some("deu"),
        "DK" => Some("dan"),
        "FI" => Some("fin"),
        "FR" => Some("fra"),
        "IT" => Some("ita"),
        "JP" => Some("jpn"),
        "KR" => Some("kor"),
        "NL" => Some("nld"),
        "NO" => Some("nor"),
        "PL" => Some("pol"),
        "RU" => Some("rus"),
        "SE" => Some("swe"),
        "TH" => Some("tha"),
        "TR" => Some("tur"),
        _ => None,
    }
}

fn is_anime_context(category: Option<&str>, title_tags: &[String]) -> bool {
    category.is_some_and(|value| value.eq_ignore_ascii_case("anime"))
        || title_tags.iter().any(|tag| tag_marks_anime(tag))
}

/// Whether a title tag marks the title as anime. Matches the bare `anime` tag
/// as well as namespaced variants such as `anime-hd`. The search facet (and the
/// `category` hint derived from it) collapses anime movies and series-movie
/// links to `movie`, so tag-based detection is the fallback signal when the
/// category alone no longer carries the anime origin.
fn tag_marks_anime(tag: &str) -> bool {
    let tag = tag.trim();
    tag.eq_ignore_ascii_case("anime")
        || tag
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("anime-"))
}

pub(crate) fn title_audio_language_context(
    title_language: Option<&str>,
    title_country: Option<&str>,
    category: Option<&str>,
    title_tags: &[String],
) -> TitleAudioLanguageContext {
    let original_language = title_language.and_then(normalize_detected_audio_language_code);
    let original_country = title_country.and_then(normalize_title_country_code);
    let is_anime = is_anime_context(category, title_tags);
    let inferred_original_audio_language = original_language
        .clone()
        .or_else(|| {
            original_country
                .as_deref()
                .and_then(inferred_audio_language_for_country)
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if is_anime {
                DEFAULT_ANIME_AUDIO_LANGUAGE.to_string()
            } else {
                DEFAULT_NON_ANIME_AUDIO_LANGUAGE.to_string()
            }
        });

    TitleAudioLanguageContext {
        original_language,
        original_country,
        inferred_original_audio_language,
        is_anime,
    }
}

fn push_language(normalized: &mut Vec<String>, language: &str) {
    if !normalized.iter().any(|existing| existing == language) {
        normalized.push(language.to_string());
    }
}

pub(crate) fn release_audio_language_hints_for_title(
    parsed: &ParsedReleaseMetadata,
    indexer_languages: Option<&[String]>,
    title_language_context: Option<&TitleAudioLanguageContext>,
    infer_unlabeled_audio: bool,
) -> Vec<String> {
    let mut normalized = normalize_required_audio_languages(parsed.languages_audio.clone());

    if let Some(indexer_languages) = indexer_languages {
        for language in indexer_languages {
            if let Some(code) = normalize_detected_audio_language_code(language)
                && !normalized.contains(&code)
            {
                normalized.push(code);
            }
        }
    }

    if parsed.is_dual_audio && normalized.is_empty() {
        // A "dual audio" release ships two audio tracks: English plus the
        // title's original-language audio. Use the title's inferred original
        // language for the second track (which defaults to Japanese for anime),
        // and fall back to the canonical anime pairing when we have no context.
        // This applies to non-anime titles too, so an unlabeled dual-audio
        // release is not falsely treated as having zero audio languages.
        push_language(&mut normalized, "eng");
        let original_language = title_language_context
            .map(|context| context.inferred_original_audio_language.as_str())
            .unwrap_or(DEFAULT_ANIME_AUDIO_LANGUAGE);
        push_language(&mut normalized, original_language);
    }

    if normalized.is_empty()
        && !parsed.is_dual_audio
        && infer_unlabeled_audio
        && let Some(context) = title_language_context
    {
        push_language(&mut normalized, &context.inferred_original_audio_language);
    }

    normalized
}

pub(crate) fn missing_required_audio_languages<'a>(
    required: &'a [String],
    actual: &'a [String],
) -> Vec<String> {
    let actual_languages: Vec<String> = normalize_required_audio_languages(actual.iter().cloned());

    let mut missing = Vec::new();
    for required_language in required {
        let Some(normalized) = normalize_detected_audio_language_code(required_language) else {
            continue;
        };
        if !actual_languages
            .iter()
            .any(|actual_language| actual_language == &normalized)
        {
            missing.push(normalized);
        }
    }

    missing
}

pub(crate) fn required_audio_languages_match(required: &[String], actual: &[String]) -> bool {
    missing_required_audio_languages(required, actual).is_empty()
}

/// Resolve all canonical audio language codes named in a free-text audio track
/// title such as "English 5.1", "Eng DTS-HD", or "Eng+Jpn".
///
/// Tries the whole string first, then each token. Uses the strict
/// (passthrough-free) resolver so codec/technical tokens (e.g. DTS, AAC, AC3)
/// are not mistaken for languages. Returns the distinct languages found, in
/// order; empty when nothing maps.
#[cfg(any(test, feature = "runtime-media-analysis"))]
pub(crate) fn resolve_audio_languages_from_track_title(title: &str) -> Vec<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Some(code) = crate::normalize_known_audio_language_code(trimmed) {
        return vec![code];
    }
    let mut found = Vec::new();
    for token in trimmed.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        if let Some(code) = crate::normalize_known_audio_language_code(token)
            && !found.contains(&code)
        {
            found.push(code);
        }
    }
    found
}

/// The resolved language(s) of a single probed audio track: its ISO tag wins;
/// otherwise its title is parsed; otherwise the track is unresolved (empty).
#[cfg(any(test, feature = "runtime-media-analysis"))]
fn resolved_track_languages(stream: &crate::AudioStreamDetail) -> Vec<String> {
    if let Some(code) = stream
        .language
        .as_deref()
        .and_then(normalize_detected_audio_language_code)
    {
        return vec![code];
    }
    stream
        .name
        .as_deref()
        .map(resolve_audio_languages_from_track_title)
        .unwrap_or_default()
}

/// Verdict for the post-download required-audio-language gate.
#[cfg(any(test, feature = "runtime-media-analysis"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RequiredAudioVerdict {
    /// Every required language is present.
    Satisfied,
    /// Required language(s) are provably absent: every audio track's language is
    /// known (via tag or title) and none — nor the release hints — supply them.
    Missing(Vec<String>),
    /// Required language(s) can be neither confirmed present nor proven absent,
    /// because one or more audio tracks carry no usable language signal.
    Indeterminate(Vec<String>),
}

/// Classify whether a probed file satisfies the required audio languages.
///
/// Uses only strong signals — per-track ISO tags, per-track titles, and the
/// release-name/indexer hints already computed for the title. Distinguishes a
/// provable absence (`Missing` → reject) from an indeterminate result
/// (`Indeterminate` → accept + flag) so a correctly-dubbed file whose tracks are
/// untagged ("und") is not falsely rejected.
#[cfg(any(test, feature = "runtime-media-analysis"))]
pub(crate) fn classify_required_audio(
    required: &[String],
    audio_streams: &[crate::AudioStreamDetail],
    release_hints: &[String],
) -> RequiredAudioVerdict {
    let required: Vec<String> = normalize_required_audio_languages(required.iter().cloned());
    if required.is_empty() {
        return RequiredAudioVerdict::Satisfied;
    }

    // A file with no audio tracks carries no usable per-track signal: never
    // reject on it (avoid burying a release on a probe oddity); flag for review.
    if audio_streams.is_empty() {
        return RequiredAudioVerdict::Indeterminate(required);
    }

    let mut resolved: Vec<String> = Vec::new();
    let mut has_unresolved_track = false;
    for stream in audio_streams {
        let langs = resolved_track_languages(stream);
        if langs.is_empty() {
            has_unresolved_track = true;
        }
        for code in langs {
            if !resolved.contains(&code) {
                resolved.push(code);
            }
        }
    }

    // Release-name / indexer hints are explicit claims about this release.
    for hint in normalize_required_audio_languages(release_hints.iter().cloned()) {
        if !resolved.contains(&hint) {
            resolved.push(hint);
        }
    }

    let missing: Vec<String> = required
        .into_iter()
        .filter(|lang| !resolved.contains(lang))
        .collect();

    if missing.is_empty() {
        RequiredAudioVerdict::Satisfied
    } else if has_unresolved_track {
        RequiredAudioVerdict::Indeterminate(missing)
    } else {
        RequiredAudioVerdict::Missing(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        missing_required_audio_languages, normalize_required_audio_languages,
        normalize_title_country_code, release_audio_language_hints_for_title,
        required_audio_languages_match, title_audio_language_context,
    };
    use crate::normalize_detected_audio_language_code;
    use crate::release_parser::parse_release_metadata;

    #[test]
    fn dual_audio_without_explicit_languages_implies_english_and_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn explicit_languages_prevent_dual_audio_fallback() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO ENG 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string()]
        );
    }

    #[test]
    fn indexer_languages_are_merged_with_release_languages() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        let context = title_audio_language_context(None, None, Some("movie"), &[]);
        assert_eq!(
            release_audio_language_hints_for_title(
                &parsed,
                Some(&["English".to_string(), "Japanese".to_string()]),
                Some(&context),
                false,
            ),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn required_audio_languages_are_normalized() {
        assert_eq!(
            normalize_required_audio_languages(vec![
                "English".to_string(),
                "eng".to_string(),
                "ja-JP".to_string(),
            ]),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn language_aliases_normalize_to_canonical_audio_codes() {
        assert_eq!(
            normalize_detected_audio_language_code("en").as_deref(),
            Some("eng")
        );
        assert_eq!(
            normalize_detected_audio_language_code("English").as_deref(),
            Some("eng")
        );
        assert_eq!(
            normalize_detected_audio_language_code("fre").as_deref(),
            Some("fra")
        );
        assert_eq!(
            normalize_detected_audio_language_code("fr-FR").as_deref(),
            Some("fra")
        );
        assert_eq!(
            normalize_detected_audio_language_code("ja-JP").as_deref(),
            Some("jpn")
        );
        assert_eq!(
            normalize_detected_audio_language_code("Ger").as_deref(),
            Some("deu")
        );
        assert_eq!(normalize_detected_audio_language_code("und"), None);
    }

    #[test]
    fn title_countries_normalize_to_uppercase_alpha2_codes() {
        assert_eq!(normalize_title_country_code(" fr ").as_deref(), Some("FR"));
        assert_eq!(normalize_title_country_code("FRA").as_deref(), Some("FR"));
        assert_eq!(
            normalize_title_country_code("France").as_deref(),
            Some("FR")
        );
        assert_eq!(normalize_title_country_code("jp").as_deref(), Some("JP"));
        assert_eq!(normalize_title_country_code("JPN").as_deref(), Some("JP"));
        assert_eq!(normalize_title_country_code("Japan").as_deref(), Some("JP"));
        assert_eq!(normalize_title_country_code("not-a-country"), None);
    }

    #[test]
    fn title_audio_context_prefers_explicit_language() {
        let context = title_audio_language_context(Some("fre"), Some("Japan"), Some("movie"), &[]);

        assert_eq!(context.original_language.as_deref(), Some("fra"));
        assert_eq!(context.original_country.as_deref(), Some("JP"));
        assert_eq!(context.inferred_original_audio_language, "fra");
        assert!(!context.is_anime);
    }

    #[test]
    fn title_audio_context_uses_high_confidence_country_fallback() {
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(context.original_country.as_deref(), Some("FR"));
        assert_eq!(context.inferred_original_audio_language, "fra");
    }

    #[test]
    fn title_audio_context_defaults_unknown_non_anime_to_english() {
        let context = title_audio_language_context(None, Some("Canada"), Some("movie"), &[]);

        assert_eq!(context.original_country.as_deref(), Some("CA"));
        assert_eq!(context.inferred_original_audio_language, "eng");
    }

    #[test]
    fn title_audio_context_defaults_unknown_anime_to_japanese() {
        let context = title_audio_language_context(None, None, Some("anime"), &[]);

        assert_eq!(context.inferred_original_audio_language, "jpn");
        assert!(context.is_anime);
    }

    #[test]
    fn unlabeled_french_origin_release_infers_french_audio() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), true),
            vec!["fra".to_string()]
        );
    }

    #[test]
    fn unlabeled_unknown_non_anime_release_infers_english_audio() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        let context = title_audio_language_context(None, None, Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), true),
            vec!["eng".to_string()]
        );
    }

    #[test]
    fn unlabeled_release_does_not_infer_audio_when_required_gating_is_disabled() {
        let parsed = parse_release_metadata("[Group] Example Title 1080p");
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            Vec::<String>::new()
        );
    }

    #[test]
    fn anime_dual_audio_uses_english_plus_inferred_original_language() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, Some("South Korea"), Some("anime"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string(), "kor".to_string()]
        );
    }

    #[test]
    fn non_anime_dual_audio_infers_english_plus_origin_language() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string(), "fra".to_string()]
        );
    }

    #[test]
    fn non_anime_dual_audio_unknown_origin_implies_english() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, None, Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string()]
        );
    }

    #[test]
    fn dual_audio_without_title_context_defaults_to_english_and_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, None, false),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn anime_tagged_movie_dual_audio_infers_english_and_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        // Anime movies / series-movie links collapse to the "movie" search
        // category but carry an anime-* tag; detection must still treat them as
        // anime so dual-audio infers eng+jpn rather than just eng.
        let context =
            title_audio_language_context(None, None, Some("movie"), &["anime-hd".to_string()]);
        assert!(context.is_anime);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            vec!["eng".to_string(), "jpn".to_string()]
        );
    }

    #[test]
    fn anime_tag_variants_drive_anime_detection() {
        assert!(
            title_audio_language_context(None, None, Some("movie"), &["anime-hd".to_string()])
                .is_anime
        );
        assert!(
            title_audio_language_context(None, None, Some("movie"), &["Anime".to_string()])
                .is_anime
        );
        // Substrings that merely start with "anime" but are not the anime marker
        // must not be misdetected.
        assert!(
            !title_audio_language_context(None, None, Some("movie"), &["animation".to_string()])
                .is_anime
        );
    }

    #[test]
    fn subtitle_language_markers_do_not_satisfy_required_audio() {
        let parsed = parse_release_metadata("[Group] Example Title GER SUBS ENG 1080p");
        let context = title_audio_language_context(None, Some("Germany"), Some("series"), &[]);
        let actual = release_audio_language_hints_for_title(&parsed, None, Some(&context), true);

        assert!(actual.contains(&"deu".to_string()));
        assert!(!required_audio_languages_match(
            &["eng".to_string()],
            &actual
        ));
    }

    #[test]
    fn missing_languages_are_reported_in_canonical_form() {
        assert_eq!(
            missing_required_audio_languages(
                &["English".to_string(), "Japanese".to_string()],
                &["eng".to_string()]
            ),
            vec!["jpn".to_string()]
        );
    }

    #[test]
    fn dual_audio_matches_required_english() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        let actual = release_audio_language_hints_for_title(&parsed, None, Some(&context), false);
        assert!(required_audio_languages_match(
            &["eng".to_string()],
            &actual
        ));
    }

    #[test]
    fn dual_audio_matches_required_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        let actual = release_audio_language_hints_for_title(&parsed, None, Some(&context), false);
        assert!(required_audio_languages_match(
            &["jpn".to_string()],
            &actual
        ));
    }

    #[test]
    fn explicit_english_audio_does_not_imply_japanese() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO ENG 1080p");
        let context = title_audio_language_context(None, None, Some("anime"), &[]);
        let actual = release_audio_language_hints_for_title(&parsed, None, Some(&context), false);
        assert!(!required_audio_languages_match(
            &["jpn".to_string()],
            &actual
        ));
    }
}
