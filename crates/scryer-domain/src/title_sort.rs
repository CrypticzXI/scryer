//! Locale-aware catalog sort keys.
//!
//! [`title_catalog_sort_key`] turns a title into an ICU collation sort key, hex-encoded so that
//! a plain byte-wise (`memcmp`) comparison of the strings reproduces the collator's order. The
//! result is persisted in `titles.catalog_sort_key` and ordered directly in SQL.
//!
//! Two invariants the storage layer depends on:
//!
//! * **Byte-wise comparison.** Ordering correctness assumes the column compares byte-for-byte.
//!   SQLite's default `BINARY` collation and Rust's `String` ordering both satisfy this. The
//!   values are lowercase hex (`0-9a-f`), which also orders identically under Postgres' common
//!   collations; a `COLLATE "C"` column would be the explicit guarantee if that ever changes.
//! * **Regeneration on change.** The key bytes are a function of this module's normalization
//!   (article lists, CJK width folding, collator strength) *and* the pinned `icu`/CLDR data.
//!   Changing any of those changes the bytes, so already-stored rows and newly written rows
//!   would sort inconsistently. Any such change MUST ship a new backfill migration that
//!   recomputes every row (mirroring migration 0145's `migrate_title_catalog_sort_keys` hook).

use icu_collator::{
    Collator, CollatorBorrowed,
    options::{CollatorOptions, Strength},
};
use icu_locale::Locale;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use unicode_normalization::UnicodeNormalization;

use crate::Title;

const DEFAULT_TITLE_CATALOG_SORT_LOCALE: &str = "en";
static COLLATOR_CACHE: LazyLock<Mutex<HashMap<String, Arc<CollatorBorrowed<'static>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const TITLE_CATALOG_WORD_ARTICLES: &[&str] = &[
    "a", "an", "the", "el", "la", "lo", "los", "las", "un", "una", "unos", "unas", "le", "les",
    "une", "des", "il", "gli", "uno", "der", "die", "das", "den", "dem", "ein", "eine", "einen",
    "einem", "einer", "eines", "de", "het", "een", "o", "os", "as", "um", "uma", "uns", "umas",
    "en", "et", "ett", "det", "els", "unes",
];

const TITLE_CATALOG_PREFIX_ARTICLES: &[&str] = &["l'", "l’", "al-"];

pub fn title_catalog_sort_key_for_title(title: &Title) -> String {
    let stored = title.catalog_sort_key.trim();
    if stored.is_empty() {
        title_catalog_sort_key(&title.name, title.metadata_language.as_deref())
    } else {
        stored.to_string()
    }
}

pub fn title_catalog_sort_key(name: &str, language: Option<&str>) -> String {
    let input = title_catalog_sort_input(name);
    title_catalog_sort_key_inner(&input, language)
}

fn title_catalog_sort_key_inner(input: &str, language: Option<&str>) -> String {
    let mut bytes = Vec::new();
    // Prefer the requested locale's collator, then the default locale, so every key stays in a
    // single comparable ICU sort-key space. The raw-byte branch below is NOT order-comparable
    // with collator keys; the default-locale fallback keeps it effectively unreachable.
    let collator = collator_for_language(language).or_else(|| collator_for_language(None));
    if let Some(collator) = collator
        && collator.write_sort_key_to(&input, &mut bytes).is_ok()
    {
        return lowercase_hex(&bytes);
    }

    // Last resort only (default collator failed to build): lowercase first so the key is at
    // least case-insensitive, matching the primary-strength collator behavior above.
    lowercase_hex(input.to_lowercase().as_bytes())
}

pub fn title_catalog_name_tie_key(name: &str) -> String {
    title_catalog_cjk_width_normalized_name(name)
        .trim()
        .to_lowercase()
}

pub fn title_catalog_sort_input(name: &str) -> String {
    let normalized_name = title_catalog_cjk_width_normalized_name(name);
    strip_catalog_sort_article(normalized_name.trim())
        .trim_start()
        .to_string()
}

fn collator_for_language(language: Option<&str>) -> Option<Arc<CollatorBorrowed<'static>>> {
    let tag = normalized_language_tag_or_default(language);
    if let Ok(cache) = COLLATOR_CACHE.lock()
        && let Some(collator) = cache.get(&tag)
    {
        return Some(Arc::clone(collator));
    }

    let locale = normalized_locale_from_tag(&tag);
    let collator = Arc::new(build_collator(locale)?);
    if let Ok(mut cache) = COLLATOR_CACHE.lock() {
        let cached = cache.entry(tag).or_insert_with(|| Arc::clone(&collator));
        return Some(Arc::clone(cached));
    }
    Some(collator)
}

fn build_collator(locale: Locale) -> Option<CollatorBorrowed<'static>> {
    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Primary);
    Collator::try_new(locale.into(), options).ok()
}

fn normalized_language_tag_or_default(language: Option<&str>) -> String {
    language
        .and_then(normalized_language_tag)
        .unwrap_or_else(|| DEFAULT_TITLE_CATALOG_SORT_LOCALE.to_string())
}

fn normalized_locale_from_tag(tag: &str) -> Locale {
    tag.parse::<Locale>()
        .unwrap_or_else(|_| icu_locale::locale!("en"))
}

fn normalized_language_tag(language: &str) -> Option<String> {
    let normalized = language.trim().replace('_', "-").to_ascii_lowercase();
    if normalized.is_empty() || normalized == "und" {
        return None;
    }
    Some(
        match normalized.as_str() {
            "eng" => "en",
            "jpn" => "ja",
            "zho" | "chi" | "cmn" | "chs" | "zhs" | "zh-cn" | "zh-hans" => "zh-Hans",
            "cht" | "zht" | "zh-tw" | "zh-hant" => "zh-Hant",
            "kor" => "ko",
            "fra" | "fre" => "fr",
            "deu" | "ger" => "de",
            "spa" => "es",
            "ita" => "it",
            "por" => "pt",
            "nld" | "dut" => "nl",
            "swe" => "sv",
            "dan" => "da",
            "nor" => "no",
            "fin" => "fi",
            "rus" => "ru",
            "ara" => "ar",
            "hin" => "hi",
            "tha" => "th",
            "vie" => "vi",
            "pol" => "pl",
            "tur" => "tr",
            "ukr" => "uk",
            "ces" | "cze" => "cs",
            "ell" | "gre" => "el",
            "heb" => "he",
            "ind" => "id",
            "msa" | "may" => "ms",
            "ron" | "rum" => "ro",
            "slk" | "slo" => "sk",
            "srp" => "sr",
            "hrv" => "hr",
            "bul" => "bg",
            "hun" => "hu",
            "est" => "et",
            "lav" => "lv",
            "lit" => "lt",
            "cat" => "ca",
            "eus" | "baq" => "eu",
            "glg" => "gl",
            "isl" | "ice" => "is",
            "gle" => "ga",
            "cym" | "wel" => "cy",
            value => return Some(value.to_string()),
        }
        .to_string(),
    )
}

fn title_catalog_cjk_width_normalized_name(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for character in value.chars() {
        if character == '\u{3000}' || ('\u{ff01}'..='\u{ff5e}').contains(&character) {
            normalized.extend(character.to_string().nfkc());
        } else {
            normalized.push(character);
        }
    }
    normalized
}

fn strip_catalog_sort_article(value: &str) -> &str {
    let lower = value.to_lowercase();
    for article in TITLE_CATALOG_PREFIX_ARTICLES {
        if lower.starts_with(article) {
            return &value[article.len()..];
        }
    }
    let Some((split_index, _)) = value.char_indices().find(|(_, ch)| ch.is_whitespace()) else {
        return value;
    };
    let article = &value[..split_index];
    let rest = &value[split_index..];
    if TITLE_CATALOG_WORD_ARTICLES
        .iter()
        .any(|candidate| article.eq_ignore_ascii_case(candidate))
    {
        rest
    } else {
        value
    }
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_multilingual_articles_before_sort_key_generation() {
        assert_eq!(title_catalog_sort_input("The Matrix"), "Matrix");
        assert_eq!(title_catalog_sort_input("L’Arc-en-Ciel"), "Arc-en-Ciel");
        assert_eq!(
            title_catalog_sort_input("O Auto da Compadecida"),
            "Auto da Compadecida"
        );
    }

    #[test]
    fn normalizes_cjk_width_before_sort_key_generation() {
        assert_eq!(title_catalog_sort_input("Ｔｈｅ　Matrix"), "Matrix");
        assert_eq!(title_catalog_sort_input("ＡＫＩＲＡ"), "AKIRA");
    }

    #[test]
    fn cjk_sort_key_is_deterministic_and_non_empty() {
        let first = title_catalog_sort_key("鋼の錬金術師", Some("jpn"));
        let second = title_catalog_sort_key("鋼の錬金術師", Some("ja"));
        assert!(!first.is_empty());
        assert_eq!(first, second);
    }

    #[test]
    fn hex_sort_key_order_matches_icu_collator_order() {
        let left = "あした";
        let right = "いま";
        let collator = collator_for_language(Some("ja")).expect("ja collator");
        let comparison = collator.compare(left, right);
        assert_eq!(
            title_catalog_sort_key(left, Some("ja"))
                .cmp(&title_catalog_sort_key(right, Some("ja"))),
            comparison
        );
    }

    #[test]
    fn invalid_language_falls_back_to_default_locale() {
        assert_eq!(
            title_catalog_sort_key("The Matrix", Some("not a language")),
            title_catalog_sort_key("The Matrix", Some("eng"))
        );
    }
}
