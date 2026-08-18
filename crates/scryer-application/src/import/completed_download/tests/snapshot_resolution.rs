use super::*;

#[tokio::test]
async fn completed_download_reresolution_ignores_conflicting_display_label() {
    let existing_title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let parsed_title = build_title("title-2", "The Other Movie", MediaFacet::Movie);
    let app = build_app(
        vec![existing_title.clone(), parsed_title],
        vec![],
        vec![],
        vec![],
    );
    let mut td = build_tracked_download(&existing_title.id, "movie", "Paper.Lantern.2012.1080p");
    td.match_type = TitleMatchType::IdOnly;
    td.source_title = None;
    let mut completed = build_completed_download(
        "The.Other.Movie.2020.1080p.WEB-DL",
        "/tmp/does-not-matter",
        Some("movie"),
    );
    completed.release_name = Some("Paper.Lantern.2012.1080p.WEB-DL".to_string());

    maybe_resolve_title_from_completed_download(&app, &mut td, &completed).await;

    assert_eq!(td.title_id.as_deref(), Some(existing_title.id.as_str()));
    assert_eq!(td.match_type, TitleMatchType::IdOnly);
    assert!(!has_id_only_conflict(&td));
    assert!(td.status_messages.is_empty());
}

#[tokio::test]
async fn completed_download_reresolution_enriches_matching_id_only_title() {
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let app = build_app(vec![title.clone()], vec![], vec![], vec![]);
    let mut td = build_tracked_download(&title.id, "movie", "Paper.Lantern.2012.1080p");
    td.match_type = TitleMatchType::IdOnly;
    td.source_title = None;
    td.facet = None;
    let mut completed = build_completed_download(
        "downloader display label",
        "/tmp/does-not-matter",
        Some("movie"),
    );
    completed.release_name = Some("Paper.Lantern.2012.1080p.WEB-DL".to_string());

    maybe_resolve_title_from_completed_download(&app, &mut td, &completed).await;

    assert_eq!(td.title_id.as_deref(), Some(title.id.as_str()));
    assert_eq!(td.match_type, TitleMatchType::IdOnly);
    assert_eq!(td.facet.as_deref(), Some("movie"));
    assert_eq!(
        td.source_title.as_deref(),
        Some("Paper.Lantern.2012.1080p.WEB-DL")
    );
    assert!(!has_id_only_conflict(&td));
}

#[tokio::test]
async fn completed_download_reresolution_replaces_title_parse_from_canonical_release_name() {
    let original_title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let completed_title = build_title("title-2", "The Other Movie", MediaFacet::Movie);
    let app = build_app(
        vec![original_title.clone(), completed_title.clone()],
        vec![],
        vec![],
        vec![],
    );
    let mut td = build_tracked_download(&original_title.id, "movie", "Paper.Lantern.2012.1080p");
    td.match_type = TitleMatchType::TitleParse;
    let mut completed = build_completed_download(
        "downloader display label",
        "/tmp/does-not-matter",
        Some("movie"),
    );
    completed.release_name = Some("The.Other.Movie.2020.1080p.WEB-DL".to_string());

    maybe_resolve_title_from_completed_download(&app, &mut td, &completed).await;

    assert_eq!(td.title_id.as_deref(), Some(completed_title.id.as_str()));
    assert_eq!(td.match_type, TitleMatchType::TitleParse);
    assert_eq!(
        td.source_title.as_deref(),
        Some("The.Other.Movie.2020.1080p.WEB-DL")
    );
}

#[test]
fn submission_match_without_scryer_origin_does_not_bypass_category_admission() {
    let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    td.match_type = TitleMatchType::Submission;
    td.client_item.is_scryer_origin = false;

    assert!(!tracked_download_has_scryer_submission(&td));
}

#[tokio::test]
async fn completed_download_proof_uses_media_basename_when_release_name_is_absent() {
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let app = build_app(vec![title.clone()], vec![], vec![], vec![]);
    let completed_dir = tempfile::tempdir().expect("create completed directory");
    std::fs::write(
        completed_dir
            .path()
            .join("Paper.Lantern.2012.1080p.WEB-DL.mkv"),
        b"video",
    )
    .expect("write completed video");
    let mut completed = build_completed_download(
        "downloader display label",
        completed_dir.path().to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.release_name = None;
    let mut td = build_tracked_download(&title.id, "movie", "Paper.Lantern.2012.1080p");
    td.match_type = TitleMatchType::TitleParse;
    td.source_title = None;
    td.client_item.is_scryer_origin = false;

    assert_eq!(
        completed_download_proves_assigned_title(&app, &td, &completed).await,
        AssignedTitleProof::Proven
    );
}

#[tokio::test]
async fn check_with_lookup_uses_snapshot_without_fetching_client_history() {
    let completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        std::env::temp_dir().to_string_lossy().as_ref(),
        Some("movie"),
    );
    let download_client = Arc::new(TestDownloadClient {
        completed_downloads: Arc::new(Mutex::new(vec![completed.clone()])),
        completed_download_calls: Arc::new(AtomicUsize::new(0)),
        recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        scoped_recent_completed_calls: Arc::new(Mutex::new(Vec::new())),
    });
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);

    check_with_lookup(&app, &mut td, Some(&lookup)).await;

    assert_eq!(
        td.completed_source
            .as_ref()
            .map(|source| source.name.as_str()),
        Some("Paper.Lantern.2012.1080p")
    );
    assert_eq!(
        download_client
            .completed_download_calls
            .load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn import_resolution_uses_snapshot_without_fetching_client_history() {
    let completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        std::env::temp_dir().to_string_lossy().as_ref(),
        Some("movie"),
    );
    let download_client = test_download_client_with_completed(completed.clone());
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    td.state = TrackedDownloadState::ImportPending;
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);

    let resolved = resolve_completed_download_for_import(&app, &mut td, Some(&lookup)).await;

    assert!(resolved.is_some());
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(
        download_client
            .completed_download_calls
            .load(Ordering::SeqCst),
        0
    );
    assert_eq!(
        download_client
            .recent_completed_download_calls
            .load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn import_resolution_missing_snapshot_entry_stays_retryable_without_full_history() {
    let completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        std::env::temp_dir().to_string_lossy().as_ref(),
        Some("movie"),
    );
    let download_client = test_download_client_with_completed(completed);
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    td.state = TrackedDownloadState::ImportPending;
    let lookup = CompletedDownloadLookup::empty_recent();

    let resolved = resolve_completed_download_for_import(&app, &mut td, Some(&lookup)).await;

    assert!(resolved.is_none());
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(
        download_client
            .completed_download_calls
            .load(Ordering::SeqCst),
        0
    );
    assert_eq!(
        download_client
            .recent_completed_download_calls
            .load(Ordering::SeqCst),
        0
    );
}
