use unicode_normalization::UnicodeNormalization;

const TRAILING_ARTICLES: &[&str] = &["a", "an", "the"];
const MOVIE_LOW_SIGNAL_TOKENS: &[&str] = &[
    "a",
    "an",
    "the",
    "movie",
    "film",
    "arc",
    "special",
    "specials",
    "part",
    "eiga",
    "gekijouban",
    "gekijoban",
    "gekijo",
    "gekijōban",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TitleMatchProfile {
    Movie,
}

pub(crate) fn canonical_lookup_key(title: &str) -> String {
    let tokens = reorder_trailing_article(canonical_tokens(title));
    if tokens.is_empty() {
        return String::new();
    }

    tokens.join(" ")
}

pub(crate) fn reduced_comparison_key(title: &str, profile: TitleMatchProfile) -> String {
    let canonical = canonical_lookup_key(title);
    if canonical.is_empty() {
        return String::new();
    }

    canonical
        .split_whitespace()
        .filter(|token| !low_signal_tokens(profile).contains(token))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn search_variants(title: &str) -> Vec<String> {
    let mut variants = Vec::new();

    let cleaned = cleaned_search_title(title);
    if !cleaned.is_empty() {
        variants.push(cleaned);
    }

    let canonical = canonical_lookup_key(title);
    if !canonical.is_empty() && !variants.iter().any(|value| value == &canonical) {
        variants.push(canonical);
    }

    variants
}

fn cleaned_search_title(title: &str) -> String {
    title
        .nfkc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn canonical_tokens(title: &str) -> Vec<String> {
    let mut normalized = String::new();
    let mut previous_space = false;

    for character in title.nfkc().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
            previous_space = false;
        } else if character.is_whitespace() || is_separator(character) {
            if !previous_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            previous_space = true;
        }
    }

    normalized.split_whitespace().map(str::to_string).collect()
}

fn reorder_trailing_article(mut tokens: Vec<String>) -> Vec<String> {
    if tokens.len() < 2 {
        return tokens;
    }

    if let Some(article) = tokens.last().cloned()
        && TRAILING_ARTICLES.contains(&article.as_str())
    {
        tokens.pop();
        let mut reordered = vec![article];
        reordered.extend(tokens);
        return reordered;
    }

    tokens
}

fn is_separator(character: char) -> bool {
    matches!(
        character,
        '.' | ','
            | ':'
            | ';'
            | '-'
            | '_'
            | '/'
            | '\\'
            | '&'
            | '+'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '\''
            | '"'
            | '!'
            | '?'
            | '~'
    )
}

fn low_signal_tokens(profile: TitleMatchProfile) -> &'static [&'static str] {
    match profile {
        TitleMatchProfile::Movie => MOVIE_LOW_SIGNAL_TOKENS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_lookup_key_reorders_trailing_articles() {
        assert_eq!(canonical_lookup_key("LANTERN, The"), "the lantern");
        assert_eq!(canonical_lookup_key("The LANTERN"), "the lantern");
    }

    #[test]
    fn canonical_lookup_key_normalizes_unicode_and_punctuation() {
        assert_eq!(
            canonical_lookup_key("Kelu\u{301}ne: Detective Kestrel"),
            "kelúne detective kestrel"
        );
        assert_eq!(
            canonical_lookup_key("Kessler: The Courier"),
            "kessler the courier"
        );
    }

    #[test]
    fn reduced_comparison_key_drops_bounded_movie_boilerplate() {
        assert_eq!(
            reduced_comparison_key("Nagare The Movie Kagami Mix", TitleMatchProfile::Movie),
            "nagare kagami mix"
        );
        assert_eq!(
            reduced_comparison_key("Eiga Nagare: Kagami Mix", TitleMatchProfile::Movie),
            "nagare kagami mix"
        );
        assert_eq!(
            reduced_comparison_key(
                "Ember Saga -Kage no Kotoba- The Movie: Iron Rail",
                TitleMatchProfile::Movie
            ),
            "ember saga kage no kotoba iron rail"
        );
    }

    #[test]
    fn search_variants_adds_canonical_form_when_needed() {
        assert_eq!(
            search_variants("LANTERN, The"),
            vec!["LANTERN, The".to_string(), "the lantern".to_string()]
        );
        assert_eq!(
            search_variants("Kessler: The Courier"),
            vec![
                "Kessler: The Courier".to_string(),
                "kessler the courier".to_string()
            ]
        );
    }

    #[test]
    fn search_variants_keep_full_movie_title_with_subtitle_and_franchise_suffix() {
        assert_eq!(
            search_variants("Circuit Breakers Crash the Grid 2"),
            vec![
                "Circuit Breakers Crash the Grid 2".to_string(),
                "circuit breakers crash the grid 2".to_string()
            ]
        );
        assert_eq!(
            reduced_comparison_key(
                "Circuit Breakers Crash the Grid 2",
                TitleMatchProfile::Movie
            ),
            "circuit breakers crash grid 2"
        );
    }
}
