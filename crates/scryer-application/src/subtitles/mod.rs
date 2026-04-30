//! Subtitle search, provider integration, scoring, sync, and orchestration.
//!
//! Start with `orchestration.rs` for the application-layer polling and trigger flow.
//! Provider behavior and language utilities live in the sibling files.

pub mod configs;
pub mod download;
mod external;
mod external_probe;
pub mod extraction;
pub mod language;
pub mod orchestration;
pub mod provider;
pub mod scoring;
pub mod search;
pub mod sync;
pub mod wanted;

pub(crate) use external::reconcile_external_subtitles_for_media_file;
pub use external_probe::{ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry};
pub use language::{
    from_opensubtitles_language, normalize_subtitle_language_code, same_subtitle_language,
    to_opensubtitles_language,
};
pub use provider::{
    SubtitleFile, SubtitleMatch, SubtitleMediaKind, SubtitleProvider, SubtitleQuery,
};
pub use scoring::{MovieScore, SeriesScore};
pub use search::SubtitleSearchOrchestrator;
