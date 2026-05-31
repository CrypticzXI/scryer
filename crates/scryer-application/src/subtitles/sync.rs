//! Subtitle timing synchronization using an optional plugin-owned sync engine.
//!
//! The sync pipeline:
//! 1. Applies policy gates that do not require media analysis.
//! 2. Reads subtitle bytes and detects the rewrite format.
//! 3. Delegates media analysis, timing, and rewriting to the optional enhanced sync plugin.
//! 4. Atomically applies the rewritten subtitle bytes returned by the plugin.

use std::{fmt, io::Write, path::Path, sync::Arc};

use crate::{
    AppError, AppResult,
    ports::{SubtitleSyncClient, SubtitleSyncJob},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scryer_plugin_sdk::{
    AudioTranscodeCodec, SubtitleSyncAlignResponse, SubtitleSyncAlignSkipReason,
    SubtitleSyncOptions,
};

/// Result of a subtitle sync operation.
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Time offset applied in milliseconds.
    pub offset_ms: i64,
    /// Whether the sync was applied.
    pub applied: bool,
    /// Detected subtitle format when one was recognized.
    pub format: Option<SubtitleTimingFormat>,
    /// Alignment consistency across split deltas.
    pub consistency_ratio: Option<f64>,
    /// Constant-offset alignment score.
    pub nosplit_score: Option<f64>,
    /// Split alignment score.
    pub split_score: Option<f64>,
    /// Why sync was skipped when `applied` is false.
    pub skipped_reason: Option<SyncSkipReason>,
}

pub const ENHANCED_SUBTITLE_SYNC_PLUGIN_ID: &str = "enhanced-subtitle-sync";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleTimingFormat {
    Srt,
    Ass,
}

impl SubtitleTimingFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Ass => "ass/ssa",
        }
    }

    fn sdk_format(self, path: &Path) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Ass => {
                if path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("ssa"))
                {
                    "ssa"
                } else {
                    "ass"
                }
            }
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

impl SyncPolicy {
    fn skip_reason(self) -> Option<SyncSkipReason> {
        if !self.enabled {
            return Some(SyncSkipReason::Disabled);
        }

        if self.forced {
            return Some(SyncSkipReason::ForcedSubtitle);
        }

        if let (Some(score), Some(threshold)) = (self.score, self.threshold)
            && score > threshold
        {
            return Some(SyncSkipReason::ScoreAboveThreshold);
        }

        None
    }
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

/// Synchronize a subtitle file with a video file's audio track using a Bazarr-style policy gate.
pub async fn sync_subtitle_with_policy(
    video_path: &Path,
    subtitle_path: &Path,
    policy: SyncPolicy,
) -> AppResult<SyncResult> {
    sync_subtitle_with_policy_and_plugin_sync(video_path, subtitle_path, policy, None, false).await
}

pub async fn sync_subtitle_with_policy_and_plugin_sync(
    video_path: &Path,
    subtitle_path: &Path,
    policy: SyncPolicy,
    subtitle_sync_client: Option<Arc<dyn SubtitleSyncClient>>,
    plugin_installed: bool,
) -> AppResult<SyncResult> {
    if let Some(reason) = policy.skip_reason() {
        tracing::debug!(
            path = %subtitle_path.display(),
            score = policy.score,
            threshold = policy.threshold,
            reason = %reason,
            "subtitle sync skipped by policy"
        );
        return Ok(skipped_sync_result(0, None, None, None, None, reason));
    }

    sync_subtitle_with_plugin_sync(
        video_path,
        subtitle_path,
        policy.max_offset_seconds,
        subtitle_sync_client,
        plugin_installed,
    )
    .await
}

/// Synchronize a subtitle file with a video file's audio track.
pub async fn sync_subtitle(
    video_path: &Path,
    subtitle_path: &Path,
    max_offset_seconds: i64,
) -> AppResult<SyncResult> {
    sync_subtitle_with_plugin_sync(video_path, subtitle_path, max_offset_seconds, None, false).await
}

pub async fn sync_subtitle_with_plugin_sync(
    video_path: &Path,
    subtitle_path: &Path,
    max_offset_seconds: i64,
    subtitle_sync_client: Option<Arc<dyn SubtitleSyncClient>>,
    plugin_installed: bool,
) -> AppResult<SyncResult> {
    let subtitle_content = std::fs::read(subtitle_path)
        .map_err(|e| AppError::Repository(format!("cannot read subtitle file: {e}")))?;
    let Some(subtitle_format) = detect_subtitle_format(subtitle_path, &subtitle_content) else {
        tracing::debug!(
            path = %subtitle_path.display(),
            "subtitle sync skipped: unsupported subtitle format"
        );
        return Ok(skipped_sync_result(
            0,
            None,
            None,
            None,
            None,
            SyncSkipReason::UnsupportedSubtitleFormat,
        ));
    };

    let Some(subtitle_sync_client) = subtitle_sync_client else {
        tracing::warn!(
            path = %video_path.display(),
            plugin_id = ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
            "{}",
            if plugin_installed {
                missing_enhanced_sync_update_hint()
            } else {
                missing_enhanced_sync_install_hint()
            }
        );
        return Ok(skipped_sync_result(
            0,
            Some(subtitle_format),
            None,
            None,
            None,
            SyncSkipReason::SubtitleSyncPluginRequired,
        ));
    };

    let subtitle_file_name = subtitle_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string);
    let subtitle_encoding_hint = subtitle_encoding_hint(&subtitle_content);

    let response = match subtitle_sync_client
        .align_subtitle(SubtitleSyncJob {
            input_path: video_path.to_path_buf(),
            subtitle_content,
            subtitle_format: subtitle_format.sdk_format(subtitle_path).to_string(),
            subtitle_file_name,
            subtitle_encoding_hint,
            max_offset_seconds,
            sync_options: SubtitleSyncOptions::default(),
            expected_codec: targeted_audio_codec_for_path(video_path),
        })
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                path = %video_path.display(),
                plugin_id = ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
                error = %error,
                "subtitle sync skipped: plugin execution failed"
            );
            return Ok(skipped_sync_result(
                0,
                Some(subtitle_format),
                None,
                None,
                None,
                SyncSkipReason::AudioDecodeFailed,
            ));
        }
    };

    if !response.warnings.is_empty() {
        tracing::debug!(
            path = %video_path.display(),
            backend = response.backend.as_str(),
            warnings = ?response.warnings,
            "subtitle sync plugin reported warnings"
        );
    }

    if !response.applied {
        return Ok(sync_result_from_plugin_response(subtitle_format, response));
    }

    let rewritten_subtitle = response.rewritten_subtitle.as_ref().ok_or_else(|| {
        AppError::Repository(
            "subtitle sync plugin reported applied without rewritten_subtitle".to_string(),
        )
    })?;
    let bytes = BASE64
        .decode(&rewritten_subtitle.content_base64)
        .map_err(|error| {
            AppError::Repository(format!(
                "subtitle sync plugin returned invalid rewritten subtitle base64: {error}"
            ))
        })?;
    write_subtitle_atomic(subtitle_path, &bytes)?;

    tracing::info!(
        path = %subtitle_path.display(),
        format = subtitle_format.label(),
        backend = response.backend.as_str(),
        offset_ms = response.offset_ms,
        consistency_ratio = response.consistency_ratio,
        nosplit_score = response.nosplit_score,
        split_score = response.split_score,
        "subtitle synchronized"
    );
    Ok(SyncResult {
        offset_ms: response.offset_ms,
        applied: true,
        format: Some(subtitle_format),
        consistency_ratio: response.consistency_ratio,
        nosplit_score: response.nosplit_score,
        split_score: response.split_score,
        skipped_reason: None,
    })
}

fn missing_enhanced_sync_install_hint() -> String {
    format!(
        "subtitle sync requires plugin '{}'; install and enable it to use subtitle sync",
        ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
    )
}

fn missing_enhanced_sync_update_hint() -> String {
    format!(
        "subtitle sync plugin '{}' is installed but too old; update it to restore subtitle sync support",
        ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
    )
}

fn sync_result_from_plugin_response(
    subtitle_format: SubtitleTimingFormat,
    response: SubtitleSyncAlignResponse,
) -> SyncResult {
    skipped_sync_result(
        response.offset_ms,
        Some(subtitle_format),
        response.consistency_ratio,
        response.nosplit_score,
        response.split_score,
        response
            .skipped_reason
            .map(map_plugin_skip_reason)
            .unwrap_or(SyncSkipReason::AudioDecodeFailed),
    )
}

fn map_plugin_skip_reason(reason: SubtitleSyncAlignSkipReason) -> SyncSkipReason {
    match reason {
        SubtitleSyncAlignSkipReason::AudioDecodeFailed => SyncSkipReason::AudioDecodeFailed,
        SubtitleSyncAlignSkipReason::NotEnoughReferenceSpans => {
            SyncSkipReason::NotEnoughReferenceSpans
        }
        SubtitleSyncAlignSkipReason::WeakAlignment => SyncSkipReason::WeakAlignment,
        SubtitleSyncAlignSkipReason::LowAlignmentConsistency => {
            SyncSkipReason::LowAlignmentConsistency
        }
        SubtitleSyncAlignSkipReason::OffsetExceedsMaximum => SyncSkipReason::OffsetExceedsMaximum,
        SubtitleSyncAlignSkipReason::OffsetTooSmall => SyncSkipReason::OffsetTooSmall,
    }
}

fn skipped_sync_result(
    offset_ms: i64,
    format: Option<SubtitleTimingFormat>,
    consistency_ratio: Option<f64>,
    nosplit_score: Option<f64>,
    split_score: Option<f64>,
    reason: SyncSkipReason,
) -> SyncResult {
    SyncResult {
        offset_ms,
        applied: false,
        format,
        consistency_ratio,
        nosplit_score,
        split_score,
        skipped_reason: Some(reason),
    }
}

fn targeted_audio_codec_for_path(video_path: &Path) -> Option<AudioTranscodeCodec> {
    let analysis = scryer_mediainfo::analyze_file_with_options(
        video_path,
        scryer_mediainfo::AnalyzeOptions {
            profile: scryer_mediainfo::AnalysisProfile::FfprobeParity,
        },
    )
    .ok()?;
    targeted_audio_codec(
        analysis.audio_codec.as_deref(),
        analysis.audio_profile.as_deref(),
    )
}

fn targeted_audio_codec(codec: Option<&str>, profile: Option<&str>) -> Option<AudioTranscodeCodec> {
    let normalized = codec?.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "ac3" => Some(AudioTranscodeCodec::Ac3),
        "eac3" => Some(AudioTranscodeCodec::Eac3),
        "truehd" | "mlp" => Some(AudioTranscodeCodec::TrueHd),
        "dts" => {
            if profile
                .map(|profile| profile.to_ascii_lowercase().contains("dts-hd ma"))
                .unwrap_or(false)
            {
                Some(AudioTranscodeCodec::DtsHdMaCore)
            } else {
                Some(AudioTranscodeCodec::Dts)
            }
        }
        _ => None,
    }
}

fn subtitle_encoding_hint(bytes: &[u8]) -> Option<String> {
    if std::str::from_utf8(bytes).is_ok() {
        return Some("utf-8".to_string());
    }

    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    Some(
        detector
            .guess(None, chardetng::Utf8Detection::Allow)
            .name()
            .to_ascii_lowercase(),
    )
}

fn detect_subtitle_format(path: &Path, content: &[u8]) -> Option<SubtitleTimingFormat> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("srt") => return Some(SubtitleTimingFormat::Srt),
        Some("ass") | Some("ssa") => return Some(SubtitleTimingFormat::Ass),
        _ => {}
    }

    if content.windows(3).any(|window| window == b"-->") {
        return Some(SubtitleTimingFormat::Srt);
    }
    if contains_ascii_case_insensitive(content, b"[Events]")
        || contains_ascii_case_insensitive(content, b"Dialogue:")
    {
        return Some(SubtitleTimingFormat::Ass);
    }

    None
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn write_subtitle_atomic(subtitle_path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = subtitle_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty());
    let mut temp = match parent {
        Some(parent) => tempfile::NamedTempFile::new_in(parent),
        None => tempfile::NamedTempFile::new(),
    }
    .map_err(|e| AppError::Repository(format!("cannot create subtitle temp file: {e}")))?;
    temp.write_all(bytes)
        .and_then(|_| temp.flush())
        .map_err(|e| AppError::Repository(format!("cannot write subtitle temp file: {e}")))?;
    let temp_path = temp.into_temp_path();
    std::fs::rename(&temp_path, subtitle_path)
        .map_err(|e| AppError::Repository(format!("cannot replace subtitle file: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RewritingSubtitleSyncClient {
        expected_subtitle_content: Vec<u8>,
        rewritten_subtitle_content: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl crate::ports::SubtitleSyncClient for RewritingSubtitleSyncClient {
        async fn align_subtitle(
            &self,
            job: crate::ports::SubtitleSyncJob,
        ) -> AppResult<SubtitleSyncAlignResponse> {
            assert_eq!(job.subtitle_content, self.expected_subtitle_content);
            assert_eq!(job.subtitle_format, "srt");
            assert_eq!(job.subtitle_file_name.as_deref(), Some("subtitle.srt"));
            assert_eq!(job.subtitle_encoding_hint.as_deref(), Some("utf-8"));
            assert_eq!(job.sync_options.start_seconds, 0);
            assert_eq!(job.sync_options.max_subtitle_duration_ms, 10_000);
            assert!(job.sync_options.precise_framerate_search);
            assert_eq!(job.sync_options.output_encoding, "same");
            Ok(SubtitleSyncAlignResponse {
                applied: true,
                offset_ms: -1000,
                rewritten_subtitle: Some(scryer_plugin_sdk::SubtitleSyncRewrittenSubtitle {
                    content_base64: BASE64.encode(&self.rewritten_subtitle_content),
                    format: "srt".to_string(),
                }),
                score: Some(42.0),
                selected_framerate_ratio: Some(1.0),
                consistency_ratio: Some(1.0),
                nosplit_score: Some(42.0),
                split_score: None,
                skipped_reason: None,
                backend: "test-subtitle-sync".to_string(),
                warnings: Vec::new(),
                message: None,
            })
        }
    }

    #[test]
    fn detects_subtitle_format_from_extension_or_content() {
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle.srt"), b""),
            Some(SubtitleTimingFormat::Srt)
        );
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle.ass"), b""),
            Some(SubtitleTimingFormat::Ass)
        );
        assert_eq!(
            detect_subtitle_format(
                Path::new("subtitle"),
                b"1\n00:00:01,000 --> 00:00:02,000\nHello\n"
            ),
            Some(SubtitleTimingFormat::Srt)
        );
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle"), b"[Events]\nDialogue: 0,0:00:01.00"),
            Some(SubtitleTimingFormat::Ass)
        );
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle"), b"plain"),
            None
        );
    }

    #[test]
    fn subtitle_encoding_hint_identifies_utf8_and_single_byte_inputs() {
        assert_eq!(
            subtitle_encoding_hint("hello".as_bytes()).as_deref(),
            Some("utf-8")
        );

        let mut bytes = b"caf".to_vec();
        bytes.push(0xe9);
        let hint = subtitle_encoding_hint(&bytes).unwrap();
        assert_ne!(hint, "utf-8");
        assert!(!hint.is_empty());
    }

    #[test]
    fn targeted_audio_codec_routes_only_ffmpeg_wasm_codecs() {
        assert_eq!(
            targeted_audio_codec(Some("ac3"), None),
            Some(AudioTranscodeCodec::Ac3)
        );
        assert_eq!(
            targeted_audio_codec(Some("eac3"), None),
            Some(AudioTranscodeCodec::Eac3)
        );
        assert_eq!(
            targeted_audio_codec(Some("truehd"), None),
            Some(AudioTranscodeCodec::TrueHd)
        );
        assert_eq!(
            targeted_audio_codec(Some("dts"), Some("DTS-HD MA")),
            Some(AudioTranscodeCodec::DtsHdMaCore)
        );
        assert_eq!(
            targeted_audio_codec(Some("dts"), Some("DTS Core")),
            Some(AudioTranscodeCodec::Dts)
        );
        assert_eq!(targeted_audio_codec(Some("aac"), None), None);
        assert_eq!(targeted_audio_codec(Some("flac"), None), None);
    }

    #[test]
    fn missing_enhanced_sync_plugin_warning_mentions_install_hint() {
        let message = missing_enhanced_sync_install_hint();
        assert!(message.contains("install and enable it"));
        assert!(message.contains(ENHANCED_SUBTITLE_SYNC_PLUGIN_ID));
    }

    #[test]
    fn missing_enhanced_sync_plugin_warning_mentions_update_hint() {
        let message = missing_enhanced_sync_update_hint();
        assert!(message.contains("too old"));
        assert!(message.contains("update"));
        assert!(message.contains(ENHANCED_SUBTITLE_SYNC_PLUGIN_ID));
    }

    #[tokio::test]
    async fn sync_applies_rewritten_subtitle_bytes_from_plugin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.srt");
        let original = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nWorld\n\n3\n00:00:05,000 --> 00:00:06,000\nAgain\n".to_vec();
        let rewritten = b"1\n00:00:00,000 --> 00:00:01,000\nHello\n\n2\n00:00:02,000 --> 00:00:03,000\nWorld\n\n3\n00:00:04,000 --> 00:00:05,000\nAgain\n".to_vec();
        std::fs::write(&subtitle_path, &original).unwrap();

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(RewritingSubtitleSyncClient {
                expected_subtitle_content: original,
                rewritten_subtitle_content: rewritten.clone(),
            })),
            true,
        )
        .await
        .unwrap();

        assert!(result.applied);
        assert_eq!(result.offset_ms, -1000);
        assert_eq!(std::fs::read(&subtitle_path).unwrap(), rewritten);
    }

    #[tokio::test]
    async fn sync_delegates_single_cue_subtitles_to_plugin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.srt");
        let original = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n".to_vec();
        let rewritten = b"1\n00:00:02,000 --> 00:00:03,000\nHello\n".to_vec();
        std::fs::write(&subtitle_path, &original).unwrap();

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(RewritingSubtitleSyncClient {
                expected_subtitle_content: original,
                rewritten_subtitle_content: rewritten.clone(),
            })),
            true,
        )
        .await
        .unwrap();

        assert!(result.applied);
        assert_eq!(std::fs::read(&subtitle_path).unwrap(), rewritten);
    }

    #[tokio::test]
    async fn sync_rejects_applied_plugin_response_without_rewritten_subtitle() {
        struct MissingRewriteClient;

        #[async_trait::async_trait]
        impl crate::ports::SubtitleSyncClient for MissingRewriteClient {
            async fn align_subtitle(
                &self,
                _job: crate::ports::SubtitleSyncJob,
            ) -> AppResult<SubtitleSyncAlignResponse> {
                Ok(SubtitleSyncAlignResponse {
                    applied: true,
                    offset_ms: 1000,
                    rewritten_subtitle: None,
                    score: Some(1.0),
                    selected_framerate_ratio: Some(1.0),
                    consistency_ratio: Some(1.0),
                    nosplit_score: Some(1.0),
                    split_score: None,
                    skipped_reason: None,
                    backend: "test-subtitle-sync".to_string(),
                    warnings: Vec::new(),
                    message: None,
                })
            }
        }

        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.srt");
        std::fs::write(
            &subtitle_path,
            "1\n00:00:01,000 --> 00:00:04,000\nHello\n\n2\n00:00:05,000 --> 00:00:08,000\nWorld\n\n3\n00:00:09,000 --> 00:00:12,000\nAgain\n",
        )
        .unwrap();

        let error = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(MissingRewriteClient)),
            true,
        )
        .await
        .expect_err("missing rewrite should fail");

        assert!(error.to_string().contains("rewritten_subtitle"));
    }

    #[tokio::test]
    async fn sync_requires_plugin_when_unavailable() {
        let subtitle = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            subtitle.path(),
            "1\n00:00:01,000 --> 00:00:04,000\nHello\n\n2\n00:00:05,000 --> 00:00:08,000\nWorld\n\n3\n00:00:09,000 --> 00:00:12,000\nAgain\n",
        )
        .unwrap();

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            subtitle.path(),
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            None,
            false,
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(
            result.skipped_reason,
            Some(SyncSkipReason::SubtitleSyncPluginRequired)
        );
        assert_eq!(result.format, Some(SubtitleTimingFormat::Srt));
    }

    #[tokio::test]
    async fn policy_skip_when_disabled() {
        let result = sync_subtitle_with_policy(
            Path::new("/tmp/video.mkv"),
            Path::new("/tmp/subtitle.srt"),
            SyncPolicy {
                enabled: false,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(result.skipped_reason, Some(SyncSkipReason::Disabled));
    }

    #[tokio::test]
    async fn policy_skip_when_forced() {
        let result = sync_subtitle_with_policy(
            Path::new("/tmp/video.mkv"),
            Path::new("/tmp/subtitle.srt"),
            SyncPolicy {
                enabled: true,
                forced: true,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(result.skipped_reason, Some(SyncSkipReason::ForcedSubtitle));
    }

    #[tokio::test]
    async fn policy_skip_when_score_above_threshold() {
        let result = sync_subtitle_with_policy(
            Path::new("/tmp/video.mkv"),
            Path::new("/tmp/subtitle.srt"),
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(91),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(
            result.skipped_reason,
            Some(SyncSkipReason::ScoreAboveThreshold)
        );
    }

    #[test]
    fn sync_result_summary_uses_skip_reason() {
        let result = skipped_sync_result(0, None, None, None, None, SyncSkipReason::ForcedSubtitle);
        assert_eq!(result.summary(), "skipped (forced_subtitle)");
    }
}
