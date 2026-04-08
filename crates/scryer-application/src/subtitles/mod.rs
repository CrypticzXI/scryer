//! Subtitle search, provider integration, scoring, sync, and orchestration.
//!
//! Start with `orchestration.rs` for the application-layer polling and trigger flow.
//! Provider behavior and language utilities live in the sibling files.

pub mod download;
pub mod language;
pub mod provider;
pub mod scoring;
pub mod search;
pub mod sync;
pub mod wanted;

pub use language::{
    from_opensubtitles_language, normalize_subtitle_language_code, same_subtitle_language,
    to_opensubtitles_language,
};
pub use provider::{
    SubtitleFile, SubtitleMatch, SubtitleMediaKind, SubtitleProvider, SubtitleQuery,
};
pub use scoring::{MovieScore, SeriesScore};
pub use search::SubtitleSearchOrchestrator;
