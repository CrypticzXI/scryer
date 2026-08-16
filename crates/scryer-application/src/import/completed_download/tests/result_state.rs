use super::*;

#[tokio::test]
async fn apply_result_marks_verified_already_present_skip_imported() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let app = build_app(
        vec![title],
        vec![collection],
        vec![episode],
        vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "already_present",
        )],
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("episode already imported".to_string()),
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::Imported);
    assert_eq!(td.status, TrackedDownloadStatus::Ok);
    assert!(td.status_messages.is_empty());
}

#[tokio::test]
async fn apply_result_keeps_rejected_already_imported_result_blocked() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let app = build_app(
        vec![title],
        vec![collection],
        vec![episode],
        vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "already_present",
        )],
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Rejected,
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("existing episode file is equal or better".to_string()),
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert_eq!(
        td.status_messages,
        vec!["existing episode file is equal or better".to_string()]
    );
}

#[tokio::test]
async fn apply_result_does_not_verify_unresolved_identity_rejection_as_imported() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let app = build_app(
        vec![title],
        vec![collection],
        vec![episode],
        vec![build_artifact("dl-1", "ep-1", "Show.S01E01.mkv")],
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Rejected,
        skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("download identity is unresolved".to_string()),
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
}

#[tokio::test]
async fn apply_result_keeps_ambiguous_obfuscated_episode_blocked_with_actionable_reason() {
    let app = build_app(vec![], vec![], vec![], vec![]);
    let release =
        "[Erai-raws].Hime-sama.Goumon.no.Jikan.Desu-09.[1080p][Multiple.Subtitle][AA7AC7E5]";
    let reason = "Automatic import could not choose a season for episode 9: the release name does not include a season and the downloaded filename is obfuscated. Open Manual Import and assign the correct season and episode.";
    let mut td = build_tracked_download("title-1", "anime", release);
    let result = ImportResult {
        import_id: "import-ambiguous-season".to_string(),
        decision: ImportDecision::Rejected,
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        title_id: Some("title-1".to_string()),
        source_system: Some("weaver".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some(release.to_string()),
        source_path: "/downloads/obfuscated".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some(reason.to_string()),
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert_eq!(td.status_messages, vec![reason.to_string()]);
}

#[tokio::test]
async fn apply_result_backs_off_no_video_import_before_blocking() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::NoVideoFiles),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: temp_dir.path().to_string_lossy().into_owned(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("no eligible video files found".to_string()),
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result.clone(), 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert!(td.status_messages[0].contains("Retrying automatically"));
    assert_eq!(td.no_video_import_retry.as_ref().unwrap().attempts, 1);

    assert!(!apply_import_result(&app, &mut td, result.clone(), 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert_eq!(td.no_video_import_retry.as_ref().unwrap().attempts, 2);

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert!(td.no_video_import_retry.is_none());
    assert!(td.status_messages[0].contains("Manual review required"));
}

#[tokio::test]
async fn apply_result_resets_no_video_retry_when_source_signature_changes() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::NoVideoFiles),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: temp_dir.path().to_string_lossy().into_owned(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("no eligible video files found".to_string()),
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result.clone(), 0).await);
    assert_eq!(td.no_video_import_retry.as_ref().unwrap().attempts, 1);
    std::fs::write(temp_dir.path().join("sample.txt"), b"not video").expect("write sample");

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.no_video_import_retry.as_ref().unwrap().attempts, 1);
}
