pub(crate) mod profile;
pub mod release_dedup;
pub(crate) mod release_group_db;
pub(crate) mod release_parser;
pub(crate) mod scoring_weights;
pub(crate) mod trash_scores;

/// The hand-authored TRaSH ranking corpus. It spans the parser, the profile
/// scoring path, the release-group tiers and the managed locale packs, so it
/// hangs off the quality module rather than any one of them.
#[cfg(test)]
#[path = "trash_ranking_corpus_tests.rs"]
mod trash_ranking_corpus_tests;
