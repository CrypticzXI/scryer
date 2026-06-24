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
        || title_tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case("anime"))
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
        if let Some(context) = title_language_context {
            if context.is_anime {
                push_language(&mut normalized, "eng");
                push_language(&mut normalized, &context.inferred_original_audio_language);
            }
        } else {
            push_language(&mut normalized, "eng");
            push_language(&mut normalized, DEFAULT_ANIME_AUDIO_LANGUAGE);
        }
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
    fn non_anime_dual_audio_does_not_invent_exact_languages() {
        let parsed = parse_release_metadata("[Group] Example Title DUAL AUDIO 1080p");
        let context = title_audio_language_context(None, Some("France"), Some("movie"), &[]);

        assert_eq!(
            release_audio_language_hints_for_title(&parsed, None, Some(&context), false),
            Vec::<String>::new()
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
