#[cfg(test)]
mod tests {
    use super::{
        COMPLETED_ORIGIN_SCOPE_CONFLICT, CompletedDownloadOriginResolution,
        CompletedDownloadSubmissionMatch, CompletedDownloadSubmissionResolution,
        ManualImportFileMapping, completed_import_status_for_result, is_sample_file,
        resolve_completed_download_origin, resolved_episode_ids_are_within_expected,
        sanitized_title_folder_component, skip_reason_for_import_check_code,
        validate_path_manual_import_mappings,
    };
    use crate::{DownloadSubmission, DownloadSubmissionPurpose, SubmissionScope};
    use chrono::Utc;
    use scryer_domain::{
        CompletedDownload, ImportDecision, ImportResult, ImportSkipReason, ImportStatus,
    };
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn title_folder_component_falls_back_when_sanitized_empty() {
        assert_eq!(sanitized_title_folder_component("///...___---"), "untitled");
    }

    #[test]
    fn title_folder_component_keeps_nonempty_values() {
        assert_eq!(
            sanitized_title_folder_component("Movie Title (2024)"),
            "Movie Title (2024)"
        );
    }

    #[test]
    fn grabbed_release_gate_allows_only_expected_episode_ids() {
        let expected = HashSet::from(["ep-1".to_string()]);

        assert!(resolved_episode_ids_are_within_expected(
            &["ep-1".to_string()],
            &expected
        ));
        assert!(!resolved_episode_ids_are_within_expected(
            &["ep-1".to_string(), "ep-2".to_string()],
            &expected
        ));
    }

    fn completed_download_with_parameters(parameters: Vec<(&str, &str)>) -> CompletedDownload {
        CompletedDownload {
            client_type: "sabnzbd".to_string(),
            client_id: "client-1".to_string(),
            download_client_item_id: "item-1".to_string(),
            download_id: Some("download-1".to_string()),
            name: "Release".to_string(),
            dest_dir: "/downloads/release".to_string(),
            category: Some("anime".to_string()),
            size_bytes: Some(1024),
            completed_at: None,
            parameters: parameters
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    fn matched_submission(
        title_id: &str,
        facet: &str,
        scope: SubmissionScope,
    ) -> CompletedDownloadSubmissionResolution {
        CompletedDownloadSubmissionResolution::Matched(Box::new(
            CompletedDownloadSubmissionMatch {
                submission: DownloadSubmission {
                    title_id: title_id.to_string(),
                    facet: facet.to_string(),
                    download_client_id: Some("client-1".to_string()),
                    download_client_type: "sabnzbd".to_string(),
                    download_client_item_id: "item-1".to_string(),
                    source_hint: None,
                    source_kind: None,
                    source_title: None,
                    request_signature: None,
                    purpose: DownloadSubmissionPurpose::Standard,
                    scope,
                },
                identity: None,
            },
        ))
    }

    fn parameter_value<'a>(parameters: &'a [(String, String)], key: &str) -> Option<&'a str> {
        parameters
            .iter()
            .find(|(candidate_key, _)| candidate_key == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn completed_origin_resolution_keeps_matching_complete_params() {
        let completed = completed_download_with_parameters(vec![
            ("*scryer_title_id", "title-1"),
            ("*scryer_facet", "anime"),
            ("*scryer_series_movie_link_id", "series-movie-link-1"),
        ]);
        let resolution = matched_submission(
            "title-1",
            "anime",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &resolution)
        else {
            panic!("expected ready completed download");
        };

        assert_eq!(resolved.parameters, completed.parameters);
    }

    #[test]
    fn completed_origin_resolution_writes_series_movie_scope_without_existing_params() {
        let completed = completed_download_with_parameters(vec![]);
        let resolution = matched_submission(
            "title-1",
            "anime",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &resolution)
        else {
            panic!("expected ready completed download");
        };

        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_title_id"),
            Some("title-1")
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_facet"),
            Some("anime")
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_series_movie_link_id"),
            Some("series-movie-link-1")
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_collection_id"),
            None
        );
    }

    #[test]
    fn completed_origin_resolution_preserves_legacy_collection_for_series_movie() {
        let completed = completed_download_with_parameters(vec![
            ("*scryer_title_id", "title-1"),
            ("*scryer_facet", "anime"),
            ("*scryer_collection_id", "legacy-collection-1"),
        ]);
        let resolution = matched_submission(
            "title-1",
            "anime",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &resolution)
        else {
            panic!("expected ready completed download");
        };

        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_collection_id"),
            Some("legacy-collection-1")
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_series_movie_link_id"),
            Some("series-movie-link-1")
        );
    }

    #[test]
    fn completed_origin_resolution_conflicts_on_title_facet_or_scope_mismatch() {
        for (completed, resolution) in [
            (
                completed_download_with_parameters(vec![("*scryer_title_id", "title-2")]),
                matched_submission("title-1", "anime", SubmissionScope::Title),
            ),
            (
                completed_download_with_parameters(vec![
                    ("*scryer_title_id", "title-1"),
                    ("*scryer_facet", "movie"),
                ]),
                matched_submission("title-1", "anime", SubmissionScope::Title),
            ),
            (
                completed_download_with_parameters(vec![
                    ("*scryer_title_id", "title-1"),
                    ("*scryer_facet", "anime"),
                    ("*scryer_series_movie_link_id", "series-movie-link-1"),
                ]),
                matched_submission(
                    "title-1",
                    "anime",
                    SubmissionScope::Collection {
                        collection_id: "collection-1".to_string(),
                    },
                ),
            ),
        ] {
            let CompletedDownloadOriginResolution::Conflict { reason, detail } =
                resolve_completed_download_origin(&completed, &resolution)
            else {
                panic!("expected origin conflict");
            };
            assert_eq!(reason, COMPLETED_ORIGIN_SCOPE_CONFLICT);
            assert!(!detail.is_empty());
        }
    }

    #[test]
    fn completed_origin_resolution_preserves_existing_params_for_stub_submission() {
        let completed = completed_download_with_parameters(vec![
            ("*scryer_title_id", "title-1"),
            ("*scryer_facet", "anime"),
        ]);
        let resolution = matched_submission(
            "",
            "",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &resolution)
        else {
            panic!("expected ready completed download");
        };

        assert_eq!(resolved.parameters, completed.parameters);
    }

    #[test]
    fn completed_origin_resolution_ignores_stub_submission_without_params() {
        let completed = completed_download_with_parameters(vec![]);
        let resolution = matched_submission(
            "",
            "",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        assert!(matches!(
            resolve_completed_download_origin(&completed, &resolution),
            CompletedDownloadOriginResolution::NoScryerOrigin
        ));
    }

    #[test]
    fn invalid_and_sample_check_codes_are_permanent_policy_mismatches() {
        assert_eq!(
            skip_reason_for_import_check_code("invalid_extension"),
            ImportSkipReason::PolicyMismatch
        );
        assert_eq!(
            skip_reason_for_import_check_code("sample_file"),
            ImportSkipReason::PolicyMismatch
        );
        assert_eq!(
            skip_reason_for_import_check_code("sample_directory"),
            ImportSkipReason::PolicyMismatch
        );
    }

    #[test]
    fn retryable_completed_import_results_remain_pending_without_terminal_status() {
        let source = tempfile::tempdir().expect("source tempdir");
        let mut result = ImportResult {
            import_id: "import-1".to_string(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            title_id: Some("title-1".to_string()),
            source_system: Some("nzbget".to_string()),
            source_ref: Some("item-1".to_string()),
            source_title: Some("Release".to_string()),
            source_path: source.path().to_string_lossy().into_owned(),
            dest_path: None,
            quality: None,
            episode_ids: Vec::new(),
            file_size_bytes: None,
            link_type: None,
            error_message: None,
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Skipped),
            ImportStatus::Pending
        );

        result.skip_reason = Some(ImportSkipReason::UnparseableEpisode);
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Skipped),
            ImportStatus::Skipped
        );

        result.skip_reason = Some(ImportSkipReason::PolicyMismatch);
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Skipped),
            ImportStatus::Skipped
        );

        result.error_message = Some("source changed during copy".to_string());
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Failed),
            ImportStatus::Pending
        );
    }

    #[test]
    fn path_manual_import_validation_rejects_files_outside_selected_source() {
        let source = tempfile::tempdir().expect("source tempdir");
        let other = tempfile::tempdir().expect("other tempdir");
        let inside = source.path().join("episode.mkv");
        let outside = other.path().join("episode.mkv");
        fs::write(&inside, b"video").expect("write inside file");
        fs::write(&outside, b"video").expect("write outside file");

        let inside_mapping = ManualImportFileMapping {
            file_path: inside.to_string_lossy().into_owned(),
            episode_id: "ep-1".to_string(),
            quality: None,
        };
        assert!(
            validate_path_manual_import_mappings(
                &source.path().to_string_lossy(),
                &[inside_mapping]
            )
            .is_ok()
        );

        let outside_mapping = ManualImportFileMapping {
            file_path: outside.to_string_lossy().into_owned(),
            episode_id: "ep-1".to_string(),
            quality: None,
        };
        let err = validate_path_manual_import_mappings(
            &source.path().to_string_lossy(),
            &[outside_mapping],
        )
        .expect_err("outside file should be rejected");
        assert!(err.to_string().contains("outside the selected source path"));
    }

    #[cfg(unix)]
    #[test]
    fn sample_file_detection_uses_lossy_non_utf8_stem() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::Path;

        let path = Path::new(OsStr::from_bytes(b"/tmp/\xFFsample-clip.mkv"));
        assert!(is_sample_file(path));
    }
}
#[cfg(test)]
#[path = "../app_usecase_import_tests.rs"]
mod app_usecase_import_tests;
