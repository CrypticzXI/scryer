#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let candidate = String::from_utf8_lossy(data);
    let words = candidate
        .split(|ch: char| matches!(ch, '.' | '_' | ' ' | '-' | '/' | '[' | ']' | '(' | ')' | '{' | '}'))
        .filter(|part| !part.trim().is_empty())
        .take(6)
        .map(str::to_string)
        .collect::<Vec<_>>();

    let title = words.first().cloned().unwrap_or_else(|| "Unknown".to_string());
    let aliases = words
        .iter()
        .skip(1)
        .take(2)
        .map(|value| scryer_release_parser_v2::ContextAlias {
            name: value.clone(),
        })
        .collect::<Vec<_>>();

    let context = scryer_release_parser_v2::ReleaseParseContext {
        facet_hint: scryer_release_parser_v2::ContextFacetHint::Anime,
        title: scryer_release_parser_v2::ContextTitle { name: title },
        aliases,
        known_years: candidate
            .split(|ch: char| !ch.is_ascii_digit())
            .filter_map(|part| {
                let year = part.parse::<i32>().ok()?;
                (1900..=2099).contains(&year).then_some(year)
            })
            .take(2)
            .collect(),
        imdb_ids: Vec::new(),
        episodes: Vec::new(),
    };

    let alternate = scryer_release_parser_v2::ReleaseParseContext {
        facet_hint: scryer_release_parser_v2::ContextFacetHint::Series,
        title: scryer_release_parser_v2::ContextTitle {
            name: words
                .get(1)
                .cloned()
                .unwrap_or_else(|| "Fallback".to_string()),
        },
        aliases: Vec::new(),
        known_years: Vec::new(),
        imdb_ids: Vec::new(),
        episodes: Vec::new(),
    };

    let _ =
        scryer_release_parser_v2::analyze_release_against_targets(candidate.as_ref(), &[context, alternate]);
});
