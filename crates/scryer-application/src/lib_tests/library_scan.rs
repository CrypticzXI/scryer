use super::*;

#[tokio::test]
async fn movie_full_scan_persists_and_reconciles_unmatched_items() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let first_path = tempdir.path().join("Unknown.One.2020.1080p.WEB-DL.mkv");
    std::fs::write(&first_path, b"movie").expect("write first movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        library_scanner.clone(),
        unmatched_items.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    let first_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("first movie scan");
    assert_eq!(first_summary.scanned, 1);
    assert_eq!(first_summary.unmatched, 1);

    let first_items = unmatched_items.items().await;
    assert_eq!(first_items.len(), 1);
    assert_eq!(first_items[0].facet, MediaFacet::Movie);
    assert_eq!(first_items[0].item_path, first_path.to_string_lossy());
    let first_session_id = first_items[0].scan_session_id.clone();

    let second_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("second movie scan");
    assert_eq!(second_summary.unmatched, 1);

    let second_items = unmatched_items.items().await;
    assert_eq!(second_items.len(), 1);
    assert_ne!(second_items[0].scan_session_id, first_session_id);

    std::fs::remove_file(&first_path).expect("remove first movie file");
    let second_path = tempdir.path().join("Unknown.Two.2021.2160p.BluRay.mkv");
    std::fs::write(&second_path, b"movie").expect("write second movie file");
    let third_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("third movie scan");
    assert_eq!(third_summary.scanned, 1);
    assert_eq!(third_summary.unmatched, 1);

    let third_items = unmatched_items.items().await;
    assert_eq!(third_items.len(), 1);
    assert_eq!(third_items[0].item_path, second_path.to_string_lossy());
}

#[tokio::test]
async fn movie_title_scan_removes_missing_tracked_movie_file() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Titanic (1997) - 2160p.mkv");
    std::fs::write(&movie_path, b"movie").expect("write movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items);
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Titanic".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                year: Some(1997),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, tempdir.path().to_string_lossy().as_ref())
        .await
        .expect("set movie folder path");

    let movie_path_string = movie_path.to_string_lossy().to_string();
    app.services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("2160p".to_string()),
            ordered_path: Some(movie_path_string.clone()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: title.monitored,
            created_at: Utc::now(),
        })
        .await
        .expect("seed movie collection");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: movie_path_string,
            size_bytes: 5,
            quality_label: Some("2160p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed movie media file");

    std::fs::remove_file(&movie_path).expect("remove movie file externally");

    let summary = app
        .scan_title_library(&user, &title.id)
        .await
        .expect("movie title scan should succeed");

    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);
    assert!(
        app.services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty()
    );
    assert!(
        app.services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .expect("list collections")
            .is_empty()
    );
}

#[tokio::test]
async fn movie_title_scan_multiple_files_picks_initial_primary_and_marks_rest_additional() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Primary Choice (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let small_path = title_dir.join("Primary.Choice.2026.720p.WEB-DL.mkv");
    let large_path = title_dir.join("Primary.Choice.2026.2160p.WEB-DL.mkv");
    std::fs::write(&small_path, vec![0_u8; 128]).expect("write smaller movie file");
    std::fs::write(&large_path, vec![0_u8; 512]).expect("write larger movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[small_path.as_path(), large_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Primary Choice", title_dir.as_path()).await;

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 2);
    assert_eq!(
        files.iter().filter(|file| file.role.is_primary()).count(),
        1
    );
    assert_eq!(
        media_file_role_for_path(&files, large_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, small_path.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn movie_library_scan_does_not_promote_additional_file_but_title_scan_does() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Additional Only (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let additional_path = title_dir.join("Additional.Only.2026.1080p.WEB-DL.mkv");
    std::fs::write(&additional_path, vec![0_u8; 256]).expect("write additional movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[additional_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Additional Only", title_dir.as_path()).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: additional_path.to_string_lossy().to_string(),
            size_bytes: 256,
            role: MediaFileRole::Additional,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed additional file");

    app.scan_library(&user, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after library scan");
    assert_eq!(
        media_file_role_for_path(&files, additional_path.as_path()),
        MediaFileRole::Additional
    );

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after title scan");
    assert_eq!(
        media_file_role_for_path(&files, additional_path.as_path()),
        MediaFileRole::Primary
    );
}

#[tokio::test]
async fn series_title_scan_imports_episode_file_as_primary() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Fresh Show (2026)");
    std::fs::create_dir(&title_dir).expect("create series folder");
    let episode_path = title_dir.join("Fresh Show - 1x01 - Pilot WEBDL-1080p.mkv");
    std::fs::write(&episode_path, vec![0_u8; 128]).expect("write episode file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(build_test_library_files(&[episode_path.as_path()]))
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile series root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fresh Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, title_dir.to_string_lossy().as_ref())
        .await
        .expect("set series folder path");
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");
    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(420),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan series title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(
        media_file_role_for_path(&files, episode_path.as_path()),
        MediaFileRole::Primary
    );
}

#[tokio::test]
async fn series_title_scan_marks_duplicate_episode_files_as_additional() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Fresh Show (2026)");
    std::fs::create_dir(&title_dir).expect("create series folder");
    let small_episode_path = title_dir.join("Fresh Show - 1x01 - Pilot WEBDL-720p.mkv");
    let large_episode_path = title_dir.join("Fresh Show - 1x01 - Pilot WEBDL-1080p.mkv");
    std::fs::write(&small_episode_path, vec![0_u8; 128]).expect("write smaller episode file");
    std::fs::write(&large_episode_path, vec![0_u8; 512]).expect("write larger episode file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(build_test_library_files(&[
            small_episode_path.as_path(),
            large_episode_path.as_path(),
        ]))
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile series root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fresh Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, title_dir.to_string_lossy().as_ref())
        .await
        .expect("set series folder path");
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");
    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(420),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan series title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 2);
    assert_eq!(
        files.iter().filter(|file| file.role.is_primary()).count(),
        1
    );
    assert_eq!(
        media_file_role_for_path(&files, large_episode_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, small_episode_path.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn series_library_scan_does_not_promote_additional_file_but_title_scan_does() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Additional Show (2026)");
    std::fs::create_dir(&title_dir).expect("create series folder");
    let episode_path = title_dir.join("Additional Show - 1x01 - Pilot WEBDL-1080p.mkv");
    std::fs::write(&episode_path, vec![0_u8; 256]).expect("write additional episode file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(build_test_library_files(&[episode_path.as_path()]))
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile series root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, title_dir.to_string_lossy().as_ref())
        .await
        .expect("set series folder path");
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");
    let episode = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(420),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");
    let file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: episode_path.to_string_lossy().to_string(),
            size_bytes: 256,
            role: MediaFileRole::Additional,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed additional episode file");
    app.services
        .library
        .media_files
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link additional file to episode");

    app.scan_library(&user, MediaFacet::Series)
        .await
        .expect("scan series library");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after library scan");
    assert_eq!(
        media_file_role_for_path(&files, episode_path.as_path()),
        MediaFileRole::Additional
    );

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan series title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after title scan");
    assert_eq!(
        media_file_role_for_path(&files, episode_path.as_path()),
        MediaFileRole::Primary
    );
}

#[tokio::test]
async fn movie_title_scan_preserves_existing_primary_even_when_other_file_scores_better() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Stable Primary (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let primary_path = title_dir.join("Stable.Primary.2026.720p.WEB-DL.mkv");
    let additional_path = title_dir.join("Stable.Primary.2026.2160p.BluRay.mkv");
    std::fs::write(&primary_path, vec![0_u8; 128]).expect("write primary movie file");
    std::fs::write(&additional_path, vec![0_u8; 1024]).expect("write additional movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[primary_path.as_path(), additional_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Stable Primary", title_dir.as_path()).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: primary_path.to_string_lossy().to_string(),
            size_bytes: 128,
            role: MediaFileRole::Primary,
            quality_label: Some("720p".to_string()),
            acquisition_score: Some(1),
            ..Default::default()
        })
        .await
        .expect("seed primary file");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: additional_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            role: MediaFileRole::Additional,
            quality_label: Some("2160p".to_string()),
            acquisition_score: Some(100_000),
            ..Default::default()
        })
        .await
        .expect("seed additional file");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_file_role_for_path(&files, primary_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, additional_path.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn movie_title_scan_preserves_user_selected_primary() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("User Primary (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let old_path = title_dir.join("User.Primary.2026.720p.WEB-DL.mkv");
    let selected_path = title_dir.join("User.Primary.2026.1080p.WEB-DL.mkv");
    std::fs::write(&old_path, vec![0_u8; 128]).expect("write old primary movie file");
    std::fs::write(&selected_path, vec![0_u8; 256]).expect("write selected movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[old_path.as_path(), selected_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "User Primary", title_dir.as_path()).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: old_path.to_string_lossy().to_string(),
            size_bytes: 128,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed old primary");
    let selected_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: selected_path.to_string_lossy().to_string(),
            size_bytes: 256,
            role: MediaFileRole::Additional,
            ..Default::default()
        })
        .await
        .expect("seed selected file");
    app.set_primary_movie_file(&user, &title.id, &selected_id)
        .await
        .expect("select primary file");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_file_role_for_path(&files, selected_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, old_path.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn movie_title_scan_repairs_multiple_primaries_by_oldest_created_at() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Primary Repair (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let oldest_path = title_dir.join("Primary.Repair.2026.720p.WEB-DL.mkv");
    let newest_path = title_dir.join("Primary.Repair.2026.2160p.WEB-DL.mkv");
    std::fs::write(&oldest_path, vec![0_u8; 128]).expect("write oldest primary movie file");
    std::fs::write(&newest_path, vec![0_u8; 1024]).expect("write newest primary movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[oldest_path.as_path(), newest_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Primary Repair", title_dir.as_path()).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: oldest_path.to_string_lossy().to_string(),
            size_bytes: 128,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed oldest primary");
    sleep(Duration::from_millis(2)).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: newest_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            role: MediaFileRole::Primary,
            quality_label: Some("2160p".to_string()),
            acquisition_score: Some(100_000),
            ..Default::default()
        })
        .await
        .expect("seed newest primary");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_file_role_for_path(&files, oldest_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, newest_path.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn movie_title_scan_cleans_out_of_canonical_folder_pollution() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let canonical_dir = tempdir.path().join("Polluted Movie (2026)");
    let duplicate_dir = tempdir.path().join("Polluted Movie Copy (2026)");
    std::fs::create_dir(&canonical_dir).expect("create canonical movie folder");
    std::fs::create_dir(&duplicate_dir).expect("create duplicate movie folder");
    let canonical_path = canonical_dir.join("Polluted.Movie.2026.1080p.WEB-DL.mkv");
    let duplicate_path = duplicate_dir.join("Polluted.Movie.2026.720p.WEB-DL.mkv");
    std::fs::write(&canonical_path, vec![0_u8; 256]).expect("write canonical movie file");
    std::fs::write(&duplicate_path, vec![0_u8; 128]).expect("write duplicate movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[canonical_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Polluted Movie", canonical_dir.as_path())
            .await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: canonical_path.to_string_lossy().to_string(),
            size_bytes: 256,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed canonical media file");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: duplicate_path.to_string_lossy().to_string(),
            size_bytes: 128,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed duplicate media file");
    for (path, id) in [
        (&canonical_path, "canonical"),
        (&duplicate_path, "duplicate"),
    ] {
        app.services
            .catalog
            .shows
            .create_collection(Collection {
                id: format!("collection-{id}"),
                title_id: title.id.clone(),
                collection_type: CollectionType::Movie,
                collection_index: id.to_string(),
                label: None,
                ordered_path: Some(path.to_string_lossy().to_string()),
                narrative_order: None,
                first_episode_number: None,
                last_episode_number: None,
                monitored: title.monitored,
                created_at: Utc::now(),
            })
            .await
            .expect("seed movie collection");
    }

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_path, canonical_path.to_string_lossy());
    assert_eq!(files[0].role, MediaFileRole::Primary);

    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    assert_eq!(collections.len(), 1);
    assert_eq!(
        collections[0].ordered_path.as_deref(),
        Some(canonical_path.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn movie_full_scan_skips_duplicate_same_title_sibling_folder_without_unmatched_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let first_dir = tempdir.path().join("Duplicate Title (2026)");
    let second_dir = tempdir.path().join("Duplicate Title Copy (2026)");
    std::fs::create_dir(&first_dir).expect("create first movie folder");
    std::fs::create_dir(&second_dir).expect("create second movie folder");
    let first_path = first_dir.join("Duplicate.Title.2026.1080p.WEB-DL.mkv");
    let second_path = second_dir.join("Duplicate.Title.2026.720p.WEB-DL.mkv");
    std::fs::write(&first_path, vec![0_u8; 256]).expect("write first movie file");
    std::fs::write(&second_path, vec![0_u8; 128]).expect("write second movie file");

    let (app, user, unmatched_items) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[first_path.as_path(), second_path.as_path()]),
        Arc::new(FixedBatchSearchMetadataGateway {
            results: vec![MetadataSearchItem {
                tvdb_id: "112233".to_string(),
                name: "Duplicate Title".to_string(),
                year: Some(2026),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        }),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Duplicate Title", first_dir.as_path()).await;

    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert!(summary.skipped >= 1);
    assert!(unmatched_items.items().await.is_empty());
    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list movie titles");
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0].id, title.id);
    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_path, first_path.to_string_lossy());
}

#[tokio::test]
async fn movie_full_scan_external_id_nfo_without_gateway_match_persists_unmatched_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Broken.Movie.2020.mkv");
    let nfo_path = tempdir.path().join("movie.nfo");
    std::fs::write(&movie_path, b"movie").expect("write movie file");
    std::fs::write(
        &nfo_path,
        r#"<movie><title>Broken Movie</title><tvdbid>123456</tvdbid></movie>"#,
    )
    .expect("write nfo");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![LibraryFile {
            path: movie_path.to_string_lossy().to_string(),
            display_name: "Broken.Movie.2020".to_string(),
            nfo_path: Some(nfo_path.to_string_lossy().to_string()),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user, _titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");
    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("movie scan should continue");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert!(
        app.list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
            .await
            .expect("list titles")
            .is_empty()
    );

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reason_code, "no_metadata_search_results");
    assert_eq!(items[0].error_message, None);
    assert_eq!(items[0].item_path, movie_path.to_string_lossy());
}

#[tokio::test]
async fn movie_full_scan_title_create_failure_from_search_persists_unmatched_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Matched.Movie.2020.mkv");
    std::fs::write(&movie_path, b"movie").expect("write movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user, titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(FixedBatchSearchMetadataGateway {
            results: vec![MetadataSearchItem {
                tvdb_id: "123456".to_string(),
                name: "Matched Movie".to_string(),
                year: Some(2020),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        }),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");
    titles
        .fail_create_or_get_existing("forced movie title creation failure from search")
        .await;

    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("movie scan should continue");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert!(
        app.list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
            .await
            .expect("list titles")
            .is_empty()
    );

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reason_code, "title_create_from_search_failed");
    assert_eq!(
        items[0].error_message.as_deref(),
        Some("repository: forced movie title creation failure from search")
    );
    assert_eq!(items[0].item_path, movie_path.to_string_lossy());
}

#[tokio::test]
async fn series_full_scan_persists_unmatched_folders() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(tempdir.path().join("Unknown Show (2020)"))
        .expect("create unknown show folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

    let summary = app
        .scan_library(&user, MediaFacet::Series)
        .await
        .expect("series scan");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Series);
    assert_eq!(items[0].display_name, "Unknown Show (2020)");
    assert_eq!(
        items[0].scan_root,
        tempdir.path().to_string_lossy().to_string()
    );
    assert_eq!(
        items[0].item_path,
        tempdir
            .path()
            .join("Unknown Show (2020)")
            .to_string_lossy()
            .to_string()
    );
}

#[tokio::test]
async fn movie_full_scan_scans_all_configured_roots_in_one_session() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create movie root one");
    std::fs::create_dir_all(&root_two).expect("create movie root two");
    std::fs::write(root_one.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie one");
    std::fs::write(root_two.join("Unknown.Two.2021.mkv"), b"movie-two").expect("seed movie two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-multi-root-full-scan";
    let summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Movie,
            Some(session_id.to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("movie full scan");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 2);
    assert_eq!(projected.status, LibraryScanStatus::Completed);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_one.to_string_lossy())
    );
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_two.to_string_lossy())
    );
}

#[tokio::test]
async fn series_full_scan_scans_all_configured_roots_in_one_session() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("series-a");
    let root_two = tempdir.path().join("series-b");
    std::fs::create_dir_all(root_one.join("Unknown Show One (2020)"))
        .expect("create first show folder");
    std::fs::create_dir_all(root_two.join("Unknown Show Two (2021)"))
        .expect("create second show folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store series roots");

    let summary = app
        .scan_library(&user, MediaFacet::Series)
        .await
        .expect("series full scan");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_one.to_string_lossy())
    );
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_two.to_string_lossy())
    );
}

#[tokio::test]
async fn movie_full_scan_marks_title_match_total_known_before_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    std::fs::create_dir_all(&movie_root).expect("create movie root");
    let movie_path = movie_root.join("Unknown.One.2020.mkv");
    std::fs::write(&movie_path, b"movie").expect("seed movie");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            movie_root.to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    let session_id = "movie-title-match-known-before-complete";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Movie,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1 && session.title_match_total_known
    })
    .await;
    assert_eq!(projected.title_match_progress.total, 1);
    assert_eq!(projected.title_match_progress.completed, 0);
    assert!(projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join movie full scan task")
        .expect("movie full scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn series_full_scan_marks_title_match_total_known_before_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

    let session_id = "series-title-match-known-before-complete";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1 && session.title_match_total_known
    })
    .await;
    assert_eq!(projected.title_match_progress.total, 1);
    assert_eq!(projected.title_match_progress.completed, 0);
    assert!(projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join series full scan task")
        .expect("series full scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn multi_root_full_scan_waits_for_final_root_to_mark_title_match_total_known() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("series-a");
    let root_two = tempdir.path().join("series-b");
    std::fs::create_dir_all(root_one.join("Unknown Show One (2020)"))
        .expect("create first series folder");
    std::fs::create_dir_all(root_two.join("Unknown Show Two (2021)"))
        .expect("create second series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::blocking_calls(&[1, 2]));
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store series roots");

    let session_id = "series-multi-root-title-match-known";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search_calls(1).await;

    let first_root_projected =
        wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
            session.found_titles == 1
        })
        .await;
    assert!(!first_root_projected.title_match_total_known);

    metadata_gateway.release_through(1);
    metadata_gateway.wait_for_batch_search_calls(2).await;

    let final_root_projected =
        wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
            session.found_titles == 2 && session.title_match_total_known
        })
        .await;
    assert_eq!(final_root_projected.title_match_progress.total, 2);
    assert!(final_root_projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join multi-root full scan task")
        .expect("multi-root full scan should complete");
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);
}

#[tokio::test]
async fn additive_scan_keeps_title_match_total_unknown_until_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        None,
        Some(vec![LibraryRootDraft {
            path: series_root.to_string_lossy().to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("store series library roots");

    let session_id = "series-additive-title-match-stays-unknown";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .background_library_refresh_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                session_id,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1
    })
    .await;
    assert!(!projected.title_match_total_known);

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join additive scan task")
        .expect("additive scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn movie_full_scan_skips_invalid_roots_and_finishes_warning() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let valid_root = tempdir.path().join("movies-valid");
    let invalid_root = tempdir.path().join("movies-missing");
    std::fs::create_dir_all(&valid_root).expect("create valid movie root");
    std::fs::write(valid_root.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie");

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&valid_root, true),
            build_root_folder_entry(&invalid_root, false),
        ]),
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-invalid-root-warning";
    let summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Movie,
            Some(session_id.to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("movie full scan with invalid root");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert_eq!(summary.skipped, 1);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 1);
    assert_eq!(projected.status, LibraryScanStatus::Warning);
}

#[tokio::test]
async fn background_refresh_movies_scans_all_configured_roots() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create movie root one");
    std::fs::create_dir_all(&root_two).expect("create movie root two");
    std::fs::write(root_one.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie one");
    std::fs::write(root_two.join("Unknown.Two.2021.mkv"), b"movie-two").expect("seed movie two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        None,
        Some(vec![
            LibraryRootDraft {
                path: root_one.to_string_lossy().to_string(),
                is_default: true,
            },
            LibraryRootDraft {
                path: root_two.to_string_lossy().to_string(),
                is_default: false,
            },
        ]),
        None,
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-multi-root-refresh";
    let summary = app
        .background_library_refresh_with_tracking(&user, MediaFacet::Movie, session_id)
        .await
        .expect("movie background refresh");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 2);
}

#[tokio::test]
async fn cancel_full_library_scan_marks_session_canceled_and_allows_restart() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

    let session_id = "cancel-full-library-scan";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let cancel_result = app
        .cancel_library_scan(&user, session_id)
        .await
        .expect("cancel full library scan");
    assert!(cancel_result.accepted);
    assert_eq!(cancel_result.session_id, session_id);

    metadata_gateway.release();

    handle
        .await
        .expect("join canceled scan task")
        .expect("canceled scan task should not error");

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected canceled session")
            .expect("canceled session snapshot");
    assert_eq!(projected.status, LibraryScanStatus::Canceled);
    assert_eq!(projected.found_titles, 1);
    assert!(
        app.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .get(session_id)
            .is_none(),
        "cancellation token should be cleared after terminal cancel",
    );

    let retry_summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Series,
            Some("cancel-full-library-scan-retry".to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("retry full scan after cancel");
    assert_eq!(retry_summary.unmatched, 1);
}

#[tokio::test]
async fn cancel_library_scan_rejects_additive_sessions() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        None,
        Some(vec![LibraryRootDraft {
            path: series_root.to_string_lossy().to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("store series library roots");

    let session_id = "cancel-additive-library-scan";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .background_library_refresh_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                session_id,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let error = app
        .cancel_library_scan(&user, session_id)
        .await
        .expect_err("additive scan should not be cancelable");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("only full library scans")),
        "unexpected cancel error: {error:?}"
    );

    metadata_gateway.release();

    handle
        .await
        .expect("join additive scan task")
        .expect("background refresh should complete");
}

#[tokio::test]
async fn ensure_library_scan_cancellation_token_reuses_existing_token() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, _user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let first = app
        .ensure_library_scan_cancellation_token("reused-library-scan-token", LibraryScanMode::Full)
        .await
        .expect("first full-scan cancel token");
    let second = app
        .ensure_library_scan_cancellation_token("reused-library-scan-token", LibraryScanMode::Full)
        .await
        .expect("second full-scan cancel token");

    first.cancel();

    assert!(
        second.is_cancelled(),
        "subsequent ensure should reuse the existing cancellation token",
    );
    assert_eq!(
        app.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .len(),
        1,
        "reusing a cancellation token should not create duplicate map entries",
    );
}

#[tokio::test]
async fn pending_import_counts_and_items_are_facet_scoped() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());
    let known_series_title = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Known Show".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                ..NewTitle::default()
            },
        )
        .await
        .expect("seed known series title");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-1",
            MediaFacet::Movie,
            "/movies",
            "/movies/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed movie item");
    let mut series_item = build_test_unmatched_item(
        "series-1",
        MediaFacet::Series,
        "/series",
        "/series/Unknown Show (2020)",
        "Unknown Show (2020)",
        "Unknown Show",
        Some(2020),
    );
    series_item.title_id = Some(known_series_title.title.id.clone());
    unmatched_items
        .upsert_library_scan_unmatched_item(&series_item)
        .await
        .expect("seed series item");
    let mut ignored_movie = build_test_unmatched_item(
        "movie-ignored-1",
        MediaFacet::Movie,
        "/movies",
        "/movies/Ignored.Movie.2020.mkv",
        "Ignored Movie",
        "Ignored Movie",
        Some(2020),
    );
    ignored_movie.status = PendingImportStatus::Ignored;
    unmatched_items
        .upsert_library_scan_unmatched_item(&ignored_movie)
        .await
        .expect("seed ignored movie item");

    let counts = app
        .pending_import_counts(&user)
        .await
        .expect("pending import counts");
    assert_eq!(counts.movie, 1);
    assert_eq!(counts.series, 1);
    assert_eq!(counts.anime, 0);

    let movie_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("movie pending imports");
    assert_eq!(movie_items.total, 1);
    assert_eq!(movie_items.items.len(), 1);
    assert_eq!(movie_items.items[0].display_name, "Unknown Movie");
    assert_eq!(movie_items.items[0].path, "/movies/Unknown.Movie.2020.mkv");
    assert_eq!(movie_items.items[0].folder_path, None);

    let ignored_movie_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored movie imports");
    assert_eq!(ignored_movie_items.total, 1);
    assert_eq!(ignored_movie_items.items.len(), 1);
    assert_eq!(ignored_movie_items.items[0].display_name, "Ignored Movie");
    assert_eq!(
        ignored_movie_items.items[0].status,
        PendingImportStatus::Ignored
    );

    let series_items = app
        .pending_imports(
            &user,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("series pending imports");
    assert_eq!(series_items.total, 1);
    assert_eq!(series_items.items.len(), 1);
    assert_eq!(
        series_items.items[0].folder_path.as_deref(),
        Some("/series/Unknown Show (2020)")
    );
    assert_eq!(
        series_items.items[0].title_id.as_deref(),
        Some(known_series_title.title.id.as_str())
    );
    assert_eq!(
        series_items.items[0].title_name.as_deref(),
        Some(known_series_title.title.name.as_str())
    );
    assert_eq!(
        series_items.items[0].title_slug,
        known_series_title.title.slug
    );
}

#[tokio::test]
async fn ignore_pending_import_moves_item_out_of_pending_counts() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-ignore-1",
            MediaFacet::Movie,
            "/movies",
            "/movies/Needs.Ignore.2020.mkv",
            "Needs Ignore",
            "Needs Ignore",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let result = app
        .ignore_pending_import(&user, "movie-ignore-1")
        .await
        .expect("ignore pending import");
    assert_eq!(result.status, PendingImportStatus::Ignored);

    let counts = app
        .pending_import_counts(&user)
        .await
        .expect("pending import counts after ignore");
    assert_eq!(counts.movie, 0);

    let pending_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("pending movie imports after ignore");
    assert_eq!(pending_items.total, 0);

    let ignored_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored movie imports after ignore");
    assert_eq!(ignored_items.total, 1);
    assert_eq!(ignored_items.items[0].id, "movie-ignore-1");
    assert_eq!(ignored_items.items[0].status, PendingImportStatus::Ignored);
}

#[tokio::test]
async fn update_media_settings_removing_root_clears_pending_imports_for_removed_root() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create root one");
    std::fs::create_dir_all(&root_two).expect("create root two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("seed movie roots");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-root-one",
            MediaFacet::Movie,
            root_one.to_string_lossy().as_ref(),
            root_one
                .join("Unknown.One.2020.mkv")
                .to_string_lossy()
                .as_ref(),
            "Unknown One",
            "Unknown One",
            Some(2020),
        ))
        .await
        .expect("seed first pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-root-two",
            MediaFacet::Movie,
            root_two.to_string_lossy().as_ref(),
            root_two
                .join("Unknown.Two.2021.mkv")
                .to_string_lossy()
                .as_ref(),
            "Unknown Two",
            "Unknown Two",
            Some(2021),
        ))
        .await
        .expect("seed second pending import");

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&root_one, true)]),
    )
    .await
    .expect("remove second movie root");

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].scan_root, root_one.to_string_lossy());
}

#[tokio::test]
async fn update_media_settings_root_folders_sync_default_library_roots() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("save movie roots");

    let library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Movie)
        .await
        .expect("lookup should succeed")
        .expect("default movie library");
    assert_eq!(
        library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            (root_one.to_string_lossy().to_string(), true),
            (root_two.to_string_lossy().to_string(), false),
        ]
    );
}

#[tokio::test]
async fn reconcile_default_library_roots_backfills_legacy_root_folders_when_bootstrap() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let legacy_roots = vec![
        RootFolderEntry {
            path: "/mnt/anime-main".to_string(),
            is_default: true,
        },
        RootFolderEntry {
            path: "/mnt/anime-archive".to_string(),
            is_default: false,
        },
    ];
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "anime.root_folders",
            &serde_json::to_string(&legacy_roots).expect("serialize legacy roots"),
        )
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings.clone(),
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );
    let anime_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let anime_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&anime_library_id)
        .await
        .expect("library lookup")
        .expect("default anime library");
    app.services
        .catalog
        .libraries
        .update(
            &anime_library_id,
            anime_library.name.clone(),
            anime_library.slug.clone(),
            vec![LibraryRootDraft {
                path: "/data/anime".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("seed bootstrap root");

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile roots");

    let media_settings = app
        .get_media_settings(&user, MediaFacet::Anime)
        .await
        .expect("anime settings");
    assert_eq!(media_settings.library_path, "/mnt/anime-main");
    assert_eq!(media_settings.root_folders, legacy_roots);
}

#[tokio::test]
async fn reconcile_default_library_roots_keeps_non_bootstrap_canonical_roots() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/legacy/movies")
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings.clone(),
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let movie_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&movie_library_id)
        .await
        .expect("library lookup")
        .expect("default movie library");
    app.services
        .catalog
        .libraries
        .update(
            &movie_library_id,
            movie_library.name.clone(),
            movie_library.slug.clone(),
            vec![LibraryRootDraft {
                path: "/canonical/movies".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("seed canonical root");

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile roots");

    let paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(paths.movie_path, "/canonical/movies");
    assert_eq!(
        app.read_setting_string_value_for_scope_explicit(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            None,
        )
        .await
        .expect("read mirror"),
        Some("/canonical/movies".to_string())
    );
}

#[tokio::test]
async fn reconcile_default_library_roots_repairs_missing_default_libraries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::empty()),
    );

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile missing defaults");

    let libraries = app
        .services
        .catalog
        .libraries
        .list(None)
        .await
        .expect("list repaired libraries");
    assert_eq!(libraries.len(), 3);

    let library_paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(library_paths.movie_path, "/data/movies");
    assert_eq!(library_paths.series_path, "/data/series");
    assert_eq!(library_paths.anime_path, "/data/anime");

    for (facet, expected_path) in [
        (MediaFacet::Movie, "/data/movies"),
        (MediaFacet::Series, "/data/series"),
        (MediaFacet::Anime, "/data/anime"),
    ] {
        let library = app
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await
            .expect("lookup repaired library")
            .expect("default library should be recreated");
        assert_eq!(
            crate::settings::runtime::root_folder_entries_from_library_roots(&library.roots),
            vec![RootFolderEntry {
                path: expected_path.to_string(),
                is_default: true,
            }]
        );
    }
}

#[tokio::test]
async fn update_library_paths_repairs_missing_default_libraries_before_save() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::empty()),
    );

    let updated = app
        .update_library_paths(
            &user,
            UpdateLibraryPaths {
                movie_path: "/wizard-movies".to_string(),
                series_path: "/wizard-series".to_string(),
                anime_path: Some("/wizard-anime".to_string()),
            },
        )
        .await
        .expect("update repaired library paths");

    assert_eq!(updated.movie_path, "/wizard-movies");
    assert_eq!(updated.series_path, "/wizard-series");
    assert_eq!(updated.anime_path, "/wizard-anime");

    for (facet, expected_path) in [
        (MediaFacet::Movie, "/wizard-movies"),
        (MediaFacet::Series, "/wizard-series"),
        (MediaFacet::Anime, "/wizard-anime"),
    ] {
        let root_folders = app
            .root_folders_for_facet(&facet)
            .await
            .expect("repaired root folders");
        assert_eq!(
            root_folders,
            vec![RootFolderEntry {
                path: expected_path.to_string(),
                is_default: true,
            }]
        );
    }
}

#[tokio::test]
async fn find_or_create_default_user_dedupes_duplicate_default_library_grants() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let duplicate_movie_library = mock_default_library(MediaFacet::Movie);
    let libraries = vec![
        duplicate_movie_library.clone(),
        duplicate_movie_library,
        mock_default_library(MediaFacet::Series),
        mock_default_library(MediaFacet::Anime),
    ];
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::with_libraries(libraries)),
    );

    let admin = app
        .find_or_create_default_user()
        .await
        .expect("create default admin");
    assert_eq!(admin.username, user.username);

    let grants = app
        .services
        .catalog
        .libraries
        .permission_masks_for_user(&admin.id)
        .await
        .expect("load grants");
    let unique_library_ids = grants
        .iter()
        .map(|grant| grant.library_id.clone())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(grants.len(), 3);
    assert_eq!(unique_library_ids.len(), 3);
}

#[tokio::test]
async fn find_or_create_default_user_creates_passwordless_default_actor() {
    let (app, _) = bootstrap();

    let admin = app
        .find_or_create_default_user()
        .await
        .expect("create default admin actor");

    assert_eq!(admin.username, "admin");
    assert!(admin.password_hash.is_none());
    assert!(
        !app.existing_default_admin_uses_bootstrap_password()
            .await
            .expect("check default admin password")
    );
}

#[tokio::test]
async fn existing_default_admin_uses_bootstrap_password_detects_admin_password() {
    let (app, _) = bootstrap();
    let mut admin = User::new_admin("admin");
    admin.password_hash = Some(app.hash_password("admin").expect("hash admin password"));
    app.services
        .identity
        .users
        .create(admin)
        .await
        .expect("seed default admin");

    assert!(
        app.existing_default_admin_uses_bootstrap_password()
            .await
            .expect("check default admin password")
    );
}

#[tokio::test]
async fn usable_admin_login_accepts_non_default_full_admin() {
    let users = Arc::new(MockUserRepo::default());
    let (app, _) = bootstrap_with_user_repo(users.clone());
    let mut owner = User::new_admin("owner");
    owner.password_hash = Some(
        app.hash_password("correct horse battery staple")
            .expect("hash owner password"),
    );
    let owner = users.create(owner).await.expect("seed owner");
    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(
            &owner.id,
            scryer_domain::UserAuthorization::full_admin().app,
        )
        .await
        .expect("grant full admin permissions");

    assert!(
        app.usable_admin_login_exists()
            .await
            .expect("check usable admin login")
    );
}

#[tokio::test]
async fn usable_admin_login_rejects_passwordless_default_admin_only() {
    let (app, _) = bootstrap();
    app.find_or_create_default_user()
        .await
        .expect("create passwordless default admin");

    assert!(
        !app.usable_admin_login_exists()
            .await
            .expect("check usable admin login")
    );
}

#[tokio::test]
async fn recover_reserved_admin_access_creates_recovery_admin() {
    let (app, _) = bootstrap();
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            MFA_REQUIRE_CONFIG_STEP_UP_KEY,
            None,
            "true".to_string(),
            "test",
            None,
        )
        .await
        .expect("seed config step-up setting");

    let recovery_admin = app
        .recover_reserved_admin_access("new recovery password")
        .await
        .expect("recover reserved admin access");
    assert_eq!(recovery_admin.username, "recovery-admin");

    let stored_recovery_admin = app
        .services
        .identity
        .users
        .get_by_username("recovery-admin")
        .await
        .expect("load recovery admin")
        .expect("recovery admin created during recovery");
    assert_eq!(stored_recovery_admin.id, recovery_admin.id);
    let password_hash = recovery_admin
        .password_hash
        .as_deref()
        .expect("recovery admin password hash");
    assert!(
        app.validate_password("new recovery password", password_hash)
            .expect("validate recovery admin password")
    );
    assert!(matches!(
        app.authenticate_credentials("recovery-admin", "new recovery password")
            .await,
        Err(AppError::Unauthorized(_))
    ));
    app.set_recovery_admin_login_enabled(true);
    assert_eq!(
        app.authenticate_credentials("recovery-admin", "new recovery password")
            .await
            .expect("authenticate recovery admin while recovery is enabled")
            .id,
        recovery_admin.id
    );
    assert!(
        app.services
            .identity
            .totp
            .get_credential_for_user(&recovery_admin.id)
            .await
            .expect("load recovery admin TOTP")
            .is_none()
    );
    assert!(
        app.services
            .identity
            .webauthn
            .list_credentials_for_user(&recovery_admin.id)
            .await
            .expect("load recovery admin passkeys")
            .is_empty()
    );

    let authorization = app
        .load_user_authorization(&recovery_admin)
        .await
        .expect("load recovery admin authorization");
    assert!(
        authorization
            .app
            .contains(scryer_domain::UserAuthorization::full_admin().app)
    );
    assert!(
        !app.security_settings()
            .await
            .expect("load security settings")
            .mfa_require_config_step_up
    );
}

#[tokio::test]
async fn update_default_library_roots_updates_all_facet_root_read_paths() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let expected_roots = vec![
        RootFolderEntry {
            path: "/library/movies-main".to_string(),
            is_default: true,
        },
        RootFolderEntry {
            path: "/library/movies-archive".to_string(),
            is_default: false,
        },
    ];

    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(
            expected_roots
                .iter()
                .map(|root| LibraryRootDraft {
                    path: root.path.clone(),
                    is_default: root.is_default,
                })
                .collect(),
        ),
        None,
    )
    .await
    .expect("update canonical roots");

    let media_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(media_settings.library_path, "/library/movies-main");
    assert_eq!(media_settings.root_folders, expected_roots);

    let library_paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(library_paths.movie_path, "/library/movies-main");

    let root_folders = app
        .root_folders_for_facet(&MediaFacet::Movie)
        .await
        .expect("root folders");
    assert_eq!(root_folders, media_settings.root_folders);
}

#[tokio::test]
async fn title_root_resolution_uses_owning_library_roots() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/library/default-movies".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("default movie library roots should update");
    let kids_library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids Movies".to_string(),
            vec![LibraryRootDraft {
                path: "/library/kids-movies".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("custom library should be created");
    let mut title = make_due_hydration_title("custom-library-title", MediaFacet::Movie, 42);
    title.library_id = kids_library.id.clone();
    title.root_folder_id = kids_library.roots[0].id.clone();

    let import_paths = crate::import_workflow::resolve_import_paths(&app, &title)
        .await
        .expect("import paths should resolve");
    assert_eq!(import_paths.media_root, "/library/kids-movies");

    let recycle_root = crate::recycle_bin::media_root_for_title(&app, &title).await;
    assert_eq!(recycle_root.as_deref(), Some("/library/kids-movies"));
}

#[tokio::test]
async fn update_default_library_rejects_empty_roots_without_persisting_them() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/library/movies-main".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("initial default roots should update");

    let error = app
        .update_library(&user, &movie_library_id, None, Some(Vec::new()), None)
        .await
        .expect_err("empty default roots should be rejected");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("libraries require at least one root folder")),
        "unexpected error: {error:?}"
    );

    let library = app
        .services
        .catalog
        .libraries
        .get_by_id(&movie_library_id)
        .await
        .expect("library lookup should succeed")
        .expect("movie library should exist");
    assert_eq!(library.roots.len(), 1);
    assert_eq!(library.roots[0].path, "/library/movies-main");
}

#[tokio::test]
async fn update_library_removing_root_clears_pending_imports_for_removed_root() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/movies-old".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("initial default roots should update");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root-canonical",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");

    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/movies-new".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("canonical roots should update");

    let items = unmatched_items.items().await;
    assert!(items.is_empty());
}

#[tokio::test]
async fn update_library_paths_removing_root_clears_pending_imports_for_removed_root() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "series-root",
            MediaFacet::Series,
            "/series",
            "/series/Unknown Show (2020)",
            "Unknown Show",
            "Unknown Show",
            Some(2020),
        ))
        .await
        .expect("seed kept pending import");

    app.update_library_paths(
        &user,
        UpdateLibraryPaths {
            movie_path: "/movies-new".to_string(),
            series_path: "/series".to_string(),
            anime_path: Some("/anime".to_string()),
        },
    )
    .await
    .expect("update library paths");

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Series);
    assert_eq!(items[0].scan_root, "/series");
}

#[tokio::test]
async fn update_library_paths_allows_partial_wizard_paths() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime-old")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings.clone(),
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items,
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

    let updated = app
        .update_library_paths(
            &user,
            UpdateLibraryPaths {
                movie_path: "".to_string(),
                series_path: "/series-new".to_string(),
                anime_path: None,
            },
        )
        .await
        .expect("update partial library paths");

    assert_eq!(updated.movie_path, "/movies-old");
    assert_eq!(updated.series_path, "/series-new");
    assert_eq!(updated.anime_path, "/anime-old");
}

#[tokio::test]
async fn save_external_import_library_paths_removing_root_clears_pending_imports_for_removed_root()
{
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root-external",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "anime-root-external",
            MediaFacet::Anime,
            "/anime",
            "/anime/Unknown Anime",
            "Unknown Anime",
            "Unknown Anime",
            Some(2021),
        ))
        .await
        .expect("seed kept pending import");

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec!["/movies-new".to_string()],
                series_paths: vec![],
                anime_paths: vec![],
            },
        )
        .await
        .expect("save external import paths");

    assert!(saved);
    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Anime);
    assert_eq!(items[0].scan_root, "/anime");
}

#[tokio::test]
async fn save_external_import_library_paths_persists_multiple_root_folders_per_facet() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec![
                    "/movies-primary".to_string(),
                    "/movies-secondary".to_string(),
                ],
                series_paths: vec!["/series-main".to_string(), "/series-archive".to_string()],
                anime_paths: vec!["/anime".to_string()],
            },
        )
        .await
        .expect("save external import paths");

    assert!(saved);

    let movie_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(movie_settings.library_path, "/movies-primary");
    assert_eq!(
        movie_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/movies-primary".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/movies-secondary".to_string(),
                is_default: false,
            },
        ]
    );

    let series_settings = app
        .get_media_settings(&user, MediaFacet::Series)
        .await
        .expect("series settings");
    assert_eq!(series_settings.library_path, "/series-main");
    assert_eq!(
        series_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/series-main".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/series-archive".to_string(),
                is_default: false,
            },
        ]
    );

    let movie_library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Movie)
        .await
        .expect("lookup should succeed")
        .expect("default movie library");
    assert_eq!(
        movie_library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            ("/movies-primary".to_string(), true),
            ("/movies-secondary".to_string(), false),
        ]
    );

    let series_library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Series)
        .await
        .expect("lookup should succeed")
        .expect("default series library");
    assert_eq!(
        series_library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            ("/series-main".to_string(), true),
            ("/series-archive".to_string(), false),
        ]
    );
}

#[tokio::test]
async fn save_external_import_library_paths_accepts_custom_selected_paths() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec![
                    "/custom/movies".to_string(),
                    "/custom/movies-archive".to_string(),
                ],
                series_paths: vec!["/custom/series".to_string()],
                anime_paths: vec!["/custom/anime".to_string()],
            },
        )
        .await
        .expect("save custom external import paths");

    assert!(saved);

    let movie_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(movie_settings.library_path, "/custom/movies");
    assert_eq!(
        movie_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/custom/movies".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/custom/movies-archive".to_string(),
                is_default: false,
            },
        ]
    );

    let series_settings = app
        .get_media_settings(&user, MediaFacet::Series)
        .await
        .expect("series settings");
    assert_eq!(series_settings.library_path, "/custom/series");
    assert_eq!(
        series_settings.root_folders,
        vec![RootFolderEntry {
            path: "/custom/series".to_string(),
            is_default: true,
        }]
    );

    let anime_settings = app
        .get_media_settings(&user, MediaFacet::Anime)
        .await
        .expect("anime settings");
    assert_eq!(anime_settings.library_path, "/custom/anime");
    assert_eq!(
        anime_settings.root_folders,
        vec![RootFolderEntry {
            path: "/custom/anime".to_string(),
            is_default: true,
        }]
    );
}

#[tokio::test]
async fn resolve_pending_import_creates_unmonitored_movie_title_and_clears_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Unknown.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    tvdb_id: 123_456,
                    name: "Matched Movie".into(),
                    slug: "matched-movie".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "Matched overview".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    background_url: None,
                    language: "eng".into(),
                    runtime_minutes: 101,
                    sort_title: "Matched Movie".into(),
                    imdb_id: "tt0123456".into(),
                    anidb_id: None,
                    genres: vec!["Drama".into()],
                    studio: "Test Studio".into(),
                    tmdb_release_date: Some("2020-01-01".into()),
                },
            )]),
        }),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Matched Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let result = app
        .resolve_pending_import(&user, "movie-resolve-1", "123456")
        .await
        .expect("resolve pending import");

    assert!(result.created);
    assert!(!result.title.monitored);
    assert_eq!(result.title.name, "Matched Movie");
    assert!(
        result.library_scan.scanned
            + result.library_scan.matched
            + result.library_scan.imported
            + result.library_scan.skipped
            + result.library_scan.unmatched
            > 0
    );
    assert!(unmatched_items.items().await.is_empty());
}

#[tokio::test]
async fn resolve_ignored_pending_import_creates_unmonitored_movie_title_and_clears_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Ignored.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    tvdb_id: 123_456,
                    name: "Matched Movie".into(),
                    slug: "matched-movie".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "Matched overview".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    background_url: None,
                    language: "eng".into(),
                    runtime_minutes: 101,
                    sort_title: "Matched Movie".into(),
                    imdb_id: "tt0123456".into(),
                    anidb_id: None,
                    genres: vec!["Drama".into()],
                    studio: "Test Studio".into(),
                    tmdb_release_date: Some("2020-01-01".into()),
                },
            )]),
        }),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    let mut ignored_item = build_test_unmatched_item(
        "movie-resolve-ignored-1",
        MediaFacet::Movie,
        tempdir.path().to_string_lossy().as_ref(),
        movie_path.to_string_lossy().as_ref(),
        "Ignored Movie",
        "Matched Movie",
        Some(2020),
    );
    ignored_item.status = PendingImportStatus::Ignored;

    unmatched_items
        .upsert_library_scan_unmatched_item(&ignored_item)
        .await
        .expect("seed ignored import");

    let result = app
        .resolve_pending_import(&user, "movie-resolve-ignored-1", "123456")
        .await
        .expect("resolve ignored pending import");

    assert!(result.created);
    assert_eq!(result.title.name, "Matched Movie");
    assert!(unmatched_items.items().await.is_empty());
}

#[tokio::test]
async fn resolve_pending_import_failure_keeps_pending_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Unknown.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-failure-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Matched Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let error = app
        .resolve_pending_import(&user, "movie-resolve-failure-1", "999999")
        .await
        .expect_err("resolution should fail without metadata");
    assert!(!error.to_string().trim().is_empty());
    assert_eq!(unmatched_items.items().await.len(), 1);
    assert!(
        app.list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn hydrate_titles_bulk_updates_title_name_for_selected_metadata_language() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, METADATA_LANGUAGE_KEY, "jpn")
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    tvdb_id: 123_456,
                    name: "デューン".into(),
                    slug: "dune".into(),
                    year: Some(2021),
                    content_status: "Released".into(),
                    overview: "日本語概要".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    background_url: None,
                    language: "jpn".into(),
                    runtime_minutes: 155,
                    sort_title: "デューン".into(),
                    imdb_id: "tt1160419".into(),
                    anidb_id: None,
                    genres: vec!["Science Fiction".into()],
                    studio: "Legendary".into(),
                    tmdb_release_date: Some("2021-10-22".into()),
                },
            )]),
        }),
    );

    let created = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Glass Harbor".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "123456".to_string(),
                }],
                root_folder_id: None,
                min_availability: None,
                poster_url: None,
                year: None,
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("seed untranslated title");
    let created_title = created.title;

    let mut outcome = app
        .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
            title: created_title.clone(),
            requested_tvdb_id: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::Interactive,
        }])
        .await
        .expect("hydrate title");

    let hydrated = outcome
        .hydrated_titles
        .remove(&created_title.id)
        .expect("hydrated title should be returned");
    assert_eq!(hydrated.name, "デューン");
    assert_eq!(hydrated.metadata_language.as_deref(), Some("jpn"));
    assert_eq!(hydrated.overview.as_deref(), Some("日本語概要"));

    let persisted = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list titles");
    assert_eq!(persisted[0].name, "デューン");
    assert_eq!(persisted[0].metadata_language.as_deref(), Some("jpn"));
}

#[tokio::test]
async fn background_title_hydrator_skips_full_scan_owned_facets_and_hydrates_other_due_titles() {
    let metadata_gateway = Arc::new(MockMetadataGateway {
        movies: HashMap::from([(101, make_movie_metadata(101, "Eligible Movie"))]),
    });
    let (app, _user, titles) = bootstrap_with_metadata_gateway_and_titles(metadata_gateway);

    TitleRepository::create(
        &*titles,
        make_due_hydration_title("movie-due", MediaFacet::Movie, 101),
    )
    .await
    .expect("seed due movie title");
    TitleRepository::create(
        &*titles,
        make_due_hydration_title("series-due", MediaFacet::Series, 202),
    )
    .await
    .expect("seed due series title");

    app.runtime
        .library
        .library_scan_tracker
        .start_session_with_id(
            "series-scan-owned".to_string(),
            MediaFacet::Series,
            LibraryScanMode::Full,
        )
        .await
        .expect("start series scan");

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.child_token(),
    ));

    let hydrated_movie = timeout(Duration::from_secs(1), async {
        loop {
            let title = app
                .services
                .catalog
                .titles
                .get_by_id("movie-due")
                .await
                .expect("load movie title")
                .expect("movie title should exist");
            if title.metadata_fetched_at.is_some() {
                break title;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("movie due title should hydrate");

    let skipped_series = app
        .services
        .catalog
        .titles
        .get_by_id("series-due")
        .await
        .expect("load series title")
        .expect("series title should exist");

    token.cancel();
    handle
        .await
        .expect("title hydration loop should stop cleanly");

    assert!(hydrated_movie.metadata_fetched_at.is_some());
    assert!(
        skipped_series.metadata_fetched_at.is_none(),
        "background worker should not hydrate titles for the facet owned by the active scan"
    );
}

#[tokio::test]
async fn background_title_hydrator_retries_scan_owned_movie_titles_after_scan_clears() {
    for mode in [LibraryScanMode::Full, LibraryScanMode::Additive] {
        let metadata_gateway = Arc::new(MockMetadataGateway {
            movies: HashMap::from([(303, make_movie_metadata(303, "Recovered Movie"))]),
        });
        let (app, _user, titles) = bootstrap_with_metadata_gateway_and_titles(metadata_gateway);

        let title_id = format!("movie-due-{}", mode.as_str());
        let session_id = format!("scan-owned-{}", mode.as_str());
        TitleRepository::create(
            &*titles,
            make_due_hydration_title(&title_id, MediaFacet::Movie, 303),
        )
        .await
        .expect("seed due movie title");

        app.runtime
            .library
            .library_scan_tracker
            .start_session_with_id(session_id.clone(), MediaFacet::Movie, mode.clone())
            .await
            .expect("start movie scan");

        let token = tokio_util::sync::CancellationToken::new();
        let handle = tokio::spawn(start_background_title_hydration_loop(
            app.clone(),
            token.child_token(),
        ));

        let premature_hydration = timeout(Duration::from_millis(250), async {
            loop {
                let title = app
                    .services
                    .catalog
                    .titles
                    .get_by_id(&title_id)
                    .await
                    .expect("load movie title")
                    .expect("movie title should exist");
                if title.metadata_fetched_at.is_some() {
                    break title;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            premature_hydration.is_err(),
            "background worker should not hydrate a scan-owned movie title while the scan is active"
        );

        app.runtime
            .library
            .library_scan_tracker
            .cancel_session(&session_id)
            .await
            .expect("clear scan-owned session");

        let hydrated = timeout(Duration::from_secs(1), async {
            loop {
                let title = app
                    .services
                    .catalog
                    .titles
                    .get_by_id(&title_id)
                    .await
                    .expect("load movie title after scan clear")
                    .expect("movie title should still exist");
                if title.metadata_fetched_at.is_some() {
                    break title;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("movie due title should hydrate after scan clears");

        token.cancel();
        handle
            .await
            .expect("title hydration loop should stop cleanly");

        assert!(
            hydrated.metadata_fetched_at.is_some(),
            "scan-owned movie title should remain due and hydrate after the scan clears"
        );
    }
}

#[tokio::test]
async fn resolve_pending_import_failure_restores_existing_title_folder_path() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Missing.Movie.2020.mkv");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner.set_library_files(vec![]).await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    let existing_title = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Existing Movie".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "123456".to_string(),
                }],
                root_folder_id: None,
                min_availability: None,
                poster_url: None,
                year: Some(2020),
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("seed existing title");
    let existing_title = existing_title.title;
    app.services
        .catalog
        .titles
        .set_folder_path(&existing_title.id, "/existing/movies/Existing Movie")
        .await
        .expect("set original folder path");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-existing-failure-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Existing Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let error = app
        .resolve_pending_import(&user, "movie-resolve-existing-failure-1", "123456")
        .await
        .expect_err("resolution should fail when scan finds no files");
    assert!(!error.to_string().trim().is_empty());
    assert_eq!(unmatched_items.items().await.len(), 1);

    let refreshed_title = app
        .services
        .catalog
        .titles
        .get_by_id(&existing_title.id)
        .await
        .expect("load existing title")
        .expect("existing title should still exist");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some("/existing/movies/Existing Movie")
    );
}
