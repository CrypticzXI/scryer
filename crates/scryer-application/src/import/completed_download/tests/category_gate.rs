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
    run_scryer_origin_identity_check(
        titles,
        assigned_title_id,
        facet,
        release_name,
        TitleMatchType::Submission,
    )
    .await
}

async fn run_scryer_origin_identity_check(
    titles: Vec<Title>,
    assigned_title_id: &str,
    facet: &str,
    release_name: &str,
    match_type: TitleMatchType,
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
    td.match_type = match_type;

    check(&app, &mut td).await;
    td
}

#[tokio::test]
async fn scryer_origin_name_proof_blocks_electric_bloom_before_import() {
    // Weaker scryer-origin provenance (no durable submission linkage) must
    // still prove identity from the release name — Electric Bloom cannot
    // prove the BLOOM alias.
    let mut title = build_title(
        "fragrant-flower",
        "The Fragrant Flower Blooms with Dignity",
        MediaFacet::Series,
    );
    title.year = Some(2025);
    title.aliases.push("BLOOM".to_string());

    let td = run_scryer_origin_identity_check(
        vec![title],
        "fragrant-flower",
        "series",
        "Electric.Bloom.S01E09.How.it.all.came.out.of.the.wash.MULTI.1080p.DSNP.WEB-DL.DDP5.1.H.264",
        TitleMatchType::TitleParse,
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
async fn scryer_submission_linkage_imports_unknown_completed_name() {
    // A Submission match is Scryer's own durable grab-time identity. A
    // completed name that names no library title is indistinguishable from
    // obfuscation, so the linkage stands as proof; only a completion that
    // positively asserts a *different* library title contradicts it.
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

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn scryer_origin_name_proof_blocks_ambiguous_one_piece() {
    let mut live = build_title("one-piece-live", "One Piece", MediaFacet::Series);
    live.year = Some(2023);
    let mut anime = build_title("one-piece-anime", "One Piece", MediaFacet::Anime);
    anime.year = Some(1999);

    let td = run_scryer_origin_identity_check(
        vec![live, anime],
        "one-piece-live",
        "series",
        "ONE.PIECE.S02E22.1080p.WEB-DL",
        TitleMatchType::TitleParse,
    )
    .await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
}

#[tokio::test]
async fn scryer_submission_linkage_imports_ambiguous_one_piece() {
    // The grab already passed the full disambiguator discipline; the linkage
    // carries that identity. A name both twins share is consistent with the
    // assignment, not a contradiction, so the import proceeds.
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

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
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

#[tokio::test]
async fn obfuscated_completed_name_proves_via_source_title() {
    let title = build_title("spy-family", "Spy x Family", MediaFacet::Series);
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let completed = build_completed_download(
        "abc123xyz987",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );
    let download_client = test_download_client_with_completed(completed);
    let app = build_app_with_download_client(vec![title], vec![], vec![], vec![], download_client);
    let mut td = build_tracked_download("spy-family", "series", "abc123xyz987");
    td.client_item.is_scryer_origin = true;
    // Weaker scryer-origin provenance — no durable linkage — so proof must come
    // from a raw name. The client obfuscated the completed name and folder
    // mid-flight; the grabbed release name Scryer recorded still proves the
    // identity.
    td.match_type = TitleMatchType::TitleParse;
    td.source_title =
        Some("ToonsHub.Spy.x.Family.S03E07.1080p.AMZN.WEB-DL.DDP2.0.H264".to_string());

    check(&app, &mut td).await;

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn contradictory_completed_name_blocks_despite_valid_source_title() {
    let spy = build_title("spy-family", "Spy x Family", MediaFacet::Series);
    let mut one_piece = build_title("one-piece-anime", "One Piece", MediaFacet::Anime);
    one_piece.year = Some(1999);
    let temp_dir = tempfile::tempdir().expect("temp dir");
    // The client's completed item positively names a different library title —
    // that is a contradiction, not obfuscation, and the historical grabbed
    // name must not override what actually finished on disk.
    let completed = build_completed_download(
        "One.Piece.1071.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );
    let download_client = test_download_client_with_completed(completed);
    let app = build_app_with_download_client(
        vec![spy, one_piece],
        vec![],
        vec![],
        vec![],
        download_client,
    );
    let mut td = build_tracked_download(
        "spy-family",
        "series",
        "One.Piece.1071.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
    );
    td.client_item.is_scryer_origin = true;
    td.match_type = TitleMatchType::Submission;
    td.source_title =
        Some("ToonsHub.Spy.x.Family.S03E07.1080p.AMZN.WEB-DL.DDP2.0.H264".to_string());

    check(&app, &mut td).await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
}

#[tokio::test]
async fn series_movie_release_proves_against_link_identity() {
    let mut series = build_title("psycho-pass", "Psycho Pass", MediaFacet::Series);
    series.year = Some(2012);
    let release_name = "Psycho-Pass.Sinners.of.the.System.Case.3.2019.1080p.BluRay.x264";
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let completed = build_completed_download(
        release_name,
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );
    let download_client = test_download_client_with_completed(completed);
    let app = build_app_with_download_client(vec![series], vec![], vec![], vec![], download_client);
    let now = chrono::Utc::now();
    // The search grabbed this release under the linked movie's own identity
    // (name, year 2019); the gate must validate against that same identity
    // instead of letting the parent series' year (2012) veto it.
    app.services
        .catalog
        .shows
        .upsert_series_movie_link(scryer_domain::SeriesMovieLink {
            id: "link-1".to_string(),
            series_title_id: "psycho-pass".to_string(),
            movie: scryer_domain::MovieEntity {
                id: "movie-1".to_string(),
                title: "Psycho-Pass Sinners of the System Case 3".to_string(),
                sort_title: None,
                slug: None,
                year: Some(2019),
                overview: None,
                poster_url: None,
                background_url: None,
                language: None,
                runtime_minutes: None,
                content_status: None,
                studio: None,
                digital_release_date: None,
                imdb_id: None,
                tvdb_id: None,
                tmdb_id: None,
                mal_id: None,
                anidb_id: None,
                created_at: now,
                updated_at: now,
            },
            placement: None,
            narrative_order: None,
            after_season: None,
            before_season: None,
            linked_episode_id: None,
            association_confidence: None,
            continuity_status: None,
            movie_form: None,
            confidence: None,
            signal_summary: None,
            source: None,
            monitored: true,
            legacy_collection_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("seed series movie link");
    let mut td = build_tracked_download("psycho-pass", "series", release_name);
    td.client_item.is_scryer_origin = true;
    // Name-proof provenance, so the link-identity evidence is what passes the
    // gate — a Submission match would be trusted by linkage before proving.
    td.match_type = TitleMatchType::TitleParse;

    check(&app, &mut td).await;

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn junk_source_title_is_not_an_identity_bypass() {
    let mut title = build_title(
        "fragrant-flower",
        "The Fragrant Flower Blooms with Dignity",
        MediaFacet::Series,
    );
    title.year = Some(2025);
    title.aliases.push("BLOOM".to_string());
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let completed = build_completed_download(
        "abc123xyz987",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );
    let download_client = test_download_client_with_completed(completed);
    let app = build_app_with_download_client(vec![title], vec![], vec![], vec![], download_client);
    let mut td = build_tracked_download("fragrant-flower", "series", "abc123xyz987");
    td.client_item.is_scryer_origin = true;
    // Name-proof provenance: without durable linkage, a junk source_title must
    // not become the identity proof.
    td.match_type = TitleMatchType::TitleParse;
    td.source_title = Some(
        "Electric.Bloom.S01E09.How.it.all.came.out.of.the.wash.MULTI.1080p.DSNP.WEB-DL.DDP5.1.H.264"
            .to_string(),
    );

    check(&app, &mut td).await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
}
