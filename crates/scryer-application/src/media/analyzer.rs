#[cfg(feature = "runtime-media-analysis")]
use std::path::Path;
use std::path::PathBuf;

use async_trait::async_trait;

use crate::{AppError, AppResult, MediaAnalysisOutcome, MediaAnalyzer, nice_thread};

pub struct NativeMediaAnalyzer;

#[async_trait]
impl MediaAnalyzer for NativeMediaAnalyzer {
    async fn analyze_file(&self, path: PathBuf) -> AppResult<MediaAnalysisOutcome> {
        tokio::task::spawn_blocking(move || {
            nice_thread();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("strm"))
            {
                return Ok(MediaAnalysisOutcome::Valid(Box::new(
                    crate::post_download_gate::build_stream_pointer_media_file_analysis(),
                )));
            }

            #[cfg(feature = "runtime-media-analysis")]
            match analyze_file_fast_then_rich(&path) {
                Ok(analysis) if scryer_mediainfo::is_valid_video(&analysis) => {
                    Ok(MediaAnalysisOutcome::Valid(Box::new(
                        crate::post_download_gate::build_media_file_analysis(&analysis),
                    )))
                }
                Ok(_) => Ok(MediaAnalysisOutcome::Invalid(
                    "file is not a valid video".to_string(),
                )),
                Err(error) => Ok(MediaAnalysisOutcome::Invalid(error.to_string())),
            }
            #[cfg(not(feature = "runtime-media-analysis"))]
            Ok(MediaAnalysisOutcome::Invalid(
                "native media analysis is not compiled into this target".to_string(),
            ))
        })
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    }
}

#[cfg(feature = "runtime-media-analysis")]
fn analyze_file_fast_then_rich(
    path: &Path,
) -> Result<scryer_mediainfo::MediaAnalysis, scryer_mediainfo::MediaInfoError> {
    if is_mkv_path(path) {
        return scryer_mediainfo::analyze_file_with_options(
            path,
            scryer_mediainfo::AnalyzeOptions {
                profile: scryer_mediainfo::AnalysisProfile::DefaultRich,
            },
        );
    }

    let fast = scryer_mediainfo::analyze_file_with_options(
        path,
        scryer_mediainfo::AnalyzeOptions {
            profile: scryer_mediainfo::AnalysisProfile::Fast,
        },
    )?;
    if fast_analysis_has_adequate_facts(&fast) {
        return Ok(fast);
    }

    scryer_mediainfo::analyze_file_with_options(
        path,
        scryer_mediainfo::AnalyzeOptions {
            profile: scryer_mediainfo::AnalysisProfile::DefaultRich,
        },
    )
}

#[cfg(feature = "runtime-media-analysis")]
fn is_mkv_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "mkv" | "webm"))
}

#[cfg(feature = "runtime-media-analysis")]
fn fast_analysis_has_adequate_facts(analysis: &scryer_mediainfo::MediaAnalysis) -> bool {
    if !scryer_mediainfo::is_valid_video(analysis) {
        return false;
    }
    if analysis.video_width.is_none()
        || analysis.video_height.is_none()
        || analysis.duration_seconds.is_none()
        || analysis.video_frame_rate.is_none()
    {
        return false;
    }
    if !analysis.audio_streams.is_empty() && analysis.audio_codec.is_none() {
        return false;
    }
    if analysis.audio_streams.iter().any(|stream| {
        stream.codec.is_none()
            || stream.channels.is_none()
            || audio_codec_needs_profile(stream.codec.as_deref()) && stream.profile.is_none()
    }) {
        return false;
    }
    if analysis
        .subtitle_streams
        .iter()
        .any(|stream| stream.codec.is_none())
        || analysis.subtitle_codecs.len() < analysis.subtitle_streams.len()
    {
        return false;
    }

    !hevc_hdr_may_need_rich_confirmation(analysis)
}

#[cfg(feature = "runtime-media-analysis")]
fn audio_codec_needs_profile(codec: Option<&str>) -> bool {
    matches!(codec, Some("eac3" | "truehd" | "dts"))
}

#[cfg(feature = "runtime-media-analysis")]
fn hevc_hdr_may_need_rich_confirmation(analysis: &scryer_mediainfo::MediaAnalysis) -> bool {
    analysis.video_codec.as_deref() == Some("hevc")
        && analysis.video_hdr_format.as_deref() != Some("Dolby Vision")
        && (analysis
            .video_bit_depth
            .is_some_and(|bit_depth| bit_depth >= 10)
            || matches!(
                analysis.video_hdr_format.as_deref(),
                Some("HDR10" | "HLG" | "HDR10+")
            ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn analyze_file_returns_minimal_valid_analysis_for_strm() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Example.Movie.2024.strm");
        std::fs::write(&path, b"https://nzbdav.example/stream/Example.Movie.2024")
            .expect("write strm");

        let analyzer = NativeMediaAnalyzer;
        let outcome = analyzer.analyze_file(path).await.expect("analyze strm");

        match outcome {
            MediaAnalysisOutcome::Valid(analysis) => {
                assert_eq!(analysis.container_format.as_deref(), Some("strm"));
                assert!(analysis.audio_streams.is_empty());
                assert!(analysis.subtitle_streams.is_empty());
            }
            MediaAnalysisOutcome::Invalid(error) => {
                panic!("expected valid strm analysis, got invalid: {error}");
            }
        }
    }
}
