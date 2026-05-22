use std::path::Path;

use chrono::Utc;

use crate::{AppResult, stored_paths::path_to_stored_string};

pub const EXTERNAL_SUBTITLE_PROBE_VERSION: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalSubtitleDetectionSource {
    Filename,
    Content,
    Unknown,
}

impl ExternalSubtitleDetectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Filename => "filename",
            Self::Content => "content",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "filename" => Some(Self::Filename),
            "content" => Some(Self::Content),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSubtitleProbeCacheEntry {
    pub media_file_id: String,
    pub file_path: String,
    pub size_bytes: i64,
    pub modified_at: Option<String>,
    pub language: Option<String>,
    pub hearing_impaired: Option<bool>,
    pub detection_source_language: ExternalSubtitleDetectionSource,
    pub detection_source_hi: ExternalSubtitleDetectionSource,
    pub probe_version: i32,
    pub updated_at: String,
}

impl ExternalSubtitleProbeCacheEntry {
    pub fn hearing_impaired_or_false(&self) -> bool {
        self.hearing_impaired.unwrap_or(false)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExternalSubtitleProbeResolution {
    pub language: Option<String>,
    pub hearing_impaired: bool,
    pub cache_entry: ExternalSubtitleProbeCacheEntry,
}

pub(crate) async fn resolve_external_subtitle(
    media_file_id: &str,
    subtitle_path: &Path,
    _extension: &str,
    filename_language: Option<&str>,
    forced: bool,
    filename_hearing_impaired: bool,
    existing_cache: Option<&ExternalSubtitleProbeCacheEntry>,
) -> AppResult<ExternalSubtitleProbeResolution> {
    if let Some(cache_entry) = existing_cache {
        return Ok(ExternalSubtitleProbeResolution {
            language: cache_entry.language.clone(),
            hearing_impaired: cache_entry.hearing_impaired_or_false(),
            cache_entry: cache_entry.clone(),
        });
    }

    let language = filename_language.map(str::to_string);
    let detection_source_language = if language.is_some() {
        ExternalSubtitleDetectionSource::Filename
    } else {
        ExternalSubtitleDetectionSource::Unknown
    };
    let detection_source_hi = if forced || filename_hearing_impaired {
        ExternalSubtitleDetectionSource::Filename
    } else {
        ExternalSubtitleDetectionSource::Unknown
    };
    let cache_entry = ExternalSubtitleProbeCacheEntry {
        media_file_id: media_file_id.to_string(),
        file_path: path_to_stored_string(subtitle_path),
        size_bytes: 0,
        modified_at: None,
        language: language.clone(),
        hearing_impaired: Some(filename_hearing_impaired),
        detection_source_language,
        detection_source_hi,
        probe_version: EXTERNAL_SUBTITLE_PROBE_VERSION,
        updated_at: Utc::now().to_rfc3339(),
    };

    Ok(ExternalSubtitleProbeResolution {
        language,
        hearing_impaired: filename_hearing_impaired,
        cache_entry,
    })
}
