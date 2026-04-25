#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let candidate = String::from_utf8_lossy(data);
    let title = candidate
        .split(|ch: char| {
            matches!(ch, '.' | '_' | ' ' | '-' | '/' | '[' | ']' | '(' | ')' | '{' | '}')
        })
        .find(|part| !part.trim().is_empty())
        .unwrap_or("Unknown")
        .to_string();

    let context = scryer_release_parser::ReleaseParseContext {
        facet_hint: scryer_release_parser::ContextFacetHint::Series,
        title: scryer_release_parser::ContextTitle { name: title },
        aliases: Vec::new(),
        known_years: Vec::new(),
        imdb_ids: Vec::new(),
        episodes: Vec::new(),
    };

    let _ = scryer_release_parser::analyze_release_for_target(candidate.as_ref(), &context);
});
