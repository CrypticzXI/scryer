use std::{fmt, path::Path, sync::Arc};

use crate::{AppResult, ports::SubtitleSyncClient};

pub const ENHANCED_SUBTITLE_SYNC_PLUGIN_ID: &str = "enhanced-subtitle-sync";

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub offset_ms: i64,
    pub applied: bool,
    pub format: Option<SubtitleTimingFormat>,
    pub consistency_ratio: Option<f64>,
    pub nosplit_score: Option<f64>,
    pub split_score: Option<f64>,
    pub skipped_reason: Option<SyncSkipReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleTimingFormat {
    Srt,
    Vtt,
    Ass,
}

impl SubtitleTimingFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => "ass/ssa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncSkipReason {
    Disabled,
    ForcedSubtitle,
    ScoreAboveThreshold,
    SubtitleSyncPluginRequired,
    UnsupportedSubtitleFormat,
    AudioDecodeFailed,
    NotEnoughReferenceSpans,
    WeakAlignment,
    LowAlignmentConsistency,
    OffsetExceedsMaximum,
    OffsetTooSmall,
}

impl SyncSkipReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::ForcedSubtitle => "forced_subtitle",
            Self::ScoreAboveThreshold => "score_above_threshold",
            Self::SubtitleSyncPluginRequired => "subtitle_sync_plugin_required",
            Self::UnsupportedSubtitleFormat => "unsupported_subtitle_format",
            Self::AudioDecodeFailed => "audio_decode_failed",
            Self::NotEnoughReferenceSpans => "not_enough_reference_spans",
            Self::WeakAlignment => "weak_alignment",
            Self::LowAlignmentConsistency => "low_alignment_consistency",
            Self::OffsetExceedsMaximum => "offset_exceeds_maximum",
            Self::OffsetTooSmall => "offset_too_small",
        }
    }
}

impl fmt::Display for SyncSkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SyncPolicy {
    pub enabled: bool,
    pub forced: bool,
    pub score: Option<i32>,
    pub threshold: Option<i32>,
    pub max_offset_seconds: i64,
}

impl SyncResult {
    pub fn summary(&self) -> String {
        if self.applied {
            format!("applied {}ms offset", self.offset_ms)
        } else if let Some(reason) = self.skipped_reason {
            format!("skipped ({reason})")
        } else {
            "skipped".to_string()
        }
    }
}

pub async fn sync_subtitle_with_policy(
    _video_path: &Path,
    _subtitle_path: &Path,
    policy: SyncPolicy,
) -> AppResult<SyncResult> {
    sync_subtitle_with_policy_and_plugin_sync(
        _video_path,
        _subtitle_path,
        policy,
        None,
        false,
        None,
    )
    .await
}

pub async fn sync_subtitle_with_policy_and_plugin_sync(
    _video_path: &Path,
    _subtitle_path: &Path,
    policy: SyncPolicy,
    _subtitle_sync_client: Option<Arc<dyn SubtitleSyncClient>>,
    _plugin_installed: bool,
    _reference_subtitle_path: Option<&Path>,
) -> AppResult<SyncResult> {
    Ok(SyncResult {
        offset_ms: 0,
        applied: false,
        format: None,
        consistency_ratio: None,
        nosplit_score: None,
        split_score: None,
        skipped_reason: Some(policy_skip_reason(policy).unwrap_or(SyncSkipReason::Disabled)),
    })
}

pub async fn sync_subtitle(
    video_path: &Path,
    subtitle_path: &Path,
    max_offset_seconds: i64,
) -> AppResult<SyncResult> {
    sync_subtitle_with_policy_and_plugin_sync(
        video_path,
        subtitle_path,
        SyncPolicy {
            enabled: false,
            forced: false,
            score: None,
            threshold: None,
            max_offset_seconds,
        },
        None,
        false,
        None,
    )
    .await
}

fn policy_skip_reason(policy: SyncPolicy) -> Option<SyncSkipReason> {
    if !policy.enabled {
        return Some(SyncSkipReason::Disabled);
    }
    if policy.forced {
        return Some(SyncSkipReason::ForcedSubtitle);
    }
    if let (Some(score), Some(threshold)) = (policy.score, policy.threshold)
        && score > threshold
    {
        return Some(SyncSkipReason::ScoreAboveThreshold);
    }
    None
}
