use super::*;

#[tokio::test]
async fn foreign_title_parse_requires_submission_origin_or_scryer_category() {
    for (completed_category, queue_category) in [(None, None), (Some("other"), None)] {
        let td = run_category_gate_check(
            Arc::new(TestSettingsRepo::default()),
            completed_category,
            queue_category,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        assert!(
            td.status_messages
                .iter()
                .any(|message| message == FOREIGN_CATEGORY_BLOCKED_MESSAGE)
        );
    }

    let td = run_category_gate_check(
        Arc::new(TestSettingsRepo::default()),
        Some("movie"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);

    let default_category_settings = Arc::new(TestSettingsRepo::default());
    set_scoped_default_category(&default_category_settings, "movie", "Configured Movies").await;
    let td = run_category_gate_check(
        default_category_settings,
        Some("Configured Movies"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn foreign_title_parse_with_orphan_submission_still_requires_scryer_category() {
    let settings = Arc::new(TestSettingsRepo::default());
    set_scoped_default_category(&settings, "movie", "movie").await;
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let completed = build_completed_download(
        "Paper.Lantern.2012.1080p.WEB-DL",
        temp_dir.path().to_string_lossy().as_ref(),
        None,
    );
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let download_client = test_download_client_with_completed(completed);
    let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
    download_submissions
        .record_submission(DownloadSubmission {
            title_id: String::new(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "dl-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Paper.Lantern.2012.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .expect("record orphan submission");
    let app = build_app_with_download_client_configs_submissions_and_settings(
        vec![title],
        vec![],
        vec![],
        vec![],
        TestAppRepositories {
            download_client,
            download_client_configs: Arc::new(NullDownloadClientConfigRepository),
            download_submissions,
            settings,
        },
    );
    let mut td = build_foreign_completed_tracked_download(None, TitleMatchType::TitleParse, false);

    check(&app, &mut td).await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert!(
        td.status_messages
            .iter()
            .any(|message| message == FOREIGN_CATEGORY_BLOCKED_MESSAGE)
    );
}

#[tokio::test]
async fn completed_category_gate_honors_facet_and_library_shadowing() {
    let facet_settings = Arc::new(TestSettingsRepo::default());
    set_scoped_routing(
        &facet_settings,
        "movie",
        r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
    )
    .await;
    let td = run_category_gate_check(
        facet_settings,
        Some("Facet Movies"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);

    let library_settings = Arc::new(TestSettingsRepo::default());
    set_scoped_routing(
        &library_settings,
        "movie",
        r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
    )
    .await;
    set_scoped_routing(
        &library_settings,
        "movie_default_library",
        r#"{"client-1":{"enabled":true,"category":"Library Movies"}}"#,
    )
    .await;
    let td = run_category_gate_check(
        library_settings.clone(),
        Some("Facet Movies"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    let td = run_category_gate_check(
        library_settings,
        Some("Library Movies"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);

    let empty_library_category_settings = Arc::new(TestSettingsRepo::default());
    set_scoped_routing(
        &empty_library_category_settings,
        "movie",
        r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
    )
    .await;
    set_scoped_routing(
        &empty_library_category_settings,
        "movie_default_library",
        r#"{"client-1":{"enabled":true,"category":""}}"#,
    )
    .await;
    let td = run_category_gate_check(
        empty_library_category_settings.clone(),
        Some("Facet Movies"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    let td = run_category_gate_check(
        empty_library_category_settings,
        Some("movie"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn completed_category_gate_honors_missing_disabled_and_invalid_routing() {
    let missing_library_client_settings = Arc::new(TestSettingsRepo::default());
    set_scoped_routing(
        &missing_library_client_settings,
        "movie",
        r#"{"client-1":{"enabled":true,"category":"movie"}}"#,
    )
    .await;
    set_scoped_routing(
        &missing_library_client_settings,
        "movie_default_library",
        r#"{"other-client":{"enabled":true,"category":"movie"}}"#,
    )
    .await;
    let td = run_category_gate_check(
        missing_library_client_settings,
        Some("movie"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);

    let missing_facet_client_settings = Arc::new(TestSettingsRepo::default());
    set_scoped_routing(
        &missing_facet_client_settings,
        "movie",
        r#"{"other-client":{"enabled":true,"category":"other"}}"#,
    )
    .await;
    let td = run_category_gate_check(
        missing_facet_client_settings,
        Some("movie"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);

    for (scope_id, settings) in [
        (
            "movie_default_library",
            Arc::new(TestSettingsRepo::default()),
        ),
        ("movie", Arc::new(TestSettingsRepo::default())),
    ] {
        set_scoped_routing(
            &settings,
            scope_id,
            r#"{"client-1":{"enabled":false,"category":"movie"}}"#,
        )
        .await;
        let td = run_category_gate_check(
            settings,
            Some("movie"),
            None,
            TitleMatchType::TitleParse,
            false,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    }

    let invalid_library_settings = Arc::new(TestSettingsRepo::default());
    set_scoped_routing(
        &invalid_library_settings,
        "movie_default_library",
        "not-json",
    )
    .await;
    set_scoped_routing(
        &invalid_library_settings,
        "movie",
        r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
    )
    .await;
    let td = run_category_gate_check(
        invalid_library_settings,
        Some("Facet Movies"),
        None,
        TitleMatchType::TitleParse,
        false,
    )
    .await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn confirmed_completed_downloads_bypass_category_gate() {
    for (match_type, is_scryer_origin) in [
        (TitleMatchType::Submission, false),
        (TitleMatchType::ClientParameter, false),
        (TitleMatchType::TitleParse, true),
    ] {
        let td = run_category_gate_check(
            Arc::new(TestSettingsRepo::default()),
            None,
            None,
            match_type,
            is_scryer_origin,
        )
        .await;
        assert_eq!(td.state, TrackedDownloadState::ImportPending);
    }
}

#[tokio::test]
async fn manual_assignment_allows_retry_after_category_block() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let completed = build_completed_download(
        "Paper.Lantern.2012.1080p.WEB-DL",
        temp_dir.path().to_string_lossy().as_ref(),
        None,
    );
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let download_client = test_download_client_with_completed(completed);
    let app = build_app_with_download_client(
        vec![title.clone()],
        vec![],
        vec![],
        vec![],
        download_client,
    );
    let mut td = build_foreign_completed_tracked_download(None, TitleMatchType::TitleParse, false);

    check(&app, &mut td).await;
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);

    crate::tracked_downloads::assign_title_to_tracked_download(&app, &mut td, &title).await;
    assert_eq!(td.match_type, TitleMatchType::Submission);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);

    td.state = TrackedDownloadState::Downloading;
    check(&app, &mut td).await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

async fn run_scryer_submission_identity_check(
    titles: Vec<Title>,
    assigned_title_id: &str,
    facet: &str,
    release_name: &str,
) -> TrackedDownload {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let completed = build_completed_download(
        release_name,
        temp_dir.path().to_string_lossy().as_ref(),
        Some(facet),
    );
    let download_client = test_download_client_with_completed(completed);
    let app = build_app_with_download_client(titles, vec![], vec![], vec![], download_client);
    let mut td = build_tracked_download(assigned_title_id, facet, release_name);
    td.client_item.is_scryer_origin = true;
    td.match_type = TitleMatchType::Submission;

    check(&app, &mut td).await;
    td
}

#[tokio::test]
async fn scryer_submission_blocks_electric_bloom_before_import() {
    let mut title = build_title(
        "fragrant-flower",
        "The Fragrant Flower Blooms with Dignity",
        MediaFacet::Series,
    );
    title.year = Some(2025);
    title.aliases.push("BLOOM".to_string());

    let td = run_scryer_submission_identity_check(
        vec![title],
        "fragrant-flower",
        "series",
        "Electric.Bloom.S01E09.How.it.all.came.out.of.the.wash.MULTI.1080p.DSNP.WEB-DL.DDP5.1.H.264",
    )
    .await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert!(
        td.status_messages.iter().any(|message| {
            message.contains("no longer proves the title assigned at grab time")
        })
    );
}

#[tokio::test]
async fn scryer_submission_blocks_ambiguous_one_piece_before_import() {
    let mut live = build_title("one-piece-live", "One Piece", MediaFacet::Series);
    live.year = Some(2023);
    let mut anime = build_title("one-piece-anime", "One Piece", MediaFacet::Anime);
    anime.year = Some(1999);

    let td = run_scryer_submission_identity_check(
        vec![live, anime],
        "one-piece-live",
        "series",
        "ONE.PIECE.S02E22.1080p.WEB-DL",
    )
    .await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
}

#[tokio::test]
async fn scryer_submission_with_complete_title_proof_reaches_import_pending() {
    let title = build_title("spy-family", "Spy x Family", MediaFacet::Series);
    let td = run_scryer_submission_identity_check(
        vec![title],
        "spy-family",
        "series",
        "ToonsHub.Spy.x.Family.S03E07.1080p.AMZN.WEB-DL.DDP2.0.H264",
    )
    .await;

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}
