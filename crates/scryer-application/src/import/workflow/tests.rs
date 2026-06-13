#[cfg(test)]
mod tests {
    use super::{
        ManualImportFileMapping, completed_import_status_for_result, is_sample_file,
        merge_scryer_origin_parameters, resolved_episode_ids_are_within_expected,
        sanitized_title_folder_component, skip_reason_for_import_check_code,
        validate_path_manual_import_mappings,
    };
    use crate::SubmissionScope;
    use chrono::Utc;
    use scryer_domain::{ImportDecision, ImportResult, ImportSkipReason, ImportStatus};
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

    #[test]
    fn completed_origin_parameters_preserve_series_movie_scope() {
        let mut parameters = Vec::new();

        merge_scryer_origin_parameters(
            &mut parameters,
            "title-1".to_string(),
            "anime".to_string(),
            &SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        assert!(parameters.iter().any(|(key, value)| {
            key == "*scryer_title_id" && value == "title-1"
        }));
        assert!(parameters.iter().any(|(key, value)| {
            key == "*scryer_facet" && value == "anime"
        }));
        assert!(parameters.iter().any(|(key, value)| {
            key == "*scryer_series_movie_link_id" && value == "series-movie-link-1"
        }));
        assert!(!parameters
            .iter()
            .any(|(key, _)| key == "*scryer_collection_id"));
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
