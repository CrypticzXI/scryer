//! Subtitle timing synchronization using an optional plugin-owned sync engine.
//!
//! The sync pipeline:
//! 1. Applies policy gates that do not require media analysis.
//! 2. Parses subtitle timing spans from SRT or ASS/SSA.
//! 3. Delegates media analysis and alignment to the optional enhanced sync plugin.
//! 4. Applies the returned constant offset when the plugin reports a safe sync.

use std::{fmt, path::Path, sync::Arc};

use crate::{
    AppError, AppResult,
    ports::{SubtitleSyncClient, SubtitleSyncJob},
};
use scryer_plugin_sdk::{
    AudioTranscodeCodec, SubtitleSyncAlignResponse, SubtitleSyncAlignSkipReason, SubtitleTimingSpan,
};

/// Result of a subtitle sync operation.
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Time offset applied in milliseconds.
    pub offset_ms: i64,
    /// Whether the sync was applied.
    pub applied: bool,
    /// Parsed subtitle format when one was recognized.
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

const MIN_SUBTITLE_SPANS: usize = 3;
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
    NotEnoughSubtitleSpans,
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
            Self::NotEnoughSubtitleSpans => "not_enough_subtitle_spans",
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

#[derive(Debug, Clone, Copy)]
struct AssEventFormat {
    field_count: usize,
    start_idx: usize,
    end_idx: usize,
}

impl Default for AssEventFormat {
    fn default() -> Self {
        Self {
            field_count: 10,
            start_idx: 1,
            end_idx: 2,
        }
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
    let Some((subtitle_format, subtitle_spans)) = read_subtitle_spans(subtitle_path)? else {
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
    if subtitle_spans.len() < MIN_SUBTITLE_SPANS {
        tracing::debug!(
            path = %subtitle_path.display(),
            format = subtitle_format.label(),
            spans = subtitle_spans.len(),
            "subtitle sync skipped: not enough subtitle spans"
        );
        return Ok(skipped_sync_result(
            0,
            Some(subtitle_format),
            None,
            None,
            None,
            SyncSkipReason::NotEnoughSubtitleSpans,
        ));
    }

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

    let response = match subtitle_sync_client
        .align_subtitle(SubtitleSyncJob {
            input_path: video_path.to_path_buf(),
            subtitle_spans,
            max_offset_seconds,
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

    apply_subtitle_offset(subtitle_path, subtitle_format, response.offset_ms)?;

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

// ── Subtitle parsing and shifting with charset detection ─────────────────────

/// Read a subtitle file, auto-detecting charset for common wild-text encodings.
fn read_subtitle_to_string(path: &Path) -> AppResult<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| AppError::Repository(format!("cannot read subtitle file: {e}")))?;

    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Ok(s.to_string());
    }

    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(&bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);

    let (decoded, _, had_errors) = encoding.decode(&bytes);
    if had_errors {
        tracing::warn!(
            path = %path.display(),
            encoding = %encoding.name(),
            "subtitle file had encoding errors during charset conversion"
        );
    }

    Ok(decoded.into_owned())
}

fn read_subtitle_spans(
    path: &Path,
) -> AppResult<Option<(SubtitleTimingFormat, Vec<SubtitleTimingSpan>)>> {
    let content = read_subtitle_to_string(path)?;
    let Some(format) = detect_subtitle_format(path, &content) else {
        return Ok(None);
    };

    let spans = match format {
        SubtitleTimingFormat::Srt => read_srt_spans_from_str(&content),
        SubtitleTimingFormat::Ass => read_ass_spans_from_str(&content),
    };
    Ok(Some((format, spans)))
}

fn detect_subtitle_format(path: &Path, content: &str) -> Option<SubtitleTimingFormat> {
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

    if content.contains("-->") {
        return Some(SubtitleTimingFormat::Srt);
    }
    if content.contains("[Events]")
        || content
            .lines()
            .any(|line| line_starts_with_ignore_ascii_case(line.trim_start(), "Dialogue:"))
    {
        return Some(SubtitleTimingFormat::Ass);
    }

    None
}

fn apply_subtitle_offset(
    subtitle_path: &Path,
    format: SubtitleTimingFormat,
    offset_ms: i64,
) -> AppResult<()> {
    let content = read_subtitle_to_string(subtitle_path)?;
    let shifted = match format {
        SubtitleTimingFormat::Srt => shift_srt_content(&content, offset_ms),
        SubtitleTimingFormat::Ass => shift_ass_content(&content, offset_ms),
    };

    std::fs::write(subtitle_path, shifted)
        .map_err(|e| AppError::Repository(format!("cannot write subtitle file: {e}")))?;
    Ok(())
}

fn read_srt_spans_from_str(content: &str) -> Vec<SubtitleTimingSpan> {
    let mut spans = Vec::new();
    for line in content.lines() {
        if let Some((start, end)) = line.split_once("-->")
            && let (Some(start), Some(end)) = (parse_srt_ts(start.trim()), parse_srt_ts(end.trim()))
        {
            spans.push(SubtitleTimingSpan {
                start_ms: start,
                end_ms: end,
            });
        }
    }
    spans
}

fn shift_srt_content(content: &str, offset_ms: i64) -> String {
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        if let Some((start_str, end_str)) = line.split_once("-->")
            && let (Some(start), Some(end)) =
                (parse_srt_ts(start_str.trim()), parse_srt_ts(end_str.trim()))
        {
            out.push_str(&format_srt_ts(start + offset_ms));
            out.push_str(" --> ");
            out.push_str(&format_srt_ts(end + offset_ms));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn read_ass_spans_from_str(content: &str) -> Vec<SubtitleTimingSpan> {
    let mut spans = Vec::new();
    let mut in_events = false;
    let mut event_format = AssEventFormat::default();

    for line in content.lines() {
        let trimmed = line.trim();
        if is_section_header(trimmed) {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if line_starts_with_ignore_ascii_case(trimmed, "Format:") {
            if let Some(parsed) = parse_ass_event_format(trimmed) {
                event_format = parsed;
            }
            continue;
        }
        if !line_starts_with_ignore_ascii_case(trimmed, "Dialogue:") {
            continue;
        }

        let Some(fields) = split_ass_fields(trimmed, event_format.field_count) else {
            continue;
        };
        if let (Some(start), Some(end)) = (
            parse_ass_ts(fields[event_format.start_idx].trim()),
            parse_ass_ts(fields[event_format.end_idx].trim()),
        ) {
            spans.push(SubtitleTimingSpan {
                start_ms: start,
                end_ms: end,
            });
        }
    }

    spans
}

fn shift_ass_content(content: &str, offset_ms: i64) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_events = false;
    let mut event_format = AssEventFormat::default();

    for line in content.lines() {
        let trimmed = line.trim();
        if is_section_header(trimmed) {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_events && line_starts_with_ignore_ascii_case(trimmed, "Format:") {
            if let Some(parsed) = parse_ass_event_format(trimmed) {
                event_format = parsed;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_events && let Some(rewritten) = rewrite_ass_event_line(line, &event_format, offset_ms)
        {
            out.push_str(&rewritten);
            out.push('\n');
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    out
}

fn parse_ass_event_format(line: &str) -> Option<AssEventFormat> {
    let (_, rest) = line.split_once(':')?;
    let fields: Vec<String> = rest
        .split(',')
        .map(|field| field.trim().to_ascii_lowercase())
        .collect();
    let start_idx = fields.iter().position(|field| field == "start")?;
    let end_idx = fields.iter().position(|field| field == "end")?;

    Some(AssEventFormat {
        field_count: fields.len(),
        start_idx,
        end_idx,
    })
}

fn split_ass_fields(line: &str, field_count: usize) -> Option<Vec<&str>> {
    let (_, rest) = line.split_once(':')?;
    let fields: Vec<&str> = rest.trim_start().splitn(field_count, ',').collect();
    if fields.len() != field_count {
        return None;
    }
    Some(fields)
}

fn rewrite_ass_event_line(line: &str, format: &AssEventFormat, offset_ms: i64) -> Option<String> {
    let colon_index = line.find(':')?;
    let prefix = &line[..colon_index];
    let event_kind = prefix.trim();
    if !matches!(
        event_kind.to_ascii_lowercase().as_str(),
        "dialogue" | "comment" | "picture" | "sound" | "movie" | "command"
    ) {
        return None;
    }

    let rest = &line[colon_index + 1..];
    let leading_ws_len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let leading_ws = &rest[..leading_ws_len];

    let mut fields: Vec<String> = rest
        .trim_start()
        .splitn(format.field_count, ',')
        .map(|field| field.to_string())
        .collect();
    if fields.len() != format.field_count {
        return None;
    }

    let start = parse_ass_ts(fields[format.start_idx].trim())?;
    let end = parse_ass_ts(fields[format.end_idx].trim())?;
    fields[format.start_idx] = format_ass_ts(start + offset_ms);
    fields[format.end_idx] = format_ass_ts(end + offset_ms);

    Some(format!("{prefix}:{leading_ws}{}", fields.join(",")))
}

fn is_section_header(line: &str) -> bool {
    line.starts_with('[') && line.ends_with(']')
}

fn line_starts_with_ignore_ascii_case(line: &str, prefix: &str) -> bool {
    line.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

// ── Timestamp parsing and formatting ─────────────────────────────────────────

fn parse_srt_ts(ts: &str) -> Option<i64> {
    let parts: Vec<&str> = ts.split([':', ',', '.']).collect();
    if parts.len() < 4 {
        return None;
    }
    let h: i64 = parts[0].trim().parse().ok()?;
    let m: i64 = parts[1].trim().parse().ok()?;
    let s: i64 = parts[2].trim().parse().ok()?;
    let ms: i64 = parts[3].trim().parse().ok()?;
    Some(h * 3_600_000 + m * 60_000 + s * 1_000 + ms)
}

fn format_srt_ts(ms: i64) -> String {
    let ms = ms.max(0);
    let ts = ms / 1000;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ts / 3600,
        (ts % 3600) / 60,
        ts % 60,
        ms % 1000
    )
}

fn parse_ass_ts(ts: &str) -> Option<i64> {
    let ts = ts.trim();
    let separator = ts.find(['.', ','])?;
    let main = &ts[..separator];
    let frac = &ts[separator + 1..];

    let parts: Vec<&str> = main.split(':').collect();
    if parts.len() != 3 {
        return None;
    }

    let h: i64 = parts[0].trim().parse().ok()?;
    let m: i64 = parts[1].trim().parse().ok()?;
    let s: i64 = parts[2].trim().parse().ok()?;
    let ms = parse_fractional_ms(frac)?;

    Some(h * 3_600_000 + m * 60_000 + s * 1_000 + ms)
}

fn parse_fractional_ms(frac: &str) -> Option<i64> {
    let digits: String = frac
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .collect();
    if digits.is_empty() {
        return Some(0);
    }

    let value: i64 = digits.parse().ok()?;
    Some(match digits.len() {
        1 => value * 100,
        2 => value * 10,
        _ => value,
    })
}

fn format_ass_ts(ms: i64) -> String {
    let total_cs = (ms.max(0) + 5) / 10;
    let total_seconds = total_cs / 100;
    format!(
        "{}:{:02}:{:02}.{:02}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60,
        total_cs % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_roundtrip() {
        assert_eq!(parse_srt_ts("00:01:23,456"), Some(83_456));
        assert_eq!(format_srt_ts(83_456), "00:01:23,456");
        assert_eq!(format_srt_ts(0), "00:00:00,000");
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

    #[test]
    fn ass_parse_and_format_roundtrip() {
        assert_eq!(parse_ass_ts("0:01:23.45"), Some(83_450));
        assert_eq!(format_ass_ts(83_450), "0:01:23.45");
        assert_eq!(format_ass_ts(-100), "0:00:00.00");
    }

    #[test]
    fn format_srt_ts_clamps_negative() {
        assert_eq!(format_srt_ts(-1000), "00:00:00,000");
        assert_eq!(format_srt_ts(-1), "00:00:00,000");
    }

    #[test]
    fn parse_srt_ts_with_dot_separator() {
        assert_eq!(parse_srt_ts("00:01:23.456"), Some(83_456));
    }

    #[test]
    fn parse_srt_ts_hours_greater_than_23() {
        assert_eq!(parse_srt_ts("25:00:00,000"), Some(25 * 3_600_000));
    }

    #[test]
    fn parse_srt_ts_rejects_too_few_parts() {
        assert_eq!(parse_srt_ts("00:01:23"), None);
        assert_eq!(parse_srt_ts(""), None);
    }

    #[test]
    fn parse_srt_ts_rejects_non_numeric() {
        assert_eq!(parse_srt_ts("ab:cd:ef,ghi"), None);
    }

    #[test]
    fn charset_detection_utf8_passthrough() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "1\n00:00:01,000 --> 00:00:02,000\nHello\n").unwrap();
        let content = read_subtitle_to_string(tmp.path()).unwrap();
        assert!(content.contains("Hello"));
    }

    #[test]
    fn charset_detection_latin1() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut data = b"1\n00:00:01,000 --> 00:00:02,000\ncaf".to_vec();
        data.push(0xe9);
        data.push(b'\n');
        std::fs::write(tmp.path(), &data).unwrap();
        let content = read_subtitle_to_string(tmp.path()).unwrap();
        assert!(
            content.contains("caf"),
            "should contain 'caf' after charset conversion"
        );
    }

    #[test]
    fn ass_spans_extract_dialogue_lines_only() {
        let content = "[Script Info]\n\
Title: Demo\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Comment: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,ignored\n\
Dialogue: 0,0:00:03.00,0:00:05.00,Default,,0,0,0,,Hello\n";
        let spans = read_ass_spans_from_str(content);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start_ms, 3_000);
        assert_eq!(spans[0].end_ms, 5_000);
    }

    #[test]
    fn shift_ass_content_rewrites_event_times_and_preserves_text_with_commas() {
        let content = "[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:03.00,0:00:05.00,Default,,0,0,0,,Hello, world\n\
Comment: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,note\n";
        let shifted = shift_ass_content(content, 1_500);
        assert!(shifted.contains("Dialogue: 0,0:00:04.50,0:00:06.50,Default,,0,0,0,,Hello, world"));
        assert!(shifted.contains("Comment: 0,0:00:02.50,0:00:03.50,Default,,0,0,0,,note"));
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
