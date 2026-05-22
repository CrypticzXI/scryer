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
            match scryer_mediainfo::analyze_file(&path) {
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
